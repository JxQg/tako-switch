use super::{
    types::{ProviderCatalog, ProviderCatalogFile, DEFAULT_PROVIDER_CONFIG},
    validation::validate_provider_catalog_file,
};
use crate::{config_paths::provider_config_path, utils::display_path};
use std::{fs, io};

pub fn load_provider_catalog_from_disk() -> Result<ProviderCatalog, String> {
    let config_path = provider_config_path()?;
    match fs::read_to_string(&config_path) {
        Ok(content) => match parse_provider_catalog_file(&content) {
            Ok(file) => Ok(provider_catalog_from_file(
                file,
                display_path(&config_path),
                None,
            )),
            Err(err) => load_default_provider_catalog(Some(format!(
                "服务商配置 {} 无效：{err}。已使用内置默认配置。",
                display_path(&config_path)
            ))),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            load_default_provider_catalog(Some(format!(
                "未找到服务商配置 {}。已使用内置默认配置。",
                display_path(&config_path)
            )))
        }
        Err(err) => load_default_provider_catalog(Some(format!(
            "读取服务商配置 {} 失败：{err}。已使用内置默认配置。",
            display_path(&config_path)
        ))),
    }
}

pub fn load_default_provider_catalog(warning: Option<String>) -> Result<ProviderCatalog, String> {
    let file = parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG)
        .map_err(|err| format!("内置服务商配置无效：{err}"))?;
    Ok(provider_catalog_from_file(
        file,
        "内置默认配置".to_string(),
        warning,
    ))
}

pub fn parse_provider_catalog_file(content: &str) -> Result<ProviderCatalogFile, String> {
    let file: ProviderCatalogFile =
        serde_json::from_str(content).map_err(|err| format!("JSON 格式无效：{err}"))?;
    validate_provider_catalog_file(&file)?;
    Ok(file)
}

fn provider_catalog_from_file(
    file: ProviderCatalogFile,
    source: String,
    warning: Option<String>,
) -> ProviderCatalog {
    ProviderCatalog {
        default_provider_id: file.default_provider_id,
        providers: file.providers,
        source,
        warning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config_paths::{
            install_dir_test_lock, TEST_PROVIDER_CONFIG_DIR, TEST_PROVIDER_CONFIG_FILE,
        },
        providers::types::{PLATFORM_CLAUDE, PLATFORM_CODEX},
    };
    use chrono::Local;
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
    fn provider_catalog_prefers_external_config() {
        let _lock = install_dir_test_lock();
        let install_dir = env::temp_dir().join(format!(
            "tako-switch-provider-test-{}",
            Local::now().format("%Y%m%d%H%M%S%3f")
        ));
        let config_dir = install_dir.join(TEST_PROVIDER_CONFIG_DIR);
        fs::create_dir_all(&config_dir).unwrap();
        let mut file = parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG).unwrap();
        file.providers[0].name = "External Tako".to_string();
        let content = serde_json::to_string_pretty(&file).unwrap();
        fs::write(config_dir.join(TEST_PROVIDER_CONFIG_FILE), content).unwrap();
        env::set_var("TAKO_SWITCH_INSTALL_DIR", &install_dir);

        let catalog = load_provider_catalog_from_disk().unwrap();

        assert_eq!(catalog.providers[0].name, "External Tako");
        assert!(catalog.warning.is_none());

        env::remove_var("TAKO_SWITCH_INSTALL_DIR");
        let _ = fs::remove_dir_all(&install_dir);
    }

    #[test]
    fn provider_catalog_falls_back_when_external_config_is_invalid() {
        let _lock = install_dir_test_lock();
        let install_dir = env::temp_dir().join(format!(
            "tako-switch-provider-invalid-test-{}",
            Local::now().format("%Y%m%d%H%M%S%3f")
        ));
        let config_dir = install_dir.join(TEST_PROVIDER_CONFIG_DIR);
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join(TEST_PROVIDER_CONFIG_FILE), "{ nope").unwrap();
        env::set_var("TAKO_SWITCH_INSTALL_DIR", &install_dir);

        let catalog = load_provider_catalog_from_disk().unwrap();

        assert_eq!(catalog.default_provider_id, "tako");
        assert!(catalog.warning.unwrap().contains("已使用内置默认配置"));

        env::remove_var("TAKO_SWITCH_INSTALL_DIR");
        let _ = fs::remove_dir_all(&install_dir);
    }
}
