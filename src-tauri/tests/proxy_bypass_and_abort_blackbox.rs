use hyper::header::{HeaderName, HeaderValue, CONTENT_LENGTH, TRANSFER_ENCODING};
use hyper::{HeaderMap, StatusCode};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::future::Future;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tako_switch_lib::agy_proxy::serve_proxy_on_listener;
use tako_switch_lib::config::{
    read_switch_state, save_switch_auth, save_switch_state, write_atomic, AppPaths, SwitchState,
};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{oneshot, watch, Mutex, MutexGuard, Notify};
use tokio::task::JoinSet;

const PROXY_ENV_KEYS: [&str; 9] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "REQUEST_METHOD", // The proxy matcher ignores proxy settings in CGI environments.
];
// All tests in this binary hold this lock before any environment-dependent work.
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

struct ProxyEnvGuard {
    saved: [Option<OsString>; PROXY_ENV_KEYS.len()],
    runtime: Option<Runtime>,
    _lock: MutexGuard<'static, ()>,
}

impl ProxyEnvGuard {
    fn new(lock: MutexGuard<'static, ()>) -> Self {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let guard = Self {
            saved: PROXY_ENV_KEYS.map(std::env::var_os),
            runtime: Some(runtime),
            _lock: lock,
        };
        for key in PROXY_ENV_KEYS {
            std::env::remove_var(key);
        }
        guard
    }

    fn set_http_proxy(&mut self, address: SocketAddr) {
        std::env::set_var("HTTP_PROXY", format!("http://{address}"));
        // A nonmatching entry also keeps OS proxy exceptions out of the fixture.
        std::env::set_var("NO_PROXY", "example.invalid");
    }

    fn run<T>(&self, test: impl Future<Output = T>) -> T {
        self.runtime.as_ref().unwrap().block_on(test)
    }
}

impl Drop for ProxyEnvGuard {
    fn drop(&mut self) {
        // The listener spawns untracked connection tasks; shutting down the runtime stops them too.
        drop(self.runtime.take());
        for (key, value) in PROXY_ENV_KEYS.into_iter().zip(&self.saved) {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

fn setup_sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create tempdir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(
        paths.is_test,
        "blackbox tests must run with paths.is_test == true"
    );
    save_switch_auth(
        &paths,
        Some("cr_test_proxy_bypass_key"),
        Some("1"),
        Some("oauth"),
    )
    .expect("seed test auth");
    (tmp, paths)
}

fn loopback_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn is_chunked(headers: &HeaderMap) -> bool {
    headers.get_all(TRANSFER_ENCODING).iter().any(|value| {
        value
            .as_bytes()
            .split(|byte| *byte == b',')
            .any(|coding| coding.trim_ascii().eq_ignore_ascii_case(b"chunked"))
    })
}

async fn write_mock_response(
    stream: &mut BufReader<TcpStream>,
    status: StatusCode,
    content_type: &str,
    body: &str,
) {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        status.as_u16(),
        status.canonical_reason().unwrap(),
        body.len(),
    );
    stream.write_all(response.as_bytes()).await.unwrap();
}

async fn read_http_line(stream: &mut BufReader<TcpStream>) -> Result<String, String> {
    let mut line = String::new();
    stream
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    if !line.ends_with("\r\n") {
        return Err(format!("expected CRLF-terminated HTTP line, got {line:?}"));
    }
    line.truncate(line.len() - 2);
    Ok(line)
}

fn parse_header_line(line: &str) -> Result<(HeaderName, HeaderValue), String> {
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| format!("invalid HTTP header: {line:?}"))?;
    let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| e.to_string())?;
    let value = HeaderValue::from_str(value.trim()).map_err(|e| e.to_string())?;
    Ok((name, value))
}

async fn read_http_headers(
    stream: &mut BufReader<TcpStream>,
) -> Result<(String, HeaderMap), String> {
    let request_line = read_http_line(stream).await?;
    let mut headers = HeaderMap::new();
    loop {
        let line = read_http_line(stream).await?;
        if line.is_empty() {
            return Ok((request_line, headers));
        }
        let (name, value) = parse_header_line(&line)?;
        headers.append(name, value);
    }
}

