use crate::config::{
    read_main_api_id, read_main_api_key, read_switch_state, save_tako_cli_credentials, AppPaths,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub logged_in: bool,
    pub base_url: String,
    pub masked_key: Option<String>,
    pub user_name: Option<String>,
    pub plan_name: Option<String>,
    pub wallet_remain: Option<i64>,
    pub wallet_used: Option<i64>,
    pub quota: Option<QuotaSummary>,
    pub error: Option<String>,
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

fn build_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(20))
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

pub fn open_external_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .status();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let status = std::process::Command::new("xdg-open").arg(url).status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("打开浏览器失败，退出码: {s}")),
        Err(e) => Err(format!("拉起浏览器失败: {e}")),
    }
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
    let platform = std::env::consts::OS;
    let device_name = format!("tako-switch@{platform}");

    let auth_url = format!(
        "{base_url}/app/authorize?client_id=tako-connect&response_type=code&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&device_name={}",
        url_encode_component(&redirect_uri),
        url_encode_component(&oauth_state),
        url_encode_component(&code_challenge),
        url_encode_component(&device_name),
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
                return Ok(code.clone());
            }
        }
    })
    .await
    .map_err(|_| "等待浏览器授权超时（180s），请重试或改用手动粘贴 API Key".to_string())??;

    exchange_desktop_token(paths, &base_url, &code, &code_verifier, &redirect_uri).await?;
    Ok(fetch_account_status(paths).await)
}

async fn exchange_desktop_token(
    paths: &AppPaths,
    base_url: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<(), String> {
    let client = build_http_client()?;
    let resp = client
        .post(format!("{base_url}/api/desktop/token"))
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "code_verifier": code_verifier,
            "redirect_uri": redirect_uri,
            "client_id": "tako-connect"
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
    save_tako_cli_credentials(paths, Some(key), api_id.as_deref())
}

pub async fn login_with_key(paths: &AppPaths, raw_key: &str) -> Result<AccountStatus, String> {
    let key = raw_key.trim();
    if key.is_empty() {
        return Err("API Key 不能为空".to_string());
    }
    let base_url = read_switch_state(paths).normalized_base_url();
    let client = build_http_client()?;

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
            .and_then(|v| v.as_str())
            .unwrap_or("无效的 Tako API Key");
        return Err(msg.to_string());
    }

    let api_id = get_key_id(&client, &base_url, key).await.ok();
    save_tako_cli_credentials(paths, Some(key), api_id.as_deref())?;
    Ok(fetch_account_status(paths).await)
}

pub async fn logout(paths: &AppPaths) -> Result<AccountStatus, String> {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    if let Some(key) = read_main_api_key(paths) {
        if let Ok(client) = build_http_client() {
            let _ = client
                .post(format!("{base_url}/api/desktop/revoke"))
                .header("Authorization", format!("Bearer {key}"))
                .send()
                .await;
        }
    }
    save_tako_cli_credentials(paths, None, None)?;
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
    let base_url = read_switch_state(paths).normalized_base_url();
    let Some(key) = read_main_api_key(paths) else {
        return AccountStatus {
            logged_in: false,
            base_url,
            masked_key: None,
            user_name: None,
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            quota: None,
            error: None,
        };
    };

    let masked = Some(mask_api_key(&key));
    let Ok(client) = build_http_client() else {
        return AccountStatus {
            logged_in: true,
            base_url,
            masked_key: masked,
            user_name: None,
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            quota: None,
            error: Some("无法初始化 HTTP 客户端".to_string()),
        };
    };

    let mut api_id = read_main_api_id(paths);
    let mut user_name = None;
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
                    user_name = val
                        .pointer("/data/user/name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    plan_name = val
                        .pointer("/data/plan/name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    wallet_remain = val.pointer("/data/quota/remain").and_then(|v| v.as_i64());
                    wallet_used = val.pointer("/data/quota/used").and_then(|v| v.as_i64());
                    if api_id.is_none() {
                        api_id = val
                            .pointer("/data/user/id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        if let Some(ref found_id) = api_id {
                            let _ = save_tako_cli_credentials(paths, Some(&key), Some(found_id));
                        }
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
                        user_name = val
                            .pointer("/user/name")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
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
            let _ = save_tako_cli_credentials(paths, Some(&key), Some(&id));
            api_id = Some(id);
        }
    }

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

    AccountStatus {
        logged_in: true,
        base_url,
        masked_key: masked,
        user_name,
        plan_name,
        wallet_remain,
        wallet_used,
        quota,
        error: err_msg,
    }
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
}
