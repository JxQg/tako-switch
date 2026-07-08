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