async fn read_chunked_or_fixed_body(
    stream: &mut BufReader<TcpStream>,
    headers: &HeaderMap,
    mut first_chunk: Option<oneshot::Sender<()>>,
) -> Result<Vec<u8>, String> {
    if let Some(encoding) = headers.get(TRANSFER_ENCODING) {
        if !encoding.as_bytes().eq_ignore_ascii_case(b"chunked") {
            return Err(format!("unexpected transfer encoding: {encoding:?}"));
        }
        if headers.contains_key(CONTENT_LENGTH) {
            return Err("chunked upload must not include Content-Length".to_string());
        }

        let mut decoded = Vec::new();
        loop {
            let line = read_http_line(stream).await?;
            let size_hex = line.split_once(';').map_or(line.as_str(), |(size, _)| size);
            if size_hex.is_empty() || !size_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("invalid chunk size: {line:?}"));
            }
            let len = usize::from_str_radix(size_hex, 16)
                .map_err(|e| format!("bad chunk size {size_hex:?}: {e}"))?;
            if len == 0 {
                loop {
                    let trailer = read_http_line(stream).await?;
                    if trailer.is_empty() {
                        return Ok(decoded);
                    }
                    parse_header_line(&trailer)?;
                }
            }

            let start = decoded.len();
            let end = start.checked_add(len).ok_or("chunk length overflow")?;
            decoded.resize(end, 0);
            stream
                .read_exact(&mut decoded[start..])
                .await
                .map_err(|e| format!("incomplete chunk data: {e}"))?;
            let mut crlf = [0; 2];
            stream
                .read_exact(&mut crlf)
                .await
                .map_err(|e| format!("incomplete chunk terminator: {e}"))?;
            if &crlf != b"\r\n" {
                return Err("invalid chunk terminator".to_string());
            }
            if let Some(tx) = first_chunk.take() {
                let _ = tx.send(());
            }
        }
    }

    if first_chunk.is_some() {
        return Err("native upload must be chunked".to_string());
    }
    let len = match headers.get(CONTENT_LENGTH) {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or("invalid Content-Length")?,
        None => 0,
    };
    let mut body = vec![0; len];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("incomplete fixed body: {e}"))?;
    Ok(body)
}

