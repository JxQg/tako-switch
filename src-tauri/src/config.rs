use crate::adapters::AppId;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_TAKO_BASE_URL: &str = "https://tako.shiroha.tech";
pub const DEFAULT_GATEWAY_DISPLAY_NAME: &str = "Tako";
pub const DEFAULT_GATEWAY_DISPLAY_SUBTITLE: &str = "Tako AI Gateway";
pub const DEFAULT_MANIFEST_URL: &str =
    "https://dl.lven.asia/tako-switch/manifest.json";
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
    pub agy_app_override_dir: Option<PathBuf>,
    pub claude_app_override_dir: Option<PathBuf>,
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
            agy_app_override_dir: None,
            claude_app_override_dir: None,
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
            agy_app_override_dir: None,
            claude_app_override_dir: None,
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

    pub fn device_id_path(&self) -> PathBuf {
        self.switch_dir().join("device-id")
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolsConfig {
    #[serde(default = "default_true")]
    pub image: bool,
    #[serde(default = "default_true")]
    pub video: bool,
    #[serde(default = "default_true")]
    pub audio: bool,
    #[serde(default = "default_true")]
    pub search: bool,
    #[serde(default = "default_true")]
    pub consult: bool,
    #[serde(default)]
    pub web_page: bool,
    #[serde(default)]
    pub edgeone_api_token: Option<String>,
    #[serde(default)]
    pub edgeone_project_name: Option<String>,
    #[serde(default)]
    pub output_dir: String,
    #[serde(default)]
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
            web_page: false,
            edgeone_api_token: None,
            edgeone_project_name: None,
            output_dir: paths.default_media_output_dir().to_string_lossy().to_string(),
            consult_model: "gpt-6.2-sol".to_string(),
        }
    }

    pub fn normalize_optional_field(raw: Option<String>) -> Option<String> {
        raw.and_then(|s| {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
    }

    pub fn normalize(&mut self) {
        self.edgeone_api_token = Self::normalize_optional_field(self.edgeone_api_token.take());
        self.edgeone_project_name =
            Self::normalize_optional_field(self.edgeone_project_name.take());
    }

    pub fn normalized_edgeone_api_token(&self) -> Option<&str> {
        self.edgeone_api_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub fn normalized_edgeone_project_name(&self) -> Option<&str> {
        self.edgeone_project_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
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
            web_page: false,
            edgeone_api_token: None,
            edgeone_project_name: None,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UiDensity {
    #[default]
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
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
    #[serde(default = "default_true")]
    pub bypass_system_proxy: bool,
    #[serde(default)]
    pub ui_density: UiDensity,
    #[serde(default)]
    pub theme_pref: ThemePref,
    #[serde(default)]
    pub mcp: McpToolsConfig,
    #[serde(default)]
    pub model_filters: ModelFiltersConfig,
    /// Apps where the user explicitly chose Official / Standalone mode.
    /// When logged into Tako, apps absent from this list default to Tako intent
    /// and lazily switch to Tako right before launch/restart so active sessions are never disrupted.
    #[serde(default)]
    pub explicit_official_apps: Vec<AppId>,
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
            bypass_system_proxy: true,
            ui_density: UiDensity::Compact,
            theme_pref: ThemePref::System,
            mcp: McpToolsConfig::with_default_dir(paths),
            model_filters: ModelFiltersConfig::default(),
            explicit_official_apps: Vec::new(),
        }
    }

    pub fn is_explicit_official(&self, app_id: AppId) -> bool {
        self.explicit_official_apps.contains(&app_id)
    }

    pub fn set_explicit_official(&mut self, app_id: AppId, official: bool) {
        if official {
            if !self.explicit_official_apps.contains(&app_id) {
                self.explicit_official_apps.push(app_id);
            }
        } else {
            self.explicit_official_apps.retain(|id| *id != app_id);
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

pub fn apply_proxy_policy(
    builder: reqwest::ClientBuilder,
    bypass_system_proxy: bool,
) -> reqwest::ClientBuilder {
    if bypass_system_proxy {
        builder.no_proxy()
    } else {
        builder
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

pub fn copy_file_atomic(
    src_path: &Path,
    dst_path: &Path,
    secret_mode: bool,
) -> Result<(), String> {
    let parent = dst_path
        .parent()
        .ok_or_else(|| format!("路径缺少父目录: {}", dst_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;

    let nonce: u32 = rand::thread_rng().gen();
    let file_name = dst_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}.{}", std::process::id(), nonce));

    let copy_res = (|| -> Result<(), String> {
        let src_file = fs::File::open(src_path)
            .map_err(|e| format!("读取待备份文件 {} 失败: {e}", src_path.display()))?;
        let dst_file = fs::File::create(&tmp_path)
            .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp_path.display()))?;
        let mut reader = BufReader::with_capacity(64 * 1024, src_file);
        let mut writer = BufWriter::with_capacity(64 * 1024, dst_file);
        io::copy(&mut reader, &mut writer)
            .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp_path.display()))?;
        writer
            .flush()
            .map_err(|e| format!("刷新临时文件 {} 失败: {e}", tmp_path.display()))?;
        Ok(())
    })();
    if let Err(err) = copy_res {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }

    #[cfg(unix)]
    if secret_mode {
        let perms = fs::Permissions::from_mode(0o600);
        if let Err(e) = fs::set_permissions(&tmp_path, perms) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!("设置权限 {} 失败: {e}", tmp_path.display()));
        }
    }
    #[cfg(not(unix))]
    let _ = secret_mode;

    if let Err(err) = fs::rename(&tmp_path, dst_path) {
        #[cfg(windows)]
        {
            if dst_path.exists() {
                let _ = fs::remove_file(dst_path);
                if fs::rename(&tmp_path, dst_path).is_ok() {
                    return Ok(());
                }
            }
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(format!("原子替换 {} 失败: {err}", dst_path.display()));
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
    state.mcp.normalize();
    state
}

pub fn save_switch_state(paths: &AppPaths, state: &SwitchState) -> Result<(), String> {
    let mut normalized = state.clone();
    normalized.mcp.normalize();
    let bytes = serde_json::to_vec_pretty(&normalized)
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
        copy_file_atomic(src_path, &bfile, true)
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

fn is_valid_device_id(id: &str) -> bool {
    (8..=16).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

pub fn read_or_create_device_id(paths: &AppPaths) -> String {
    let path = paths.device_id_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        let trimmed = raw.trim();
        if is_valid_device_id(trimmed) {
            return trimmed.to_string();
        }
    }
    let id = format!("{:08x}", rand::thread_rng().gen::<u32>());
    let _ = write_atomic(&path, id.as_bytes(), false);
    id
}

pub fn sanitize_host_label(raw: &str) -> Option<String> {
    let first_label = raw.trim().split('.').next().unwrap_or("");
    let mut out = String::with_capacity(first_label.len());
    let mut prev_dash = false;
    for ch in first_label.chars() {
        let mapped = if ch.is_whitespace() {
            Some('-')
        } else if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-') {
            Some(ch)
        } else {
            None
        };
        if let Some(c) = mapped {
            if c == '-' {
                if !prev_dash {
                    out.push('-');
                    prev_dash = true;
                }
            } else {
                out.push(c);
                prev_dash = false;
            }
        }
    }
    let cleaned = out.trim_matches(['-', '.', '_', '+']);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

pub fn detect_raw_hostname() -> Option<String> {
    for key in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    let mut cmd = std::process::Command::new("hostname");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let host = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

pub fn build_desktop_device_name_with_host(paths: &AppPaths, raw_host: Option<&str>) -> String {
    let device_id = read_or_create_device_id(paths);
    let max_host_len = 32usize.saturating_sub(1 + device_id.len());
    let candidate = raw_host
        .and_then(sanitize_host_label)
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    let mut host = candidate
        .chars()
        .take(max_host_len)
        .collect::<String>()
        .trim_end_matches(['-', '.', '_', '+'])
        .to_string();
    if host.is_empty() {
        host = std::env::consts::OS
            .chars()
            .take(max_host_len)
            .collect::<String>()
            .trim_end_matches(['-', '.', '_', '+'])
            .to_string();
    }
    format!("{host}-{device_id}")
}

pub fn build_desktop_device_name(paths: &AppPaths) -> String {
    let raw_host = detect_raw_hostname();
    build_desktop_device_name_with_host(paths, raw_host.as_deref())
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
        assert!(loaded.bypass_system_proxy);
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

    #[test]
    fn device_id_is_persisted_reused_and_preserved_across_auth_clear() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let id1 = read_or_create_device_id(&paths);
        assert_eq!(id1.len(), 8);
        assert!(id1
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        assert!(paths.device_id_path().exists());

        let id2 = read_or_create_device_id(&paths);
        assert_eq!(id1, id2);

        save_switch_auth(&paths, Some("cr_switch_key"), Some("7"), Some("oauth")).unwrap();
        save_switch_auth(&paths, None, None, None).unwrap();
        assert!(paths.device_id_path().exists());
        assert_eq!(read_or_create_device_id(&paths), id1);

        // Pre-existing valid 8..=16 lowercase alphanumeric ID is trimmed and reused
        write_atomic(&paths.device_id_path(), b"  abc123def456  \n", false).unwrap();
        assert_eq!(read_or_create_device_id(&paths), "abc123def456");

        // Invalid existing ID (too short / uppercase / punctuation) is regenerated
        for invalid in [b"short".as_slice(), b"ABC12345", b"bad-id12", b"12345678901234567"] {
            write_atomic(&paths.device_id_path(), invalid, false).unwrap();
            let regen = read_or_create_device_id(&paths);
            assert_eq!(regen.len(), 8);
            assert!(regen
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
    }

    #[test]
    fn sanitize_host_label_and_device_name_enforce_contract() {
        assert_eq!(
            sanitize_host_label("  MacBook-Pro.local "),
            Some("MacBook-Pro".to_string())
        );
        assert_eq!(
            sanitize_host_label("Alice's  MacBook\tPro.corp.internal"),
            Some("Alices-MacBook-Pro".to_string())
        );
        assert_eq!(
            sanitize_host_label("--__++team_box+1--."),
            Some("team_box+1".to_string())
        );
        assert_eq!(sanitize_host_label("  ---._+  "), None);
        assert_eq!(sanitize_host_label("中文主机名.local"), None);
        assert_eq!(sanitize_host_label(".local"), None);

        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        write_atomic(&paths.device_id_path(), b"a1b2c3d4", false).unwrap();

        let name = build_desktop_device_name_with_host(&paths, Some("MacBook-Pro.local"));
        assert_eq!(name, "MacBook-Pro-a1b2c3d4");
        assert!((1..=32).contains(&name.len()));

        // Long hostname truncates cleanly without leaving trailing separators before -device_id
        let boundary = build_desktop_device_name_with_host(
            &paths,
            Some("1234567890123456789012-rest.local"),
        );
        assert_eq!(boundary, "1234567890123456789012-a1b2c3d4");
        assert_eq!(boundary.len(), 31);

        let very_long = build_desktop_device_name_with_host(
            &paths,
            Some("super-long-workstations-hostname-for-engineering.corp"),
        );
        assert_eq!(very_long, "super-long-workstations-a1b2c3d4");
        assert_eq!(very_long.len(), 32);
        assert!(very_long.ends_with("-a1b2c3d4"));
        assert!(very_long
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'@' | b'+' | b'-')));

        // Fallback to OS when host is None or sanitizes to None
        let fallback_none = build_desktop_device_name_with_host(&paths, None);
        assert_eq!(fallback_none, format!("{}-a1b2c3d4", std::env::consts::OS));
        let fallback_cn = build_desktop_device_name_with_host(&paths, Some("我的电脑"));
        assert_eq!(fallback_cn, format!("{}-a1b2c3d4", std::env::consts::OS));

        // Longer 16-char device-id still respects total 32-char cap
        write_atomic(&paths.device_id_path(), b"0123456789abcdef", false).unwrap();
        let long_id_name = build_desktop_device_name_with_host(
            &paths,
            Some("abcdefghijklmnopqrstuvwxyz.local"),
        );
        assert_eq!(long_id_name, "abcdefghijklmno-0123456789abcdef");
        assert_eq!(long_id_name.len(), 32);
    }

    #[test]
    fn legacy_mcp_config_deserializes_with_edgeone_defaults_and_trims_blank_fields() {
        let legacy_mcp_json = r#"{
            "image": true,
            "video": false,
            "audio": true,
            "search": true,
            "consult": false,
            "outputDir": "~/Downloads/Tako",
            "consultModel": "gpt-6.2-sol"
        }"#;
        let parsed: McpToolsConfig = serde_json::from_str(legacy_mcp_json).unwrap();
        assert!(parsed.image);
        assert!(!parsed.video);
        assert!(!parsed.consult);
        assert!(!parsed.web_page);
        assert_eq!(parsed.edgeone_api_token, None);
        assert_eq!(parsed.edgeone_project_name, None);

        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let legacy_state = serde_json::json!({
            "baseUrl": "https://tako.shiroha.tech",
            "manifestUrl": DEFAULT_MANIFEST_URL,
            "fallbackManifestUrl": DEFAULT_FALLBACK_MANIFEST_URL,
            "autoCheckUpdates": true,
            "mcp": {
                "image": true,
                "video": true,
                "audio": true,
                "search": true,
                "consult": true,
                "outputDir": "~/Downloads/Tako",
                "consultModel": "gpt-6.2-sol"
            }
        });
        write_atomic(
            &paths.state_path(),
            serde_json::to_string(&legacy_state).unwrap().as_bytes(),
            false,
        )
        .unwrap();

        let loaded = read_switch_state(&paths);
        assert!(!loaded.mcp.web_page);
        assert_eq!(loaded.mcp.edgeone_api_token, None);
        assert_eq!(loaded.mcp.edgeone_project_name, None);

        let mut st = loaded;
        st.mcp.web_page = true;
        st.mcp.edgeone_api_token = Some("   \t ".to_string());
        st.mcp.edgeone_project_name = Some("  my-site  ".to_string());
        save_switch_state(&paths, &st).unwrap();

        let reloaded = read_switch_state(&paths);
        assert!(reloaded.mcp.web_page);
        assert_eq!(reloaded.mcp.edgeone_api_token, None);
        assert_eq!(
            reloaded.mcp.edgeone_project_name.as_deref(),
            Some("my-site")
        );

        let raw_on_disk = fs::read_to_string(paths.state_path()).unwrap();
        let val_on_disk: serde_json::Value = serde_json::from_str(&raw_on_disk).unwrap();
        assert!(val_on_disk["mcp"]["edgeoneApiToken"].is_null());
        assert_eq!(val_on_disk["mcp"]["edgeoneProjectName"], "my-site");
        assert_eq!(val_on_disk["uiDensity"], "compact");
        assert_eq!(val_on_disk["themePref"], "system");
    }

    #[test]
    fn ui_density_and_theme_pref_default_to_compact_system_and_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let default_state = read_switch_state(&paths);
        assert_eq!(default_state.ui_density, UiDensity::Compact);
        assert_eq!(default_state.theme_pref, ThemePref::System);

        let mut updated = default_state;
        updated.ui_density = UiDensity::Comfortable;
        updated.theme_pref = ThemePref::Dark;
        save_switch_state(&paths, &updated).unwrap();

        let reloaded = read_switch_state(&paths);
        assert_eq!(reloaded.ui_density, UiDensity::Comfortable);
        assert_eq!(reloaded.theme_pref, ThemePref::Dark);

        let val: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(paths.state_path()).unwrap()).unwrap();
        assert_eq!(val["uiDensity"], "comfortable");
        assert_eq!(val["themePref"], "dark");
    }
}
