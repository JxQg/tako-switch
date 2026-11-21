use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use tako_switch_lib::auth::{
    build_oauth_authorize_url, exchange_desktop_token, fetch_account_status, logout,
    DESKTOP_OAUTH_CLIENT_ID,
};
use tako_switch_lib::config::{
    build_desktop_device_name, build_desktop_device_name_with_host, read_main_api_key,
    read_or_create_device_id, read_switch_state, sanitize_host_label, save_switch_state,
    AppPaths,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
struct AuthCodeEntry {
    user_id: String,
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    device_name: String,
}

#[derive(Debug, Clone)]
struct IssuedToken {
    user_id: String,
    key_id: String,
    token_name: String,
    key: String,
}

#[derive(Default)]
struct GatewayState {
    next_seq: u64,
    auth_codes: HashMap<String, AuthCodeEntry>,
    active_tokens: Vec<IssuedToken>,
    revoked_keys: Vec<String>,
    deleted_for_replacement: Vec<(String, String)>,
    authorize_calls: Vec<HashMap<String, String>>,
    token_calls: Vec<Value>,
    revoke_calls: Vec<String>,
}

struct MockResponse {
    status: u16,
    headers: Vec<(String, String)>,
    content_type: String,
    body: Vec<u8>,
}

impl MockResponse {
    fn json(status: u16, val: &Value) -> Self {
        Self {
            status,
            headers: Vec::new(),
            content_type: "application/json".to_string(),
            body: serde_json::to_vec(val).expect("serialize mock json"),
        }
    }

    fn redirect(location: &str, body_val: &Value) -> Self {
        Self {
            status: 302,
            headers: vec![("Location".to_string(), location.to_string())],
            content_type: "application/json".to_string(),
            body: serde_json::to_vec(body_val).expect("serialize redirect body"),
        }
    }
}

struct MockTakoGateway {
    base_url: String,
    state: Arc<Mutex<GatewayState>>,
    handle: JoinHandle<()>,
}

impl Drop for MockTakoGateway {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct ParsedHttpRequest {
    method: String,
    path: String,
    query: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn url_encode_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    out
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        }
    }
    map
}

fn pkce_s256_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<ParsedHttpRequest> {
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 2048];
    let header_end;
    loop {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }

    let header_text = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next()?;
    let mut req_parts = request_line.split_whitespace();
    let method = req_parts.next()?.to_string();
    let raw_target = req_parts.next()?.to_string();
    let (path, query) = match raw_target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw_target, String::new()),
    };

    let mut content_length: usize = 0;
    let mut authorization: Option<String> = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim();
            if key == "content-length" {
                content_length = val.parse::<usize>().unwrap_or(0);
            } else if key == "authorization" {
                authorization = Some(val.to_string());
            }
        }
    }

    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);

    Some(ParsedHttpRequest {
        method,
        path,
        query,
        authorization,
        body,
    })
}

async fn write_http_response(stream: &mut tokio::net::TcpStream, resp: MockResponse) {
    let reason = match resp.status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp.status,
        reason,
        resp.content_type,
        resp.body.len()
    );
    for (k, v) in resp.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&resp.body).await;
    let _ = stream.shutdown().await;
}

