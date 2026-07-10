#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{collections::BTreeSet, env, path::Path, path::PathBuf, process::Command};
#[cfg(windows)]
use serde::Deserialize;
#[cfg(windows)]
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    RegKey,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AppInstallMarker {
    Path(PathBuf),
    #[cfg(windows)]
    WindowsStoreApp(WindowsStoreApp),
}

impl AppInstallMarker {
    fn exists(&self) -> bool {
        match self {
            AppInstallMarker::Path(path) => path.exists(),
            #[cfg(windows)]
            AppInstallMarker::WindowsStoreApp(app) => !app.app_id.is_empty(),
        }
    }

    fn display(&self) -> String {
        match self {
            AppInstallMarker::Path(path) => path.to_string_lossy().to_string(),
            #[cfg(windows)]
            AppInstallMarker::WindowsStoreApp(app) => app.display_path(),
        }
    }
}

impl From<PathBuf> for AppInstallMarker {
    fn from(path: PathBuf) -> Self {
        AppInstallMarker::Path(path)
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum WindowsAppLaunchTarget {
    Path(PathBuf),
    AppId(String),
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WindowsStoreApp {
    pub app_id: String,
    pub install_location: Option<PathBuf>,
}

#[cfg(windows)]
impl WindowsStoreApp {
    fn display_path(&self) -> String {
        self.install_location
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| windows_shell_app_folder_target(&self.app_id))
    }
}

#[cfg(windows)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsStoreAppJson {
    app_id: Option<String>,
    install_location: Option<String>,
}

#[cfg(windows)]
impl WindowsStoreApp {
    fn from_json(raw: WindowsStoreAppJson) -> Option<Self> {
        let app_id = raw.app_id?.trim().to_string();
        if app_id.is_empty() {
            return None;
        }

        Some(Self {
            app_id,
            install_location: raw
                .install_location
                .map(|path| path.trim().to_string())
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        })
    }
}

#[cfg(windows)]
impl WindowsAppLaunchTarget {
    fn display(&self) -> String {
        match self {
            WindowsAppLaunchTarget::Path(path) => path.to_string_lossy().to_string(),
            WindowsAppLaunchTarget::AppId(app_id) => windows_shell_app_folder_target(app_id),
        }
    }
}

pub fn codex_app_supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

pub fn claude_app_supported() -> bool {
    true
}

pub fn detect_app_from_markers<M>(markers: Vec<M>) -> Option<String>
where
    M: Into<AppInstallMarker>,
{
    markers
        .into_iter()
        .map(Into::into)
        .find(|marker| marker.exists())
        .map(|marker| marker.display())
}

pub fn open_tool_app(tool: &str) -> Result<(), String> {
    match normalize_tool_key(tool).as_deref() {
        Some("codex") => open_codex_app(),
        Some("claude") => open_claude_app(),
        _ => Err("不支持打开这个工具的桌面端。".to_string()),
    }
}

fn normalize_tool_key(tool: &str) -> Option<&'static str> {
    let normalized = tool.trim().to_lowercase();
    if normalized.contains("claude") {
        Some("claude")
    } else if normalized.contains("codex") || normalized.contains("chatgpt") {
        Some("codex")
    } else {
        None
    }
}

fn open_codex_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return open_macos_app(&["ChatGPT", "Codex"], "未能打开 ChatGPT / Codex 桌面端。");
    }

    #[cfg(windows)]
    {
        return open_windows_app(
            codex_app_launch_candidates(),
            &["ChatGPT.exe", "Codex.exe"],
            "未找到可启动的 ChatGPT / Codex 桌面端。",
        );
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Err("Linux 暂不支持 Codex 桌面端检测，请使用 Codex CLI。".to_string())
    }
}

fn open_claude_app() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return open_macos_app(&["Claude"], "未能打开 Claude 桌面端。");
    }

    #[cfg(windows)]
    {
        return open_windows_app(
            claude_app_launch_candidates(),
            &["Claude.exe"],
            "未找到可启动的 Claude 桌面端。",
        );
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        open_linux_desktop_app(
            &["claude", "Claude", "com.anthropic.Claude"],
            claude_app_install_markers(),
            "未找到可启动的 Claude 桌面端。",
        )
    }
}

