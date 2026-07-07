use super::binding_name;
use crate::{
    backups::write_config_file,
    config_paths::claude_settings_path,
    models::{AppliedFile, FilePreview},
    providers::{NormalizedPlatformInput, PlatformWriter},
    redaction::redact_json_text,
    utils::{display_path, ensure_trailing_newline},
};
use serde_json::{json, Map, Value};
use std::fs;

pub fn preview(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<FilePreview>,
) -> Result<(), String> {
    let path = claude_settings_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_settings(
        &before,
        &platform.base_url,
        api_key,
        platform.model.as_deref(),
        &platform.definition.writer,
    )?;

    files.push(FilePreview {
        target: platform.id.clone(),
        path: display_path(&path),
        exists: path.exists(),
        backup_path: display_path(&crate::backups::make_backup_path(&platform.id, &path)?),
        before: redact_json_text(before),
        after: redact_json_text(after),
    });
    Ok(())
}

pub fn apply(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<AppliedFile>,
) -> Result<(), String> {
    let path = claude_settings_path()?;
    let before = fs::read_to_string(&path).unwrap_or_default();
    let after = merge_settings(
        &before,
        &platform.base_url,
        api_key,
        platform.model.as_deref(),
        &platform.definition.writer,
    )?;
    files.push(write_config_file(&platform.id, &path, &after)?);
    Ok(())
}

pub fn merge_settings(
    existing: &str,
    gateway_base_url: &str,
    api_key: &str,
    claude_model: Option<&str>,
    writer: &PlatformWriter,
) -> Result<String, String> {
    let base_url_key = binding_name(writer, "baseUrl")?;
    let api_key_key = binding_name(writer, "apiKey")?;
    let model_key = binding_name(writer, "model")?;

    let mut root = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(existing)
            .map_err(|err| format!("Existing Claude settings are not valid JSON: {err}"))?
    };

    if !root.is_object() {
        root = json!({});
    }

    let object = root.as_object_mut().expect("object checked");
    let env_value = object
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !env_value.is_object() {
        *env_value = Value::Object(Map::new());
    }

    let env_object = env_value.as_object_mut().expect("object checked");
    env_object.insert(
        base_url_key.to_string(),
        Value::String(gateway_base_url.to_string()),
    );
    env_object.insert(api_key_key.to_string(), Value::String(api_key.to_string()));
    if let Some(model) = claude_model {
        env_object.insert(model_key.to_string(), Value::String(model.to_string()));
    } else {
        env_object.remove(model_key);
    }

    serde_json::to_string_pretty(&root)
        .map(ensure_trailing_newline)
        .map_err(|err| format!("Failed to render Claude settings JSON: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        loader::parse_provider_catalog_file,
        types::{DEFAULT_PROVIDER_CONFIG, PLATFORM_CLAUDE},
    };

    fn default_writer() -> PlatformWriter {
        parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG)
            .unwrap()
            .providers
            .into_iter()
            .find(|provider| provider.id == "tako")
            .unwrap()
            .platforms
            .get(PLATFORM_CLAUDE)
            .unwrap()
            .writer
            .clone()
    }

    #[test]
    fn merge_preserves_existing_settings_and_env() {
        let existing = r#"{
  "theme": "dark",
  "env": {
    "OTHER": "1"
  }
}"#;
        let writer = default_writer();
        let merged = merge_settings(
            existing,
            "https://gateway.example.com/v1",
            "sk-test-secret",
            Some("claude-custom"),
            &writer,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(value["theme"], "dark");
        assert_eq!(value["env"]["OTHER"], "1");
        assert_eq!(
            value["env"]["ANTHROPIC_BASE_URL"],
            "https://gateway.example.com/v1"
        );
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-test-secret");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-custom");
    }

    #[test]
    fn merge_removes_empty_model() {
        let existing = r#"{"env":{"ANTHROPIC_MODEL":"old"}}"#;
        let writer = default_writer();
        let merged = merge_settings(
            existing,
            "http://localhost:3000/v1",
            "sk-test",
            None,
            &writer,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();

        assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
    }
}
