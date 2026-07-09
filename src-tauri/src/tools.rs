use crate::models::ToolStatus;
#[cfg(any(windows, target_os = "macos"))]
use std::env;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::{collections::BTreeSet, path::PathBuf};
#[cfg(windows)]
use winreg::{
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    RegKey,
};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn detect_tools() -> Vec<ToolStatus> {
    vec![detect_codex_tool(), detect_claude_tool()]
}

fn detect_codex_tool() -> ToolStatus {
    let status = detect_tool("Codex", codex_command_candidates());
    if status.installed {
        return status;
    }

    detect_codex_app_status().unwrap_or(status)
}

fn codex_command_candidates() -> Vec<String> {
    let mut candidates = tool_command_candidates("codex");
    candidates.extend(
        codex_app_command_candidates()
            .into_iter()
            .map(|path| path.to_string_lossy().to_string()),
    );
    unique_non_empty(candidates)
}

fn detect_claude_tool() -> ToolStatus {
    let status = detect_tool_with_version_validator(
        "Claude Code",
        claude_command_candidates(),
        is_claude_code_version_output,
    );
    if status.installed {
        return status;
    }

    detect_claude_app_status().unwrap_or(status)
}

fn claude_command_candidates() -> Vec<String> {
    let mut candidates = tool_command_candidates("claude");
    candidates.extend(
        claude_cli_command_candidates()
            .into_iter()
            .map(|path| path.to_string_lossy().to_string()),
    );
    unique_non_empty(candidates)
}

fn tool_command_candidates(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        return vec![
            format!("{command}.cmd"),
            format!("{command}.exe"),
            command.to_string(),
        ];
    }

    #[cfg(all(not(windows), target_os = "macos"))]
    {
        return macos_tool_command_candidates(command);
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        vec![command.to_string()]
    }
}

#[cfg(target_os = "macos")]
fn macos_tool_command_candidates(command: &str) -> Vec<String> {
    let mut candidates = vec![command.to_string()];
    candidates.extend(
        macos_common_bin_dirs()
            .into_iter()
            .map(|dir| dir.join(command).to_string_lossy().to_string()),
    );

    unique_non_empty(candidates)
}

fn detect_tool(name: &str, commands: Vec<String>) -> ToolStatus {
    detect_tool_with_version_validator(name, commands, |_| true)
}

fn detect_tool_with_version_validator<F>(
    name: &str,
    commands: Vec<String>,
    is_valid_version: F,
) -> ToolStatus
where
    F: Fn(Option<&str>) -> bool,
{
    let mut errors = Vec::new();

    for command in &commands {
        match command_version(command) {
            Ok((version, None)) => {
                if !is_valid_version(version.as_deref()) {
                    let details = version
                        .map(|value| format!("版本输出不符合 {name}：{value}"))
                        .unwrap_or_else(|| format!("版本输出不符合 {name}。"));
                    errors.push(format!("{command}: {details}"));
                    continue;
                }

                return ToolStatus {
                    name: name.to_string(),
                    installed: true,
                    version,
                    error: None,
                };
            }
            Ok((version, Some(error))) => {
                let details = version
                    .map(|value| format!("{error}；输出：{value}"))
                    .unwrap_or(error);
                errors.push(format!("{command}: {details}"));
            }
            Err(error) => errors.push(format!("{command}: {error}")),
        }
    }

    ToolStatus {
        name: name.to_string(),
        installed: false,
        version: None,
        error: Some(format!(
            "已尝试 {}。最后一次错误：{}",
            commands.join(", "),
            errors
                .last()
                .cloned()
                .unwrap_or_else(|| "没有可用的命令候选。".to_string())
        )),
    }
}

fn is_claude_code_version_output(version: Option<&str>) -> bool {
    version
        .map(|value| value.to_lowercase().contains("claude code"))
        .unwrap_or(false)
}

#[cfg(any(windows, target_os = "macos"))]
fn detect_claude_app_status() -> Option<ToolStatus> {
    detect_claude_app_status_from_markers(claude_app_install_markers())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn detect_claude_app_status() -> Option<ToolStatus> {
    None
}

#[cfg(any(windows, target_os = "macos"))]
fn detect_claude_app_status_from_markers(markers: Vec<PathBuf>) -> Option<ToolStatus> {
    markers
        .into_iter()
        .find(|path| path.exists())
        .map(|_| ToolStatus {
            name: "Claude Code".to_string(),
            installed: true,
            version: Some("Claude Desktop 已安装（未检测到 claude 命令）".to_string()),
            error: None,
        })
}

#[cfg(any(windows, target_os = "macos"))]
fn detect_codex_app_status() -> Option<ToolStatus> {
    detect_codex_app_status_from_markers(codex_app_install_markers())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn detect_codex_app_status() -> Option<ToolStatus> {
    None
}

#[cfg(any(windows, target_os = "macos"))]
fn detect_codex_app_status_from_markers(markers: Vec<PathBuf>) -> Option<ToolStatus> {
    markers
        .into_iter()
        .find(|path| path.exists())
        .map(|_| ToolStatus {
            name: "Codex".to_string(),
            installed: true,
            version: Some("Codex App 已安装（未检测到 codex 命令）".to_string()),
            error: None,
        })
}

#[cfg(windows)]
fn codex_app_command_candidates() -> Vec<PathBuf> {
    let mut candidates =
        windows_codex_app_command_candidates_from_base_dirs(windows_codex_app_base_dirs());

    for install_dir in windows_codex_registry_install_dirs() {
        candidates.push(install_dir.join("bin").join("codex.exe"));
        candidates.push(install_dir.join("codex.exe"));
    }

    unique_paths(candidates)
}

#[cfg(target_os = "macos")]
fn codex_app_command_candidates() -> Vec<PathBuf> {
    codex_app_install_markers()
        .into_iter()
        .map(|path| path.join("Contents").join("MacOS").join("Codex"))
        .collect()
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn codex_app_command_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn codex_app_install_markers() -> Vec<PathBuf> {
    let mut markers = windows_codex_app_marker_candidates_from_base_dirs(
        windows_codex_app_base_dirs(),
        windows_start_menu_program_dirs(),
    );
    markers.extend(windows_codex_registry_install_dirs());
    unique_paths(markers)
}

#[cfg(windows)]
fn claude_app_install_markers() -> Vec<PathBuf> {
    unique_paths(windows_claude_app_marker_candidates_from_base_dirs(
        windows_claude_app_base_dirs(),
        windows_start_menu_program_dirs(),
    ))
}

#[cfg(windows)]
fn claude_cli_command_candidates() -> Vec<PathBuf> {
    let mut candidates =
        windows_claude_cli_command_candidates_from_base_dirs(windows_claude_cli_base_dirs());

    for install_dir in windows_tool_registry_install_dirs("Claude Code") {
        candidates.push(install_dir.join("claude.exe"));
        candidates.push(install_dir.join("bin").join("claude.exe"));
    }

    unique_paths(candidates)
}

#[cfg(target_os = "macos")]
fn codex_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![PathBuf::from("/Applications/Codex.app")];
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("Codex.app"));
    }
    unique_paths(markers)
}

