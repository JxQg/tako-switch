use super::*;

#[test]
fn external_url_validation_only_allows_http_and_https() {
    assert!(validate_external_url("https://tako.shiroha.tech/app/authorize").is_ok());
    assert!(validate_external_url("http://localhost:1420").is_ok());
    assert!(validate_external_url("takoswitch://v1/import?resource=auth").is_err());
    assert!(validate_external_url("not a url").is_err());
}

#[test]
fn auth_deeplink_parses_key_and_state() {
    let event =
        parse_auth_deeplink("takoswitch://v1/import?resource=auth&key=cr_abc123&state=s-xyz")
            .unwrap();

    assert_eq!(event.key, "cr_abc123");
    assert_eq!(event.state.as_deref(), Some("s-xyz"));
}

#[test]
fn auth_deeplink_rejects_non_auth_or_missing_key() {
    assert!(parse_auth_deeplink("takoswitch://v1/import?resource=provider&key=cr_x").is_none());
    assert!(parse_auth_deeplink("takoswitch://v1/import?resource=auth").is_none());
    assert!(parse_auth_deeplink("takoswitch://v1/import?resource=auth&key=").is_none());
    assert!(parse_auth_deeplink("https://tako.shiroha.tech/app/authorize").is_none());
}