#[cfg(target_os = "macos")]
fn open_macos_app(app_names: &[&str], fallback_error: &str) -> Result<(), String> {
    let mut errors = Vec::new();
    for app_name in app_names {
        match Command::new("open").arg("-a").arg(app_name).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{app_name}: {err}")),
        }
    }

    Err(format!(
        "{fallback_error} 请确认已安装后重试。{}",
        format_launch_diagnostics(errors)
    ))
}

#[cfg(windows)]
fn open_windows_app(
    launch_candidates: Vec<WindowsAppLaunchTarget>,
    fallback_commands: &[&str],
    fallback_error: &str,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for candidate in launch_candidates {
        match launch_windows_target(&candidate) {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{}: {err}", candidate.display())),
        }
    }

    for command in fallback_commands {
        let mut process = Command::new(command);
        process.creation_flags(CREATE_NO_WINDOW);
        match process.spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{command}: {err}")),
        }
    }

    Err(format!(
        "{fallback_error} 可以点“前往下载”安装桌面端。{}",
        format_launch_diagnostics(errors)
    ))
}

#[cfg(windows)]
fn launch_windows_target(target: &WindowsAppLaunchTarget) -> Result<(), String> {
    match target {
        WindowsAppLaunchTarget::Path(path) => {
            if !path.exists() {
                return Err("path does not exist".to_string());
            }
            launch_windows_path(path)
        }
        WindowsAppLaunchTarget::AppId(app_id) => launch_windows_app_id(app_id),
    }
}

#[cfg(windows)]
fn launch_windows_path(path: &Path) -> Result<(), String> {
    let mut process = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("lnk"))
        .unwrap_or(false)
    {
        let mut command = Command::new("cmd");
        command.arg("/C").arg("start").arg("").arg(path);
        command
    } else {
        Command::new(path)
    };
    process.creation_flags(CREATE_NO_WINDOW);
    process.spawn().map(|_| ()).map_err(|err| err.to_string())
}

#[cfg(windows)]
fn launch_windows_app_id(app_id: &str) -> Result<(), String> {
    let mut process = Command::new("explorer.exe");
    process.arg(windows_shell_app_folder_target(app_id));
    process.creation_flags(CREATE_NO_WINDOW);
    process.spawn().map(|_| ()).map_err(|err| err.to_string())
}

#[cfg(windows)]
fn windows_shell_app_folder_target(app_id: &str) -> String {
    format!("shell:AppsFolder\\{app_id}")
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn open_linux_desktop_app(
    desktop_ids: &[&str],
    desktop_files: Vec<PathBuf>,
    fallback_error: &str,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for desktop_id in desktop_ids {
        match Command::new("gtk-launch").arg(desktop_id).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("gtk-launch {desktop_id}: {err}")),
        }

        let desktop_file_id = format!("{desktop_id}.desktop");
        match Command::new("gtk-launch").arg(&desktop_file_id).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("gtk-launch {desktop_file_id}: {err}")),
        }
    }

    for desktop_file in desktop_files {
        if !desktop_file.exists() {
            continue;
        }

        match Command::new("xdg-open").arg(&desktop_file).spawn() {
            Ok(_) => return Ok(()),
            Err(err) => errors.push(format!("{}: {err}", desktop_file.display())),
        }
    }

    Err(format!(
        "{fallback_error} 可以点“前往下载”安装桌面端。{}",
        format_launch_diagnostics(errors)
    ))
}

