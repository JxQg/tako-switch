use crate::adapters::{
    read_json_object_or_empty, sync_managed_chinese_markdown, AppId, AppMode, DesktopAppAdapter,
    DiagnosticResult, InstallInfo, EDGEONE_MCP_NPX_ARGS, EDGEONE_MCP_SERVER_KEY,
};
use crate::agy_proxy::{
    ensure_gateway_service, is_proxy_healthy, stop_gateway_service_if_unused,
    CODEX_PROXY_BASE_URL, CODEX_PROXY_CATALOG_URL, LOCAL_GATEWAY_PORT, LOCAL_GATEWAY_URL,
};
use crate::config::{
    backup_file_once, read_backup, read_main_api_key, read_switch_state, write_atomic, AppPaths,
    McpToolsConfig,
};
use serde_json::Value as JsonValue;
use std::fs;
use std::path::PathBuf;
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version, sync_macos_apple_languages,
};

pub struct CodexAdapter;

#[cfg(target_os = "macos")]
const CODEX_MACOS_BUNDLE_ID: &str = "com.openai.codex";

const CODEX_CHINESE_AGENTS_BODY: &str = "\
- 默认使用简体中文与用户沟通，包括最终回复、代码解释、变更摘要与错误分析（除非用户明确要求使用其他语言）。\n\
- 保持代码标识符、文件路径、命令行、配置键名与日志原文的原始格式，不强行翻译专有名词。";

impl CodexAdapter {
    fn config_path(paths: &AppPaths) -> PathBuf {
        paths.codex_dir.join("config.toml")
    }

    fn auth_path(paths: &AppPaths) -> PathBuf {
        paths.codex_dir.join("auth.json")
    }

    pub fn agents_md_path(paths: &AppPaths) -> PathBuf {
        paths.codex_dir.join("AGENTS.md")
    }

    pub fn is_chinese_configured(paths: &AppPaths) -> bool {
        let has_locale = Self::read_doc(paths)
            .ok()
            .and_then(|doc| {
                doc.get("desktop")
                    .and_then(|i| i.as_table_like())
                    .and_then(|t| t.get("localeOverride"))
                    .and_then(|v| v.as_str())
                    .map(|s| s == "zh-CN")
            })
            .unwrap_or(false);
        has_locale && crate::adapters::has_managed_chinese_block(&Self::agents_md_path(paths))
    }

    pub fn sync_standalone_chinese_locale(paths: &AppPaths, enable: bool) -> Result<(), String> {
        let config_path = Self::config_path(paths);
        if enable || config_path.exists() {
            backup_file_once(paths, "codex", "config.toml", &config_path)?;
            let mut doc = Self::read_doc(paths)?;
            if enable {
                Self::set_desktop_locale_zh_cn(&mut doc);
                write_atomic(&config_path, doc.to_string().as_bytes(), false)?;
            } else {
                Self::restore_backup_desktop_locale(&mut doc, paths);
                let absent_marker = paths.backups_dir("codex").join("config.toml.absent");
                if doc.as_table().is_empty() && absent_marker.exists() && config_path.exists() {
                    fs::remove_file(&config_path)
                        .map_err(|e| format!("移除临时 config.toml 失败: {e}"))?;
                } else {
                    write_atomic(&config_path, doc.to_string().as_bytes(), false)?;
                }
            }
        }
        sync_managed_chinese_markdown(
            paths,
            "codex",
            "AGENTS.md",
            &Self::agents_md_path(paths),
            enable,
            CODEX_CHINESE_AGENTS_BODY,
        )?;
        #[cfg(target_os = "macos")]
        sync_macos_apple_languages(paths, CODEX_MACOS_BUNDLE_ID, enable);
        Ok(())
    }

    fn is_oauth_auth_json(bytes: &[u8]) -> bool {
        let Ok(val) = serde_json::from_slice::<JsonValue>(bytes) else {
            return false;
        };
        let Some(obj) = val.as_object() else {
            return false;
        };
        if obj.get("auth_mode").and_then(JsonValue::as_str) == Some("chatgpt") {
            return true;
        }
        obj.get("tokens")
            .and_then(JsonValue::as_object)
            .map(|t| !t.is_empty())
            .unwrap_or(false)
    }

