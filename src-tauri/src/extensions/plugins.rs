use super::{
    build_http_client, codex_config_path, extension_store_path,
    is_safe_ascii_name as is_safe_name, read_json_store_checked, write_json_store, ExtPlatform,
};
use crate::config::{backup_file_once, write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use toml_edit::{DocumentMut, Item, Table};

const OFFICIAL_MARKETPLACE: &str = "antigravity-plugins-official";
const PARTNER_MARKETPLACE: &str = "antigravity-plugins-partners";
const CATALOG_URLS: [&str; 2] = [
    "https://www.gstatic.com/antigravity/plugins/marketplace.json",
    "https://www.gstatic.com/antigravity/plugins-partners/marketplace.json",
];
const CACHE_FILE: &str = "plugin_cache.json";
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
static MUTATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CATALOG_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginError {
    pub error_code: String,
    pub message: String,
    #[serde(default)]
    pub detail: Option<String>,
}

impl PluginError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            error_code: code.to_string(),
            message: message.into(),
            detail: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into().chars().take(4000).collect());
        self
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PluginError {}

impl From<String> for PluginError {
    fn from(message: String) -> Self {
        Self::new("PLUGIN_CONFIG_ERROR", message)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginCliSource {
    Builtin,
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCli {
    pub path: PathBuf,
    pub source: PluginCliSource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub version: Option<String>,
    pub description: String,
    pub category: Option<String>,
    pub marketplace: Option<String>,
    pub install_path: Option<String>,
    pub dir_name: Option<String>,
    pub scope: Option<String>,
    pub enabled: bool,
    pub manageable: bool,
    pub removable: bool,
    pub needs_configuration: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginMarketplace {
    pub name: String,
    pub source: String,
    pub install_location: Option<String>,
    pub manageable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsHubSnapshot {
    pub platform: ExtPlatform,
    pub cli_available: bool,
    pub cli_path: Option<String>,
    pub cli_source: Option<PluginCliSource>,
    pub marketplaces_supported: bool,
    pub installed: Vec<PluginEntry>,
    pub available: Vec<PluginEntry>,
    pub marketplaces: Vec<PluginMarketplace>,
    pub catalog_from_cache: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginActionOutcome {
    pub platform: ExtPlatform,
    pub plugin_id: String,
    pub needs_configuration: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginAction {
    Toggle(bool),
    Install,
    Update,
    Uninstall,
    AddMarketplace,
    RemoveMarketplace,
}

/// Overrides are accepted only with sandbox AppPaths, never through IPC.
#[derive(Debug, Clone, Default)]
pub struct PluginTestOverrides {
    pub builtin_cli_paths: Vec<PathBuf>,
    pub user_cli_paths: Vec<PathBuf>,
    pub path_dirs: Vec<PathBuf>,
    pub catalog_urls: Option<[String; 2]>,
    pub short_timeout: Option<Duration>,
    pub long_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedCatalog {
    #[serde(default)]
    source_url: String,
    #[serde(default)]
    entries: Vec<PluginEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginCatalogCache {
    #[serde(default)]
    catalogs: BTreeMap<String, CachedCatalog>,
    #[serde(default)]
    installed_counts: BTreeMap<ExtPlatform, usize>,
}

fn invalid_parameter() -> PluginError {
    PluginError::new(
        "PLUGIN_INVALID_ARGUMENT",
        "插件 ID 或市场名称无效，只允许安全名称或 name@marketplace，不能包含命令参数或特殊字符。",
    )
}

pub fn validate_plugin_id(id: &str) -> Result<(), PluginError> {
    let mut parts = id.split('@');
    if !parts.next().is_some_and(is_safe_name)
        || parts.next().is_some_and(|part| !is_safe_name(part))
        || parts.next().is_some()
    {
        return Err(invalid_parameter());
    }
    Ok(())
}

fn is_installed_directory(name: &str) -> bool {
    is_safe_name(name)
        || name.strip_prefix('-').is_some_and(|digits| {
            !digits.is_empty() && digits.len() <= 30 && digits.bytes().all(|b| b.is_ascii_digit())
        })
}

fn check_sandbox_path(paths: &AppPaths, path: &Path) -> Result<(), PluginError> {
    if !paths.is_test {
        return Ok(());
    }
    if !path.starts_with(&paths.home)
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(PluginError::new(
            "PLUGIN_TEST_BOUNDARY",
            "测试路径必须位于当前沙盒内。",
        ));
    }
    // Resolve existing ancestors too, so an absent file under a symlink cannot escape.
    let root = fs::canonicalize(&paths.home).map_err(|error| {
        PluginError::new("PLUGIN_TEST_BOUNDARY", format!("无法解析测试沙盒: {error}"))
    })?;
    let existing = path.ancestors().find(|ancestor| ancestor.exists());
    if let Some(existing) = existing {
        let resolved = fs::canonicalize(existing).map_err(|error| {
            PluginError::new("PLUGIN_TEST_BOUNDARY", format!("无法解析测试路径: {error}"))
        })?;
        if !resolved.starts_with(root) {
            return Err(PluginError::new(
                "PLUGIN_TEST_BOUNDARY",
                "测试路径不能指向沙盒外。",
            ));
        }
    }
    Ok(())
}

fn validate_overrides(
    paths: &AppPaths,
    overrides: Option<&PluginTestOverrides>,
) -> Result<(), PluginError> {
    let Some(overrides) = overrides else {
        return Ok(());
    };
    if !paths.is_test {
        return Err(PluginError::new(
            "PLUGIN_TEST_BOUNDARY",
            "生产环境不接受插件测试覆盖配置。",
        ));
    }
    for path in overrides
        .builtin_cli_paths
        .iter()
        .chain(&overrides.user_cli_paths)
        .chain(&overrides.path_dirs)
    {
        check_sandbox_path(paths, path)?;
    }
    if let Some(urls) = &overrides.catalog_urls {
        for raw in urls {
            let url = reqwest::Url::parse(raw).map_err(|_| invalid_parameter())?;
            if url.scheme() != "http"
                || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(PluginError::new(
                    "PLUGIN_TEST_BOUNDARY",
                    "测试目录只能使用本地 HTTP Mock 服务。",
                ));
            }
        }
    }
    Ok(())
}

fn cli_name(platform: ExtPlatform) -> &'static str {
    match platform {
        ExtPlatform::Claude => "claude",
        ExtPlatform::Codex => "codex",
        ExtPlatform::Antigravity => "agy",
    }
}

fn executable_names(platform: ExtPlatform) -> Vec<String> {
    let name = cli_name(platform);
    if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.into(),
        ]
    } else {
        vec![name.into()]
    }
}

fn bundled_candidates(root: &Path, platform: ExtPlatform) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for prefix in [
        "bin",
        "resources",
        "app/resources",
        "resources/app/bin",
        "Contents/Resources",
        "Contents/Resources/app/bin",
    ] {
        for name in executable_names(platform) {
            candidates.push(root.join(prefix).join(name));
        }
    }
    // A desktop's root Codex/Claude executable is its GUI, not the bundled CLI.
    if platform == ExtPlatform::Antigravity {
        candidates.extend(
            executable_names(platform)
                .into_iter()
                .map(|name| root.join(name)),
        );
    }
    candidates
}

fn probe_locations(paths: &AppPaths, platform: ExtPlatform) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut builtin = Vec::new();
    let mut user = Vec::new();
    let override_root = match platform {
        ExtPlatform::Claude => paths.claude_app_override_dir.as_ref(),
        ExtPlatform::Antigravity => paths.agy_app_override_dir.as_ref(),
        ExtPlatform::Codex => None,
    };
    if let Some(root) = override_root {
        if root.is_file() {
            builtin.push(root.clone());
        }
        builtin.extend(bundled_candidates(root, platform));
    }

    let app_name = match platform {
        ExtPlatform::Claude => "Claude",
        ExtPlatform::Codex => "Codex",
        ExtPlatform::Antigravity => "Antigravity",
    };
    #[cfg(windows)]
    {
        let local = if paths.is_test {
            None
        } else {
            std::env::var_os("LOCALAPPDATA")
        }
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData/Local"));
        let roaming = if paths.is_test {
            None
        } else {
            std::env::var_os("APPDATA")
        }
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData/Roaming"));
        builtin.extend(bundled_candidates(
            &local.join("Programs").join(app_name),
            platform,
        ));
        if platform == ExtPlatform::Claude {
            builtin.extend(bundled_candidates(&local.join("AnthropicClaude"), platform));
        }
        if !paths.is_test {
            let program_files = std::env::var_os("ProgramFiles")
                .map(PathBuf::from)
                .or_else(|| {
                    paths
                        .home
                        .ancestors()
                        .last()
                        .map(|root| root.join("Program Files"))
                });
            if let Some(program_files) = program_files {
                builtin.extend(bundled_candidates(&program_files.join(app_name), platform));
            }
        }
        user.push(
            roaming
                .join("npm")
                .join(format!("{}.cmd", cli_name(platform))),
        );
    }
    #[cfg(not(windows))]
    {
        if cfg!(target_os = "macos") && !paths.is_test {
            builtin.extend(bundled_candidates(
                &PathBuf::from("/Applications").join(format!("{app_name}.app")),
                platform,
            ));
        }
        builtin.extend(bundled_candidates(
            &paths
                .home
                .join("Applications")
                .join(format!("{app_name}.app")),
            platform,
        ));
    }
    if platform == ExtPlatform::Claude {
        for directory in [
            paths.home.join(".local/bin"),
            paths.home.join(".claude/local"),
        ] {
            user.extend(
                executable_names(platform)
                    .into_iter()
                    .map(|name| directory.join(name)),
            );
        }
    }
    if platform == ExtPlatform::Codex {
        user.extend(
            executable_names(platform)
                .into_iter()
                .map(|name| paths.home.join(".cargo/bin").join(name)),
        );
    }
    (builtin, user)
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

fn detect_cli(
    paths: &AppPaths,
    platform: ExtPlatform,
    overrides: Option<&PluginTestOverrides>,
) -> Result<Option<PluginCli>, PluginError> {
    validate_overrides(paths, overrides)?;
    let (mut builtin, mut user) = probe_locations(paths, platform);
    if let Some(overrides) = overrides {
        builtin.splice(0..0, overrides.builtin_cli_paths.iter().cloned());
        user.splice(0..0, overrides.user_cli_paths.iter().cloned());
    }
    let path_dirs = if paths.is_test {
        overrides
            .map(|value| value.path_dirs.clone())
            .unwrap_or_default()
    } else {
        std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default()
    };
    for directory in path_dirs {
        user.extend(
            executable_names(platform)
                .into_iter()
                .map(|name| directory.join(name)),
        );
    }
    for (candidates, source) in [
        (builtin, PluginCliSource::Builtin),
        (user, PluginCliSource::Path),
    ] {
        for path in candidates {
            check_sandbox_path(paths, &path)?;
            if is_executable_file(&path) {
                return Ok(Some(PluginCli { path, source }));
            }
        }
    }
    Ok(None)
}

pub fn detect_plugin_cli_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
) -> Result<Option<PluginCli>, PluginError> {
    detect_cli(paths, platform, None)
}

pub fn detect_plugin_cli_with_overrides(
    paths: &AppPaths,
    platform: ExtPlatform,
    overrides: &PluginTestOverrides,
) -> Result<Option<PluginCli>, PluginError> {
    detect_cli(paths, platform, Some(overrides))
}

fn classify_cli_error(platform: ExtPlatform, output: &str) -> Option<PluginError> {
    let lower = output.to_lowercase();
    if [
        "ebusy",
        "eperm",
        "being used by another process",
        "access is denied",
        "permission denied",
        "拒绝访问",
        "另一个程序正在使用",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Some(
            PluginError::new(
                "PLUGIN_IN_USE",
                format!(
                    "插件文件正在被占用或无法访问，请先关闭运行中的 {} 客户端会话后重试。",
                    platform.display_name()
                ),
            )
            .with_detail(output),
        );
    }
    let marketplace = lower.contains("marketplace") || lower.contains("市场");
    let confirmation = [
        "interactive confirmation",
        "requires interactive",
        "confirm the marketplace",
        "confirm marketplace",
        "trust this marketplace",
        "run in a terminal",
        "requires confirmation",
        "needs confirmation",
        "confirm the source",
        "do you trust",
        "not trusted",
        "需要确认",
        "交互式确认",
        "确认市场",
        "信任该市场",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if marketplace && confirmation {
        return Some(
            PluginError::new(
                "PLUGIN_NEEDS_TERMINAL_CONFIRM",
                format!(
                    "{} 需要确认市场来源，请先在终端使用对应 CLI 确认并信任该市场，然后重试。",
                    platform.display_name()
                ),
            )
            .with_detail(output),
        );
    }
    None
}

fn decode_cli_output(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn MultiByteToWideChar(
                code_page: u32,
                flags: u32,
                input: *const u8,
                input_len: i32,
                output: *mut u16,
                output_len: i32,
            ) -> i32;
        }
        if let Ok(length) = i32::try_from(bytes.len()) {
            // cmd diagnostics use the system OEM code page when stdout is redirected.
            let required = unsafe {
                MultiByteToWideChar(1, 0, bytes.as_ptr(), length, std::ptr::null_mut(), 0)
            };
            if required > 0 {
                let mut wide = vec![0; required as usize];
                let written = unsafe {
                    MultiByteToWideChar(1, 0, bytes.as_ptr(), length, wide.as_mut_ptr(), required)
                };
                if written > 0 {
                    return String::from_utf16_lossy(&wide[..written as usize]);
                }
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
fn windows_command(cli: &Path, args: &[&str]) -> Result<Command, PluginError> {
    let extension = cli
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if !extension.eq_ignore_ascii_case("cmd") && !extension.eq_ignore_ascii_case("bat") {
        return Ok(Command::new(cli));
    }
    let normalized: PathBuf = cli.components().collect();
    let executable = normalized.to_string_lossy();
    if executable.contains(['"', '%', '!', '\r', '\n'])
        || args
            .iter()
            .any(|arg| arg.contains(['"', '%', '!', '^', '&', '|', '<', '>', '\r', '\n']))
    {
        return Err(invalid_parameter());
    }
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| PluginError::new("PLUGIN_CLI_ERROR", "无法定位 Windows 命令解释器。"))?;
    let mut command = Command::new(PathBuf::from(system_root).join("System32").join("cmd.exe"));
    let quoted_args = args
        .iter()
        .map(|arg| format!("\"{arg}\""))
        .collect::<Vec<_>>()
        .join(" ");
    command.args(["/d", "/v:off", "/s", "/c"]);
    command.raw_arg(format!("\"\"{executable}\" {quoted_args}\""));
    Ok(command)
}

async fn run_cli(
    paths: &AppPaths,
    platform: ExtPlatform,
    cli: &PluginCli,
    args: &[&str],
    long_running: bool,
    overrides: Option<&PluginTestOverrides>,
) -> Result<String, PluginError> {
    check_sandbox_path(paths, &cli.path)?;
    #[cfg(windows)]
    let mut command = windows_command(&cli.path, args)?;
    #[cfg(not(windows))]
    let mut command = Command::new(&cli.path);
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
        let extension = cli
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if !extension.eq_ignore_ascii_case("cmd") && !extension.eq_ignore_ascii_case("bat") {
            command.args(args);
        }
    }
    #[cfg(not(windows))]
    command.args(args);

    if paths.is_test {
        command.env_clear();
        for name in ["SystemRoot", "WINDIR", "COMSPEC"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.env("PATH", cli.path.parent().unwrap_or(&paths.home));
        command.env("APPDATA", paths.home.join("AppData/Roaming"));
        command.env("LOCALAPPDATA", paths.home.join("AppData/Local"));
        command.env("XDG_CONFIG_HOME", paths.home.join(".config"));
    }
    command
        .env("HOME", &paths.home)
        .env("USERPROFILE", &paths.home)
        .env("CLAUDE_CONFIG_DIR", paths.home.join(".claude"))
        .env("CODEX_HOME", &paths.codex_dir)
        .env("GEMINI_CLI_HOME", paths.home.join(".gemini"))
        .env("GEMINI_CONFIG_DIR", &paths.agy_gemini_config_dir)
        .env("AGY_CONFIG_DIR", &paths.agy_gemini_config_dir)
        .current_dir(&paths.home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let duration = if long_running {
        overrides
            .and_then(|value| value.long_timeout)
            .unwrap_or(Duration::from_secs(180))
    } else {
        overrides
            .and_then(|value| value.short_timeout)
            .unwrap_or(Duration::from_secs(60))
    };
    let output = tokio::time::timeout(duration, command.output())
        .await
        .map_err(|_| {
            PluginError::new(
                "PLUGIN_CLI_TIMEOUT",
                "插件命令执行超时，已中止子进程，请稍后重试。",
            )
        })?
        .map_err(|error| {
            PluginError::new(
                "PLUGIN_CLI_ERROR",
                format!("无法执行 {} CLI: {error}", platform.display_name()),
            )
        })?;
    let stdout = decode_cli_output(&output.stdout);
    let stderr = decode_cli_output(&output.stderr);
    let combined = format!("{stderr}\n{stdout}");
    let parsed = extract_cli_json(&stdout);
    if !output.status.success()
        || parsed.as_ref().is_ok_and(|value| {
            value.get("error").is_some()
                || value.get("success").and_then(Value::as_bool) == Some(false)
        })
        || parsed.is_err()
    {
        if let Some(error) = classify_cli_error(platform, &combined) {
            return Err(error);
        }
    }
    let json_error = parsed.as_ref().is_ok_and(|value| {
        value.get("error").is_some_and(|error| !error.is_null())
            || value.get("success").and_then(Value::as_bool) == Some(false)
    });
    if !output.status.success() || json_error {
        return Err(PluginError::new(
            "PLUGIN_CLI_ERROR",
            format!("{} 插件命令执行失败。", platform.display_name()),
        )
        .with_detail(combined.trim()));
    }
    if let Some(error) = classify_cli_error(platform, &stderr) {
        return Err(error);
    }
    Ok(stdout)
}

pub fn extract_cli_json(output: &str) -> Result<Value, PluginError> {
    let trimmed = output.trim().trim_start_matches('\u{feff}');
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if value.is_object() || value.is_array() {
            return Ok(value);
        }
    }
    if let Some(line) = trimmed.lines().last() {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            if value.is_object() || value.is_array() {
                return Ok(value);
            }
        }
    }
    for (index, character) in trimmed.char_indices() {
        if !matches!(character, '{' | '[') {
            continue;
        }
        if let Some(Ok(value)) = serde_json::Deserializer::from_str(&trimmed[index..])
            .into_iter::<Value>()
            .next()
        {
            if value.is_object() || value.is_array() {
                return Ok(value);
            }
        }
    }
    Err(PluginError::new(
        "PLUGIN_INVALID_RESPONSE",
        "CLI 未返回可识别的插件 JSON 数据。",
    ))
}

pub fn parse_plugin_jsonc(raw: &str) -> Result<Value, PluginError> {
    let mut bytes = raw.trim_start_matches('\u{feff}').as_bytes().to_vec();
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                bytes[index] = b' ';
                index += 1;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            bytes[index..index + 2].fill(b' ');
            index += 2;
            while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                if !matches!(bytes[index], b'\n' | b'\r') {
                    bytes[index] = b' ';
                }
                index += 1;
            }
            if index + 1 >= bytes.len() {
                return Err(PluginError::new(
                    "PLUGIN_INVALID_RESPONSE",
                    "插件清单包含未闭合的块注释。",
                ));
            }
            bytes[index..index + 2].fill(b' ');
            index += 2;
        } else {
            index += 1;
        }
    }
    in_string = false;
    escaped = false;
    for index in 0..bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b',' {
            let next = bytes[index + 1..]
                .iter()
                .find(|byte| !byte.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[index] = b' ';
            }
        }
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        PluginError::new(
            "PLUGIN_INVALID_RESPONSE",
            format!("解析插件清单失败: {error}"),
        )
    })
}

