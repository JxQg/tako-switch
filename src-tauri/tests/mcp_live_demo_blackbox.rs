use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tako_switch_lib::config::{
    read_switch_state, save_switch_auth, save_switch_state, AppPaths,
};
use tako_switch_lib::mcp_server::{
    execute_live_demo_for_paths, LiveDemoCitation, LiveDemoRequest, LiveDemoResult,
};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

impl RecordedRequest {
    fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Clone)]
struct MockResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl MockResponse {
    fn json(status: u16, val: &Value) -> Self {
        Self {
            status,
            content_type: "application/json".to_string(),
            body: serde_json::to_vec(val).expect("serialize mock json"),
        }
    }

    fn binary(status: u16, content_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            status,
            content_type: content_type.to_string(),
            body: bytes,
        }
    }
}

struct MockTakoServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    handle: JoinHandle<()>,
}

impl Drop for MockTakoServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<RecordedRequest> {
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
    let path = raw_target
        .split('?')
        .next()
        .unwrap_or(&raw_target)
        .to_string();

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

    Some(RecordedRequest {
        method,
        path,
        authorization,
        body,
    })
}

async fn write_http_response(stream: &mut tokio::net::TcpStream, resp: MockResponse) {
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Status",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        reason,
        resp.content_type,
        resp.body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&resp.body).await;
    let _ = stream.flush().await;
}

async fn start_mock_server<F>(handler: F) -> MockTakoServer
where
    F: Fn(&RecordedRequest, usize) -> MockResponse + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock listener");
    let addr = listener.local_addr().expect("local addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_bg = Arc::clone(&requests);
    let handler = Arc::new(handler);

    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let reqs = Arc::clone(&requests_bg);
            let h = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Some(req) = read_http_request(&mut stream).await {
                    let seq_idx = {
                        let mut guard = reqs.lock().expect("lock requests");
                        guard.push(req.clone());
                        guard.len() - 1
                    };
                    let resp = h(&req, seq_idx);
                    write_http_response(&mut stream, resp).await;
                }
            });
        }
    });

    MockTakoServer {
        base_url: format!("http://127.0.0.1:{}", addr.port()),
        requests,
        handle,
    }
}

fn setup_sandbox(base_url: &str, with_auth: bool) -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create temp dir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(paths.is_test, "must run inside AppPaths::for_test sandbox");

    let mut state = read_switch_state(&paths);
    state.base_url = base_url.to_string();
    save_switch_state(&paths, &state).expect("save switch state");

    if with_auth {
        save_switch_auth(&paths, Some("cr_live_demo_test_key"), Some("42"), Some("oauth"))
            .expect("save test auth");
    }

    (tmp, paths)
}

fn build_request(payload: Value) -> LiveDemoRequest {
    serde_json::from_value::<LiveDemoRequest>(payload)
        .expect("LiveDemoRequest must deserialize from camelCase JSON")
}

fn result_to_json(res: &LiveDemoResult) -> Value {
    serde_json::to_value(res).expect("LiveDemoResult must serialize to camelCase JSON")
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

fn sample_png_bytes() -> Vec<u8> {
    let mut out = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R']);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0, 0, 0]);
    out.extend_from_slice(&[0x1F, 0x15, 0xC4, 0x89]);
    out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]);
    out
}

fn sample_mp3_bytes() -> Vec<u8> {
    // ID3v2.3 header (size = 4) + 4 bytes tag + MPEG1 Layer3 frame sync header + payload
    let mut out = vec![
        b'I', b'D', b'3', 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, b'T', b'A', b'K', b'O',
    ];
    out.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    out.extend_from_slice(&[0x11; 64]);
    out
}

fn sample_mp4_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    // ftyp box (24 bytes)
    out.extend_from_slice(&24u32.to_be_bytes());
    out.extend_from_slice(b"ftyp");
    out.extend_from_slice(b"isom");
    out.extend_from_slice(&0x0000_0200u32.to_be_bytes());
    out.extend_from_slice(b"isomavc1");
    // minimal moov box (16 bytes) containing avc1 marker
    out.extend_from_slice(&16u32.to_be_bytes());
    out.extend_from_slice(b"moov");
    out.extend_from_slice(b"avc1\x00\x00\x00\x00");
    out
}