fn extract_bearer(auth: &Option<String>) -> Option<String> {
    let raw = auth.as_deref()?.trim();
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn handle_gateway_request(req: ParsedHttpRequest, state: &Arc<Mutex<GatewayState>>) -> MockResponse {
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/app/authorize") => {
            let params = parse_query_params(&req.query);
            let mut st = state.lock().expect("lock gateway state");
            st.authorize_calls.push(params.clone());

            let client_id = params.get("client_id").cloned().unwrap_or_default();
            let response_type = params.get("response_type").cloned().unwrap_or_default();
            let redirect_uri = params.get("redirect_uri").cloned().unwrap_or_default();
            let oauth_state = params.get("state").cloned().unwrap_or_default();
            let code_challenge = params.get("code_challenge").cloned().unwrap_or_default();
            let method = params
                .get("code_challenge_method")
                .cloned()
                .unwrap_or_default();
            let device_name = params.get("device_name").cloned().unwrap_or_default();

            if client_id != "takoswitch"
                || response_type != "code"
                || method != "S256"
                || redirect_uri.is_empty()
                || code_challenge.is_empty()
                || device_name.is_empty()
            {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "invalid authorize params"
                    }),
                );
            }

            st.next_seq += 1;
            let code = format!("auth_code_{}", st.next_seq);
            st.auth_codes.insert(
                code.clone(),
                AuthCodeEntry {
                    user_id: "user_shiroha".to_string(),
                    client_id,
                    redirect_uri: redirect_uri.clone(),
                    code_challenge,
                    device_name,
                },
            );

            let location = format!(
                "{}?code={}&state={}",
                redirect_uri,
                url_encode_component(&code),
                url_encode_component(&oauth_state)
            );
            MockResponse::redirect(&location, &json!({ "code": code, "state": oauth_state }))
        }
        ("POST", "/api/desktop/token") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let mut st = state.lock().expect("lock gateway state");
            st.token_calls.push(body.clone());

            let grant_type = body["grant_type"].as_str().unwrap_or("");
            let code = body["code"].as_str().unwrap_or("");
            let code_verifier = body["code_verifier"].as_str().unwrap_or("");
            let redirect_uri = body["redirect_uri"].as_str().unwrap_or("");
            let client_id = body["client_id"].as_str().unwrap_or("");

            if grant_type != "authorization_code" || client_id != "takoswitch" {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "unsupported grant_type or client_id"
                    }),
                );
            }

            let Some(entry) = st.auth_codes.remove(code) else {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "invalid or consumed authorization code"
                    }),
                );
            };

            if entry.client_id != client_id || entry.redirect_uri != redirect_uri {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "client_id or redirect_uri mismatch"
                    }),
                );
            }

            if pkce_s256_challenge(code_verifier) != entry.code_challenge {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "PKCE S256 verification failed"
                    }),
                );
            }

            let device_name = if !entry.device_name.is_empty() {
                entry.device_name
            } else {
                body["device_name"].as_str().unwrap_or("").to_string()
            };
            if device_name.is_empty() {
                return MockResponse::json(
                    400,
                    &json!({
                        "success": false,
                        "message": "missing device_name"
                    }),
                );
            }

            // 1:1 mirror of tako-src behavior: build `{client_id}@{device_name}` and delete
            // all previously issued keys for the same (user_id, token_name).
            let token_name = format!("{client_id}@{device_name}");
            let mut deleted = Vec::new();
            st.active_tokens.retain(|tok| {
                let same_slot = tok.user_id == entry.user_id && tok.token_name == token_name;
                if same_slot {
                    deleted.push((tok.token_name.clone(), tok.key.clone()));
                }
                !same_slot
            });
            st.deleted_for_replacement.extend(deleted);

            st.next_seq += 1;
            let seq = st.next_seq;
            let key = format!("cr_desktop_secret_{seq:04}");
            let key_id = format!("kid_{seq:04}");
            st.active_tokens.push(IssuedToken {
                user_id: entry.user_id,
                key_id,
                token_name: token_name.clone(),
                key: key.clone(),
            });

            MockResponse::json(
                200,
                &json!({
                    "success": true,
                    "data": {
                        "key": key,
                        "tokenName": token_name
                    }
                }),
            )
        }
        ("GET", "/api/desktop/me") => {
            let Some(bearer) = extract_bearer(&req.authorization) else {
                return MockResponse::json(
                    401,
                    &json!({ "success": false, "error": "missing bearer token" }),
                );
            };
            let st = state.lock().expect("lock gateway state");
            if let Some(tok) = st.active_tokens.iter().find(|t| t.key == bearer) {
                MockResponse::json(
                    200,
                    &json!({
                        "success": true,
                        "data": {
                            "user": { "id": tok.user_id, "name": "Shiroha" },
                            "plan": { "name": "Pro x4" },
                            "quota": { "remain": 5000000, "used": 1000000 }
                        }
                    }),
                )
            } else {
                MockResponse::json(
                    401,
                    &json!({ "success": false, "error": "token revoked or replaced" }),
                )
            }
        }
        ("POST", "/api/desktop/revoke") => {
            let Some(bearer) = extract_bearer(&req.authorization) else {
                return MockResponse::json(
                    401,
                    &json!({ "success": false, "error": "missing bearer token" }),
                );
            };
            let mut st = state.lock().expect("lock gateway state");
            st.revoke_calls.push(bearer.clone());
            let before = st.active_tokens.len();
            st.active_tokens.retain(|t| t.key != bearer);
            if st.active_tokens.len() < before {
                st.revoked_keys.push(bearer);
            }
            MockResponse::json(200, &json!({ "success": true }))
        }
        ("POST", "/apiStats/api/verify-identity") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let api_key = body["apiKey"].as_str().unwrap_or("");
            let st = state.lock().expect("lock gateway state");
            if st.active_tokens.iter().any(|t| t.key == api_key) {
                MockResponse::json(
                    200,
                    &json!({
                        "success": true,
                        "user": {
                            "name": "Shiroha",
                            "plan": { "name": "Pro x4" }
                        }
                    }),
                )
            } else {
                MockResponse::json(
                    401,
                    &json!({
                        "success": false,
                        "error": "API Key 已被吊销或不存在"
                    }),
                )
            }
        }
        ("POST", "/apiStats/api/get-key-id") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let api_key = body["apiKey"].as_str().unwrap_or("");
            let st = state.lock().expect("lock gateway state");
            if let Some(tok) = st.active_tokens.iter().find(|t| t.key == api_key) {
                MockResponse::json(
                    200,
                    &json!({
                        "success": true,
                        "data": { "id": tok.key_id }
                    }),
                )
            } else {
                MockResponse::json(
                    404,
                    &json!({ "success": false, "error": "key not found" }),
                )
            }
        }
        ("POST", "/apiStats/api/user-quota") => {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let api_id = body["apiId"].as_str().unwrap_or("");
            let st = state.lock().expect("lock gateway state");
            if st.active_tokens.iter().any(|t| t.key_id == api_id) {
                MockResponse::json(
                    200,
                    &json!({
                        "plan": {
                            "window_cost_limit": 10.0,
                            "window_minutes": 300.0,
                            "daily_cost_limit": 0.0,
                            "weekly_cost_limit": 50.0
                        },
                        "usage": {
                            "windowCost": 1.5,
                            "dailyCost": 0.0,
                            "weeklyCost": 5.0
                        }
                    }),
                )
            } else {
                MockResponse::json(
                    404,
                    &json!({ "success": false, "error": "apiId not found" }),
                )
            }
        }
        _ => MockResponse::json(404, &json!({ "error": "not found" })),
    }
}

