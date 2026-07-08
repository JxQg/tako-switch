use super::*;

#[test]
fn secret_masking_keeps_edges_only() {
    assert_eq!(mask_secret("sk-1234567890"), "sk-1****7890");
    assert_eq!(mask_secret("short"), "****");
}

#[test]
fn redacts_codex_inline_bearer_token() {
    let redacted = redact_plain_text("experimental_bearer_token = \"sk-1234567890\"".to_string());

    assert_eq!(redacted, "experimental_bearer_token= sk-1****7890");
}