#[test]
fn switch_state_bypass_system_proxy_defaults_true_and_roundtrips_false() {
    let _environment = ENV_LOCK.blocking_lock();
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

#[test]
fn gateway_honors_bypass_system_proxy_toggle_live_without_restart() {
    let lock = ENV_LOCK.blocking_lock();
    let (_tmp, paths) = setup_sandbox();
    let mut environment = ProxyEnvGuard::new(lock);
    let proxy_listener = environment.run(TcpListener::bind("127.0.0.1:0")).unwrap();
    environment.set_http_proxy(proxy_listener.local_addr().unwrap());
    environment.run(check_live_proxy_toggle(paths, proxy_listener));
}

async fn check_live_proxy_toggle(paths: AppPaths, proxy_listener: TcpListener) {
    let mut tasks = JoinSet::new();
    let proxy_hits = Arc::new(AtomicUsize::new(0));
    let proxy_hits_clone = proxy_hits.clone();
    tasks.spawn(async move {
        loop {
            let (sock, _) = proxy_listener.accept().await.unwrap();
            let mut sock = BufReader::new(sock);
            let (request_line, _) = read_http_headers(&mut sock).await.unwrap();
            proxy_hits_clone.fetch_add(1, Ordering::SeqCst);
            let body = json!({
                "models": [{
                    "name": "models/gemini-4-carbon",
                    "displayName": "Gemini 4 Carbon",
                    "description": request_line
                }]
            });
            write_mock_response(
                &mut sock,
                StatusCode::OK,
                "application/json",
                &body.to_string(),
            )
            .await;
        }
    });

    // Keep the direct origin bound: a refused loopback connect can take seconds on Windows.
    let direct_upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_upstream_addr = direct_upstream.local_addr().unwrap();
    tasks.spawn(async move {
        loop {
            let (sock, _) = direct_upstream.accept().await.unwrap();
            let mut sock = BufReader::new(sock);
            read_http_headers(&mut sock).await.unwrap();
            write_mock_response(
                &mut sock,
                StatusCode::SERVICE_UNAVAILABLE,
                "application/json",
                "",
            )
            .await;
        }
    });
    let mut state = read_switch_state(&paths);
    state.base_url = format!("http://{direct_upstream_addr}");
    assert!(state.bypass_system_proxy);
    save_switch_state(&paths, &state).unwrap();

    let gw_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    let paths_for_gw = paths.clone();
    tasks.spawn(async move {
        serve_proxy_on_listener(gw_listener, paths_for_gw)
            .await
            .unwrap();
    });

    let test_client = loopback_client();

    // 1) Default `bypass_system_proxy = true`: must NOT touch `HTTP_PROXY`.
    let resp_direct = test_client
        .get(format!("http://{gw_addr}/v1beta/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp_direct.status().as_u16(), 503);
    resp_direct.bytes().await.unwrap();
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
    resp_via_proxy.bytes().await.unwrap();
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
    assert_eq!(resp_direct_again.status().as_u16(), 503);
    resp_direct_again.bytes().await.unwrap();
    assert_eq!(
        proxy_hits.load(Ordering::SeqCst),
        hits_before,
        "re-enabling bypass_system_proxy=true must stop hitting HTTP_PROXY immediately"
    );
    tasks.shutdown().await;
}

#[test]
fn downstream_disconnect_immediately_aborts_hanging_upstream_and_streams_large_body() {
    let lock = ENV_LOCK.blocking_lock();
    let (_tmp, paths) = setup_sandbox();
    let environment = ProxyEnvGuard::new(lock);
    environment.run(check_streaming_abort(paths));
}

async fn check_streaming_abort(paths: AppPaths) {
    let mut tasks = JoinSet::new();

    let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = upstream_listener.local_addr().unwrap();

    let pricing_started = Arc::new(Notify::new());
    let pricing_started_upstream = pricing_started.clone();
    let (release_pricing, pricing_released) = watch::channel(false);
    let (hang1_streamed_tx, hang1_streamed_rx) = oneshot::channel();
    let (hang1_arrived_tx, hang1_arrived_rx) = oneshot::channel::<Vec<u8>>();
    let (hang1_closed_tx, hang1_closed_rx) = oneshot::channel();
    let (hang2_arrived_tx, hang2_arrived_rx) = oneshot::channel::<Value>();
    let (hang2_closed_tx, hang2_closed_rx) = oneshot::channel();
    let hang1_streamed = Arc::new(std::sync::Mutex::new(Some(hang1_streamed_tx)));
    let hang1_arrived = Arc::new(std::sync::Mutex::new(Some(hang1_arrived_tx)));
    let hang1_closed = Arc::new(std::sync::Mutex::new(Some(hang1_closed_tx)));
    let hang2_arrived = Arc::new(std::sync::Mutex::new(Some(hang2_arrived_tx)));
    let hang2_closed = Arc::new(std::sync::Mutex::new(Some(hang2_closed_tx)));

    let get_had_chunked = Arc::new(AtomicUsize::new(0));
    let get_had_chunked_clone = get_had_chunked.clone();

    tasks.spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            let (sock, _) = tokio::select! {
                connection = upstream_listener.accept() => connection.unwrap(),
                finished = connections.join_next(), if !connections.is_empty() => {
                    finished.unwrap().unwrap();
                    continue;
                }
            };
            let pricing_started = pricing_started_upstream.clone();
            let mut pricing_released = pricing_released.clone();
            let h1_streamed = hang1_streamed.clone();
            let h1_a = hang1_arrived.clone();
            let h1_c = hang1_closed.clone();
            let h2_a = hang2_arrived.clone();
            let h2_c = hang2_closed.clone();
            let get_chunked = get_had_chunked_clone.clone();

            connections.spawn(async move {
                let mut sock = BufReader::new(sock);
                let (req_line, headers) = read_http_headers(&mut sock).await.unwrap();

                if req_line.starts_with("GET /v1beta/models ") {
                    // Leave the synthetic route unset so its request exercises the pricing fallback.
                    let body = r#"{"models":[]}"#;
                    write_mock_response(&mut sock, StatusCode::OK, "application/json", body).await;
                    return;
                }

                if req_line.starts_with("GET /api/pricing ") {
                    pricing_started.notify_one();
                    if pricing_released
                        .wait_for(|released| *released)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let body = json!({
                        "data": [{
                            "model_name": "gemini-4-carbon",
                            "reasoning_levels": ["low", "medium", "high"]
                        }]
                    })
                    .to_string();
                    write_mock_response(&mut sock, StatusCode::OK, "application/json", &body).await;
                    return;
                }

                if req_line.starts_with("GET /codex/ping ") || req_line.starts_with("GET /ping ") {
                    if is_chunked(&headers) {
                        get_chunked.fetch_add(1, Ordering::SeqCst);
                    }
                    let body = r#"{"pong":true}"#;
                    write_mock_response(&mut sock, StatusCode::OK, "application/json", body).await;
                    return;
                }

                let first_chunk = if req_line.starts_with(
                    "POST /v1beta/models/gemini-4-carbon:streamGenerateContent?alt=sse ",
                ) {
                    h1_streamed.lock().unwrap().take()
                } else {
                    None
                };
                let body_bytes = read_chunked_or_fixed_body(&mut sock, &headers, first_chunk)
                    .await
                    .unwrap_or_else(|err| {
                        panic!("cannot read upstream body for {req_line}: {err}")
                    });

                if req_line.starts_with("POST /v1/responses ")
                    || req_line.contains("/codex/v1/responses")
                {
                    let sse = format!(
                        "data: {{\"received_bytes\":{}}}\n\ndata: [DONE]\n\n",
                        body_bytes.len()
                    );
                    write_mock_response(&mut sock, StatusCode::OK, "text/event-stream", &sse).await;
                    return;
                }

                // Hanging streamGenerateContent endpoint: hold response headers until downstream aborts us.
                assert!(
                    req_line.starts_with(
                        "POST /v1beta/models/gemini-4-carbon:streamGenerateContent?alt=sse "
                    ),
                    "unexpected request: {req_line}"
                );
                let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
                let has_injected_effort = body_json
                    .pointer("/generationConfig/thinkingConfig/thinkingLevel")
                    .is_some();

                if !has_injected_effort {
                    if let Some(tx) = h1_a.lock().unwrap().take() {
                        let _ = tx.send(body_bytes);
                    }
                } else if let Some(tx) = h2_a.lock().unwrap().take() {
                    let _ = tx.send(body_json);
                }

                let closed = match sock.read(&mut [0u8; 1]).await {
                    Ok(0) => Ok(()),
                    Err(err)
                        if matches!(
                            err.kind(),
                            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
                        ) =>
                    {
                        Ok(())
                    }
                    result => Err(format!("expected upstream EOF or reset, got {result:?}")),
                };

                if !has_injected_effort {
                    if let Some(tx) = h1_c.lock().unwrap().take() {
                        let _ = tx.send(closed);
                    }
                } else if let Some(tx) = h2_c.lock().unwrap().take() {
                    let _ = tx.send(closed);
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
    tasks.spawn(async move {
        serve_proxy_on_listener(gw_listener, paths_for_gw)
            .await
            .unwrap();
    });

    let client = loopback_client();

    // A) Empty GET request must NOT emit Transfer-Encoding: chunked.
    let ping_resp = client
        .get(format!("http://{gw_addr}/codex/ping"))
        .send()
        .await
        .unwrap();
    assert_eq!(ping_resp.status().as_u16(), 200);
    ping_resp.bytes().await.unwrap();
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

    // Hold the pricing fetch open while the native model streams its request.
    let catalog_client = client.clone();
    let (catalog_done, catalog) = oneshot::channel();
    tasks.spawn(async move {
        let result = async {
            let response = catalog_client
                .get(format!("http://{gw_addr}/v1beta/models"))
                .send()
                .await?;
            let status = response.status();
            response.bytes().await?;
            Ok::<_, reqwest::Error>(status)
        }
        .await;
        let _ = catalog_done.send(result);
    });
    tokio::time::timeout(Duration::from_secs(2), pricing_started.notified())
        .await
        .expect("catalog request should start a pricing fetch");

    // C) Native streamed POST: withhold the tail to detect buffering before forwarding.
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
    let (prefix, tail) = body1.as_bytes().split_at(body1.len() / 2);
    raw_client1.write_all(prefix).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), hang1_streamed_rx)
        .await
        .expect("native upload should start before pricing or the full request body is available")
        .expect("native upload should deliver a chunked body frame");
    raw_client1.write_all(tail).await.unwrap();

    let arrived_body = tokio::time::timeout(Duration::from_secs(2), hang1_arrived_rx)
        .await
        .expect("upstream should receive streamed request body")
        .unwrap();
    assert!(arrived_body == body1.as_bytes(), "native body was modified");

    // Drop the whole downstream socket only after the upstream consumes the body.
    let disconnect_started = Instant::now();
    drop(raw_client1);

    tokio::time::timeout(Duration::from_millis(600), hang1_closed_rx)
        .await
        .expect("upstream connection must be aborted promptly when downstream disconnects")
        .unwrap()
        .unwrap();
    assert!(
        disconnect_started.elapsed() < Duration::from_millis(600),
        "abort took too long after downstream close"
    );

    release_pricing.send(true).unwrap();
    assert_eq!(catalog.await.unwrap().unwrap(), StatusCode::OK);

    // D) Synthetic rewritten POST: client disconnects while awaiting response headers.
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
        .unwrap()
        .unwrap();
    assert!(disconnect_started2.elapsed() < Duration::from_millis(600));
    tasks.shutdown().await;
}