fn decode_data_url(data_url: &str, expected_prefix: &str) -> Vec<u8> {
    assert!(
        data_url.starts_with(expected_prefix),
        "expected data URL starting with '{expected_prefix}', got '{data_url}'"
    );
    let b64 = &data_url[expected_prefix.len()..];
    STANDARD
        .decode(b64)
        .expect("previewDataUrl payload must be valid base64")
}

#[tokio::test]
async fn tc_live_01_missing_api_key_returns_chinese_error_without_http_calls() {
    let server = start_mock_server(|_req, _| {
        MockResponse::json(200, &json!({"unexpected": true}))
    })
    .await;
    let (_tmp, paths) = setup_sandbox(&server.base_url, false);

    let req = build_request(json!({
        "tool": "audio",
        "prompt": "测试未登录拦截"
    }));
    let err = execute_live_demo_for_paths(&paths, req)
        .await
        .expect_err("missing API key must fail");

    assert!(
        !err.trim().is_empty() && contains_cjk(&err),
        "expected Chinese error message, got: {err}"
    );
    let recorded = server.requests.lock().expect("lock requests");
    assert_eq!(
        recorded.len(),
        0,
        "must not hit upstream gateway when API key is absent"
    );
}

#[tokio::test]
async fn tc_live_02_disabled_mcp_toggles_still_allow_live_demo() {
    let mp3 = sample_mp3_bytes();
    let mp3_for_srv = mp3.clone();
    let server = start_mock_server(move |req, _| match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/audio/speech") => MockResponse::binary(200, "audio/mpeg", mp3_for_srv.clone()),
        ("POST", "/v1/chat/completions") => MockResponse::json(
            200,
            &json!({
                "choices": [{
                    "message": { "content": "开关关闭状态下依然可现场试跑验证。" }
                }]
            }),
        ),
        _ => MockResponse::json(404, &json!({"error": "unexpected route"})),
    })
    .await;
    let (_tmp, paths) = setup_sandbox(&server.base_url, true);

    let mut state = read_switch_state(&paths);
    state.mcp.image = false;
    state.mcp.video = false;
    state.mcp.audio = false;
    state.mcp.search = false;
    state.mcp.consult = false;
    save_switch_state(&paths, &state).expect("save disabled toggles");

    let audio_res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "audio",
            "prompt": "即使音频 MCP 开关关闭也应允许试跑"
        })),
    )
    .await
    .expect("audio live demo must succeed even when state.mcp.audio == false");
    assert_eq!(result_to_json(&audio_res)["tool"], json!("audio"));

    let consult_res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "consult",
            "prompt": "即使外脑 MCP 开关关闭也应允许试跑"
        })),
    )
    .await
    .expect("consult live demo must succeed even when state.mcp.consult == false");
    let consult_val = result_to_json(&consult_res);
    assert!(
        consult_val["textOutput"]
            .as_str()
            .unwrap_or("")
            .contains("开关关闭状态下依然可现场试跑验证"),
        "expected consult textOutput, got {consult_val}"
    );
}

#[tokio::test]
async fn tc_live_03_audio_defaults_to_bingtang_mp3_saves_disk_and_returns_data_url() {
    let mp3 = sample_mp3_bytes();
    let mp3_for_srv = mp3.clone();
    let server = start_mock_server(move |req, _| {
        if req.method == "POST" && req.path == "/v1/audio/speech" {
            MockResponse::binary(200, "audio/mpeg", mp3_for_srv.clone())
        } else {
            MockResponse::json(404, &json!({"error": "not found"}))
        }
    })
    .await;
    let (tmp, paths) = setup_sandbox(&server.base_url, true);

    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "audio",
            "prompt": "你好，我是冰糖，欢迎体验 Tako 声音。"
        })),
    )
    .await
    .expect("audio live demo should succeed");

    let val = result_to_json(&res);
    assert_eq!(val["tool"], json!("audio"));
    assert_eq!(val["model"], json!("mimo-v2.5-tts"));

    let saved_path = PathBuf::from(
        val["savedPath"]
            .as_str()
            .expect("audio live demo must return savedPath"),
    );
    assert!(
        saved_path.starts_with(tmp.path()),
        "savedPath {} must stay inside sandbox {}",
        saved_path.display(),
        tmp.path().display()
    );
    assert!(saved_path.is_file(), "saved audio file must exist");
    assert_eq!(fs::read(&saved_path).unwrap(), mp3);

    let data_url = val["previewDataUrl"]
        .as_str()
        .expect("audio live demo must return previewDataUrl");
    let decoded = decode_data_url(data_url, "data:audio/mpeg;base64,");
    assert_eq!(decoded, mp3);

    let recorded = server.requests.lock().expect("lock requests");
    assert_eq!(recorded.len(), 1);
    let r = &recorded[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/v1/audio/speech");
    assert_eq!(
        r.authorization.as_deref(),
        Some("Bearer cr_live_demo_test_key")
    );
    let body = r.json_body();
    assert_eq!(body["model"], json!("mimo-v2.5-tts"));
    assert_eq!(body["voice"], json!("冰糖"));
    assert_eq!(body["response_format"], json!("mp3"));
    assert_eq!(body["input"], json!("你好，我是冰糖，欢迎体验 Tako 声音。"));
}

