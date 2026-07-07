use crate::models::ToolStatus;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn detect_tools() -> Vec<ToolStatus> {
    vec![
        detect_tool("Codex", tool_command_candidates("codex")),
        detect_tool("Claude Code", tool_command_candidates("claude")),
    ]
}

fn tool_command_candidates(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{command}.cmd"),
            format!("{command}.exe"),
            command.to_string(),
        ]
    } else {
        vec![command.to_string()]
    }
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
                    .map(|value| format!("{error}; output: {value}"))
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
            "Tried {}. Last error: {}",
            commands.join(", "),
            errors
                .last()
                .cloned()
                .unwrap_or_else(|| "no candidates were available".to_string())
        )),
    }
}

fn command_version(command: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut process = Command::new(command);
    process.arg("--version");
    configure_detection_command(&mut process);

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
                Ok((
                    version,
                    Some(format!("Command exited with {}", output.status)),
                ))
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(windows)]
fn configure_detection_command(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_detection_command(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_tool_candidates_prefer_cmd_then_exe() {
        assert_eq!(
            tool_command_candidates("codex"),
            vec![
                "codex.cmd".to_string(),
                "codex.exe".to_string(),
                "codex".to_string(),
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_tool_candidates_use_plain_command() {
        assert_eq!(tool_command_candidates("codex"), vec!["codex".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn detect_tool_tries_later_windows_candidates() {
        let test_dir = std::env::temp_dir().join(format!(
            "tako-switch-detect-test-{}",
            chrono::Local::now().format("%Y%m%d%H%M%S%3f")
        ));
        std::fs::create_dir_all(&test_dir).unwrap();
        let command_path = test_dir.join("fake-tool.cmd");
        std::fs::write(&command_path, "@echo off\r\necho fake-tool 1.2.3\r\n").unwrap();

        let status = detect_tool(
            "Fake Tool",
            vec![
                test_dir
                    .join("missing-tool.cmd")
                    .to_string_lossy()
                    .to_string(),
                command_path.to_string_lossy().to_string(),
            ],
        );

        let _ = std::fs::remove_file(&command_path);
        let _ = std::fs::remove_dir(&test_dir);

        assert!(status.installed);
        assert_eq!(status.version, Some("fake-tool 1.2.3".to_string()));
        assert!(status.error.is_none());
    }
}