fn format_launch_diagnostics(errors: Vec<String>) -> String {
    errors
        .last()
        .map(|error| format!("最后一次尝试：{error}"))
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn codex_app_command_candidates() -> Vec<PathBuf> {
    let mut candidates =
        windows_codex_app_command_candidates_from_base_dirs(windows_codex_app_base_dirs());

    for install_dir in windows_codex_registry_install_dirs() {
        candidates.push(install_dir.join("bin").join("codex.exe"));
        candidates.push(install_dir.join("codex.exe"));
    }

    unique_paths(candidates)
}

#[cfg(target_os = "macos")]
pub fn codex_app_command_candidates() -> Vec<PathBuf> {
    codex_legacy_app_install_markers()
        .into_iter()
        .map(|path| path.join("Contents").join("MacOS").join("Codex"))
        .collect()
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn codex_app_command_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn codex_app_install_markers() -> Vec<PathBuf> {
    windows_codex_app_path_marker_candidates()
}

#[cfg(windows)]
pub fn codex_app_install_marker_candidates() -> Vec<AppInstallMarker> {
    windows_codex_app_marker_candidates()
}

#[cfg(target_os = "macos")]
pub fn codex_app_install_markers() -> Vec<PathBuf> {
    let mut markers = codex_legacy_app_install_markers();
    markers.push(PathBuf::from("/Applications/ChatGPT.app"));
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("ChatGPT.app"));
    }
    unique_paths(markers)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn codex_app_install_markers() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn codex_app_install_marker_candidates() -> Vec<PathBuf> {
    codex_app_install_markers()
}

#[cfg(target_os = "macos")]
fn codex_legacy_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![PathBuf::from("/Applications/Codex.app")];
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("Codex.app"));
    }
    unique_paths(markers)
}

#[cfg(windows)]
#[allow(dead_code)]
pub fn claude_app_install_markers() -> Vec<PathBuf> {
    windows_claude_app_path_marker_candidates()
}

#[cfg(windows)]
pub fn claude_app_install_marker_candidates() -> Vec<AppInstallMarker> {
    windows_claude_app_marker_candidates()
}

#[cfg(windows)]
fn claude_app_launch_candidates() -> Vec<WindowsAppLaunchTarget> {
    windows_claude_app_launch_targets()
}

#[cfg(target_os = "macos")]
pub fn claude_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![PathBuf::from("/Applications/Claude.app")];
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("Claude.app"));
    }
    unique_paths(markers)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub fn claude_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![
        PathBuf::from("/usr/share/applications/claude.desktop"),
        PathBuf::from("/usr/share/applications/Claude.desktop"),
        PathBuf::from("/usr/local/share/applications/claude.desktop"),
        PathBuf::from("/opt/Claude"),
        PathBuf::from("/opt/Claude/claude"),
    ];
    if let Some(home) = env::var_os("HOME") {
        let applications = PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("applications");
        markers.push(applications.join("claude.desktop"));
        markers.push(applications.join("Claude.desktop"));
        markers.push(applications.join("com.anthropic.Claude.desktop"));
    }
    unique_paths(markers)
}

#[cfg(not(windows))]
pub fn claude_app_install_marker_candidates() -> Vec<PathBuf> {
    claude_app_install_markers()
}

#[cfg(windows)]
pub fn claude_cli_command_candidates() -> Vec<PathBuf> {
    let mut candidates =
        windows_claude_cli_command_candidates_from_base_dirs(windows_claude_cli_base_dirs());

    for install_dir in windows_tool_registry_install_dirs("Claude Code") {
        candidates.push(install_dir.join("claude.exe"));
        candidates.push(install_dir.join("bin").join("claude.exe"));
    }

    unique_paths(candidates)
}

#[cfg(not(windows))]
pub fn claude_cli_command_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".local")
                .join("bin")
                .join("claude"),
        );
    }
    unique_paths(candidates)
}

#[cfg(windows)]
fn windows_codex_app_base_dirs() -> Vec<PathBuf> {
    [
        env::var_os("LOCALAPPDATA"),
        env::var_os("APPDATA"),
        env::var_os("ProgramFiles"),
        env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .collect()
}

#[cfg(windows)]
fn windows_start_menu_program_dirs() -> Vec<PathBuf> {
    [
        env::var_os("APPDATA").map(|path| {
            PathBuf::from(path)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        }),
        env::var_os("ProgramData").map(|path| {
            PathBuf::from(path)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(windows)]
fn windows_claude_cli_base_dirs() -> Vec<PathBuf> {
    [
        env::var_os("USERPROFILE"),
        env::var_os("HOME"),
        env::var_os("LOCALAPPDATA"),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .collect()
}

#[cfg(windows)]
fn windows_claude_app_base_dirs() -> Vec<PathBuf> {
    [
        env::var_os("LOCALAPPDATA"),
        env::var_os("ProgramFiles"),
        env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    .map(PathBuf::from)
    .collect()
}

#[cfg(windows)]
pub fn windows_codex_app_command_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join("codex.exe"),
                base_dir.join("Codex").join("bin").join("codex.exe"),
            ]
        })
        .collect()
}

#[cfg(windows)]
pub fn windows_claude_cli_command_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join(".local").join("bin").join("claude.exe"),
                base_dir.join(".local").join("bin").join("claude"),
                base_dir
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links")
                    .join("claude.exe"),
                base_dir
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Packages")
                    .join("Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe")
                    .join("claude.exe"),
            ]
        })
        .collect()
}

