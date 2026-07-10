#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{collections::BTreeSet, env, path::Path, path::PathBuf, process::Command};
#[cfg(windows)]
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    RegKey,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn codex_app_supported() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

pub fn claude_app_supported() -> bool {
    true
}

pub fn detect_app_from_markers(markers: Vec<PathBuf>) -> Option<PathBuf> {
    markers.into_iter().find(|path| path.exists())
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
    launch_candidates: Vec<PathBuf>,
    fallback_commands: &[&str],
    fallback_error: &str,
) -> Result<(), String> {
    let mut errors = Vec::new();

    for candidate in launch_candidates {
        if !candidate.exists() {
            continue;
        }

        match launch_windows_path(&candidate) {
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
pub fn codex_app_install_markers() -> Vec<PathBuf> {
    let mut markers = windows_codex_app_marker_candidates_from_base_dirs(
        windows_codex_app_base_dirs(),
        windows_start_menu_program_dirs(),
    );
    markers.extend(windows_codex_registry_install_dirs());
    unique_paths(markers)
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

#[cfg(target_os = "macos")]
fn codex_legacy_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![PathBuf::from("/Applications/Codex.app")];
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("Codex.app"));
    }
    unique_paths(markers)
}

#[cfg(windows)]
pub fn claude_app_install_markers() -> Vec<PathBuf> {
    unique_paths(windows_claude_app_marker_candidates_from_base_dirs(
        windows_claude_app_base_dirs(),
        windows_start_menu_program_dirs(),
    ))
}

#[cfg(windows)]
fn claude_app_launch_candidates() -> Vec<PathBuf> {
    unique_paths(windows_claude_app_launch_candidates_from_base_dirs(
        windows_claude_app_base_dirs(),
        windows_start_menu_program_dirs(),
    ))
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
                base_dir
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("Claude.exe"),
                base_dir.join("Packages").join("Claude_pzs8sxrjxfjjc"),
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
pub fn windows_claude_app_launch_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = base_dirs
        .into_iter()
        .flat_map(|base_dir| {
            [
                base_dir.join("Claude").join("Claude.exe"),
                base_dir.join("Programs").join("Claude").join("Claude.exe"),
                base_dir.join("Anthropic").join("Claude").join("Claude.exe"),
                base_dir
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("Claude.exe"),
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
                base_dir.join("OpenAI").join("Codex"),
                base_dir
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join("codex.exe"),
                base_dir.join("Codex"),
                base_dir.join("Codex").join("bin").join("codex.exe"),
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
fn codex_app_launch_candidates() -> Vec<PathBuf> {
    let mut candidates = windows_codex_app_launch_candidates_from_base_dirs(
        windows_codex_app_base_dirs(),
        windows_start_menu_program_dirs(),
    );

    for install_dir in windows_codex_registry_install_dirs() {
        candidates.push(install_dir.join("ChatGPT.exe"));
        candidates.push(install_dir.join("Codex.exe"));
        candidates.push(install_dir.join("bin").join("codex.exe"));
    }

    unique_paths(candidates)
}

#[cfg(windows)]
pub fn windows_codex_app_launch_candidates_from_base_dirs(
    base_dirs: Vec<PathBuf>,
    start_menu_dirs: Vec<PathBuf>,
) -> Vec<PathBuf> {
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
                base_dir
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join("codex.exe"),
                base_dir.join("Codex").join("bin").join("codex.exe"),
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
}

#[cfg(windows)]
fn windows_codex_registry_install_dirs() -> Vec<PathBuf> {
    let mut dirs = windows_tool_registry_install_dirs("Codex");
    dirs.extend(windows_tool_registry_install_dirs("ChatGPT"));
    unique_paths(dirs)
}

#[cfg(windows)]
fn windows_tool_registry_install_dirs(display_name_keyword: &str) -> Vec<PathBuf> {
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
        windows_uninstall_tool_install_dirs(hkey, subkey, display_name_keyword)
    })
    .collect()
}

#[cfg(windows)]
fn windows_uninstall_tool_install_dirs(
    hkey: winreg::HKEY,
    subkey: &str,
    display_name_keyword: &str,
) -> Vec<PathBuf> {
    let root = RegKey::predef(hkey);
    let Ok(uninstall) = root.open_subkey(subkey) else {
        return Vec::new();
    };

    let display_name_keyword = display_name_keyword.to_lowercase();
    uninstall
        .enum_keys()
        .flatten()
        .filter_map(|key| uninstall.open_subkey(key).ok())
        .filter(|app| {
            app.get_value::<String, _>("DisplayName")
                .map(|name| name.to_lowercase().contains(&display_name_keyword))
                .unwrap_or(false)
        })
        .filter_map(|app| app.get_value::<String, _>("InstallLocation").ok())
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}