fn text_field(value: &Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| value.get(name).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn plugin_rows<'a>(
    value: &'a Value,
    section: &str,
) -> Result<Vec<(Option<&'a str>, &'a Value)>, PluginError> {
    if let Some(array) = value.as_array() {
        return Ok(array.iter().map(|row| (None, row)).collect());
    }
    let Some(object) = value.as_object() else {
        return Err(PluginError::new(
            "PLUGIN_INVALID_RESPONSE",
            "插件列表必须是 JSON 数组或对象。",
        ));
    };
    for key in [section, "plugins"] {
        if let Some(nested) = object.get(key) {
            return plugin_rows(nested, section);
        }
    }
    if object.contains_key("name") || object.contains_key("id") {
        return Ok(vec![(None, value)]);
    }
    if object.contains_key("installed") || object.contains_key("available") {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for (key, row) in object {
        if let Some(array) = row.as_array() {
            rows.extend(array.iter().map(|entry| (Some(key.as_str()), entry)));
        } else if row.is_object() || row.is_boolean() {
            rows.push((Some(key.as_str()), row));
        }
    }
    Ok(rows)
}

fn enrich_manifest(
    paths: &AppPaths,
    platform: ExtPlatform,
    entry: &mut PluginEntry,
) -> Result<(), PluginError> {
    let root = if platform == ExtPlatform::Antigravity {
        let Some(directory) = entry
            .dir_name
            .as_deref()
            .filter(|directory| is_installed_directory(directory))
        else {
            return Ok(());
        };
        paths.agy_gemini_config_dir.join("plugins").join(directory)
    } else {
        let Some(root) = entry.install_path.as_deref() else {
            return Ok(());
        };
        let path = PathBuf::from(root);
        if path.is_absolute() {
            path
        } else {
            paths.home.join(path)
        }
    };
    check_sandbox_path(paths, &root)?;
    for folder in if platform == ExtPlatform::Antigravity {
        &[
            "",
            ".antigravity-plugin",
            ".gemini-plugin",
            ".claude-plugin",
        ][..]
    } else {
        &[".claude-plugin"][..]
    } {
        let path = root.join(folder).join("plugin.json");
        check_sandbox_path(paths, &path)?;
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(PluginError::new(
                    "PLUGIN_CONFIG_ERROR",
                    format!("读取插件清单失败: {error}"),
                ))
            }
        };
        let manifest = parse_plugin_jsonc(&raw)?;
        if platform == ExtPlatform::Antigravity {
            if let Some(name) = text_field(&manifest, &["name"]).filter(|name| is_safe_name(name)) {
                if entry.display_name == entry.name {
                    entry.display_name.clone_from(&name);
                }
                entry.name = name;
            }
            if entry.marketplace.is_none() {
                entry.marketplace = text_field(&manifest, &["marketplace"])
                    .filter(|marketplace| is_safe_name(marketplace));
            }
        }
        if let Some(display_name) = text_field(&manifest, &["displayName", "display_name"]) {
            entry.display_name = display_name;
        }
        if let Some(description) = text_field(&manifest, &["description"]) {
            entry.description = description;
        }
        if let Some(category) = text_field(&manifest, &["category"]) {
            entry.category = Some(category);
        }
        if let Some(version) = text_field(&manifest, &["version"]) {
            entry.version = Some(version);
        }
        break;
    }
    Ok(())
}

