use super::{binding_name, constant_value};
use crate::{
    backups::write_config_file,
    config_paths::codex_config_path,
    env_vars::{cleanup_legacy_codex_api_key, read_legacy_codex_api_key, LEGACY_CODEX_API_KEY_ENV},
    models::{AppliedFile, EnvPreview, FilePreview},
    providers::{NormalizedPlatformInput, PlatformWriter},
    redaction::redact_plain_text,
    utils::{display_path, ensure_trailing_newline},
};
use std::fs;
use toml_edit::{value, DocumentMut, Item, Table};

pub fn preview(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<FilePreview>,
    env_updates: &mut Vec<EnvPreview>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let _ = (env_updates, warnings);
    let path = codex_config_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_config(
        &before,
        platform.model.as_deref().unwrap_or_default(),
        &platform.base_url,
        api_key,
        &platform.definition.writer,
    )?;

    files.push(FilePreview {
        target: platform.id.clone(),
        path: display_path(&path),
        exists: path.exists(),
        backup_path: display_path(&crate::backups::make_backup_path(&platform.id, &path)?),
        before: redact_plain_text(before),
        after: redact_plain_text(after),
    });

    Ok(())
}

pub fn apply(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<AppliedFile>,
    env_updates: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let _ = env_updates;

    let path = codex_config_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_config(
        &before,
        platform.model.as_deref().unwrap_or_default(),
        &platform.base_url,
        api_key,
        &platform.definition.writer,
    )?;
    files.push(write_config_file(&platform.id, &path, &after)?);
    if let Err(err) = cleanup_legacy_codex_api_key() {
        warnings.push(format!(
            "旧版 Codex 密钥环境变量 {LEGACY_CODEX_API_KEY_ENV} 清理失败：{err}"
        ));
    }
    Ok(())
}

pub fn merge_config(
    existing: &str,
    codex_model: &str,
    gateway_base_url: &str,
    api_key: &str,
    writer: &PlatformWriter,
) -> Result<String, String> {
    let provider_id = constant_value(writer, "providerId")?;
    let provider_name = constant_value(writer, "providerName")?;
    let wire_api = constant_value(writer, "wireApi")?;
    let api_key_field = binding_name(writer, "apiKey")?;

    let mut document = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .map_err(|err| format!("现有 Codex 配置不是有效的 TOML：{err}"))?
    };

    document["model"] = value(codex_model);
    document["model_provider"] = value(provider_id);

    let providers = document
        .as_table_mut()
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    if !providers.is_table() {
        *providers = Item::Table(Table::new());
    }

    let providers_table = providers.as_table_mut().expect("table checked");
    let provider = providers_table
        .entry(provider_id)
        .or_insert(Item::Table(Table::new()));
    if !provider.is_table() {
        *provider = Item::Table(Table::new());
    }

    let provider_table = provider.as_table_mut().expect("table checked");
    provider_table.remove("auth");
    provider_table.remove("env_key");
    provider_table.remove("env_key_instructions");
    provider_table.remove("requires_openai_auth");
    provider_table.insert("name", value(provider_name));
    provider_table.insert("base_url", value(gateway_base_url));
    provider_table.insert("wire_api", value(wire_api));
    provider_table.insert(api_key_field, value(api_key));

    Ok(ensure_trailing_newline(document.to_string()))
}

pub fn migrate_legacy_config(
    writer: &PlatformWriter,
    default_model: Option<&str>,
    default_base_url: &str,
    fallback_api_key: Option<&str>,
    files: &mut Vec<AppliedFile>,
    warnings: &mut Vec<String>,
) -> Result<bool, String> {
    let path = codex_config_path()?;
    if !path.exists() {
        return Ok(false);
    }

    let before = fs::read_to_string(&path).map_err(|err| format!("读取 Codex 配置失败：{err}"))?;
    let Some(legacy) = legacy_tako_config(&before, writer, default_model, default_base_url)? else {
        return Ok(false);
    };

    let api_key = match read_legacy_codex_api_key()? {
        Some(value) => value,
        None => fallback_api_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_default(),
    };
    if api_key.is_empty() {
        return Ok(false);
    }

    let after = merge_config(&before, &legacy.model, &legacy.base_url, &api_key, writer)?;
    if after == before {
        return Ok(false);
    }

    files.push(write_config_file("codex", &path, &after)?);
    if let Err(err) = cleanup_legacy_codex_api_key() {
        warnings.push(format!(
            "旧版 Codex 密钥环境变量 {LEGACY_CODEX_API_KEY_ENV} 清理失败：{err}"
        ));
    }
    Ok(true)
}

struct LegacyTakoConfig {
    model: String,
    base_url: String,
}

