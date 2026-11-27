use crate::config::{
    activate_saved_key, deactivate_current_auth, read_active_saved_key, read_main_api_id,
    read_main_api_key, read_switch_auth, read_switch_state, remove_saved_key_entry,
    save_switch_auth, update_saved_key_alias, update_saved_key_metadata, upsert_saved_key,
    write_atomic, AppPaths, SavedApiKey, UpsertSavedKeyInput,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const TAKO_QUOTA_PER_USD: f64 = 500_000.0;
pub const DESKTOP_OAUTH_CLIENT_ID: &str = "takoswitch";

pub fn quota_units_to_usd(units: i64) -> f64 {
    (units as f64) / TAKO_QUOTA_PER_USD
}

pub fn format_usd(usd: f64) -> String {
    format!("${usd:.2}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSummary {
    pub window_cost_limit: f64,
    pub window_used: f64,
    pub window_minutes: f64,
    pub daily_cost_limit: f64,
    pub daily_used: f64,
    pub weekly_cost_limit: f64,
    pub weekly_used: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SavedKeySummary {
    pub id: String,
    pub masked_key: String,
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    pub display_label: String,
    #[serde(default)]
    pub auth_source: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub logged_in: bool,
    pub base_url: String,
    pub masked_key: Option<String>,
    pub user_name: Option<String>,
    #[serde(default)]
    pub key_name: Option<String>,
    #[serde(default)]
    pub active_key_id: Option<String>,
    #[serde(default)]
    pub active_key_alias: Option<String>,
    #[serde(default)]
    pub display_label: Option<String>,
    #[serde(default)]
    pub saved_keys: Vec<SavedKeySummary>,
    pub plan_name: Option<String>,
    pub wallet_remain: Option<i64>,
    pub wallet_used: Option<i64>,
    #[serde(default)]
    pub wallet_remain_usd: Option<f64>,
    #[serde(default)]
    pub wallet_used_usd: Option<f64>,
    pub quota: Option<QuotaSummary>,
    pub error: Option<String>,
}

pub fn format_key_display_label(
    user_name: Option<&str>,
    key_name: Option<&str>,
    masked_key: &str,
) -> String {
    let user = user_name.map(str::trim).filter(|s| !s.is_empty());
    let key = key_name.map(str::trim).filter(|s| !s.is_empty());
    match (user, key) {
        (Some(u), Some(k)) => format!("{u} - {k}"),
        (Some(u), None) => format!("{u} - {masked_key}"),
        (None, Some(k)) => k.to_string(),
        (None, None) => masked_key.to_string(),
    }
}

pub fn summarize_saved_key(entry: &SavedApiKey, active_key_id: Option<&str>) -> SavedKeySummary {
    let masked = mask_api_key(&entry.api_key);
    let display_label = format_key_display_label(
        entry.user_name.as_deref(),
        entry.key_name.as_deref(),
        &masked,
    );
    SavedKeySummary {
        id: entry.id.clone(),
        masked_key: masked,
        user_name: entry.user_name.clone(),
        key_name: entry.key_name.clone(),
        alias: entry.alias.clone(),
        display_label,
        auth_source: entry.auth_source.clone(),
        active: active_key_id == Some(entry.id.as_str()),
    }
}

pub fn list_saved_key_summaries(paths: &AppPaths) -> Vec<SavedKeySummary> {
    let auth = read_switch_auth(paths);
    let active_id = auth.active_key_id.as_deref();
    auth.saved_keys
        .iter()
        .map(|entry| summarize_saved_key(entry, active_id))
        .collect()
}

pub fn build_local_account_status(
    paths: &AppPaths,
    cached: Option<AccountStatus>,
) -> AccountStatus {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let auth = read_switch_auth(paths);
    let active_id = auth.active_key_id.clone();
    let saved_keys: Vec<SavedKeySummary> = auth
        .saved_keys
        .iter()
        .map(|entry| summarize_saved_key(entry, active_id.as_deref()))
        .collect();

    let Some(key) = auth.api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return AccountStatus {
            logged_in: false,
            base_url,
            masked_key: None,
            user_name: None,
            key_name: None,
            active_key_id: None,
            active_key_alias: None,
            display_label: None,
            saved_keys,
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            wallet_remain_usd: None,
            wallet_used_usd: None,
            quota: None,
            error: None,
        };
    };

    let masked = mask_api_key(key);
    let active_entry = active_id
        .as_deref()
        .and_then(|id| auth.saved_keys.iter().find(|k| k.id == id));
    let matching_cache = cached.filter(|c| {
        c.logged_in
            && c.masked_key.as_deref() == Some(masked.as_str())
            && (c.active_key_id.is_none() || c.active_key_id == active_id)
    });

    if let Some(c) = matching_cache {
        let user_name = c
            .user_name
            .or_else(|| active_entry.and_then(|e| e.user_name.clone()));
        let key_name = c
            .key_name
            .or_else(|| active_entry.and_then(|e| e.key_name.clone()));
        let active_key_alias = active_entry
            .and_then(|e| e.alias.clone())
            .or(c.active_key_alias);
        let display_label = Some(format_key_display_label(
            user_name.as_deref(),
            key_name.as_deref(),
            &masked,
        ));
        AccountStatus {
            logged_in: true,
            base_url,
            masked_key: Some(masked),
            user_name,
            key_name,
            active_key_id: active_id,
            active_key_alias,
            display_label,
            saved_keys,
            plan_name: c.plan_name,
            wallet_remain: c.wallet_remain,
            wallet_used: c.wallet_used,
            wallet_remain_usd: c
                .wallet_remain_usd
                .or_else(|| c.wallet_remain.map(quota_units_to_usd)),
            wallet_used_usd: c
                .wallet_used_usd
                .or_else(|| c.wallet_used.map(quota_units_to_usd)),
            quota: c.quota,
            error: c.error,
        }
    } else {
        let user_name = active_entry.and_then(|e| e.user_name.clone());
        let key_name = active_entry.and_then(|e| e.key_name.clone());
        let active_key_alias = active_entry.and_then(|e| e.alias.clone());
        let display_label = Some(format_key_display_label(
            user_name.as_deref(),
            key_name.as_deref(),
            &masked,
        ));
        AccountStatus {
            logged_in: true,
            base_url,
            masked_key: Some(masked),
            user_name,
            key_name,
            active_key_id: active_id,
            active_key_alias,
            display_label,
            saved_keys,
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            wallet_remain_usd: None,
            wallet_used_usd: None,
            quota: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceCache {
    pub cached_at_unix: u64,
    pub status: AccountStatus,
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn read_balance_cache(paths: &AppPaths) -> Option<BalanceCache> {
    let raw = std::fs::read_to_string(paths.balance_cache_path()).ok()?;
    serde_json::from_str::<BalanceCache>(&raw).ok()
}

pub fn write_balance_cache(paths: &AppPaths, status: &AccountStatus) {
    let cache = BalanceCache {
        cached_at_unix: unix_now_secs(),
        status: status.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec_pretty(&cache) {
        let _ = write_atomic(&paths.balance_cache_path(), &bytes, true);
    }
}

pub fn clear_balance_cache(paths: &AppPaths) {
    let p = paths.balance_cache_path();
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

fn calc_pct(used: f64, limit: f64) -> u64 {
    if limit <= 0.0 || used <= 0.0 {
        return 0;
    }
    ((used / limit) * 100.0).round().clamp(0.0, 100.0) as u64
}

pub fn format_status_line_text(status: &AccountStatus, session_cost_usd: Option<f64>) -> String {
    if !status.logged_in {
        return "Tako · 未登录".to_string();
    }
    let mut parts = Vec::new();
    let label = status
        .plan_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Tako");
    parts.push(label.to_string());

    if let Some(rem) = status.wallet_remain_usd {
        parts.push(format!("余额 {}", format_usd(rem)));
    }
    if let Some(cost) = session_cost_usd.filter(|c| *c > 0.0001) {
        parts.push(format!("本次 {}", format_usd(cost)));
    }
    if let Some(q) = &status.quota {
        if q.window_cost_limit > 0.0 || q.weekly_cost_limit > 0.0 {
            let p5h = calc_pct(q.window_used, q.window_cost_limit);
            let p7d = calc_pct(q.weekly_used, q.weekly_cost_limit);
            parts.push(format!("5h {p5h}% / 7d {p7d}%"));
        }
    }
    parts.join(" · ")
}

pub async fn render_status_line(paths: &AppPaths, stdin_json: Option<&str>) -> String {
    let session_cost_usd = stdin_json
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.pointer("/cost/total_cost_usd").and_then(|c| c.as_f64()));

    if read_main_api_key(paths).is_none() {
        return "Tako · 未登录".to_string();
    }

    let cached = read_balance_cache(paths);
    if let Some(ref c) = cached {
        if unix_now_secs().saturating_sub(c.cached_at_unix) <= 30 {
            return format_status_line_text(&c.status, session_cost_usd);
        }
    }

    match tokio::time::timeout(Duration::from_secs(4), fetch_account_status(paths)).await {
        Ok(live) if live.logged_in && live.error.is_none() => {
            format_status_line_text(&live, session_cost_usd)
        }
        Ok(live) => match cached {
            Some(c) if c.status.wallet_remain_usd.is_some() => {
                format_status_line_text(&c.status, session_cost_usd)
            }
            _ => format_status_line_text(&live, session_cost_usd),
        },
        Err(_) => match cached {
            Some(c) => format_status_line_text(&c.status, session_cost_usd),
            None => "Tako · 查询超时".to_string(),
        },
    }
}

pub fn mask_api_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 10 {
        return "cr_***".to_string();
    }
    let head: String = trimmed.chars().take(7).collect();
    let tail: String = trimmed
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

fn build_http_client(bypass_system_proxy: bool) -> Result<Client, String> {
    crate::config::apply_proxy_policy(
        Client::builder().timeout(Duration::from_secs(20)),
        bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))
}

fn generate_random_url_safe(num_bytes: usize) -> String {
    let mut buf = vec![0u8; num_bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn pkce_s256_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
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
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        }
    }
    map
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

#[cfg(target_os = "windows")]
fn open_external_url_windows(url: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut std::ffi::c_void,
            lp_operation: *const u16,
            lp_file: *const u16,
            lp_parameters: *const u16,
            lp_directory: *const u16,
            n_show_cmd: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;
    let op: Vec<u16> = OsStr::new("open").encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = OsStr::new(url).encode_wide().chain(Some(0)).collect();
    let ret = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if ret > 32 {
        return Ok(());
    }

    // Fallback: PowerShell Start-Process with single-quoted literal URL (never parses '&' or '%')
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let escaped = url.replace('\'', "''");
    let ps_cmd = format!("Start-Process -FilePath '{escaped}'");
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &ps_cmd,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("打开浏览器失败（ShellExecuteW={ret}, PowerShell={s}）")),
        Err(e) => Err(format!("拉起浏览器失败（ShellExecuteW={ret}）: {e}")),
    }
}

pub fn open_external_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        open_external_url_windows(url)
    }
    #[cfg(not(target_os = "windows"))]
    {
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open").arg(url).status();
        #[cfg(not(target_os = "macos"))]
        let status = std::process::Command::new("xdg-open").arg(url).status();

        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("打开浏览器失败，退出码: {s}")),
            Err(e) => Err(format!("拉起浏览器失败: {e}")),
        }
    }
}

