use crate::adapters::{
    build_edgeone_mcp_json_entry, build_stdio_mcp_json_entry, has_edgeone_mcp_server,
    has_tako_mcp_server, merge_edgeone_mcp_server, merge_tako_mcp_server,
    read_json_object_or_empty, AppId, AppMode, DesktopAppAdapter, DiagnosticResult, InstallInfo,
    EDGEONE_MCP_SERVER_KEY,
};
use crate::agy_proxy::{
    ensure_gateway_service, is_proxy_healthy, load_workbuddy_model_catalog,
    stop_gateway_service_if_unused, LOCAL_GATEWAY_PORT, LOCAL_GATEWAY_URL,
    WORKBUDDY_PROXY_BASE_URL,
};
use crate::config::{
    backup_file_once, read_backup, read_main_api_key, read_switch_state, write_atomic, AppPaths,
    ClientModelFilter, ModelFilterMode,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version,
};

pub struct WorkBuddyAdapter;

#[cfg(target_os = "macos")]
const WORKBUDDY_MACOS_BUNDLE_ID: &str = "com.tencent.workbuddy.mac";
#[cfg(target_os = "macos")]
const WORKBUDDY_MACOS_FALLBACK_BUNDLE_ID: &str = "com.tencent.workbuddy.workbuddy";

/// Placeholder API key written into `~/.workbuddy/models.json`.
/// WorkBuddy validates non-empty printable ASCII `apiKey` (otherwise raises 4100),
/// while the local Tako gateway (`127.0.0.1:18419/workbuddy/v1`) injects the real
/// user API key on every upstream request so rotating keys never requires rewriting
/// WorkBuddy's local config.
pub const WORKBUDDY_LOCAL_API_KEY_PLACEHOLDER: &str = "tako-local-gateway";
pub const WORKBUDDY_CHAT_COMPLETIONS_URL: &str =
    "http://127.0.0.1:18419/workbuddy/v1/chat/completions";

#[derive(Debug, Clone)]
pub struct WorkBuddyModelEntrySpec {
    pub id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub supports_images: bool,
    pub supports_reasoning: bool,
    pub effort_levels: Vec<String>,
}

impl WorkBuddyAdapter {
    pub fn models_json_path(paths: &AppPaths) -> PathBuf {
        paths.workbuddy_dir.join("models.json")
    }

    pub fn mcp_json_path(paths: &AppPaths) -> PathBuf {
        paths.workbuddy_dir.join("mcp.json")
    }

    pub fn mcp_approvals_json_path(paths: &AppPaths) -> PathBuf {
        paths.workbuddy_dir.join("mcp-approvals.json")
    }

    pub fn settings_json_path(paths: &AppPaths) -> PathBuf {
        paths.workbuddy_dir.join("settings.json")
    }

    pub fn is_tako_managed_model(val: &Value) -> bool {
        let Some(obj) = val.as_object() else {
            return false;
        };
        if obj.get("takoManaged").and_then(Value::as_bool) == Some(true) {
            return true;
        }
        obj.get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|u| {
                u.starts_with(WORKBUDDY_PROXY_BASE_URL)
                    || u.starts_with("http://localhost:18419/workbuddy")
            })
    }

    fn count_tako_models(paths: &AppPaths) -> usize {
        let path = Self::models_json_path(paths);
        let Ok(raw) = fs::read_to_string(path) else {
            return 0;
        };
        let Ok(val) = serde_json::from_str::<Value>(&raw) else {
            return 0;
        };
        let arr = if let Some(list) = val.as_array() {
            list
        } else if let Some(list) = val.get("models").and_then(Value::as_array) {
            list
        } else {
            return 0;
        };
        arr.iter().filter(|m| Self::is_tako_managed_model(m)).count()
    }

    fn default_fallback_specs(filter: &ClientModelFilter) -> Vec<WorkBuddyModelEntrySpec> {
        let ids: Vec<String> = match filter.mode {
            ModelFilterMode::Custom if !filter.enabled_ids.is_empty() => filter.enabled_ids.clone(),
            _ => ClientModelFilter::recommended_ids_for(AppId::WorkBuddy)
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        };

        ids.into_iter()
            .map(|id| build_fallback_spec_from_id(&id))
            .collect()
    }

    fn resolve_target_models(paths: &AppPaths) -> Vec<WorkBuddyModelEntrySpec> {
        let state = read_switch_state(paths);
        let filter = &state.model_filters.workbuddy;
        if let Some(cached) = load_workbuddy_model_catalog(paths, filter) {
            if !cached.is_empty() {
                return cached;
            }
        }
        Self::default_fallback_specs(filter)
    }

    fn build_workbuddy_model_value(
        gateway_name: &str,
        spec: &WorkBuddyModelEntrySpec,
    ) -> Value {
        let prefix = gateway_name.trim();
        let display = if prefix.is_empty() {
            spec.display_name.clone()
        } else {
            format!("{prefix} / {}", spec.display_name)
        };
        let max_input = spec.context_window.unwrap_or(200_000);
        let max_output = infer_max_output_tokens(&spec.id, max_input);

        let mut obj = Map::new();
        obj.insert("id".to_string(), Value::String(spec.id.clone()));
        obj.insert("name".to_string(), Value::String(display));
        obj.insert("vendor".to_string(), Value::String("Custom".to_string()));
        obj.insert(
            "url".to_string(),
            Value::String(WORKBUDDY_CHAT_COMPLETIONS_URL.to_string()),
        );
        obj.insert(
            "apiKey".to_string(),
            Value::String(WORKBUDDY_LOCAL_API_KEY_PLACEHOLDER.to_string()),
        );
        obj.insert("supportsToolCall".to_string(), Value::Bool(true));
        obj.insert(
            "supportsImages".to_string(),
            Value::Bool(spec.supports_images),
        );
        obj.insert(
            "supportsReasoning".to_string(),
            Value::Bool(spec.supports_reasoning),
        );
        obj.insert("maxInputTokens".to_string(), json!(max_input));
        obj.insert("maxOutputTokens".to_string(), json!(max_output));
        obj.insert("takoManaged".to_string(), Value::Bool(true));

        if spec.supports_reasoning {
            const EFFORT_ORDER: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];
            let mut efforts: Vec<String> = Vec::new();
            for &lvl in EFFORT_ORDER {
                if spec
                    .effort_levels
                    .iter()
                    .any(|e| e.eq_ignore_ascii_case(lvl))
                {
                    efforts.push(lvl.to_string());
                }
            }
            if !efforts.is_empty() {
                let default_effort = if efforts.iter().any(|e| e == "medium") {
                    "medium".to_string()
                } else if efforts.iter().any(|e| e == "high") {
                    "high".to_string()
                } else {
                    efforts[0].clone()
                };
                // WorkBuddy's `convertInternalModelToOptions` + `dedupeInputModelOptions`
                // defaults thinking to OFF unless `onlyReasoning: true` (matching built-in
                // reasoning models in `acc-product-config-v3`). Setting `onlyReasoning: true`
                // + explicit `supportedEfforts` makes the effort selector active by default.
                obj.insert("onlyReasoning".to_string(), Value::Bool(true));
                obj.insert(
                    "reasoning".to_string(),
                    json!({
                        "defaultEffort": default_effort,
                        "supportedEfforts": efforts,
                        "canDisableThinking": false,
                        "summary": "auto",
                    }),
                );

                // WorkBuddy CLI's `LegacyXhighFallbackRule` & `CustomModelFieldCleanupPlugin`
                // silently collapse `xhigh`/`max` to `high` whenever `thinkingLevelMap` is
                // absent, and `getSupportedThinkingLevels` treats omitted `off`/`minimal`/`low`
                // as enabled. Provide an explicit map for all 7 levels so every supported
                // effort passes straight through to the local Tako gateway untouched.
                let mut level_map = Map::new();
                for lvl in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
                    if efforts.iter().any(|e| e == lvl) {
                        level_map.insert(lvl.to_string(), Value::String(lvl.to_string()));
                    } else {
                        level_map.insert(lvl.to_string(), Value::Null);
                    }
                }
                obj.insert("thinkingLevelMap".to_string(), Value::Object(level_map));
            }
        }

        Value::Object(obj)
    }

    fn read_models_root(path: &PathBuf) -> Result<Map<String, Value>, String> {
        if !path.exists() {
            return Ok(Map::new());
        }
        let raw = fs::read_to_string(path)
            .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Map::new());
        }
        let val: Value = serde_json::from_str(&raw)
            .map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
        match val {
            Value::Object(map) => Ok(map),
            Value::Array(arr) => {
                let mut map = Map::new();
                map.insert("models".to_string(), Value::Array(arr));
                Ok(map)
            }
            _ => Err(format!("{} 必须为 JSON object 或 array", path.display())),
        }
    }

    fn sync_default_model_in_settings(
        paths: &AppPaths,
        tako_model_id: Option<&str>,
    ) -> Result<(), String> {
        let settings_path = Self::settings_json_path(paths);
        match tako_model_id {
            Some(model_id) => {
                backup_file_once(paths, "workbuddy", "settings.json", &settings_path)?;
                let mut root = if settings_path.exists() {
                    read_json_object_or_empty(&settings_path)?
                } else {
                    Map::new()
                };
                root.insert("model".to_string(), Value::String(model_id.to_string()));
                let bytes = serde_json::to_vec_pretty(&Value::Object(root))
                    .map_err(|e| format!("序列化 {} 失败: {e}", settings_path.display()))?;
                write_atomic(&settings_path, &bytes, false)
            }
            None => {
                let absent_marker = paths.backups_dir("workbuddy").join("settings.json.absent");
                if let Some(backup_bytes) = read_backup(paths, "workbuddy", "settings.json") {
                    let backup_model = serde_json::from_slice::<Value>(&backup_bytes)
                        .ok()
                        .and_then(|v| v.get("model").cloned());
                    if settings_path.exists() {
                        let mut root = read_json_object_or_empty(&settings_path)?;
                        match backup_model {
                            Some(orig) => {
                                root.insert("model".to_string(), orig);
                            }
                            None => {
                                root.remove("model");
                            }
                        }
                        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
                            .map_err(|e| format!("序列化 {} 失败: {e}", settings_path.display()))?;
                        write_atomic(&settings_path, &bytes, false)?;
                    }
                } else if settings_path.exists() {
                    let mut root = read_json_object_or_empty(&settings_path)?;
                    root.remove("model");
                    if root.is_empty() && absent_marker.exists() {
                        let _ = fs::remove_file(&settings_path);
                    } else {
                        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
                            .map_err(|e| format!("序列化 {} 失败: {e}", settings_path.display()))?;
                        write_atomic(&settings_path, &bytes, false)?;
                    }
                }
                Ok(())
            }
        }
    }
}