fn legacy_tako_config(
    existing: &str,
    writer: &PlatformWriter,
    default_model: Option<&str>,
    default_base_url: &str,
) -> Result<Option<LegacyTakoConfig>, String> {
    let provider_id = constant_value(writer, "providerId")?;
    let api_key_field = binding_name(writer, "apiKey")?;

    let document = existing
        .parse::<DocumentMut>()
        .map_err(|err| format!("现有 Codex 配置不是有效的 TOML：{err}"))?;

    let Some(provider) = document
        .get("model_providers")
        .and_then(|item| item.get(provider_id))
        .and_then(Item::as_table)
    else {
        return Ok(None);
    };

    if provider.get(api_key_field).and_then(Item::as_str).is_some() {
        return Ok(None);
    }

    if provider.get("env_key").and_then(Item::as_str) != Some(LEGACY_CODEX_API_KEY_ENV) {
        return Ok(None);
    }

    let model = document
        .get("model")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(default_model)
        .unwrap_or_default()
        .to_string();
    let base_url = provider
        .get("base_url")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_base_url)
        .to_string();

    Ok(Some(LegacyTakoConfig { model, base_url }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config_paths::install_dir_test_lock,
        providers::{
            loader::parse_provider_catalog_file,
            types::{DEFAULT_PROVIDER_CONFIG, PLATFORM_CODEX},
        },
    };
    use chrono::Local;
    use std::{env, fs};

    fn default_writer() -> PlatformWriter {
        parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG)
            .unwrap()
            .providers
            .into_iter()
            .find(|provider| provider.id == "tako")
            .unwrap()
            .platforms
            .get(PLATFORM_CODEX)
            .unwrap()
            .writer
            .clone()
    }

    fn isolated_codex_home(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
        let lock = install_dir_test_lock();
        let root = env::temp_dir().join(format!(
            "tako-switch-{name}-{}",
            Local::now().format("%Y%m%d%H%M%S%3f")
        ));
        fs::create_dir_all(&root).unwrap();
        env::set_var("CODEX_HOME", &root);
        env::set_var("TAKO_SWITCH_DATA_DIR", root.join("app-data"));
        (lock, root)
    }

    fn clear_isolated_env(root: &std::path::Path) {
        env::remove_var("CODEX_HOME");
        env::remove_var("TAKO_SWITCH_DATA_DIR");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn merge_preserves_existing_fields_and_is_idempotent() {
        let existing = r#"
approval_policy = "on-request"

[model_providers.other]
name = "Other"
"#;
        let writer = default_writer();
        let first = merge_config(
            existing,
            "gpt-5.4",
            "http://127.0.0.1:3000/v1",
            "sk-test-123456",
            &writer,
        )
        .unwrap();
        let second = merge_config(
            &first,
            "gpt-5.4",
            "http://127.0.0.1:3000/v1",
            "sk-test-123456",
            &writer,
        )
        .unwrap();

        assert_eq!(first, second);
        assert!(first.contains("approval_policy = \"on-request\""));
        assert!(first.contains("model = \"gpt-5.4\""));
        assert!(first.contains("model_provider = \"tako_proxy\""));
        assert!(first.contains("wire_api = \"responses\""));
        assert!(first.contains("experimental_bearer_token = \"sk-test-123456\""));
        assert!(!first.contains("env_key"));
    }

    #[test]
    fn merge_removes_conflicting_auth_fields() {
        let existing = r#"
[model_providers.tako_proxy]
name = "Old Tako"
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
env_key_instructions = "set env"
requires_openai_auth = true
wire_api = "responses"

[model_providers.tako_proxy.auth]
command = "fetch-token"
"#;
        let writer = default_writer();
        let merged = merge_config(
            existing,
            "gpt-5.4",
            "https://tako.shiroha.tech/v1",
            "sk-new-123456",
            &writer,
        )
        .unwrap();

        assert!(merged.contains("experimental_bearer_token = \"sk-new-123456\""));
        assert!(!merged.contains("env_key"));
        assert!(!merged.contains("env_key_instructions"));
        assert!(!merged.contains("requires_openai_auth"));
        assert!(!merged.contains("[model_providers.tako_proxy.auth]"));
    }

    #[test]
    fn migration_converts_legacy_env_key_config_with_session_key() {
        let (_lock, root) = isolated_codex_home("codex-migration");
        let config_path = root.join("config.toml");
        fs::write(
            &config_path,
            r#"
model = "gpt-old"
model_provider = "tako_proxy"

[model_providers.tako_proxy]
name = "Tako LLM Gateway"
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
requires_openai_auth = true
wire_api = "responses"
"#,
        )
        .unwrap();

        let writer = default_writer();
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        let migrated = migrate_legacy_config(
            &writer,
            Some("gpt-5.4"),
            "https://tako.shiroha.tech/v1",
            Some("sk-session-123456"),
            &mut files,
            &mut warnings,
        )
        .unwrap();
        let after = fs::read_to_string(&config_path).unwrap();

        assert!(migrated);
        assert_eq!(files.len(), 1);
        assert!(after.contains("model = \"gpt-old\""));
        assert!(after.contains("base_url = \"https://old.example.com/v1\""));
        assert!(after.contains("experimental_bearer_token = \"sk-session-123456\""));
        assert!(!after.contains("env_key"));
        assert!(!after.contains("requires_openai_auth"));

        let mut second_files = Vec::new();
        let second = migrate_legacy_config(
            &writer,
            Some("gpt-5.4"),
            "https://tako.shiroha.tech/v1",
            Some("sk-session-123456"),
            &mut second_files,
            &mut warnings,
        )
        .unwrap();
        assert!(!second);
        assert!(second_files.is_empty());

        clear_isolated_env(&root);
    }

    #[test]
    fn migration_skips_without_key_source() {
        let (_lock, root) = isolated_codex_home("codex-migration-no-key");
        let config_path = root.join("config.toml");
        let before = r#"
[model_providers.tako_proxy]
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
"#;
        fs::write(&config_path, before).unwrap();

        let writer = default_writer();
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        let migrated = migrate_legacy_config(
            &writer,
            Some("gpt-5.4"),
            "https://tako.shiroha.tech/v1",
            None,
            &mut files,
            &mut warnings,
        )
        .unwrap();

        assert!(!migrated);
        assert!(files.is_empty());
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);

        clear_isolated_env(&root);
    }
}