fn parse_entries(
    paths: &AppPaths,
    platform: ExtPlatform,
    value: &Value,
    installed: bool,
    default_marketplace: Option<&str>,
) -> Result<Vec<PluginEntry>, PluginError> {
    let mut entries = Vec::new();
    for (key, row) in plugin_rows(value, if installed { "installed" } else { "available" })? {
        let raw_id = text_field(row, &["id", "pluginId", "plugin_id"])
            .or_else(|| key.filter(|key| key.contains('@')).map(str::to_string))
            .or_else(|| text_field(row, &["name"]))
            .or_else(|| key.map(str::to_string));
        let Some(raw_id) = raw_id else { continue };
        let (id_name, id_marketplace) = raw_id.split_once('@').unwrap_or((&raw_id, ""));
        let marketplace = text_field(row, &["marketplace", "marketplaceName", "marketplace_name"])
            .or_else(|| {
                row.get("marketplace")
                    .and_then(|value| text_field(value, &["name"]))
            })
            .or_else(|| (!id_marketplace.is_empty()).then(|| id_marketplace.to_string()))
            .or_else(|| default_marketplace.map(str::to_string));
        let scope = row.get("scope").and_then(Value::as_str).map(str::to_string);
        let manageable =
            platform != ExtPlatform::Claude || scope.as_deref().is_none_or(|scope| scope == "user");
        let name = text_field(row, &["name"])
            .map(|name| name.split('@').next().unwrap_or(&name).to_string())
            .unwrap_or_else(|| id_name.to_string());
        let directory = if platform == ExtPlatform::Antigravity && installed {
            Some(
                text_field(row, &["dirName", "dir_name", "directory", "directoryName"])
                    .or_else(|| key.map(str::to_string))
                    .unwrap_or_else(|| id_name.to_string()),
            )
        } else {
            None
        };
        let enabled = installed
            && row
                .get("enabled")
                .and_then(Value::as_bool)
                .or_else(|| row.as_bool())
                .unwrap_or_else(|| row.get("status").and_then(Value::as_str) != Some("disabled"));
        let mut entry = PluginEntry {
            id: raw_id.clone(),
            name: name.clone(),
            display_name: text_field(row, &["displayName", "display_name", "title"])
                .unwrap_or(name),
            version: text_field(row, &["version"]),
            description: text_field(row, &["description"]).unwrap_or_default(),
            category: text_field(row, &["category"]),
            marketplace,
            install_path: text_field(row, &["installPath", "install_path", "path"]),
            dir_name: directory,
            scope,
            enabled,
            manageable,
            removable: manageable,
            needs_configuration: platform == ExtPlatform::Antigravity && installed && !enabled,
        };
        if platform == ExtPlatform::Antigravity && installed {
            if !entry
                .dir_name
                .as_deref()
                .is_some_and(is_installed_directory)
            {
                return Err(invalid_parameter());
            }
        } else {
            validate_plugin_id(&entry.id)?;
        }
        if installed || (platform != ExtPlatform::Antigravity && entry.install_path.is_some()) {
            enrich_manifest(paths, platform, &mut entry)?;
        }
        if let Some(marketplace) = &entry.marketplace {
            if !is_safe_name(marketplace) {
                return Err(invalid_parameter());
            }
            if is_safe_name(&entry.name) {
                entry.id = format!("{}@{marketplace}", entry.name);
            } else if platform == ExtPlatform::Antigravity && installed {
                entry.marketplace = None;
            } else {
                return Err(invalid_parameter());
            }
        } else if platform == ExtPlatform::Antigravity && is_safe_name(&entry.name) {
            entry.id = entry.name.clone();
        }
        entries.push(entry);
    }
    entries.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
    });
    entries.dedup_by(|left, right| left.id == right.id && left.scope == right.scope);
    Ok(entries)
}

