use crate::models::ToolStatus;
mod app_detection;

use app_detection::{
    claude_app_install_markers, claude_app_supported, claude_cli_command_candidates,
    codex_app_command_candidates, codex_app_install_markers, codex_app_supported,
    detect_app_from_markers,
};
use std::collections::BTreeSet;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const DETECTED_BY_CLI: &str = "cli";
const DETECTED_BY_APP: &str = "app";
const DETECTED_BY_NONE: &str = "none";

#[derive(Debug)]
struct CommandDetection {
    installed: bool,
    version: Option<String>,
    error: Option<String>,
    path: Option<String>,
}

pub fn detect_tools() -> Vec<ToolStatus> {
    vec![detect_codex_tool(), detect_claude_tool()]
}

pub fn open_tool_app(tool: &str) -> Result<(), String> {
    app_detection::open_tool_app(tool)
}

fn detect_codex_tool() -> ToolStatus {
    let cli = detect_command_tool("Codex", codex_command_candidates(), |_| true);
    let app_supported = codex_app_supported();
    let app_path = app_supported
        .then(|| detect_app_from_markers(codex_app_install_markers()))
        .flatten();

    merge_tool_status(
        "Codex",
        "codex",
        "Codex 桌面端",
        cli,
        app_path,
        app_supported,
    )
}

fn detect_claude_tool() -> ToolStatus {
    let cli = detect_command_tool(
        "Claude Code",
        claude_command_candidates(),
        is_claude_code_version_output,
    );
    let app_supported = claude_app_supported();
    let app_path = app_supported
        .then(|| detect_app_from_markers(claude_app_install_markers()))
        .flatten();

    merge_tool_status(
        "Claude Code",
        "claude",
        "Claude Code 桌面端",
        cli,
        app_path,
        app_supported,
    )
}

fn merge_tool_status(
    name: &str,
    command_name: &str,
    app_label: &str,
    cli: CommandDetection,
    app_path: Option<std::path::PathBuf>,
    app_supported: bool,
) -> ToolStatus {
    let app_installed = app_path.is_some();
    let installed = cli.installed || app_installed;
    let detected_by = if cli.installed {
        DETECTED_BY_CLI
    } else if app_installed {
        DETECTED_BY_APP
    } else {
        DETECTED_BY_NONE
    };
    let detail = if cli.installed {
        cli.version
            .clone()
            .unwrap_or_else(|| format!("{name} CLI 已安装"))
    } else if app_installed {
        format!("{app_label}已安装，CLI 未检测到")
    } else if app_supported {
        format!("未检测到 {app_label}或 {command_name} 命令")
    } else {
        format!("未检测到 {command_name} 命令")
    };

    ToolStatus {
        name: name.to_string(),
        installed,
        version: cli.version,
        error: cli.error,
        detail: Some(detail),
        cli_installed: Some(cli.installed),
        app_installed: Some(app_installed),
        app_supported: Some(app_supported),
        detected_by: Some(detected_by.to_string()),
        cli_path: cli.path,
        app_path: app_path.map(|path| path.to_string_lossy().to_string()),
    }
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

    #[cfg(not(windows))]
    {
        unix_tool_command_candidates(command)
    }
}

#[cfg(not(windows))]
fn unix_tool_command_candidates(command: &str) -> Vec<String> {
    let mut candidates = vec![command.to_string()];
    candidates.extend(
        unix_common_bin_dirs()
            .into_iter()
            .map(|dir| dir.join(command).to_string_lossy().to_string()),
    );

    unique_non_empty(candidates)
}

#[cfg(test)]
fn detect_tool(name: &str, commands: Vec<String>) -> ToolStatus {
    detect_tool_with_version_validator(name, commands, |_| true)
}

#[cfg(test)]
fn detect_tool_with_version_validator<F>(
    name: &str,
    commands: Vec<String>,
    is_valid_version: F,
) -> ToolStatus
where
    F: Fn(Option<&str>) -> bool,
{
    let cli = detect_command_tool(name, commands, is_valid_version);
    merge_tool_status(name, name, &format!("{name} 桌面端"), cli, None, false)
}

fn detect_command_tool<F>(
    name: &str,
    commands: Vec<String>,
    is_valid_version: F,
) -> CommandDetection
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

                return CommandDetection {
                    installed: true,
                    version,
                    error: None,
                    path: Some(command.clone()),
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

    CommandDetection {
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
        path: None,
    }
}

fn is_claude_code_version_output(version: Option<&str>) -> bool {
    version
        .map(|value| value.to_lowercase().contains("claude code"))
        .unwrap_or(false)
}

#[cfg(test)]
fn detect_claude_app_status_from_markers(markers: Vec<std::path::PathBuf>) -> Option<ToolStatus> {
    detect_app_status_from_markers(
        "Claude Code",
        "claude",
        "Claude Code 桌面端",
        claude_app_supported(),
        markers,
    )
}

#[cfg(test)]
fn detect_codex_app_status_from_markers(markers: Vec<std::path::PathBuf>) -> Option<ToolStatus> {
    detect_app_status_from_markers(
        "Codex",
        "codex",
        "Codex 桌面端",
        codex_app_supported(),
        markers,
    )
}

#[cfg(test)]
fn detect_app_status_from_markers(
    name: &str,
    command_name: &str,
    app_label: &str,
    app_supported: bool,
    markers: Vec<std::path::PathBuf>,
) -> Option<ToolStatus> {
    if let Some(path) = detect_app_from_markers(markers) {
        Some(merge_tool_status(
            name,
            command_name,
            app_label,
            CommandDetection {
                installed: false,
                version: None,
                error: None,
                path: None,
            },
            Some(path),
            app_supported,
        ))
    } else {
        None
    }
}

fn command_version(command: &str) -> Result<(Option<String>, Option<String>), String> {
    if is_path_like_command(command) && !Path::new(command).exists() {
        return Err("路径不存在".to_string());
    }

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
fn configure_detection_command(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_detection_command(_command: &mut Command) {}

#[cfg(not(windows))]
fn unix_common_bin_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
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

fn is_path_like_command(command: &str) -> bool {
    command.contains('/') || command.contains('\\')
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
