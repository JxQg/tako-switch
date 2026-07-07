use super::{binding_name, constant_value};
use crate::{
    backups::write_config_file,
    config_paths::codex_config_path,
    env_vars::{codex_env_note, profile_warnings, write_user_env_var},
    models::{AppliedFile, EnvPreview, FilePreview},
    providers::{NormalizedPlatformInput, PlatformWriter},
    redaction::{mask_secret, redact_plain_text},
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
    let env_key = binding_name(&platform.definition.writer, "apiKey")?;
    let path = codex_config_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_config(
        &before,
        platform.model.as_deref().unwrap_or_default(),
        &platform.base_url,
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

    env_updates.push(EnvPreview {
        name: env_key.to_string(),
        masked_value: mask_secret(api_key),
        note: codex_env_note(env_key),
    });
    warnings.extend(profile_warnings(env_key));
    Ok(())
}

pub fn apply(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<AppliedFile>,
    env_updates: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let env_key = binding_name(&platform.definition.writer, "apiKey")?;
    env_updates.push(write_user_env_var(env_key, api_key)?);
    warnings.extend(profile_warnings(env_key));

    let path = codex_config_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_config(
        &before,
        platform.model.as_deref().unwrap_or_default(),
        &platform.base_url,
        &platform.definition.writer,
    )?;
    files.push(write_config_file(&platform.id, &path, &after)?);
    Ok(())
}

pub fn merge_config(
    existing: &str,
    codex_model: &str,
    gateway_base_url: &str,
    writer: &PlatformWriter,
) -> Result<String, String> {
    let provider_id = constant_value(writer, "providerId")?;
    let provider_name = constant_value(writer, "providerName")?;
    let wire_api = constant_value(writer, "wireApi")?;
    let env_key = binding_name(writer, "apiKey")?;

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
    provider_table.insert("name", value(provider_name));
    provider_table.insert("base_url", value(gateway_base_url));
    provider_table.insert("env_key", value(env_key));
    provider_table.insert("wire_api", value(wire_api));

    Ok(ensure_trailing_newline(document.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        loader::parse_provider_catalog_file,
        types::{DEFAULT_PROVIDER_CONFIG, PLATFORM_CODEX},
    };

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

    #[test]
    fn merge_preserves_existing_fields_and_is_idempotent() {
        let existing = r#"
approval_policy = "on-request"

[model_providers.other]
name = "Other"
"#;
        let writer = default_writer();
        let first = merge_config(existing, "gpt-5.4", "http://127.0.0.1:3000/v1", &writer).unwrap();
        let second = merge_config(&first, "gpt-5.4", "http://127.0.0.1:3000/v1", &writer).unwrap();

        assert_eq!(first, second);
        assert!(first.contains("approval_policy = \"on-request\""));
        assert!(first.contains("model = \"gpt-5.4\""));
        assert!(first.contains("model_provider = \"tako_proxy\""));
        assert!(first.contains("env_key = \"TAKO_CODEX_API_KEY\""));
        assert!(first.contains("wire_api = \"responses\""));
    }
}
