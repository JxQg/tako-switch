use crate::models::ToolStatus;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::{collections::BTreeSet, env, path::PathBuf};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn detect_tools() -> Vec<ToolStatus> {
    vec![
        detect_tool("Codex", tool_command_candidates("codex")),
        detect_tool("Claude Code", tool_command_candidates("claude")),
    ]
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
    let mut errors = Vec::new();

    for command in &commands {
        match command_version(command) {
            Ok((version, None)) => {
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

#[cfg(target_os = "macos")]
fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(target_os = "macos")]
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
