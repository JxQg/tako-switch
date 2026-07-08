use super::*;
use crate::{
    providers::types::{ConfigPlatforms, DEFAULT_PROVIDER_CONFIG},
    tests::install_dir_test_lock,
};

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
fn default_config_validates() {
    let file: ProviderCatalogFile = serde_json::from_str(DEFAULT_PROVIDER_CONFIG).unwrap();
    validate_provider_catalog_file(&file).unwrap();
}
