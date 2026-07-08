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
#[path = "../tests/providers/loader.rs"]
mod tests;