fn exclude_installed(available: &mut Vec<PluginEntry>, installed: &[PluginEntry]) {
    available.retain(|candidate| {
        !installed.iter().any(|entry| {
            candidate.id == entry.id
                || (candidate.name == entry.name
                    && (candidate.marketplace == entry.marketplace || entry.marketplace.is_none()))
        })
    });
}

fn read_codex_doc(paths: &AppPaths) -> Result<DocumentMut, PluginError> {
    let path = codex_config_path(paths);
    check_sandbox_path(paths, &path)?;
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(PluginError::new(
                "PLUGIN_CONFIG_ERROR",
                format!("读取 Codex 配置失败: {error}"),
            ))
        }
    };
    raw.parse::<DocumentMut>().map_err(|error| {
        PluginError::new(
            "PLUGIN_CONFIG_ERROR",
            format!("解析 Codex 配置失败: {error}"),
        )
    })
}

fn apply_codex_enabled(paths: &AppPaths, entries: &mut [PluginEntry]) -> Result<(), PluginError> {
    let document = read_codex_doc(paths)?;
    for entry in entries {
        if let Some(enabled) = document
            .get("plugins")
            .and_then(Item::as_table_like)
            .and_then(|plugins| plugins.get(&entry.id))
            .and_then(Item::as_table_like)
            .and_then(|plugin| plugin.get("enabled"))
            .and_then(Item::as_bool)
        {
            entry.enabled = enabled;
        }
    }
    Ok(())
}

