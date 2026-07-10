#[cfg(windows)]
use super::app_detection::{
    codex_app_install_marker_candidates, codex_app_install_markers,
    is_allowed_store_app_id,
    windows_claude_app_launch_targets_from_base_dirs,
    windows_claude_app_marker_candidates_from_base_dirs,
    windows_claude_cli_command_candidates_from_base_dirs,
    windows_codex_app_command_candidates_from_base_dirs,
    windows_codex_app_launch_targets_from_base_dirs,
    windows_codex_app_marker_candidates_from_base_dirs, AppInstallMarker, WindowsAppLaunchTarget,
    WindowsStoreApp,
};
use super::*;
#[cfg(windows)]
use std::path::PathBuf;

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

#[test]
fn missing_absolute_command_reports_clean_diagnostic() {
    let missing = crate::tests::unique_temp_dir("missing-command").join("missing-tool");

    let status = detect_tool("Missing Tool", vec![missing.to_string_lossy().to_string()]);

    assert!(!status.installed);
    assert_eq!(
        status.detail,
        Some("未检测到 Missing Tool 命令".to_string())
    );
    assert!(status.error.unwrap().contains("路径不存在"));
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
    assert!(!markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Microsoft\\WindowsApps\\Claude.exe"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Claude.lnk"
    )));
}

#[cfg(windows)]
#[test]
fn windows_claude_launch_targets_use_shared_desktop_app_shape() {
    let local_app_data = PathBuf::from("C:\\Users\\demo\\AppData\\Local");
    let start_menu_dir = PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs",
    );

    let targets =
        windows_claude_app_launch_targets_from_base_dirs(vec![local_app_data], vec![start_menu_dir]);

    assert!(targets.contains(&WindowsAppLaunchTarget::Path(PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Programs\\Claude\\Claude.exe"
    ))));
    assert!(!targets.contains(&WindowsAppLaunchTarget::Path(PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Microsoft\\WindowsApps\\Claude.exe"
    ))));
    assert!(targets.contains(&WindowsAppLaunchTarget::Path(PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Claude.lnk"
    ))));
}

#[cfg(windows)]
#[test]
fn invalid_store_app_text_is_not_allowed_as_app_id() {
    assert!(!is_allowed_store_app_id(
        "What would you like to know about Anthropic or Claude? I can help with:",
        &["OpenAI.Codex"],
    ));
    assert!(!is_allowed_store_app_id(
        "shell:AppsFolder\\What would you like to know about Anthropic or Claude?",
        &["OpenAI.Codex"],
    ));
    assert!(is_allowed_store_app_id(
        "OpenAI.Codex_2p2nqsd0c76g0!App",
        &["OpenAI.Codex"],
    ));
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
    assert_eq!(status.version, None);
    assert_eq!(
        status.detail,
        Some("Claude Code 桌面端已安装，CLI 未检测到".to_string())
    );
    assert_eq!(status.cli_installed, Some(false));
    assert_eq!(status.app_installed, Some(true));
    assert_eq!(status.detected_by, Some("app".to_string()));
    assert_eq!(
        status.app_path,
        Some(test_dir.to_string_lossy().to_string())
    );
    assert_eq!(status.cli_path, None);
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

    assert!(!markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex"
    )));
    assert!(!markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex\\bin\\codex.exe"
    )));
    assert!(markers.contains(&PathBuf::from("C:\\Users\\demo\\AppData\\Local\\ChatGPT")));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Programs\\ChatGPT\\ChatGPT.exe"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Codex.lnk"
    )));
    assert!(markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\ChatGPT.lnk"
    )));
}

#[cfg(windows)]
#[test]
fn windows_codex_cli_path_is_not_a_desktop_app_marker() {
    let base_dir = PathBuf::from("C:\\Users\\demo\\AppData\\Local");

    let markers = codex_app_install_markers();
    let synthetic_markers =
        windows_codex_app_marker_candidates_from_base_dirs(vec![base_dir], Vec::new());

    assert!(!synthetic_markers.contains(&PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex\\bin\\codex.exe"
    )));
    assert!(!markers.iter().any(|path| path.ends_with("bin\\codex.exe")));
}

#[cfg(windows)]
#[test]
fn windows_codex_app_id_marker_reports_apps_folder_target() {
    let app_id = "OpenAI.Codex_2p2nqsd0c76g0!App".to_string();

    let detected = detect_app_from_markers(vec![AppInstallMarker::WindowsStoreApp(
        WindowsStoreApp {
            app_id: app_id.clone(),
            install_location: None,
        },
    )])
    .unwrap();

    assert_eq!(
        detected,
        "shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App"
    );
}

#[cfg(windows)]
#[test]
fn windows_codex_store_marker_reports_install_location_when_available() {
    let detected = detect_app_from_markers(vec![AppInstallMarker::WindowsStoreApp(
        WindowsStoreApp {
            app_id: "OpenAI.Codex_2p2nqsd0c76g0!App".to_string(),
            install_location: Some(PathBuf::from(
                "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0",
            )),
        },
    )])
    .unwrap();

    assert_eq!(
        detected,
        "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.707.3748.0_x64__2p2nqsd0c76g0"
    );
}

#[cfg(windows)]
#[test]
fn windows_codex_launch_targets_keep_app_id_before_paths() {
    let app_id = "OpenAI.Codex_2p2nqsd0c76g0!App".to_string();
    let local_app_data = PathBuf::from("C:\\Users\\demo\\AppData\\Local");

    let mut targets = vec![WindowsAppLaunchTarget::AppId(app_id.clone())];
    targets.extend(windows_codex_app_launch_targets_from_base_dirs(
        vec![local_app_data],
        Vec::new(),
    ));

    assert_eq!(targets.first(), Some(&WindowsAppLaunchTarget::AppId(app_id)));
    assert!(targets.contains(&WindowsAppLaunchTarget::Path(PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\Programs\\ChatGPT\\ChatGPT.exe"
    ))));
    assert!(!targets.contains(&WindowsAppLaunchTarget::Path(PathBuf::from(
        "C:\\Users\\demo\\AppData\\Local\\OpenAI\\Codex\\bin\\codex.exe"
    ))));
}

#[cfg(windows)]
#[test]
fn windows_live_codex_app_marker_prefers_store_app_when_present() {
    let markers = codex_app_install_marker_candidates();

    if markers
        .iter()
        .any(|marker| matches!(marker, AppInstallMarker::WindowsStoreApp(app) if app.app_id.contains("OpenAI.Codex")))
    {
        assert!(matches!(
            markers.first(),
            Some(AppInstallMarker::WindowsStoreApp(app)) if app.app_id.contains("OpenAI.Codex")
        ));
    }
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
    assert_eq!(status.version, None);
    assert_eq!(
        status.detail,
        Some("Codex 桌面端已安装，CLI 未检测到".to_string())
    );
    assert_eq!(status.cli_installed, Some(false));
    assert_eq!(status.app_installed, Some(true));
    assert_eq!(status.detected_by, Some("app".to_string()));
    assert_eq!(
        status.app_path,
        Some(test_dir.to_string_lossy().to_string())
    );
    assert_eq!(status.cli_path, None);
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
    assert_eq!(status.detail, Some("fake-tool 1.2.3".to_string()));
    assert_eq!(status.cli_installed, Some(true));
    assert_eq!(status.detected_by, Some("cli".to_string()));
    assert_eq!(
        status.cli_path,
        Some(command_path.to_string_lossy().to_string())
    );
    assert_eq!(status.app_path, None);
    assert!(status.error.is_none());
}