async fn start_mock_gateway() -> MockTakoGateway {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock gateway");
    let addr = listener.local_addr().expect("gateway local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());
    let state = Arc::new(Mutex::new(GatewayState::default()));
    let state_clone = Arc::clone(&state);

    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&state_clone);
            tokio::spawn(async move {
                if let Some(req) = read_http_request(&mut stream).await {
                    let resp = handle_gateway_request(req, &st);
                    write_http_response(&mut stream, resp).await;
                }
            });
        }
    });

    MockTakoGateway {
        base_url,
        state,
        handle,
    }
}

fn setup_sandbox(base_url: &str) -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create tempdir");
    let paths = AppPaths::for_test(tmp.path());
    let mut state = read_switch_state(&paths);
    state.base_url = base_url.to_string();
    save_switch_state(&paths, &state).expect("save switch state");
    (tmp, paths)
}

async fn perform_oauth_login_with_host(
    gateway: &MockTakoGateway,
    paths: &AppPaths,
    raw_host: Option<&str>,
    tag: &str,
) -> (String, String) {
    let device_name = build_desktop_device_name_with_host(paths, raw_host);
    let redirect_uri = "http://127.0.0.1:49999/tako/callback";
    let oauth_state = format!("state_{tag}");
    let code_verifier = format!("verifier_{tag}_0123456789_abcdefghijklmnopqrstuvwxyz");
    let code_challenge = pkce_s256_challenge(&code_verifier);

    let auth_url = build_oauth_authorize_url(
        &gateway.base_url,
        redirect_uri,
        &oauth_state,
        &code_challenge,
        &device_name,
    );

    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("build no-redirect client");
    let auth_resp = client
        .get(&auth_url)
        .send()
        .await
        .expect("GET /app/authorize");
    assert_eq!(
        auth_resp.status().as_u16(),
        302,
        "expected 302 redirect from /app/authorize"
    );

    let location = auth_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header on /app/authorize redirect");
    let query = location
        .split_once('?')
        .map(|(_, q)| q)
        .expect("query in redirect Location");
    let params = parse_query_params(query);
    assert_eq!(
        params.get("state").map(String::as_str),
        Some(oauth_state.as_str())
    );
    let code = params
        .get("code")
        .cloned()
        .expect("authorization code in redirect Location");

    exchange_desktop_token(paths, &gateway.base_url, &code, &code_verifier, redirect_uri)
        .await
        .expect("exchange_desktop_token should succeed");

    let key = read_main_api_key(paths).expect("oauth login must persist api key");
    (device_name, key)
}

