use crate::adapters::{snapshot_app, AppId, UiPatchStatus};
use crate::config::{self, AppPaths, DEFAULT_TELEMETRY_URL};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_IDENTICAL_ERROR_REPORTS_PER_SESSION: u32 = 5;
const TELEMETRY_HTTP_TIMEOUT_SECS: u64 = 5;
const FEEDBACK_HTTP_TIMEOUT_SECS: u64 = 12;

static ERROR_REPORT_COUNTS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
static PANIC_HOOK_INSTALLED: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendErrorInput {
    pub source: String,
    #[serde(default)]
    pub category: String,
    pub error_type: String,
    pub message: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub stack: String,
    #[serde(default)]
    pub context: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSubmission {
    pub category: String,
    #[serde(default)]
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub contact: String,
    #[serde(default = "default_true")]
    pub include_diagnostics: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackReceipt {
    pub ok: bool,
    pub id: i64,
}

fn default_true() -> bool {
    true
}

pub fn resolve_telemetry_base_url() -> String {
    if let Ok(raw) = std::env::var("TAKO_TELEMETRY_URL") {
        let trimmed = raw.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    DEFAULT_TELEMETRY_URL.trim_end_matches('/').to_string()
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn is_path_segment_char(b: u8) -> bool {
    !matches!(
        b,
        b'/' | b'\\' | b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b',' | b';' | b':' | b')' | b']' | b'}'
    )
}

fn replace_prefixed_token(input: &str, prefix: &str, min_rest: usize, replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let pbytes = prefix.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + pbytes.len() <= bytes.len()
            && &bytes[i..i + pbytes.len()] == pbytes
            && (i == 0 || !is_token_char(bytes[i - 1]))
        {
            let mut j = i + pbytes.len();
            while j < bytes.len() && is_token_char(bytes[j]) {
                j += 1;
            }
            if j - (i + pbytes.len()) >= min_rest {
                out.push_str(replacement);
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_bearer_tokens(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let bytes = input.as_bytes();
    let lbytes = lower.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 7 <= lbytes.len() && &lbytes[i..i + 7] == b"bearer " {
            let mut j = i + 7;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let tok_start = j;
            while j < bytes.len() && is_token_char(bytes[j]) {
                j += 1;
            }
            if j - tok_start >= 6 {
                out.push_str("Bearer <redacted>");
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_jwts(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 3 <= bytes.len()
            && &bytes[i..i + 3] == b"eyJ"
            && (i == 0 || !is_token_char(bytes[i - 1]))
        {
            let mut j = i;
            let mut dots = 0usize;
            while j < bytes.len() && is_token_char(bytes[j]) {
                if bytes[j] == b'.' {
                    dots += 1;
                }
                j += 1;
            }
            if dots == 2 && j - i >= 24 {
                out.push_str("<redacted-jwt>");
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_unix_home(input: &str, prefix: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let pbytes = prefix.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + pbytes.len() <= bytes.len() && &bytes[i..i + pbytes.len()] == pbytes {
            let mut j = i + pbytes.len();
            while j < bytes.len() && is_path_segment_char(bytes[j]) {
                j += 1;
            }
            if j > i + pbytes.len() {
                out.push_str(replacement);
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_windows_home(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 9 <= bytes.len()
            && bytes[i].is_ascii_alphabetic()
            && bytes[i + 1] == b':'
            && matches!(bytes[i + 2], b'\\' | b'/')
            && bytes[i + 3..i + 8].eq_ignore_ascii_case(b"users")
            && matches!(bytes[i + 8], b'\\' | b'/')
        {
            let mut j = i + 9;
            while j < bytes.len() && is_path_segment_char(bytes[j]) {
                j += 1;
            }
            if j > i + 9 {
                let drive = bytes[i] as char;
                let sep = bytes[i + 2] as char;
                out.push(drive);
                out.push(':');
                out.push(sep);
                out.push_str("Users");
                out.push(sep);
                out.push_str("<redacted>");
                i = j;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_kv_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for needle in ["api_key=", "apikey=", "token=", "secret=", "password=", "edgeone_pages_api_token="] {
        let lower = out.to_ascii_lowercase();
        if !lower.contains(needle) {
            continue;
        }
        let bytes = out.as_bytes();
        let lbytes = lower.as_bytes();
        let nbytes = needle.as_bytes();
        let mut rebuilt = String::with_capacity(out.len());
        let mut i = 0;
        while i < bytes.len() {
            if i + nbytes.len() <= lbytes.len() && &lbytes[i..i + nbytes.len()] == nbytes {
                rebuilt.push_str(&out[i..i + nbytes.len()]);
                let mut j = i + nbytes.len();
                while j < bytes.len()
                    && !matches!(bytes[j], b'&' | b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b',' | b';')
                {
                    j += 1;
                }
                rebuilt.push_str("<redacted>");
                i = j;
                continue;
            }
            let ch = out[i..].chars().next().unwrap();
            rebuilt.push(ch);
            i += ch.len_utf8();
        }
        out = rebuilt;
    }
    out
}

pub fn sanitize_text(input: &str, max_len: usize) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() || max_len == 0 {
        return String::new();
    }
    let mut s = redact_bearer_tokens(trimmed);
    s = redact_jwts(&s);
    s = replace_prefixed_token(&s, "sk-", 8, "<redacted-key>");
    s = replace_prefixed_token(&s, "cr_", 8, "<redacted-key>");
    s = replace_prefixed_token(&s, "eo_", 6, "<redacted-key>");
    s = replace_prefixed_token(&s, "ghp_", 10, "<redacted-key>");
    s = replace_prefixed_token(&s, "github_pat_", 10, "<redacted-key>");
    s = redact_kv_secrets(&s);
    s = redact_unix_home(&s, "/Users/", "/Users/<redacted>");
    s = redact_unix_home(&s, "/home/", "/home/<redacted>");
    s = redact_windows_home(&s);

    if s.chars().count() <= max_len {
        s
    } else {
        s.chars().take(max_len).collect()
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "key",
        "token",
        "secret",
        "pass",
        "cookie",
        "auth",
        "credential",
        "authorization",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn sanitize_json_value(value: serde_json::Value) -> serde_json::Value {
    sanitize_json_depth(value, 0)
}

fn sanitize_json_depth(value: serde_json::Value, depth: usize) -> serde_json::Value {
    if depth >= 6 {
        return serde_json::Value::String("<truncated>".to_string());
    }
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(sanitize_text(&s, 512)),
        serde_json::Value::Array(arr) => serde_json::Value::Array(
            arr.into_iter()
                .take(48)
                .map(|v| sanitize_json_depth(v, depth + 1))
                .collect(),
        ),
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map.into_iter().take(48) {
                if is_sensitive_key(&k) {
                    out.insert(k, serde_json::Value::String("<redacted>".to_string()));
                } else {
                    out.insert(k, sanitize_json_depth(v, depth + 1));
                }
            }
            serde_json::Value::Object(out)
        }
        other => other,
    }
}

fn compute_user_id_hash(paths: &AppPaths) -> String {
    let Some(api_id) = config::read_main_api_id(paths) else {
        return String::new();
    };
    let digest = Sha256::digest(format!("tako-uid:{api_id}").as_bytes());
    hex::encode(&digest[..8])
}

pub fn enabled_mcp_names(state: &config::SwitchState) -> Vec<&'static str> {
    let mut out = Vec::new();
    if state.mcp.image {
        out.push("image");
    }
    if state.mcp.video {
        out.push("video");
    }
    if state.mcp.audio {
        out.push("audio");
    }
    if state.mcp.search {
        out.push("search");
    }
    if state.mcp.consult {
        out.push("consult");
    }
    if state.mcp.web_page {
        out.push("webPage");
    }
    out
}

pub fn build_sanitized_diagnostics(paths: &AppPaths) -> serde_json::Value {
    let state = config::read_switch_state(paths);
    let logged_in = config::read_main_api_key(paths).is_some();
    let device_id = config::read_or_create_device_id(paths);
    let local_gateway_healthy = if paths.is_test {
        false
    } else {
        crate::agy_proxy::is_proxy_healthy(crate::agy_proxy::LOCAL_GATEWAY_PORT)
    };

    let apps: Vec<serde_json::Value> = AppId::all()
        .iter()
        .map(|&app_id| {
            let snap = snapshot_app(paths, app_id);
            let issues: Vec<String> = snap
                .diagnostic
                .issues
                .iter()
                .map(|item| sanitize_text(item, 240))
                .collect();
            serde_json::json!({
                "id": snap.id.as_str(),
                "mode": snap.mode,
                "installed": snap.install.installed,
                "version": snap.install.version,
                "running": snap.running,
                "uiPatchStatus": snap.ui_patch.status,
                "uiPatchVersion": snap.ui_patch.patch_version,
                "diagnosticOk": snap.diagnostic.ok,
                "diagnosticIssues": issues,
            })
        })
        .collect();

    let raw = serde_json::json!({
        "appVersion": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "deviceId": device_id,
        "localGatewayHealthy": local_gateway_healthy,
        "preferChineseUi": state.prefer_chinese_ui,
        "bypassSystemProxy": state.bypass_system_proxy,
        "autoCheckUpdates": state.auto_check_updates,
        "telemetryEnabled": state.telemetry_enabled,
        "loggedIn": logged_in,
        "enabledMcps": {
            "image": state.mcp.image,
            "video": state.mcp.video,
            "audio": state.mcp.audio,
            "search": state.mcp.search,
            "consult": state.mcp.consult,
            "webPage": state.mcp.web_page,
        },
        "apps": apps,
    });

    sanitize_json_value(raw)
}

pub(crate) fn telemetry_allowed(paths: &AppPaths) -> bool {
    !paths.is_test && config::read_switch_state(paths).telemetry_enabled
}

fn build_http_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(timeout_secs.min(4)))
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建网络客户端失败: {e}"))
}

fn spawn_fire_and_forget(url: String, payload: serde_json::Value) {
    let task = async move {
        let Ok(client) = build_http_client(TELEMETRY_HTTP_TIMEOUT_SECS) else {
            return;
        };
        let _ = client.post(&url).json(&payload).send().await;
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(task);
    } else {
        std::thread::spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(task);
            }
        });
    }
}

fn build_event_envelope(
    paths: &AppPaths,
    event_name: &str,
    target_app: &str,
    ok: bool,
    duration_ms: u64,
    props: serde_json::Value,
) -> serde_json::Value {
    let device_id = config::read_or_create_device_id(paths);
    let uid_hash = compute_user_id_hash(paths);
    let clean_props = match sanitize_json_value(props) {
        serde_json::Value::Object(m) => serde_json::Value::Object(m),
        _ => serde_json::json!({}),
    };

    serde_json::json!({
        "device_id": device_id,
        "app_name": "tako-switch",
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "user_id_hash": uid_hash,
        "events": [
            {
                "event_name": sanitize_text(event_name, 64),
                "target_app": sanitize_text(target_app, 32),
                "ok": ok,
                "duration_ms": duration_ms,
                "event_time": now_unix(),
                "props": clean_props,
            }
        ]
    })
}

pub fn track_event(
    paths: &AppPaths,
    event_name: &str,
    target_app: &str,
    ok: bool,
    duration_ms: u64,
    props: serde_json::Value,
) {
    if !telemetry_allowed(paths) {
        return;
    }
    let payload = build_event_envelope(paths, event_name, target_app, ok, duration_ms, props);
    let url = format!("{}/v1/events", resolve_telemetry_base_url());
    spawn_fire_and_forget(url, payload);
}

pub fn track_launch_heartbeat(paths: &AppPaths) {
    if !telemetry_allowed(paths) {
        return;
    }
    let state = config::read_switch_state(paths);
    let logged_in = config::read_main_api_key(paths).is_some();

    let mut installed_clients: Vec<&'static str> = Vec::new();
    let mut patched_clients: Vec<&'static str> = Vec::new();
    for &app_id in AppId::all() {
        let snap = snapshot_app(paths, app_id);
        if snap.install.installed {
            installed_clients.push(app_id.as_str());
        }
        if snap.ui_patch.status == UiPatchStatus::Patched {
            patched_clients.push(app_id.as_str());
        }
    }

    let enabled_mcps = enabled_mcp_names(&state);
    track_event(
        paths,
        "app_launch",
        "tako-switch",
        true,
        0,
        serde_json::json!({
            "installed_clients": installed_clients,
            "patched_clients": patched_clients,
            "enabled_mcps": enabled_mcps,
            "logged_in": logged_in,
            "prefer_chinese_ui": state.prefer_chinese_ui,
        }),
    );
}

fn should_send_error_fingerprint(fingerprint: &str) -> bool {
    let map = ERROR_REPORT_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut guard) = map.lock() else {
        return true;
    };
    if guard.len() > 256 && !guard.contains_key(fingerprint) {
        return false;
    }
    let entry = guard.entry(fingerprint.to_string()).or_insert(0);
    if *entry >= MAX_IDENTICAL_ERROR_REPORTS_PER_SESSION {
        return false;
    }
    *entry += 1;
    true
}

pub struct ErrorReport<'a> {
    pub source: &'a str,
    pub category: &'a str,
    pub error_type: &'a str,
    pub message: &'a str,
    pub location: &'a str,
    pub stack: &'a str,
    pub context: serde_json::Value,
}

fn build_error_envelope(paths: &AppPaths, report: &ErrorReport<'_>) -> serde_json::Value {
    let device_id = config::read_or_create_device_id(paths);
    let clean_ctx = match sanitize_json_value(report.context.clone()) {
        serde_json::Value::Object(m) => serde_json::Value::Object(m),
        _ => serde_json::json!({}),
    };

    serde_json::json!({
        "device_id": device_id,
        "app_name": "tako-switch",
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "errors": [
            {
                "source": sanitize_text(report.source, 32),
                "category": sanitize_text(report.category, 32),
                "error_type": sanitize_text(report.error_type, 80),
                "message": sanitize_text(report.message, 600),
                "location": sanitize_text(report.location, 180),
                "stack": sanitize_text(report.stack, 3000),
                "context": clean_ctx,
            }
        ]
    })
}

pub fn report_error(paths: &AppPaths, report: ErrorReport<'_>) {
    if !telemetry_allowed(paths) {
        return;
    }
    let fp = format!(
        "{}|{}|{}|{}",
        report.source.trim(),
        report.category.trim(),
        report.error_type.trim(),
        if report.location.trim().is_empty() {
            sanitize_text(report.message, 64)
        } else {
            sanitize_text(report.location, 120)
        }
    );
    if !should_send_error_fingerprint(&fp) {
        return;
    }

    let payload = build_error_envelope(paths, &report);
    let url = format!("{}/v1/errors", resolve_telemetry_base_url());
    spawn_fire_and_forget(url, payload);
}

pub fn report_frontend_error(paths: &AppPaths, input: FrontendErrorInput) {
    let source = match input.source.trim() {
        "frontend_error" | "unhandled_rejection" | "tauri_invoke" => input.source.trim(),
        _ => "frontend_error",
    };
    let category = if input.category.trim().is_empty() {
        "ui"
    } else {
        input.category.trim()
    };
    let err_type = if input.error_type.trim().is_empty() {
        "FrontendError"
    } else {
        input.error_type.trim()
    };
    report_error(
        paths,
        ErrorReport {
            source,
            category,
            error_type: err_type,
            message: &input.message,
            location: &input.location,
            stack: &input.stack,
            context: input.context,
        },
    );
}

pub fn install_panic_hook() {
    PANIC_HOOK_INSTALLED.get_or_init(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Rust panic".to_string()
            };
            let loc = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();

            if let Ok(paths) = AppPaths::detect() {
                if telemetry_allowed(&paths) {
                    let payload = build_error_envelope(
                        &paths,
                        &ErrorReport {
                            source: "rust_panic",
                            category: "general",
                            error_type: "Panic",
                            message: &msg,
                            location: &loc,
                            stack: "",
                            context: serde_json::json!({}),
                        },
                    );
                    let url = format!("{}/v1/errors", resolve_telemetry_base_url());
                    let handle = std::thread::spawn(move || {
                        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            rt.block_on(async move {
                                if let Ok(client) = build_http_client(2) {
                                    let _ = client.post(&url).json(&payload).send().await;
                                }
                            });
                        }
                    });
                    // Wait briefly so panic report can flush without hanging shutdown.
                    let deadline = std::time::Instant::now() + Duration::from_millis(1500);
                    while !handle.is_finished() && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
            }

            prev(info);
        }));
    });
}

pub async fn submit_feedback(
    paths: &AppPaths,
    input: FeedbackSubmission,
) -> Result<FeedbackReceipt, String> {
    let base = resolve_telemetry_base_url();
    submit_feedback_to_url(paths, &base, input).await
}

pub(crate) async fn submit_feedback_to_url(
    paths: &AppPaths,
    base_url: &str,
    input: FeedbackSubmission,
) -> Result<FeedbackReceipt, String> {
    let desc = sanitize_text(&input.description, 4000);
    if desc.is_empty() {
        return Err("请填写问题描述后再提交".to_string());
    }
    let category = match input.category.trim().to_ascii_lowercase().as_str() {
        "bug" | "install" | "i18n" | "mcp" | "feature" | "other" => {
            input.category.trim().to_ascii_lowercase()
        }
        _ => "other".to_string(),
    };
    let diagnostics = if input.include_diagnostics {
        build_sanitized_diagnostics(paths)
    } else {
        serde_json::json!({})
    };

    let payload = serde_json::json!({
        "device_id": config::read_or_create_device_id(paths),
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "category": category,
        "title": sanitize_text(&input.title, 160),
        "description": desc,
        "contact": sanitize_text(&input.contact, 120),
        "diagnostics": diagnostics,
    });

    let client = build_http_client(FEEDBACK_HTTP_TIMEOUT_SECS)?;
    let url = format!("{}/v1/feedback", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("无法连接反馈服务，请稍后重试或前往 GitHub Issues ({e})"))?;

    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "提交反馈失败 (HTTP {}): {}",
            status.as_u16(),
            sanitize_text(&body_text, 200)
        ));
    }

    let parsed: FeedbackReceipt = serde_json::from_str(&body_text)
        .map_err(|e| format!("解析反馈回执失败: {e}"))?;
    Ok(parsed)
}

#[cfg(test)]
pub(crate) async fn post_event_to_url(
    paths: &AppPaths,
    base_url: &str,
    event_name: &str,
    target_app: &str,
    ok: bool,
    duration_ms: u64,
    props: serde_json::Value,
) -> Result<u16, String> {
    let payload = build_event_envelope(paths, event_name, target_app, ok, duration_ms, props);
    let client = build_http_client(TELEMETRY_HTTP_TIMEOUT_SECS)?;
    let url = format!("{}/v1/events", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

#[cfg(test)]
pub(crate) async fn post_error_to_url(
    paths: &AppPaths,
    base_url: &str,
    report: ErrorReport<'_>,
) -> Result<u16, String> {
    let payload = build_error_envelope(paths, &report);
    let client = build_http_client(TELEMETRY_HTTP_TIMEOUT_SECS)?;
    let url = format!("{}/v1/errors", base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().as_u16())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn sanitizer_redacts_secrets_jwts_and_user_paths() {
        let raw = "auth failed Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.sig123456789 with sk-supersecretkey123456 and eo_live_token999 at /Users/shiroha/Library/App and C:\\Users\\Alice\\AppData\\Local";
        let clean = sanitize_text(raw, 500);
        for forbidden in [
            "sk-supersecretkey123456",
            "eo_live_token999",
            "eyJhbGciOiJIUzI1NiJ9",
            "/Users/shiroha",
            "C:\\Users\\Alice",
        ] {
            assert!(
                !clean.contains(forbidden),
                "expected {forbidden} redacted in: {clean}"
            );
        }
        assert!(clean.contains("/Users/<redacted>"));
        assert!(clean.contains("C:\\Users\\<redacted>"));
    }

    #[test]
    fn diagnostics_never_leaks_api_keys_edgeone_tokens_or_usernames() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        config::save_switch_auth(
            &paths,
            Some("sk-secretmainkey1234567890"),
            Some("42"),
            Some("oauth"),
        )
        .unwrap();
        let mut st = config::read_switch_state(&paths);
        st.agy_api_key = Some("sk-secretagykey9876543210".to_string());
        st.mcp.web_page = true;
        st.mcp.edgeone_api_token = Some("eo_topsecret_token_999".to_string());
        st.mcp.edgeone_project_name = Some("private-proj".to_string());
        st.mcp.output_dir = "/Users/alice_private/Downloads/Tako".to_string();
        config::save_switch_state(&paths, &st).unwrap();

        let diag = build_sanitized_diagnostics(&paths);
        let serialized = serde_json::to_string(&diag).unwrap();

        for secret in [
            "sk-secretmainkey1234567890",
            "sk-secretagykey9876543210",
            "eo_topsecret_token_999",
            "private-proj",
            "alice_private",
        ] {
            assert!(
                !serialized.contains(secret),
                "diagnostics leaked sensitive data ({secret}): {serialized}"
            );
        }
        assert_eq!(diag["loggedIn"], serde_json::json!(true));
        assert_eq!(diag["enabledMcps"]["webPage"], serde_json::json!(true));
    }

    async fn spawn_capture_server(status_line: &'static str, response_body: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx.send(req_str);
                let resp = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (format!("http://{}", addr), rx)
    }

    #[tokio::test]
    async fn events_errors_and_feedback_post_clean_payloads() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let (ev_url, ev_rx) = spawn_capture_server("202 Accepted", r#"{"ok":true,"accepted":1}"#).await;
        let code = post_event_to_url(
            &paths,
            &ev_url,
            "client_switch",
            "claude",
            true,
            120,
            serde_json::json!({"mode": "tako", "api_key": "sk-mustredact123456"}),
        )
        .await
        .unwrap();
        assert_eq!(code, 202);
        let ev_req = ev_rx.await.unwrap();
        assert!(ev_req.starts_with("POST /v1/events "));
        assert!(ev_req.contains("client_switch"));
        assert!(!ev_req.contains("sk-mustredact123456"));

        let (err_url, err_rx) = spawn_capture_server("202 Accepted", r#"{"ok":true,"accepted":1}"#).await;
        let code = post_error_to_url(
            &paths,
            &err_url,
            ErrorReport {
                source: "rust_error",
                category: "asar_patch",
                error_type: "PatchFailed",
                message: "failed at /Users/bob/Claude.app with sk-hidden987654321",
                location: "claude_i18n.rs:100",
                stack: "",
                context: serde_json::json!({"token": "eo_secret_12345"}),
            },
        )
        .await
        .unwrap();
        assert_eq!(code, 202);
        let err_req = err_rx.await.unwrap();
        assert!(err_req.starts_with("POST /v1/errors "));
        assert!(!err_req.contains("/Users/bob"));
        assert!(!err_req.contains("sk-hidden987654321"));
        assert!(!err_req.contains("eo_secret_12345"));

        let (fb_url, fb_rx) = spawn_capture_server("201 Created", r#"{"ok":true,"id":77}"#).await;
        let receipt = submit_feedback_to_url(
            &paths,
            &fb_url,
            FeedbackSubmission {
                category: "i18n".into(),
                title: "漏翻反馈".into(),
                description: "路径 /Users/bob/app 未汉化".into(),
                contact: "qq:10001".into(),
                include_diagnostics: true,
            },
        )
        .await
        .unwrap();
        assert!(receipt.ok);
        assert_eq!(receipt.id, 77);
        let fb_req = fb_rx.await.unwrap();
        assert!(fb_req.starts_with("POST /v1/feedback "));
        assert!(!fb_req.contains("/Users/bob"));
    }
}