pub fn build_oauth_authorize_url(
    base_url: &str,
    redirect_uri: &str,
    oauth_state: &str,
    code_challenge: &str,
    device_name: &str,
) -> String {
    let base = base_url.trim_end_matches('/');
    format!(
        "{base}/app/authorize?client_id={}&response_type=code&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&device_name={}",
        url_encode_component(DESKTOP_OAUTH_CLIENT_ID),
        url_encode_component(redirect_uri),
        url_encode_component(oauth_state),
        url_encode_component(code_challenge),
        url_encode_component(device_name),
    )
}

pub async fn start_oauth_login(paths: &AppPaths) -> Result<AccountStatus, String> {
    let state_cfg = read_switch_state(paths);
    let base_url = state_cfg.normalized_base_url();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("绑定本地 OAuth 回调端口失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取本地监听端口失败: {e}"))?
        .port();

    let redirect_uri = format!("http://127.0.0.1:{port}/tako/callback");
    let oauth_state = generate_random_url_safe(24);
    let code_verifier = generate_random_url_safe(48);
    let code_challenge = pkce_s256_challenge(&code_verifier);
    let device_name = crate::config::build_desktop_device_name(paths);

    let auth_url = build_oauth_authorize_url(
        &base_url,
        &redirect_uri,
        &oauth_state,
        &code_challenge,
        &device_name,
    );

    open_external_url(&auth_url)?;

    let code = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("接收回调失败: {e}"))?;
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let first_line = req.lines().next().unwrap_or("");
            let path_part = first_line.split_whitespace().nth(1).unwrap_or("");
            if !path_part.starts_with("/tako/callback") {
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .await;
                continue;
            }
            let query = path_part.split_once('?').map(|(_, q)| q).unwrap_or("");
            let params = parse_query_params(query);
            if params.get("state").map(String::as_str) != Some(oauth_state.as_str()) {
                let html = "<html><body><h3>授权校验失败（state 不匹配）</h3><p>请回到 Tako Switch 重试。</p></body></html>";
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return Err("OAuth 回调 state 校验失败".to_string());
            }
            if let Some(err) = params.get("error") {
                let html = "<html><body><h3>授权已取消或失败</h3><p>可以关闭此页面并回到 Tako Switch。</p></body></html>";
                let resp = format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                return Err(format!("浏览器授权返回错误: {err}"));
            }
            if let Some(code) = params.get("code").filter(|c| !c.trim().is_empty()) {
                let html = "<html><body style='font-family:-apple-system,sans-serif;text-align:center;padding:48px'><h2>Tako 授权成功</h2><p>已同步设备凭据，可以关闭本窗口回到 Tako Switch。</p><script>window.close()</script></body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                    html.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.flush().await;
                let _ = stream.shutdown().await;
                return Ok(code.clone());
            }
        }
    })
    .await
    .map_err(|_| "等待浏览器授权超时（180s），请重试或改用手动粘贴 API Key".to_string())??;

    exchange_desktop_token(paths, &base_url, &code, &code_verifier, &redirect_uri).await?;
    Ok(fetch_account_status(paths).await)
}