fn is_valid_device_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '+' | '-')
}

#[tokio::test]
async fn tc_md_01_same_hostname_two_machines_concurrent_no_eviction() {
    let gateway = start_mock_gateway().await;
    let (_tmp_a, paths_a) = setup_sandbox(&gateway.base_url);
    let (_tmp_b, paths_b) = setup_sandbox(&gateway.base_url);

    let (device_name_a, key_a) =
        perform_oauth_login_with_host(&gateway, &paths_a, Some("MacBook-Pro.local"), "a1").await;
    let (device_name_b, key_b) =
        perform_oauth_login_with_host(&gateway, &paths_b, Some("MacBook-Pro.local"), "b1").await;

    let id_a = read_or_create_device_id(&paths_a);
    let id_b = read_or_create_device_id(&paths_b);
    assert_eq!(id_a.len(), 8, "device_id must be 8 chars, got '{id_a}'");
    assert_eq!(id_b.len(), 8, "device_id must be 8 chars, got '{id_b}'");
    assert_ne!(id_a, id_b, "two machines must have distinct device IDs");
    assert!(paths_a.device_id_path().exists());
    assert!(paths_b.device_id_path().exists());

    assert!(device_name_a.contains(&id_a));
    assert!(device_name_b.contains(&id_b));
    assert_ne!(device_name_a, device_name_b);
    assert_ne!(key_a, key_b);

    {
        let st = gateway.state.lock().expect("lock gateway state");
        assert_eq!(
            st.active_tokens.len(),
            2,
            "both machines' keys must remain active"
        );
        let tok_a = &st.active_tokens[0];
        let tok_b = &st.active_tokens[1];
        assert_ne!(tok_a.token_name, tok_b.token_name);
        assert!(tok_a.token_name.starts_with("takoswitch@"));
        assert!(tok_b.token_name.starts_with("takoswitch@"));
        assert!(tok_a.token_name.contains(&id_a));
        assert!(tok_b.token_name.contains(&id_b));
        assert!(
            st.deleted_for_replacement.is_empty(),
            "distinct machines must not evict each other"
        );
    }

    let (status_a, status_b) = tokio::join!(
        fetch_account_status(&paths_a),
        fetch_account_status(&paths_b)
    );
    assert!(
        status_a.logged_in && status_a.error.is_none(),
        "machine A status failed: {:?}",
        status_a
    );
    assert!(
        status_b.logged_in && status_b.error.is_none(),
        "machine B status failed: {:?}",
        status_b
    );
}

#[tokio::test]
async fn tc_md_02_same_machine_relogin_replaces_idempotently_without_leak() {
    let gateway = start_mock_gateway().await;
    let (_tmp_a, paths_a) = setup_sandbox(&gateway.base_url);
    let (_tmp_b, paths_b) = setup_sandbox(&gateway.base_url);

    let (device_name_a1, key_a1) =
        perform_oauth_login_with_host(&gateway, &paths_a, Some("MacBook-Pro.local"), "a1").await;
    let (_device_name_b, key_b) =
        perform_oauth_login_with_host(&gateway, &paths_b, Some("MacBook-Pro.local"), "b1").await;

    let id_a_before = read_or_create_device_id(&paths_a);

    // Re-login machine A on the same sandbox
    let (device_name_a2, key_a2) =
        perform_oauth_login_with_host(&gateway, &paths_a, Some("MacBook-Pro.local"), "a2").await;

    let id_a_after = read_or_create_device_id(&paths_a);
    assert_eq!(id_a_before, id_a_after, "re-login must reuse device-id");
    assert_eq!(device_name_a1, device_name_a2);
    assert_ne!(key_a1, key_a2, "re-login must issue a fresh key");

    {
        let st = gateway.state.lock().expect("lock gateway state");
        assert_eq!(
            st.active_tokens.len(),
            2,
            "total active tokens must remain 2 (no stale leak)"
        );
        assert!(st.active_tokens.iter().any(|t| t.key == key_a2));
        assert!(st.active_tokens.iter().any(|t| t.key == key_b));
        assert!(!st.active_tokens.iter().any(|t| t.key == key_a1));
        assert_eq!(st.deleted_for_replacement.len(), 1);
        assert_eq!(st.deleted_for_replacement[0].1, key_a1);
    }

    // Verify old key_a1 actually gets 401 on /api/desktop/me
    let client = reqwest::Client::new();
    let old_me = client
        .get(format!("{}/api/desktop/me", gateway.base_url))
        .header("Authorization", format!("Bearer {key_a1}"))
        .send()
        .await
        .expect("query /api/desktop/me with replaced key");
    assert_eq!(old_me.status().as_u16(), 401);

    let (status_a, status_b) = tokio::join!(
        fetch_account_status(&paths_a),
        fetch_account_status(&paths_b)
    );
    assert!(status_a.logged_in && status_a.error.is_none());
    assert!(status_b.logged_in && status_b.error.is_none());
}