fn set_codex_enabled(paths: &AppPaths, id: &str, enabled: bool) -> Result<(), PluginError> {
    let path = codex_config_path(paths);
    let mut document = read_codex_doc(paths)?;
    if !document.contains_key("plugins") {
        document["plugins"] = Item::Table(Table::new());
    }
    let plugins = document
        .get_mut("plugins")
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            PluginError::new(
                "PLUGIN_CONFIG_ERROR",
                "Codex plugins 配置不是表，已保留原配置。",
            )
        })?;
    if !plugins.contains_key(id) {
        plugins.insert(id, Item::Table(Table::new()));
    }
    let plugin = plugins
        .get_mut(id)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            PluginError::new(
                "PLUGIN_CONFIG_ERROR",
                "Codex 插件配置不是表，已保留原配置。",
            )
        })?;
    if let Some(value) = plugin.get_mut("enabled") {
        let decor = value.as_value().map(|value| value.decor().clone());
        let mut replacement = toml_edit::Value::from(enabled);
        if let Some(decor) = decor {
            *replacement.decor_mut() = decor;
        }
        *value = Item::Value(replacement);
    } else {
        plugin.insert("enabled", toml_edit::value(enabled));
    }
    backup_file_once(paths, "codex", "config.toml", &path)?;
    write_atomic(&path, document.to_string().as_bytes(), true)?;
    Ok(())
}