fn scalar_json_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
        _ => None,
    }
}

fn sort_js_utf16(items: &mut [String]) {
    items.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
}

fn safe_url_origin(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(u) => {
            let origin = u.origin().ascii_serialization();
            if origin == "null" {
                raw.to_string()
            } else {
                origin
            }
        }
        Err(_) => raw.to_string(),
    }
}

/// Computes WorkBuddy's exact `trustMaterial(transport)` string from a raw `mcpServers.<name>`
/// JSON object (matching `main/connector-harness-marketplace.js::toTransport` +
/// `main/server.js::trustMaterial`).
pub fn compute_mcp_trust_material(entry: &Value) -> Option<String> {
    let obj = entry.as_object()?;
    if let Some(cmd) = obj
        .get("command")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let mut args: Vec<String> = obj
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(scalar_json_to_string).collect())
            .unwrap_or_default();
        sort_js_utf16(&mut args);

        let mut env_keys: Vec<String> = obj
            .get("env")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter(|(_, v)| scalar_json_to_string(v).is_some())
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        sort_js_utf16(&mut env_keys);

        let cwd = obj
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());

        return Some(match cwd {
            Some(c) => format!("{}|{}|{}|{}", cmd, args.join(","), env_keys.join(","), c),
            None => format!("{}|{}|{}", cmd, args.join(","), env_keys.join(",")),
        });
    }

    if let Some(url) = obj
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(safe_url_origin(url));
    }

    None
}

pub fn compute_mcp_trust_hash(entry: &Value) -> Option<String> {
    let material = compute_mcp_trust_material(entry)?;
    let mut hasher = Sha256::new();
    hasher.update(material.as_bytes());
    Some(hex::encode(hasher.finalize()))
}

pub fn approval_key_for_mcp_entry(server_name: &str, entry: &Value) -> Option<String> {
    let hash = compute_mcp_trust_hash(entry)?;
    Some(format!("{hash}::{server_name}"))
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(1)
        .max(1)
}

/// Pre-approves a user MCP server in `~/.workbuddy/mcp-approvals.json` so WorkBuddy loads
/// the server immediately on launch without prompting for manual trust confirmation.
pub fn approve_workbuddy_mcp_server(
    paths: &AppPaths,
    server_name: &str,
    entry: &Value,
) -> Result<(), String> {
    let Some(key) = approval_key_for_mcp_entry(server_name, entry) else {
        return Ok(());
    };
    let path = WorkBuddyAdapter::mcp_approvals_json_path(paths);
    let mut root = read_json_object_or_empty(&path).unwrap_or_default();
    let suffix = format!("::{server_name}");
    let existing_ts = root
        .get(&key)
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .filter(|&n| n > 0);
    root.retain(|k, _| k == &key || !k.ends_with(&suffix));
    root.insert(key, json!(existing_ts.unwrap_or_else(current_epoch_ms)));
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(&path, &bytes, false)
}

/// Removes all approval entries for `server_name` from `~/.workbuddy/mcp-approvals.json`.
pub fn revoke_workbuddy_mcp_server(
    paths: &AppPaths,
    server_name: &str,
    remove_if_empty_and_absent: bool,
) -> Result<(), String> {
    let path = WorkBuddyAdapter::mcp_approvals_json_path(paths);
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_json_object_or_empty(&path).unwrap_or_default();
    let suffix = format!("::{server_name}");
    let before = root.len();
    root.retain(|k, _| !k.ends_with(&suffix));
    let removed = root.len() != before;
    let absent_marker = paths
        .backups_dir("workbuddy")
        .join("mcp-approvals.json.absent");
    if root.is_empty() && remove_if_empty_and_absent && absent_marker.exists() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    if !removed {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(&path, &bytes, false)
}

pub fn is_workbuddy_mcp_approved(paths: &AppPaths, server_name: &str, entry: &Value) -> bool {
    let Some(key) = approval_key_for_mcp_entry(server_name, entry) else {
        return false;
    };
    let path = WorkBuddyAdapter::mcp_approvals_json_path(paths);
    read_json_object_or_empty(&path)
        .ok()
        .and_then(|m| m.get(&key).cloned())
        .is_some_and(|v| v.as_f64().is_some_and(f64::is_finite))
}

fn infer_max_output_tokens(id: &str, max_input: u64) -> u64 {
    let lower = id.to_ascii_lowercase();
    if lower.starts_with("claude-opus")
        || lower.starts_with("claude-sonnet")
        || lower.starts_with("gpt-6")
        || lower.starts_with("gemini-")
        || lower.starts_with("deepseek-")
    {
        64_000
    } else if max_input >= 128_000 {
        32_000
    } else {
        16_384
    }
}

fn build_fallback_spec_from_id(id: &str) -> WorkBuddyModelEntrySpec {
    let lower = id.to_ascii_lowercase();
    let display_name = match lower.as_str() {
        "claude-opus-5-5" => "Claude Opus 5.5".to_string(),
        "claude-sonnet-5-5" => "Claude Sonnet 5.5".to_string(),
        "claude-fable-5-2" => "Claude Fable 5.2".to_string(),
        "gpt-6.2-sol" => "GPT-6.2 Sol".to_string(),
        "gpt-6.2-astra" => "GPT-6.2 Astra".to_string(),
        "deepseek-v4.1-flash" => "DeepSeek V4.1 Flash".to_string(),
        "swe-2" => "SWE-2".to_string(),
        "swe-2-flash" => "SWE-2 Flash".to_string(),
        "swe-2-1" => "SWE-2.1".to_string(),
        "gemini-4-neon" => "Gemini 4 Neon".to_string(),
        "gemini-4-hydrogen" => "Gemini 4 Hydrogen".to_string(),
        "gemini-4-argon" => "Gemini 4 Argon".to_string(),
        "gemini-4-carbon" => "Gemini 4 Carbon".to_string(),
        _ => id.to_string(),
    };
    let context_window = if lower.contains("gpt-6")
        || lower.contains("gemini-")
        || lower.contains("deepseek-v4")
    {
        Some(1_000_000)
    } else {
        Some(200_000)
    };
    let supports_images = lower.starts_with("claude-")
        || lower.starts_with("gpt-")
        || lower.starts_with("gemini-")
        || lower.contains("kimi")
        || lower.contains("glm-5");

    let effort_levels: Vec<String> = if lower.starts_with("gpt-6")
        || lower.starts_with("gpt-5")
        || lower == "swe-2-1"
    {
        vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
            "max".to_string(),
        ]
    } else if lower == "swe-2" || lower == "swe-2-flash" {
        vec!["medium".to_string(), "high".to_string(), "max".to_string()]
    } else if lower == "gemini-4-neon" || lower == "gemini-4-hydrogen" {
        vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
            "xhigh".to_string(),
        ]
    } else if lower.starts_with("claude-opus")
        || lower.starts_with("claude-sonnet")
        || lower.starts_with("deepseek-v4")
        || lower.starts_with("deepseek-r")
        || lower.starts_with("gemini-4")
        || lower.starts_with("gemini-3")
        || lower.starts_with("glm-5")
        || lower.starts_with("kimi-k3")
        || lower.starts_with("grok-4")
        || lower.starts_with("minimax-m")
        || lower.starts_with("mimo-v2")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("swe-")
    {
        vec!["low".to_string(), "medium".to_string(), "high".to_string()]
    } else {
        Vec::new()
    };
    let supports_reasoning = !effort_levels.is_empty();

    WorkBuddyModelEntrySpec {
        id: id.to_string(),
        display_name,
        context_window,
        supports_images,
        supports_reasoning,
        effort_levels,
    }
}