    fn stash_official_auth(paths: &AppPaths) -> Result<(), String> {
        let auth_path = Self::auth_path(paths);
        if auth_path.exists() {
            let bytes = fs::read(&auth_path)
                .map_err(|e| format!("读取 {} 失败: {e}", auth_path.display()))?;
            if Self::is_oauth_auth_json(&bytes) {
                let bdir = paths.backups_dir("codex");
                let absent = bdir.join("auth.json.absent");
                if absent.exists() {
                    let _ = fs::remove_file(absent);
                }
                return write_atomic(&bdir.join("auth.json"), &bytes, true);
            }
        }
        backup_file_once(paths, "codex", "auth.json", &auth_path)
    }

    fn restore_official_auth(paths: &AppPaths) -> Result<(), String> {
        let auth_path = Self::auth_path(paths);
        if auth_path.exists() {
            if let Ok(bytes) = fs::read(&auth_path) {
                if Self::is_oauth_auth_json(&bytes) {
                    return Ok(());
                }
            }
        }
        if let Some(bytes) = read_backup(paths, "codex", "auth.json") {
            return write_atomic(&auth_path, &bytes, true);
        }
        let absent_marker = paths.backups_dir("codex").join("auth.json.absent");
        if absent_marker.exists() && auth_path.exists() {
            fs::remove_file(&auth_path)
                .map_err(|e| format!("移除临时 {} 失败: {e}", auth_path.display()))?;
        }
        Ok(())
    }