async fn installed_entries(
    paths: &AppPaths,
    platform: ExtPlatform,
    cli: &PluginCli,
    overrides: Option<&PluginTestOverrides>,
) -> Result<Vec<PluginEntry>, PluginError> {
    let output = run_cli(
        paths,
        platform,
        cli,
        &["plugin", "list", "--json"],
        false,
        overrides,
    )
    .await?;
    let mut entries = parse_entries(paths, platform, &extract_cli_json(&output)?, true, None)?;
    if platform == ExtPlatform::Codex {
        apply_codex_enabled(paths, &mut entries)?;
    }
    Ok(entries)
}

fn parse_marketplaces(value: &Value) -> Result<Vec<PluginMarketplace>, PluginError> {
    let mut marketplaces = Vec::new();
    for (key, row) in plugin_rows(value, "marketplaces")? {
        let Some(name) = text_field(row, &["name"]).or_else(|| key.map(str::to_string)) else {
            continue;
        };
        if !is_safe_name(&name) {
            return Err(invalid_parameter());
        }
        let source = text_field(row, &["source", "url", "repo"])
            .or_else(|| {
                row.get("source")
                    .and_then(|source| text_field(source, &["repo", "url", "path"]))
            })
            .unwrap_or_default();
        let scope = row.get("scope").and_then(Value::as_str).map(str::to_string);
        marketplaces.push(PluginMarketplace {
            name,
            source,
            install_location: text_field(row, &["installLocation", "install_location"]),
            manageable: scope.as_deref().is_none_or(|scope| scope == "user"),
        });
    }
    Ok(marketplaces)
}

async fn fetch_catalog(
    paths: &AppPaths,
    client: &reqwest::Client,
    url: &str,
    default_marketplace: &str,
) -> Result<CachedCatalog, PluginError> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| {
            PluginError::new(
                "PLUGIN_CATALOG_UNAVAILABLE",
                format!("插件目录请求失败: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            PluginError::new(
                "PLUGIN_CATALOG_UNAVAILABLE",
                format!("插件目录返回错误: {error}"),
            )
        })?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        PluginError::new(
            "PLUGIN_CATALOG_UNAVAILABLE",
            format!("读取插件目录失败: {error}"),
        )
    })? {
        if bytes.len() + chunk.len() > MAX_CATALOG_BYTES {
            return Err(PluginError::new(
                "PLUGIN_INVALID_RESPONSE",
                "插件目录超过大小限制。",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|error| {
        PluginError::new(
            "PLUGIN_INVALID_RESPONSE",
            format!("解析插件目录失败: {error}"),
        )
    })?;
    let marketplace = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| is_safe_name(name))
        .unwrap_or(default_marketplace);
    let entries = parse_entries(
        paths,
        ExtPlatform::Antigravity,
        &value,
        false,
        Some(marketplace),
    )?;
    Ok(CachedCatalog {
        source_url: url.to_string(),
        entries,
    })
}

async fn antigravity_catalog(
    paths: &AppPaths,
    overrides: Option<&PluginTestOverrides>,
) -> Result<(Vec<PluginEntry>, bool, Vec<String>), PluginError> {
    let _guard = CATALOG_LOCK.lock().await;
    if paths.is_test
        && overrides
            .and_then(|value| value.catalog_urls.as_ref())
            .is_none()
    {
        return Ok((Vec::new(), false, Vec::new()));
    }
    let urls = overrides
        .and_then(|value| value.catalog_urls.clone())
        .unwrap_or_else(|| CATALOG_URLS.map(str::to_string));
    let cache_path = extension_store_path(paths, CACHE_FILE);
    check_sandbox_path(paths, &cache_path)?;
    let mut warnings = Vec::new();
    let mut cache = match read_json_store_checked::<PluginCatalogCache>(&cache_path) {
        Ok(cache) => cache,
        Err(error) => {
            warnings.push(error);
            PluginCatalogCache::default()
        }
    };
    let client = build_http_client(paths, Duration::from_secs(20), true)?;
    let (official, partners) = tokio::join!(
        fetch_catalog(paths, &client, &urls[0], OFFICIAL_MARKETPLACE),
        fetch_catalog(paths, &client, &urls[1], PARTNER_MARKETPLACE),
    );
    let mut entries = Vec::new();
    let mut from_cache = false;
    let mut changed = false;
    for ((name, url), result) in [OFFICIAL_MARKETPLACE, PARTNER_MARKETPLACE]
        .into_iter()
        .zip(urls)
        .zip([official, partners])
    {
        match result {
            Ok(catalog) => {
                entries.extend(catalog.entries.clone());
                cache.catalogs.insert(name.to_string(), catalog);
                changed = true;
            }
            Err(error) => {
                let cached = cache
                    .catalogs
                    .get(name)
                    .filter(|catalog| catalog.source_url.is_empty() || catalog.source_url == url);
                if let Some(catalog) = cached {
                    entries.extend(catalog.entries.clone());
                    from_cache = true;
                    warnings.push(format!("{name} 暂时不可用，正在使用本地缓存。"));
                } else {
                    warnings.push(format!("{name} 暂时不可用: {}", error.message));
                }
            }
        }
    }
    if changed {
        write_json_store(&cache_path, &cache)?;
    }
    let mut seen = HashSet::new();
    entries.retain(|entry| seen.insert(entry.id.clone()));
    Ok((entries, from_cache, warnings))
}

fn record_cached_installed_count(paths: &AppPaths, platform: ExtPlatform, count: usize) {
    let cache_path = extension_store_path(paths, CACHE_FILE);
    if check_sandbox_path(paths, &cache_path).is_err() {
        return;
    }
    if let Ok(mut cache) = read_json_store_checked::<PluginCatalogCache>(&cache_path) {
        if cache.installed_counts.get(&platform).copied() != Some(count) {
            cache.installed_counts.insert(platform, count);
            let _ = write_json_store(&cache_path, &cache);
        }
    }
}

fn count_disk_plugins_for_platform(paths: &AppPaths, platform: ExtPlatform) -> usize {
    match platform {
        ExtPlatform::Antigravity => {
            let root = paths.agy_gemini_config_dir.join("plugins");
            let Ok(entries) = fs::read_dir(root) else {
                return 0;
            };
            entries
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    is_installed_directory(&name)
                        && ["", ".antigravity-plugin", ".gemini-plugin", ".claude-plugin"]
                            .iter()
                            .any(|folder| entry.path().join(folder).join("plugin.json").is_file())
                })
                .count()
        }
        ExtPlatform::Claude => {
            let path = paths.home.join(".claude/plugins/installed_plugins.json");
            let Ok(bytes) = fs::read(path) else {
                return 0;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                return 0;
            };
            plugin_rows(&value, "installed")
                .map(|rows| rows.len())
                .unwrap_or(0)
        }
        ExtPlatform::Codex => {
            let plugins_dir = paths.codex_dir.join("plugins");
            if let Ok(entries) = fs::read_dir(&plugins_dir) {
                let dir_count = entries
                    .flatten()
                    .filter(|entry| entry.path().is_dir())
                    .count();
                if dir_count > 0 {
                    return dir_count;
                }
            }
            read_codex_doc(paths)
                .ok()
                .and_then(|doc| {
                    doc.get("plugins")
                        .and_then(Item::as_table_like)
                        .map(|table| table.len())
                })
                .unwrap_or(0)
        }
    }
}

