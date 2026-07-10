use super::*;
use crate::{
    providers::types::{ConfigPlatforms, PlatformOptionsInput, DEFAULT_PROVIDER_CONFIG},
    tests::install_dir_test_lock,
};

fn platform_input(
    enabled: bool,
    base_url: &str,
    model: Option<&str>,
) -> Option<PlatformConfigInput> {
    platform_input_with_options(enabled, base_url, model, Default::default())
}

fn platform_input_with_options(
    enabled: bool,
    base_url: &str,
    model: Option<&str>,
    options: PlatformOptionsInput,
) -> Option<PlatformConfigInput> {
    Some(PlatformConfigInput {
        enabled,
        base_url: base_url.to_string(),
        model: model.map(str::to_string),
        options,
    })
}

#[test]
fn validation_rejects_empty_secret_and_bad_url() {
    let _lock = install_dir_test_lock();
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
    let _lock = install_dir_test_lock();
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
fn validation_rejects_invalid_codex_default_permissions() {
    let _lock = install_dir_test_lock();
    let input = ConfigInput {
        provider_id: "tako".to_string(),
        api_key: "sk-test".to_string(),
        platforms: ConfigPlatforms {
            codex: platform_input_with_options(
                true,
                "https://tako.shiroha.tech/v1",
                Some("gpt-5.4"),
                PlatformOptionsInput {
                    default_permissions: Some(":unknown".to_string()),
                    ..Default::default()
                },
            ),
            claude: None,
        },
    };

    let error = validate_input(input).unwrap_err();
    assert!(error.contains("Codex 权限 profile"));
}

#[test]
fn validation_clears_codex_options_from_claude_input() {
    let _lock = install_dir_test_lock();
    let input = ConfigInput {
        provider_id: "tako".to_string(),
        api_key: "sk-test".to_string(),
        platforms: ConfigPlatforms {
            codex: None,
            claude: platform_input_with_options(
                true,
                "https://tako.shiroha.tech",
                None,
                PlatformOptionsInput {
                    sandbox_mode: Some("danger-full-access".to_string()),
                    approval_policy: Some("never".to_string()),
                    windows_sandbox: Some("elevated".to_string()),
                    default_permissions: Some(":danger-full-access".to_string()),
                    permissions_default_mode: Some("bypassPermissions".to_string()),
                    skip_dangerous_mode_permission_prompt: Some(true),
                    ..Default::default()
                },
            ),
        },
    };

    let normalized = validate_input(input).unwrap();
    let options = &normalized.platforms[0].options;
    assert!(options.sandbox_mode.is_none());
    assert!(options.approval_policy.is_none());
    assert!(options.windows_sandbox.is_none());
    assert!(options.default_permissions.is_none());
    assert_eq!(
        options.permissions_default_mode.as_deref(),
        Some("bypassPermissions")
    );
    assert_eq!(options.skip_dangerous_mode_permission_prompt, Some(true));
}

#[test]
fn validation_rejects_dangerous_prompt_skip_without_claude_bypass() {
    let _lock = install_dir_test_lock();
    let input = ConfigInput {
        provider_id: "tako".to_string(),
        api_key: "sk-test".to_string(),
        platforms: ConfigPlatforms {
            codex: None,
            claude: platform_input_with_options(
                true,
                "https://tako.shiroha.tech",
                None,
                PlatformOptionsInput {
                    permissions_default_mode: Some("auto".to_string()),
                    skip_dangerous_mode_permission_prompt: Some(true),
                    ..Default::default()
                },
            ),
        },
    };

    let error = validate_input(input).unwrap_err();
    assert!(error.contains("绕过权限检查"));
}

#[test]
fn default_config_validates() {
    let file: ProviderCatalogFile = serde_json::from_str(DEFAULT_PROVIDER_CONFIG).unwrap();
    validate_provider_catalog_file(&file).unwrap();
}
