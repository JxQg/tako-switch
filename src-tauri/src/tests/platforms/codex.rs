use super::*;
use crate::{
    providers::types::{CodexFeatureOptionsInput, PlatformOptionsInput, PLATFORM_CODEX},
    tests::{default_platform_writer, install_dir_test_lock, unique_temp_dir},
};
use std::{env, fs};

fn isolated_codex_home(name: &str) -> (std::sync::MutexGuard<'static, ()>, std::path::PathBuf) {
    let lock = install_dir_test_lock();
    let root = unique_temp_dir(name);
    fs::create_dir_all(&root).unwrap();
    env::set_var("CODEX_HOME", &root);
    env::set_var("TAKO_SWITCH_DATA_DIR", root.join("app-data"));
    (lock, root)
}

fn clear_isolated_env(root: &std::path::Path) {
    env::remove_var("CODEX_HOME");
    env::remove_var("TAKO_SWITCH_DATA_DIR");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn merge_preserves_existing_fields_and_is_idempotent() {
    let existing = r#"
approval_policy = "on-request"

[model_providers.other]
name = "Other"
"#;
    let writer = default_platform_writer(PLATFORM_CODEX);
    let first = merge_config(
        existing,
        "gpt-5.4",
        "http://127.0.0.1:3000/v1",
        "sk-test-123456",
        &writer,
        &Default::default(),
    )
    .unwrap();
    let second = merge_config(
        &first,
        "gpt-5.4",
        "http://127.0.0.1:3000/v1",
        "sk-test-123456",
        &writer,
        &Default::default(),
    )
    .unwrap();

    assert_eq!(first, second);
    assert!(first.contains("approval_policy = \"on-request\""));
    assert!(first.contains("model = \"gpt-5.4\""));
    assert!(first.contains("model_provider = \"tako_proxy\""));
    assert!(first.contains("wire_api = \"responses\""));
    assert!(first.contains("experimental_bearer_token = \"sk-test-123456\""));
    assert!(!first.contains("env_key"));
}

#[test]
fn merge_removes_conflicting_auth_fields() {
    let existing = r#"
[model_providers.tako_proxy]
name = "Old Tako"
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
env_key_instructions = "set env"
requires_openai_auth = true
wire_api = "responses"

[model_providers.tako_proxy.auth]
command = "fetch-token"
"#;
    let writer = default_platform_writer(PLATFORM_CODEX);
    let merged = merge_config(
        existing,
        "gpt-5.4",
        "https://tako.shiroha.tech/v1",
        "sk-new-123456",
        &writer,
        &Default::default(),
    )
    .unwrap();

    assert!(merged.contains("experimental_bearer_token = \"sk-new-123456\""));
    assert!(!merged.contains("env_key"));
    assert!(!merged.contains("env_key_instructions"));
    assert!(!merged.contains("requires_openai_auth"));
    assert!(!merged.contains("[model_providers.tako_proxy.auth]"));
}

#[test]
fn merge_applies_advanced_options_and_preserves_unselected_values() {
    let existing = r#"
sandbox_mode = "workspace-write"
approval_policy = "on-request"

[windows]
sandbox = "unelevated"

[features]
memories = false
shell_snapshot = false
multi_agent = true
"#;
    let writer = default_platform_writer(PLATFORM_CODEX);
    let options = PlatformOptionsInput {
        sandbox_mode: Some("danger-full-access".to_string()),
        approval_policy: Some("never".to_string()),
        windows_sandbox: Some("elevated".to_string()),
        features: CodexFeatureOptionsInput {
            js_repl: Some(false),
            unified_exec: Some(false),
            shell_snapshot: Some(true),
            memories: None,
        },
        ..Default::default()
    };
    let merged = merge_config(
        existing,
        "gpt-5.4",
        "https://tako.shiroha.tech/v1",
        "sk-new-123456",
        &writer,
        &options,
    )
    .unwrap();

    assert!(merged.contains("sandbox_mode = \"danger-full-access\""));
    assert!(merged.contains("approval_policy = \"never\""));
    assert!(merged.contains("[windows]"));
    assert!(merged.contains("sandbox = \"elevated\""));
    assert!(merged.contains("js_repl = false"));
    assert!(merged.contains("unified_exec = false"));
    assert!(merged.contains("shell_snapshot = true"));
    assert!(merged.contains("memories = false"));
    assert!(merged.contains("multi_agent = true"));
}

#[test]
fn merge_leaves_advanced_options_unchanged_when_unselected() {
    let existing = r#"
sandbox_mode = "workspace-write"

[windows]
sandbox = "unelevated"

[features]
shell_snapshot = false
"#;
    let writer = default_platform_writer(PLATFORM_CODEX);
    let merged = merge_config(
        existing,
        "gpt-5.4",
        "https://tako.shiroha.tech/v1",
        "sk-new-123456",
        &writer,
        &Default::default(),
    )
    .unwrap();

    assert!(merged.contains("sandbox_mode = \"workspace-write\""));
    assert!(merged.contains("sandbox = \"unelevated\""));
    assert!(merged.contains("shell_snapshot = false"));
}

#[test]
fn migration_converts_legacy_env_key_config_with_session_key() {
    let (_lock, root) = isolated_codex_home("codex-migration");
    let config_path = root.join("config.toml");
    fs::write(
        &config_path,
        r#"
model = "gpt-old"
model_provider = "tako_proxy"

[model_providers.tako_proxy]
name = "Tako LLM Gateway"
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
requires_openai_auth = true
wire_api = "responses"
"#,
    )
    .unwrap();

    let writer = default_platform_writer(PLATFORM_CODEX);
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let migrated = migrate_legacy_config(
        &writer,
        Some("gpt-5.4"),
        "https://tako.shiroha.tech/v1",
        Some("sk-session-123456"),
        &mut files,
        &mut warnings,
    )
    .unwrap();
    let after = fs::read_to_string(&config_path).unwrap();

    assert!(migrated);
    assert_eq!(files.len(), 1);
    assert!(after.contains("model = \"gpt-old\""));
    assert!(after.contains("base_url = \"https://old.example.com/v1\""));
    assert!(after.contains("experimental_bearer_token = \"sk-session-123456\""));
    assert!(!after.contains("env_key"));
    assert!(!after.contains("requires_openai_auth"));

    let mut second_files = Vec::new();
    let second = migrate_legacy_config(
        &writer,
        Some("gpt-5.4"),
        "https://tako.shiroha.tech/v1",
        Some("sk-session-123456"),
        &mut second_files,
        &mut warnings,
    )
    .unwrap();
    assert!(!second);
    assert!(second_files.is_empty());

    clear_isolated_env(&root);
}

#[test]
fn migration_skips_without_key_source() {
    let (_lock, root) = isolated_codex_home("codex-migration-no-key");
    let config_path = root.join("config.toml");
    let before = r#"
[model_providers.tako_proxy]
base_url = "https://old.example.com/v1"
env_key = "TAKO_CODEX_API_KEY"
"#;
    fs::write(&config_path, before).unwrap();

    let writer = default_platform_writer(PLATFORM_CODEX);
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let migrated = migrate_legacy_config(
        &writer,
        Some("gpt-5.4"),
        "https://tako.shiroha.tech/v1",
        None,
        &mut files,
        &mut warnings,
    )
    .unwrap();

    assert!(!migrated);
    assert!(files.is_empty());
    assert_eq!(fs::read_to_string(&config_path).unwrap(), before);

    clear_isolated_env(&root);
}
