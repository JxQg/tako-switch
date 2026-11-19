use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_TAKO_BASE_URL: &str = "https://tako.shiroha.tech";
pub const DEFAULT_MANIFEST_URL: &str =
    "https://downloads.tako.shiroha.tech/tako-switch/manifest.json";
pub const DEFAULT_FALLBACK_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tako-dev/tako-switch/main/manifest/manifest.json";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub home: PathBuf,
    pub tako_dir: PathBuf,
    pub codex_dir: PathBuf,
    pub claude_1p_dir: PathBuf,
    pub claude_3p_dir: PathBuf,
    pub agy_gemini_config_dir: PathBuf,
    pub launch_agents_dir: PathBuf,
    pub is_test: bool,
}

impl AppPaths {
    pub fn detect() -> Result<Self, String> {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .ok_or_else(|| "无法解析用户 Home 目录".to_string())?;

        let tako_dir = std::env::var_os("TAKO_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".tako"));

        let codex_dir = std::env::var_os("CODEX_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));

        let (claude_1p_dir, claude_3p_dir) = detect_claude_dirs(&home);
        let agy_gemini_config_dir = home.join(".gemini").join("config");
        let launch_agents_dir = home.join("Library").join("LaunchAgents");

        Ok(Self {
            home,
            tako_dir,
            codex_dir,
            claude_1p_dir,
            claude_3p_dir,
            agy_gemini_config_dir,
            launch_agents_dir,
            is_test: false,
        })
    }

    pub fn for_test(root: &Path) -> Self {
        Self {
            home: root.to_path_buf(),
            tako_dir: root.join(".tako"),
            codex_dir: root.join(".codex"),
            claude_1p_dir: root.join("claude_1p"),
            claude_3p_dir: root.join("claude_3p"),
            agy_gemini_config_dir: root.join(".gemini").join("config"),
            launch_agents_dir: root.join("LaunchAgents"),
            is_test: true,
        }
    }

    pub fn tako_config_path(&self) -> PathBuf {
        self.tako_dir.join("config.json")
    }

    pub fn switch_dir(&self) -> PathBuf {
        self.tako_dir.join("switch")
    }

    pub fn state_path(&self) -> PathBuf {
        self.switch_dir().join("state.json")
    }

    pub fn backups_dir(&self, app_id: &str) -> PathBuf {
        self.switch_dir().join("backups").join(app_id)
    }

    pub fn default_media_output_dir(&self) -> PathBuf {
        self.home.join("Downloads").join("Tako")
    }

    pub fn mcp_server_exe(&self) -> PathBuf {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tako-switch"))
    }
}

fn detect_claude_dirs(home: &Path) -> (PathBuf, PathBuf) {
    #[cfg(target_os = "macos")]
    {
        let app_support = home.join("Library").join("Application Support");
        (app_support.join("Claude"), app_support.join("Claude-3p"))
    }
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        let roaming_app_data = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));

        // Windows 1p: prefer MSIX LocalCache\Roaming\Claude when present, else %APPDATA%\Claude.
        let msix_candidates = ["Claude_pzs8sxrjxfjjc", "AnthropicPBC.Claude_fnn82j28hfe8t"];
        let one_p = msix_candidates
            .iter()
            .map(|pfn| {
                local_app_data
                    .join("Packages")
                    .join(pfn)
                    .join("LocalCache")
                    .join("Roaming")
                    .join("Claude")
            })
            .find(|p| p.exists())
            .unwrap_or_else(|| roaming_app_data.join("Claude"));

