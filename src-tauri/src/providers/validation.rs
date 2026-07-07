use super::load_provider_catalog_from_disk;
use super::types::{
    BaseUrlRules, ConfigInput, NormalizedInput, NormalizedPlatformInput, PlatformConfigInput,
    PlatformDefinition, PlatformWriter, ProviderCatalogFile, ProviderDefinition, WriterBinding,
    PLATFORM_CLAUDE, PLATFORM_CODEX, PLATFORM_ORDER, WRITER_CLAUDE_SETTINGS_JSON,
    WRITER_CODEX_CONFIG_TOML,
};
use url::Url;

pub fn validate_provider_catalog_file(file: &ProviderCatalogFile) -> Result<(), String> {
    if file.default_provider_id.trim().is_empty() {
        return Err("defaultProviderId cannot be empty".to_string());
    }
    if file.providers.is_empty() {
        return Err("providers cannot be empty".to_string());
    }

    let mut found_default = false;
    for provider in &file.providers {
        validate_provider_definition(provider)?;
        if provider.id == file.default_provider_id {
            found_default = true;
        }
    }

    if !found_default {
        return Err(format!(
            "default provider {} was not found",
            file.default_provider_id
        ));
    }

    Ok(())
}

fn validate_provider_definition(provider: &ProviderDefinition) -> Result<(), String> {
    let provider_label = if provider.id.trim().is_empty() {
        "(unknown)"
    } else {
        provider.id.as_str()
    };

    require_text(&provider.id, "provider.id")?;
    require_text(&provider.name, "provider.name")?;
    require_text(&provider.account.label, "provider.account.label")?;
    require_text(
        &provider.account.login_status_label,
        "provider.account.loginStatusLabel",
    )?;
    require_text(
        &provider.account.login_description,
        "provider.account.loginDescription",
    )?;
    require_http_url(
        &provider.account.auth_service_url,
        "provider.account.authServiceUrl",
    )?;
    require_http_url(&provider.account.keys_url, "provider.account.keysUrl")?;

    if provider.platforms.is_empty() {
        return Err(format!("provider {provider_label} has no platforms"));
    }

    for (platform_id, platform) in &provider.platforms {
        validate_platform_definition(provider_label, platform_id, platform)?;
    }

    Ok(())
}

fn validate_platform_definition(
    provider_id: &str,
    platform_id: &str,
    platform: &PlatformDefinition,
) -> Result<(), String> {
    require_text(platform_id, "platform id")?;
    require_http_url(
        &platform.defaults.base_url,
        &format!("provider {provider_id} platform {platform_id} defaults.baseUrl"),
    )?;
    validate_writer(provider_id, platform_id, &platform.writer)?;
    Ok(())
}

fn validate_writer(
    provider_id: &str,
    platform_id: &str,
    writer: &PlatformWriter,
) -> Result<(), String> {
    require_text(
        &writer.kind,
        &format!("provider {provider_id} platform {platform_id} writer.kind"),
    )?;

    match writer.kind.as_str() {
        WRITER_CODEX_CONFIG_TOML => {
            require_binding(writer, "apiKey", provider_id, platform_id)?;
            require_constant(writer, "providerId", provider_id, platform_id)?;
            require_constant(writer, "providerName", provider_id, platform_id)?;
            require_constant(writer, "wireApi", provider_id, platform_id)?;
        }
        WRITER_CLAUDE_SETTINGS_JSON => {
            require_binding(writer, "baseUrl", provider_id, platform_id)?;
            require_binding(writer, "apiKey", provider_id, platform_id)?;
            require_binding(writer, "model", provider_id, platform_id)?;
        }
        other => {
            return Err(format!(
                "provider {provider_id} platform {platform_id} has unsupported writer.kind {other}"
            ))
        }
    }

    Ok(())
}

fn require_binding<'a>(
    writer: &'a PlatformWriter,
    field: &str,
    provider_id: &str,
    platform_id: &str,
) -> Result<&'a WriterBinding, String> {
    let binding = writer.bindings.get(field).ok_or_else(|| {
        format!("provider {provider_id} platform {platform_id} is missing writer.bindings.{field}")
    })?;
    require_text(
        &binding.storage,
        &format!("provider {provider_id} platform {platform_id} writer.bindings.{field}.storage"),
    )?;
    require_text(
        &binding.name,
        &format!("provider {provider_id} platform {platform_id} writer.bindings.{field}.name"),
    )?;
    Ok(binding)
}

fn require_constant<'a>(
    writer: &'a PlatformWriter,
    field: &str,
    provider_id: &str,
    platform_id: &str,
) -> Result<&'a str, String> {
    writer
        .constants
        .get(field)
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "provider {provider_id} platform {platform_id} is missing writer.constants.{field}"
            )
        })
}

pub fn require_text(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} cannot be empty"));
    }
    Ok(())
}

pub fn require_http_url(value: &str, field: &str) -> Result<(), String> {
    parse_http_url(value, field).map(|_| ())
}

pub fn parse_http_url(value: &str, field: &str) -> Result<Url, String> {
    let parsed = Url::parse(value.trim())
        .map_err(|_| format!("{field} must be a valid http:// or https:// URL"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!("{field} must start with http:// or https://"));
    }
    Ok(parsed)
}