/// 快速无子进程统计三端已安装插件数（优先使用最近一次 CLI 快照缓存，未探测端回退磁盘扫描）。
pub fn count_local_installed_plugins_for_paths(paths: &AppPaths) -> usize {
    let cache = read_json_store_checked::<PluginCatalogCache>(&extension_store_path(paths, CACHE_FILE))
        .unwrap_or_default();
    ExtPlatform::all()
        .iter()
        .map(|&platform| {
            cache
                .installed_counts
                .get(&platform)
                .copied()
                .unwrap_or_else(|| count_disk_plugins_for_platform(paths, platform))
        })
        .sum()
}

async fn snapshot(
    paths: &AppPaths,
    platform: ExtPlatform,
    include_available: bool,
    overrides: Option<&PluginTestOverrides>,
) -> Result<PluginsHubSnapshot, PluginError> {
    let cli = detect_cli(paths, platform, overrides)?;
    let mut snapshot = PluginsHubSnapshot {
        platform,
        cli_available: cli.is_some(),
        cli_path: cli
            .as_ref()
            .map(|cli| cli.path.to_string_lossy().into_owned()),
        cli_source: cli.as_ref().map(|cli| cli.source),
        marketplaces_supported: platform != ExtPlatform::Antigravity,
        installed: Vec::new(),
        available: Vec::new(),
        marketplaces: Vec::new(),
        catalog_from_cache: false,
        warnings: Vec::new(),
    };
    let Some(cli) = cli else { return Ok(snapshot) };
    snapshot.installed = installed_entries(paths, platform, &cli, overrides).await?;
    if !include_available {
        if overrides.is_none() || extension_store_path(paths, CACHE_FILE).exists() {
            record_cached_installed_count(paths, platform, snapshot.installed.len());
        }
        return Ok(snapshot);
    }
    if platform == ExtPlatform::Antigravity {
        let (available, from_cache, warnings) = antigravity_catalog(paths, overrides).await?;
        snapshot.available = available;
        snapshot.catalog_from_cache = from_cache;
        snapshot.warnings = warnings;
    } else {
        let (available, marketplaces) = tokio::join!(
            run_cli(
                paths,
                platform,
                &cli,
                &["plugin", "list", "--available", "--json"],
                false,
                overrides
            ),
            run_cli(
                paths,
                platform,
                &cli,
                &["plugin", "marketplace", "list", "--json"],
                false,
                overrides
            ),
        );
        snapshot.available = parse_entries(
            paths,
            platform,
            &extract_cli_json(&available?)?,
            false,
            None,
        )?;
        snapshot.marketplaces = parse_marketplaces(&extract_cli_json(&marketplaces?)?)?;
    }
    exclude_installed(&mut snapshot.available, &snapshot.installed);
    if overrides.is_none() || extension_store_path(paths, CACHE_FILE).exists() {
        record_cached_installed_count(paths, platform, snapshot.installed.len());
    }
    Ok(snapshot)
}

pub async fn get_plugins_hub_snapshot_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
    include_available: bool,
) -> Result<PluginsHubSnapshot, PluginError> {
    snapshot(paths, platform, include_available, None).await
}

pub async fn get_plugins_hub_snapshot_with_overrides(
    paths: &AppPaths,
    platform: ExtPlatform,
    include_available: bool,
    overrides: &PluginTestOverrides,
) -> Result<PluginsHubSnapshot, PluginError> {
    snapshot(paths, platform, include_available, Some(overrides)).await
}

fn validate_marketplace_source(source: &str) -> Result<(), PluginError> {
    if source.starts_with("https://") {
        let url = reqwest::Url::parse(source).map_err(|_| invalid_parameter())?;
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !source
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":/._-".contains(&byte))
        {
            return Err(invalid_parameter());
        }
        return Ok(());
    }
    let parts: Vec<_> = source.split('/').collect();
    if parts.len() == 2 && parts.iter().all(|part| is_safe_name(part)) {
        Ok(())
    } else {
        Err(invalid_parameter())
    }
}

fn validate_action_argument(
    platform: ExtPlatform,
    action: PluginAction,
    argument: &str,
) -> Result<(), PluginError> {
    match action {
        PluginAction::AddMarketplace => validate_marketplace_source(argument),
        PluginAction::RemoveMarketplace => {
            if is_safe_name(argument) {
                Ok(())
            } else {
                Err(invalid_parameter())
            }
        }
        PluginAction::Toggle(_) | PluginAction::Uninstall
            if platform == ExtPlatform::Antigravity =>
        {
            if is_installed_directory(argument) {
                Ok(())
            } else {
                Err(invalid_parameter())
            }
        }
        PluginAction::Install | PluginAction::Update if platform == ExtPlatform::Antigravity => {
            validate_plugin_id(argument)?;
            if !argument.contains('@') {
                return Err(invalid_parameter());
            }
            Ok(())
        }
        _ => validate_plugin_id(argument),
    }
}

