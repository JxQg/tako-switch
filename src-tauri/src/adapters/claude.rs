use crate::adapters::{
    has_tako_mcp_server, merge_tako_mcp_server, read_json_object_or_empty, AppId, AppMode,
    DesktopAppAdapter, DiagnosticResult, InstallInfo,
};
use crate::agy_proxy::{
    ensure_gateway_service, is_proxy_healthy, stop_gateway_service_if_unused,
    CLAUDE_PROXY_BASE_URL, LOCAL_GATEWAY_PORT, LOCAL_GATEWAY_URL,
};
use crate::config::{backup_file_once, read_backup, read_main_api_key, read_switch_state, write_atomic, AppPaths};
use base64::engine::general_purpose::STANDARD as BASE64_STD;
use base64::Engine;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

const TAKO_LOGO_PNG: &[u8] = include_bytes!("../../icons/128x128.png");

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version, sync_macos_apple_languages,
};

pub const TAKO_CLAUDE_3P_UUID: &str = "7a405017-c100-4a80-9000-7a4050170001";
#[cfg(target_os = "macos")]
const CLAUDE_MACOS_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    fn mcp_1p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_1p_dir.join("claude_desktop_config.json")
    }

    fn mcp_3p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_3p_dir.join("claude_desktop_config.json")
    }

    fn meta_3p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_3p_dir.join("configLibrary").join("_meta.json")
    }

    fn profile_3p_path(paths: &AppPaths) -> PathBuf {
        paths
            .claude_3p_dir
            .join("configLibrary")
            .join(format!("{TAKO_CLAUDE_3P_UUID}.json"))
    }

    fn claude_3p_app_config_path(paths: &AppPaths) -> PathBuf {
        paths.claude_3p_dir.join("config.json")
    }

    pub fn claude_1p_app_config_path(paths: &AppPaths) -> PathBuf {
        paths.claude_1p_dir.join("config.json")
    }

    fn claude_code_settings_path(paths: &AppPaths) -> PathBuf {
        paths.home.join(".claude").join("settings.json")
    }

    fn has_managed_tako_mcp(paths: &AppPaths) -> bool {
        read_json_object_or_empty(&Self::profile_3p_path(paths))
            .ok()
            .and_then(|m| m.get("managedMcpServers").and_then(Value::as_array).cloned())
            .map(|entries| {
                entries.iter().any(|e| {
                    e.get("name").and_then(Value::as_str) == Some("tako")
                        && e.get("transport").and_then(Value::as_str) == Some("stdio")
                        && e.get("command").and_then(Value::as_str).is_some_and(|c| !c.is_empty())
                })
            })
            .unwrap_or(false)
    }

    fn set_claude_code_mcp_permissions(paths: &AppPaths, enable: bool) -> Result<(), String> {
        let path = Self::claude_code_settings_path(paths);
        if !enable && !path.exists() {
            return Ok(());
        }
        backup_file_once(paths, "claude", "claude_code_settings.json", &path)?;
        let mut root = read_json_object_or_empty(&path)?;

        let mut rules = Vec::with_capacity(crate::adapters::TAKO_MCP_TOOL_NAMES.len() + 1);
        rules.push("mcp__tako__*".to_string());
        for &tool_name in crate::adapters::TAKO_MCP_TOOL_NAMES {
            rules.push(format!("mcp__tako__{tool_name}"));
        }

        if enable {
            let permissions = root
                .entry("permissions".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !permissions.is_object() {
                *permissions = Value::Object(Map::new());
            }
            if let Some(perm_obj) = permissions.as_object_mut() {
                let allow = perm_obj
                    .entry("allow".to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if !allow.is_array() {
                    *allow = Value::Array(Vec::new());
                }
                if let Some(arr) = allow.as_array_mut() {
                    for rule in &rules {
                        if !arr.iter().any(|v| v.as_str() == Some(rule.as_str())) {
                            arr.push(Value::String(rule.clone()));
                        }
                    }
                }
            }
        } else {
            let backup_obj = read_backup(paths, "claude", "claude_code_settings.json")
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|v| v.as_object().cloned());
            let backup_had_permissions = backup_obj
                .as_ref()
                .is_some_and(|o| o.contains_key("permissions"));
            let backup_had_allow = backup_obj
                .as_ref()
                .and_then(|o| o.get("permissions"))
                .and_then(Value::as_object)
                .is_some_and(|p| p.contains_key("allow"));

            if let Some(perm_obj) = root.get_mut("permissions").and_then(Value::as_object_mut) {
                if let Some(arr) = perm_obj.get_mut("allow").and_then(Value::as_array_mut) {
                    arr.retain(|v| {
                        v.as_str()
                            .map(|s| !rules.iter().any(|r| r == s))
                            .unwrap_or(true)
                    });
                    if arr.is_empty() && !backup_had_allow {
                        perm_obj.remove("allow");
                    }
                }
                if perm_obj.is_empty() && !backup_had_permissions {
                    root.remove("permissions");
                }
            }
        }

        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
        write_atomic(&path, &bytes, false)
    }

    fn tako_deployment_logo_data_uri() -> String {
        format!("data:image/png;base64,{}", BASE64_STD.encode(TAKO_LOGO_PNG))
    }

    fn sync_claude_json_locale_field(
        paths: &AppPaths,
        path: &Path,
        backup_name: &str,
        key: &str,
        target_value: &str,
        enable: bool,
    ) -> Result<(), String> {
        if !enable && !path.exists() {
            return Ok(());
        }
        backup_file_once(paths, "claude", backup_name, path)?;
        let mut root = read_json_object_or_empty(path)?;
        if enable {
            root.insert(key.to_string(), Value::String(target_value.to_string()));
        } else {
            let restored = read_backup(paths, "claude", backup_name)
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|v| v.as_object().and_then(|o| o.get(key)).cloned());
            if let Some(orig_val) = restored {
                root.insert(key.to_string(), orig_val);
            } else {
                root.remove(key);
            }
            let absent_marker = paths
                .backups_dir("claude")
                .join(format!("{backup_name}.absent"));
            if root.is_empty() && absent_marker.exists() {
                if path.exists() {
                    fs::remove_file(path)
                        .map_err(|e| format!("移除临时 {} 失败: {e}", path.display()))?;
                }
                return Ok(());
            }
        }
        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
        write_atomic(path, &bytes, false)
    }

    fn sync_claude_chinese_locale(paths: &AppPaths, enable: bool) -> Result<(), String> {
        Self::sync_claude_json_locale_field(
            paths,
            &Self::claude_3p_app_config_path(paths),
            "claude_3p_config.json",
            "locale",
            "zh-Hans",
            enable,
        )?;
        Self::sync_claude_json_locale_field(
            paths,
            &Self::claude_1p_app_config_path(paths),
            "claude_1p_config.json",
            "locale",
            "zh-Hans",
            enable,
        )?;
        Self::sync_claude_json_locale_field(
            paths,
            &Self::claude_code_settings_path(paths),
            "claude_code_settings.json",
            "language",
            "Simplified Chinese",
            enable,
        )?;
        #[cfg(target_os = "macos")]
        sync_macos_apple_languages(paths, CLAUDE_MACOS_BUNDLE_ID, enable);
        Ok(())
    }

    pub fn is_chinese_configured(paths: &AppPaths) -> bool {
        let has_1p = read_json_object_or_empty(&Self::claude_1p_app_config_path(paths))
            .ok()
            .and_then(|m| {
                m.get("locale")
                    .and_then(Value::as_str)
                    .map(|s| s == "zh-Hans")
            })
            .unwrap_or(false);
        let has_3p = read_json_object_or_empty(&Self::claude_3p_app_config_path(paths))
            .ok()
            .and_then(|m| {
                m.get("locale")
                    .and_then(Value::as_str)
                    .map(|s| s == "zh-Hans")
            })
            .unwrap_or(false);
        let has_cc = read_json_object_or_empty(&Self::claude_code_settings_path(paths))
            .ok()
            .and_then(|m| {
                m.get("language")
                    .and_then(Value::as_str)
                    .map(|s| s == "Simplified Chinese")
            })
            .unwrap_or(false);
        has_1p || has_3p || has_cc
    }

    pub fn sync_standalone_chinese_locale(paths: &AppPaths, enable: bool) -> Result<(), String> {
        Self::sync_claude_chinese_locale(paths, enable)
    }

    fn deactivate_3p(paths: &AppPaths) -> Result<(), String> {
        let meta_path = Self::meta_3p_path(paths);
        if meta_path.exists() {
            let mut meta = read_json_object_or_empty(&meta_path)?;
            let mut changed = false;
            if meta.get("appliedId").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID) {
                let restored_applied = read_backup(paths, "claude", "3p_meta.json")
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|v| {
                        v.get("appliedId")
                            .and_then(Value::as_str)
                            .filter(|id| *id != TAKO_CLAUDE_3P_UUID)
                            .map(str::to_string)
                    });
                if let Some(prev) = restored_applied {
                    meta.insert("appliedId".to_string(), Value::String(prev));
                } else {
                    meta.remove("appliedId");
                }
                changed = true;
            }
            if let Some(entries) = meta.get_mut("entries").and_then(Value::as_array_mut) {
                let prev_len = entries.len();
                entries.retain(|e| {
                    e.get("id").and_then(Value::as_str) != Some(TAKO_CLAUDE_3P_UUID)
                });
                if entries.len() != prev_len {
                    changed = true;
                }
            }
            if changed {
                let bytes = serde_json::to_vec_pretty(&Value::Object(meta))
                    .map_err(|e| format!("序列化 Claude 3p _meta.json 失败: {e}"))?;
                write_atomic(&meta_path, &bytes, false)?;
            }
        }
        let prof_path = Self::profile_3p_path(paths);
        if prof_path.exists() {
            let _ = fs::remove_file(&prof_path);
        }
        Ok(())
    }
}

