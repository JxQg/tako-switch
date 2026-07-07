pub mod claude;
pub mod codex;

use crate::{
    models::{AppliedFile, EnvPreview, FilePreview},
    providers::{NormalizedPlatformInput, WRITER_CLAUDE_SETTINGS_JSON, WRITER_CODEX_CONFIG_TOML},
};

pub fn preview_platform(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<FilePreview>,
    env_updates: &mut Vec<EnvPreview>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    match platform.definition.writer.kind.as_str() {
        WRITER_CODEX_CONFIG_TOML => codex::preview(platform, api_key, files, env_updates, warnings),
        WRITER_CLAUDE_SETTINGS_JSON => claude::preview(platform, api_key, files),
        other => Err(format!("Unsupported writer kind: {other}")),
    }
}

pub fn apply_platform(
    platform: &NormalizedPlatformInput,
    api_key: &str,
    files: &mut Vec<AppliedFile>,
    env_updates: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    match platform.definition.writer.kind.as_str() {
        WRITER_CODEX_CONFIG_TOML => codex::apply(platform, api_key, files, env_updates, warnings),
        WRITER_CLAUDE_SETTINGS_JSON => claude::apply(platform, api_key, files),
        other => Err(format!("Unsupported writer kind: {other}")),
    }
}

pub fn binding_name<'a>(
    writer: &'a crate::providers::PlatformWriter,
    field: &str,
) -> Result<&'a str, String> {
    writer
        .bindings
        .get(field)
        .map(|binding| binding.name.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("writer is missing binding for {field}"))
}

pub fn constant_value<'a>(
    writer: &'a crate::providers::PlatformWriter,
    field: &str,
) -> Result<&'a str, String> {
    writer
        .constants
        .get(field)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("writer is missing constant {field}"))
}