#[tokio::test]
async fn tc_md_03_single_machine_logout_revokes_only_self_and_preserves_device_id() {
    let gateway = start_mock_gateway().await;
    let (_tmp_a, paths_a) = setup_sandbox(&gateway.base_url);
    let (_tmp_b, paths_b) = setup_sandbox(&gateway.base_url);

    let (_dn_a, key_a) =
        perform_oauth_login_with_host(&gateway, &paths_a, Some("MacBook-Pro.local"), "a1").await;
    let (_dn_b, key_b) =
        perform_oauth_login_with_host(&gateway, &paths_b, Some("MacBook-Pro.local"), "b1").await;

    let id_a_before = read_or_create_device_id(&paths_a);

    let logout_status = logout(&paths_a).await.expect("logout should succeed");
    assert!(!logout_status.logged_in);
    assert!(read_main_api_key(&paths_a).is_none());

    // device-id file must be preserved across logout
    assert!(
        paths_a.device_id_path().exists(),
        "device_id_path must survive logout"
    );
    let persisted_id = fs::read_to_string(paths_a.device_id_path()).expect("read device-id file");
    assert_eq!(persisted_id.trim(), id_a_before);
    assert_eq!(read_or_create_device_id(&paths_a), id_a_before);

    {
        let st = gateway.state.lock().expect("lock gateway state");
        assert!(st.revoke_calls.contains(&key_a));
        assert!(st.revoked_keys.contains(&key_a));
        assert!(!st.revoked_keys.contains(&key_b));
        assert_eq!(st.active_tokens.len(), 1);
        assert_eq!(st.active_tokens[0].key, key_b);
    }

    let status_b = fetch_account_status(&paths_b).await;
    assert!(
        status_b.logged_in && status_b.error.is_none(),
        "machine B must remain logged in after machine A logs out: {:?}",
        status_b
    );
}

#[test]
fn tc_md_04_extreme_hostname_sanitization_and_32_char_cap() {
    let tmp = TempDir::new().expect("create tempdir");
    let paths = AppPaths::for_test(tmp.path());
    let device_id = read_or_create_device_id(&paths);
    assert_eq!(device_id.len(), 8);
    assert_eq!(
        read_or_create_device_id(&paths),
        device_id,
        "read_or_create_device_id must be idempotent"
    );

    // sanitize_host_label checks
    assert_eq!(sanitize_host_label(""), None);
    assert_eq!(sanitize_host_label("   \t\n  "), None);
    assert_eq!(sanitize_host_label("中文主机名"), None);

    let mixed_sanitized = sanitize_host_label("Shiroha 的 MacBook Pro (16-inch).local")
        .expect("mixed hostname should retain ASCII parts");
    assert!(!mixed_sanitized.is_empty());
    assert!(mixed_sanitized.chars().all(is_valid_device_name_char));

    let long_host = "x".repeat(80);
    let cases: Vec<Option<&str>> = vec![
        Some(&long_host),
        Some("Shiroha 的 MacBook Pro (16-inch).local"),
        Some(""),
        Some("   "),
        Some("..."),
        Some("---"),
        Some("中文主机名"),
        Some("MacBook-Pro.local"),
        Some("studio_node+01@home"),
        None,
    ];

    for raw in cases {
        let name = build_desktop_device_name_with_host(&paths, raw);
        assert!(
            (1..=32).contains(&name.len()),
            "device_name '{name}' (from {raw:?}) length {} must be in 1..=32",
            name.len()
        );
        assert!(
            name.chars().all(is_valid_device_name_char),
            "device_name '{name}' (from {raw:?}) contains invalid char"
        );
        assert!(
            name.contains(&device_id),
            "device_name '{name}' (from {raw:?}) must include device_id '{device_id}'"
        );
    }

    let auto_name = build_desktop_device_name(&paths);
    assert!((1..=32).contains(&auto_name.len()));
    assert!(auto_name.chars().all(is_valid_device_name_char));
    assert!(auto_name.contains(&device_id));
}