pub async fn exchange_desktop_token(
    paths: &AppPaths,
    base_url: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<(), String> {
    let bypass_proxy = read_switch_state(paths).bypass_system_proxy;
    let client = build_http_client(bypass_proxy)?;
    let resp = client
        .post(format!("{base_url}/api/desktop/token"))
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": code_verifier,
            "redirect_uri": redirect_uri,
            "client_id": DESKTOP_OAUTH_CLIENT_ID
        }))
        .send()
        .await
        .map_err(|e| format!("兑换设备凭据请求失败: {e}"))?;

    let status = resp.status();
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析凭据响应失败: {e}"))?;

    if !status.is_success() || payload.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = payload
            .get("message")
            .or_else(|| payload.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("兑换失败");
        return Err(format!("OAuth 兑换凭据失败 ({status}): {msg}"));
    }

    let key = payload
        .pointer("/data/key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "响应缺少 data.key".to_string())?;
    let api_id = get_key_id(&client, base_url, key).await.ok();
    save_switch_auth(paths, Some(key), api_id.as_deref(), Some("oauth"))
}

fn json_value_to_id_string(val: Option<&serde_json::Value>) -> Option<String> {
    let v = val?;
    if let Some(s) = v.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(n) = v.as_i64() {
        return Some(n.to_string());
    }
    if let Some(n) = v.as_u64() {
        return Some(n.to_string());
    }
    None
}

