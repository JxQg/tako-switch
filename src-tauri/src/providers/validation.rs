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
        return Err("defaultProviderId 不能为空。".to_string());
    }
    if file.providers.is_empty() {
        return Err("providers 不能为空。".to_string());
    }

    let mut found_default = false;
    for provider in &file.providers {
        validate_provider_definition(provider)?;
        if provider.id == file.default_provider_id {
            found_default = true;
        }
    }

    if !found_default {
        return Err(format!("未找到默认服务商：{}", file.default_provider_id));
    }

    Ok(())
}

fn validate_provider_definition(provider: &ProviderDefinition) -> Result<(), String> {
    let provider_label = if provider.id.trim().is_empty() {
        "(未知服务商)"
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

    if provider.platforms.is_empty() {
        return Err(format!("服务商 {provider_label} 没有可用平台配置。"));
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
                "服务商 {provider_id} 的平台 {platform_id} 使用了不支持的 writer.kind：{other}"
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
        format!("服务商 {provider_id} 的平台 {platform_id} 缺少 writer.bindings.{field}")
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
            format!("服务商 {provider_id} 的平台 {platform_id} 缺少 writer.constants.{field}")
        })
}

pub fn require_text(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} 不能为空。"));
    }
    Ok(())
}

pub fn require_http_url(value: &str, field: &str) -> Result<(), String> {
    parse_http_url(value, field).map(|_| ())
}

pub fn parse_http_url(value: &str, field: &str) -> Result<Url, String> {
    let parsed = Url::parse(value.trim())
        .map_err(|_| format!("{field} 必须是有效的 http:// 或 https:// 地址。"))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(format!("{field} 必须以 http:// 或 https:// 开头。"));
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
        .ok_or_else(|| format!("未找到服务商：{provider_id}"))?;

    let api_key = input.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key / Token 不能为空。".to_string());
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
            .ok_or_else(|| format!("服务商 {} 不支持 {platform_id}。", provider.id))?;
        if !definition.enabled {
            return Err(format!(
                "服务商 {} 暂未启用 {}。",
                provider.name,
                platform_display_name(platform_id)
            ));
        }

        platforms.push(normalize_platform_input(
            platform_id,
            platform_input,
            definition,
        )?);
    }

    if platforms.is_empty() {
        return Err("请至少选择 Codex 或 Claude Code。".to_string());
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
            "选择 {} 时必须填写 {} 模型。",
            platform_display_name(platform_id),
            platform_display_name(platform_id)
        ));
    }

    Ok(NormalizedPlatformInput {
        id: platform_id.to_string(),
        base_url,
        model,
        options: normalize_platform_options(platform_id, input)?,
        definition,
    })
}

fn normalize_platform_options(
    platform_id: &str,
    input: &PlatformConfigInput,
) -> Result<super::types::PlatformOptionsInput, String> {
    let mut options = input.options.clone();

    match platform_id {
        PLATFORM_CODEX => {
            if let Some(value) = normalize_optional_string(&options.sandbox_mode) {
                match value.as_str() {
                    "read-only" | "workspace-write" | "danger-full-access" => {
                        options.sandbox_mode = Some(value)
                    }
                    _ => return Err("Codex 沙箱权限不是有效选项。".to_string()),
                }
            } else {
                options.sandbox_mode = None;
            }

            if let Some(value) = normalize_optional_string(&options.approval_policy) {
                match value.as_str() {
                    "untrusted" | "on-request" | "never" => options.approval_policy = Some(value),
                    _ => return Err("Codex 审批策略不是有效选项。".to_string()),
                }
            } else {
                options.approval_policy = None;
            }

            if let Some(value) = normalize_optional_string(&options.windows_sandbox) {
                match value.as_str() {
                    "elevated" | "unelevated" => options.windows_sandbox = Some(value),
                    _ => return Err("Windows 沙箱模式不是有效选项。".to_string()),
                }
            } else {
                options.windows_sandbox = None;
            }

            options.permissions_default_mode = None;
            options.skip_dangerous_mode_permission_prompt = None;
        }
        PLATFORM_CLAUDE => {
            if let Some(value) = normalize_optional_string(&options.permissions_default_mode) {
                match value.as_str() {
                    "default" | "acceptEdits" | "plan" | "auto" | "dontAsk"
                    | "bypassPermissions" => options.permissions_default_mode = Some(value),
                    _ => return Err("Claude Code 权限模式不是有效选项。".to_string()),
                }
            } else {
                options.permissions_default_mode = None;
            }

            options.sandbox_mode = None;
            options.approval_policy = None;
            options.windows_sandbox = None;
            options.features = Default::default();
        }
        _ => {}
    }

    Ok(options)
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
                "{} 地址不能以 {} 结尾。",
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

fn normalize_optional_string(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
#[path = "../tests/providers/validation.rs"]
mod tests;