    fn read_doc(paths: &AppPaths) -> Result<DocumentMut, String> {
        let path = Self::config_path(paths);
        if !path.exists() {
            return Ok(DocumentMut::new());
        }
        let raw = fs::read_to_string(&path)
            .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(DocumentMut::new());
        }
        raw.parse::<DocumentMut>()
            .map_err(|e| format!("解析 {} 失败: {e}", path.display()))
    }

    fn restore_backup_provider(doc: &mut DocumentMut, paths: &AppPaths) {
        let backup_doc = read_backup(paths, "codex", "config.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<DocumentMut>().ok());

        if let Some(bdoc) = &backup_doc {
            if let Some(orig_provider) = bdoc.get("model_provider").and_then(|v| v.as_str()) {
                if orig_provider != "tako" {
                    doc["model_provider"] = toml_edit::value(orig_provider);
                } else {
                    doc.remove("model_provider");
                }
            } else {
                doc.remove("model_provider");
            }
        } else {
            doc.remove("model_provider");
        }

        if let Some(providers) = doc.get_mut("model_providers").and_then(|i| i.as_table_like_mut()) {
            let had_orig_tako = backup_doc
                .as_ref()
                .and_then(|b| b.get("model_providers"))
                .and_then(|i| i.as_table_like())
                .and_then(|t| t.get("tako"))
                .cloned();
            if let Some(orig_tako) = had_orig_tako {
                providers.insert("tako", orig_tako);
            } else {
                providers.remove("tako");
            }
            if providers.is_empty() {
                doc.remove("model_providers");
            }
        }
    }

    fn set_desktop_locale_zh_cn(doc: &mut DocumentMut) {
        if doc
            .get("desktop")
            .and_then(|i| i.as_table_like())
            .is_none()
        {
            doc["desktop"] = Item::Table(Table::new());
        }
        if let Some(desktop) = doc.get_mut("desktop").and_then(|i| i.as_table_like_mut()) {
            desktop.insert("localeOverride", toml_edit::value("zh-CN"));
        }
    }

    fn restore_backup_desktop_locale(doc: &mut DocumentMut, paths: &AppPaths) {
        let backup_doc = read_backup(paths, "codex", "config.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse::<DocumentMut>().ok());

        let orig_locale = backup_doc
            .as_ref()
            .and_then(|b| b.get("desktop"))
            .and_then(|i| i.as_table_like())
            .and_then(|t| t.get("localeOverride"))
            .cloned();

        let had_nonempty_backup_desktop = backup_doc
            .as_ref()
            .and_then(|b| b.get("desktop"))
            .and_then(|i| i.as_table_like())
            .map(|t| !t.is_empty())
            .unwrap_or(false);

        if let Some(locale_item) = orig_locale {
            if doc
                .get("desktop")
                .and_then(|i| i.as_table_like())
                .is_none()
            {
                doc["desktop"] = Item::Table(Table::new());
            }
            if let Some(desktop) = doc.get_mut("desktop").and_then(|i| i.as_table_like_mut()) {
                desktop.insert("localeOverride", locale_item);
            }
        } else if let Some(desktop) = doc.get_mut("desktop").and_then(|i| i.as_table_like_mut()) {
            desktop.remove("localeOverride");
            if desktop.is_empty() && !had_nonempty_backup_desktop {
                doc.remove("desktop");
            }
        }
    }

    fn set_mcp_server(doc: &mut DocumentMut, paths: &AppPaths, enable: bool) {
        if enable {
            if !doc.contains_table("mcp_servers") {
                doc["mcp_servers"] = Item::Table(Table::new());
            }
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_like_mut()) {
                let mut entry = Table::new();
                entry["command"] = toml_edit::value(paths.mcp_server_exe().to_string_lossy().to_string());
                let mut args = Array::new();
                args.push("mcp-server");
                entry["args"] = TomlValue::Array(args).into();
                entry["tool_timeout_sec"] = toml_edit::value(300);
                entry["default_tools_approval_mode"] = toml_edit::value("approve");
                let mut tools_tbl = Table::new();
                for &tool_name in crate::adapters::TAKO_MCP_TOOL_NAMES {
                    let mut tool_cfg = Table::new();
                    tool_cfg["approval_mode"] = toml_edit::value("approve");
                    tools_tbl.insert(tool_name, Item::Table(tool_cfg));
                }
                entry["tools"] = Item::Table(tools_tbl);
                servers.insert("tako", Item::Table(entry));
            }
        } else if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_like_mut()) {
            servers.remove("tako");
            if servers.is_empty() {
                doc.remove("mcp_servers");
            }
        }
    }

    fn has_tako_mcp(doc: &DocumentMut) -> bool {
        doc.get("mcp_servers")
            .and_then(|i| i.as_table_like())
            .map(|t| t.contains_key("tako"))
            .unwrap_or(false)
    }

    fn set_edgeone_mcp_server(doc: &mut DocumentMut, mcp: &McpToolsConfig, enable: bool) {
        if enable {
            if !doc.contains_table("mcp_servers") {
                doc["mcp_servers"] = Item::Table(Table::new());
            }
            if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_like_mut()) {
                let mut entry = Table::new();
                entry["command"] = toml_edit::value("npx");
                let mut args = Array::new();
                for &arg in EDGEONE_MCP_NPX_ARGS {
                    args.push(arg);
                }
                entry["args"] = TomlValue::Array(args).into();
                let mut env_tbl = Table::new();
                if let Some(token) = mcp.normalized_edgeone_api_token() {
                    env_tbl["EDGEONE_PAGES_API_TOKEN"] = toml_edit::value(token);
                }
                if let Some(project) = mcp.normalized_edgeone_project_name() {
                    env_tbl["EDGEONE_PAGES_PROJECT_NAME"] = toml_edit::value(project);
                }
                if !env_tbl.is_empty() {
                    entry["env"] = Item::Table(env_tbl);
                }
                servers.insert(EDGEONE_MCP_SERVER_KEY, Item::Table(entry));
            }
        } else if let Some(servers) = doc.get_mut("mcp_servers").and_then(|i| i.as_table_like_mut()) {
            servers.remove(EDGEONE_MCP_SERVER_KEY);
            if servers.is_empty() {
                doc.remove("mcp_servers");
            }
        }
    }

    fn has_edgeone_mcp(doc: &DocumentMut) -> bool {
        doc.get("mcp_servers")
            .and_then(|i| i.as_table_like())
            .map(|t| t.contains_key(EDGEONE_MCP_SERVER_KEY))
            .unwrap_or(false)
    }
}