#[derive(Debug, Default)]
struct ProbedKeyIdentity {
    api_id: Option<String>,
    token_id: Option<String>,
    user_name: Option<String>,
    key_name: Option<String>,
}

async fn verify_and_probe_key(
    client: &Client,
    base_url: &str,
    key: &str,
) -> Result<ProbedKeyIdentity, String> {
    let mut desktop_me_err: Option<String> = None;

    if let Ok(resp) = client
        .get(format!("{base_url}/api/desktop/me"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
    {
        let status = resp.status();
        if let Ok(val) = resp.json::<serde_json::Value>().await {
            if status.is_success() && val.get("success").and_then(|v| v.as_bool()) == Some(true) {
                let mut identity = ProbedKeyIdentity {
                    api_id: json_value_to_id_string(val.pointer("/data/user/id")),
                    token_id: json_value_to_id_string(val.pointer("/data/token/id")),
                    user_name: val
                        .pointer("/data/user/name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                    key_name: val
                        .pointer("/data/token/name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                };
                if identity.api_id.is_none() {
                    identity.api_id = get_key_id(client, base_url, key).await.ok();
                }
                return Ok(identity);
            }
            desktop_me_err = val
                .get("message")
                .or_else(|| val.get("error"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }

    let verify_resp = client
        .post(format!("{base_url}/apiStats/api/verify-identity"))
        .json(&serde_json::json!({ "apiKey": key }))
        .send()
        .await
        .map_err(|e| format!("校验 Key 网络失败: {e}"))?;

    let body: serde_json::Value = verify_resp
        .json()
        .await
        .map_err(|e| format!("解析校验响应失败: {e}"))?;
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("error")
            .or_else(|| body.get("message"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or(desktop_me_err)
            .unwrap_or_else(|| "无效的 Tako API Key".to_string());
        return Err(msg);
    }

    let api_id = get_key_id(client, base_url, key).await.ok();
    let user_name = body
        .pointer("/user/name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let key_name = body
        .pointer("/token/name")
        .or_else(|| body.pointer("/key/name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(ProbedKeyIdentity {
        api_id,
        token_id: None,
        user_name,
        key_name,
    })
}

pub async fn add_saved_key(
    paths: &AppPaths,
    raw_key: &str,
    alias: Option<&str>,
    activate: bool,
) -> Result<AccountStatus, String> {
    let key = raw_key.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let client = build_http_client(state.bypass_system_proxy)?;

    let probed = verify_and_probe_key(&client, &base_url, key).await?;
    let had_active = read_main_api_key(paths).is_some();
    let will_activate = activate || !had_active;

    let alias_arg = alias.map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    upsert_saved_key(
        paths,
        UpsertSavedKeyInput {
            api_key: key,
            api_id: probed.api_id.as_deref(),
            token_id: probed.token_id.as_deref(),
            user_name: probed.user_name.as_deref(),
            key_name: probed.key_name.as_deref(),
            alias: alias_arg,
            auth_source: Some("manual_key"),
            activate: will_activate,
        },
    )?;

    if will_activate {
        clear_balance_cache(paths);
    }
    Ok(fetch_account_status(paths).await)
}

pub async fn switch_saved_key(paths: &AppPaths, key_id: &str) -> Result<AccountStatus, String> {
    activate_saved_key(paths, key_id)?;
    clear_balance_cache(paths);
    Ok(fetch_account_status(paths).await)
}

pub async fn remove_saved_key(paths: &AppPaths, key_id: &str) -> Result<AccountStatus, String> {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let (removed, next_active) = remove_saved_key_entry(paths, key_id)?;
    if removed.auth_source.as_deref() == Some("oauth") {
        if let Ok(client) = build_http_client(state.bypass_system_proxy) {
            let _ = client
                .post(format!("{base_url}/api/desktop/revoke"))
                .header("Authorization", format!("Bearer {}", removed.api_key))
                .send()
                .await;
        }
    }
    if next_active.is_some() || read_main_api_key(paths).is_none() {
        clear_balance_cache(paths);
    }
    Ok(fetch_account_status(paths).await)
}

pub async fn rename_saved_key_alias(
    paths: &AppPaths,
    key_id: &str,
    alias: Option<&str>,
) -> Result<AccountStatus, String> {
    update_saved_key_alias(paths, key_id, alias)?;
    let cached = read_balance_cache(paths).map(|c| c.status);
    let updated = build_local_account_status(paths, cached);
    if updated.logged_in && updated.error.is_none() {
        write_balance_cache(paths, &updated);
    }
    Ok(updated)
}

pub async fn login_with_key(paths: &AppPaths, raw_key: &str) -> Result<AccountStatus, String> {
    add_saved_key(paths, raw_key, None, true).await
}

pub async fn logout(paths: &AppPaths) -> Result<AccountStatus, String> {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let auth = read_switch_auth(paths);
    if auth.auth_source.as_deref() == Some("oauth") {
        if let Some(key) = auth.api_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if let Ok(client) = build_http_client(state.bypass_system_proxy) {
                let _ = client
                    .post(format!("{base_url}/api/desktop/revoke"))
                    .header("Authorization", format!("Bearer {key}"))
                    .send()
                    .await;
            }
        }
    }
    deactivate_current_auth(paths, true)?;
    clear_balance_cache(paths);
    Ok(fetch_account_status(paths).await)
}

async fn get_key_id(client: &Client, base_url: &str, key: &str) -> Result<String, String> {
    let resp = client
        .post(format!("{base_url}/apiStats/api/get-key-id"))
        .json(&serde_json::json!({ "apiKey": key }))
        .send()
        .await
        .map_err(|e| format!("get-key-id 请求失败: {e}"))?;
    let val: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 get-key-id 失败: {e}"))?;
    val.pointer("/data/id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "未能解析 apiId".to_string())
}

pub async fn fetch_account_status(paths: &AppPaths) -> AccountStatus {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let Some(key) = read_main_api_key(paths) else {
        return AccountStatus {
            logged_in: false,
            base_url,
            masked_key: None,
            user_name: None,
            key_name: None,
            active_key_id: None,
            active_key_alias: None,
            display_label: None,
            saved_keys: list_saved_key_summaries(paths),
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            wallet_remain_usd: None,
            wallet_used_usd: None,
            quota: None,
            error: None,
        };
    };

    let active_entry = read_active_saved_key(paths);
    let active_key_id = active_entry.as_ref().map(|e| e.id.clone());
    let active_key_alias = active_entry.as_ref().and_then(|e| e.alias.clone());
    let masked_str = mask_api_key(&key);
    let masked = Some(masked_str.clone());
    let Ok(client) = build_http_client(state.bypass_system_proxy) else {
        let user_name = active_entry.as_ref().and_then(|e| e.user_name.clone());
        let key_name = active_entry.as_ref().and_then(|e| e.key_name.clone());
        let display_label = Some(format_key_display_label(
            user_name.as_deref(),
            key_name.as_deref(),
            &masked_str,
        ));
        return AccountStatus {
            logged_in: true,
            base_url,
            masked_key: masked,
            user_name,
            key_name,
            active_key_id,
            active_key_alias,
            display_label,
            saved_keys: list_saved_key_summaries(paths),
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            wallet_remain_usd: None,
            wallet_used_usd: None,
            quota: None,
            error: Some("无法初始化 HTTP 客户端".to_string()),
        };
    };

    let mut api_id = read_main_api_id(paths);
    let mut token_id = active_entry.as_ref().and_then(|e| e.token_id.clone());
    let auth_source = read_switch_auth(paths).auth_source;
    let mut user_name = active_entry.as_ref().and_then(|e| e.user_name.clone());
    let mut key_name = active_entry.as_ref().and_then(|e| e.key_name.clone());
    let mut plan_name = None;
    let mut wallet_remain = None;
    let mut wallet_used = None;
    let mut err_msg = None;

    match client
        .get(format!("{base_url}/api/desktop/me"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(val) = resp.json::<serde_json::Value>().await {
                if val.get("success").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(u) = val
                        .pointer("/data/user/name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        user_name = Some(u.to_string());
                    }
                    if let Some(kn) = val
                        .pointer("/data/token/name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        key_name = Some(kn.to_string());
                    }
                    if let Some(tid) = json_value_to_id_string(val.pointer("/data/token/id")) {
                        token_id = Some(tid);
                    }
                    plan_name = val
                        .pointer("/data/plan/name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    wallet_remain = val.pointer("/data/quota/remain").and_then(|v| v.as_i64());
                    wallet_used = val.pointer("/data/quota/used").and_then(|v| v.as_i64());
                    if api_id.is_none() {
                        api_id = json_value_to_id_string(val.pointer("/data/user/id"));
                    }
                }
            }
        }
        Ok(resp) => {
            // Standard user cr_ keys created in Web console might not match desktop/me if not device tokens; fallback to verify-identity.
            if let Ok(ver) = client
                .post(format!("{base_url}/apiStats/api/verify-identity"))
                .json(&serde_json::json!({ "apiKey": &key }))
                .send()
                .await
            {
                if let Ok(val) = ver.json::<serde_json::Value>().await {
                    if val.get("success").and_then(|v| v.as_bool()) == Some(true) {
                        if let Some(u) = val
                            .pointer("/user/name")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            user_name = Some(u.to_string());
                        }
                        if let Some(kn) = val
                            .pointer("/token/name")
                            .or_else(|| val.pointer("/key/name"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            key_name = Some(kn.to_string());
                        }
                        plan_name = val
                            .pointer("/user/plan/name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                    } else {
                        err_msg = Some(format!("Key 校验失败 ({})", resp.status()));
                    }
                }
            } else {
                err_msg = Some(format!("拉取用户信息失败 ({})", resp.status()));
            }
        }
        Err(e) => {
            err_msg = Some(format!("连接 Tako 网关超时: {e}"));
        }
    }

    if api_id.is_none() {
        if let Ok(id) = get_key_id(&client, &base_url, &key).await {
            let _ = save_switch_auth(paths, Some(&key), Some(&id), auth_source.as_deref());
            api_id = Some(id);
        }
    }

    let _ = update_saved_key_metadata(
        paths,
        &key,
        api_id.as_deref(),
        token_id.as_deref(),
        user_name.as_deref(),
        key_name.as_deref(),
    );

    let mut quota = None;
    if let Some(ref uid) = api_id {
        if let Ok(qresp) = client
            .post(format!("{base_url}/apiStats/api/user-quota"))
            .json(&serde_json::json!({ "apiId": uid }))
            .send()
            .await
        {
            if let Ok(val) = qresp.json::<serde_json::Value>().await {
                if let Some(plan) = val.get("plan").and_then(|v| v.as_object()) {
                    let usage = val.get("usage");
                    quota = Some(QuotaSummary {
                        window_cost_limit: plan
                            .get("window_cost_limit")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        window_used: usage
                            .and_then(|u| u.get("windowCost"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        window_minutes: plan
                            .get("window_minutes")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(300.0),
                        daily_cost_limit: plan
                            .get("daily_cost_limit")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        daily_used: usage
                            .and_then(|u| u.get("dailyCost"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        weekly_cost_limit: plan
                            .get("weekly_cost_limit")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                        weekly_used: usage
                            .and_then(|u| u.get("weeklyCost"))
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0),
                    });
                }
            }
        }
    }

    let display_label = Some(format_key_display_label(
        user_name.as_deref(),
        key_name.as_deref(),
        &masked_str,
    ));
    let saved_keys = list_saved_key_summaries(paths);

    let status = AccountStatus {
        logged_in: true,
        base_url,
        masked_key: masked,
        user_name,
        key_name,
        active_key_id,
        active_key_alias,
        display_label,
        saved_keys,
        plan_name,
        wallet_remain,
        wallet_used,
        wallet_remain_usd: wallet_remain.map(quota_units_to_usd),
        wallet_used_usd: wallet_used.map(quota_units_to_usd),
        quota,
        error: err_msg,
    };
    if status.logged_in && status.error.is_none() {
        write_balance_cache(paths, &status);
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_and_url_encode_work() {
        let challenge = pkce_s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(
            url_encode_component("http://127.0.0.1:8080/tako/callback"),
            "http%3A%2F%2F127.0.0.1%3A8080%2Ftako%2Fcallback"
        );
    }

    #[test]
    fn mask_key_hides_secret_middle() {
        assert_eq!(mask_api_key("cr_1234567890abcdef"), "cr_1234…cdef");
        assert_eq!(mask_api_key("short"), "cr_***");
    }

    #[test]
    fn converts_quota_units_and_formats_status_line() {
        assert!((quota_units_to_usd(500_000) - 1.0).abs() < f64::EPSILON);
        assert_eq!(format_usd(12.345), "$12.35");

        let balance_only = AccountStatus {
            logged_in: true,
            base_url: "https://tako.shiroha.tech".to_string(),
            masked_key: Some("cr_abcd…efgh".to_string()),
            user_name: Some("Shiroha".to_string()),
            key_name: Some("主力密钥".to_string()),
            active_key_id: Some("k1".to_string()),
            active_key_alias: None,
            display_label: Some("Shiroha - 主力密钥".to_string()),
            saved_keys: Vec::new(),
            plan_name: None,
            wallet_remain: Some(6_250_000),
            wallet_used: Some(1_000_000),
            wallet_remain_usd: Some(12.5),
            wallet_used_usd: Some(2.0),
            quota: None,
            error: None,
        };
        assert_eq!(
            format_status_line_text(&balance_only, Some(0.32)),
            "Tako · 余额 $12.50 · 本次 $0.32"
        );

        let with_plan = AccountStatus {
            plan_name: Some("Pro x4".to_string()),
            quota: Some(QuotaSummary {
                window_cost_limit: 10.0,
                window_used: 2.0,
                window_minutes: 300.0,
                daily_cost_limit: 0.0,
                daily_used: 0.0,
                weekly_cost_limit: 50.0,
                weekly_used: 25.0,
            }),
            ..balance_only
        };
        assert_eq!(
            format_status_line_text(&with_plan, None),
            "Pro x4 · 余额 $12.50 · 5h 20% / 7d 50%"
        );
    }

    #[test]
    fn oauth_authorize_url_uses_takoswitch_client_id_and_device_name() {
        assert_eq!(DESKTOP_OAUTH_CLIENT_ID, "takoswitch");
        let url = build_oauth_authorize_url(
            "https://tako.shiroha.tech/",
            "http://127.0.0.1:43210/tako/callback",
            "state-xyz",
            "challenge-abc",
            "MacBook-Pro-a1b2c3d4",
        );
        assert!(url.starts_with("https://tako.shiroha.tech/app/authorize?"));
        assert!(url.contains("client_id=takoswitch"));
        assert!(!url.contains("tako-connect"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("code_challenge=challenge-abc"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-xyz"));
        assert!(url.contains("device_name=MacBook-Pro-a1b2c3d4"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A43210%2Ftako%2Fcallback"));
    }

    #[tokio::test]
    async fn exchange_desktop_token_sends_takoswitch_client_id_and_logout_keeps_device_id() {
        use tempfile::TempDir;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let mut token_body_seen = String::new();
            for _ in 0..3 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let resp_body = if req.starts_with("POST /api/desktop/token ") {
                    token_body_seen = req.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                    r#"{"success":true,"data":{"key":"cr_oauth_device_key"}}"#
                } else if req.starts_with("POST /apiStats/api/get-key-id ") {
                    r#"{"success":true,"data":{"id":"key_99"}}"#
                } else if req.starts_with("POST /api/desktop/revoke ") {
                    r#"{"success":true}"#
                } else {
                    r#"{"success":false}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
            token_body_seen
        });

        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let device_id = crate::config::read_or_create_device_id(&paths);
        let mut st = read_switch_state(&paths);
        st.base_url = format!("http://{addr}");
        crate::config::save_switch_state(&paths, &st).unwrap();

        exchange_desktop_token(
            &paths,
            &format!("http://{addr}"),
            "auth_code_1",
            "verifier_1",
            "http://127.0.0.1:9999/tako/callback",
        )
        .await
        .unwrap();

        assert_eq!(read_main_api_key(&paths).as_deref(), Some("cr_oauth_device_key"));
        assert_eq!(read_main_api_id(&paths).as_deref(), Some("key_99"));
        assert_eq!(read_switch_auth(&paths).auth_source.as_deref(), Some("oauth"));

        let status_after_logout = logout(&paths).await.unwrap();
        assert!(!status_after_logout.logged_in);
        assert!(paths.device_id_path().exists());
        assert_eq!(crate::config::read_or_create_device_id(&paths), device_id);

        let token_body = server.await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&token_body).unwrap();
        assert_eq!(parsed["client_id"], "takoswitch");
        assert_eq!(parsed["grant_type"], "authorization_code");
        assert_eq!(parsed["code"], "auth_code_1");
        assert_eq!(parsed["code_verifier"], "verifier_1");
        assert_eq!(
            parsed["redirect_uri"],
            "http://127.0.0.1:9999/tako/callback"
        );
    }

    #[test]
    fn format_key_display_label_combines_account_and_key_name() {
        assert_eq!(
            format_key_display_label(Some("Alice"), Some("主力密钥"), "sk-1234…abcd"),
            "Alice - 主力密钥"
        );
        assert_eq!(
            format_key_display_label(Some("Alice"), None, "sk-1234…abcd"),
            "Alice - sk-1234…abcd"
        );
        assert_eq!(
            format_key_display_label(None, Some("备用 Key"), "sk-1234…abcd"),
            "备用 Key"
        );
        assert_eq!(
            format_key_display_label(None, None, "sk-1234…abcd"),
            "sk-1234…abcd"
        );
    }

    #[tokio::test]
    async fn multi_key_same_account_add_switch_rename_logout_and_remove() {
        use tempfile::TempDir;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    let Ok(n) = sock.read(&mut buf).await else {
                        return;
                    };
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let resp_body = if req.starts_with("GET /api/desktop/me ") {
                        if req.contains("Bearer sk-primary-key-11112222") {
                            r#"{"success":true,"data":{"user":{"id":"42","name":"Alice"},"token":{"id":101,"name":"主力密钥"},"plan":{"name":"Pro"},"quota":{"remain":50000000,"used":10000000}}}"#
                        } else if req.contains("Bearer sk-backup-key-33334444") {
                            r#"{"success":true,"data":{"user":{"id":"42","name":"Alice"},"token":{"id":102,"name":"备用密钥"},"plan":{"name":""},"quota":{"remain":5000000,"used":500000}}}"#
                        } else {
                            r#"{"success":false,"message":"invalid token"}"#
                        }
                    } else if req.starts_with("POST /apiStats/api/verify-identity ") {
                        r#"{"success":false,"error":"invalid key"}"#
                    } else if req.starts_with("POST /apiStats/api/user-quota ") {
                        r#"{"success":true}"#
                    } else {
                        r#"{"success":false}"#
                    };
                    let status_line = if resp_body.contains("\"success\":true") {
                        "HTTP/1.1 200 OK"
                    } else {
                        "HTTP/1.1 401 Unauthorized"
                    };
                    let resp = format!(
                        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });

        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let mut st = read_switch_state(&paths);
        st.base_url = format!("http://{addr}");
        crate::config::save_switch_state(&paths, &st).unwrap();

        // 1. Add first key (same account "Alice", key "主力密钥")
        let s1 = add_saved_key(&paths, "sk-primary-key-11112222", Some("主力"), true)
            .await
            .unwrap();
        assert!(s1.logged_in);
        assert_eq!(s1.user_name.as_deref(), Some("Alice"));
        assert_eq!(s1.key_name.as_deref(), Some("主力密钥"));
        assert_eq!(s1.display_label.as_deref(), Some("Alice - 主力密钥"));
        assert_eq!(s1.active_key_alias.as_deref(), Some("主力"));
        assert_eq!(s1.saved_keys.len(), 1);
        let id1 = s1.active_key_id.clone().unwrap();

        // 2. Add second key belonging to the SAME account ("Alice", key "备用密钥") without activating yet
        let s2 = add_saved_key(&paths, "sk-backup-key-33334444", Some("备用"), false)
            .await
            .unwrap();
        assert_eq!(s2.active_key_id.as_deref(), Some(id1.as_str()));
        assert_eq!(s2.saved_keys.len(), 2);
        let id2 = s2
            .saved_keys
            .iter()
            .find(|k| k.id != id1)
            .unwrap()
            .id
            .clone();
        assert_eq!(
            s2.saved_keys.iter().find(|k| k.id == id2).unwrap().display_label,
            "Alice - 备用密钥"
        );

        // 3. Switch to key 2 -> active key, balance & display label switch cleanly
        let s3 = switch_saved_key(&paths, &id2).await.unwrap();
        assert_eq!(read_main_api_key(&paths).as_deref(), Some("sk-backup-key-33334444"));
        assert_eq!(s3.display_label.as_deref(), Some("Alice - 备用密钥"));
        assert_eq!(s3.wallet_remain_usd, Some(10.0));
        assert_eq!(s3.active_key_alias.as_deref(), Some("备用"));

        // 4. Rename alias
        let s4 = rename_saved_key_alias(&paths, &id2, Some("测试备用"))
            .await
            .unwrap();
        assert_eq!(s4.active_key_alias.as_deref(), Some("测试备用"));

        // 5. Logout keeps manual keys in saved_keys so user can switch back with one click
        let s5 = logout(&paths).await.unwrap();
        assert!(!s5.logged_in);
        assert_eq!(s5.saved_keys.len(), 2);
        assert!(s5.saved_keys.iter().all(|k| !k.active));

        // 6. Re-activate key 1, then remove active key 1 -> auto-activates remaining key 2
        let s6 = switch_saved_key(&paths, &id1).await.unwrap();
        assert_eq!(s6.display_label.as_deref(), Some("Alice - 主力密钥"));
        assert_eq!(s6.wallet_remain_usd, Some(100.0));

        let s7 = remove_saved_key(&paths, &id1).await.unwrap();
        assert!(s7.logged_in);
        assert_eq!(s7.saved_keys.len(), 1);
        assert_eq!(s7.active_key_id.as_deref(), Some(id2.as_str()));
        assert_eq!(read_main_api_key(&paths).as_deref(), Some("sk-backup-key-33334444"));

        // 7. Invalid key rejected without mutating saved list
        let err = add_saved_key(&paths, "sk-bad-key-99999999", None, true)
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert_eq!(list_saved_key_summaries(&paths).len(), 1);

        server.abort();
    }
}
