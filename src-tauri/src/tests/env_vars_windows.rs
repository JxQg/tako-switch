use super::*;

#[test]
fn cleanup_missing_legacy_variable_is_ok() {
    cleanup_legacy_codex_api_key().unwrap();
}