#[cfg(windows)]
pub fn windows_claude_app_marker_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut markers: Vec<PathBuf> = base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join("Claude"),
                base_dir.join("Claude").join("Claude.exe"),
                base_dir.join("Programs").join("Claude"),
                base_dir.join("Programs").join("Claude").join("Claude.exe"),
                base_dir.join("Anthropic").join("Claude"),
                base_dir.join("Anthropic").join("Claude").join("Claude.exe"),
            ]
        })
        .collect();

    markers.extend(start_menu_dirs.into_iter().flat_map(|dir| {
        [
            dir.join("Claude.lnk"),
            dir.join("Anthropic").join("Claude.lnk"),
        ]
    }));

    markers
}

#[cfg(windows)]
fn windows_claude_app_path_marker_candidates() -> Vec<PathBuf> {
    let mut markers = windows_claude_app_marker_candidates_from_base_dirs(
        windows_claude_app_base_dirs(),
        windows_start_menu_program_dirs(),
    );
    for install_dir in windows_claude_desktop_registry_install_dirs() {
        markers.push(install_dir.clone());
        markers.push(install_dir.join("Claude.exe"));
    }
    unique_paths(markers)
}

#[cfg(windows)]
fn windows_claude_app_marker_candidates() -> Vec<AppInstallMarker> {
    let paths = windows_claude_app_path_marker_candidates();
    unique_markers(paths.into_iter().map(AppInstallMarker::Path).collect())
}

#[cfg(windows)]
pub fn windows_claude_app_launch_targets_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<WindowsAppLaunchTarget> {
    let mut candidates: Vec<PathBuf> = base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join("Claude").join("Claude.exe"),
                base_dir.join("Programs").join("Claude").join("Claude.exe"),
                base_dir.join("Anthropic").join("Claude").join("Claude.exe"),
            ]
        })
        .collect();

    candidates.extend(start_menu_dirs.into_iter().flat_map(|dir| {
        [
            dir.join("Claude.lnk"),
            dir.join("Anthropic").join("Claude.lnk"),
        ]
    }));

    candidates
        .into_iter()
        .map(WindowsAppLaunchTarget::Path)
        .collect()
}

#[cfg(windows)]
fn windows_claude_app_launch_targets() -> Vec<WindowsAppLaunchTarget> {
    let mut candidates: Vec<WindowsAppLaunchTarget> = Vec::new();
    candidates.extend(windows_claude_app_launch_targets_from_base_dirs(
        windows_claude_app_base_dirs(),
        windows_start_menu_program_dirs(),
    ));
    for install_dir in windows_claude_desktop_registry_install_dirs() {
        candidates.push(WindowsAppLaunchTarget::Path(install_dir.join("Claude.exe")));
    }
    unique_windows_launch_targets(candidates)
}

#[cfg(windows)]
pub fn windows_codex_app_marker_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut markers: Vec<PathBuf> = base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join("OpenAI").join("ChatGPT"),
                base_dir.join("ChatGPT"),
                base_dir.join("Programs").join("ChatGPT"),
                base_dir
                    .join("Programs")
                    .join("ChatGPT")
                    .join("ChatGPT.exe"),
            ]
        })
        .collect();

    markers.extend(start_menu_dirs.into_iter().flat_map(|dir| {
        [
            dir.join("Codex.lnk"),
            dir.join("OpenAI").join("Codex.lnk"),
            dir.join("ChatGPT.lnk"),
            dir.join("OpenAI").join("ChatGPT.lnk"),
        ]
    }));

    markers
}

