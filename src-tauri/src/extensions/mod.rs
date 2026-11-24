pub mod skills;

pub use crate::sync_hub::SyncPlatform as ExtPlatform;

use crate::config::{apply_proxy_policy, read_switch_state, write_atomic, AppPaths};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn extensions_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("extensions")
}

pub fn platform_skills_dirs(paths: &AppPaths, platform: ExtPlatform) -> Vec<PathBuf> {
    match platform {
        ExtPlatform::Claude => vec![paths.home.join(".claude").join("skills")],
        ExtPlatform::Codex => vec![paths.codex_dir.join("skills")],
        ExtPlatform::Antigravity => {
            let mut roots = vec![paths.agy_gemini_config_dir.join("skills")];
            for name in ["antigravity", "antigravity-cli"] {
                let root = paths.home.join(".gemini").join(name).join("skills");
                if root.is_dir() {
                    roots.push(root);
                }
            }
            roots
        }
    }
}

pub fn read_json_store<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("读取扩展配置 {} 失败: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("读取扩展配置 {} 失败: {e}", path.display())),
    }
}

pub fn write_json_store<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|e| format!("序列化扩展配置 {} 失败: {e}", path.display()))?;
    write_atomic(path, &bytes, false)
}

pub fn http_client_for_paths(paths: &AppPaths) -> Result<reqwest::Client, String> {
    apply_proxy_policy(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION"))),
        read_switch_state(paths).bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("创建扩展下载客户端失败: {e}"))
}

pub fn validate_identifier(value: &str) -> Result<(), String> {
    let reserved = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if value.is_empty()
        || value.len() > 128
        || value.starts_with(['.', '-'])
        || value.ends_with(['.', ' '])
        || value.contains("..")
        || !value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || matches!(reserved.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (reserved.len() == 4
            && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
            && matches!(reserved.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(format!("非法扩展标识符: {value}"));
    }
    Ok(())
}
