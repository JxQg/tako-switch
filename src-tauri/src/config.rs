use crate::adapters::AppId;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_TAKO_BASE_URL: &str = "https://tako.shiroha.tech";
pub const DEFAULT_GATEWAY_DISPLAY_NAME: &str = "Tako";
pub const DEFAULT_GATEWAY_DISPLAY_SUBTITLE: &str = "Tako AI Gateway";
pub const DEFAULT_MANIFEST_URL: &str =
    "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/tako-switch/manifest.json";
pub const DEFAULT_FALLBACK_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tako-dev/tako-switch/main/manifest/manifest.json";

pub const DEFAULT_CLAUDE_RECOMMENDED_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-fable-5-2",
];
pub const DEFAULT_CODEX_RECOMMENDED_MODELS: &[&str] = &[
    "gpt-6.2-sol",
    "gpt-6.2-astra",
    "swe-2",
    "deepseek-v4.1-flash",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
];
pub const DEFAULT_ANTIGRAVITY_RECOMMENDED_MODELS: &[&str] = &[
    "gemini-4-neon",
    "gemini-4-hydrogen",
    "gemini-3.8-flash",
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "gpt-6.2-sol",
    "gpt-6.2-astra",
    "deepseek-v4.1-flash",
    "swe-2",
    "gemini-3.1-flash-image",
];

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub home: PathBuf,
    pub tako_dir: PathBuf,
    pub dsh_tako_dir: PathBuf,
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

        let dsh_tako_dir = std::env::var_os("DSH_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".dsh-tako"));

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
            dsh_tako_dir,
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
            dsh_tako_dir: root.join(".dsh-tako"),
            codex_dir: root.join(".codex"),
            claude_1p_dir: root.join("claude_1p"),
            claude_3p_dir: root.join("claude_3p"),
            agy_gemini_config_dir: root.join(".gemini").join("config"),
            launch_agents_dir: root.join("LaunchAgents"),
            is_test: true,
        }
    }

    pub fn switch_dir(&self) -> PathBuf {
        self.tako_dir.join("switch")
    }

    pub fn auth_path(&self) -> PathBuf {
        self.switch_dir().join("auth.json")
    }

    pub fn state_path(&self) -> PathBuf {
        self.switch_dir().join("state.json")
    }

    pub fn balance_cache_path(&self) -> PathBuf {
        self.switch_dir().join("balance-cache.json")
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
            consult_model: "gpt-6.2-sol".to_string(),
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
            consult_model: "gpt-6.2-sol".to_string(),
        }
    }
}

fn default_gateway_display_name() -> String {
    DEFAULT_GATEWAY_DISPLAY_NAME.to_string()
}

