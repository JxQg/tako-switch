use super::*;
use crate::{
    providers::types::{PlatformOptionsInput, PLATFORM_CLAUDE},
    tests::default_platform_writer,
};
use serde_json::Value;

#[test]
fn merge_preserves_existing_settings_and_env() {
    let existing = r#"{
  "theme": "dark",
  "env": {
    "OTHER": "1"
  }
}"#;
    let writer = default_platform_writer(PLATFORM_CLAUDE);
    let merged = merge_settings(
        existing,
        "https://gateway.example.com/v1",
        "sk-test-secret",
        Some("claude-custom"),
        &writer,
        &Default::default(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&merged).unwrap();

    assert_eq!(value["theme"], "dark");
    assert_eq!(value["env"]["OTHER"], "1");
    assert_eq!(
        value["env"]["ANTHROPIC_BASE_URL"],
        "https://gateway.example.com/v1"
    );
    assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-test-secret");
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-custom");
}

#[test]
fn merge_removes_empty_model() {
    let existing = r#"{"env":{"ANTHROPIC_MODEL":"old"}}"#;
    let writer = default_platform_writer(PLATFORM_CLAUDE);
    let merged = merge_settings(
        existing,
        "http://localhost:3000/v1",
        "sk-test",
        None,
        &writer,
        &Default::default(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&merged).unwrap();

    assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
}

#[test]
fn merge_applies_permissions_options_and_preserves_existing_fields() {
    let existing = r#"{
  "theme": "dark",
  "permissions": {
    "allow": ["Bash(git status)"],
    "defaultMode": "default"
  }
}"#;
    let writer = default_platform_writer(PLATFORM_CLAUDE);
    let options = PlatformOptionsInput {
        permissions_default_mode: Some("bypassPermissions".to_string()),
        skip_dangerous_mode_permission_prompt: Some(true),
        ..Default::default()
    };
    let merged = merge_settings(
        existing,
        "https://gateway.example.com",
        "sk-test-secret",
        None,
        &writer,
        &options,
    )
    .unwrap();
    let value: Value = serde_json::from_str(&merged).unwrap();

    assert_eq!(value["theme"], "dark");
    assert_eq!(value["permissions"]["allow"][0], "Bash(git status)");
    assert_eq!(value["permissions"]["defaultMode"], "bypassPermissions");
    assert_eq!(value["skipDangerousModePermissionPrompt"], true);
}

#[test]
fn merge_leaves_permissions_options_unchanged_when_unselected() {
    let existing = r#"{
  "permissions": {
    "defaultMode": "plan"
  },
  "skipDangerousModePermissionPrompt": false
}"#;
    let writer = default_platform_writer(PLATFORM_CLAUDE);
    let merged = merge_settings(
        existing,
        "https://gateway.example.com",
        "sk-test-secret",
        None,
        &writer,
        &Default::default(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&merged).unwrap();

    assert_eq!(value["permissions"]["defaultMode"], "plan");
    assert_eq!(value["skipDangerousModePermissionPrompt"], false);
}