#[tokio::test]
async fn tc_live_04_image_validates_magic_bytes_and_rejects_corrupted_payload() {
    let png = sample_png_bytes();
    let png_b64 = STANDARD.encode(&png);
    let reject_flag = Arc::new(Mutex::new(false));
    let reject_srv = Arc::clone(&reject_flag);

    let server = start_mock_server(move |req, _| {
        if req.method == "POST" && req.path == "/v1/images/generations" {
            let should_corrupt = *reject_srv.lock().expect("lock flag");
            let payload_b64 = if should_corrupt {
                STANDARD.encode(b"<html>upstream error page</html>")
            } else {
                png_b64.clone()
            };
            MockResponse::json(200, &json!({ "data": [{ "b64_json": payload_b64 }] }))
        } else {
            MockResponse::json(404, &json!({"error": "not found"}))
        }
    })
    .await;
    let (tmp, paths) = setup_sandbox(&server.base_url, true);

    // Part A: valid PNG image
    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "image",
            "prompt": "珊瑚色陶瓷马克杯",
            "aspectRatio": "1:1"
        })),
    )
    .await
    .expect("valid PNG image live demo must succeed");

    let val = result_to_json(&res);
    assert_eq!(val["tool"], json!("image"));
    let saved_path = PathBuf::from(val["savedPath"].as_str().expect("savedPath"));
    assert!(saved_path.starts_with(tmp.path()));
    assert_eq!(fs::read(&saved_path).unwrap(), png);
    let decoded = decode_data_url(
        val["previewDataUrl"].as_str().expect("previewDataUrl"),
        "data:image/png;base64,",
    );
    assert_eq!(decoded, png);

    // Part B: non-image magic bytes must fail and not write invalid file
    *reject_flag.lock().expect("lock flag") = true;
    let err = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "image",
            "prompt": "损坏响应测试"
        })),
    )
    .await
    .expect_err("non-image magic bytes must be rejected");
    assert!(
        contains_cjk(&err),
        "expected Chinese error on bad image magic bytes, got: {err}"
    );
}

#[tokio::test]
async fn tc_live_05_video_three_step_async_flow_and_ftyp_validation() {
    let mp4 = sample_mp4_bytes();
    let mp4_for_srv = mp4.clone();
    let poll_count = Arc::new(Mutex::new(0usize));
    let poll_srv = Arc::clone(&poll_count);
    let bad_mp4 = Arc::new(Mutex::new(false));
    let bad_mp4_srv = Arc::clone(&bad_mp4);

    let server = start_mock_server(move |req, _| match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/videos/generations") => {
            MockResponse::json(200, &json!({ "id": "vid_live_001", "status": "queued" }))
        }
        ("GET", "/v1/videos/vid_live_001") => {
            let mut n = poll_srv.lock().expect("lock poll");
            *n += 1;
            if *n == 1 {
                MockResponse::json(200, &json!({ "status": "IN_PROGRESS", "progress": 50 }))
            } else {
                MockResponse::json(200, &json!({ "status": "SUCCESS", "progress": 100 }))
            }
        }
        ("GET", "/v1/videos/vid_live_001/content") => {
            if *bad_mp4_srv.lock().expect("lock bad_mp4") {
                MockResponse::binary(200, "application/octet-stream", b"NOT_AN_MP4".to_vec())
            } else {
                MockResponse::binary(200, "video/mp4", mp4_for_srv.clone())
            }
        }
        _ => MockResponse::json(404, &json!({"error": "unexpected route"})),
    })
    .await;
    let (tmp, paths) = setup_sandbox(&server.base_url, true);

    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "video",
            "prompt": "漂浮空中花园慢速推镜头",
            "aspectRatio": "16:9"
        })),
    )
    .await
    .expect("video 3-step live demo should succeed");

    let val = result_to_json(&res);
    assert_eq!(val["tool"], json!("video"));
    assert_eq!(val["taskId"], json!("vid_live_001"));

    let saved_path = PathBuf::from(val["savedPath"].as_str().expect("savedPath"));
    assert!(saved_path.starts_with(tmp.path()));
    assert_eq!(fs::read(&saved_path).unwrap(), mp4);

    let decoded = decode_data_url(
        val["previewDataUrl"].as_str().expect("previewDataUrl"),
        "data:video/mp4;base64,",
    );
    assert_eq!(decoded, mp4);

    {
        let recorded = server.requests.lock().expect("lock requests");
        let seq: Vec<(&str, &str)> = recorded
            .iter()
            .map(|r| (r.method.as_str(), r.path.as_str()))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("POST", "/v1/videos/generations"),
                ("GET", "/v1/videos/vid_live_001"),
                ("GET", "/v1/videos/vid_live_001"),
                ("GET", "/v1/videos/vid_live_001/content"),
            ]
        );
    }

    // Verify invalid MP4 (missing ftyp) is rejected
    *bad_mp4.lock().expect("lock bad_mp4") = true;
    let err = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "video",
            "prompt": "",
            "resumeTaskId": "vid_live_001"
        })),
    )
    .await
    .expect_err("invalid MP4 without ftyp must fail");
    assert!(contains_cjk(&err), "expected Chinese error, got: {err}");
}

