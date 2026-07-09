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

#[cfg(all(not(windows), not(target_os = "macos")))]
#[test]
fn non_windows_tool_candidates_use_plain_command() {
    assert_eq!(tool_command_candidates("codex"), vec!["codex".to_string()]);
}

#[cfg(windows)]
#[test]
fn windows_codex_candidates_include_desktop_app_cli() {
    let base_dir = PathBuf::from("C:\\Users\\demo\\AppData\\Local");

    let candidates = windows_codex_app_command_candidates_from_base_dirs(vec![base_dir]);

    assert!(candidates.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex\\bin\\codex.exe"
    )));
}

#[cfg(windows)]
#[test]
fn windows_claude_candidates_include_native_and_winget_cli() {
    let user_dir = PathBuf::from("C:\\Users\\demo");
    let local_app_data = PathBuf::from("C:\\Users\\demo\\AppData\\Local");

    let candidates =
        windows_claude_cli_command_candidates_from_base_dirs(vec![user_dir, local_app_data]);

    assert!(candidates.contains(&PathBuf::from("C:\\Users\\demo\\.local\\bin\\claude.exe")));
    assert!(candidates.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Microsoft\\WinGet\\Links\\claude.exe"
    )));
    assert!(candidates.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe\\claude.exe"
    )));
}

#[test]
fn claude_code_version_validator_requires_claude_code_output() {
    assert!(is_claude_code_version_output(Some("2.1.131 (Claude Code)")));
    assert!(!is_claude_code_version_output(Some("Claude 1.0.0")));
    assert!(!is_claude_code_version_output(None));
}

#[cfg(windows)]
#[test]
fn windows_claude_app_markers_include_desktop_app_locations() {
    let local_app_data = PathBuf::from("C:\\Users\\demo\\AppData\\Local");
    let start_menu_dir = PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs",
    );

    let markers = windows_claude_app_marker_candidates_from_base_dirs(
        vec![local_app_data],
        vec![start_menu_dir],
    );

    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Programs\\Claude\\Claude.exe"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Microsoft\\WindowsApps\\Claude.exe"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Claude.lnk"
    )));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn claude_app_marker_status_reports_installed_without_command() {
    let test_dir = crate::tests::unique_temp_dir("claude-app-marker");
    std::fs::create_dir_all(&test_dir).unwrap();

    let status = detect_claude_app_status_from_markers(vec![test_dir.clone()]).unwrap();

    let _ = std::fs::remove_dir(&test_dir);

    assert!(status.installed);
    assert_eq!(status.name, "Claude Code");
    assert_eq!(
        status.version,
        Some("Claude Desktop 已安装（未检测到 claude 命令）".to_string())
    );
    assert!(status.error.is_none());
}

#[cfg(windows)]
#[test]
fn windows_codex_markers_include_app_directory_and_start_menu_shortcut() {
    let base_dir = PathBuf::from("C:\\Users\\demo\\AppData\\Local");
    let start_menu_dir = PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs",
    );

    let markers =
        windows_codex_app_marker_candidates_from_base_dirs(vec![base_dir], vec![start_menu_dir]);

    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Codex.lnk"
    )));
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn codex_app_marker_status_reports_installed_without_command() {
    let test_dir = crate::tests::unique_temp_dir("codex-app-marker");
    std::fs::create_dir_all(&test_dir).unwrap();

    let status = detect_codex_app_status_from_markers(vec![test_dir.clone()]).unwrap();

    let _ = std::fs::remove_dir(&test_dir);

    assert!(status.installed);
    assert_eq!(status.name, "Codex");
    assert_eq!(
        status.version,
        Some("Codex App 已安装（未检测到 codex 命令）".to_string())
    );
    assert!(status.error.is_none());
}

#[cfg(windows)]
#[test]
fn detect_tool_tries_later_windows_candidates() {
    let test_dir = crate::tests::unique_temp_dir("detect-test");
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
