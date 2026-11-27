use crate::adapters::AppId;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const DEFAULT_TAKO_BASE_URL: &str = "https://tako.shiroha.tech";
pub const DEFAULT_GATEWAY_DISPLAY_NAME: &str = "Tako";
pub const DEFAULT_GATEWAY_DISPLAY_SUBTITLE: &str = "Tako AI Gateway";
pub const DEFAULT_MANIFEST_URL: &str =
    "https://dl.lven.asia/tako-switch/manifest.json";
pub const DEFAULT_FALLBACK_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tako-dev/tako-switch/main/manifest/manifest.json";
pub const DEFAULT_TELEMETRY_URL: &str = "https://telemetry.lven.asia";

pub const DEFAULT_CLAUDE_RECOMMENDED_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-fable-5-2",
    "gpt-6.2-sol",
    "gpt-6.2-astra",
    "swe-2-1",
    "deepseek-v4.1-flash",
    "glm-5.3",
    "kimi-k3.1-pro",
    "grok-4.7",
];
pub const DEFAULT_CODEX_RECOMMENDED_MODELS: &[&str] = &[
    "gpt-6.2-sol",
    "gpt-6.2-astra",
    "swe-2-1",
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
    "swe-2-1",
    "gemini-3.1-flash-image",
];
pub const DEFAULT_WORKBUDDY_RECOMMENDED_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "gpt-6.2-sol",
    "gpt-6.2-astra",
    "deepseek-v4.1-flash",
    "swe-2-1",
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
    pub agy_user_data_dir: PathBuf,
    pub workbuddy_dir: PathBuf,
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
        let agy_user_data_dir = detect_agy_user_data_dir(&home);
        let workbuddy_dir = std::env::var_os("WORKBUDDY_CONFIG_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".workbuddy"));
        let launch_agents_dir = home.join("Library").join("LaunchAgents");

        Ok(Self {
            home,
            tako_dir,
            dsh_tako_dir,
            codex_dir,
            claude_1p_dir,
            claude_3p_dir,
            agy_gemini_config_dir,
            agy_user_data_dir,
            workbuddy_dir,
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
            agy_user_data_dir: root.join("antigravity_user_data"),
            workbuddy_dir: root.join(".workbuddy"),
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

fn detect_agy_user_data_dir(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join("Antigravity")
    }
    #[cfg(target_os = "windows")]
    {
        let roaming_app_data = std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
        roaming_app_data.join("Antigravity")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let cfg = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        cfg.join("Antigravity")
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
            AppId::WorkBuddy => Self::default_workbuddy(),
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

    pub fn default_workbuddy() -> Self {
        Self {
            mode: ModelFilterMode::Recommended,
            enabled_ids: DEFAULT_WORKBUDDY_RECOMMENDED_MODELS
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
            AppId::WorkBuddy => DEFAULT_WORKBUDDY_RECOMMENDED_MODELS,
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
            let effective =
                if self.mode == ModelFilterMode::Recommended && trimmed.eq_ignore_ascii_case("swe-2")
                {
                    "swe-2-1"
                } else {
                    trimmed
                };
            let lower = effective.to_ascii_lowercase();
            if seen.insert(lower) {
                enabled_ids.push(effective.to_string());
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
    #[serde(default = "ClientModelFilter::default_workbuddy")]
    pub workbuddy: ClientModelFilter,
}

impl Default for ModelFiltersConfig {
    fn default() -> Self {
        Self {
            dsh_tako: ClientModelFilter::default_dsh_tako(),
            claude: ClientModelFilter::default_claude(),
            codex: ClientModelFilter::default_codex(),
            antigravity: ClientModelFilter::default_antigravity(),
            workbuddy: ClientModelFilter::default_workbuddy(),
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
            AppId::WorkBuddy => &self.workbuddy,
        }
    }

    pub fn for_app_mut(&mut self, app_id: AppId) -> &mut ClientModelFilter {
        match app_id {
            AppId::DshTako => &mut self.dsh_tako,
            AppId::Claude => &mut self.claude,
            AppId::Codex => &mut self.codex,
            AppId::Antigravity => &mut self.antigravity,
            AppId::WorkBuddy => &mut self.workbuddy,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeWorkspaceMode {
    /// Default: `desktopHome: "off"` (Code / Agent only, avoiding the file-restricted Chat tab).
    #[default]
    AgentOnly,
    /// Full desktop home (`desktopHome: "standard"`): exposes Chat, Cowork, and Code tabs.
    Full,
}

impl ClaudeWorkspaceMode {
    pub fn desktop_home_value(self) -> &'static str {
        match self {
            Self::AgentOnly => "off",
            Self::Full => "standard",
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
    #[serde(default = "default_true")]
    pub bypass_system_proxy: bool,
    #[serde(default = "default_true")]
    pub telemetry_enabled: bool,
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
    /// Apps hidden from the home overview and tray menu by the user.
    /// Empty by default so all tools are shown unless explicitly hidden in settings.
    #[serde(default)]
    pub hidden_apps: Vec<AppId>,
    /// Optional preferred default model ID for WorkBuddy (raw ID without `custom-local:` prefix).
    /// When None, automatically selects the first enabled Tako model in WorkBuddy's new-task selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workbuddy_default_model: Option<String>,
    /// Claude Desktop workspace surface mode in Tako 3p profile.
    /// Defaults to `AgentOnly` (`desktopHome: "off"`) so users land straight in Agent/Code mode.
    #[serde(default)]
    pub claude_workspace_mode: ClaudeWorkspaceMode,
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
            telemetry_enabled: true,
            ui_density: UiDensity::Compact,
            theme_pref: ThemePref::System,
            mcp: McpToolsConfig::with_default_dir(paths),
            model_filters: ModelFiltersConfig::default(),
            explicit_official_apps: Vec::new(),
            hidden_apps: Vec::new(),
            workbuddy_default_model: None,
            claude_workspace_mode: ClaudeWorkspaceMode::AgentOnly,
        }
    }

    pub fn is_app_hidden(&self, app_id: AppId) -> bool {
        self.hidden_apps.contains(&app_id)
    }

    pub fn normalize_hidden_apps(&mut self) {
        let mut deduped = Vec::with_capacity(self.hidden_apps.len());
        for &id in &self.hidden_apps {
            if !deduped.contains(&id) {
                deduped.push(id);
            }
        }
        // Never allow every desktop client to disappear simultaneously.
        if AppId::all().iter().all(|id| deduped.contains(id)) {
            deduped.clear();
        }
        self.hidden_apps = deduped;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedApiKey {
    pub id: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct UpsertSavedKeyInput<'a> {
    pub api_key: &'a str,
    pub api_id: Option<&'a str>,
    pub token_id: Option<&'a str>,
    pub user_name: Option<&'a str>,
    pub key_name: Option<&'a str>,
    pub alias: Option<Option<&'a str>>,
    pub auth_source: Option<&'a str>,
    pub activate: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_keys: Vec<SavedApiKey>,
}

fn unix_now_secs_config() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn clean_opt_str(raw: Option<&str>, max_chars: usize) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max_chars).collect())
}

/// Deterministic, non-reversible identifier derived from raw API key content.
/// Ensures keys are deduplicated by key identity rather than account identity.
pub fn key_profile_id(raw_key: &str) -> String {
    let trimmed = raw_key.trim();
    let digest = Sha256::digest(format!("tako-key:{trimmed}").as_bytes());
    hex::encode(&digest[..6])
}

fn normalize_switch_auth(mut auth: SwitchAuth) -> SwitchAuth {
    auth.api_key = clean_opt_str(auth.api_key.as_deref(), 512);
    auth.api_id = clean_opt_str(auth.api_id.as_deref(), 128);
    auth.auth_source = clean_opt_str(auth.auth_source.as_deref(), 64);
    auth.active_key_id = clean_opt_str(auth.active_key_id.as_deref(), 64);

    let mut seen_ids = HashSet::new();
    let mut normalized_keys = Vec::with_capacity(auth.saved_keys.len());
    for mut item in auth.saved_keys.drain(..) {
        let Some(trimmed_key) = clean_opt_str(Some(item.api_key.as_str()), 512) else {
            continue;
        };
        let derived_id = key_profile_id(&trimmed_key);
        if !seen_ids.insert(derived_id.clone()) {
            continue;
        }
        item.id = derived_id;
        item.api_key = trimmed_key;
        item.api_id = clean_opt_str(item.api_id.as_deref(), 128);
        item.token_id = clean_opt_str(item.token_id.as_deref(), 128);
        item.user_name = clean_opt_str(item.user_name.as_deref(), 120);
        item.key_name = clean_opt_str(item.key_name.as_deref(), 120);
        item.alias = clean_opt_str(item.alias.as_deref(), 40);
        item.auth_source = clean_opt_str(item.auth_source.as_deref(), 64);
        normalized_keys.push(item);
    }

    if let Some(ref active_key) = auth.api_key {
        let derived_id = key_profile_id(active_key);
        if let Some(existing) = normalized_keys.iter_mut().find(|k| k.id == derived_id) {
            if existing.api_id.is_none() && auth.api_id.is_some() {
                existing.api_id = auth.api_id.clone();
            }
            if existing.auth_source.is_none() && auth.auth_source.is_some() {
                existing.auth_source = auth.auth_source.clone();
            }
        } else {
            normalized_keys.insert(
                0,
                SavedApiKey {
                    id: derived_id.clone(),
                    api_key: active_key.clone(),
                    api_id: auth.api_id.clone(),
                    token_id: None,
                    user_name: None,
                    key_name: None,
                    alias: None,
                    auth_source: auth.auth_source.clone(),
                    added_at_unix: None,
                    last_used_at_unix: None,
                },
            );
        }
        auth.active_key_id = Some(derived_id);
    } else {
        auth.active_key_id = None;
    }

    auth.saved_keys = normalized_keys;
    auth
}

pub fn read_switch_auth(paths: &AppPaths) -> SwitchAuth {
    let path = paths.auth_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        return SwitchAuth::default();
    };
    let parsed = serde_json::from_str::<SwitchAuth>(&raw).unwrap_or_default();
    normalize_switch_auth(parsed)
}

pub fn write_switch_auth(paths: &AppPaths, auth: &SwitchAuth) -> Result<(), String> {
    let normalized = normalize_switch_auth(auth.clone());
    if normalized.api_key.is_none() && normalized.saved_keys.is_empty() {
        let path = paths.auth_path();
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("清理凭据文件 {} 失败: {e}", path.display()))?;
        }
        return Ok(());
    }
    let serialized = serde_json::to_vec_pretty(&normalized)
        .map_err(|e| format!("序列化 Tako Switch 凭据失败: {e}"))?;
    write_atomic(&paths.auth_path(), &serialized, true)
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

    upsert_saved_key(
        paths,
        UpsertSavedKeyInput {
            api_key: key,
            api_id,
            token_id: None,
            user_name: None,
            key_name: None,
            alias: None,
            auth_source,
            activate: true,
        },
    )?;
    Ok(())
}

pub fn upsert_saved_key(
    paths: &AppPaths,
    input: UpsertSavedKeyInput<'_>,
) -> Result<SavedApiKey, String> {
    let key = input.api_key.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let now = unix_now_secs_config();
    let id = key_profile_id(key);
    let mut auth = read_switch_auth(paths);

    let api_id = clean_opt_str(input.api_id, 128);
    let token_id = clean_opt_str(input.token_id, 128);
    let user_name = clean_opt_str(input.user_name, 120);
    let key_name = clean_opt_str(input.key_name, 120);
    let auth_source = clean_opt_str(input.auth_source, 64);

    // OAuth re-login on the same device revokes/replaces the previous OAuth token for the same user.
    // Remove superseded OAuth entries for the same user so dead OAuth keys don't pile up, while never touching manual keys.
    if auth_source.as_deref() == Some("oauth") {
        if let Some(ref uid) = api_id {
            auth.saved_keys.retain(|k| {
                k.id == id
                    || k.auth_source.as_deref() != Some("oauth")
                    || k.api_id.as_deref() != Some(uid.as_str())
            });
        }
    }

    let should_activate = input.activate || auth.api_key.is_none();

    let updated = if let Some(existing) = auth.saved_keys.iter_mut().find(|k| k.id == id) {
        if api_id.is_some() {
            existing.api_id = api_id;
        }
        if token_id.is_some() {
            existing.token_id = token_id;
        }
        if user_name.is_some() {
            existing.user_name = user_name;
        }
        if key_name.is_some() {
            existing.key_name = key_name;
        }
        if let Some(alias_opt) = input.alias {
            existing.alias = clean_opt_str(alias_opt, 40);
        }
        if auth_source.is_some() {
            existing.auth_source = auth_source;
        }
        if existing.added_at_unix.is_none() {
            existing.added_at_unix = Some(now);
        }
        if should_activate {
            existing.last_used_at_unix = Some(now);
        }
        existing.clone()
    } else {
        let entry = SavedApiKey {
            id: id.clone(),
            api_key: key.to_string(),
            api_id,
            token_id,
            user_name,
            key_name,
            alias: input.alias.and_then(|v| clean_opt_str(v, 40)),
            auth_source,
            added_at_unix: Some(now),
            last_used_at_unix: should_activate.then_some(now),
        };
        auth.saved_keys.push(entry.clone());
        entry
    };

    if should_activate {
        auth.api_key = Some(updated.api_key.clone());
        auth.api_id = updated.api_id.clone();
        auth.auth_source = updated.auth_source.clone();
        auth.active_key_id = Some(updated.id.clone());
    }

    write_switch_auth(paths, &auth)?;
    Ok(updated)
}

pub fn activate_saved_key(paths: &AppPaths, key_id: &str) -> Result<SavedApiKey, String> {
    let target_id = key_id.trim();
    if target_id.is_empty() {
        return Err("密钥 ID 不能为空".to_string());
    }
    let mut auth = read_switch_auth(paths);
    let now = unix_now_secs_config();
    let Some(entry) = auth.saved_keys.iter_mut().find(|k| k.id == target_id) else {
        return Err("未找到对应的本地保存密钥".to_string());
    };
    entry.last_used_at_unix = Some(now);
    let activated = entry.clone();
    auth.api_key = Some(activated.api_key.clone());
    auth.api_id = activated.api_id.clone();
    auth.auth_source = activated.auth_source.clone();
    auth.active_key_id = Some(activated.id.clone());
    write_switch_auth(paths, &auth)?;
    Ok(activated)
}

pub fn remove_saved_key_entry(
    paths: &AppPaths,
    key_id: &str,
) -> Result<(SavedApiKey, Option<SavedApiKey>), String> {
    let target_id = key_id.trim();
    let mut auth = read_switch_auth(paths);
    let Some(pos) = auth.saved_keys.iter().position(|k| k.id == target_id) else {
        return Err("未找到对应的本地保存密钥".to_string());
    };
    let removed = auth.saved_keys.remove(pos);
    let was_active = auth.active_key_id.as_deref() == Some(removed.id.as_str())
        || auth.api_key.as_deref() == Some(removed.api_key.as_str());

    let next_active = if was_active {
        if let Some(first) = auth.saved_keys.first_mut() {
            first.last_used_at_unix = Some(unix_now_secs_config());
            let next = first.clone();
            auth.api_key = Some(next.api_key.clone());
            auth.api_id = next.api_id.clone();
            auth.auth_source = next.auth_source.clone();
            auth.active_key_id = Some(next.id.clone());
            Some(next)
        } else {
            auth.api_key = None;
            auth.api_id = None;
            auth.auth_source = None;
            auth.active_key_id = None;
            None
        }
    } else {
        None
    };

    write_switch_auth(paths, &auth)?;
    Ok((removed, next_active))
}

pub fn update_saved_key_alias(
    paths: &AppPaths,
    key_id: &str,
    alias: Option<&str>,
) -> Result<SavedApiKey, String> {
    let target_id = key_id.trim();
    let mut auth = read_switch_auth(paths);
    let Some(entry) = auth.saved_keys.iter_mut().find(|k| k.id == target_id) else {
        return Err("未找到对应的本地保存密钥".to_string());
    };
    entry.alias = clean_opt_str(alias, 40);
    let updated = entry.clone();
    write_switch_auth(paths, &auth)?;
    Ok(updated)
}

pub fn update_saved_key_metadata(
    paths: &AppPaths,
    raw_key: &str,
    api_id: Option<&str>,
    token_id: Option<&str>,
    user_name: Option<&str>,
    key_name: Option<&str>,
) -> Result<(), String> {
    let id = key_profile_id(raw_key);
    let mut auth = read_switch_auth(paths);
    let Some(entry) = auth.saved_keys.iter_mut().find(|k| k.id == id) else {
        return Ok(());
    };
    let mut changed = false;
    if let Some(v) = clean_opt_str(api_id, 128) {
        if entry.api_id.as_deref() != Some(v.as_str()) {
            entry.api_id = Some(v.clone());
            changed = true;
        }
        if auth.active_key_id.as_deref() == Some(id.as_str())
            && auth.api_id.as_deref() != Some(v.as_str())
        {
            auth.api_id = Some(v);
            changed = true;
        }
    }
    if let Some(v) = clean_opt_str(token_id, 128) {
        if entry.token_id.as_deref() != Some(v.as_str()) {
            entry.token_id = Some(v);
            changed = true;
        }
    }
    if let Some(v) = clean_opt_str(user_name, 120) {
        if entry.user_name.as_deref() != Some(v.as_str()) {
            entry.user_name = Some(v);
            changed = true;
        }
    }
    if let Some(v) = clean_opt_str(key_name, 120) {
        if entry.key_name.as_deref() != Some(v.as_str()) {
            entry.key_name = Some(v);
            changed = true;
        }
    }
    if changed {
        write_switch_auth(paths, &auth)?;
    }
    Ok(())
}

/// Logs out the currently active key:
/// - If the active key was an OAuth device token (just revoked on the server), removes it from `saved_keys`.
/// - If it was a manually added key, keeps it in `saved_keys` so the user can switch back with one click.
/// - Clears `api_key` / `active_key_id` so the app enters the logged-out state; deletes `auth.json` if no saved keys remain.
pub fn deactivate_current_auth(paths: &AppPaths, remove_if_oauth: bool) -> Result<(), String> {
    let mut auth = read_switch_auth(paths);
    if remove_if_oauth && auth.auth_source.as_deref() == Some("oauth") {
        if let Some(active_id) = auth.active_key_id.clone() {
            auth.saved_keys.retain(|k| k.id != active_id);
        }
    }
    auth.api_key = None;
    auth.api_id = None;
    auth.auth_source = None;
    auth.active_key_id = None;
    write_switch_auth(paths, &auth)
}

pub fn read_active_saved_key(paths: &AppPaths) -> Option<SavedApiKey> {
    let auth = read_switch_auth(paths);
    let active_id = auth.active_key_id.as_deref()?;
    auth.saved_keys.into_iter().find(|k| k.id == active_id)
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
    state.normalize_hidden_apps();
    state
}

pub fn save_switch_state(paths: &AppPaths, state: &SwitchState) -> Result<(), String> {
    let mut normalized = state.clone();
    normalized.mcp.normalize();
    normalized.normalize_hidden_apps();
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

        // Recommended mode migrates legacy `swe-2` -> `swe-2-1` (deduped), while Custom mode preserves explicit `swe-2`.
        let rec_migrated = ClientModelFilter {
            mode: ModelFilterMode::Recommended,
            enabled_ids: vec![
                "gpt-6.2-sol".to_string(),
                "swe-2".to_string(),
                "swe-2-1".to_string(),
            ],
        }
        .normalized();
        assert_eq!(rec_migrated.enabled_ids, vec!["gpt-6.2-sol", "swe-2-1"]);

        let custom_kept = ClientModelFilter {
            mode: ModelFilterMode::Custom,
            enabled_ids: vec!["swe-2".to_string()],
        }
        .normalized();
        assert_eq!(custom_kept.enabled_ids, vec!["swe-2"]);
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

    #[test]
    fn hidden_apps_defaults_empty_dedupes_and_never_hides_all_tools() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let default_state = read_switch_state(&paths);
        assert!(default_state.hidden_apps.is_empty());

        let mut updated = default_state;
        updated.hidden_apps = vec![AppId::Codex, AppId::Codex, AppId::DshTako];
        save_switch_state(&paths, &updated).unwrap();

        let reloaded = read_switch_state(&paths);
        assert_eq!(reloaded.hidden_apps, vec![AppId::Codex, AppId::DshTako]);
        assert!(reloaded.is_app_hidden(AppId::Codex));
        assert!(!reloaded.is_app_hidden(AppId::Claude));

        // Manually corrupted state hiding every client resets to all visible on load/save.
        let mut all_hidden = reloaded;
        all_hidden.hidden_apps = AppId::all().to_vec();
        save_switch_state(&paths, &all_hidden).unwrap();
        let recovered = read_switch_state(&paths);
        assert!(recovered.hidden_apps.is_empty());
    }
}