impl DesktopAppAdapter for CodexAdapter {
    fn id(&self) -> AppId {
        AppId::Codex
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) =
                find_macos_app(paths, &["ChatGPT.app", "Codex.app"], CODEX_MACOS_BUNDLE_ID)
            {
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
            if let Some((ver, loc)) = detect_windows_codex() {
                return InstallInfo {
                    installed: true,
                    version: ver,
                    install_path: loc,
                    install_kind: Some("appx".to_string()),
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
            is_macos_bundle_id_running(CODEX_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            super::windows_process::is_running(AppId::Codex)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            quit_macos_bundle_id(CODEX_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_process("Codex")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path =
                find_macos_app(paths, &["ChatGPT.app", "Codex.app"], CODEX_MACOS_BUNDLE_ID);
            launch_macos_app(app_path.as_deref(), CODEX_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_codex()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 Codex Desktop".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        let Ok(doc) = Self::read_doc(paths) else {
            return AppMode::Official;
        };
        let is_tako_provider = doc.get("model_provider").and_then(|v| v.as_str()) == Some("tako")
            && doc
                .get("model_providers")
                .and_then(|i| i.as_table_like())
                .map(|t| t.contains_key("tako"))
                .unwrap_or(false);
        if is_tako_provider {
            AppMode::Tako
        } else {
            AppMode::Official
        }
    }

    fn read_mcp_enabled(&self, paths: &AppPaths, _mode: AppMode) -> bool {
        Self::read_doc(paths)
            .map(|doc| Self::has_tako_mcp(&doc))
            .unwrap_or(false)
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let config_path = Self::config_path(paths);
        backup_file_once(paths, "codex", "config.toml", &config_path)?;
        let mut doc = Self::read_doc(paths)?;

        match mode {
            AppMode::Official => {
                Self::restore_official_auth(paths)?;
                Self::restore_backup_provider(&mut doc, paths);
                Self::restore_backup_desktop_locale(&mut doc, paths);
                Self::set_mcp_server(&mut doc, paths, false);
                Self::set_edgeone_mcp_server(&mut doc, &McpToolsConfig::default(), false);
                let absent_marker = paths.backups_dir("codex").join("config.toml.absent");
                if doc.as_table().is_empty() && absent_marker.exists() && config_path.exists() {
                    fs::remove_file(&config_path)
                        .map_err(|e| format!("移除临时 config.toml 失败: {e}"))?;
                } else {
                    write_atomic(&config_path, doc.to_string().as_bytes(), false)?;
                }
                sync_managed_chinese_markdown(
                    paths,
                    "codex",
                    "AGENTS.md",
                    &Self::agents_md_path(paths),
                    false,
                    "",
                )?;
                #[cfg(target_os = "macos")]
                sync_macos_apple_languages(paths, CODEX_MACOS_BUNDLE_ID, false);
                stop_gateway_service_if_unused(paths)
            }
            AppMode::Tako => {
                let api_key = read_main_api_key(paths)
                    .ok_or_else(|| "请先登录 Tako 账号或填入 Key，再切换到全 Tako 模式".to_string())?;
                let state = read_switch_state(paths);
                ensure_gateway_service(paths)?;

                doc["model_provider"] = toml_edit::value("tako");
                if !doc.contains_table("model_providers") {
                    doc["model_providers"] = Item::Table(Table::new());
                }
                let mut tako = Table::new();
                tako["name"] = toml_edit::value(state.normalized_gateway_name());
                tako["base_url"] = toml_edit::value(CODEX_PROXY_BASE_URL);
                tako["model_catalog_url"] = toml_edit::value(CODEX_PROXY_CATALOG_URL);
                tako["wire_api"] = toml_edit::value("responses");

                if state.prefer_chinese_ui {
                    Self::set_desktop_locale_zh_cn(&mut doc);
                    sync_managed_chinese_markdown(
                        paths,
                        "codex",
                        "AGENTS.md",
                        &Self::agents_md_path(paths),
                        true,
                        CODEX_CHINESE_AGENTS_BODY,
                    )?;
                    #[cfg(target_os = "macos")]
                    sync_macos_apple_languages(paths, CODEX_MACOS_BUNDLE_ID, true);
                } else {
                    Self::restore_backup_desktop_locale(&mut doc, paths);
                    sync_managed_chinese_markdown(
                        paths,
                        "codex",
                        "AGENTS.md",
                        &Self::agents_md_path(paths),
                        false,
                        "",
                    )?;
                    #[cfg(target_os = "macos")]
                    sync_macos_apple_languages(paths, CODEX_MACOS_BUNDLE_ID, false);
                }

                if state.codex_preserve_official_auth {
                    Self::restore_official_auth(paths)?;
                    tako["requires_openai_auth"] = toml_edit::value(false);
                    tako["experimental_bearer_token"] = toml_edit::value(api_key);
                } else {
                    Self::stash_official_auth(paths)?;
                    let auth_payload = serde_json::json!({
                        "OPENAI_API_KEY": api_key,
                    });
                    let auth_bytes = serde_json::to_vec_pretty(&auth_payload)
                        .map_err(|e| format!("序列化 Codex auth.json 失败: {e}"))?;
                    write_atomic(&Self::auth_path(paths), &auth_bytes, true)?;
                    tako["requires_openai_auth"] = toml_edit::value(true);
                }

                if let Some(providers) =
                    doc.get_mut("model_providers").and_then(|i| i.as_table_like_mut())
                {
                    providers.insert("tako", Item::Table(tako));
                }
                Self::set_mcp_server(&mut doc, paths, true);
                Self::set_edgeone_mcp_server(&mut doc, &state.mcp, state.mcp.web_page);
                let has_secret_in_toml = !state.codex_preserve_official_auth
                    || (state.mcp.web_page && state.mcp.normalized_edgeone_api_token().is_some());
                write_atomic(&config_path, doc.to_string().as_bytes(), has_secret_in_toml)
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let mut notes = Vec::new();
        let config_path = Self::config_path(paths);
        notes.push(format!("配置路径: {}", config_path.display()));

        match Self::read_doc(paths) {
            Err(err) => issues.push(err),
            Ok(doc) => {
                let mode = self.read_mode(paths);
                if mode == AppMode::Tako {
                    let tako_tbl = doc
                        .get("model_providers")
                        .and_then(|i| i.as_table_like())
                        .and_then(|t| t.get("tako"))
                        .and_then(|i| i.as_table_like());
                    match tako_tbl {
                        None => issues.push("缺少 [model_providers.tako] 段".to_string()),
                        Some(tbl) => {
                            let url = tbl.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
                            if !url.ends_with("/v1") {
                                issues.push(format!("Codex base_url 需以 /v1 结尾，当前为: {url}"));
                            } else if url.starts_with(LOCAL_GATEWAY_URL) && !paths.is_test {
                                if is_proxy_healthy(LOCAL_GATEWAY_PORT) {
                                    notes.push(format!("本地统一网关: 运行中 (127.0.0.1:{LOCAL_GATEWAY_PORT})"));
                                } else {
                                    issues.push(format!(
                                        "本地统一网关 (127.0.0.1:{LOCAL_GATEWAY_PORT}) 未运行，请重新点击 Tako 或重启 Tako Switch"
                                    ));
                                }
                            }
                            if let Some(catalog_url) =
                                tbl.get("model_catalog_url").and_then(|v| v.as_str())
                            {
                                notes.push(format!("模型目录: {catalog_url}"));
                            }
                            let wire = tbl.get("wire_api").and_then(|v| v.as_str()).unwrap_or("");
                            if wire != "responses" {
                                issues.push(format!("wire_api 应为 responses，当前为: {wire}"));
                            }
                            let requires_auth = tbl
                                .get("requires_openai_auth")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if requires_auth {
                                match read_json_object_or_empty(&Self::auth_path(paths)) {
                                    Err(e) => issues.push(e),
                                    Ok(auth_obj) => {
                                        let key = auth_obj
                                            .get("OPENAI_API_KEY")
                                            .and_then(JsonValue::as_str)
                                            .unwrap_or("");
                                        if key.trim().is_empty() {
                                            issues.push("Codex auth.json 缺少 OPENAI_API_KEY".to_string());
                                        }
                                    }
                                }
                            } else {
                                let token = tbl
                                    .get("experimental_bearer_token")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                if token.is_empty() {
                                    issues.push("experimental_bearer_token 为空".to_string());
                                }
                            }
                        }
                    }
                    if read_switch_state(paths).prefer_chinese_ui || Self::is_chinese_configured(paths) {
                        notes.push("中文增强: 已启用 (localeOverride=zh-CN / AGENTS.md 简体中文规则)".to_string());
                    }
                } else if Self::is_chinese_configured(paths) {
                    notes.push("中文增强: 已启用 (localeOverride=zh-CN / AGENTS.md 简体中文规则)".to_string());
                }
                if Self::has_edgeone_mcp(&doc) {
                    notes.push("网站上线 MCP (EdgeOne): 已启用 (tako-web-page)".to_string());
                }
                if Self::has_tako_mcp(&doc) {
                    if let Some(cmd) = doc
                        .get("mcp_servers")
                        .and_then(|i| i.as_table_like())
                        .and_then(|t| t.get("tako"))
                        .and_then(|i| i.as_table_like())
                        .and_then(|t| t.get("command"))
                        .and_then(|v| v.as_str())
                    {
                        if !PathBuf::from(cmd).exists() && !paths.is_test {
                            notes.push(format!("MCP 命令路径检查: {cmd}"));
                        }
                    }
                }
            }
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
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn detect_windows_codex() -> Option<(Option<String>, Option<String>)> {
    let raw = ps_hidden(
        "$p = Get-AppxPackage -Name 'OpenAI.Codex' | Select-Object -First 1; if ($p) { Write-Output ($p.Version + '|' + $p.InstallLocation) }",
    )?;
    if raw.is_empty() {
        return None;
    }
    let mut parts = raw.splitn(2, '|');
    let ver = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let loc = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    Some((ver, loc))
}

#[cfg(target_os = "windows")]
fn quit_windows_process(name: &str) -> Result<(), String> {
    let script = format!(
        "Stop-Process -Name '{name}' -Force -ErrorAction SilentlyContinue; \
         Get-CimInstance Win32_Process -Filter \"Name='ChatGPT.exe'\" -ErrorAction SilentlyContinue | \
         Where-Object {{ $_.ExecutablePath -like '*OpenAI.Codex*' }} | \
         ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"
    );
    let _ = ps_hidden(&script);
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_codex() -> Result<(), String> {
    let app_id = ps_hidden(
        "(Get-StartApps | Where-Object { $_.AppID -like 'OpenAI.Codex*' } | Select-Object -First 1).AppID",
    )
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "OpenAI.Codex_2p2nqsd0c76g0!App".to_string());
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("启动 Codex 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{read_switch_state, save_switch_auth, save_switch_state};
    use tempfile::TempDir;

    #[test]
    fn codex_roundtrip_preserves_unrelated_toml_and_auth_json() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.codex_dir).unwrap();
        fs::write(paths.codex_dir.join("auth.json"), r#"{"tokens":{"access":"untouched"}}"#)
            .unwrap();
        fs::write(
            paths.codex_dir.join("config.toml"),
            r#"# user header
model = "gpt-5.4"
model_provider = "openai"

[projects."/tmp/repo"]
trust_level = "trusted"
"#,
        )
        .unwrap();

        save_switch_auth(&paths, Some("cr_codex_secret"), Some("1"), Some("oauth")).unwrap();
        let adapter = CodexAdapter;

        // Default Route A: stashes official OAuth auth.json and writes OPENAI_API_KEY so Codex opens without login
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        let tako_raw = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(tako_raw.contains("model_provider = \"tako\""));
        assert!(tako_raw.contains(CODEX_PROXY_BASE_URL));
        assert!(tako_raw.contains(CODEX_PROXY_CATALOG_URL));
        assert!(tako_raw.contains("wire_api = \"responses\""));
        assert!(tako_raw.contains("requires_openai_auth = true"));
        assert!(tako_raw.contains("default_tools_approval_mode = \"approve\""));
        assert!(tako_raw.contains("mcp_servers.tako.tools.tako_generate_image"));
        assert!(tako_raw.contains("approval_mode = \"approve\""));
        assert!(!tako_raw.contains("experimental_bearer_token"));
        assert!(tako_raw.contains("# user header"));
        let live_auth = fs::read_to_string(paths.codex_dir.join("auth.json")).unwrap();
        assert!(live_auth.contains("cr_codex_secret"));
        assert!(adapter.diagnose(&paths).ok);

        // Switching back to Official restores stashed official auth.json
        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        assert!(!adapter.read_mcp_enabled(&paths, AppMode::Official));
        let off_raw = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(off_raw.contains("model_provider = \"openai\""));
        assert_eq!(
            fs::read_to_string(paths.codex_dir.join("auth.json")).unwrap(),
            r#"{"tokens":{"access":"untouched"}}"#
        );

        // Route B: codex_preserve_official_auth keeps auth.json untouched and uses experimental_bearer_token
        let mut st = read_switch_state(&paths);
        st.codex_preserve_official_auth = true;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let tako_b_raw = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(tako_b_raw.contains("requires_openai_auth = false"));
        assert!(tako_b_raw.contains("experimental_bearer_token = \"cr_codex_secret\""));
        assert_eq!(
            fs::read_to_string(paths.codex_dir.join("auth.json")).unwrap(),
            r#"{"tokens":{"access":"untouched"}}"#
        );
        assert!(adapter.diagnose(&paths).ok);
    }

    #[test]
    fn codex_chinese_i18n_preserves_existing_desktop_and_agents_md_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.codex_dir).unwrap();
        fs::write(
            paths.codex_dir.join("config.toml"),
            r#"model = "gpt-5.4"

[desktop]
reviewDelivery = "inline"
localeOverride = "ja-JP"
"#,
        )
        .unwrap();
        let user_agents = "# My Codex Rules\nAlways run cargo test.\n";
        fs::write(CodexAdapter::agents_md_path(&paths), user_agents).unwrap();

        save_switch_auth(&paths, Some("cr_codex_secret"), Some("1"), Some("oauth")).unwrap();
        let adapter = CodexAdapter;

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let tako_toml = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        let tako_doc = tako_toml.parse::<DocumentMut>().unwrap();
        let desktop_tbl = tako_doc.get("desktop").and_then(|i| i.as_table_like()).unwrap();
        assert_eq!(
            desktop_tbl.get("localeOverride").and_then(|v| v.as_str()),
            Some("zh-CN")
        );
        assert_eq!(
            desktop_tbl.get("reviewDelivery").and_then(|v| v.as_str()),
            Some("inline")
        );

        let agents_first = fs::read_to_string(CodexAdapter::agents_md_path(&paths)).unwrap();
        assert!(agents_first.contains("# My Codex Rules"));
        assert!(agents_first.contains("Always run cargo test."));
        assert!(agents_first.contains(crate::adapters::TAKO_CHINESE_BLOCK_START));
        assert!(agents_first.contains(crate::adapters::TAKO_CHINESE_BLOCK_END));
        assert!(agents_first.contains("简体中文"));

        assert!(adapter
            .diagnose(&paths)
            .notes
            .iter()
            .any(|n| n.contains("中文增强: 已启用")));

        // Applying Tako again must be idempotent
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let agents_second = fs::read_to_string(CodexAdapter::agents_md_path(&paths)).unwrap();
        assert_eq!(agents_first, agents_second);
        assert_eq!(
            agents_second
                .matches(crate::adapters::TAKO_CHINESE_BLOCK_START)
                .count(),
            1
        );

        // Disabling prefer_chinese_ui in Tako mode restores localeOverride and AGENTS.md
        let mut st = read_switch_state(&paths);
        st.prefer_chinese_ui = false;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let disabled_doc = fs::read_to_string(paths.codex_dir.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        let disabled_desktop = disabled_doc
            .get("desktop")
            .and_then(|i| i.as_table_like())
            .unwrap();
        assert_eq!(
            disabled_desktop.get("localeOverride").and_then(|v| v.as_str()),
            Some("ja-JP")
        );
        assert_eq!(
            disabled_desktop.get("reviewDelivery").and_then(|v| v.as_str()),
            Some("inline")
        );
        assert_eq!(
            fs::read_to_string(CodexAdapter::agents_md_path(&paths)).unwrap(),
            user_agents
        );
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

        let off_doc = fs::read_to_string(paths.codex_dir.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        let off_desktop = off_doc.get("desktop").and_then(|i| i.as_table_like()).unwrap();
        assert_eq!(
            off_desktop.get("localeOverride").and_then(|v| v.as_str()),
            Some("ja-JP")
        );
        assert_eq!(
            off_desktop.get("reviewDelivery").and_then(|v| v.as_str()),
            Some("inline")
        );
        assert_eq!(
            fs::read_to_string(CodexAdapter::agents_md_path(&paths)).unwrap(),
            user_agents
        );
    }

    #[test]
    fn codex_chinese_i18n_cleans_up_absent_files_on_official() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        save_switch_auth(&paths, Some("cr_codex_secret"), Some("1"), Some("oauth")).unwrap();
        let adapter = CodexAdapter;

        assert!(!CodexAdapter::config_path(&paths).exists());
        assert!(!CodexAdapter::agents_md_path(&paths).exists());

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert!(CodexAdapter::config_path(&paths).exists());
        assert!(CodexAdapter::agents_md_path(&paths).exists());

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert!(!CodexAdapter::config_path(&paths).exists());
        assert!(!CodexAdapter::agents_md_path(&paths).exists());
    }

    #[test]
    fn codex_standalone_chinese_locale_works_in_official_mode_and_restores_cleanly() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        assert!(!CodexAdapter::is_chinese_configured(&paths));

        CodexAdapter::sync_standalone_chinese_locale(&paths, true).unwrap();
        assert!(CodexAdapter::is_chinese_configured(&paths));
        assert_eq!(CodexAdapter.read_mode(&paths), AppMode::Official);

        CodexAdapter::sync_standalone_chinese_locale(&paths, false).unwrap();
        assert!(!CodexAdapter::is_chinese_configured(&paths));
        assert!(!CodexAdapter::config_path(&paths).exists());
        assert!(!CodexAdapter::agents_md_path(&paths).exists());
    }

    #[test]
    fn codex_edgeone_mcp_sync_enable_disable_and_official_preserves_other_servers() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.codex_dir).unwrap();
        fs::write(
            CodexAdapter::config_path(&paths),
            r#"model = "gpt-5.4"

[mcp_servers.custom]
command = "custom-mcp"
"#,
        )
        .unwrap();
        save_switch_auth(&paths, Some("cr_codex_secret"), Some("1"), Some("oauth")).unwrap();

        let mut st = read_switch_state(&paths);
        st.mcp.web_page = true;
        save_switch_state(&paths, &st).unwrap();

        let adapter = CodexAdapter;
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let doc = CodexAdapter::read_doc(&paths).unwrap();
        let servers = doc["mcp_servers"].as_table_like().unwrap();
        assert!(servers.contains_key("custom"));
        assert!(servers.contains_key("tako"));
        let eo = servers
            .get(EDGEONE_MCP_SERVER_KEY)
            .unwrap()
            .as_table_like()
            .unwrap();
        assert_eq!(eo.get("command").and_then(|v| v.as_str()), Some("npx"));
        assert!(eo.get("env").is_none());

        // Add token & project -> env table written
        st.mcp.edgeone_api_token = Some("eo_codex_tok".to_string());
        st.mcp.edgeone_project_name = Some("codex-site".to_string());
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let doc_tok = CodexAdapter::read_doc(&paths).unwrap();
        let eo_tok = doc_tok["mcp_servers"]
            .as_table_like()
            .unwrap()
            .get(EDGEONE_MCP_SERVER_KEY)
            .unwrap()
            .as_table_like()
            .unwrap();
        let env_tbl = eo_tok.get("env").unwrap().as_table_like().unwrap();
        assert_eq!(
            env_tbl
                .get("EDGEONE_PAGES_API_TOKEN")
                .and_then(|v| v.as_str()),
            Some("eo_codex_tok")
        );
        assert_eq!(
            env_tbl
                .get("EDGEONE_PAGES_PROJECT_NAME")
                .and_then(|v| v.as_str()),
            Some("codex-site")
        );

        // Disable web_page -> removes tako-web-page, keeps tako and custom
        st.mcp.web_page = false;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let doc_off_web = CodexAdapter::read_doc(&paths).unwrap();
        let servers_off_web = doc_off_web["mcp_servers"].as_table_like().unwrap();
        assert!(servers_off_web.contains_key("custom"));
        assert!(servers_off_web.contains_key("tako"));
        assert!(!servers_off_web.contains_key(EDGEONE_MCP_SERVER_KEY));

        // Re-enable then switch to Official -> removes both tako and tako-web-page, keeps custom
        st.mcp.web_page = true;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        adapter.apply_mode(&paths, AppMode::Official).unwrap();

        let doc_official = CodexAdapter::read_doc(&paths).unwrap();
        let servers_official = doc_official["mcp_servers"].as_table_like().unwrap();
        assert!(servers_official.contains_key("custom"));
        assert!(!servers_official.contains_key("tako"));
        assert!(!servers_official.contains_key(EDGEONE_MCP_SERVER_KEY));
    }
}
