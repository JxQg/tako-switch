pub mod mcp;
pub mod plugins;
pub mod prompts;
pub mod skills;

use crate::adapters::{claude::ClaudeAdapter, AppMode, DesktopAppAdapter};
use crate::config::{apply_proxy_policy, backup_file_once, read_switch_state, write_atomic, AppPaths};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// 通用扩展中心统一纳管的三端平台（严禁触碰 `~/.dsh-tako`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtPlatform {
    Claude,
    Codex,
    Antigravity,
}

impl ExtPlatform {
    pub fn all() -> &'static [ExtPlatform] {
        &[
            ExtPlatform::Claude,
            ExtPlatform::Codex,
            ExtPlatform::Antigravity,
        ]
    }

    pub fn key(self) -> &'static str {
        match self {
            ExtPlatform::Claude => "claude",
            ExtPlatform::Codex => "codex",
            ExtPlatform::Antigravity => "antigravity",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ExtPlatform::Claude => "Claude",
            ExtPlatform::Codex => "Codex",
            ExtPlatform::Antigravity => "Antigravity",
        }
    }
}

pub fn platform_key(platform: ExtPlatform) -> &'static str {
    platform.key()
}

pub fn normalize_platforms(platforms: &[ExtPlatform]) -> Vec<ExtPlatform> {
    ExtPlatform::all()
        .iter()
        .copied()
        .filter(|platform| platforms.contains(platform))
        .collect()
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn store_lock() -> Result<MutexGuard<'static, ()>, String> {
    Ok(STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()))
}

pub fn extensions_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("extensions")
}

pub fn extension_store_path(paths: &AppPaths, file_name: &str) -> PathBuf {
    extensions_dir(paths).join(file_name)
}

pub fn skills_ssot_dir(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("skills")
}

pub fn codex_config_path(paths: &AppPaths) -> PathBuf {
    paths.codex_dir.join("config.toml")
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
        ExtPlatform::Codex => vec![codex_config_path(paths)],
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

pub fn platform_skills_dirs(paths: &AppPaths, platform: ExtPlatform) -> Vec<PathBuf> {
    skills_dirs(paths, platform)
}

pub fn prompt_path(paths: &AppPaths, platform: ExtPlatform) -> PathBuf {
    match platform {
        ExtPlatform::Claude => paths.home.join(".claude").join("CLAUDE.md"),
        ExtPlatform::Codex => paths.codex_dir.join("AGENTS.md"),
        ExtPlatform::Antigravity => paths.home.join(".gemini").join("AGENTS.md"),
    }
}

/// 严格读取扩展中心 JSON 存储：文件不存在（如低版本平滑升级首次进入）时返回 `Ok(T::default())`。
pub fn read_json_store_checked<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|err| format!("解析扩展配置 {} 失败: {err}", path.display())),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(T::default()),
        Err(err) => Err(format!("读取扩展配置 {} 失败: {err}", path.display())),
    }
}

/// 容错读取扩展中心 JSON 存储：文件不存在或历史结构损坏时自动回退至 `T::default()`，确保低版本升级零崩溃。
pub fn read_json_store<T: DeserializeOwned + Default>(path: &Path) -> T {
    read_json_store_checked(path).unwrap_or_else(|err| {
        eprintln!("{err}，已使用兼容默认值");
        T::default()
    })
}

pub fn write_json_store<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    write_atomic(path, &json_bytes(value)?, false)
}

pub(crate) fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(value).map_err(|err| format!("序列化扩展配置失败: {err}"))
}

/// 统一构建遵循 `SwitchState.bypass_system_proxy` 的 HTTP 客户端。
pub fn build_http_client(
    paths: &AppPaths,
    timeout: Duration,
    follow_redirects: bool,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION")));
    if !follow_redirects {
        builder = builder.redirect(reqwest::redirect::Policy::none());
    }
    apply_proxy_policy(builder, read_switch_state(paths).bypass_system_proxy)
        .build()
        .map_err(|err| format!("创建扩展网络客户端失败: {err}"))
}

pub fn is_safe_ascii_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub fn is_windows_reserved_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

pub fn validate_extension_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 160
        || value.starts_with(['.', '-'])
        || value.ends_with(['.', ' '])
        || value.contains("..")
        || !value
            .chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        || is_windows_reserved_name(value)
    {
        return Err(format!("非法扩展标识符: {value}"));
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
            backup: Some((platform.key().into(), backup_key.into())),
        }
    }
}

pub(crate) fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
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

/// 统一在阻塞线程池中执行基于 `AppPaths` 的扩展中心同步逻辑。
pub async fn with_extension_paths<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&AppPaths) -> Result<T, String> + Send + 'static,
{
    let paths = AppPaths::detect()?;
    tauri::async_runtime::spawn_blocking(move || operation(&paths))
        .await
        .map_err(|err| format!("扩展中心操作失败: {err}"))?
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionsSummary {
    #[serde(default)]
    pub mcp_count: usize,
    #[serde(default)]
    pub skills_count: usize,
    #[serde(default)]
    pub skills_updatable_count: usize,
    #[serde(default)]
    pub plugins_count: usize,
    #[serde(default)]
    pub prompts_count: usize,
}

pub fn get_extensions_summary_for_paths(paths: &AppPaths) -> Result<ExtensionsSummary, String> {
    let mcp_count = match mcp::get_mcp_hub_snapshot_for_paths(paths) {
        Ok(snap) => snap.entries.len(),
        Err(err) => {
            eprintln!("统计通用 MCP 数量失败: {err}");
            0
        }
    };
    let (skills_count, skills_updatable_count) = skills::skills_summary_counts_for_paths(paths);
    let plugins_count = plugins::count_local_installed_plugins_for_paths(paths);
    let prompts_count = match prompts::get_prompts_hub_snapshot_for_paths(paths) {
        Ok(prompts) => prompts
            .platforms
            .values()
            .map(|platform| platform.presets.len())
            .sum(),
        Err(err) => {
            eprintln!("统计提示词预设数量失败: {err}");
            0
        }
    };
    Ok(ExtensionsSummary {
        mcp_count,
        skills_count,
        skills_updatable_count,
        plugins_count,
        prompts_count,
    })
}