impl DesktopAppAdapter for ClaudeAdapter {
    fn id(&self) -> AppId {
        AppId::Claude
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        if let Some(ref override_dir) = paths.claude_app_override_dir {
            if override_dir.exists() {
                let path_str = override_dir.to_string_lossy().to_string();
                let kind = if path_str.to_ascii_lowercase().contains("windowsapps") {
                    "msix"
                } else if override_dir
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("app"))
                    || override_dir.join("Contents").is_dir()
                {
                    "app_bundle"
                } else {
                    "exe"
                };
                return InstallInfo {
                    installed: true,
                    version: None,
                    install_path: Some(path_str),
                    install_kind: Some(kind.to_string()),
                };
            }
            return InstallInfo {
                installed: false,
                version: None,
                install_path: None,
                install_kind: None,
            };
        }
        if paths.is_test {
            return InstallInfo {
                installed: false,
                version: None,
                install_path: None,
                install_kind: None,
            };
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) = find_macos_app(paths, &["Claude.app"], CLAUDE_MACOS_BUNDLE_ID) {
                let version = read_macos_app_bundle_version(&app_path);
                return InstallInfo {
                    installed: true,
                    version,
                    install_path: Some(app_path.to_string_lossy().to_string()),
                    install_kind: Some("app_bundle".to_string()),
                };
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some((ver, path, kind)) = detect_windows_claude(paths) {
                return InstallInfo {
                    installed: true,
                    version: ver,
                    install_path: Some(path),
                    install_kind: Some(kind),
                };
            }
        }
        InstallInfo {
            installed: false,
            version: None,
            install_path: None,
            install_kind: None,
        }
    }

    fn is_running(&self, _paths: &AppPaths) -> bool {
        #[cfg(target_os = "macos")]
        {
            is_macos_bundle_id_running(CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            super::windows_process::is_running(AppId::Claude)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            quit_macos_bundle_id(CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_claude()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path = find_macos_app(paths, &["Claude.app"], CLAUDE_MACOS_BUNDLE_ID);
            launch_macos_app(app_path.as_deref(), CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_claude(paths)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 Claude Desktop".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        let meta_path = Self::meta_3p_path(paths);
        if let Ok(meta) = read_json_object_or_empty(&meta_path) {
            if meta.get("appliedId").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID)
                && Self::profile_3p_path(paths).exists()
            {
                return AppMode::Tako;
            }
        }
        AppMode::Official
    }

    fn read_mcp_enabled(&self, paths: &AppPaths, mode: AppMode) -> bool {
        match mode {
            AppMode::Tako => Self::has_managed_tako_mcp(paths),
            AppMode::Official => has_tako_mcp_server(&Self::mcp_1p_path(paths)),
        }
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let mcp_1p = Self::mcp_1p_path(paths);
        let mcp_3p = Self::mcp_3p_path(paths);
        let meta_3p = Self::meta_3p_path(paths);

        backup_file_once(paths, "claude", "1p_claude_desktop_config.json", &mcp_1p)?;
        backup_file_once(paths, "claude", "3p_claude_desktop_config.json", &mcp_3p)?;
        backup_file_once(paths, "claude", "3p_meta.json", &meta_3p)?;

        match mode {
            AppMode::Official => {
                Self::deactivate_3p(paths)?;
                merge_tako_mcp_server(&mcp_1p, paths, false)?;
                merge_tako_mcp_server(&mcp_3p, paths, false)?;
                Self::set_claude_code_mcp_permissions(paths, false)?;
                Self::sync_claude_chinese_locale(paths, false)?;
                stop_gateway_service_if_unused(paths)
            }
            AppMode::Tako => {
                let api_key = read_main_api_key(paths)
                    .ok_or_else(|| "请先登录 Tako 账号或填入 Key，再切换到全 Tako 模式".to_string())?;
                let state = read_switch_state(paths);
                ensure_gateway_service(paths)?;
                let base_url = CLAUDE_PROXY_BASE_URL.to_string();
                let display_name = state.normalized_gateway_name();
                let subtitle = state.normalized_gateway_subtitle();
                let balance_remain =
                    crate::auth::read_balance_cache(paths).and_then(|c| c.status.wallet_remain_usd);
                let effective_subtitle: Option<String> = match (balance_remain, subtitle.as_deref())
                {
                    (Some(rem), Some(sub))
                        if sub != crate::config::DEFAULT_GATEWAY_DISPLAY_SUBTITLE =>
                    {
                        Some(
                            format!("{sub} · 余额 {}", crate::auth::format_usd(rem))
                                .chars()
                                .take(60)
                                .collect(),
                        )
                    }
                    (Some(rem), _) => Some(
                        format!("余额 {} · {}", crate::auth::format_usd(rem), display_name)
                            .chars()
                            .take(60)
                            .collect(),
                    ),
                    (None, Some(sub)) => Some(sub.to_string()),
                    (None, None) => None,
                };
                let instructions = if state.prefer_chinese_ui {
                    "默认使用简体中文回复用户（除非用户明确要求其他语言）。需要生图/改图、生成视频、语音合成、联网搜索、多模型参谋或查询 Tako 账户余额/额度时，优先调用已挂载的 Tako MCP 工具（tako_generate_image / tako_generate_video / tako_generate_audio / tako_web_search / tako_consult / tako_account_status）。遵循 tako-skill 规范：保留用户指定模型；生图/视频超时或报错时切勿自动盲目重试（先查 tako_account_status，视频超时应传 resume_task_id 恢复）；联网搜索须引用实际返回的 results[].url，未返回来源时不要编造链接。"
                } else {
                    "When generating/editing images, generating videos, synthesizing audio, searching the web, consulting a second frontier model, or checking current Tako balance/quota, prefer the mounted Tako MCP tools (tako_generate_image / tako_generate_video / tako_generate_audio / tako_web_search / tako_consult / tako_account_status). Follow tako-skill guardrails: preserve user-selected models; never blindly auto-retry paid image/video calls on timeout or failure (check tako_account_status first and pass resume_task_id for video recovery); cite actual results[].url in web searches without fabricating URLs."
                };

                let mut profile = Map::new();
                profile.insert("inferenceProvider".to_string(), Value::String("gateway".to_string()));
                profile.insert("inferenceGatewayBaseUrl".to_string(), Value::String(base_url));
                profile.insert("inferenceGatewayAuthScheme".to_string(), Value::String("bearer".to_string()));
                profile.insert("inferenceCredentialKind".to_string(), Value::String("static".to_string()));
                profile.insert("inferenceGatewayApiKey".to_string(), Value::String(api_key));
                profile.insert("deploymentDisplayName".to_string(), Value::String(display_name.clone()));
                if let Some(sub) = effective_subtitle {
                    profile.insert("deploymentDisplaySubtitle".to_string(), Value::String(sub));
                }
                profile.insert(
                    "deploymentLogo".to_string(),
                    Value::String(Self::tako_deployment_logo_data_uri()),
                );
                profile.insert("inferenceModelPricingEnabled".to_string(), Value::Bool(true));
                profile.insert("mcpToolTimeoutSec".to_string(), serde_json::json!(300));
                profile.insert("isLocalDevMcpEnabled".to_string(), Value::Bool(true));
                profile.insert(
                    "mcpPersistentAlwaysAllowEnabled".to_string(),
                    Value::Bool(true),
                );
                // The 3p deployment gates tool egress with a fail-closed allowlist, which
                // blocks WebFetch/WebSearch on any host not listed here.
                profile.insert(
                    "coworkEgressAllowedHosts".to_string(),
                    serde_json::json!(["*"]),
                );
                profile.insert(
                    "builtinToolPolicy".to_string(),
                    serde_json::json!({
                        "WebFetch": "allow",
                        "WebSearch": "allow",
                    }),
                );
                let mut tool_policy = Map::new();
                tool_policy.insert("*".to_string(), Value::String("allow".to_string()));
                for &tool_name in crate::adapters::TAKO_MCP_TOOL_NAMES {
                    tool_policy.insert(tool_name.to_string(), Value::String("allow".to_string()));
                }
                // Deliver the MCP server as a managed stdio entry: Claude Desktop only
                // enforces `toolPolicy` for servers the deployment itself provides, so this
                // is what makes the Tako tools usable without a per-tool approval prompt.
                let mcp_entry = crate::adapters::build_stdio_mcp_json_entry(paths);
                profile.insert(
                    "managedMcpServers".to_string(),
                    serde_json::json!([{
                        "name": "tako",
                        "transport": "stdio",
                        "command": mcp_entry.get("command").cloned().unwrap_or(Value::Null),
                        "args": mcp_entry.get("args").cloned().unwrap_or_else(|| serde_json::json!([])),
                        "toolPolicy": Value::Object(tool_policy)
                    }]),
                );
                profile.insert(
                    "organizationInstructions".to_string(),
                    Value::String(instructions.to_string()),
                );
                // Desktop only offers effort levels for gateway models declared here.
                if let Some(inference_models) =
                    crate::agy_proxy::load_claude_inference_models(paths, &state.model_filters.claude)
                {
                    profile.insert("inferenceModels".to_string(), inference_models);
                }

                let prof_bytes = serde_json::to_vec_pretty(&Value::Object(profile))
                    .map_err(|e| format!("序列化 Claude 3p profile 失败: {e}"))?;
                write_atomic(&Self::profile_3p_path(paths), &prof_bytes, true)?;

                let mut meta = if meta_3p.exists() {
                    read_json_object_or_empty(&meta_3p)?
                } else {
                    Map::new()
                };
                meta.insert(
                    "appliedId".to_string(),
                    Value::String(TAKO_CLAUDE_3P_UUID.to_string()),
                );
                let mut entries = meta
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                entries.retain(|e| {
                    e.get("id").and_then(Value::as_str) != Some(TAKO_CLAUDE_3P_UUID)
                });
                entries.push(serde_json::json!({
                    "id": TAKO_CLAUDE_3P_UUID,
                    "name": display_name,
                    "provider": "gateway",
                    "note": "Managed by Tako Switch"
                }));
                meta.insert("entries".to_string(), Value::Array(entries));
                let meta_bytes = serde_json::to_vec_pretty(&Value::Object(meta))
                    .map_err(|e| format!("序列化 Claude 3p _meta.json 失败: {e}"))?;
                write_atomic(&meta_3p, &meta_bytes, false)?;

                merge_tako_mcp_server(&mcp_1p, paths, false)?;
                // The 3p slot gets tako from the managed profile entry, not the user config,
                // so leave the user config alone to avoid starting a second instance.
                merge_tako_mcp_server(&mcp_3p, paths, false)?;
                Self::set_claude_code_mcp_permissions(paths, true)?;
                Self::sync_claude_chinese_locale(paths, state.prefer_chinese_ui)?;
                Ok(())
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let mut notes = vec![
            format!("1p MCP 配置: {}", Self::mcp_1p_path(paths).display()),
            format!("3p 网关目录: {}", paths.claude_3p_dir.display()),
        ];
        let mode = self.read_mode(paths);
        if mode == AppMode::Tako {
            match read_json_object_or_empty(&Self::profile_3p_path(paths)) {
                Err(e) => issues.push(e),
                Ok(prof) => {
                    let url = prof
                        .get("inferenceGatewayBaseUrl")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if url.is_empty() {
                        issues.push("inferenceGatewayBaseUrl 为空".to_string());
                    } else if url.trim_end_matches('/').ends_with("/v1") {
                        issues.push(format!("Claude Desktop 网关 URL 不能带 /v1 后缀，当前为: {url}"));
                    } else if url.starts_with(LOCAL_GATEWAY_URL) && !paths.is_test {
                        if is_proxy_healthy(LOCAL_GATEWAY_PORT) {
                            notes.push(format!("本地统一网关: 运行中 (127.0.0.1:{LOCAL_GATEWAY_PORT})"));
                        } else {
                            issues.push(format!(
                                "本地统一网关 (127.0.0.1:{LOCAL_GATEWAY_PORT}) 未运行，请重新点击 Tako 或重启 Tako Switch"
                            ));
                        }
                    }
                    let scheme = prof
                        .get("inferenceGatewayAuthScheme")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if scheme != "bearer" {
                        issues.push(format!("inferenceGatewayAuthScheme 应为 bearer，当前为: {scheme}"));
                    }
                    let cred_kind = prof
                        .get("inferenceCredentialKind")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if cred_kind != "static" {
                        issues.push(format!("inferenceCredentialKind 应为 static，当前为: {cred_kind}"));
                    }
                    let key = prof
                        .get("inferenceGatewayApiKey")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if key.is_empty() {
                        issues.push("inferenceGatewayApiKey 为空".to_string());
                    }
                }
            }
            if !Self::has_managed_tako_mcp(paths) {
                notes.push("提示: Claude-3p profile 尚未下发 tako MCP".to_string());
            }
            if read_switch_state(paths).prefer_chinese_ui || Self::is_chinese_configured(paths) {
                notes.push("中文增强: 已启用 (locale=zh-Hans / Claude Code language=Simplified Chinese)".to_string());
            }
        } else if Self::is_chinese_configured(paths) {
            notes.push("中文增强: 已启用 (locale=zh-Hans / Claude Code language=Simplified Chinese)".to_string());
        }

        DiagnosticResult {
            ok: issues.is_empty(),
            issues,
            notes,
        }
    }
}

#[cfg(target_os = "windows")]
fn ps_hidden(script: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn detect_windows_claude(paths: &AppPaths) -> Option<(Option<String>, String, String)> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
    let exe = local_app_data.join("AnthropicClaude").join("Claude.exe");
    if exe.exists() {
        let script = format!(
            "(Get-Item '{}').VersionInfo.ProductVersion",
            exe.to_string_lossy().replace('\'', "''")
        );
        let ver = ps_hidden(&script).filter(|s| !s.is_empty());
        return Some((ver, exe.to_string_lossy().to_string(), "exe".to_string()));
    }
    let msix = ps_hidden(
        "$p = Get-AppxPackage | Where-Object { $_.Name -eq 'Claude' -or $_.Name -eq 'AnthropicPBC.Claude' } | Select-Object -First 1; if ($p) { Write-Output ($p.Version + '|' + $p.InstallLocation) }",
    )?;
    if msix.is_empty() {
        return None;
    }
    let mut parts = msix.splitn(2, '|');
    let ver = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let loc = parts.next().map(str::trim).unwrap_or("").to_string();
    Some((ver, loc, "msix".to_string()))
}

#[cfg(target_os = "windows")]
fn quit_windows_claude() -> Result<(), String> {
    let _ = ps_hidden("Stop-Process -Name 'Claude' -Force -ErrorAction SilentlyContinue");
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_claude(paths: &AppPaths) -> Result<(), String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
    let exe = local_app_data.join("AnthropicClaude").join("Claude.exe");
    if exe.exists() {
        std::process::Command::new(&exe)
            .spawn()
            .map_err(|e| format!("启动 Claude 失败: {e}"))?;
        return Ok(());
    }
    let app_id = ps_hidden(
        "(Get-StartApps | Where-Object { $_.AppID -like '*Claude*' } | Select-Object -First 1).AppID",
    )
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "Claude_pzs8sxrjxfjjc!Claude".to_string());
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("启动 Claude 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{read_switch_state, save_switch_auth, save_switch_state};
    use tempfile::TempDir;

    #[test]
    fn claude_dual_slot_1p_3p_isolation() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.claude_1p_dir).unwrap();
        fs::write(
            paths.claude_1p_dir.join("claude_desktop_config.json"),
            r#"{"mcpServers":{"existing":{"command":"foo"}}}"#,
        )
        .unwrap();
        fs::create_dir_all(paths.home.join(".claude")).unwrap();
        fs::write(
            ClaudeAdapter::claude_code_settings_path(&paths),
            r#"{"permissions":{"allow":["Bash(git status)"]},"model":"claude-sonnet-5-5"}"#,
        )
        .unwrap();

        save_switch_auth(&paths, Some("cr_claude_secret"), Some("2"), Some("oauth")).unwrap();
        let adapter = ClaudeAdapter;

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        assert!(adapter.read_mcp_enabled(&paths, AppMode::Tako));
        assert!(adapter.diagnose(&paths).ok);
        let p1 = read_json_object_or_empty(&paths.claude_1p_dir.join("claude_desktop_config.json")).unwrap();
        let servers = p1.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers.contains_key("existing"));
        assert!(!servers.contains_key("tako"));
        let prof = read_json_object_or_empty(&ClaudeAdapter::profile_3p_path(&paths)).unwrap();
        assert_eq!(
            prof.get("inferenceGatewayBaseUrl").and_then(Value::as_str),
            Some(CLAUDE_PROXY_BASE_URL)
        );
        assert_eq!(
            prof.get("inferenceGatewayAuthScheme").and_then(Value::as_str),
            Some("bearer")
        );
        assert_eq!(
            prof.get("inferenceCredentialKind").and_then(Value::as_str),
            Some("static")
        );
        assert_eq!(
            prof.get("deploymentDisplayName").and_then(Value::as_str),
            Some("Tako")
        );
        assert_eq!(
            prof.get("deploymentDisplaySubtitle").and_then(Value::as_str),
            Some("Tako AI Gateway")
        );
        assert_eq!(prof.get("mcpToolTimeoutSec").and_then(Value::as_u64), Some(300));
        assert_eq!(
            prof.get("isLocalDevMcpEnabled").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            prof.get("mcpPersistentAlwaysAllowEnabled")
                .and_then(Value::as_bool),
            Some(true)
        );
        let managed = prof
            .get("managedMcpServers")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(managed.len(), 1);
        assert_eq!(managed[0].get("name").and_then(Value::as_str), Some("tako"));
        assert_eq!(
            managed[0].get("transport").and_then(Value::as_str),
            Some("stdio")
        );
        assert!(managed[0]
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|c| !c.is_empty()));
        assert_eq!(
            managed[0].get("args").and_then(Value::as_array).map(Vec::len),
            Some(1)
        );
        // The 3p user config must not also define tako, or Desktop starts two instances.
        assert!(!has_tako_mcp_server(&ClaudeAdapter::mcp_3p_path(&paths)));
        assert_eq!(
            managed[0]
                .get("toolPolicy")
                .and_then(Value::as_object)
                .and_then(|tp| tp.get("*"))
                .and_then(Value::as_str),
            Some("allow")
        );
        assert_eq!(
            managed[0]
                .get("toolPolicy")
                .and_then(Value::as_object)
                .and_then(|tp| tp.get("tako_generate_image"))
                .and_then(Value::as_str),
            Some("allow")
        );
        let cc_tako = read_json_object_or_empty(&ClaudeAdapter::claude_code_settings_path(&paths)).unwrap();
        assert_eq!(cc_tako.get("model").and_then(Value::as_str), Some("claude-sonnet-5-5"));
        let cc_allow = cc_tako
            .get("permissions")
            .and_then(Value::as_object)
            .and_then(|p| p.get("allow"))
            .and_then(Value::as_array)
            .unwrap();
        assert!(cc_allow.iter().any(|v| v.as_str() == Some("Bash(git status)")));
        assert!(cc_allow.iter().any(|v| v.as_str() == Some("mcp__tako__*")));
        assert!(cc_allow.iter().any(|v| v.as_str() == Some("mcp__tako__tako_generate_image")));
        assert_eq!(
            prof.get("inferenceModelPricingEnabled").and_then(Value::as_bool),
            Some(true)
        );
        let logo = prof.get("deploymentLogo").and_then(Value::as_str).unwrap();
        assert!(logo.starts_with("data:image/png;base64,"));
        assert!(logo.len() <= 32768);
        assert!(!prof
            .get("organizationInstructions")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty());
        assert!(prof.get("defaultModel").is_none());
        let claude_cfg = read_json_object_or_empty(&ClaudeAdapter::claude_3p_app_config_path(&paths)).unwrap();
        assert_eq!(
            claude_cfg.get("locale").and_then(Value::as_str),
            Some("zh-Hans")
        );
        let meta = read_json_object_or_empty(&ClaudeAdapter::meta_3p_path(&paths)).unwrap();
        let entries = meta.get("entries").and_then(Value::as_array).unwrap();
        assert!(entries
            .iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID)));
        // 1p should no longer carry tako when 3p is active
        assert!(!has_tako_mcp_server(&paths.claude_1p_dir.join("claude_desktop_config.json")));

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        assert!(!ClaudeAdapter::profile_3p_path(&paths).exists());
        let meta_after = read_json_object_or_empty(&ClaudeAdapter::meta_3p_path(&paths)).unwrap();
        let entries_after = meta_after.get("entries").and_then(Value::as_array).unwrap();
        assert!(!entries_after
            .iter()
            .any(|e| e.get("id").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID)));
        let p1_after =
            read_json_object_or_empty(&paths.claude_1p_dir.join("claude_desktop_config.json")).unwrap();
        let servers_after = p1_after.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers_after.contains_key("existing"));
        assert!(!servers_after.contains_key("tako"));
        let cc_after = read_json_object_or_empty(&ClaudeAdapter::claude_code_settings_path(&paths)).unwrap();
        let cc_allow_after = cc_after
            .get("permissions")
            .and_then(Value::as_object)
            .and_then(|p| p.get("allow"))
            .and_then(Value::as_array)
            .unwrap();
        assert!(cc_allow_after.iter().any(|v| v.as_str() == Some("Bash(git status)")));
        assert!(!cc_allow_after.iter().any(|v| v.as_str() == Some("mcp__tako__*")));
        assert!(!cc_allow_after.iter().any(|v| v.as_str() == Some("mcp__tako__tako_generate_image")));
    }

    #[test]
    fn claude_chinese_i18n_preserves_existing_keys_and_restores_original_locale() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.claude_3p_dir).unwrap();
        fs::create_dir_all(&paths.claude_1p_dir).unwrap();
        fs::create_dir_all(paths.home.join(".claude")).unwrap();

        fs::write(
            ClaudeAdapter::claude_3p_app_config_path(&paths),
            r#"{"darkMode":"dark","locale":"en-US"}"#,
        )
        .unwrap();
        fs::write(
            ClaudeAdapter::claude_1p_app_config_path(&paths),
            r#"{"scale":1.25}"#,
        )
        .unwrap();
        fs::write(
            ClaudeAdapter::claude_code_settings_path(&paths),
            r#"{"language":"Japanese","model":"claude-opus-5-5","permissions":{"allow":["Bash(ls)"]}}"#,
        )
        .unwrap();

        save_switch_auth(&paths, Some("cr_claude_secret"), Some("2"), Some("oauth")).unwrap();
        let adapter = ClaudeAdapter;

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        // Idempotent second apply
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let cfg_3p = read_json_object_or_empty(&ClaudeAdapter::claude_3p_app_config_path(&paths)).unwrap();
        assert_eq!(cfg_3p.get("locale").and_then(Value::as_str), Some("zh-Hans"));
        assert_eq!(cfg_3p.get("darkMode").and_then(Value::as_str), Some("dark"));

        let cfg_1p = read_json_object_or_empty(&ClaudeAdapter::claude_1p_app_config_path(&paths)).unwrap();
        assert_eq!(cfg_1p.get("locale").and_then(Value::as_str), Some("zh-Hans"));
        assert_eq!(cfg_1p.get("scale").and_then(Value::as_f64), Some(1.25));

        let cc_tako = read_json_object_or_empty(&ClaudeAdapter::claude_code_settings_path(&paths)).unwrap();
        assert_eq!(
            cc_tako.get("language").and_then(Value::as_str),
            Some("Simplified Chinese")
        );
        assert_eq!(
            cc_tako.get("model").and_then(Value::as_str),
            Some("claude-opus-5-5")
        );
        let cc_allow = cc_tako
            .get("permissions")
            .and_then(Value::as_object)
            .and_then(|p| p.get("allow"))
            .and_then(Value::as_array)
            .unwrap();
        assert!(cc_allow.iter().any(|v| v.as_str() == Some("Bash(ls)")));
        assert!(cc_allow.iter().any(|v| v.as_str() == Some("mcp__tako__*")));

        assert!(adapter
            .diagnose(&paths)
            .notes
            .iter()
            .any(|n| n.contains("中文增强: 已启用")));

        // Turn off prefer_chinese_ui while staying in Tako mode
        let mut st = read_switch_state(&paths);
        st.prefer_chinese_ui = false;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let cfg_3p_off = read_json_object_or_empty(&ClaudeAdapter::claude_3p_app_config_path(&paths)).unwrap();
        assert_eq!(cfg_3p_off.get("locale").and_then(Value::as_str), Some("en-US"));
        assert_eq!(cfg_3p_off.get("darkMode").and_then(Value::as_str), Some("dark"));

        let cfg_1p_off = read_json_object_or_empty(&ClaudeAdapter::claude_1p_app_config_path(&paths)).unwrap();
        assert!(cfg_1p_off.get("locale").is_none());
        assert_eq!(cfg_1p_off.get("scale").and_then(Value::as_f64), Some(1.25));

        let cc_off = read_json_object_or_empty(&ClaudeAdapter::claude_code_settings_path(&paths)).unwrap();
        assert_eq!(cc_off.get("language").and_then(Value::as_str), Some("Japanese"));
        assert!(!adapter
            .diagnose(&paths)
            .notes
            .iter()
            .any(|n| n.contains("中文增强: 已启用")));

        // Re-enable then switch to Official -> full restoration
        st.prefer_chinese_ui = true;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        adapter.apply_mode(&paths, AppMode::Official).unwrap();

        let cfg_3p_final = read_json_object_or_empty(&ClaudeAdapter::claude_3p_app_config_path(&paths)).unwrap();
        assert_eq!(cfg_3p_final.get("locale").and_then(Value::as_str), Some("en-US"));
        assert_eq!(cfg_3p_final.get("darkMode").and_then(Value::as_str), Some("dark"));

        let cfg_1p_final = read_json_object_or_empty(&ClaudeAdapter::claude_1p_app_config_path(&paths)).unwrap();
        assert!(cfg_1p_final.get("locale").is_none());
        assert_eq!(cfg_1p_final.get("scale").and_then(Value::as_f64), Some(1.25));

        let cc_final = read_json_object_or_empty(&ClaudeAdapter::claude_code_settings_path(&paths)).unwrap();
        assert_eq!(cc_final.get("language").and_then(Value::as_str), Some("Japanese"));
        let cc_final_allow = cc_final
            .get("permissions")
            .and_then(Value::as_object)
            .and_then(|p| p.get("allow"))
            .and_then(Value::as_array)
            .unwrap();
        assert!(cc_final_allow.iter().any(|v| v.as_str() == Some("Bash(ls)")));
        assert!(!cc_final_allow.iter().any(|v| v.as_str() == Some("mcp__tako__*")));
    }

    #[test]
    fn claude_chinese_i18n_cleans_up_absent_files_on_official() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        save_switch_auth(&paths, Some("cr_claude_secret"), Some("2"), Some("oauth")).unwrap();
        let adapter = ClaudeAdapter;

        let cfg_3p = ClaudeAdapter::claude_3p_app_config_path(&paths);
        let cfg_1p = ClaudeAdapter::claude_1p_app_config_path(&paths);
        let cc_settings = ClaudeAdapter::claude_code_settings_path(&paths);
        assert!(!cfg_3p.exists());
        assert!(!cfg_1p.exists());
        assert!(!cc_settings.exists());

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert!(cfg_3p.exists());
        assert!(cfg_1p.exists());
        assert!(cc_settings.exists());

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert!(!cfg_3p.exists());
        assert!(!cfg_1p.exists());
        assert!(!cc_settings.exists());
    }
}