#[test]
fn tc_md_05_authorize_url_contract_uses_takoswitch_and_encodes_device_name() {
    assert_eq!(DESKTOP_OAUTH_CLIENT_ID, "takoswitch");

    let url = build_oauth_authorize_url(
        "https://tako.shiroha.tech",
        "http://127.0.0.1:45678/tako/callback",
        "state_token_123",
        "challenge_abc_xyz",
        "my+host@1234abcd",
    );

    assert!(
        url.starts_with("https://tako.shiroha.tech/app/authorize?"),
        "unexpected authorize URL prefix: {url}"
    );
    assert!(
        !url.contains("tako-connect"),
        "authorize URL must not contain legacy tako-connect: {url}"
    );

    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    assert!(query.contains("client_id=takoswitch"), "query: {query}");
    assert!(query.contains("response_type=code"), "query: {query}");
    assert!(
        query.contains("code_challenge_method=S256"),
        "query: {query}"
    );
    assert!(
        query.contains("device_name=my%2Bhost%401234abcd"),
        "device_name must be URL-encoded in query: {query}"
    );

    let decoded = parse_query_params(query);
    assert_eq!(
        decoded.get("client_id").map(String::as_str),
        Some("takoswitch")
    );
    assert_eq!(
        decoded.get("response_type").map(String::as_str),
        Some("code")
    );
    assert_eq!(
        decoded.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        decoded.get("redirect_uri").map(String::as_str),
        Some("http://127.0.0.1:45678/tako/callback")
    );
    assert_eq!(
        decoded.get("state").map(String::as_str),
        Some("state_token_123")
    );
    assert_eq!(
        decoded.get("code_challenge").map(String::as_str),
        Some("challenge_abc_xyz")
    );
    assert_eq!(
        decoded.get("device_name").map(String::as_str),
        Some("my+host@1234abcd")
    );
}

#[tokio::test]
async fn tc_md_06_gateway_rejects_legacy_client_id_and_bad_pkce_and_unlogged_sandbox() {
    let gateway = start_mock_gateway().await;
    let (_tmp, paths) = setup_sandbox(&gateway.base_url);

    // Unlogged sandbox returns logged_in == false
    let unlogged = fetch_account_status(&paths).await;
    assert!(!unlogged.logged_in);

    // Legacy client_id must be rejected
    let client = reqwest::Client::new();
    let bad_client_resp = client
        .post(format!("{}/api/desktop/token", gateway.base_url))
        .json(&json!({
            "grant_type": "authorization_code",
            "code": "auth_code_1",
            "code_verifier": "v",
            "redirect_uri": "http://127.0.0.1:49999/tako/callback",
            "client_id": "tako-connect"
        }))
        .send()
        .await
        .expect("POST /api/desktop/token");
    assert_eq!(bad_client_resp.status().as_u16(), 400);

    // Wrong PKCE code_verifier must be rejected by exchange_desktop_token without saving auth
    let redirect_uri = "http://127.0.0.1:49999/tako/callback";
    let device_name = build_desktop_device_name_with_host(&paths, Some("MacBook-Pro.local"));
    let auth_url = build_oauth_authorize_url(
        &gateway.base_url,
        redirect_uri,
        "state_bad_pkce",
        &pkce_s256_challenge("correct_verifier_0123456789_abcdefghijklmnopqrstuvwxyz"),
        &device_name,
    );
    let no_redirect_client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("build no-redirect client");
    let auth_resp = no_redirect_client
        .get(&auth_url)
        .send()
        .await
        .expect("GET /app/authorize");
    assert_eq!(auth_resp.status().as_u16(), 302);
    let location = auth_resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .expect("Location header");
    let code = parse_query_params(location.split_once('?').map(|(_, q)| q).unwrap_or(""))
        .get("code")
        .cloned()
        .expect("code in Location");

    let err = exchange_desktop_token(
        &paths,
        &gateway.base_url,
        &code,
        "wrong_verifier_0123456789",
        redirect_uri,
    )
    .await
    .expect_err("wrong PKCE verifier must fail exchange_desktop_token");
    assert!(!err.is_empty());
    assert!(read_main_api_key(&paths).is_none());
}