#[cfg(windows)]
fn windows_codex_app_path_marker_candidates() -> Vec<PathBuf> {
    let mut markers = windows_codex_app_marker_candidates_from_base_dirs(
        windows_codex_app_base_dirs(),
        windows_start_menu_program_dirs(),
    );
    for install_dir in windows_codex_registry_install_dirs() {
        markers.push(install_dir.clone());
        markers.push(install_dir.join("ChatGPT.exe"));
        markers.push(install_dir.join("Codex.exe"));
    }
    unique_paths(markers)
}

#[cfg(windows)]
fn windows_codex_app_marker_candidates() -> Vec<AppInstallMarker> {
    let store_apps = windows_store_apps_by_package_names(&["OpenAI.Codex"]);
    let paths = windows_codex_app_path_marker_candidates();
    unique_markers(
        store_apps
            .into_iter()
            .map(AppInstallMarker::WindowsStoreApp)
            .chain(paths.into_iter().map(AppInstallMarker::Path))
            .collect(),
    )
}

#[cfg(windows)]
fn codex_app_launch_candidates() -> Vec<WindowsAppLaunchTarget> {
    let mut candidates: Vec<WindowsAppLaunchTarget> =
        windows_store_apps_by_package_names(&["OpenAI.Codex"])
            .into_iter()
            .map(|app| WindowsAppLaunchTarget::AppId(app.app_id))
            .collect();
    candidates.extend(windows_codex_app_launch_targets_from_base_dirs(
        windows_codex_app_base_dirs(),
        windows_start_menu_program_dirs(),
    ));
    for install_dir in windows_codex_registry_install_dirs() {
        candidates.push(WindowsAppLaunchTarget::Path(install_dir.join("ChatGPT.exe")));
        candidates.push(WindowsAppLaunchTarget::Path(install_dir.join("Codex.exe")));
    }
    unique_windows_launch_targets(candidates)
}

#[cfg(windows)]
pub fn windows_codex_app_launch_targets_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<WindowsAppLaunchTarget> {
    let mut candidates: Vec<PathBuf> = base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join("OpenAI").join("ChatGPT").join("ChatGPT.exe"),
                base_dir.join("ChatGPT").join("ChatGPT.exe"),
                base_dir
                    .join("Programs")
                    .join("ChatGPT")
                    .join("ChatGPT.exe"),
            ]
        })
        .collect();

    candidates.extend(start_menu_dirs.into_iter().flat_map(|dir| {
        [
            dir.join("ChatGPT.lnk"),
            dir.join("OpenAI").join("ChatGPT.lnk"),
            dir.join("Codex.lnk"),
            dir.join("OpenAI").join("Codex.lnk"),
        ]
    }));

    candidates
        .into_iter()
        .map(WindowsAppLaunchTarget::Path)
        .collect()
}

#[cfg(windows)]
fn windows_codex_registry_install_dirs() -> Vec<PathBuf> {
    let mut dirs = windows_tool_registry_install_dirs("Codex");
    dirs.extend(windows_tool_registry_install_dirs("ChatGPT"));
    unique_paths(dirs)
}

#[cfg(windows)]
fn windows_claude_desktop_registry_install_dirs() -> Vec<PathBuf> {
    windows_tool_registry_install_dirs_by_display_name(|display_name| {
        let normalized = display_name.to_lowercase();
        normalized.contains("claude") && !normalized.contains("code")
    })
}

#[cfg(windows)]
fn windows_tool_registry_install_dirs(display_name_keyword: &str) -> Vec<PathBuf> {
    let keyword = display_name_keyword.to_lowercase();
    windows_tool_registry_install_dirs_by_display_name(move |display_name| {
        display_name.to_lowercase().contains(&keyword)
    })
}

