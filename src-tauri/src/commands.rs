use crate::{
    backups::{
        load_latest_apply_result_from_disk, restore_backup_file, save_latest_apply_result_to_disk,
    },
    config_paths::{claude_settings_path, codex_config_path},
    models::{ApplyResult, ExistingConfig, LoadedConfigs, PreviewResult, RestoreResult},
    platforms::{apply_platform, codex, preview_platform},
    providers::{
        load_provider_catalog_from_disk, types::PLATFORM_CODEX, validation::parse_http_url,
        validation::validate_input, ConfigInput, ProviderCatalog,
    },
    redaction::{redact_json_text, redact_plain_text},
    tools,
    utils::display_path,
};
use serde::Serialize;
use tauri::{Emitter, Manager};
use url::Url;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TakoAuthEvent {
    key: String,
    state: Option<String>,
}

#[tauri::command]
pub fn detect_tools() -> Vec<crate::models::ToolStatus> {
    tools::detect_tools()
}

#[tauri::command]
pub fn load_current_configs() -> Result<LoadedConfigs, String> {
    let codex_path = codex_config_path()?;
    let claude_path = claude_settings_path()?;

    Ok(LoadedConfigs {
        codex: read_existing_config("codex", &codex_path, redact_plain_text),
        claude: read_existing_config("claude", &claude_path, redact_json_text),
    })
}

#[tauri::command]
pub fn load_provider_catalog() -> Result<ProviderCatalog, String> {
    load_provider_catalog_from_disk()
}

#[tauri::command]
pub fn preview_changes(input: ConfigInput) -> Result<PreviewResult, String> {
    let normalized = validate_input(input)?;
    let mut files = Vec::new();
    let mut env_updates = Vec::new();
    let mut warnings = normalized.warnings.clone();

    for platform in &normalized.platforms {
        preview_platform(
            platform,
            &normalized.api_key,
            &mut files,
            &mut env_updates,
            &mut warnings,
        )?;
    }

    Ok(PreviewResult {
        files,
        env_updates,
        warnings,
    })
}

#[tauri::command]
pub fn apply_configs(input: ConfigInput) -> Result<ApplyResult, String> {
    let normalized = validate_input(input)?;
    let mut files = Vec::new();
    let mut env_updates = Vec::new();
    let mut warnings = normalized.warnings.clone();

    for platform in &normalized.platforms {
        apply_platform(
            platform,
            &normalized.api_key,
            &mut files,
            &mut env_updates,
            &mut warnings,
        )?;
    }

    let result = ApplyResult {
        files,
        env_updates,
        tools: tools::detect_tools(),
        warnings,
    };
    save_latest_apply_result_to_disk(&result)?;
    Ok(result)
}

#[tauri::command]
pub fn migrate_legacy_codex_config(api_key: Option<String>) -> Result<Option<ApplyResult>, String> {
    let catalog = load_provider_catalog_from_disk()?;
    let Some(provider) = catalog
        .providers
        .iter()
        .find(|provider| provider.id == catalog.default_provider_id)
    else {
        return Ok(None);
    };
    let Some(platform) = provider.platforms.get(PLATFORM_CODEX) else {
        return Ok(None);
    };

    let mut files = Vec::new();
    let mut warnings = catalog.warning.into_iter().collect::<Vec<_>>();
    let migrated = codex::migrate_legacy_config(
        &platform.writer,
        platform.defaults.model.as_deref(),
        &platform.defaults.base_url,
        api_key.as_deref(),
        &mut files,
        &mut warnings,
    )?;
    if !migrated {
        return Ok(None);
    }

    let result = ApplyResult {
        files,
        env_updates: Vec::new(),
        tools: tools::detect_tools(),
        warnings,
    };
    save_latest_apply_result_to_disk(&result)?;
    Ok(Some(result))
}

#[tauri::command]
pub fn restore_backup(target: String, backup_path: String) -> Result<RestoreResult, String> {
    restore_backup_file(target, backup_path)
}

#[tauri::command]
pub fn load_latest_apply_result() -> Result<Option<ApplyResult>, String> {
    load_latest_apply_result_from_disk()
}

#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let parsed = validate_external_url(&url)?;
    tauri_plugin_opener::open_url(parsed.as_str(), None::<&str>)
        .map_err(|err| format!("打开浏览器失败：{err}"))
}

pub fn validate_external_url(raw: &str) -> Result<Url, String> {
    parse_http_url(raw, "外部链接")
}

fn read_existing_config<F>(target: &str, path: &std::path::Path, redact: F) -> ExistingConfig
where
    F: Fn(String) -> String,
{
    let content = std::fs::read_to_string(path).unwrap_or_default();
    ExistingConfig {
        target: target.to_string(),
        path: display_path(path),
        exists: path.exists(),
        content: redact(content),
    }
}

fn parse_auth_deeplink(url: &str) -> Option<TakoAuthEvent> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "takoswitch" {
        return None;
    }

    let params: std::collections::HashMap<String, String> =
        parsed.query_pairs().into_owned().collect();
    if params.get("resource").map(String::as_str) != Some("auth") {
        return None;
    }

    let key = params.get("key").filter(|value| !value.is_empty())?.clone();
    let state = params
        .get("state")
        .filter(|value| !value.is_empty())
        .cloned();
    Some(TakoAuthEvent { key, state })
}

pub fn handle_deeplink_url(app: &tauri::AppHandle, url: &str) -> bool {
    let Some(auth_event) = parse_auth_deeplink(url) else {
        return false;
    };

    if let Err(err) = app.emit("tako-auth", auth_event) {
        eprintln!("发送 Tako 授权事件失败：{err}");
    }

    focus_main_window(app);
    true
}

pub fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_validation_only_allows_http_and_https() {
        assert!(validate_external_url("https://tako.shiroha.tech/app/authorize").is_ok());
        assert!(validate_external_url("http://localhost:1420").is_ok());
        assert!(validate_external_url("takoswitch://v1/import?resource=auth").is_err());
        assert!(validate_external_url("not a url").is_err());
    }

    #[test]
    fn auth_deeplink_parses_key_and_state() {
        let event =
            parse_auth_deeplink("takoswitch://v1/import?resource=auth&key=cr_abc123&state=s-xyz")
                .unwrap();

        assert_eq!(event.key, "cr_abc123");
        assert_eq!(event.state.as_deref(), Some("s-xyz"));
    }

    #[test]
    fn auth_deeplink_rejects_non_auth_or_missing_key() {
        assert!(parse_auth_deeplink("takoswitch://v1/import?resource=provider&key=cr_x").is_none());
        assert!(parse_auth_deeplink("takoswitch://v1/import?resource=auth").is_none());
        assert!(parse_auth_deeplink("takoswitch://v1/import?resource=auth&key=").is_none());
        assert!(parse_auth_deeplink("https://tako.shiroha.tech/app/authorize").is_none());
    }
}