impl DesktopAppAdapter for WorkBuddyAdapter {
    fn id(&self) -> AppId {
        AppId::WorkBuddy
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) =
                find_macos_app(paths, &["WorkBuddy.app"], WORKBUDDY_MACOS_BUNDLE_ID)
                    .or_else(|| {
                        find_macos_app(
                            paths,
                            &["WorkBuddy.app"],
                            WORKBUDDY_MACOS_FALLBACK_BUNDLE_ID,
                        )
                    })
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
            if let Some((ver, path, kind)) = detect_windows_workbuddy(paths) {
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
            is_macos_bundle_id_running(WORKBUDDY_MACOS_BUNDLE_ID)
                || is_macos_bundle_id_running(WORKBUDDY_MACOS_FALLBACK_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            super::windows_process::is_running(AppId::WorkBuddy)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            let _ = quit_macos_bundle_id(WORKBUDDY_MACOS_FALLBACK_BUNDLE_ID);
            quit_macos_bundle_id(WORKBUDDY_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_workbuddy()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path = find_macos_app(paths, &["WorkBuddy.app"], WORKBUDDY_MACOS_BUNDLE_ID)
                .or_else(|| {
                    find_macos_app(
                        paths,
                        &["WorkBuddy.app"],
                        WORKBUDDY_MACOS_FALLBACK_BUNDLE_ID,
                    )
                });
            launch_macos_app(app_path.as_deref(), WORKBUDDY_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_workbuddy(paths)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 WorkBuddy".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        if Self::count_tako_models(paths) > 0 {
            AppMode::Tako
        } else {
            AppMode::Official
        }
    }

    fn read_mcp_enabled(&self, paths: &AppPaths, _mode: AppMode) -> bool {
        has_tako_mcp_server(&Self::mcp_json_path(paths))
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let models_path = Self::models_json_path(paths);
        let mcp_path = Self::mcp_json_path(paths);

        match mode {
            AppMode::Tako => {
                if read_main_api_key(paths).is_none() {
                    return Err(
                        "请先在顶部登录 Tako 账号，再将 WorkBuddy 接入 Tako 网关".to_string()
                    );
                }

                ensure_gateway_service(paths)?;

                backup_file_once(paths, "workbuddy", "models.json", &models_path)?;
                backup_file_once(paths, "workbuddy", "mcp.json", &mcp_path)?;
                let approvals_path = Self::mcp_approvals_json_path(paths);
                backup_file_once(
                    paths,
                    "workbuddy",
                    "mcp-approvals.json",
                    &approvals_path,
                )?;

                let state = read_switch_state(paths);
                let gateway_name = state.normalized_gateway_name();
                let target_specs = Self::resolve_target_models(paths);

                let mut root = Self::read_models_root(&models_path)?;
                let existing_models = root
                    .get("models")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();

                // Preserve user's own non-Tako custom models; replace Tako-managed entries atomically.
                let tako_ids: HashSet<String> = target_specs
                    .iter()
                    .map(|s| s.id.to_ascii_lowercase())
                    .collect();
                let mut merged_models: Vec<Value> = existing_models
                    .into_iter()
                    .filter(|item| {
                        if Self::is_tako_managed_model(item) {
                            return false;
                        }
                        let id_conflicts = item
                            .get("id")
                            .and_then(Value::as_str)
                            .is_some_and(|id| tako_ids.contains(&id.to_ascii_lowercase()));
                        !id_conflicts
                    })
                    .collect();

                for spec in &target_specs {
                    merged_models.push(Self::build_workbuddy_model_value(&gateway_name, spec));
                }

                root.insert("models".to_string(), Value::Array(merged_models));
                let bytes = serde_json::to_vec_pretty(&Value::Object(root))
                    .map_err(|e| format!("序列化 {} 失败: {e}", models_path.display()))?;
                write_atomic(&models_path, &bytes, false)?;

                if let Some(first) = target_specs.first() {
                    let _ = Self::sync_default_model_in_settings(paths, Some(&first.id));
                }

                merge_tako_mcp_server(&mcp_path, paths, true)?;
                merge_edgeone_mcp_server(&mcp_path, &state.mcp, state.mcp.web_page)?;

                // Pre-approve injected MCP servers in `~/.workbuddy/mcp-approvals.json`
                // so WorkBuddy trusts them immediately without manual confirmation prompts.
                let tako_entry = build_stdio_mcp_json_entry(paths);
                approve_workbuddy_mcp_server(paths, "tako", &tako_entry)?;
                if state.mcp.web_page {
                    let edgeone_entry = build_edgeone_mcp_json_entry(&state.mcp);
                    approve_workbuddy_mcp_server(paths, EDGEONE_MCP_SERVER_KEY, &edgeone_entry)?;
                } else {
                    revoke_workbuddy_mcp_server(paths, EDGEONE_MCP_SERVER_KEY, false)?;
                }
                Ok(())
            }
            AppMode::Official => {
                let models_absent = paths.backups_dir("workbuddy").join("models.json.absent");
                if models_path.exists() {
                    let mut root = Self::read_models_root(&models_path)?;
                    let kept_models: Vec<Value> = root
                        .get("models")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|item| !Self::is_tako_managed_model(item))
                        .collect();

                    if kept_models.is_empty() && models_absent.exists() && root.len() <= 1 {
                        let _ = fs::remove_file(&models_path);
                    } else {
                        root.insert("models".to_string(), Value::Array(kept_models));
                        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
                            .map_err(|e| format!("序列化 {} 失败: {e}", models_path.display()))?;
                        write_atomic(&models_path, &bytes, false)?;
                    }
                } else if let Some(backup) = read_backup(paths, "workbuddy", "models.json") {
                    write_atomic(&models_path, &backup, false)?;
                }

                Self::sync_default_model_in_settings(paths, None)?;

                let mcp_absent = paths.backups_dir("workbuddy").join("mcp.json.absent");
                if mcp_path.exists() {
                    merge_tako_mcp_server(&mcp_path, paths, false)?;
                    let state = read_switch_state(paths);
                    merge_edgeone_mcp_server(&mcp_path, &state.mcp, false)?;
                    if mcp_absent.exists() {
                        let is_empty_servers = read_json_object_or_empty(&mcp_path)
                            .ok()
                            .map(|m| {
                                m.get("mcpServers")
                                    .and_then(Value::as_object)
                                    .map(|s| s.is_empty())
                                    .unwrap_or(true)
                                    && m.len() <= 1
                            })
                            .unwrap_or(false);
                        if is_empty_servers {
                            let _ = fs::remove_file(&mcp_path);
                        }
                    }
                }

                let _ = revoke_workbuddy_mcp_server(paths, "tako", true);
                let _ = revoke_workbuddy_mcp_server(paths, EDGEONE_MCP_SERVER_KEY, true);

                let _ = stop_gateway_service_if_unused(paths);
                Ok(())
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let models_path = Self::models_json_path(paths);
        let mcp_path = Self::mcp_json_path(paths);
        let mut notes = vec![
            format!("自定义模型配置: {}", models_path.display()),
            format!("MCP 配置: {}", mcp_path.display()),
        ];

        let install = self.detect_install(paths);
        if let Some(path) = install.install_path {
            notes.push(format!("应用路径: {path}"));
        } else {
            notes.push("未检测到 WorkBuddy，可点击「一键安装」从官方国内 COS 源自动下载安装".to_string());
        }

        let mode = self.read_mode(paths);
        let tako_count = Self::count_tako_models(paths);
        if mode == AppMode::Tako {
            notes.push(format!(
                "已通过 models.json 注入 {tako_count} 个 Tako 网关模型（{WORKBUDDY_PROXY_BASE_URL}），在 WorkBuddy 模型选择器「自定义模型」分组可见"
            ));
            if !paths.is_test && !is_proxy_healthy(LOCAL_GATEWAY_PORT) {
                issues.push(format!(
                    "本地网关 {LOCAL_GATEWAY_URL} 未运行，WorkBuddy 无法调用 Tako 自定义模型"
                ));
            }
            if !has_tako_mcp_server(&mcp_path) {
                issues.push("已切至 Tako 模式但 mcp.json 未写入 Tako 超级 MCP".to_string());
            } else if is_workbuddy_mcp_approved(paths, "tako", &build_stdio_mcp_json_entry(paths)) {
                notes.push(
                    "Tako 超级 MCP 已注入 ~/.workbuddy/mcp.json 并写入自动信任（mcp-approvals.json）"
                        .to_string(),
                );
            } else {
                notes.push(
                    "Tako 超级 MCP 已注入 ~/.workbuddy/mcp.json（重新切一次 Tako 可写入自动信任，省去手动点击信任）"
                        .to_string(),
                );
            }
            let switch_state = read_switch_state(paths);
            if switch_state.mcp.web_page {
                if !has_edgeone_mcp_server(&mcp_path) {
                    notes.push(
                        "网站上线 MCP (EdgeOne) 尚未同步到 WorkBuddy，重新切换 Tako 即可写入"
                            .to_string(),
                    );
                } else if is_workbuddy_mcp_approved(
                    paths,
                    EDGEONE_MCP_SERVER_KEY,
                    &build_edgeone_mcp_json_entry(&switch_state.mcp),
                ) {
                    notes.push("网站上线 MCP (EdgeOne) 已自动信任".to_string());
                }
            }
        } else {
            notes.push("当前为官方原版模式；切至 Tako 后将自动把 Tako 模型与超级 MCP 写入 ~/.workbuddy".to_string());
        }

        notes.push("WorkBuddy 国内版原生简体中文，无需额外汉化补丁".to_string());

        DiagnosticResult {
            ok: issues.is_empty(),
            issues,
            notes,
        }
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const WORKBUDDY_PRODUCT_REG_GUID: &str = "BFD312E9-1019-4F57-9F44-F86246833B50";

/// Known WorkBuddy NSIS (`electron-builder`) and legacy Inno Setup `Uninstall` subkey names.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const KNOWN_WORKBUDDY_UNINSTALL_KEYS: &[&str] = &[
    "BFD312E9-1019-4F57-9F44-F86246833B50",
    "{BFD312E9-1019-4F57-9F44-F86246833B50}_is1",
    "{BFD312E9-1019-4F57-9F44-F86246833B50}}_is1",
    "{61A882F2-969F-45BF-A652-FA5C6B03DE22}_is1",
    "{61A882F2-969F-45BF-A652-FA5C6B03DE22}}_is1",
    "{B7353D0F-D01C-4EC3-96A9-5CEA88358454}_is1",
    "{B7353D0F-D01C-4EC3-96A9-5CEA88358454}}_is1",
    "{E5979B74-105C-46B5-AED6-599C029B689D}_is1",
    "{E5979B74-105C-46B5-AED6-599C029B689D}}_is1",
];

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug, Clone, Default)]
struct WindowsUninstallRecord {
    key_name: String,
    display_name: Option<String>,
    display_version: Option<String>,
    install_location: Option<String>,
    install_source: Option<String>,
    display_icon: Option<String>,
    uninstall_string: Option<String>,
    quiet_uninstall_string: Option<String>,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn strip_windows_icon_index(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some((before, after)) = trimmed.rsplit_once(',') {
        let idx = after.trim().strip_prefix('-').unwrap_or_else(|| after.trim());
        if !idx.is_empty() && idx.bytes().all(|b| b.is_ascii_digit()) {
            return before.trim();
        }
    }
    trimmed
}

/// Extracts an `.exe` path from a Windows registry command or icon string such as:
/// - `"D:\Program Files\WorkBuddy\Uninstall WorkBuddy.exe" /currentuser`
/// - `D:\Program Files\WorkBuddy\WorkBuddy.exe,0`
/// - `D:\Program Files\WorkBuddy\Uninstall WorkBuddy.exe /allusers`
/// - `"D:\WorkBuddy\WorkBuddy.exe" "%1"`
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_windows_exe_path_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        let end_quote = rest.find('"')?;
        let inner = strip_windows_icon_index(&rest[..end_quote]).trim();
        return (!inner.is_empty()).then(|| inner.to_string());
    }

    let without_icon = strip_windows_icon_index(trimmed);
    if without_icon.len() >= 4
        && without_icon[without_icon.len() - 4..].eq_ignore_ascii_case(".exe")
    {
        return Some(without_icon.to_string());
    }

    let bytes = without_icon.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        if without_icon
            .get(i..i + 4)
            .is_some_and(|seg| seg.eq_ignore_ascii_case(".exe"))
        {
            let boundary_ok = match bytes.get(i + 4) {
                None => true,
                Some(b' ' | b'\t' | b',' | b'"' | b'/') => true,
                Some(_) => false,
            };
            if boundary_ok {
                let candidate = without_icon[..i + 4].trim();
                if !candidate.is_empty() {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_path_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn windows_path_parent(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let idx = trimmed.rfind(['\\', '/'])?;
    if idx == 2 && trimmed.as_bytes().get(1) == Some(&b':') {
        return Some(&trimmed[..3]);
    }
    (idx > 0).then_some(&trimmed[..idx])
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn join_windows_workbuddy_exe(dir: &str) -> Option<String> {
    let cleaned = dir.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    if cleaned.is_empty() {
        return None;
    }
    if windows_path_file_name(cleaned).eq_ignore_ascii_case("WorkBuddy.exe") {
        return Some(cleaned.to_string());
    }
    let sep = if cleaned.contains('/') && !cleaned.contains('\\') {
        '/'
    } else {
        '\\'
    };
    Some(format!("{cleaned}{sep}WorkBuddy.exe"))
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn expand_windows_env_vars(raw: &str, mut lookup: impl FnMut(&str) -> Option<String>) -> String {
    if !raw.contains('%') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        if let Some(end) = after_start.find('%') {
            let var_name = &after_start[..end];
            if !var_name.is_empty()
                && var_name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'(' || b == b')')
            {
                if let Some(val) = lookup(var_name) {
                    out.push_str(&val);
                } else {
                    out.push('%');
                    out.push_str(var_name);
                    out.push('%');
                }
                rest = &after_start[end + 1..];
                continue;
            }
        }
        out.push('%');
        rest = after_start;
    }
    out.push_str(rest);
    out
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn push_unique_str(out: &mut Vec<String>, candidate: String) {
    if !candidate.is_empty() && !out.iter().any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        out.push(candidate);
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn collect_workbuddy_exe_candidates_from_cmd_str(cmd: &str, out: &mut Vec<String>) {
    let Some(exe_str) = parse_windows_exe_path_str(cmd) else {
        return;
    };
    if windows_path_file_name(&exe_str).eq_ignore_ascii_case("WorkBuddy.exe") {
        push_unique_str(out, exe_str.clone());
    }
    if let Some(parent) = windows_path_parent(&exe_str) {
        if let Some(sibling_wb) = join_windows_workbuddy_exe(parent) {
            push_unique_str(out, sibling_wb);
        }
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn is_workbuddy_uninstall_entry(rec: &WindowsUninstallRecord) -> bool {
    let key_trimmed = rec.key_name.trim();
    if KNOWN_WORKBUDDY_UNINSTALL_KEYS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(key_trimmed))
    {
        return true;
    }
    if rec
        .display_name
        .as_deref()
        .is_some_and(|n| n.to_ascii_lowercase().contains("workbuddy"))
    {
        return true;
    }
    for raw_cmd in [&rec.display_icon, &rec.uninstall_string, &rec.quiet_uninstall_string]
        .into_iter()
        .flatten()
    {
        if let Some(exe) = parse_windows_exe_path_str(raw_cmd) {
            let fname = windows_path_file_name(&exe);
            if fname.eq_ignore_ascii_case("WorkBuddy.exe")
                || fname.eq_ignore_ascii_case("Uninstall WorkBuddy.exe")
            {
                return true;
            }
        }
    }
    false
}

/// Extracts candidate `WorkBuddy.exe` paths from a matched Windows `Uninstall` record.
///
/// WorkBuddy's `electron-builder` NSIS installer writes `DisplayName`, `DisplayVersion`,
/// `DisplayIcon` (`<INSTDIR>\WorkBuddy.exe,0`), and `UninstallString`
/// (`"<INSTDIR>\Uninstall WorkBuddy.exe" /currentuser`) under `Uninstall\<GUID>`, while
/// leaving `InstallLocation` empty on the `Uninstall` subkey itself.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn extract_candidates_from_uninstall_record(
    rec: &WindowsUninstallRecord,
    mut env_lookup: impl FnMut(&str) -> Option<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for dir_opt in [&rec.install_location, &rec.install_source] {
        if let Some(raw_dir) = dir_opt.as_deref() {
            let expanded = expand_windows_env_vars(raw_dir, &mut env_lookup);
            if let Some(exe) = join_windows_workbuddy_exe(&expanded) {
                push_unique_str(&mut out, exe);
            }
        }
    }
    for cmd_opt in [
        &rec.display_icon,
        &rec.uninstall_string,
        &rec.quiet_uninstall_string,
    ] {
        if let Some(raw_cmd) = cmd_opt.as_deref() {
            let expanded = expand_windows_env_vars(raw_cmd, &mut env_lookup);
            collect_workbuddy_exe_candidates_from_cmd_str(&expanded, &mut out);
        }
    }
    out
}

/// Parses `%USERPROFILE%\.workbuddy\logs\installer.log` (newest lines first) for
/// `WorkBuddy.exe` candidate paths recorded by the NSIS installer.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn parse_workbuddy_installer_log_candidates(log_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in log_text.lines().rev() {
        let trimmed = line.trim();
        if let Some((_, rest)) = trimmed.split_once("INFO extraction sentinel OK: ") {
            let path = rest.strip_suffix(" present").unwrap_or(rest).trim();
            if let Some(exe) = join_windows_workbuddy_exe(path) {
                push_unique_str(&mut out, exe);
            }
            continue;
        }
        for marker in [
            "INFO final install directory: ",
            "INFO install directory confirmed after authorization: ",
            "INFO install directory confirmed: ",
            "INFO install target dir: ",
        ] {
            if let Some((_, rest)) = trimmed.split_once(marker) {
                let dir = rest.strip_suffix(" (access=yes)").unwrap_or(rest).trim();
                if let Some(exe) = join_windows_workbuddy_exe(dir) {
                    push_unique_str(&mut out, exe);
                }
                break;
            }
        }
    }
    out
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn extract_workbuddy_paths_from_decoded_text(text: &str, out: &mut Vec<String>) {
    const SUFFIX: &str = "workbuddy.exe";
    let lower = text.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find(SUFFIX) {
        let match_start = search_from + rel;
        let match_end = match_start + SUFFIX.len();
        search_from = match_end;

        // Require path separator right before `WorkBuddy.exe` and non-path boundary after it.
        let prev_char = text[..match_start].chars().next_back();
        if !matches!(prev_char, Some('\\' | '/')) {
            continue;
        }
        let next_char = text[match_end..].chars().next();
        if next_char.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
            continue;
        }

        // Walk backwards across valid Windows path characters to locate `<Drive>:\` or `%VAR%\`.
        let mut run_start = match_start;
        for (idx, ch) in text[..match_start].char_indices().rev() {
            if ch < ' ' || matches!(ch, '<' | '>' | '"' | '|' | '?' | '*' | '\u{FFFD}') {
                break;
            }
            run_start = idx;
        }
        let segment = &text[run_start..match_end];
        let seg_bytes = segment.as_bytes();
        let mut extracted: Option<&str> = None;
        for i in 1..seg_bytes.len().saturating_sub(1) {
            if seg_bytes[i] == b':'
                && seg_bytes[i - 1].is_ascii_alphabetic()
                && matches!(seg_bytes[i + 1], b'\\' | b'/')
            {
                extracted = segment.get(i - 1..);
                break;
            }
        }
        if extracted.is_none() {
            if let Some(pct_idx) = segment.find('%') {
                if segment[pct_idx + 1..].contains('%') {
                    extracted = Some(&segment[pct_idx..]);
                }
            }
        }
        if let Some(found) = extracted {
            push_unique_str(out, found.to_string());
        }
    }
}

/// Extracts target `WorkBuddy.exe` paths from a Windows `.lnk` ShellLink binary without
/// spawning PowerShell or COM (`WScript.Shell`), supporting both even and odd UTF-16LE
/// field offsets as well as ANSI `LocalBasePath`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn extract_workbuddy_exe_from_lnk_bytes(bytes: &[u8]) -> Vec<String> {
    let capped = &bytes[..bytes.len().min(64 * 1024)];
    let mut out = Vec::new();

    for offset in [0usize, 1usize] {
        if capped.len() <= offset + 1 {
            continue;
        }
        let u16_units: Vec<u16> = capped[offset..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let decoded = String::from_utf16_lossy(&u16_units);
        extract_workbuddy_paths_from_decoded_text(&decoded, &mut out);
    }

    let ansi = String::from_utf8_lossy(capped);
    extract_workbuddy_paths_from_decoded_text(&ansi, &mut out);
    out
}

#[cfg(target_os = "windows")]
fn lookup_host_env(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
struct RegistryUninstallMatch {
    display_version: Option<String>,
    candidates: Vec<String>,
}

#[cfg(target_os = "windows")]
struct WindowsRegistryProbe {
    uninstall_matches: Vec<RegistryUninstallMatch>,
    extra_candidates: Vec<String>,
}

#[cfg(target_os = "windows")]
fn read_windows_registry_candidates() -> WindowsRegistryProbe {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let mut uninstall_matches: Vec<RegistryUninstallMatch> = Vec::new();
    let mut extra_candidates: Vec<String> = Vec::new();

    let roots = [
        RegKey::predef(HKEY_CURRENT_USER),
        RegKey::predef(HKEY_LOCAL_MACHINE),
    ];

    // 1. Electron-builder NSIS product key (`Software\BFD312E9-1019-4F57-9F44-F86246833B50\InstallLocation`)
    let product_subkeys = [
        format!(r"Software\{WORKBUDDY_PRODUCT_REG_GUID}"),
        format!(r"Software\WOW6432Node\{WORKBUDDY_PRODUCT_REG_GUID}"),
    ];
    for root in &roots {
        for sub_path in &product_subkeys {
            if let Ok(sub) = root.open_subkey(sub_path) {
                if let Ok(loc) = sub.get_value::<String, _>("InstallLocation") {
                    let expanded = expand_windows_env_vars(&loc, lookup_host_env);
                    if let Some(exe) = join_windows_workbuddy_exe(&expanded) {
                        push_unique_str(&mut extra_candidates, exe);
                    }
                }
            }
        }
    }

    // 2. Uninstall entries (`InstallLocation`, `DisplayIcon`, `UninstallString`, `QuietUninstallString`)
    for root in &roots {
        for sub_path in [
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ] {
            let Ok(uninstall) = root.open_subkey(sub_path) else {
                continue;
            };
            for key_name in uninstall.enum_keys().flatten() {
                let Ok(sub) = uninstall.open_subkey(&key_name) else {
                    continue;
                };
                let rec = WindowsUninstallRecord {
                    key_name,
                    display_name: sub.get_value::<String, _>("DisplayName").ok(),
                    display_version: sub
                        .get_value::<String, _>("DisplayVersion")
                        .ok()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    install_location: sub.get_value::<String, _>("InstallLocation").ok(),
                    install_source: sub.get_value::<String, _>("InstallSource").ok(),
                    display_icon: sub.get_value::<String, _>("DisplayIcon").ok(),
                    uninstall_string: sub.get_value::<String, _>("UninstallString").ok(),
                    quiet_uninstall_string: sub.get_value::<String, _>("QuietUninstallString").ok(),
                };
                if is_workbuddy_uninstall_entry(&rec) {
                    let candidates = extract_candidates_from_uninstall_record(&rec, lookup_host_env);
                    uninstall_matches.push(RegistryUninstallMatch {
                        display_version: rec.display_version,
                        candidates,
                    });
                }
            }
        }
    }

    // 3. Deep-link protocol, Applications shell command, App Paths, and login Run entry
    for root in &roots {
        for cmd_subkey in [
            r"Software\Classes\workbuddy\shell\open\command",
            r"Software\Classes\Applications\WorkBuddy.exe\shell\open\command",
            r"Software\Microsoft\Windows\CurrentVersion\App Paths\WorkBuddy.exe",
        ] {
            if let Ok(sub) = root.open_subkey(cmd_subkey) {
                if let Ok(default_cmd) = sub.get_value::<String, _>("") {
                    let expanded = expand_windows_env_vars(&default_cmd, lookup_host_env);
                    collect_workbuddy_exe_candidates_from_cmd_str(&expanded, &mut extra_candidates);
                }
                if let Ok(app_path_dir) = sub.get_value::<String, _>("Path") {
                    let expanded = expand_windows_env_vars(&app_path_dir, lookup_host_env);
                    if let Some(exe) = join_windows_workbuddy_exe(&expanded) {
                        push_unique_str(&mut extra_candidates, exe);
                    }
                }
            }
        }
    }
    if let Ok(run_key) =
        RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
    {
        for val_name in ["WorkBuddy.WorkBuddy", "WorkBuddy"] {
            if let Ok(run_cmd) = run_key.get_value::<String, _>(val_name) {
                let expanded = expand_windows_env_vars(&run_cmd, lookup_host_env);
                collect_workbuddy_exe_candidates_from_cmd_str(&expanded, &mut extra_candidates);
            }
        }
    }

    WindowsRegistryProbe {
        uninstall_matches,
        extra_candidates,
    }
}

#[cfg(target_os = "windows")]
fn ps_hidden(script: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
fn read_exe_file_version(exe: &std::path::Path) -> Option<String> {
    let escaped = exe.to_string_lossy().replace('\'', "''");
    let script = format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion");
    ps_hidden(&script)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn read_unpacked_package_json_version(exe: &std::path::Path) -> Option<String> {
    let pkg_path = exe.parent()?.join("resources").join("app").join("package.json");
    let raw = fs::read_to_string(pkg_path).ok()?;
    let val: Value = serde_json::from_str(&raw).ok()?;
    val.get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn read_installer_log_tail(log_path: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(log_path).ok()?;
    let len = file.metadata().ok()?.len();
    if len > 64 * 1024 {
        let _ = file.seek(SeekFrom::End(-(64 * 1024)));
    }
    let mut buf = Vec::with_capacity(len.min(64 * 1024) as usize);
    file.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(target_os = "windows")]
fn find_windows_workbuddy_exe_and_version(paths: &AppPaths) -> Option<(Option<String>, PathBuf)> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push_path = |p: PathBuf| {
        if !p.as_os_str().is_empty()
            && !candidates.iter().any(|existing| {
                existing
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&p.to_string_lossy())
            })
        {
            candidates.push(p);
        }
    };

    let uninstall_matches = if paths.is_test {
        Vec::new()
    } else {
        let probe = read_windows_registry_candidates();
        for entry in &probe.uninstall_matches {
            for c in &entry.candidates {
                push_path(PathBuf::from(c));
            }
        }
        for c in probe.extra_candidates {
            push_path(PathBuf::from(c));
        }
        if let Some(running_exe) = super::windows_process::running_exe_path(AppId::WorkBuddy) {
            push_path(running_exe);
        }
        probe.uninstall_matches
    };

    let local_app_data = if paths.is_test {
        None
    } else {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    }
    .unwrap_or_else(|| paths.home.join("AppData").join("Local"));

    let roaming_app_data = if paths.is_test {
        None
    } else {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    .unwrap_or_else(|| paths.home.join("AppData").join("Roaming"));

    push_path(
        local_app_data
            .join("Programs")
            .join("WorkBuddy")
            .join("WorkBuddy.exe"),
    );
    push_path(
        local_app_data
            .join("Programs")
            .join("workbuddy")
            .join("WorkBuddy.exe"),
    );
    push_path(local_app_data.join("WorkBuddy").join("WorkBuddy.exe"));

    if !paths.is_test {
        let program_files = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        push_path(program_files.join("WorkBuddy").join("WorkBuddy.exe"));

        if let Some(pf_x86) = std::env::var_os("ProgramFiles(x86)").map(PathBuf::from) {
            push_path(pf_x86.join("WorkBuddy").join("WorkBuddy.exe"));
        }
    }

    // Start Menu & Desktop `.lnk` shortcuts
    let mut lnk_paths = vec![
        roaming_app_data
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("WorkBuddy.lnk"),
        roaming_app_data
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("WorkBuddy")
            .join("WorkBuddy.lnk"),
        paths.home.join("Desktop").join("WorkBuddy.lnk"),
    ];
    if !paths.is_test {
        let program_data = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        lnk_paths.push(
            program_data
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("WorkBuddy.lnk"),
        );
        lnk_paths.push(
            program_data
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("WorkBuddy")
                .join("WorkBuddy.lnk"),
        );
        let public_dir = std::env::var_os("PUBLIC")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Users\Public"));
        lnk_paths.push(public_dir.join("Desktop").join("WorkBuddy.lnk"));
    }
    for lnk in lnk_paths {
        if lnk.is_file() {
            if let Ok(bytes) = fs::read(&lnk) {
                for raw_target in extract_workbuddy_exe_from_lnk_bytes(&bytes) {
                    let expanded = if paths.is_test {
                        raw_target
                    } else {
                        expand_windows_env_vars(&raw_target, lookup_host_env)
                    };
                    push_path(PathBuf::from(expanded));
                }
            }
        }
    }

    // WorkBuddy NSIS installer log (`~/.workbuddy/logs/installer.log`)
    let installer_log = paths.workbuddy_dir.join("logs").join("installer.log");
    if let Some(log_tail) = read_installer_log_tail(&installer_log) {
        for raw_exe in parse_workbuddy_installer_log_candidates(&log_tail) {
            push_path(PathBuf::from(raw_exe));
        }
    }

    // System PATH fallback
    if !paths.is_test {
        if let Some(path_os) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_os) {
                if !dir.as_os_str().is_empty() {
                    push_path(dir.join("WorkBuddy.exe"));
                }
            }
        }
    }

    let found_exe = candidates.into_iter().find(|p| p.is_file())?;
    let found_str = found_exe.to_string_lossy();
    let reg_ver = uninstall_matches
        .iter()
        .find(|entry| {
            entry.display_version.is_some()
                && entry
                    .candidates
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&found_str))
        })
        .and_then(|entry| entry.display_version.clone())
        .or_else(|| {
            uninstall_matches
                .into_iter()
                .find_map(|entry| entry.display_version)
        });

    let ver = reg_ver
        .or_else(|| read_unpacked_package_json_version(&found_exe))
        .or_else(|| {
            if paths.is_test {
                None
            } else {
                read_exe_file_version(&found_exe)
            }
        });

    Some((ver, found_exe))
}

#[cfg(target_os = "windows")]
fn find_windows_workbuddy_exe(paths: &AppPaths) -> Option<PathBuf> {
    find_windows_workbuddy_exe_and_version(paths).map(|(_, exe)| exe)
}

#[cfg(target_os = "windows")]
fn detect_windows_workbuddy(paths: &AppPaths) -> Option<(Option<String>, String, String)> {
    let (ver, exe) = find_windows_workbuddy_exe_and_version(paths)?;
    Some((ver, exe.to_string_lossy().to_string(), "nsis".to_string()))
}

#[cfg(target_os = "windows")]
fn quit_windows_workbuddy() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/IM", "WorkBuddy.exe", "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_workbuddy(paths: &AppPaths) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let exe = find_windows_workbuddy_exe(paths)
        .ok_or_else(|| "未检测到 WorkBuddy.exe，请先安装 WorkBuddy".to_string())?;
    let mut cmd = std::process::Command::new(&exe);
    if let Some(parent) = exe.parent() {
        cmd.current_dir(parent);
    }
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| format!("启动 WorkBuddy 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{save_switch_auth, save_switch_state};
    use tempfile::TempDir;

    fn test_paths() -> (TempDir, AppPaths) {
        let dir = TempDir::new().expect("tempdir");
        let paths = AppPaths::for_test(dir.path());
        (dir, paths)
    }

    #[test]
    fn apply_tako_and_restore_official_preserves_user_models_and_mcps() {
        let (_dir, paths) = test_paths();
        save_switch_auth(&paths, Some("sk-tako-test-key"), None, Some("test")).unwrap();

        // Pre-populate user custom model and user MCP server.
        let models_path = WorkBuddyAdapter::models_json_path(&paths);
        let user_models = json!({
            "models": [
                {
                    "id": "my-ollama-qwen",
                    "name": "Local Qwen",
                    "vendor": "Ollama",
                    "url": "http://localhost:11434/v1/chat/completions",
                    "supportsToolCall": true
                }
            ]
        });
        write_atomic(
            &models_path,
            serde_json::to_string_pretty(&user_models).unwrap().as_bytes(),
            false,
        )
        .unwrap();

        let mcp_path = WorkBuddyAdapter::mcp_json_path(&paths);
        let user_mcp = json!({
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"]
                }
            }
        });
        write_atomic(
            &mcp_path,
            serde_json::to_string_pretty(&user_mcp).unwrap().as_bytes(),
            false,
        )
        .unwrap();

        let settings_path = WorkBuddyAdapter::settings_json_path(&paths);
        write_atomic(
            &settings_path,
            br#"{"theme":"dark","model":"my-ollama-qwen"}"#,
            false,
        )
        .unwrap();

        let adapter = WorkBuddyAdapter;
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);

        // Apply Tako mode
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        assert!(adapter.read_mcp_enabled(&paths, AppMode::Tako));

        let written_models: Value =
            serde_json::from_str(&fs::read_to_string(&models_path).unwrap()).unwrap();
        let arr = written_models["models"].as_array().unwrap();
        // Keeps user model + injects all recommended Tako models.
        assert!(arr.iter().any(|m| m["id"] == "my-ollama-qwen"));
        assert!(arr.iter().any(|m| {
            m["id"] == "claude-opus-5-5"
                && m["url"] == WORKBUDDY_CHAT_COMPLETIONS_URL
                && m["apiKey"] == WORKBUDDY_LOCAL_API_KEY_PLACEHOLDER
                && m["takoManaged"] == true
        }));

        let written_mcp: Value =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(written_mcp["mcpServers"]["filesystem"].is_object());
        assert!(written_mcp["mcpServers"]["tako"].is_object());

        let written_settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(written_settings["theme"], "dark");
        assert_eq!(written_settings["model"], "claude-opus-5-5");

        // Switch back to Official: removes Tako entries, preserves user model/MCP/settings.
        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        assert!(!adapter.read_mcp_enabled(&paths, AppMode::Official));

        let restored_models: Value =
            serde_json::from_str(&fs::read_to_string(&models_path).unwrap()).unwrap();
        let restored_arr = restored_models["models"].as_array().unwrap();
        assert_eq!(restored_arr.len(), 1);
        assert_eq!(restored_arr[0]["id"], "my-ollama-qwen");

        let restored_mcp: Value =
            serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(restored_mcp["mcpServers"]["filesystem"].is_object());
        assert!(restored_mcp["mcpServers"].get("tako").is_none());

        let restored_settings: Value =
            serde_json::from_str(&fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(restored_settings["model"], "my-ollama-qwen");
        assert_eq!(restored_settings["theme"], "dark");
    }

    #[test]
    fn custom_filter_writes_selected_models_and_clean_official_removes_absent_files() {
        let (_dir, paths) = test_paths();
        save_switch_auth(&paths, Some("sk-tako-test-key"), None, Some("test")).unwrap();
        let mut st = read_switch_state(&paths);
        st.model_filters.workbuddy = ClientModelFilter {
            mode: ModelFilterMode::Custom,
            enabled_ids: vec!["gpt-6.2-sol".to_string(), "swe-2".to_string()],
        };
        save_switch_state(&paths, &st).unwrap();

        let adapter = WorkBuddyAdapter;
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let models_path = WorkBuddyAdapter::models_json_path(&paths);
        let written: Value =
            serde_json::from_str(&fs::read_to_string(&models_path).unwrap()).unwrap();
        let ids: Vec<&str> = written["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["gpt-6.2-sol", "swe-2"]);

        // Verify reasoning efforts, onlyReasoning, and thinkingLevelMap so WorkBuddy defaults
        // thinking ON and preserves xhigh/max without CustomModelFieldCleanup downgrade.
        let gpt = &written["models"][0];
        assert_eq!(gpt["supportsReasoning"], true);
        assert_eq!(gpt["onlyReasoning"], true);
        assert_eq!(
            gpt["reasoning"]["supportedEfforts"],
            json!(["low", "medium", "high", "xhigh", "max"])
        );
        assert_eq!(gpt["reasoning"]["canDisableThinking"], false);
        assert_eq!(gpt["thinkingLevelMap"]["xhigh"], "xhigh");
        assert_eq!(gpt["thinkingLevelMap"]["max"], "max");
        assert!(gpt["thinkingLevelMap"]["off"].is_null());

        let swe = &written["models"][1];
        assert_eq!(swe["supportsReasoning"], true);
        assert_eq!(swe["onlyReasoning"], true);
        assert_eq!(
            swe["reasoning"]["supportedEfforts"],
            json!(["medium", "high", "max"])
        );
        assert!(swe["thinkingLevelMap"]["low"].is_null());
        assert_eq!(swe["thinkingLevelMap"]["max"], "max");

        // Verify Tako MCP was auto-approved in mcp-approvals.json.
        let approvals_path = WorkBuddyAdapter::mcp_approvals_json_path(&paths);
        assert!(approvals_path.exists());
        assert!(is_workbuddy_mcp_approved(
            &paths,
            "tako",
            &build_stdio_mcp_json_entry(&paths)
        ));

        // Reverting to Official removes files that did not exist before.
        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert!(!models_path.exists());
        assert!(!WorkBuddyAdapter::mcp_json_path(&paths).exists());
        assert!(!WorkBuddyAdapter::settings_json_path(&paths).exists());
        assert!(!approvals_path.exists());
    }

    #[test]
    fn mcp_trust_hash_matches_workbuddy_golden_vector_and_http_origin() {
        // Golden vector verified directly against WorkBuddy 5.11.3 ~/.workbuddy/mcp-approvals.json
        let edgeone_entry = json!({
            "command": "npx",
            "args": ["-y", "@edgeone/makers-mcp@latest", "--region", "china"]
        });
        let key = approval_key_for_mcp_entry("tako-web-page", &edgeone_entry).unwrap();
        assert_eq!(
            key,
            "4089d830c06f4f4fb7ea0633fcd82f83b21154a6a1852ba0bea3b0b651dd7869::tako-web-page"
        );

        // HTTP/SSE entry hashes URL origin (scheme + host + non-default port)
        let http_entry = json!({
            "type": "http",
            "url": "https://docs.example.com:8443/v1/mcp?token=abc"
        });
        assert_eq!(
            compute_mcp_trust_material(&http_entry).as_deref(),
            Some("https://docs.example.com:8443")
        );

        // Non-reasoning model must not emit reasoning or thinkingLevelMap
        let non_reason = WorkBuddyAdapter::build_workbuddy_model_value(
            "Tako",
            &build_fallback_spec_from_id("gpt-4o-mini"),
        );
        assert_eq!(non_reason["supportsReasoning"], false);
        assert!(non_reason.get("onlyReasoning").is_none());
        assert!(non_reason.get("reasoning").is_none());
        assert!(non_reason.get("thinkingLevelMap").is_none());
    }

    #[test]
    fn apply_tako_approves_and_revokes_edgeone_mcp_while_preserving_user_approvals() {
        let (_dir, paths) = test_paths();
        save_switch_auth(&paths, Some("sk-tako-test-key"), None, Some("test")).unwrap();

        // Pre-existing user approval in mcp-approvals.json
        let approvals_path = WorkBuddyAdapter::mcp_approvals_json_path(&paths);
        write_atomic(
            &approvals_path,
            br#"{"deadbeef::user-custom-mcp": 1700000000000}"#,
            false,
        )
        .unwrap();

        let mut st = read_switch_state(&paths);
        st.mcp.web_page = true;
        save_switch_state(&paths, &st).unwrap();

        let adapter = WorkBuddyAdapter;
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        assert!(is_workbuddy_mcp_approved(
            &paths,
            "tako",
            &build_stdio_mcp_json_entry(&paths)
        ));
        assert!(is_workbuddy_mcp_approved(
            &paths,
            EDGEONE_MCP_SERVER_KEY,
            &build_edgeone_mcp_json_entry(&st.mcp)
        ));

        // Disable web_page and re-apply Tako -> revokes tako-web-page, keeps tako + user approval.
        st.mcp.web_page = false;
        save_switch_state(&paths, &st).unwrap();
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();

        let map = read_json_object_or_empty(&approvals_path).unwrap();
        assert!(map.contains_key("deadbeef::user-custom-mcp"));
        assert!(map.keys().any(|k| k.ends_with("::tako")));
        assert!(!map.keys().any(|k| k.ends_with("::tako-web-page")));

        // Official mode removes tako, preserves user approval.
        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        let map2 = read_json_object_or_empty(&approvals_path).unwrap();
        assert_eq!(map2.len(), 1);
        assert!(map2.contains_key("deadbeef::user-custom-mcp"));
    }

    #[test]
    fn windows_uninstall_record_extracts_custom_dir_when_install_location_missing() {
        // Real electron-builder NSIS entry: InstallLocation is empty on the Uninstall subkey,
        // while DisplayIcon and UninstallString point to the user-chosen D: drive folder.
        let nsis_rec = WindowsUninstallRecord {
            key_name: WORKBUDDY_PRODUCT_REG_GUID.to_string(),
            display_name: Some("WorkBuddy 5.11.3".to_string()),
            display_version: Some("5.11.3".to_string()),
            install_location: None,
            install_source: None,
            display_icon: Some(r"D:\Program Files\WorkBuddy\WorkBuddy.exe,0".to_string()),
            uninstall_string: Some(
                r#""D:\Program Files\WorkBuddy\Uninstall WorkBuddy.exe" /currentuser"#.to_string(),
            ),
            quiet_uninstall_string: Some(
                r#""D:\Program Files\WorkBuddy\Uninstall WorkBuddy.exe" /currentuser /S"#
                    .to_string(),
            ),
        };
        assert!(is_workbuddy_uninstall_entry(&nsis_rec));
        let cands = extract_candidates_from_uninstall_record(&nsis_rec, |_| None);
        assert_eq!(cands, vec![r"D:\Program Files\WorkBuddy\WorkBuddy.exe"]);

        // Unquoted UninstallString with spaces + env var expansion + legacy Inno Setup key.
        let legacy_rec = WindowsUninstallRecord {
            key_name: "{61A882F2-969F-45BF-A652-FA5C6B03DE22}_is1".to_string(),
            display_name: None,
            display_version: Some("5.8.0".to_string()),
            install_location: Some(r"%LOCALAPPDATA%\Programs\WorkBuddy\".to_string()),
            install_source: None,
            display_icon: None,
            uninstall_string: Some(
                r"E:\Custom Apps\WorkBuddy\Uninstall WorkBuddy.exe /allusers".to_string(),
            ),
            quiet_uninstall_string: None,
        };
        assert!(is_workbuddy_uninstall_entry(&legacy_rec));
        let legacy_cands = extract_candidates_from_uninstall_record(&legacy_rec, |k| {
            (k == "LOCALAPPDATA").then(|| r"C:\Users\Demo\AppData\Local".to_string())
        });
        assert_eq!(
            legacy_cands,
            vec![
                r"C:\Users\Demo\AppData\Local\Programs\WorkBuddy\WorkBuddy.exe",
                r"E:\Custom Apps\WorkBuddy\WorkBuddy.exe",
            ]
        );

        // Unrelated software must never match.
        let unrelated = WindowsUninstallRecord {
            key_name: "{EA457B21-F73E-494C-ACAB-524FDE069978}_is1".to_string(),
            display_name: Some("Microsoft Visual Studio Code".to_string()),
            display_version: Some("1.146.0".to_string()),
            install_location: Some(r"D:\Program Files\Microsoft VS Code\".to_string()),
            install_source: None,
            display_icon: Some(r"D:\Program Files\Microsoft VS Code\Code.exe".to_string()),
            uninstall_string: Some(r#""D:\Program Files\Microsoft VS Code\unins000.exe""#.to_string()),
            quiet_uninstall_string: None,
        };
        assert!(!is_workbuddy_uninstall_entry(&unrelated));
    }

    #[test]
    fn windows_installer_log_and_lnk_parsers_resolve_custom_and_unicode_paths() {
        let log_sample = "\
[2026-10-01 10:00:00] INFO final install directory: C:\\Old\\WorkBuddy\r\n\
[2026-11-26 09:30:01] INFO install target dir: D:\\DevTools\\WorkBuddy\r\n\
[2026-11-26 09:30:05] INFO install directory confirmed: D:\\DevTools\\WorkBuddy (access=yes)\r\n\
[2026-11-26 09:30:12] INFO extraction sentinel OK: D:\\DevTools\\WorkBuddy\\WorkBuddy.exe present\r\n";
        let log_cands = parse_workbuddy_installer_log_candidates(log_sample);
        assert_eq!(
            log_cands,
            vec![
                r"D:\DevTools\WorkBuddy\WorkBuddy.exe",
                r"C:\Old\WorkBuddy\WorkBuddy.exe",
            ]
        );

        // Build synthetic .lnk binary with odd-aligned UTF-16LE CJK path (no NUL before next length prefix)
        // plus an ANSI %LOCALAPPDATA% fallback block.
        let mut lnk = vec![0x4Cu8, 0x00, 0x00, 0x00, 0xFF]; // 5 bytes -> odd offset!
        let target_u16 = r"D:\办公软件\WorkBuddy\WorkBuddy.exe";
        lnk.extend_from_slice(&(target_u16.encode_utf16().count() as u16).to_le_bytes());
        for unit in target_u16.encode_utf16() {
            lnk.extend_from_slice(&unit.to_le_bytes());
        }
        lnk.extend_from_slice(&18u16.to_le_bytes()); // immediately followed by next StringData count
        lnk.extend_from_slice(b"\0%LOCALAPPDATA%\\Programs\\WorkBuddy\\WorkBuddy.exe\0");

        let lnk_cands = extract_workbuddy_exe_from_lnk_bytes(&lnk);
        assert_eq!(
            lnk_cands,
            vec![
                r"D:\办公软件\WorkBuddy\WorkBuddy.exe",
                r"%LOCALAPPDATA%\Programs\WorkBuddy\WorkBuddy.exe",
            ]
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_detect_install_resolves_custom_dir_alongside_tako_injection() {
        let (dir, paths) = test_paths();
        save_switch_auth(&paths, Some("sk-tako-test-key"), None, Some("test")).unwrap();

        let adapter = WorkBuddyAdapter;
        assert!(!adapter.detect_install(&paths).installed);

        // Inject Tako models & MCP first (reproducing user flow where config succeeds before install probe).
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        assert!(adapter.read_mcp_enabled(&paths, AppMode::Tako));

        // 1. Custom directory outside AppData/ProgramFiles discovered via ~/.workbuddy/logs/installer.log
        let custom_dir = dir.path().join("CustomDisk").join("WorkBuddy");
        let custom_exe = custom_dir.join("WorkBuddy.exe");
        let pkg_json = custom_dir.join("resources").join("app").join("package.json");
        write_atomic(&custom_exe, b"MZ", false).unwrap();
        write_atomic(&pkg_json, br#"{"name":"workbuddy","version":"5.11.3"}"#, false).unwrap();

        let log_path = paths.workbuddy_dir.join("logs").join("installer.log");
        let log_body = format!(
            "[2026-11-26 10:00:00] INFO final install directory: {}\r\n",
            custom_dir.display()
        );
        write_atomic(&log_path, log_body.as_bytes(), false).unwrap();

        let info = adapter.detect_install(&paths);
        assert!(info.installed);
        assert_eq!(info.version.as_deref(), Some("5.11.3"));
        assert_eq!(
            info.install_path.as_deref(),
            Some(custom_exe.to_string_lossy().as_ref())
        );
        let diag = adapter.diagnose(&paths);
        assert!(diag.notes.iter().any(|n| n.starts_with("应用路径: ")));
        assert!(!diag.notes.iter().any(|n| n.contains("未检测到 WorkBuddy")));

        // 2. Remove installer.log and discover relocated CJK directory via Desktop WorkBuddy.lnk
        fs::remove_file(&log_path).unwrap();
        fs::remove_file(&custom_exe).unwrap();

        let shortcut_exe = dir
            .path()
            .join("ShortcutDisk")
            .join("办公软件")
            .join("WorkBuddy")
            .join("WorkBuddy.exe");
        write_atomic(&shortcut_exe, b"MZ", false).unwrap();

        let mut lnk_bytes = vec![0x4Cu8, 0x00, 0x00, 0x00, 0xAA]; // odd alignment
        for unit in shortcut_exe.to_string_lossy().encode_utf16() {
            lnk_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        lnk_bytes.extend_from_slice(&[0x00, 0x00]);
        write_atomic(&paths.home.join("Desktop").join("WorkBuddy.lnk"), &lnk_bytes, false).unwrap();

        let info_via_lnk = adapter.detect_install(&paths);
        assert!(info_via_lnk.installed);
        assert_eq!(
            info_via_lnk.install_path.as_deref(),
            Some(shortcut_exe.to_string_lossy().as_ref())
        );

        // 3. Stale shortcut after executable deletion must NOT false-positive as installed.
        fs::remove_file(&shortcut_exe).unwrap();
        assert!(!adapter.detect_install(&paths).installed);
    }
}