        // Per Claude Desktop index.pre.js su(): 3p on Windows always resolves to %LOCALAPPDATA%\Claude-3p.
        (one_p, local_app_data.join("Claude-3p"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        (cfg.join("Claude"), cfg.join("Claude-3p"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolsConfig {
    pub image: bool,
    pub video: bool,
    pub audio: bool,
    pub search: bool,
    pub consult: bool,
    pub output_dir: String,
    pub consult_model: String,
}

impl McpToolsConfig {
    pub fn with_default_dir(paths: &AppPaths) -> Self {
        Self {
            image: true,
            video: true,
            audio: true,
            search: true,
            consult: true,
            output_dir: paths.default_media_output_dir().to_string_lossy().to_string(),
            consult_model: "gpt-5.4".to_string(),
        }
    }
}

impl Default for McpToolsConfig {
    fn default() -> Self {
        Self {
            image: true,
            video: true,
            audio: true,
            search: true,
            consult: true,
            output_dir: "~/Downloads/Tako".to_string(),
            consult_model: "gpt-5.4".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchState {
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agy_api_key: Option<String>,
    pub manifest_url: String,
    pub fallback_manifest_url: String,
    pub auto_check_updates: bool,
    #[serde(default)]
    pub mcp: McpToolsConfig,
}

impl SwitchState {
    pub fn default_for(paths: &AppPaths) -> Self {
        Self {
            base_url: DEFAULT_TAKO_BASE_URL.to_string(),
            agy_api_key: None,
            manifest_url: DEFAULT_MANIFEST_URL.to_string(),
            fallback_manifest_url: DEFAULT_FALLBACK_MANIFEST_URL.to_string(),
            auto_check_updates: true,
            mcp: McpToolsConfig::with_default_dir(paths),
        }
    }

    pub fn normalized_base_url(&self) -> String {
        let trimmed = self.base_url.trim().trim_end_matches('/');
        if let Some(stripped) = trimmed.strip_suffix("/v1") {
            stripped.to_string()
        } else if trimmed.is_empty() {
            DEFAULT_TAKO_BASE_URL.to_string()
        } else {
            trimmed.to_string()
        }
    }
}

pub fn write_atomic(path: &Path, content: &[u8], secret_mode: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("路径缺少父目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;

    let nonce: u32 = rand::thread_rng().gen();
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}.{}", std::process::id(), nonce));

    fs::write(&tmp_path, content)
        .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp_path.display()))?;

    #[cfg(unix)]
    if secret_mode {
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&tmp_path, perms)
            .map_err(|e| format!("设置权限 {} 失败: {e}", tmp_path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = secret_mode;

    // On Windows fs::rename fails if destination exists when using some filesystems; std handles replace cleanly on modern Rust, fallback remove+rename if needed.
    if let Err(err) = fs::rename(&tmp_path, path) {
        #[cfg(windows)]
        {
            if path.exists() {
                let _ = fs::remove_file(path);
                if fs::rename(&tmp_path, path).is_ok() {
                    return Ok(());
                }
            }
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("原子替换 {} 失败: {err}", path.display()));
    }
    Ok(())
}

pub fn read_tako_cli_config(paths: &AppPaths) -> Result<Map<String, Value>, String> {
    let path = paths.tako_config_path();
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
    parsed
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{} 顶层必须是 JSON object", path.display()))
}

pub fn save_tako_cli_credentials(
    paths: &AppPaths,
    api_key: Option<&str>,
    api_id: Option<&str>,
) -> Result<(), String> {
    let mut map = read_tako_cli_config(paths)?;
    match api_key.map(str::trim).filter(|s| !s.is_empty()) {
        Some(k) => {
            map.insert("apiKey".to_string(), Value::String(k.to_string()));
        }
        None => {
            map.remove("apiKey");
        }
    }
    match api_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => {
            map.insert("apiId".to_string(), Value::String(id.to_string()));
        }
        None => {
            map.remove("apiId");
        }
    }
    let serialized = serde_json::to_vec_pretty(&Value::Object(map))
        .map_err(|e| format!("序列化 Tako 配置失败: {e}"))?;
    write_atomic(&paths.tako_config_path(), &serialized, true)
}

pub fn read_main_api_key(paths: &AppPaths) -> Option<String> {
    read_tako_cli_config(paths).ok().and_then(|m| {
        m.get("apiKey")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

pub fn read_main_api_id(paths: &AppPaths) -> Option<String> {
    read_tako_cli_config(paths).ok().and_then(|m| {
        m.get("apiId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

pub fn read_switch_state(paths: &AppPaths) -> SwitchState {
    let path = paths.state_path();
    let default = SwitchState::default_for(paths);
    let Ok(raw) = fs::read_to_string(&path) else {
        return default;
    };
    let Ok(mut state) = serde_json::from_str::<SwitchState>(&raw) else {
        return default;
    };
    if state.base_url.trim().is_empty() {
        state.base_url = default.base_url;
    }
    if state.manifest_url.trim().is_empty() {
        state.manifest_url = default.manifest_url;
    }
    if state.fallback_manifest_url.trim().is_empty() {
        state.fallback_manifest_url = default.fallback_manifest_url;
    }
    if state.mcp.output_dir.trim().is_empty() {
        state.mcp.output_dir = default.mcp.output_dir;
    }
    if state.mcp.consult_model.trim().is_empty() {
        state.mcp.consult_model = default.mcp.consult_model;
    }
    state
}

pub fn save_switch_state(paths: &AppPaths, state: &SwitchState) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| format!("序列化 switch state 失败: {e}"))?;
    write_atomic(&paths.state_path(), &bytes, true)
}

pub fn effective_agy_api_key(paths: &AppPaths) -> Option<String> {
    let state = read_switch_state(paths);
    state
        .agy_api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| read_main_api_key(paths))
}

pub fn resolve_output_dir(paths: &AppPaths, raw_dir: &str) -> PathBuf {
    let trimmed = raw_dir.trim();
    if trimmed.is_empty() {
        return paths.default_media_output_dir();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return paths.home.join(rest);
    }
    if trimmed == "~" {
        return paths.home.clone();
    }
    PathBuf::from(trimmed)
}

pub fn backup_file_once(
    paths: &AppPaths,
    app_id: &str,
    backup_key: &str,
    src_path: &Path,
) -> Result<(), String> {
    let bdir = paths.backups_dir(app_id);
    let bfile = bdir.join(backup_key);
    let missing_marker = bdir.join(format!("{backup_key}.absent"));
    if bfile.exists() || missing_marker.exists() {
        return Ok(());
    }
    if src_path.exists() {
        let bytes = fs::read(src_path)
            .map_err(|e| format!("读取待备份文件 {} 失败: {e}", src_path.display()))?;
        write_atomic(&bfile, &bytes, true)
    } else {
        write_atomic(&missing_marker, b"absent", false)
    }
}

pub fn read_backup(paths: &AppPaths, app_id: &str, backup_key: &str) -> Option<Vec<u8>> {
    let bfile = paths.backups_dir(app_id).join(backup_key);
    fs::read(bfile).ok()
}

pub fn has_backup(paths: &AppPaths, app_id: &str) -> bool {
    let bdir = paths.backups_dir(app_id);
    fs::read_dir(bdir)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn preserves_tako_cli_fields_on_credential_update() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.tako_dir).unwrap();
        fs::write(
            paths.tako_config_path(),
            r#"{
              "installedClients": {"agy": {"version": "0.3.60"}},
              "recentProjects": [{"path": "/tmp/foo", "launchCount": 3}],
              "clientProviderMap": {"codex": "tako"}
            }"#,
        )
        .unwrap();

        save_tako_cli_credentials(&paths, Some("cr_test_key_123"), Some("7")).unwrap();
        let loaded = read_tako_cli_config(&paths).unwrap();
        assert_eq!(loaded.get("apiKey").and_then(Value::as_str), Some("cr_test_key_123"));
        assert_eq!(loaded.get("apiId").and_then(Value::as_str), Some("7"));
        assert!(loaded.get("installedClients").is_some());
        assert!(loaded.get("recentProjects").is_some());
        assert!(loaded.get("clientProviderMap").is_some());

        save_tako_cli_credentials(&paths, None, None).unwrap();
        let cleared = read_tako_cli_config(&paths).unwrap();
        assert!(cleared.get("apiKey").is_none());
        assert!(cleared.get("recentProjects").is_some());
    }

    #[test]
    fn agy_key_falls_back_to_main_key() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        save_tako_cli_credentials(&paths, Some("cr_main_key"), Some("1")).unwrap();
        assert_eq!(effective_agy_api_key(&paths).as_deref(), Some("cr_main_key"));

        let mut st = read_switch_state(&paths);
        st.agy_api_key = Some("cr_dedicated_agy".to_string());
        save_switch_state(&paths, &st).unwrap();
        assert_eq!(
            effective_agy_api_key(&paths).as_deref(),
            Some("cr_dedicated_agy")
        );
    }
}
