use super::{
    types::{ProviderCatalog, ProviderCatalogFile, DEFAULT_PROVIDER_CONFIG},
    validation::validate_provider_catalog_file,
};
use crate::{
    config_paths::{bundled_provider_config_relative_path, provider_config_path},
    utils::display_path,
};
use std::{fs, io};
use tauri::{AppHandle, Manager, Runtime};

#[cfg(test)]
pub fn load_provider_catalog_from_disk() -> Result<ProviderCatalog, String> {
    load_provider_catalog_from_sources(None)
}

pub fn load_provider_catalog_from_app<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<ProviderCatalog, String> {
    let resource_path = app
        .path()
        .resource_dir()
        .map(|dir| dir.join(bundled_provider_config_relative_path()))
        .map_err(|err| err.to_string());

    load_provider_catalog_from_sources(Some(resource_path))
}

#[cfg(test)]
pub fn load_provider_catalog_with_bundled_path(
    bundled_config_path: std::path::PathBuf,
) -> Result<ProviderCatalog, String> {
    load_provider_catalog_from_sources(Some(Ok(bundled_config_path)))
}

fn load_provider_catalog_from_sources(
    bundled_config_path: Option<Result<std::path::PathBuf, String>>,
) -> Result<ProviderCatalog, String> {
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
            load_bundled_provider_catalog(bundled_config_path)
        }
        Err(err) => load_default_provider_catalog(Some(format!(
            "读取服务商配置 {} 失败：{err}。已使用内置默认配置。",
            display_path(&config_path)
        ))),
    }
}

fn load_bundled_provider_catalog(
    bundled_config_path: Option<Result<std::path::PathBuf, String>>,
) -> Result<ProviderCatalog, String> {
    let Some(resource_path) = bundled_config_path else {
        return load_default_provider_catalog(None);
    };

    match resource_path {
        Ok(path) => match fs::read_to_string(&path) {
            Ok(content) => match parse_provider_catalog_file(&content) {
                Ok(file) => Ok(provider_catalog_from_file(file, display_path(&path), None)),
                Err(err) => load_default_provider_catalog(Some(format!(
                    "打包服务商配置 {} 无效：{err}。已使用内置默认配置。",
                    display_path(&path)
                ))),
            },
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                load_default_provider_catalog(None)
            }
            Err(err) => load_default_provider_catalog(Some(format!(
                "读取打包服务商配置 {} 失败：{err}。已使用内置默认配置。",
                display_path(&path)
            ))),
        },
        Err(err) => load_default_provider_catalog(Some(format!(
            "无法确定打包服务商配置目录：{err}。已使用内置默认配置。"
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