fn default_gateway_display_subtitle() -> String {
    DEFAULT_GATEWAY_DISPLAY_SUBTITLE.to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFilterMode {
    Recommended,
    Custom,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientModelFilter {
    pub mode: ModelFilterMode,
    #[serde(default)]
    pub enabled_ids: Vec<String>,
}

impl ClientModelFilter {
    pub fn default_for_app(app_id: AppId) -> Self {
        match app_id {
            AppId::DshTako => Self::default_dsh_tako(),
            AppId::Claude => Self::default_claude(),
            AppId::Codex => Self::default_codex(),
            AppId::Antigravity => Self::default_antigravity(),
        }
    }

    pub fn default_dsh_tako() -> Self {
        Self {
            mode: ModelFilterMode::All,
            enabled_ids: Vec::new(),
        }
    }

    pub fn default_claude() -> Self {
        Self {
            mode: ModelFilterMode::Recommended,
            enabled_ids: DEFAULT_CLAUDE_RECOMMENDED_MODELS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    pub fn default_codex() -> Self {
        Self {
            mode: ModelFilterMode::All,
            enabled_ids: DEFAULT_CODEX_RECOMMENDED_MODELS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    pub fn default_antigravity() -> Self {
        Self {
            mode: ModelFilterMode::All,
            enabled_ids: DEFAULT_ANTIGRAVITY_RECOMMENDED_MODELS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    pub fn recommended_ids_for(app_id: AppId) -> &'static [&'static str] {
        match app_id {
            AppId::DshTako => &[],
            AppId::Claude => DEFAULT_CLAUDE_RECOMMENDED_MODELS,
            AppId::Codex => DEFAULT_CODEX_RECOMMENDED_MODELS,
            AppId::Antigravity => DEFAULT_ANTIGRAVITY_RECOMMENDED_MODELS,
        }
    }

    pub fn normalized(self) -> Self {
        let mut seen = HashSet::new();
        let mut enabled_ids = Vec::with_capacity(self.enabled_ids.len());
        for raw in self.enabled_ids {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_ascii_lowercase();
            if seen.insert(lower) {
                enabled_ids.push(trimmed.to_string());
            }
        }
        Self {
            mode: self.mode,
            enabled_ids,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelFiltersConfig {
    #[serde(default = "ClientModelFilter::default_dsh_tako")]
    pub dsh_tako: ClientModelFilter,
    #[serde(default = "ClientModelFilter::default_claude")]
    pub claude: ClientModelFilter,
    #[serde(default = "ClientModelFilter::default_codex")]
    pub codex: ClientModelFilter,
    #[serde(default = "ClientModelFilter::default_antigravity")]
    pub antigravity: ClientModelFilter,
}

impl Default for ModelFiltersConfig {
    fn default() -> Self {
        Self {
            dsh_tako: ClientModelFilter::default_dsh_tako(),
            claude: ClientModelFilter::default_claude(),
            codex: ClientModelFilter::default_codex(),
            antigravity: ClientModelFilter::default_antigravity(),
        }
    }
}

impl ModelFiltersConfig {
    pub fn for_app(&self, app_id: AppId) -> &ClientModelFilter {
        match app_id {
            AppId::DshTako => &self.dsh_tako,
            AppId::Claude => &self.claude,
            AppId::Codex => &self.codex,
            AppId::Antigravity => &self.antigravity,
        }
    }

    pub fn for_app_mut(&mut self, app_id: AppId) -> &mut ClientModelFilter {
        match app_id {
            AppId::DshTako => &mut self.dsh_tako,
            AppId::Claude => &mut self.claude,
            AppId::Codex => &mut self.codex,
            AppId::Antigravity => &mut self.antigravity,
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
    pub codex_preserve_official_auth: bool,
    #[serde(default = "default_gateway_display_name")]
    pub gateway_display_name: String,
    #[serde(default = "default_gateway_display_subtitle")]
    pub gateway_display_subtitle: String,
    #[serde(default = "default_true")]
    pub prefer_chinese_ui: bool,
    #[serde(default = "default_true")]
    pub show_tray_balance: bool,
    #[serde(default)]
    pub mcp: McpToolsConfig,
    #[serde(default)]
    pub model_filters: ModelFiltersConfig,
}

impl SwitchState {
    pub fn default_for(paths: &AppPaths) -> Self {
        Self {
            base_url: DEFAULT_TAKO_BASE_URL.to_string(),
            agy_api_key: None,
            manifest_url: DEFAULT_MANIFEST_URL.to_string(),
            fallback_manifest_url: DEFAULT_FALLBACK_MANIFEST_URL.to_string(),
            auto_check_updates: true,
            codex_preserve_official_auth: false,
            gateway_display_name: DEFAULT_GATEWAY_DISPLAY_NAME.to_string(),
            gateway_display_subtitle: DEFAULT_GATEWAY_DISPLAY_SUBTITLE.to_string(),
            prefer_chinese_ui: true,
            show_tray_balance: true,
            mcp: McpToolsConfig::with_default_dir(paths),
            model_filters: ModelFiltersConfig::default(),
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

    pub fn normalized_gateway_name(&self) -> String {
        let trimmed = self.gateway_display_name.trim();
        let effective = if trimmed.is_empty() {
            DEFAULT_GATEWAY_DISPLAY_NAME
        } else {
            trimmed
        };
        effective.chars().take(60).collect()
    }

    pub fn normalized_gateway_subtitle(&self) -> Option<String> {
        let trimmed = self.gateway_display_subtitle.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(60).collect())
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SwitchAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_source: Option<String>,
}

pub fn read_switch_auth(paths: &AppPaths) -> SwitchAuth {
    let path = paths.auth_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        return SwitchAuth::default();
    };
    serde_json::from_str::<SwitchAuth>(&raw).unwrap_or_default()
}

pub fn save_switch_auth(
    paths: &AppPaths,
    api_key: Option<&str>,
    api_id: Option<&str>,
    auth_source: Option<&str>,
) -> Result<(), String> {
    let Some(key) = api_key.map(str::trim).filter(|s| !s.is_empty()) else {
        let path = paths.auth_path();
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("清理凭据文件 {} 失败: {e}", path.display()))?;
        }
        return Ok(());
    };

    let payload = SwitchAuth {
        api_key: Some(key.to_string()),
        api_id: api_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        auth_source: auth_source
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };
    let serialized = serde_json::to_vec_pretty(&payload)
        .map_err(|e| format!("序列化 Tako Switch 凭据失败: {e}"))?;
    write_atomic(&paths.auth_path(), &serialized, true)
}

pub fn read_main_api_key(paths: &AppPaths) -> Option<String> {
    read_switch_auth(paths)
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn read_main_api_id(paths: &AppPaths) -> Option<String> {
    read_switch_auth(paths)
        .api_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
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
    if state.gateway_display_name.trim().is_empty() {
        state.gateway_display_name = default.gateway_display_name;
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
    fn switch_auth_is_isolated_from_tako_cli_config() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.tako_dir).unwrap();
        let cli_cfg_path = paths.tako_dir.join("config.json");
        let original_cli = r#"{"apiKey":"cr_cli_only_key","recentProjects":[]}"#;
        fs::write(&cli_cfg_path, original_cli).unwrap();

        assert!(read_main_api_key(&paths).is_none());

        save_switch_auth(&paths, Some("cr_switch_key"), Some("7"), Some("oauth")).unwrap();
        assert_eq!(read_main_api_key(&paths).as_deref(), Some("cr_switch_key"));
        assert_eq!(read_main_api_id(&paths).as_deref(), Some("7"));
        assert_eq!(
            read_switch_auth(&paths).auth_source.as_deref(),
            Some("oauth")
        );
        assert_eq!(fs::read_to_string(&cli_cfg_path).unwrap(), original_cli);

        save_switch_auth(&paths, None, None, None).unwrap();
        assert!(read_main_api_key(&paths).is_none());
        assert!(!paths.auth_path().exists());
        assert_eq!(fs::read_to_string(&cli_cfg_path).unwrap(), original_cli);
    }

    #[test]
    fn agy_key_falls_back_to_main_key() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        save_switch_auth(&paths, Some("cr_main_key"), Some("1"), Some("oauth")).unwrap();
        assert_eq!(effective_agy_api_key(&paths).as_deref(), Some("cr_main_key"));

        let mut st = read_switch_state(&paths);
        st.agy_api_key = Some("cr_dedicated_agy".to_string());
        save_switch_state(&paths, &st).unwrap();
        assert_eq!(
            effective_agy_api_key(&paths).as_deref(),
            Some("cr_dedicated_agy")
        );
    }

    #[test]
    fn legacy_switch_state_defaults_model_filters_and_normalizes() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let legacy_json = serde_json::json!({
            "baseUrl": "https://tako.shiroha.tech",
            "manifestUrl": DEFAULT_MANIFEST_URL,
            "fallbackManifestUrl": DEFAULT_FALLBACK_MANIFEST_URL,
            "autoCheckUpdates": true,
            "mcp": McpToolsConfig::default(),
        });
        write_atomic(
            &paths.state_path(),
            serde_json::to_string(&legacy_json).unwrap().as_bytes(),
            false,
        )
        .unwrap();

        let loaded = read_switch_state(&paths);
        assert_eq!(
            loaded.model_filters.claude.mode,
            ModelFilterMode::Recommended
        );
        assert_eq!(
            loaded.model_filters.claude.enabled_ids,
            DEFAULT_CLAUDE_RECOMMENDED_MODELS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(loaded.model_filters.codex.mode, ModelFilterMode::All);
        assert_eq!(loaded.model_filters.antigravity.mode, ModelFilterMode::All);

        let normalized = ClientModelFilter {
            mode: ModelFilterMode::Custom,
            enabled_ids: vec![
                " claude-opus-5-5 ".to_string(),
                "".to_string(),
                "   ".to_string(),
                "Claude-Opus-5-5".to_string(),
                "claude-sonnet-5-5".to_string(),
            ],
        }
        .normalized();
        assert_eq!(
            normalized.enabled_ids,
            vec!["claude-opus-5-5", "claude-sonnet-5-5"]
        );
    }
}