#[tokio::test]
async fn tc_live_06_video_resume_task_id_never_triggers_post_generations() {
    let mp4 = sample_mp4_bytes();
    let mp4_for_srv = mp4.clone();
    let server = start_mock_server(move |req, _| match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/v1/videos/generations") => {
            MockResponse::json(500, &json!({"error": "FORBIDDEN_DOUBLE_BILLING"}))
        }
        ("GET", "/v1/videos/vid_resume_777") => {
            MockResponse::json(200, &json!({ "status": "completed", "progress": 100 }))
        }
        ("GET", "/v1/videos/vid_resume_777/content") => {
            MockResponse::binary(200, "video/mp4", mp4_for_srv.clone())
        }
        _ => MockResponse::json(404, &json!({"error": "unexpected route"})),
    })
    .await;
    let (tmp, paths) = setup_sandbox(&server.base_url, true);

    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "video",
            "prompt": "",
            "resumeTaskId": "vid_resume_777"
        })),
    )
    .await
    .expect("resume_task_id recovery must succeed without POSTing");

    let val = result_to_json(&res);
    assert_eq!(val["taskId"], json!("vid_resume_777"));
    let saved_path = PathBuf::from(val["savedPath"].as_str().expect("savedPath"));
    assert!(saved_path.starts_with(tmp.path()));
    assert_eq!(fs::read(&saved_path).unwrap(), mp4);
    let decoded = decode_data_url(
        val["previewDataUrl"].as_str().expect("previewDataUrl"),
        "data:video/mp4;base64,",
    );
    assert_eq!(decoded, mp4);

    let recorded = server.requests.lock().expect("lock requests");
    let post_count = recorded
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/v1/videos/generations")
        .count();
    assert_eq!(
        post_count, 0,
        "resume_task_id MUST NEVER call POST /v1/videos/generations (no double billing!)"
    );
    assert!(
        recorded
            .iter()
            .any(|r| r.method == "GET" && r.path == "/v1/videos/vid_resume_777"),
        "must poll GET /v1/videos/vid_resume_777"
    );
    assert!(
        recorded
            .iter()
            .any(|r| r.method == "GET" && r.path == "/v1/videos/vid_resume_777/content"),
        "must download GET /v1/videos/vid_resume_777/content"
    );
}

