use super::*;
use crate::{
    providers::types::{PLATFORM_CLAUDE, PLATFORM_CODEX},
    tests::{install_dir_test_lock, provider_config_dir, provider_config_file, unique_temp_dir},
};
use std::{env, fs};

#[test]
fn built_in_provider_config_uses_unified_platform_schema() {
    let file = parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG).unwrap();
    let provider = file
        .providers
        .iter()
        .find(|provider| provider.id == file.default_provider_id)
        .unwrap();
    let codex = provider.platforms.get(PLATFORM_CODEX).unwrap();
    let claude = provider.platforms.get(PLATFORM_CLAUDE).unwrap();

    assert_eq!(codex.defaults.base_url, "https://tako.shiroha.tech/v1");
    assert_eq!(claude.defaults.base_url, "https://tako.shiroha.tech");
    assert!(codex.writer.bindings.contains_key("apiKey"));
    assert!(claude.writer.bindings.contains_key("baseUrl"));
    assert!(claude.writer.bindings.contains_key("apiKey"));
    assert!(claude.writer.bindings.contains_key("model"));
}

#[test]
fn provider_catalog_uses_built_in_without_warning_when_user_config_missing() {
    let _lock = install_dir_test_lock();
    let data_dir = unique_temp_dir("provider-missing-test");
    env::set_var("TAKO_SWITCH_INSTALL_DIR", &data_dir);

    let catalog = load_provider_catalog_from_disk().unwrap();

    assert_eq!(catalog.default_provider_id, "tako");
    assert!(catalog.warning.is_none());
    assert_eq!(catalog.source, "内置默认配置");

    env::remove_var("TAKO_SWITCH_INSTALL_DIR");
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn provider_catalog_prefers_user_config() {
    let _lock = install_dir_test_lock();
    let data_dir = unique_temp_dir("provider-test");
    let config_dir = data_dir.join(provider_config_dir());
    fs::create_dir_all(&config_dir).unwrap();
    let mut file = parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG).unwrap();
    file.providers[0].name = "External Tako".to_string();
    let content = serde_json::to_string_pretty(&file).unwrap();
    fs::write(config_dir.join(provider_config_file()), content).unwrap();
    env::set_var("TAKO_SWITCH_INSTALL_DIR", &data_dir);

    let catalog = load_provider_catalog_from_disk().unwrap();

    assert_eq!(catalog.providers[0].name, "External Tako");
    assert!(catalog.warning.is_none());

    env::remove_var("TAKO_SWITCH_INSTALL_DIR");
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn provider_catalog_falls_back_when_external_config_is_invalid() {
    let _lock = install_dir_test_lock();
    let data_dir = unique_temp_dir("provider-invalid-test");
    let config_dir = data_dir.join(provider_config_dir());
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(config_dir.join(provider_config_file()), "{ nope").unwrap();
    env::set_var("TAKO_SWITCH_INSTALL_DIR", &data_dir);

    let catalog = load_provider_catalog_from_disk().unwrap();

    assert_eq!(catalog.default_provider_id, "tako");
    assert!(catalog.warning.unwrap().contains("已使用内置默认配置"));

    env::remove_var("TAKO_SWITCH_INSTALL_DIR");
    let _ = fs::remove_dir_all(&data_dir);
}

#[test]
fn provider_catalog_uses_bundled_config_when_user_config_missing() {
    let _lock = install_dir_test_lock();
    let install_dir = unique_temp_dir("provider-bundled-install-test");
    let bundled_dir = unique_temp_dir("provider-bundled-resource-test");
    let config_dir = bundled_dir.join(provider_config_dir());
    fs::create_dir_all(&config_dir).unwrap();
    let mut file = parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG).unwrap();
    file.providers[0].name = "Bundled Tako".to_string();
    let content = serde_json::to_string_pretty(&file).unwrap();
    let bundled_path = config_dir.join(provider_config_file());
    fs::write(&bundled_path, content).unwrap();
    env::set_var("TAKO_SWITCH_INSTALL_DIR", &install_dir);

    let catalog = load_provider_catalog_with_bundled_path(bundled_path.clone()).unwrap();

    assert_eq!(catalog.providers[0].name, "Bundled Tako");
    assert!(catalog.warning.is_none());
    assert_eq!(catalog.source, bundled_path.to_string_lossy());

    env::remove_var("TAKO_SWITCH_INSTALL_DIR");
    let _ = fs::remove_dir_all(&install_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
}