pub fn validate_input(input: ConfigInput) -> Result<NormalizedInput, String> {
    let catalog = load_provider_catalog_from_disk()?;
    let provider_id = input.provider_id.trim();
    let provider_id = if provider_id.is_empty() {
        catalog.default_provider_id.as_str()
    } else {
        provider_id
    };
    let provider = catalog
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .cloned()
        .ok_or_else(|| format!("Provider not found: {provider_id}"))?;

    let api_key = input.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API key cannot be empty.".to_string());
    }

    let mut platforms = Vec::new();
    for platform_id in PLATFORM_ORDER {
        let Some(platform_input) = input.platforms.get(platform_id) else {
            continue;
        };
        if !platform_input.enabled {
            continue;
        }

        let definition = provider
            .platforms
            .get(platform_id)
            .cloned()
            .ok_or_else(|| format!("Provider {} does not support {platform_id}.", provider.id))?;
        if !definition.enabled {
            return Err(format!(
                "{} is disabled for provider {}.",
                platform_display_name(platform_id),
                provider.name
            ));
        }

        platforms.push(normalize_platform_input(
            platform_id,
            platform_input,
            definition,
        )?);
    }

    if platforms.is_empty() {
        return Err("Please select at least Codex or Claude Code.".to_string());
    }

    Ok(NormalizedInput {
        api_key,
        platforms,
        warnings: catalog.warning.into_iter().collect(),
    })
}

fn normalize_platform_input(
    platform_id: &str,
    input: &PlatformConfigInput,
    definition: PlatformDefinition,
) -> Result<NormalizedPlatformInput, String> {
    let base_url = normalize_base_url(&input.base_url, platform_id)?;
    validate_base_url_rules(platform_id, &base_url, definition.rules.base_url.as_ref())?;

    let model = trim_optional(input.model.clone());
    let model_required = definition
        .rules
        .model
        .as_ref()
        .map(|rules| rules.required)
        .unwrap_or(false);
    if model_required && model.is_none() {
        return Err(format!(
            "{} model cannot be empty when {} is selected.",
            platform_display_name(platform_id),
            platform_display_name(platform_id)
        ));
    }

    Ok(NormalizedPlatformInput {
        id: platform_id.to_string(),
        base_url,
        model,
        definition,
    })
}

fn normalize_base_url(value: &str, platform_id: &str) -> Result<String, String> {
    let trimmed = value.trim().trim_end_matches('/').to_string();
    parse_http_url(
        &trimmed,
        &format!("{} baseUrl", platform_display_name(platform_id)),
    )?;
    Ok(trimmed)
}

fn validate_base_url_rules(
    platform_id: &str,
    base_url: &str,
    rules: Option<&BaseUrlRules>,
) -> Result<(), String> {
    let Some(rules) = rules else {
        return Ok(());
    };
    let parsed = parse_http_url(
        base_url,
        &format!("{} baseUrl", platform_display_name(platform_id)),
    )?;
    let path = parsed.path().trim_end_matches('/');
    for suffix in &rules.forbid_path_suffixes {
        let normalized_suffix = suffix.trim().trim_end_matches('/');
        if !normalized_suffix.is_empty() && path.ends_with(normalized_suffix) {
            return Err(format!(
                "{} baseUrl cannot end with {}.",
                platform_display_name(platform_id),
                suffix
            ));
        }
    }
    Ok(())
}

fn platform_display_name(platform_id: &str) -> &'static str {
    match platform_id {
        PLATFORM_CODEX => "Codex",
        PLATFORM_CLAUDE => "Claude Code",
        _ => "Platform",
    }
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::types::{ConfigPlatforms, DEFAULT_PROVIDER_CONFIG};

    fn platform_input(
        enabled: bool,
        base_url: &str,
        model: Option<&str>,
    ) -> Option<PlatformConfigInput> {
        Some(PlatformConfigInput {
            enabled,
            base_url: base_url.to_string(),
            model: model.map(str::to_string),
        })
    }

    #[test]
    fn validation_rejects_empty_secret_and_bad_url() {
        let input = ConfigInput {
            provider_id: "tako".to_string(),
            api_key: "".to_string(),
            platforms: ConfigPlatforms {
                codex: platform_input(true, "ftp://localhost", Some("gpt-5.4")),
                claude: None,
            },
        };

        assert!(validate_input(input).is_err());
    }

    #[test]
    fn validation_rejects_claude_v1_suffix_from_config_rule() {
        let input = ConfigInput {
            provider_id: "tako".to_string(),
            api_key: "sk-test".to_string(),
            platforms: ConfigPlatforms {
                codex: None,
                claude: platform_input(true, "https://tako.shiroha.tech/v1", None),
            },
        };

        let error = validate_input(input).unwrap_err();
        assert!(error.contains("/v1"));
    }

    #[test]
    fn default_config_validates() {
        let file: ProviderCatalogFile = serde_json::from_str(DEFAULT_PROVIDER_CONFIG).unwrap();
        validate_provider_catalog_file(&file).unwrap();
    }
}