#[cfg(windows)]
fn windows_tool_registry_install_dirs_by_display_name<F>(matches_display_name: F) -> Vec<PathBuf>
where
    F: Fn(&str) -> bool,
{
    [
        (
            HKEY_CURRENT_USER,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
    ]
    .into_iter()
    .flat_map(|(hkey, subkey)| {
        windows_uninstall_tool_install_dirs(hkey, subkey, &matches_display_name)
    })
    .collect()
}

#[cfg(windows)]
fn windows_uninstall_tool_install_dirs<F>(
    hkey: winreg::HKEY,
    subkey: &str,
    matches_display_name: &F,
) -> Vec<PathBuf>
where
    F: Fn(&str) -> bool,
{
    let root = RegKey::predef(hkey);
    let Ok(uninstall) = root.open_subkey(subkey) else {
        return Vec::new();
    };

    uninstall
        .enum_keys()
        .flatten()
        .filter_map(|key| uninstall.open_subkey(key).ok())
        .filter(|app| {
            app.get_value::<String, _>("DisplayName")
                .map(|name| matches_display_name(&name))
                .unwrap_or(false)
        })
        .filter_map(|app| app.get_value::<String, _>("InstallLocation").ok())
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

#[cfg(windows)]
fn windows_store_apps_by_package_names(package_names: &[&str]) -> Vec<WindowsStoreApp> {
    let apps = package_names
        .iter()
        .flat_map(|package_name| windows_store_apps_by_package_name(package_name))
        .filter(|app| is_allowed_store_app_id(&app.app_id, package_names))
        .collect::<Vec<_>>();
    unique_store_apps(apps)
}

#[cfg(windows)]
fn windows_store_apps_by_package_name(package_name: &str) -> Vec<WindowsStoreApp> {
    let script = format!(
        r#"
$package = Get-AppxPackage -Name '{package_name}' -ErrorAction SilentlyContinue
if ($null -eq $package) {{
  return
}}
$manifest = Get-AppxPackageManifest -Package $package.PackageFullName
$manifest.Package.Applications.Application |
  ForEach-Object {{
    [PSCustomObject]@{{
      appId = "$($package.PackageFamilyName)!$($_.Id)"
      installLocation = $package.InstallLocation
    }}
  }} |
  ConvertTo-Json -Compress
"#,
        package_name = powershell_single_quoted_literal(package_name)
    );

    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script);
    command.creation_flags(CREATE_NO_WINDOW);

    match command.output() {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            parse_windows_store_apps_json(&text)
        }
        _ => Vec::new(),
    }
}

#[cfg(windows)]
fn powershell_single_quoted_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(windows)]
fn parse_windows_store_apps_json(text: &str) -> Vec<WindowsStoreApp> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(apps) = serde_json::from_str::<Vec<WindowsStoreAppJson>>(trimmed) {
        return apps.into_iter().filter_map(WindowsStoreApp::from_json).collect();
    }

    serde_json::from_str::<WindowsStoreAppJson>(trimmed)
        .ok()
        .and_then(WindowsStoreApp::from_json)
        .into_iter()
        .collect()
}

#[cfg(windows)]
pub fn is_allowed_store_app_id(app_id: &str, package_names: &[&str]) -> bool {
    let Some((family_name, application_id)) = app_id.split_once('!') else {
        return false;
    };
    if family_name.trim().is_empty() || application_id.trim().is_empty() {
        return false;
    }

    package_names
        .iter()
        .any(|package_name| family_name.starts_with(&format!("{package_name}_")))
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(windows)]
fn unique_store_apps(apps: Vec<WindowsStoreApp>) -> Vec<WindowsStoreApp> {
    let mut seen = BTreeSet::new();
    apps.into_iter()
        .filter(|app| seen.insert(app.app_id.clone()))
        .collect()
}

fn unique_markers(markers: Vec<AppInstallMarker>) -> Vec<AppInstallMarker> {
    let mut seen = BTreeSet::new();
    markers
        .into_iter()
        .filter(|marker| seen.insert(marker.clone()))
        .collect()
}

#[cfg(windows)]
fn unique_windows_launch_targets(
    targets: Vec<WindowsAppLaunchTarget>,
) -> Vec<WindowsAppLaunchTarget> {
    let mut seen = BTreeSet::new();
    targets
        .into_iter()
        .filter(|target| seen.insert(target.clone()))
        .collect()
}
