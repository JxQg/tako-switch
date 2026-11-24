use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tako_switch_lib::agy_proxy::serve_proxy_on_listener;
use tako_switch_lib::config::{
    read_switch_state, save_switch_auth, save_switch_state, write_atomic, AppPaths, SwitchState,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

fn setup_sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create tempdir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(paths.is_test, "blackbox tests must run with paths.is_test == true");
    save_switch_auth(&paths, Some("cr_test_proxy_bypass_key"), Some("1"), Some("oauth"))
        .expect("seed test auth");
    (tmp, paths)
}

async fn read_http_headers(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    loop {
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("read headers err: {e}"))?;
        if n == 0 {
            return Err("eof before end of headers".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos + 4]).into_owned();
            let remainder = buf[pos + 4..].to_vec();
            return Ok((head, remainder));
        }
    }
}

async fn read_chunked_or_fixed_body(
    stream: &mut TcpStream,
    headers: &str,
    mut remainder: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let lower = headers.to_ascii_lowercase();
    if lower.contains("transfer-encoding: chunked") {
        let mut decoded = Vec::new();
        loop {
            while !remainder.windows(2).any(|w| w == b"\r\n") {
                let mut tmp = [0u8; 4096];
                let n = stream
                    .read(&mut tmp)
                    .await
                    .map_err(|e| format!("chunk size read err: {e}"))?;
                if n == 0 {
                    return Err("unexpected eof in chunked size".to_string());
                }
                remainder.extend_from_slice(&tmp[..n]);
            }
            let crlf = remainder.windows(2).position(|w| w == b"\r\n").unwrap();
            let size_hex = String::from_utf8_lossy(&remainder[..crlf])
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let chunk_len = usize::from_str_radix(&size_hex, 16)
                .map_err(|e| format!("bad chunk size {size_hex:?}: {e}"))?;
            remainder.drain(..crlf + 2);
            if chunk_len == 0 {
                while remainder.len() < 2 {
                    let mut tmp = [0u8; 64];
                    let n = stream
                        .read(&mut tmp)
                        .await
                        .map_err(|e| format!("trailer read err: {e}"))?;
                    if n == 0 {
                        break;
                    }
                    remainder.extend_from_slice(&tmp[..n]);
                }
                return Ok(decoded);
            }
            while remainder.len() < chunk_len + 2 {
                let mut tmp = [0u8; 16384];
                let n = stream
                    .read(&mut tmp)
                    .await
                    .map_err(|e| format!("chunk data read err: {e}"))?;
                if n == 0 {
                    return Err("unexpected eof in chunk data".to_string());
                }
                remainder.extend_from_slice(&tmp[..n]);
            }
            decoded.extend_from_slice(&remainder[..chunk_len]);
            remainder.drain(..chunk_len + 2);
        }
    }

    let content_len = headers
        .lines()
        .find_map(|line| {
            let (k, v) = line.split_once(':')?;
            if k.trim().eq_ignore_ascii_case("content-length") {
                v.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    while remainder.len() < content_len {
        let mut tmp = [0u8; 16384];
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("fixed body read err: {e}"))?;
        if n == 0 {
            return Err(format!(
                "unexpected eof reading fixed body ({}/{})",
                remainder.len(),
                content_len
            ));
        }
        remainder.extend_from_slice(&tmp[..n]);
    }
    remainder.truncate(content_len);
    Ok(remainder)
}

#[test]
fn switch_state_bypass_system_proxy_defaults_true_and_roundtrips_false() {
    let (_tmp, paths) = setup_sandbox();

    let fresh = SwitchState::default_for(&paths);
    assert!(fresh.bypass_system_proxy);

    // Legacy state.json without `bypassSystemProxy` must default to `true`.
    let legacy = json!({
        "baseUrl": "https://tako.shiroha.tech",
        "manifestUrl": fresh.manifest_url,
        "fallbackManifestUrl": fresh.fallback_manifest_url,
        "autoCheckUpdates": true
    });
    write_atomic(
        &paths.state_path(),
        serde_json::to_string_pretty(&legacy).unwrap().as_bytes(),
        false,
    )
    .unwrap();
    let migrated = read_switch_state(&paths);
    assert!(
        migrated.bypass_system_proxy,
        "legacy state without bypassSystemProxy must default to true"
    );

    // Explicitly toggling off persists `bypassSystemProxy: false`.
    let mut updated = migrated;
    updated.bypass_system_proxy = false;
    save_switch_state(&paths, &updated).unwrap();

    let raw_saved: Value =
        serde_json::from_str(&std::fs::read_to_string(paths.state_path()).unwrap()).unwrap();
    assert_eq!(raw_saved["bypassSystemProxy"], Value::Bool(false));
    assert!(!read_switch_state(&paths).bypass_system_proxy);
}

#[tokio::test]
async fn gateway_honors_bypass_system_proxy_toggle_live_without_restart() {
    let (_tmp, paths) = setup_sandbox();

    // Start a mock HTTP forward proxy on 127.0.0.1:<ephemeral>
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_hits = Arc::new(AtomicUsize::new(0));
    let proxy_hits_clone = proxy_hits.clone();

    tokio::spawn(async move {
        while let Ok((mut sock, _)) = proxy_listener.accept().await {
            let hits = proxy_hits_clone.clone();
            tokio::spawn(async move {
                if let Ok((head, _rem)) = read_http_headers(&mut sock).await {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let body = json!({
                        "models": [{
                            "name": "models/gemini-4-carbon",
                            "displayName": "Gemini 4 Carbon",
                            "description": head.lines().next().unwrap_or("")
                        }]
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    // Configure process env HTTP_PROXY before gateway clients are constructed.
    // Note: this test runs in its own integration test binary process.
    std::env::set_var("HTTP_PROXY", format!("http://{proxy_addr}"));
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");

    // A closed loopback port fails deterministically without external DNS timeouts.
    // - With `bypass_system_proxy = true` (default), `.no_proxy()` skips `HTTP_PROXY` and fails locally (0 proxy hits).
    // - With `bypass_system_proxy = false`, `system_proxy_client` uses the mock HTTP proxy (200 OK, proxy_hits > 0).
    let unavailable = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let unavailable_addr = unavailable.local_addr().unwrap();
    drop(unavailable);
    let mut state = read_switch_state(&paths);
    state.base_url = format!("http://{unavailable_addr}");
    assert!(state.bypass_system_proxy);
    save_switch_state(&paths, &state).unwrap();

    let gw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    let paths_for_gw = paths.clone();
    tokio::spawn(async move {
        let _ = serve_proxy_on_listener(gw_listener, paths_for_gw).await;
    });

    let test_client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // 1) Default `bypass_system_proxy = true`: must NOT touch `HTTP_PROXY`.
    let resp_direct = test_client
        .get(format!("http://{gw_addr}/v1beta/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_direct.status().as_u16(), 502);
    assert_eq!(
        proxy_hits.load(Ordering::SeqCst),
        0,
        "default bypass_system_proxy=true must never send traffic to HTTP_PROXY"
    );

    // 2) Flip `bypass_system_proxy = false` on disk: live gateway switches to system proxy immediately.
    state.bypass_system_proxy = false;
    save_switch_state(&paths, &state).unwrap();

    let resp_via_proxy = test_client
        .get(format!("http://{gw_addr}/v1beta/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_via_proxy.status().as_u16(), 200);
    assert!(
        proxy_hits.load(Ordering::SeqCst) >= 1,
        "bypass_system_proxy=false must route through HTTP_PROXY"
    );

    // 3) Flip back to `true`: returns to direct mode without hitting `HTTP_PROXY`.
    let hits_before = proxy_hits.load(Ordering::SeqCst);
    state.bypass_system_proxy = true;
    save_switch_state(&paths, &state).unwrap();

    let resp_direct_again = test_client
        .get(format!("http://{gw_addr}/v1beta/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_direct_again.status().as_u16(), 502);
    assert_eq!(
        proxy_hits.load(Ordering::SeqCst),
        hits_before,
        "re-enabling bypass_system_proxy=true must stop hitting HTTP_PROXY immediately"
    );
}

#[tokio::test]
async fn downstream_disconnect_immediately_aborts_hanging_upstream_and_streams_large_body() {
    let (_tmp, paths) = setup_sandbox();

    // Start mock upstream server that:
    // - handles `/api/pricing` quickly (declaring `gemini-4-carbon` reasoning levels)
    // - on `/echo-large`, verifies >100 KB streamed body arrives intact and responds with SSE
    // - on `:streamGenerateContent`, signals arrival and holds response headers until client disconnects,
    //   recording exact time until upstream TCP EOF.
    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    let (hang1_arrived_tx, hang1_arrived_rx) = oneshot::channel::<usize>();
    let (hang1_closed_tx, hang1_closed_rx) = oneshot::channel::<Duration>();
    let (hang2_arrived_tx, hang2_arrived_rx) = oneshot::channel::<Value>();
    let (hang2_closed_tx, hang2_closed_rx) = oneshot::channel::<Duration>();
    let hang1_arrived = Arc::new(std::sync::Mutex::new(Some(hang1_arrived_tx)));
    let hang1_closed = Arc::new(std::sync::Mutex::new(Some(hang1_closed_tx)));
    let hang2_arrived = Arc::new(std::sync::Mutex::new(Some(hang2_arrived_tx)));
    let hang2_closed = Arc::new(std::sync::Mutex::new(Some(hang2_closed_tx)));

    let get_had_chunked = Arc::new(AtomicUsize::new(0));
    let get_had_chunked_clone = get_had_chunked.clone();

    tokio::spawn(async move {
        while let Ok((mut sock, _)) = upstream_listener.accept().await {
            let h1_a = hang1_arrived.clone();
            let h1_c = hang1_closed.clone();
            let h2_a = hang2_arrived.clone();
            let h2_c = hang2_closed.clone();
            let get_chunked = get_had_chunked_clone.clone();

            tokio::spawn(async move {
                let Ok((head, rem)) = read_http_headers(&mut sock).await else {
                    return;
                };
                let req_line = head.lines().next().unwrap_or("").to_string();

                if req_line.starts_with("GET /api/pricing ") {
                    let body = json!({
                        "data": [{
                            "model_name": "gemini-4-carbon",
                            "reasoning_levels": ["low", "medium", "high"]
                        }]
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    return;
                }

                if req_line.starts_with("GET /codex/ping ") || req_line.starts_with("GET /ping ") {
                    if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
                        get_chunked.fetch_add(1, Ordering::SeqCst);
                    }
                    let body = r#"{"pong":true}"#;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    return;
                }

                let Ok(body_bytes) = read_chunked_or_fixed_body(&mut sock, &head, rem).await else {
                    return;
                };

                if req_line.starts_with("POST /v1/responses ")
                    || req_line.contains("/codex/v1/responses")
                {
                    let sse = format!(
                        "data: {{\"received_bytes\":{}}}\n\ndata: [DONE]\n\n",
                        body_bytes.len()
                    );
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        sse.len(),
                        sse
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    return;
                }

                // Hanging streamGenerateContent endpoint: hold response headers until downstream aborts us.
                let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
                let has_injected_effort = body_json
                    .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                    .is_some();

                let received_at = Instant::now();
                if !has_injected_effort {
                    if let Some(tx) = h1_a.lock().unwrap().take() {
                        let _ = tx.send(body_bytes.len());
                    }
                } else if let Some(tx) = h2_a.lock().unwrap().take() {
                    let _ = tx.send(body_json);
                }

                let mut probe = [0u8; 16];
                let _ = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut probe)).await;
                let elapsed = received_at.elapsed();

                if !has_injected_effort {
                    if let Some(tx) = h1_c.lock().unwrap().take() {
                        let _ = tx.send(elapsed);
                    }
                } else if let Some(tx) = h2_c.lock().unwrap().take() {
                    let _ = tx.send(elapsed);
                }
            });
        }
    });

    let mut state = read_switch_state(&paths);
    state.base_url = format!("http://{upstream_addr}");
    state.bypass_system_proxy = true;
    save_switch_state(&paths, &state).unwrap();

    let gw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    let paths_for_gw = paths.clone();
    tokio::spawn(async move {
        let _ = serve_proxy_on_listener(gw_listener, paths_for_gw).await;
    });

    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    // A) Empty GET request must NOT emit Transfer-Encoding: chunked.
    let ping_resp = client
        .get(format!("http://{gw_addr}/codex/ping"))
        .send()
        .await
        .unwrap();
    assert_eq!(ping_resp.status().as_u16(), 200);
    assert_eq!(
        get_had_chunked.load(Ordering::SeqCst),
        0,
        "GET with empty body must not send Transfer-Encoding: chunked"
    );

    // B) Large (>100 KB) untouched body streams end-to-end intact and returns SSE payload.
    let big_text = "x".repeat(120 * 1024);
    let large_payload = json!({
        "model": "gpt-6.2-sol",
        "input": big_text,
    })
    .to_string();
    let expected_len = large_payload.len();
    let sse_resp = client
        .post(format!("http://{gw_addr}/codex/v1/responses"))
        .header("content-type", "application/json")
        .body(large_payload)
        .send()
        .await
        .unwrap();
    assert_eq!(sse_resp.status().as_u16(), 200);
    let sse_text = sse_resp.text().await.unwrap();
    assert!(
        sse_text.contains(&format!("\"received_bytes\":{expected_len}")),
        "streamed large body size mismatch, got: {sse_text}"
    );

    // C) Case 1: Native streamed large POST (`gemini-4-carbon`, >80 KB) — client disconnects while awaiting headers.
    let mut raw_client1 = TcpStream::connect(gw_addr).await.unwrap();
    let body1 = json!({
        "contents": [{
            "role": "user",
            "parts": [{ "text": "a".repeat(90 * 1024) }]
        }]
    })
    .to_string();
    let req1 = format!(
        "POST /v1beta/models/gemini-4-carbon:streamGenerateContent?alt=sse HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body1.len()
    );
    raw_client1.write_all(req1.as_bytes()).await.unwrap();
    raw_client1.write_all(body1.as_bytes()).await.unwrap();

    let arrived_len = tokio::time::timeout(Duration::from_secs(2), hang1_arrived_rx)
        .await
        .expect("upstream should receive streamed request body")
        .unwrap();
    assert_eq!(arrived_len, body1.len());

    // Simulate Antigravity 60s ResponseHeaderTimeout closing the local socket.
    let disconnect_started = Instant::now();
    drop(raw_client1);

    let _upstream_total = tokio::time::timeout(Duration::from_millis(600), hang1_closed_rx)
        .await
        .expect("upstream connection must be aborted promptly when downstream disconnects")
        .unwrap();
    assert!(
        disconnect_started.elapsed() < Duration::from_millis(600),
        "abort took too long after downstream close"
    );

    // D) Case 2: Synthetic rewritten POST (`gemini-4-carbon-high`) — client disconnects while awaiting headers.
    let mut raw_client2 = TcpStream::connect(gw_addr).await.unwrap();
    let body2 = json!({
        "contents": [{
            "role": "user",
            "parts": [{ "text": "hello synthetic effort" }]
        }]
    })
    .to_string();
    let req2 = format!(
        "POST /v1beta/models/gemini-4-carbon-high:streamGenerateContent?alt=sse HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body2.len()
    );
    raw_client2.write_all(req2.as_bytes()).await.unwrap();
    raw_client2.write_all(body2.as_bytes()).await.unwrap();

    let rewritten_json = tokio::time::timeout(Duration::from_secs(2), hang2_arrived_rx)
        .await
        .expect("upstream should receive rewritten synthetic request")
        .unwrap();
    assert_eq!(
        rewritten_json.pointer("/generationConfig/thinkingConfig/thinkingLevel"),
        Some(&Value::String("HIGH".to_string()))
    );

    let disconnect_started2 = Instant::now();
    drop(raw_client2);

    tokio::time::timeout(Duration::from_millis(600), hang2_closed_rx)
        .await
        .expect("rewritten synthetic upstream connection must also abort promptly on client close")
        .unwrap();
    assert!(disconnect_started2.elapsed() < Duration::from_millis(600));
}