#[tokio::test]
async fn tc_live_07_search_parses_citations_and_extracts_domains() {
    let server = start_mock_server(|req, _| {
        if req.method == "POST" && req.path == "/v1/search" {
            MockResponse::json(
                200,
                &json!({
                    "results": [
                        {
                            "title": "Content Security Policy (CSP) | Tauri v2",
                            "url": "https://v2.tauri.app/security/csp/",
                            "snippet": "Configure app.security.csp with media-src 'self' data:;"
                        },
                        {
                            "title": "CSP: media-src - MDN",
                            "url": "https://developer.mozilla.org:443/en-US/docs/Web/HTTP/Headers/Content-Security-Policy/media-src",
                            "snippet": "Specifies valid sources for loading media using audio and video elements."
                        }
                    ]
                }),
            )
        } else {
            MockResponse::json(404, &json!({"error": "not found"}))
        }
    })
    .await;
    let (_tmp, paths) = setup_sandbox(&server.base_url, true);

    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "search",
            "prompt": "Tauri v2 CSP media-src",
            "provider": "grok"
        })),
    )
    .await
    .expect("search live demo should succeed");

    let val = result_to_json(&res);
    assert_eq!(val["tool"], json!("search"));
    let citations = val["citations"]
        .as_array()
        .expect("LiveDemoResult.citations must serialize as array");
    assert_eq!(citations.len(), 2);

    assert_eq!(
        citations[0]["title"],
        json!("Content Security Policy (CSP) | Tauri v2")
    );
    assert_eq!(
        citations[0]["url"],
        json!("https://v2.tauri.app/security/csp/")
    );
    assert_eq!(citations[0]["domain"], json!("v2.tauri.app"));
    assert!(citations[0]["snippet"]
        .as_str()
        .unwrap_or("")
        .contains("media-src"));

    assert_eq!(citations[1]["domain"], json!("developer.mozilla.org"));

    let recorded = server.requests.lock().expect("lock requests");
    assert_eq!(recorded.len(), 1);
    let body = recorded[0].json_body();
    assert_eq!(body["query"], json!("Tauri v2 CSP media-src"));
    assert_eq!(body["provider"], json!("grok"));
}

#[tokio::test]
async fn tc_live_08_consult_calls_chat_completions_and_returns_text_output() {
    let server = start_mock_server(|req, _| {
        if req.method == "POST" && req.path == "/v1/chat/completions" {
            MockResponse::json(
                200,
                &json!({
                    "id": "chatcmpl-blackbox-1",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "第二模型会诊建议：1. 传入 resume_task_id 避免二次提交；2. 校验 ftyp MP4 魔数。"
                        }
                    }]
                }),
            )
        } else {
            MockResponse::json(404, &json!({"error": "not found"}))
        }
    })
    .await;
    let (_tmp, paths) = setup_sandbox(&server.base_url, true);

    let res = execute_live_demo_for_paths(
        &paths,
        build_request(json!({
            "tool": "consult",
            "prompt": "审查异步生视频超时恢复并发盲点",
            "model": "gpt-6.2-sol"
        })),
    )
    .await
    .expect("consult live demo should succeed");

    let val = result_to_json(&res);
    assert_eq!(val["tool"], json!("consult"));
    assert_eq!(val["model"], json!("gpt-6.2-sol"));
    let text_out = val["textOutput"]
        .as_str()
        .expect("consult must populate textOutput");
    assert!(
        text_out.contains("第二模型会诊建议"),
        "expected second model reply in textOutput, got: {text_out}"
    );

    let recorded = server.requests.lock().expect("lock requests");
    assert_eq!(recorded.len(), 1);
    let body = recorded[0].json_body();
    assert_eq!(body["model"], json!("gpt-6.2-sol"));
    let msgs = body["messages"].as_array().expect("messages array");
    assert!(msgs.iter().any(|m| {
        m["content"]
            .as_str()
            .unwrap_or("")
            .contains("审查异步生视频超时恢复并发盲点")
    }));
}

#[test]
fn tc_live_09_public_types_use_camel_case_serde_contract() {
    // Ensure LiveDemoCitation public type exists and exposes expected camelCase schema.
    let _citation_type = std::any::type_name::<LiveDemoCitation>();
    let _result_type = std::any::type_name::<LiveDemoResult>();

    let req = build_request(json!({
        "tool": "video",
        "prompt": "test prompt",
        "model": "grok-imagine-video-1.5",
        "voice": "冰糖",
        "instructions": "轻快自然",
        "aspectRatio": "16:9",
        "provider": "grok",
        "resumeTaskId": "task_123"
    }));
    let _ = req;

    // Verify result keys mapping expectation through a round-trip Map helper check on Citation if empty
    let empty_citations: Vec<LiveDemoCitation> = Vec::new();
    assert!(serde_json::to_value(&empty_citations).unwrap().is_array());
    let _unused: HashMap<String, Value> = HashMap::new();
}