async fn execute_action(
    paths: &AppPaths,
    platform: ExtPlatform,
    action: PluginAction,
    argument: &str,
    overrides: Option<&PluginTestOverrides>,
) -> Result<PluginActionOutcome, PluginError> {
    validate_action_argument(platform, action, argument)?;
    validate_overrides(paths, overrides)?;
    let _guard = MUTATION_LOCK.lock().await;
    let marketplace_action = matches!(
        action,
        PluginAction::AddMarketplace | PluginAction::RemoveMarketplace
    );
    if platform == ExtPlatform::Antigravity && marketplace_action {
        return Err(PluginError::new(
            "PLUGIN_UNSUPPORTED",
            "Antigravity 暂不支持管理市场源。",
        ));
    }
    let cli = detect_cli(paths, platform, overrides)?.ok_or_else(|| {
        PluginError::new(
            "PLUGIN_CLI_UNAVAILABLE",
            "未检测到对应 CLI，当前客户端暂不支持该功能",
        )
    })?;
    if action == PluginAction::RemoveMarketplace {
        let output = run_cli(
            paths,
            platform,
            &cli,
            &["plugin", "marketplace", "list", "--json"],
            false,
            overrides,
        )
        .await?;
        let markets = parse_marketplaces(&extract_cli_json(&output)?)?;
        if markets
            .iter()
            .any(|market| market.name == argument && !market.manageable)
        {
            return Err(PluginError::new(
                "PLUGIN_SCOPE_READ_ONLY",
                "此市场由项目或管理员管理，请在对应客户端内操作。",
            ));
        }
    }
    if matches!(
        action,
        PluginAction::Toggle(_) | PluginAction::Uninstall | PluginAction::Update
    ) {
        let installed = installed_entries(paths, platform, &cli, overrides).await?;
        let entry = installed
            .iter()
            .find(|entry| {
                if platform == ExtPlatform::Antigravity && !matches!(action, PluginAction::Update) {
                    entry.dir_name.as_deref() == Some(argument)
                } else if platform == ExtPlatform::Antigravity
                    && matches!(action, PluginAction::Update)
                {
                    entry.id == argument
                        || (entry.marketplace.is_none()
                            && entry.name == argument.split('@').next().unwrap_or(argument))
                } else {
                    entry.id == argument
                }
            })
            .ok_or_else(|| {
                PluginError::new("PLUGIN_NOT_INSTALLED", "未找到该已安装插件，请刷新后重试。")
            })?;
        // An unscoped CLI operation must never target another project's installation.
        if !entry.manageable
            || installed
                .iter()
                .any(|other| other.id == entry.id && !other.manageable)
        {
            return Err(PluginError::new(
                "PLUGIN_SCOPE_READ_ONLY",
                "此插件由项目或管理员管理，请在对应客户端内操作。",
            ));
        }
    }
    if let PluginAction::Toggle(enabled) = action {
        if platform == ExtPlatform::Codex {
            set_codex_enabled(paths, argument, enabled)?;
            return Ok(PluginActionOutcome {
                platform,
                plugin_id: argument.to_string(),
                needs_configuration: false,
                message: if enabled {
                    "插件已启用"
                } else {
                    "插件已停用"
                }
                .to_string(),
            });
        }
    }
    let verb = match action {
        PluginAction::Toggle(true) => "enable",
        PluginAction::Toggle(false) => "disable",
        PluginAction::Install => "install",
        PluginAction::Update if platform == ExtPlatform::Antigravity => "install",
        PluginAction::Update => "update",
        PluginAction::Uninstall => "uninstall",
        PluginAction::AddMarketplace => "add",
        PluginAction::RemoveMarketplace => "remove",
    };
    let mut args = vec!["plugin"];
    if marketplace_action {
        args.push("marketplace");
    }
    args.extend([verb, argument]);
    let long_running = matches!(
        action,
        PluginAction::Install | PluginAction::Update | PluginAction::AddMarketplace
    );
    run_cli(paths, platform, &cli, &args, long_running, overrides).await?;
    let mut needs_configuration = false;
    if platform == ExtPlatform::Antigravity
        && matches!(action, PluginAction::Install | PluginAction::Update)
    {
        let installed = installed_entries(paths, platform, &cli, overrides).await?;
        let name = argument.split('@').next().unwrap_or(argument);
        let entry = installed
            .iter()
            .find(|entry| entry.id == argument)
            .or_else(|| {
                installed
                    .iter()
                    .find(|entry| entry.name == name && entry.marketplace.is_none())
            })
            .ok_or_else(|| {
                PluginError::new(
                    "PLUGIN_INSTALL_NOT_VERIFIED",
                    "CLI 已返回，但未检测到安装结果，请刷新插件列表确认。",
                )
            })?;
        needs_configuration = !entry.enabled;
    }
    Ok(PluginActionOutcome {
        platform,
        plugin_id: argument.to_string(),
        needs_configuration,
        message: if needs_configuration {
            "插件已安装，请在客户端内完成授权配置。"
        } else {
            "插件操作已完成"
        }
        .to_string(),
    })
}

pub async fn execute_plugin_action_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
    action: PluginAction,
    argument: &str,
) -> Result<PluginActionOutcome, PluginError> {
    execute_action(paths, platform, action, argument, None).await
}

pub async fn execute_plugin_action_with_overrides(
    paths: &AppPaths,
    platform: ExtPlatform,
    action: PluginAction,
    argument: &str,
    overrides: &PluginTestOverrides,
) -> Result<PluginActionOutcome, PluginError> {
    execute_action(paths, platform, action, argument, Some(overrides)).await
}
