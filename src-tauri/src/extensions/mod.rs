pub mod mcp;
pub mod prompts;

pub use crate::sync_hub::SyncPlatform as ExtPlatform;

use crate::adapters::{claude::ClaudeAdapter, AppMode, DesktopAppAdapter};
use crate::config::{backup_file_once, read_switch_state, write_atomic, AppPaths};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

static STORE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn store_lock() -> Result<MutexGuard<'static, ()>, String> {
    STORE_LOCK
        .lock()
        .map_err(|_| "扩展中心正在恢复，请重启后重试".to_string())
}

pub fn extensions_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("extensions")
}

pub fn skills_ssot_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("skills")
}

pub fn platform_key(platform: ExtPlatform) -> &'static str {
    match platform {
        ExtPlatform::Claude => "claude",
        ExtPlatform::Codex => "codex",
        ExtPlatform::Antigravity => "antigravity",
    }
}

pub fn mcp_config_paths(paths: &AppPaths, platform: ExtPlatform) -> Vec<PathBuf> {
    match platform {
        ExtPlatform::Claude => {
            let mut targets = vec![paths.claude_1p_dir.join("claude_desktop_config.json")];
            if paths.claude_3p_dir.exists() || ClaudeAdapter.read_mode(paths) == AppMode::Tako {
                targets.push(paths.claude_3p_dir.join("claude_desktop_config.json"));
            }
            targets.push(paths.home.join(".claude.json"));
            targets
        }
        ExtPlatform::Codex => vec![paths.codex_dir.join("config.toml")],
        ExtPlatform::Antigravity => vec![paths.agy_gemini_config_dir.join("mcp_config.json")],
    }
}

pub fn skills_dir(paths: &AppPaths, platform: ExtPlatform) -> PathBuf {
    match platform {
        ExtPlatform::Claude => paths.home.join(".claude").join("skills"),
        ExtPlatform::Codex => paths.codex_dir.join("skills"),
        ExtPlatform::Antigravity => paths.agy_gemini_config_dir.join("skills"),
    }
}

pub fn skills_dirs(paths: &AppPaths, platform: ExtPlatform) -> Vec<PathBuf> {
    let mut dirs = vec![skills_dir(paths, platform)];
    if platform == ExtPlatform::Antigravity {
        for name in ["antigravity", "antigravity-cli"] {
            let candidate = paths.home.join(".gemini").join(name).join("skills");
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        }
    }
    dirs
}

pub fn prompt_path(paths: &AppPaths, platform: ExtPlatform) -> PathBuf {
    match platform {
        ExtPlatform::Claude => paths.home.join(".claude").join("CLAUDE.md"),
        ExtPlatform::Codex => paths.codex_dir.join("AGENTS.md"),
        ExtPlatform::Antigravity => paths.home.join(".gemini").join("AGENTS.md"),
    }
}

pub fn read_json_store<T: DeserializeOwned + Default>(path: &Path) -> T {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
            eprintln!(
                "读取扩展中心存储 {} 失败，使用兼容默认值: {err}",
                path.display()
            );
            T::default()
        }),
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                eprintln!("读取扩展中心存储 {} 失败: {err}", path.display());
            }
            T::default()
        }
    }
}

pub fn write_json_store<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    write_atomic(path, &json_bytes(value)?, true)
}

pub(crate) fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value).map_err(|err| format!("序列化扩展配置失败: {err}"))
}

pub fn build_http_client_for_paths(paths: &AppPaths) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION")));
    if read_switch_state(paths).bypass_system_proxy {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|err| format!("创建扩展网络客户端失败: {err}"))
}

pub fn validate_extension_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 160
        || id.starts_with(['.', '-'])
        || id.contains("..")
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(
            "扩展标识只能包含字母、数字、点、下划线和连字符，且不能以点或连字符开头".into(),
        );
    }
    Ok(())
}

pub(crate) struct FileUpdate {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub backup: Option<(String, String)>,
}

impl FileUpdate {
    pub fn store<T: Serialize>(path: PathBuf, value: &T) -> Result<Self, String> {
        Ok(Self {
            path,
            bytes: json_bytes(value)?,
            backup: None,
        })
    }

    pub fn client(path: PathBuf, bytes: Vec<u8>, platform: ExtPlatform, backup_key: &str) -> Self {
        Self {
            path,
            bytes,
            backup: Some((platform_key(platform).into(), backup_key.into())),
        }
    }
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("读取 {} 失败: {err}", path.display())),
    }
}

pub(crate) fn apply_file_updates(paths: &AppPaths, updates: Vec<FileUpdate>) -> Result<(), String> {
    let mut pending = Vec::new();
    let mut seen = BTreeSet::new();
    for update in updates {
        if !seen.insert(update.path.clone()) {
            return Err(format!("同一次操作包含重复目标: {}", update.path.display()));
        }
        let before = read_optional_file(&update.path)?;
        if before.as_deref() != Some(update.bytes.as_slice()) {
            pending.push((update, before));
        }
    }
    for (update, _) in &pending {
        if let Some((app_key, backup_key)) = &update.backup {
            backup_file_once(paths, app_key, backup_key, &update.path)?;
        }
    }
    for (index, (update, _)) in pending.iter().enumerate() {
        if let Err(err) = write_atomic(&update.path, &update.bytes, true) {
            let mut failures = vec![err];
            // Include the failed target: the platform atomic writer may already have replaced it.
            for (written, before) in pending[..=index].iter().rev() {
                let result = match before {
                    Some(bytes) => write_atomic(&written.path, bytes, true),
                    None if written.path.exists() => {
                        fs::remove_file(&written.path).map_err(|rollback| {
                            format!("清理 {} 失败: {rollback}", written.path.display())
                        })
                    }
                    None => Ok(()),
                };
                if let Err(rollback) = result {
                    failures.push(rollback);
                }
            }
            return Err(failures.join("\n"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionsSummary {
    #[serde(default)]
    pub mcp_count: usize,
    #[serde(default)]
    pub skills_count: usize,
    #[serde(default)]
    pub plugins_count: usize,
    #[serde(default)]
    pub prompts_count: usize,
}

pub fn get_extensions_summary_for_paths(paths: &AppPaths) -> Result<ExtensionsSummary, String> {
    let mcp_count = mcp::get_mcp_hub_snapshot_for_paths(paths)?.entries.len();
    let prompts = prompts::get_prompts_hub_snapshot_for_paths(paths)?;
    let mut skills = BTreeSet::new();
    for directory in ExtPlatform::all()
        .iter()
        .flat_map(|&platform| skills_dirs(paths, platform))
        .chain(std::iter::once(skills_ssot_dir(paths)))
    {
        if let Ok(entries) = fs::read_dir(directory) {
            for entry in entries.flatten() {
                if entry.path().join("SKILL.md").is_file() {
                    skills.insert(entry.file_name());
                }
            }
        }
    }
    Ok(ExtensionsSummary {
        mcp_count,
        skills_count: skills.len(),
        plugins_count: 0,
        prompts_count: prompts
            .platforms
            .values()
            .map(|platform| platform.presets.len())
            .sum(),
    })
}