#[cfg(target_os = "macos")]
fn claude_app_install_markers() -> Vec<PathBuf> {
    let mut markers = vec![PathBuf::from("/Applications/Claude.app")];
    if let Some(home) = env::var_os("HOME") {
        markers.push(PathBuf::from(home).join("Applications").join("Claude.app"));
    }
    unique_paths(markers)
}

#[cfg(target_os = "macos")]
fn claude_cli_command_candidates() -> Vec<PathBuf> {
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

#[cfg(all(not(windows), not(target_os = "macos")))]
fn claude_cli_command_candidates() -> Vec<PathBuf> {
    Vec::new()
}

fn command_version(command: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut process = Command::new(command);
    process.arg("--version");
    configure_detection_command(&mut process);

    command_output(process).or_else(|err| {
        #[cfg(target_os = "macos")]
        {
            if !is_path_like_command(command) {
                return command_version_from_macos_login_shell(command)
                    .map_err(|shell_err| format!("{err}; 登录 shell 检测也失败：{shell_err}"));
            }
        }

        Err(err)
    })
}

fn command_output(mut process: Command) -> Result<(Option<String>, Option<String>), String> {
    match process.output() {
        Ok(output) => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            let version = text.lines().next().map(|line| line.trim().to_string());
            if output.status.success() {
                Ok((version, None))
            } else {
                Ok((version, Some(format!("命令退出状态为 {}", output.status))))
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn command_version_from_macos_login_shell(
    command: &str,
) -> Result<(Option<String>, Option<String>), String> {
    for shell in macos_login_shell_candidates() {
        let mut process = Command::new(&shell);
        process.arg("-lc").arg(format!(
            "command -v {command} >/dev/null && {command} --version"
        ));
        configure_detection_command(&mut process);

        match command_output(process) {
            Ok((version, None)) => return Ok((version, None)),
            Ok((version, Some(error))) => {
                let detail = version
                    .map(|value| format!("{error}; 输出：{value}"))
                    .unwrap_or(error);
                return Ok((None, Some(format!("{} -lc: {detail}", shell.display()))));
            }
            Err(_) => continue,
        }
    }

    Err("没有可用的 zsh/bash 登录 shell".to_string())
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
fn windows_codex_app_command_candidates_from_base_dirs(base_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
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
fn windows_claude_cli_command_candidates_from_base_dirs(base_dirs: Vec<PathBuf>) -> Vec<PathBuf> {
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
fn windows_claude_app_marker_candidates_from_base_dirs(
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
fn windows_codex_app_marker_candidates_from_base_dirs(
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
            ]
        })
        .collect();

    markers.extend(
        start_menu_dirs
            .into_iter()
            .flat_map(|dir| [dir.join("Codex.lnk"), dir.join("OpenAI").join("Codex.lnk")]),
    );

    markers
}

#[cfg(windows)]
fn windows_codex_registry_install_dirs() -> Vec<PathBuf> {
    windows_tool_registry_install_dirs("Codex")
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

#[cfg(windows)]
fn configure_detection_command(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_detection_command(_command: &mut Command) {}

#[cfg(target_os = "macos")]
fn macos_common_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".bun").join("bin"));
        dirs.push(home.join(".npm-global").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
    }

    dirs
}

#[cfg(target_os = "macos")]
fn macos_login_shell_candidates() -> Vec<PathBuf> {
    let mut shells = Vec::new();
    if let Some(shell) = env::var_os("SHELL") {
        shells.push(PathBuf::from(shell));
    }
    shells.push(PathBuf::from("/bin/zsh"));
    shells.push(PathBuf::from("/bin/bash"));
    unique_paths(shells)
}

#[cfg(target_os = "macos")]
fn is_path_like_command(command: &str) -> bool {
    command.contains('/') || command.contains('\\')
}

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn unique_non_empty(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

#[cfg(test)]
#[path = "tests/tools.rs"]
mod tests;
