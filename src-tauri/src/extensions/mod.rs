pub mod plugins;

pub use crate::sync_hub::SyncPlatform as ExtPlatform;

use crate::config::{apply_proxy_policy, read_switch_state, write_atomic, AppPaths};
use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;
use std::time::Duration;

pub fn extensions_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("extensions")
}

pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn read_json_store<T: DeserializeOwned + Default>(
    paths: &AppPaths,
    file_name: &str,
) -> Result<T, String> {
    let path = extensions_dir(paths).join(file_name);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("解析 {} 失败: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(format!("读取 {} 失败: {error}", path.display())),
    }
}

pub(crate) fn write_json_store<T: Serialize>(
    paths: &AppPaths,
    file_name: &str,
    store: &T,
) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(store).map_err(|error| format!("序列化扩展配置失败: {error}"))?;
    write_atomic(&extensions_dir(paths).join(file_name), &bytes, false)
}

pub(crate) fn http_client_for_paths(paths: &AppPaths) -> Result<reqwest::Client, String> {
    apply_proxy_policy(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION"))),
        read_switch_state(paths).bypass_system_proxy,
    )
    .build()
    .map_err(|error| format!("创建扩展目录客户端失败: {error}"))
}
