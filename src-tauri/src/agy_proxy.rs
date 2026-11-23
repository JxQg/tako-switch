use crate::adapters::{get_adapter, AppId, AppMode};
#[cfg(not(target_os = "windows"))]
use crate::config::write_atomic;
use crate::config::{
    apply_proxy_policy, effective_agy_api_key, read_main_api_key, read_switch_state, AppPaths,
    ClientModelFilter, ModelFilterMode,
};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::{Body as HttpBody, Frame, Incoming};
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::{Client, Method};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream as TokioTcpStream};
use tokio::sync::{Notify, RwLock};

pub const AGY_PROXY_PORT: u16 = 18419;
pub const AGY_PROXY_URL: &str = "http://127.0.0.1:18419";
pub const AGY_PROXY_HEALTH_PATH: &str = "/__tako_proxy_health";

pub const LOCAL_GATEWAY_PORT: u16 = AGY_PROXY_PORT;
pub const LOCAL_GATEWAY_URL: &str = AGY_PROXY_URL;
pub const CODEX_PROXY_BASE_URL: &str = "http://127.0.0.1:18419/codex/v1";
pub const CODEX_PROXY_CATALOG_URL: &str = "http://127.0.0.1:18419/codex/models";
pub const CLAUDE_PROXY_BASE_URL: &str = "http://127.0.0.1:18419/claude";

#[cfg(not(target_os = "windows"))]
pub const GATEWAY_PLIST_NAME: &str = "tech.shiroha.tako-switch.gateway.plist";
#[cfg(not(target_os = "windows"))]
const LEGACY_AGY_PROXY_PLIST_NAME: &str = "tech.shiroha.tako-switch.agy-proxy.plist";
#[cfg(target_os = "macos")]
const GATEWAY_PLIST_LABEL: &str = "tech.shiroha.tako-switch.gateway";
#[cfg(target_os = "macos")]
const LEGACY_AGY_PROXY_PLIST_LABEL: &str = "tech.shiroha.tako-switch.agy-proxy";

const PRICING_CACHE_TTL: Duration = Duration::from_secs(300);
const ALLOWED_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const AGY_MAX_CUSTOM_PLACEHOLDERS: usize = 32;
pub const OFFICIAL_PRICE_RATIO_BASE: f64 = 20.4;
pub const GEMINI_4_PROMO_CUTOFF: &str = "2026-11-30";
const DEFAULT_CODEX_BASE_INSTRUCTIONS: &str = "You are Codex, an expert AI software engineering assistant. Follow user instructions carefully, inspect files before editing, and verify changes using available tools.";

static IN_PROCESS_PROXY_STARTED: AtomicBool = AtomicBool::new(false);

type ProxyBody = BoxBody<Bytes, std::io::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Antigravity,
    Codex,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelChannelPrice {
    pub name: String,
    pub ratio: f64,
    pub standard_load_usd_per_m: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelPricingInfo {
    pub is_free: bool,
    pub is_dynamic_pricing: bool,
    pub standard_load_usd_per_m: Option<f64>,
    pub standard_load_max_usd_per_m: Option<f64>,
    pub realized_usd_per_m: Option<f64>,
    pub cache_hit_rate: Option<f64>,
    pub input_usd_per_m: Option<f64>,
    pub cache_usd_per_m: Option<f64>,
    pub output_usd_per_m: Option<f64>,
    pub request_usd: Option<f64>,
    pub quota_type: i64,
    pub model_ratio: Option<f64>,
    pub official_multiplier: Option<f64>,
    pub promo_ends_on: Option<String>,
    pub capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub vendor_name: Option<String>,
    pub context_window: Option<u64>,
    pub channels: Vec<ModelChannelPrice>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelPerfInfo {
    pub avg_tps: Option<f64>,
    pub avg_latency_ms: Option<f64>,
    pub success_rate: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PricingCache {
    pub reasoning_levels: HashMap<String, Vec<String>>,
    pub descriptions: HashMap<String, String>,
    pub has_flash_image: bool,
    pub prices: HashMap<String, ModelPricingInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntheticRoute {
    pub base_model: String,
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientModelOption {
    pub id: String,
    pub display_name: String,
    pub family: String,
    pub description: String,
    pub context_window: Option<u64>,
    pub recommended: bool,
    pub visible: bool,
    pub effort_levels: Vec<String>,
    pub is_free: bool,
    pub is_dynamic_pricing: bool,
    pub standard_load_usd_per_m: Option<f64>,
    pub standard_load_max_usd_per_m: Option<f64>,
    pub realized_usd_per_m: Option<f64>,
    pub cache_hit_rate: Option<f64>,
    pub input_usd_per_m: Option<f64>,
    pub cache_usd_per_m: Option<f64>,
    pub output_usd_per_m: Option<f64>,
    pub request_usd: Option<f64>,
    pub quota_type: i64,
    pub model_ratio: Option<f64>,
    pub official_multiplier: Option<f64>,
    pub promo_ends_on: Option<String>,
    pub capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub vendor_name: Option<String>,
    pub channels: Vec<ModelChannelPrice>,
    pub avg_tps: Option<f64>,
    pub avg_latency_ms: Option<f64>,
    pub success_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientModelsResponse {
    pub app_id: AppId,
    pub filter: ClientModelFilter,
    pub recommended_ids: Vec<String>,
    pub visible_count: usize,
    pub total_count: usize,
    pub models: Vec<ClientModelOption>,
}

fn catalog_filter_key(_app_id: AppId, id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

fn normalized_filter_set(app_id: AppId, filter: &ClientModelFilter) -> Option<HashSet<String>> {
    match filter.mode {
        ModelFilterMode::All => None,
        ModelFilterMode::Recommended => Some(
            ClientModelFilter::recommended_ids_for(app_id)
                .iter()
                .map(|s| catalog_filter_key(app_id, s))
                .collect(),
        ),
        ModelFilterMode::Custom => Some(
            filter
                .enabled_ids
                .iter()
                .map(|s| catalog_filter_key(app_id, s))
                .filter(|s| !s.is_empty())
                .collect(),
        ),
    }
}

fn is_recommended_model(app_id: AppId, id: &str) -> bool {
    let key = catalog_filter_key(app_id, id);
    ClientModelFilter::recommended_ids_for(app_id)
        .iter()
        .any(|rec| rec.eq_ignore_ascii_case(&key))
}

fn model_family_label(app_id: AppId, id: &str) -> &'static str {
    if app_id == AppId::Claude {
        return match parse_claude_tier_and_version(id).0 {
            Some("opus") => "Opus",
            Some("sonnet") => "Sonnet",
            Some("fable") => "Fable",
            Some("haiku") => "Haiku",
            Some("mythos") => "Mythos",
            _ => "Claude",
        };
    }
    let lower = id.trim().to_ascii_lowercase();
    if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        "GPT"
    } else if lower.starts_with("claude-") {
        "Claude"
    } else if lower.starts_with("gemini-") || lower.starts_with("gemma-") {
        "Gemini"
    } else if lower.starts_with("deepseek-") {
        "DeepSeek"
    } else if lower.starts_with("swe-") {
        "SWE"
    } else if lower.starts_with("glm-") {
        "GLM"
    } else if lower.starts_with("kimi-") {
        "Kimi"
    } else if lower.starts_with("grok-") {
        "Grok"
    } else if lower.starts_with("minimax-") {
        "MiniMax"
    } else if lower.starts_with("mimo-") {
        "MiMo"
    } else {
        "其他"
    }
}

const PASSTHROUGH_STREAM_THRESHOLD_BYTES: u64 = 64 * 1024;

struct ProxyState {
    paths: AppPaths,
    direct_client: Client,
    system_proxy_client: Client,
    pricing_cache: RwLock<Option<(String, Instant, PricingCache)>>,
    synthetic_routes: RwLock<HashMap<String, SyntheticRoute>>,
    codex_model_messages: RwLock<Option<Value>>,
}

impl ProxyState {
    fn upstream_client(&self, bypass_system_proxy: bool) -> &Client {
        if bypass_system_proxy {
            &self.direct_client
        } else {
            &self.system_proxy_client
        }
    }
}

struct SharedClientTcp {
    inner: Arc<std::sync::Mutex<TokioTcpStream>>,
}

impl AsyncRead for SharedClientTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Pin::new(&mut *guard).poll_read(cx, buf)
    }
}

impl AsyncWrite for SharedClientTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Pin::new(&mut *guard).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Pin::new(&mut *guard).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut guard = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Pin::new(&mut *guard).poll_shutdown(cx)
    }
}

struct BodyDoneSignal {
    done: AtomicBool,
    notify: Notify,
}

impl BodyDoneSignal {
    fn new(already_done: bool) -> Arc<Self> {
        Arc::new(Self {
            done: AtomicBool::new(already_done),
            notify: Notify::new(),
        })
    }

    fn mark_done(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.done.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

async fn wait_for_client_disconnect(
    downstream_tcp: Option<Arc<std::sync::Mutex<TokioTcpStream>>>,
    body_done: Arc<BodyDoneSignal>,
) {
    let Some(tcp) = downstream_tcp else {
        std::future::pending::<()>().await;
        return;
    };
    body_done.wait().await;
    let outcome = std::future::poll_fn(|cx| {
        let guard = tcp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut raw = [0u8; 1];
        let mut buf = ReadBuf::new(&mut raw);
        guard.poll_peek(cx, &mut buf)
    })
    .await;
    match outcome {
        Ok(0) | Err(_) => {}
        Ok(_) => {
            std::future::pending::<()>().await;
        }
    }
}

fn wrap_incoming_body_stream(incoming: Incoming, body_done: Arc<BodyDoneSignal>) -> reqwest::Body {
    let stream = futures_util::stream::unfold(
        (incoming.into_data_stream(), body_done, false),
        |(mut data_stream, signal, finished)| async move {
            if finished {
                return None;
            }
            match data_stream.next().await {
                Some(Ok(chunk)) => Some((
                    Ok::<Bytes, std::io::Error>(chunk),
                    (data_stream, signal, false),
                )),
                Some(Err(err)) => {
                    signal.mark_done();
                    Some((
                        Err(std::io::Error::other(format!("读取下游请求体中断: {err}"))),
                        (data_stream, signal, true),
                    ))
                }
                None => {
                    signal.mark_done();
                    None
                }
            }
        },
    );
    reqwest::Body::wrap_stream(stream)
}

fn current_utc_ymd() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

pub fn resolve_promo_ends_on_at_date(
    model_id: &str,
    item: Option<&Value>,
    today_ymd: &str,
) -> Option<String> {
    if let Some(explicit) = item
        .and_then(|v| v.get("promo_ends_on").or_else(|| v.get("promoEndsOn")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if explicit >= today_ymd {
            return Some(explicit.to_string());
        }
        return None;
    }
    let lower = model_id.trim().to_ascii_lowercase();
    let (base, _) = split_effort_slug(&lower);
    if matches!(base, "gemini-4-carbon" | "gemini-4-argon") && today_ymd <= GEMINI_4_PROMO_CUTOFF {
        return Some(GEMINI_4_PROMO_CUTOFF.to_string());
    }
    None
}

pub fn resolve_promo_ends_on(model_id: &str, item: Option<&Value>) -> Option<String> {
    let today = current_utc_ymd();
    resolve_promo_ends_on_at_date(model_id, item, &today)
}

fn extract_expr_named_f64(expr: &str, keys: &[&str]) -> Option<f64> {
    for &key in keys {
        let mut cursor = expr;
        while let Some(idx) = cursor.find(key) {
            let after_key = &cursor[idx + key.len()..];
            let trimmed = after_key.trim_start_matches(['"', '\'', ' ', '\t', ':', '=']);
            if trimmed.len() < after_key.len() {
                let num_len = trimmed
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .count();
                if num_len > 0 {
                    if let Ok(val) = trimmed[..num_len].parse::<f64>() {
                        if val.is_finite() && val > 0.0 {
                            return Some(val);
                        }
                    }
                }
            }
            cursor = after_key;
        }
    }
    None
}

fn extract_first_tier_prices(
    item: &Value,
    billing_expr: &str,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let read_from_tier_obj = |tier: &Value| -> (Option<f64>, Option<f64>, Option<f64>) {
        let inp = tier
            .get("inputPrice")
            .or_else(|| tier.get("input_price"))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);
        let cache = tier
            .get("cacheReadPrice")
            .or_else(|| tier.get("cache_read_price"))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);
        let out = tier
            .get("outputPrice")
            .or_else(|| tier.get("output_price"))
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);
        (inp, cache, out)
    };

    for field in ["tiered_prices", "tiers", "pricing_tiers", "billing_tiers"] {
        if let Some(first) = item
            .get(field)
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
        {
            let res = read_from_tier_obj(first);
            if res.0.is_some() || res.1.is_some() || res.2.is_some() {
                return res;
            }
        }
    }

    if !billing_expr.is_empty() {
        if let Ok(parsed) = serde_json::from_str::<Value>(billing_expr) {
            if let Some(first) = parsed.as_array().and_then(|a| a.first()).or_else(|| {
                parsed
                    .get("tiers")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
            }) {
                let res = read_from_tier_obj(first);
                if res.0.is_some() || res.1.is_some() || res.2.is_some() {
                    return res;
                }
            } else if parsed.is_object() {
                let res = read_from_tier_obj(&parsed);
                if res.0.is_some() || res.1.is_some() || res.2.is_some() {
                    return res;
                }
            }
        }
        let inp = extract_expr_named_f64(billing_expr, &["inputPrice", "input_price"]);
        let cache = extract_expr_named_f64(billing_expr, &["cacheReadPrice", "cache_read_price"]);
        let out = extract_expr_named_f64(billing_expr, &["outputPrice", "output_price"]);
        if inp.is_some() || cache.is_some() || out.is_some() {
            return (inp, cache, out);
        }
    }

    (None, None, None)
}

fn has_tiered_plugin_variant(item: &Value) -> bool {
    if item
        .get("has_tiered_plugin_variant")
        .or_else(|| item.get("hasTieredPluginVariant"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return true;
    }
    item.get("channels")
        .and_then(Value::as_array)
        .is_some_and(|arr| {
            arr.iter().any(|ch| {
                ch.get("has_tiered_plugin_variant")
                    .and_then(Value::as_bool)
                    == Some(true)
                    || ch.get("is_tiered").and_then(Value::as_bool) == Some(true)
                    || ch.get("billing_mode").and_then(Value::as_str) == Some("tiered_expr")
                    || ch
                        .get("billing_expr")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
            })
        })
}

fn extract_item_tags(item: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut push_tag = |raw: &str| {
        let t = raw.trim();
        if !t.is_empty() && seen.insert(t.to_ascii_lowercase()) {
            out.push(t.to_string());
        }
    };
    if let Some(arr) = item.get("tags").and_then(Value::as_array) {
        for v in arr {
            if let Some(s) = v.as_str() {
                push_tag(s);
            }
        }
    } else if let Some(s) = item.get("tags").and_then(Value::as_str) {
        for part in s.split([',', '，', ';']) {
            push_tag(part);
        }
    }
    out
}

pub fn infer_vendor_name(model_id: &str, item: Option<&Value>) -> Option<String> {
    if let Some(raw_vendor) = item
        .and_then(|v| {
            v.get("vendor_name")
                .or_else(|| v.get("vendorName"))
                .or_else(|| v.get("vendor"))
                .or_else(|| v.get("provider"))
                .or_else(|| v.get("owned_by"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let norm = match raw_vendor.to_ascii_lowercase().as_str() {
            "openai" => Some("OpenAI"),
            "anthropic" | "claude" => Some("Anthropic"),
            "google" | "gemini" | "vertex" | "vertex-ai" => Some("Google"),
            "deepseek" => Some("DeepSeek"),
            "xai" | "grok" => Some("xAI"),
            "moonshot" | "kimi" => Some("Moonshot"),
            "zhipu" | "glm" | "bigmodel" => Some("Zhipu"),
            "minimax" => Some("MiniMax"),
            "xiaomi" | "mimo" => Some("Xiaomi"),
            "tencent" | "hunyuan" => Some("Tencent"),
            "bytedance" | "doubao" | "volcengine" | "seed" => Some("ByteDance"),
            "alibaba" | "qwen" | "dashscope" => Some("Alibaba"),
            "custom" | "system" | "default" | "tako" => None,
            _ => Some(raw_vendor),
        };
        if let Some(v) = norm {
            return Some(v.to_string());
        }
    }

    let lower = model_id.trim().to_ascii_lowercase();
    let inferred = if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("sora")
        || lower.starts_with("dall-e")
        || lower.starts_with("whisper")
        || lower.starts_with("text-embedding-")
    {
        Some("OpenAI")
    } else if lower.starts_with("claude-") {
        Some("Anthropic")
    } else if lower.starts_with("gemini-")
        || lower.starts_with("gemma-")
        || lower.starts_with("imagen")
        || lower.starts_with("veo")
    {
        Some("Google")
    } else if lower.starts_with("deepseek-") {
        Some("DeepSeek")
    } else if lower.starts_with("grok-") {
        Some("xAI")
    } else if lower.starts_with("kimi-") {
        Some("Moonshot")
    } else if lower.starts_with("glm-") || lower.starts_with("cogvideo") {
        Some("Zhipu")
    } else if lower.starts_with("minimax-") || lower.starts_with("hailuo") {
        Some("MiniMax")
    } else if lower.starts_with("mimo-") {
        Some("Xiaomi")
    } else if lower == "hy3" || lower.starts_with("hunyuan") || lower.starts_with("hy-") {
        Some("Tencent")
    } else if lower.starts_with("qwen") || lower.starts_with("wan") || lower.starts_with("cosyvoice") {
        Some("Alibaba")
    } else if lower.starts_with("seed") || lower.starts_with("doubao") {
        Some("ByteDance")
    } else if lower.starts_with("flux") {
        Some("Black Forest Labs")
    } else if lower.starts_with("kling") {
        Some("Kuaishou")
    } else if lower.starts_with("vidu") {
        Some("Shengshu")
    } else if lower.starts_with("swe-") || lower.starts_with("composer-") {
        Some("Cognition / Cursor")
    } else {
        None
    };
    inferred.map(str::to_string)
}

fn parse_compact_token_count(token_str: &str) -> Option<u64> {
    let cleaned = token_str
        .trim()
        .trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '，' | ',' | '。' | '.'))
        .replace("上下文", "")
        .replace("context", "")
        .replace("Context", "")
        .replace("tokens", "")
        .replace("Tokens", "");
    let s = cleaned.trim();
    if s.is_empty() {
        return None;
    }
    let last = s.chars().last()?;
    if matches!(last, 'm' | 'M') {
        let num = s[..s.len() - 1].trim().parse::<f64>().ok()?;
        if (0.1..=100.0).contains(&num) {
            return Some((num * 1_000_000.0).round() as u64);
        }
    } else if matches!(last, 'k' | 'K') {
        let num = s[..s.len() - 1].trim().parse::<f64>().ok()?;
        if (4.0..=9999.0).contains(&num) {
            return Some((num * 1_000.0).round() as u64);
        }
    }
    None
}

fn extract_context_from_description(desc: &str) -> Option<u64> {
    if desc.trim().is_empty() {
        return None;
    }
    for marker in ["上下文", "context", "Context"] {
        if let Some(idx) = desc.find(marker) {
            let after = desc[idx + marker.len()..]
                .trim_start_matches([' ', ':', '：', '为', '达', '长']);
            let tok_after: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect();
            if let Some(v) = parse_compact_token_count(&tok_after) {
                return Some(v);
            }

            let before = desc[..idx].trim_end();
            let rev_tok: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if let Some(v) = parse_compact_token_count(&rev_tok) {
                return Some(v);
            }
        }
    }
    None
}

pub fn derive_model_capabilities(
    model_id: &str,
    item: Option<&Value>,
    tags: &[String],
    desc: &str,
    has_reasoning_levels: bool,
) -> Vec<String> {
    let lower = model_id.trim().to_ascii_lowercase();
    let desc_lower = desc.to_ascii_lowercase();
    let tags_joined = tags.join(" ").to_ascii_lowercase();
    let endpoints: Vec<String> = item
        .map(extract_endpoint_types)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let mut set: HashSet<&'static str> = HashSet::new();
    if let Some(arr) = item
        .and_then(|v| v.get("capabilities"))
        .and_then(Value::as_array)
    {
        for cap in arr.iter().filter_map(Value::as_str) {
            match cap.trim().to_ascii_lowercase().as_str() {
                "chat" => {
                    set.insert("chat");
                }
                "reason" | "reasoning" | "thinking" => {
                    set.insert("reason");
                }
                "vision" | "multimodal" => {
                    set.insert("vision");
                }
                "code" | "coding" => {
                    set.insert("code");
                }
                "image" | "images" => {
                    set.insert("image");
                }
                "video" => {
                    set.insert("video");
                }
                "audio" | "speech" | "voice" => {
                    set.insert("audio");
                }
                "embed" | "embedding" | "embeddings" => {
                    set.insert("embed");
                }
                _ => {}
            }
        }
    }

    let is_embed = lower.contains("embed")
        || lower.contains("rerank")
        || lower.starts_with("bge-")
        || endpoints.iter().any(|e| e.contains("embed"))
        || tags_joined.contains("向量")
        || tags_joined.contains("embedding");
    if is_embed {
        set.insert("embed");
    }

    let is_image = lower.contains("image")
        || lower.contains("imagen")
        || lower.starts_with("flux")
        || lower.starts_with("dall-e")
        || lower.contains("ideogram")
        || lower.contains("recraft")
        || lower.contains("midjourney")
        || lower.contains("seedream")
        || lower.contains("kolors")
        || endpoints.iter().any(|e| e.contains("image") || e.contains("dall-e"))
        || tags_joined.contains("生图")
        || tags_joined.contains("文生图")
        || tags_joined.contains("改图")
        || desc_lower.contains("图像生成")
        || desc_lower.contains("生图");
    if is_image {
        set.insert("image");
    }

    let is_video = lower.contains("video")
        || lower.starts_with("veo")
        || lower.contains("-veo")
        || lower.starts_with("sora")
        || lower.starts_with("kling")
        || lower.starts_with("vidu")
        || lower.starts_with("hailuo")
        || lower.starts_with("wan-")
        || lower.starts_with("wan2")
        || lower.contains("seedance")
        || lower.contains("luma")
        || lower.contains("runway")
        || endpoints.iter().any(|e| e.contains("video"))
        || tags_joined.contains("视频")
        || desc_lower.contains("视频生成");
    if is_video {
        set.insert("video");
    }

    let is_audio = lower.contains("tts")
        || lower.contains("asr")
        || lower.contains("whisper")
        || lower.contains("speech")
        || lower.contains("audio")
        || lower.contains("voice")
        || lower.contains("cosyvoice")
        || endpoints.iter().any(|e| e.contains("audio") || e.contains("speech"))
        || tags_joined.contains("语音")
        || tags_joined.contains("声音")
        || tags_joined.contains("tts")
        || desc_lower.contains("语音合成")
        || desc_lower.contains("语音识别");
    if is_audio {
        set.insert("audio");
    }

    let has_chat_endpoint = endpoints.iter().any(|e| {
        matches!(
            e.as_str(),
            "openai" | "openai-response" | "anthropic" | "gemini" | "chat"
        )
    });
    let pure_media_or_embed =
        (is_embed || is_video || is_audio || (is_image && !lower.starts_with("gpt-4o")))
            && !has_chat_endpoint;
    let is_search_tool_only = lower == "web_search";

    if !pure_media_or_embed
        && !is_search_tool_only
        && !is_embed
        && !is_video
        && !(is_image
            && (lower.starts_with("flux")
                || lower.starts_with("dall-e")
                || lower.starts_with("imagen")
                || lower.starts_with("gpt-image")
                || lower.contains("seedream")
                || lower.contains("recraft")
                || lower.contains("ideogram")))
        && !(is_audio
            && (lower.contains("tts") || lower.contains("asr") || lower.contains("whisper")))
    {
        set.insert("chat");
    }

    if set.contains("chat") {
        let is_reason = has_reasoning_levels
            || lower.contains("reason")
            || lower.contains("thinking")
            || lower.contains("qwq")
            || lower.starts_with("o1")
            || lower.starts_with("o3")
            || lower.starts_with("o4")
            || lower.starts_with("gpt-5")
            || lower.starts_with("gpt-6")
            || lower.starts_with("claude-opus")
            || lower.starts_with("claude-sonnet")
            || lower.starts_with("gemini-3")
            || lower.starts_with("gemini-4")
            || lower.starts_with("deepseek-r")
            || lower.starts_with("deepseek-v4")
            || lower.starts_with("kimi-k3")
            || lower.starts_with("glm-5")
            || lower.starts_with("grok-4")
            || lower.starts_with("minimax-m")
            || lower.starts_with("mimo-v2")
            || lower.starts_with("swe-")
            || tags_joined.contains("推理")
            || tags_joined.contains("思考")
            || desc_lower.contains("推理")
            || desc_lower.contains("reasoning");
        if is_reason && !is_image {
            set.insert("reason");
        }

        let is_vision = lower.contains("vision")
            || lower.contains("-vl")
            || lower.contains("4o")
            || lower.starts_with("gpt-5")
            || lower.starts_with("gpt-6")
            || lower.starts_with("claude-")
            || lower.starts_with("gemini-")
            || lower.starts_with("grok-4")
            || lower.starts_with("kimi-k3")
            || lower.starts_with("glm-5")
            || lower.starts_with("deepseek-v4")
            || lower.starts_with("minimax-m3")
            || tags_joined.contains("视觉")
            || tags_joined.contains("看图")
            || tags_joined.contains("多模态")
            || desc_lower.contains("多模态")
            || desc_lower.contains("看图")
            || desc_lower.contains("视觉")
            || desc_lower.contains("vision");
        if is_vision {
            set.insert("vision");
        }

        let is_code = lower.contains("code")
            || lower.contains("codex")
            || lower.starts_with("swe-")
            || lower.starts_with("composer-")
            || lower.starts_with("claude-opus")
            || lower.starts_with("claude-sonnet")
            || lower.starts_with("gpt-5")
            || lower.starts_with("gpt-6")
            || lower.starts_with("deepseek-v")
            || lower.starts_with("deepseek-coder")
            || lower.starts_with("glm-5")
            || lower.starts_with("kimi-k3")
            || lower.starts_with("grok-4")
            || lower.starts_with("minimax-m3")
            || lower.starts_with("mimo-v2")
            || (lower.starts_with("gemini-3") && !is_image && !lower.contains("lite"))
            || (lower.starts_with("gemini-4") && !is_image)
            || tags_joined.contains("编程")
            || tags_joined.contains("编码")
            || tags_joined.contains("代码")
            || desc_lower.contains("编程")
            || desc_lower.contains("编码")
            || desc_lower.contains("代码")
            || desc_lower.contains("coding");
        if is_code {
            set.insert("code");
        }
    }

    const ORDER: &[&str] = &[
        "chat", "reason", "vision", "code", "image", "video", "audio", "embed",
    ];
    ORDER
        .iter()
        .filter(|&&c| set.contains(c))
        .map(|&c| c.to_string())
        .collect()
}

pub fn extract_pricing_context_window(
    model_id: &str,
    item: Option<&Value>,
    tags: &[String],
    desc: &str,
    capabilities: &[String],
) -> Option<u64> {
    if let Some(obj) = item {
        for field in [
            "context_length",
            "contextLength",
            "context_window",
            "contextWindow",
            "max_context_length",
            "max_input_tokens",
        ] {
            if let Some(n) = obj.get(field).and_then(Value::as_u64) {
                if n >= 1_000 {
                    return Some(n);
                }
            }
            if let Some(s) = obj.get(field).and_then(Value::as_str) {
                if let Some(parsed) = parse_compact_token_count(s) {
                    return Some(parsed);
                }
            }
        }
    }

    for tag in tags {
        if let Some(parsed) = parse_compact_token_count(tag) {
            return Some(parsed);
        }
    }

    if let Some(parsed) = extract_context_from_description(desc) {
        return Some(parsed);
    }

    let is_chat_llm = capabilities.iter().any(|c| c == "chat");
    let is_media_only = !is_chat_llm
        && capabilities
            .iter()
            .any(|c| matches!(c.as_str(), "image" | "video" | "audio" | "embed"));
    if is_media_only {
        return None;
    }

    let lower = model_id.trim().to_ascii_lowercase();
    if lower.starts_with("claude-")
        || lower.starts_with("gpt-")
        || lower.starts_with("gemini-")
        || lower.starts_with("gemma-")
        || lower.starts_with("deepseek-")
        || lower.starts_with("glm-")
        || lower.starts_with("kimi-")
        || lower.starts_with("grok-")
        || lower.starts_with("minimax-")
        || lower.starts_with("mimo-")
        || lower.starts_with("swe-")
        || lower.starts_with("composer-")
        || lower == "hy3"
    {
        return Some(model_context_window(model_id));
    }
    None
}

pub fn parse_pricing_metadata(body: &Value) -> PricingCache {
    let mut cache = PricingCache::default();
    let top_group_ratio: HashMap<String, f64> = body
        .get("group_ratio")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_f64().filter(|n| n.is_finite()).map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();

    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return cache;
    };
    for item in items {
        let Some(model_name) = item
            .get("model_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if model_name == "gemini-3.1-flash-image" {
            cache.has_flash_image = true;
        }
        if let Some(desc) = item
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            cache
                .descriptions
                .insert(model_name.to_string(), desc.to_string());
        }
        if let Some(raw_levels) = item.get("reasoning_levels").and_then(Value::as_array) {
            let mut present = HashSet::new();
            for lvl in raw_levels {
                if let Some(s) = lvl.as_str() {
                    let norm = s.trim().to_ascii_lowercase();
                    if ALLOWED_EFFORTS.contains(&norm.as_str()) {
                        present.insert(norm);
                    }
                }
            }
            let ordered: Vec<String> = ALLOWED_EFFORTS
                .iter()
                .filter(|eff| present.contains(**eff))
                .map(|eff| (*eff).to_string())
                .collect();
            if !ordered.is_empty() {
                cache.reasoning_levels.insert(model_name.to_string(), ordered);
            }
        }

        let item_group_ratio = item.get("group_ratio").and_then(Value::as_object);
        let ch_min_map = item
            .get("group_channel_ratio_min")
            .and_then(Value::as_object);
        let ch_max_map = item
            .get("group_channel_ratio_max")
            .and_then(Value::as_object);
        let enable_groups: Vec<&str> = item
            .get("enable_groups")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let mut min_ratio = f64::INFINITY;
        let mut max_ratio = f64::NEG_INFINITY;
        for g in &enable_groups {
            let gr = item_group_ratio
                .and_then(|m| m.get(*g))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .or_else(|| top_group_ratio.get(*g).copied())
                .unwrap_or(1.0);
            let cm = ch_min_map
                .and_then(|m| m.get(*g))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .unwrap_or(1.0);
            let cx = ch_max_map
                .and_then(|m| m.get(*g))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .unwrap_or(cm);
            min_ratio = min_ratio.min(gr * cm);
            max_ratio = max_ratio.max(gr * cx);
        }
        let display_group_ratio = if min_ratio.is_finite() {
            min_ratio
        } else {
            1.0
        };
        let display_group_ratio_max = if max_ratio.is_finite() {
            max_ratio
        } else {
            display_group_ratio
        };

        let quota_type = item.get("quota_type").and_then(Value::as_i64).unwrap_or(0);
        let billing_mode = item
            .get("billing_mode")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let billing_expr = item
            .get("billing_expr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let is_dynamic_pricing = billing_mode == "tiered_expr" || !billing_expr.is_empty();
        let model_ratio = item
            .get("model_ratio")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .unwrap_or(0.0);
        let completion_ratio = item
            .get("completion_ratio")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .unwrap_or(0.0);
        let model_price = item
            .get("model_price")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .unwrap_or(0.0);
        let std_load_raw = item
            .get("standard_load_price_per_million")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);
        let realized_usd_per_m = item
            .get("realized_price_per_million")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);
        let cache_hit_rate = item
            .get("cache_hit_rate")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0 && *n <= 1.0);

        let has_usage_schema = item
            .get("billing_usage_schema")
            .and_then(Value::as_object)
            .is_some_and(|m| !m.is_empty());
        let is_non_free_special = matches!(
            model_name.to_ascii_lowercase().as_str(),
            "tako-auto" | "web_search" | "speech_tts" | "speech_asr"
        );
        let is_free = !is_dynamic_pricing
            && !has_usage_schema
            && !is_non_free_special
            && realized_usd_per_m.is_none()
            && ((quota_type == 1 && model_price <= 0.0)
                || (quota_type == 0 && model_ratio <= 0.0 && model_price <= 0.0));

        let standard_load_usd_per_m = if quota_type == 0 {
            std_load_raw
                .map(|base| base * display_group_ratio)
                .filter(|v| v.is_finite() && *v > 0.0)
        } else {
            None
        };
        let standard_load_max_usd_per_m = if quota_type == 0
            && display_group_ratio_max.is_finite()
            && display_group_ratio_max > display_group_ratio + 1e-9
        {
            std_load_raw
                .map(|base| base * display_group_ratio_max)
                .filter(|v| v.is_finite() && *v > 0.0)
        } else {
            None
        };
        let (tier_in, tier_cache, tier_out) = extract_first_tier_prices(item, billing_expr);
        let cache_ratio = item
            .get("cache_ratio")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n > 0.0);

        let input_usd_per_m = if quota_type == 0 && model_ratio > 0.0 {
            Some(model_ratio * 2.0 * display_group_ratio).filter(|v| v.is_finite() && *v > 0.0)
        } else if quota_type == 0 && is_dynamic_pricing {
            tier_in
                .map(|p| p * display_group_ratio)
                .filter(|v| v.is_finite() && *v > 0.0)
        } else {
            None
        };
        let cache_usd_per_m = if quota_type == 0 {
            if is_dynamic_pricing {
                tier_cache
                    .map(|p| p * display_group_ratio)
                    .or_else(|| {
                        input_usd_per_m.zip(cache_ratio).map(|(inp, cr)| inp * cr)
                    })
                    .filter(|v| v.is_finite() && *v > 0.0)
            } else {
                input_usd_per_m
                    .zip(cache_ratio)
                    .map(|(inp, cr)| inp * cr)
                    .filter(|v| v.is_finite() && *v > 0.0)
            }
        } else {
            None
        };
        let output_usd_per_m = if quota_type == 0 && model_ratio > 0.0 && completion_ratio > 0.0 {
            Some(model_ratio * 2.0 * completion_ratio * display_group_ratio)
                .filter(|v| v.is_finite() && *v > 0.0)
        } else if quota_type == 0 && is_dynamic_pricing {
            tier_out
                .map(|p| p * display_group_ratio)
                .filter(|v| v.is_finite() && *v > 0.0)
        } else {
            None
        };
        let request_usd = if quota_type == 1 && model_price > 0.0 {
            Some(model_price * display_group_ratio).filter(|v| v.is_finite() && *v > 0.0)
        } else {
            None
        };
        let channels: Vec<ModelChannelPrice> = item
            .get("channels")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|ch| {
                        let name = ch
                            .get("channel_name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())?
                            .to_string();
                        let ratio = ch
                            .get("channel_ratio")
                            .and_then(Value::as_f64)
                            .filter(|r| r.is_finite() && *r > 0.0)?;
                        let std_load = if quota_type == 0 {
                            ch.get("standard_load_price_per_million")
                                .and_then(Value::as_f64)
                                .filter(|v| v.is_finite() && *v > 0.0)
                        } else {
                            None
                        };
                        Some(ModelChannelPrice {
                            name,
                            ratio,
                            standard_load_usd_per_m: std_load,
                        })
                    })
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();

        let min_channel_ratio = item
            .get("channels")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .filter_map(|ch| {
                        ch.get("channel_ratio")
                            .and_then(Value::as_f64)
                            .filter(|r| r.is_finite() && *r > 0.0)
                    })
                    .reduce(f64::min)
            })
            .or_else(|| {
                if ch_min_map.is_some() && min_ratio.is_finite() && min_ratio > 0.0 {
                    Some(min_ratio)
                } else {
                    None
                }
            });

        let official_multiplier = if quota_type == 0
            && model_ratio > 0.0
            && !has_usage_schema
            && !is_dynamic_pricing
            && !has_tiered_plugin_variant(item)
        {
            min_channel_ratio
                .map(|r| r / OFFICIAL_PRICE_RATIO_BASE)
                .filter(|m| m.is_finite() && *m > 0.0)
        } else {
            None
        };

        let raw_desc = item
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        let tags = extract_item_tags(item);
        let has_levels = cache.reasoning_levels.contains_key(model_name);
        let capabilities =
            derive_model_capabilities(model_name, Some(item), &tags, raw_desc, has_levels);
        let context_window = extract_pricing_context_window(
            model_name,
            Some(item),
            &tags,
            raw_desc,
            &capabilities,
        );
        let promo_ends_on = resolve_promo_ends_on(model_name, Some(item));
        let vendor_name = infer_vendor_name(model_name, Some(item));

        cache.prices.insert(
            model_name.to_ascii_lowercase(),
            ModelPricingInfo {
                is_free,
                is_dynamic_pricing,
                standard_load_usd_per_m,
                standard_load_max_usd_per_m,
                realized_usd_per_m,
                cache_hit_rate,
                input_usd_per_m,
                cache_usd_per_m,
                output_usd_per_m,
                request_usd,
                quota_type,
                model_ratio: (model_ratio > 0.0).then_some(model_ratio),
                official_multiplier,
                promo_ends_on,
                capabilities,
                tags,
                vendor_name,
                context_window,
                channels,
            },
        );
    }
    cache
}

pub fn parse_perf_metrics_summary(body: &Value) -> HashMap<String, ModelPerfInfo> {
    let mut map = HashMap::new();
    let models = body
        .get("data")
        .and_then(|d| d.get("models"))
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array));
    let Some(models) = models else {
        return map;
    };
    for item in models {
        let Some(name) = item
            .get("model_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let avg_tps = item
            .get("avg_tps")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0);
        let avg_latency_ms = item
            .get("avg_latency_ms")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v > 0.0);
        let success_rate = item
            .get("success_rate")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0);
        if avg_tps.is_some() || avg_latency_ms.is_some() || success_rate.is_some() {
            map.insert(
                name.to_ascii_lowercase(),
                ModelPerfInfo {
                    avg_tps,
                    avg_latency_ms,
                    success_rate,
                },
            );
        }
    }
    map
}

pub fn effective_effort_levels(base_id: &str, pricing: &PricingCache) -> Option<Vec<String>> {
    if let Some(levels) = pricing.reasoning_levels.get(base_id) {
        if levels.len() >= 2 {
            return Some(levels.clone());
        }
    }
    if base_id == "gemini-3.1-pro-preview" || base_id == "gemini-3.1-pro" {
        return Some(vec!["low".to_string(), "high".to_string()]);
    }
    None
}

fn strip_effort_display_suffix(raw: &str) -> String {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    for eff in ALLOWED_EFFORTS {
        for pat in [format!(" ({eff})"), format!(" ({eff} thinking)")] {
            if lower.ends_with(&pat) && trimmed.len() > pat.len() {
                return trimmed[..trimmed.len() - pat.len()].trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

pub fn display_base_name(base_id: &str, orig_display: Option<&str>) -> String {
    let lower = base_id.trim().to_ascii_lowercase();
    match lower.as_str() {
        "gemini-4-argon" => "Gemini 4 Argon".to_string(),
        "gemini-4-carbon" => "Gemini 4 Carbon".to_string(),
        "gemini-4-hydrogen" => "Gemini 4 Hydrogen".to_string(),
        "gemini-4-neon" => "Gemini 4 Neon".to_string(),
        "gemini-3.8-flash" => "Gemini 3.8 Flash".to_string(),
        "gemini-3.7-flash" => "Gemini 3.7 Flash".to_string(),
        "gemini-3.6-flash" => "Gemini 3.6 Flash".to_string(),
        "gemini-3.5-flash" => "Gemini 3.5 Flash".to_string(),
        "gemini-3.5-flash-lite" => "Gemini 3.5 Flash Lite".to_string(),
        "gemini-3.1-pro-preview" | "gemini-3.1-pro" => "Gemini 3.1 Pro".to_string(),
        "gemini-3-pro-preview" | "gemini-3-pro" => "Gemini 3 Pro".to_string(),
        "gemini-3.1-flash-image" => "Gemini 3.1 Flash Image".to_string(),
        "gemini-3-pro-image" => "Gemini 3 Pro Image".to_string(),
        "claude-opus-5-5" => "Claude Opus 5.5".to_string(),
        "claude-sonnet-5-5" => "Claude Sonnet 5.5".to_string(),
        "swe-2" => "SWE-2".to_string(),
        "swe-2-flash" => "SWE-2 Flash".to_string(),
        _ => {
            if let Some(cand) = orig_display.map(str::trim).filter(|s| !s.is_empty()) {
                let stripped = strip_effort_display_suffix(cand);
                if !stripped.eq_ignore_ascii_case(&lower) && stripped.chars().any(char::is_uppercase)
                {
                    return stripped;
                }
            }
            pretty_model_display_name(base_id)
        }
    }
}

pub fn effort_label(effort: &str) -> &'static str {
    match effort {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "XHigh",
        "max" => "Max",
        _ => "Medium",
    }
}

/// Returns true for model slugs that match Antigravity's built-in `geminimodels` catalog or
/// native gateway image models, which do not consume the 32 custom placeholder slots.
pub fn is_antigravity_builtin_model(id: &str) -> bool {
    matches!(
        id.trim().to_ascii_lowercase().as_str(),
        "gemini-3.1-pro-low"
            | "gemini-3.1-pro-high"
            | "gemini-3.1-pro-preview"
            | "gemini-3.1-pro-preview-customtools"
            | "gemini-3.1-flash-lite-preview"
            | "gemini-3.1-flash-image"
            | "gemini-3-pro-image"
            | "gemini-3.5-flash-low"
            | "gemini-3.5-flash-medium"
            | "gemini-3.5-flash-high"
            | "gemini-3.6-flash-low"
            | "gemini-3.6-flash-medium"
            | "gemini-3.6-flash-high"
            | "gemini-3.7-flash-low"
            | "gemini-3.7-flash-medium"
            | "gemini-3.7-flash-high"
            | "gemini-3.8-flash-low"
            | "gemini-3.8-flash-medium"
            | "gemini-3.8-flash-high"
            | "gemini-4-argon-low"
            | "gemini-4-argon-medium"
            | "gemini-4-argon-high"
            | "gemini-4-carbon-low"
            | "gemini-4-carbon-medium"
            | "gemini-4-carbon-high"
            | "gemini-4-hydrogen-low"
            | "gemini-4-hydrogen-medium"
            | "gemini-4-hydrogen-high"
            | "gemini-4-hydrogen-xhigh"
    )
}

fn split_effort_slug(id: &str) -> (&str, u8) {
    let lower = id.trim();
    for (idx, effort) in ALLOWED_EFFORTS.iter().enumerate() {
        let suffix_len = effort.len() + 1;
        if lower.len() > suffix_len
            && lower.as_bytes()[lower.len() - suffix_len] == b'-'
            && lower[lower.len() - effort.len()..].eq_ignore_ascii_case(effort)
        {
            return (&lower[..lower.len() - suffix_len], idx as u8);
        }
    }
    (lower, 0)
}

fn antigravity_model_sort_key(id: &str) -> (u16, u8, String) {
    let lower_id = id.trim().to_ascii_lowercase();
    let (base, eff_rank) = split_effort_slug(&lower_id);
    let family_prio: u16 = match base {
        "gemini-4-neon" => 10,
        "gemini-4-hydrogen" => 11,
        "gemini-4-carbon" => 12,
        "gemini-4-argon" => 13,
        "gemini-3.8-flash" => 14,
        "gemini-3.7-flash" => 15,
        "gemini-3.6-flash" => 16,
        "gemini-3.5-flash" => 17,
        "gemini-3.1-pro" | "gemini-3.1-pro-preview" => 18,
        "gemini-3-pro" | "gemini-3-pro-preview" => 19,
        "gemini-3.5-flash-lite" => 20,
        "claude-opus-5-5" => 30,
        "claude-opus-5" => 31,
        "claude-opus-4-8" => 32,
        "claude-opus-4-7" => 33,
        "claude-opus-4-6" => 34,
        "claude-sonnet-5-5" => 35,
        "claude-sonnet-5" => 36,
        "claude-sonnet-4-6" => 37,
        "claude-haiku-5-5" => 38,
        "claude-fable-5-2" => 39,
        "claude-haiku-4-5" => 40,
        "gpt-6.2-sol" => 50,
        "gpt-6.2-astra" => 51,
        "gpt-6.1-sol" => 52,
        "gpt-6-astra" => 53,
        "gpt-6.1-terra" => 54,
        "gpt-6-sol" => 55,
        "gpt-6-luna" => 56,
        "deepseek-v4-pro" => 60,
        "deepseek-v4.1-flash" => 61,
        "deepseek-v4-flash" => 62,
        "kimi-k3.1-pro" => 65,
        "kimi-k3.1-pro-highspeed" => 66,
        "kimi-k3" => 67,
        "glm-5.3" => 70,
        "glm-5.3-flash" => 71,
        "grok-4.7" => 75,
        "grok-4.6" => 76,
        "grok-4.5" => 77,
        "minimax-m3" => 80,
        "mimo-v2.6-pro" => 81,
        "mimo-v2.6-flash" => 82,
        "swe-2" => 85,
        "swe-2-flash" => 86,
        "composer-2.5" => 88,
        "hy3" => 89,
        "gemma-4-31b-it" => 90,
        "tako-auto" => 91,
        "gemini-3.1-flash-image" | "gemini-3-pro-image" => 98,
        _ if base.starts_with("claude-") => 45,
        _ if base.starts_with("gpt-") => 58,
        _ if base.starts_with("deepseek-") => 63,
        _ => 94,
    };
    (family_prio, eff_rank, base.to_string())
}

struct AgyBaseCandidate {
    base_id: String,
    variant_prefix: String,
    display_name: String,
    effort_levels: Vec<String>,
    raw_obj: Map<String, Value>,
}

fn collect_antigravity_base_candidates(body: &Value, pricing: &PricingCache) -> Vec<AgyBaseCandidate> {
    let Some(models) = body.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut all_base_ids = HashSet::new();
    for item in models {
        if let Some(raw_name) = item
            .as_object()
            .and_then(|o| o.get("name"))
            .and_then(Value::as_str)
        {
            let base_id = raw_name.strip_prefix("models/").unwrap_or(raw_name).trim();
            if !base_id.is_empty() {
                all_base_ids.insert(base_id.to_string());
            }
        }
    }

    let mut candidates = Vec::with_capacity(models.len() + 2);
    let mut seen_base = HashSet::new();
    let mut has_flash_image_entry = false;

    for item in models {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let raw_name = obj.get("name").and_then(Value::as_str).unwrap_or("");
        let base_id = raw_name.strip_prefix("models/").unwrap_or(raw_name).trim();
        if base_id.is_empty() {
            continue;
        }
        let lower_base = base_id.to_ascii_lowercase();
        if lower_base == "codex-auto-review"
            || lower_base == "hy4flash-free"
            || lower_base.starts_with("jev-")
            || is_duplicate_alias(base_id, &all_base_ids)
        {
            continue;
        }
        if lower_base == "gemini-3.1-flash-image" {
            has_flash_image_entry = true;
        }
        if !seen_base.insert(lower_base) {
            continue;
        }

        let effort_levels = effective_effort_levels(base_id, pricing).unwrap_or_default();
        let display_name = display_base_name(
            base_id,
            obj.get("displayName").and_then(Value::as_str),
        );
        // Emit Antigravity's built-in slug `gemini-3.1-pro-{low,high}` for
        // `gemini-3.1-pro-preview` so it takes 0 custom placeholder slots.
        let variant_prefix = if base_id == "gemini-3.1-pro-preview" {
            "gemini-3.1-pro".to_string()
        } else {
            base_id.to_string()
        };

        candidates.push(AgyBaseCandidate {
            base_id: base_id.to_string(),
            variant_prefix,
            display_name,
            effort_levels,
            raw_obj: obj.clone(),
        });
    }

    if pricing.has_flash_image
        && !has_flash_image_entry
        && seen_base.insert("gemini-3.1-flash-image".to_string())
    {
        let raw_val = json!({
            "name": "models/gemini-3.1-flash-image",
            "displayName": "Gemini 3.1 Flash Image",
            "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
        });
        if let Some(raw_obj) = raw_val.as_object() {
            candidates.push(AgyBaseCandidate {
                base_id: "gemini-3.1-flash-image".to_string(),
                variant_prefix: "gemini-3.1-flash-image".to_string(),
                display_name: "Gemini 3.1 Flash Image".to_string(),
                effort_levels: Vec::new(),
                raw_obj: raw_obj.clone(),
            });
        }
    }

    candidates.sort_by_key(|c| antigravity_model_sort_key(&c.base_id));
    candidates
}

fn antigravity_candidate_matches(
    cand: &AgyBaseCandidate,
    allowed: &HashSet<String>,
) -> bool {
    let lower_base = cand.base_id.to_ascii_lowercase();
    let lower_prefix = cand.variant_prefix.to_ascii_lowercase();
    allowed.contains(&lower_base)
        || allowed.contains(&lower_prefix)
        || (lower_base == "gemini-3.1-pro" && allowed.contains("gemini-3.1-pro-preview"))
}

fn resolve_antigravity_selected_base_ids(
    candidates: &[AgyBaseCandidate],
    filter: &ClientModelFilter,
) -> HashSet<String> {
    let Some(allowed) = normalized_filter_set(AppId::Antigravity, filter) else {
        return candidates
            .iter()
            .map(|c| c.base_id.to_ascii_lowercase())
            .collect();
    };
    let mut selected: HashSet<String> = candidates
        .iter()
        .filter(|c| antigravity_candidate_matches(c, &allowed))
        .map(|c| c.base_id.to_ascii_lowercase())
        .collect();
    if selected.is_empty() && !candidates.is_empty() {
        for cand in candidates.iter().take(3) {
            selected.insert(cand.base_id.to_ascii_lowercase());
        }
    }
    selected
}

pub fn expand_models_catalog(
    body: Value,
    pricing: &PricingCache,
) -> (Value, HashMap<String, SyntheticRoute>) {
    expand_models_catalog_with_filter(body, pricing, None)
}

pub fn expand_models_catalog_with_filter(
    mut body: Value,
    pricing: &PricingCache,
    filter: Option<&ClientModelFilter>,
) -> (Value, HashMap<String, SyntheticRoute>) {
    let mut routes = HashMap::new();
    if body.get("models").and_then(Value::as_array).is_none() {
        return (body, routes);
    }

    let candidates = collect_antigravity_base_candidates(&body, pricing);
    let selected_set = filter.map(|f| resolve_antigravity_selected_base_ids(&candidates, f));

    struct ExpandedEntry {
        id: String,
        value: Value,
        builtin: bool,
    }

    let mut entries: Vec<ExpandedEntry> = Vec::with_capacity(candidates.len() * 2 + 4);
    let mut seen_ids = HashSet::new();

    for cand in candidates {
        let keep_candidate = selected_set
            .as_ref()
            .map(|set| set.contains(&cand.base_id.to_ascii_lowercase()))
            .unwrap_or(true);

        if !cand.effort_levels.is_empty() {
            for effort in cand.effort_levels {
                let synth_id = format!("{}-{effort}", cand.variant_prefix);
                let route = SyntheticRoute {
                    base_model: cand.base_id.clone(),
                    effort: effort.clone(),
                };
                routes.insert(synth_id.clone(), route.clone());
                if cand.variant_prefix != cand.base_id {
                    routes.insert(format!("{}-{effort}", cand.base_id), route);
                }

                if keep_candidate && seen_ids.insert(synth_id.clone()) {
                    let mut variant = cand.raw_obj.clone();
                    variant.insert("name".to_string(), Value::String(format!("models/{synth_id}")));
                    variant.insert(
                        "displayName".to_string(),
                        Value::String(format!("{} ({})", cand.display_name, effort_label(&effort))),
                    );
                    variant.insert("thinking".to_string(), Value::Bool(true));
                    entries.push(ExpandedEntry {
                        builtin: is_antigravity_builtin_model(&synth_id),
                        id: synth_id,
                        value: Value::Object(variant),
                    });
                }
            }
        } else if keep_candidate && seen_ids.insert(cand.base_id.clone()) {
            let mut next_obj = cand.raw_obj;
            next_obj.insert("displayName".to_string(), Value::String(cand.display_name));
            entries.push(ExpandedEntry {
                builtin: is_antigravity_builtin_model(&cand.base_id),
                id: cand.base_id,
                value: Value::Object(next_obj),
            });
        }
    }

    let mut builtins = Vec::new();
    let mut customs = Vec::new();
    for entry in entries {
        if entry.builtin {
            builtins.push(entry);
        } else {
            customs.push(entry);
        }
    }

    // Antigravity language_server allocates at most 32 custom placeholder model enums;
    // keep the highest-priority custom models so flagship entries are never silently dropped.
    customs.sort_by_key(|e| antigravity_model_sort_key(&e.id));
    customs.truncate(AGY_MAX_CUSTOM_PLACEHOLDERS);

    let mut kept: Vec<ExpandedEntry> = Vec::with_capacity(builtins.len() + customs.len());
    kept.extend(builtins);
    kept.extend(customs);
    kept.sort_by_key(|e| antigravity_model_sort_key(&e.id));

    let expanded: Vec<Value> = kept.into_iter().map(|e| e.value).collect();

    if let Some(root) = body.as_object_mut() {
        root.insert("models".to_string(), Value::Array(expanded));
    }

    (body, routes)
}

pub fn parse_gemini_action_path(path: &str) -> Option<(String, String)> {
    let rest = path.strip_prefix("/v1beta/models/")?;
    let colon_idx = rest.rfind(':')?;
    let model = rest[..colon_idx].trim();
    let action = rest[colon_idx + 1..].trim();
    if model.is_empty() || action.is_empty() {
        return None;
    }
    Some((model.to_string(), action.to_string()))
}

pub fn resolve_synthetic_route(
    model: &str,
    routes: &HashMap<String, SyntheticRoute>,
    pricing: &PricingCache,
) -> Option<SyntheticRoute> {
    if let Some(hit) = routes.get(model) {
        return Some(hit.clone());
    }
    for effort in ["xhigh", "medium", "high", "low", "max"] {
        let suffix = format!("-{effort}");
        if let Some(base) = model.strip_suffix(&suffix) {
            if let Some(levels) = effective_effort_levels(base, pricing) {
                if levels.iter().any(|l| l == effort) {
                    let target_base = if base == "gemini-3.1-pro"
                        && !pricing.descriptions.contains_key("gemini-3.1-pro")
                    {
                        "gemini-3.1-pro-preview"
                    } else {
                        base
                    };
                    return Some(SyntheticRoute {
                        base_model: target_base.to_string(),
                        effort: effort.to_string(),
                    });
                }
            }
        }
    }
    None
}

pub fn inject_thinking_effort(body_bytes: &[u8], effort: &str) -> Vec<u8> {
    let Ok(mut root) = serde_json::from_slice::<Value>(body_bytes) else {
        return body_bytes.to_vec();
    };
    let Some(root_obj) = root.as_object_mut() else {
        return body_bytes.to_vec();
    };

    let gen_cfg = root_obj
        .entry("generationConfig".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !gen_cfg.is_object() {
        *gen_cfg = Value::Object(Map::new());
    }
    let Some(gen_obj) = gen_cfg.as_object_mut() else {
        return body_bytes.to_vec();
    };

    let thinking_cfg = gen_obj
        .entry("thinkingConfig".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !thinking_cfg.is_object() {
        *thinking_cfg = Value::Object(Map::new());
    }
    if let Some(tc_obj) = thinking_cfg.as_object_mut() {
        tc_obj.remove("thinkingBudget");
        tc_obj.remove("thinking_budget");
        tc_obj.remove("thinking_level");
        tc_obj.insert(
            "thinkingLevel".to_string(),
            Value::String(effort.to_ascii_uppercase()),
        );
        if !tc_obj.contains_key("includeThoughts") && !tc_obj.contains_key("include_thoughts") {
            tc_obj.insert("includeThoughts".to_string(), Value::Bool(true));
        }
    }

    serde_json::to_vec(&root).unwrap_or_else(|_| body_bytes.to_vec())
}

fn title_case_token(tok: &str) -> String {
    let mut chars = tok.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(tok.len());
    out.extend(first.to_uppercase());
    out.push_str(&chars.as_str().to_ascii_lowercase());
    out
}

fn format_general_token(tok: &str) -> String {
    let lower = tok.to_ascii_lowercase();
    if lower.is_empty() {
        return String::new();
    }
    if lower.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return lower;
    }
    if let Some(rest) = lower.strip_prefix('v') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return format!("V{rest}");
        }
    }
    if let Some(rest) = lower.strip_prefix('m') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return format!("M{rest}");
        }
    }
    if let Some(rest) = lower.strip_prefix('k') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return format!("K{rest}");
        }
    }
    if let Some(num) = lower.strip_suffix('b') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return format!("{num}B");
        }
    }
    if lower == "oss" {
        return "OSS".to_string();
    }
    title_case_token(&lower)
}

fn extract_endpoint_types(item: &Value) -> Vec<String> {
    item.get("supported_endpoint_types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// Gemini-family models are only exposed through Antigravity; Claude and Codex catalogs skip them.
fn is_gemini_family(id: &str, owned_by: &str) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    let owner = owned_by.trim().to_ascii_lowercase();
    lower.starts_with("gemini-")
        || lower.starts_with("gemma-")
        || lower.starts_with("imagen-")
        || owner.contains("gemini")
        || owner.contains("google")
}

/// Models kept out of the Claude and Codex catalogs; Antigravity keeps them.
fn is_client_excluded_model(id: &str, owned_by: &str) -> bool {
    is_gemini_family(id, owned_by) || id.trim().eq_ignore_ascii_case("deepseek-v4-pro")
}

fn is_non_chat_or_noise_model(id: &str, eps: &[String]) -> bool {
    let lower = id.trim().to_ascii_lowercase();
    if lower.is_empty()
        || lower == "hy4flash-free"
        || lower.starts_with("jev-")
        || lower.contains("image")
        || lower.contains("video")
        || lower.contains("audio")
        || lower.contains("tts")
        || lower.contains("whisper")
        || lower.contains("embedding")
        || lower.contains("speech")
        || lower.contains("asr")
        || lower.contains("rerank")
        || lower.contains("moderation")
        || lower.contains("systemone")
    {
        return true;
    }
    if !eps.is_empty() {
        let has_chat_ep = eps.iter().any(|ep| {
            matches!(
                ep.as_str(),
                "openai" | "openai-response" | "anthropic" | "gemini"
            )
        });
        if !has_chat_ep {
            return true;
        }
    }
    false
}

fn strip_dated_snapshot_suffix(id: &str) -> Option<&str> {
    let idx = id.rfind('-')?;
    let suffix = &id[idx + 1..];
    if !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if suffix.len() == 8 {
        return Some(&id[..idx]);
    }
    if suffix.len() == 4 {
        let mm: u32 = suffix[..2].parse().ok()?;
        let dd: u32 = suffix[2..].parse().ok()?;
        if (1..=12).contains(&mm) && (1..=31).contains(&dd) {
            return Some(&id[..idx]);
        }
    }
    None
}

fn is_duplicate_alias(id: &str, all_ids: &HashSet<String>) -> bool {
    if id.eq_ignore_ascii_case("k3") && all_ids.contains("kimi-k3") {
        return true;
    }
    if let Some(base) = strip_dated_snapshot_suffix(id) {
        if all_ids.contains(base) {
            return true;
        }
    }
    false
}

pub fn pretty_model_display_name(id: &str) -> String {
    let lower = id.trim().to_ascii_lowercase();
    if lower.starts_with("claude-") {
        let (tier, major, minor) = parse_claude_tier_and_version(&lower);
        return claude_display_name(id, tier, major, minor);
    }
    if let Some(rest) = lower.strip_prefix("gpt-") {
        let parts: Vec<String> = rest
            .split('-')
            .map(|seg| match seg {
                "4o" => "4o".to_string(),
                _ => format_general_token(seg),
            })
            .collect();
        if parts.is_empty() {
            return "GPT".to_string();
        }
        let first_is_ver = parts[0].chars().any(|c| c.is_ascii_digit());
        if parts.len() == 1 || !first_is_ver {
            return format!("GPT-{}", parts.join(" "));
        }
        return format!("GPT-{} {}", parts[0], parts[1..].join(" "));
    }
    if let Some(rest) = lower.strip_prefix("deepseek-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("DeepSeek {}", parts.join(" "));
    }
    match lower.as_str() {
        "gemini-3.1-pro-preview" | "gemini-3.1-pro" => return "Gemini 3.1 Pro".to_string(),
        "gemini-3.8-flash" => return "Gemini 3.8 Flash".to_string(),
        "gemini-3.5-flash-lite" => return "Gemini 3.5 Flash Lite".to_string(),
        "gemma-4-31b-it" => return "Gemma 4 31B".to_string(),
        "swe-2" => return "SWE-2".to_string(),
        "swe-2-flash" => return "SWE-2 Flash".to_string(),
        "hy3" => return "Hunyuan Hy3".to_string(),
        "composer-2.5" => return "Composer 2.5".to_string(),
        "tako-auto" => return "Tako Auto".to_string(),
        _ => {}
    }
    if let Some(rest) = lower.strip_prefix("gemini-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("Gemini {}", parts.join(" "));
    }
    if let Some(rest) = lower.strip_prefix("gemma-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("Gemma {}", parts.join(" "));
    }
    if let Some(rest) = lower.strip_prefix("glm-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        if parts.len() <= 1 {
            return format!("GLM-{}", parts.join(" "));
        }
        return format!("GLM-{} {}", parts[0], parts[1..].join(" "));
    }
    if let Some(rest) = lower.strip_prefix("kimi-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("Kimi {}", parts.join(" "));
    }
    if let Some(rest) = lower.strip_prefix("grok-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("Grok {}", parts.join(" "));
    }
    if let Some(rest) = lower.strip_prefix("minimax-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("MiniMax {}", parts.join(" "));
    }
    if let Some(rest) = lower.strip_prefix("mimo-") {
        let parts: Vec<String> = rest.split('-').map(format_general_token).collect();
        return format!("MiMo {}", parts.join(" "));
    }
    id.split('-')
        .map(format_general_token)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn model_context_window(id: &str) -> u64 {
    let lower = id.trim().to_ascii_lowercase();
    match lower.as_str() {
        "claude-haiku-4-5" => 200_000,
        "hy3" => 256_000,
        "swe-2" | "swe-2-flash" => 262_000,
        "gpt-5.5" => 1_100_000,
        "gpt-6-astra" | "gpt-6.1-terra" => 1_050_000,
        "gpt-4o-mini" => 128_000,
        _ if lower.starts_with("gpt-5.6") => 272_000,
        _ if lower.starts_with("grok-4.") => 500_000,
        _ if lower.starts_with("gemma-") => 128_000,
        _ => 1_000_000,
    }
}

fn is_boilerplate_clause(clause: &str) -> bool {
    let trimmed = clause.trim();
    if trimmed.is_empty() {
        return true;
    }
    const BOILERPLATE_SUBSTRINGS: &[&str] = &[
        "仅支持 Claude Code",
        "其他客户端上游会报错",
        "反代渠道",
        "订阅池渠道",
        "不要和",
        "挂在官方订阅渠道",
        "官方与自部署",
        "官方价",
        "缓存输入",
        "缓存读",
        "价格更便宜",
        "上游短名",
        "优先用",
        "同一模型",
    ];
    if BOILERPLATE_SUBSTRINGS
        .iter()
        .any(|pat| trimmed.contains(pat))
    {
        return true;
    }
    // Pure context-length suffix like "上下文 1M" or "上下文 1.05M" or "1M 上下文"
    if (trimmed.starts_with("上下文") || trimmed.ends_with("上下文"))
        && trimmed.chars().count() <= 14
    {
        return true;
    }
    false
}

fn looks_like_model_name_prefix(clause: &str, id: &str) -> bool {
    let trimmed = clause.trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return false;
    }
    let has_cjk = trimmed
        .chars()
        .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
    if !has_cjk {
        return true;
    }
    let id_head = id
        .split(['-', '.', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    !id_head.is_empty()
        && trimmed.to_ascii_lowercase().starts_with(&id_head)
        && trimmed.chars().count() <= 24
}

pub fn concise_model_description(id: &str, raw_desc: Option<&str>) -> String {
    let lower = id.trim().to_ascii_lowercase();
    let curated = match lower.as_str() {
        "claude-opus-5-5" => Some("1M · 最新旗舰推理与编码"),
        "claude-opus-5" => Some("1M · 旗舰推理与编码"),
        "claude-opus-4-8" => Some("1M · 攻坚推理与长程智能体"),
        "claude-opus-4-7" => Some("1M · 软件工程与高阶推理"),
        "claude-opus-4-6" => Some("1M · 高阶编码与规划"),
        "claude-sonnet-5-5" => Some("1M · 最新均衡主力档"),
        "claude-sonnet-5" => Some("1M · 日常编码与规划主力"),
        "claude-sonnet-4-6" => Some("1M · 生产向编码智能体"),
        "claude-fable-5-2" | "claude-fable-5-1" | "claude-fable-5" => {
            Some("1M · 创作、写作与分析")
        }
        "claude-haiku-5-5" => Some("1M · 轻量高速响应"),
        "claude-haiku-4-5" => Some("200K · 极速轻量助手"),
        "deepseek-v4-pro" => Some("1M · 开源 MoE 旗舰编码"),
        "deepseek-v4.1-flash" => Some("1M · 视觉与工具调用高速档"),
        "deepseek-v4-flash" => Some("1M · 高性价比推理编码"),
        "gpt-6.2-sol" => Some("1M · 6.2 编程与智能体主力"),
        "gpt-6.2-astra" => Some("1M · 6.2 端到端旗舰"),
        "gpt-6.1-sol" => Some("1M · 6.1 编程与智能体主力"),
        "gpt-6.1-terra" => Some("1M · 视觉与电脑操控档"),
        "gpt-6-astra" => Some("1M · 端到端复杂工作旗舰"),
        "gpt-6-sol" => Some("1M · 6 代编程主力档"),
        "gpt-6-luna" => Some("1M · 6 代高性价比日常档"),
        "gpt-5.6-sol" => Some("272K · 5.6 专业编码主力"),
        "gpt-5.6-terra" => Some("272K · 5.6 均衡日常档"),
        "gpt-5.6-luna" => Some("272K · 5.6 高性价比档"),
        "gpt-5.6" => Some("272K · 5.6 综合编码档"),
        "gpt-5.5" => Some("1.1M · 编码与研究通用档"),
        "gpt-4o-mini" => Some("轻量通用快速档"),
        "gemini-3.8-flash" => Some("1M · 高效多模态与编码"),
        "gemini-3.1-pro-preview" | "gemini-3.1-pro" => Some("1M · 高阶推理与多模态"),
        "gemini-3.5-flash-lite" => Some("1M · 轻量高吞吐日常档"),
        "gemma-4-31b-it" => Some("开源指令模型（免费档）"),
        "glm-5.3" => Some("1M · 旗舰长程编码与交付"),
        "glm-5.3-flash" => Some("1M · 多模态高效编码"),
        "kimi-k3.1-pro" => Some("1M · 长上下文深度推理"),
        "kimi-k3.1-pro-highspeed" => Some("1M · 高速深度推理"),
        "kimi-k3" => Some("1M · 多模态长程智能体"),
        "grok-4.7" => Some("500K · 最新前沿编码与看图"),
        "grok-4.6" => Some("500K · 长程智能体与看图"),
        "grok-4.5" => Some("500K · 低幻觉对话与编码"),
        "minimax-m3" => Some("1M · 多模态编码与推理"),
        "mimo-v2.6-pro" => Some("1M · 旗舰长程智能体"),
        "mimo-v2.6-flash" => Some("1M · 高性价比 MoE 档"),
        "swe-2" => Some("262K · 专精编码智能体"),
        "swe-2-flash" => Some("262K · 快速编码智能体"),
        "composer-2.5" => Some("高速代码编辑与生成"),
        "hy3" => Some("256K · 混元 MoE 通用档"),
        "tako-auto" => Some("免费模型自动调度池"),
        "codex-auto-review" => Some("代码评审自动路由"),
        _ => None,
    };
    if let Some(text) = curated {
        return text.to_string();
    }

    if let Some(raw) = raw_desc.map(str::trim).filter(|s| !s.is_empty()) {
        for (idx, sent) in raw.split('。').enumerate() {
            let trimmed = sent.trim();
            if trimmed.is_empty() {
                continue;
            }
            if idx == 0 && looks_like_model_name_prefix(trimmed, &lower) {
                continue;
            }
            if is_boilerplate_clause(trimmed) {
                continue;
            }
            for sub in trimmed.split(['，', ',']) {
                let s = sub.trim();
                if idx == 0 && looks_like_model_name_prefix(s, &lower) {
                    continue;
                }
                if is_boilerplate_clause(s) {
                    continue;
                }
                return s.chars().take(26).collect();
            }
        }
    }

    "Tako 托管模型".to_string()
}

fn third_party_family_order(id: &str) -> (i64, String) {
    let lower = id.trim().to_ascii_lowercase();
    if lower.starts_with("deepseek-") {
        let sub = match lower.as_str() {
            "deepseek-v4-pro" => 0,
            "deepseek-v4.1-flash" => 1,
            "deepseek-v4-flash" => 2,
            _ => 9,
        };
        return (10, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("gpt-") {
        let prio = gpt_model_sub_priority(&lower);
        return (20, format!("{prio:03}_{lower}"));
    }
    if lower.starts_with("gemini-") || lower.starts_with("gemma-") {
        let sub = match lower.as_str() {
            "gemini-3.8-flash" => 0,
            "gemini-3.1-pro-preview" | "gemini-3.1-pro" => 1,
            "gemini-3.5-flash-lite" => 2,
            "gemma-4-31b-it" => 3,
            _ => 9,
        };
        return (30, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("glm-") {
        let sub = match lower.as_str() {
            "glm-5.3" => 0,
            "glm-5.3-flash" => 1,
            _ => 9,
        };
        return (40, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("kimi-") {
        let sub = match lower.as_str() {
            "kimi-k3.1-pro" => 0,
            "kimi-k3.1-pro-highspeed" => 1,
            "kimi-k3" => 2,
            _ => 9,
        };
        return (50, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("grok-") {
        let sub = match lower.as_str() {
            "grok-4.7" => 0,
            "grok-4.6" => 1,
            "grok-4.5" => 2,
            _ => 9,
        };
        return (60, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("minimax-") {
        return (70, lower);
    }
    if lower.starts_with("mimo-") {
        let sub = match lower.as_str() {
            "mimo-v2.6-pro" => 0,
            "mimo-v2.6-flash" => 1,
            _ => 9,
        };
        return (80, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("swe-") {
        let sub = match lower.as_str() {
            "swe-2" => 0,
            "swe-2-flash" => 1,
            _ => 9,
        };
        return (90, format!("{sub:02}_{lower}"));
    }
    if lower.starts_with("composer-") {
        return (95, lower);
    }
    if lower == "hy3" {
        return (96, lower);
    }
    if lower == "tako-auto" {
        return (98, lower);
    }
    (99, lower)
}

fn gpt_model_sub_priority(id: &str) -> i64 {
    match id {
        "gpt-6.2-sol" => 1,
        "gpt-6.2-astra" => 2,
        "gpt-6.1-sol" => 3,
        "gpt-6-astra" => 4,
        "gpt-6.1-terra" => 5,
        "gpt-6-sol" => 6,
        "gpt-6-luna" => 9,
        "gpt-5.6-sol" => 10,
        "gpt-5.6-terra" => 11,
        "gpt-5.6-luna" => 12,
        "gpt-5.6" => 13,
        "gpt-5.5" => 14,
        "gpt-4o-mini" => 17,
        _ => 18,
    }
}

fn codex_model_sort_key(id: &str) -> (i64, u8, Reverse<u32>, Reverse<u32>, String) {
    let lower = id.trim().to_ascii_lowercase();
    let prio = match lower.as_str() {
        "gpt-6.2-sol" => 1,
        "gpt-6.2-astra" => 2,
        "gpt-6.1-sol" => 3,
        "gpt-6-astra" => 4,
        "gpt-6.1-terra" => 5,
        "gpt-6-sol" => 6,
        "swe-2" => 7,
        "swe-2-flash" => 8,
        "gpt-6-luna" => 9,
        "gpt-5.6-sol" => 10,
        "gpt-5.6-terra" => 11,
        "gpt-5.6-luna" => 12,
        "gpt-5.6" => 13,
        "gpt-5.5" => 14,
        "gpt-4o-mini" => 17,
        "codex-auto-review" => 99,
        _ if lower.starts_with("gpt-") => 18,
        _ if lower.starts_with("claude-") => {
            let (tier, major, minor) = parse_claude_tier_and_version(&lower);
            return (
                20,
                claude_tier_rank(tier),
                Reverse(major),
                Reverse(minor),
                lower,
            );
        }
        _ if lower.starts_with("deepseek-") => 25,
        _ if lower.starts_with("gemini-") || lower.starts_with("gemma-") => 28,
        _ if lower.starts_with("glm-") => 30,
        _ if lower.starts_with("kimi-") => 32,
        _ if lower.starts_with("grok-") => 34,
        _ if lower.starts_with("minimax-") => 36,
        _ if lower.starts_with("mimo-") => 38,
        _ if lower.starts_with("composer-") => 40,
        _ if lower == "hy3" => 42,
        _ if lower == "tako-auto" => 45,
        _ => 50,
    };
    let (fam, sub) = third_party_family_order(&lower);
    let _ = fam;
    (prio, 0, Reverse(0), Reverse(0), sub)
}

fn effort_description(effort: &str) -> &'static str {
    match effort {
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth",
        "high" => "Greater reasoning depth for complex problems",
        "xhigh" => "Extra high reasoning depth",
        "max" => "Maximum reasoning depth",
        _ => "Balanced reasoning depth",
    }
}

fn codex_efforts_for_model(id: &str, pricing: &PricingCache) -> (&'static str, Vec<&'static str>) {
    if let Some(levels) = pricing.reasoning_levels.get(id) {
        let mapped: Vec<&'static str> = levels
            .iter()
            .filter_map(|lvl| match lvl.as_str() {
                "low" => Some("low"),
                "medium" => Some("medium"),
                "high" => Some("high"),
                "xhigh" => Some("xhigh"),
                "max" => Some("max"),
                _ => None,
            })
            .collect();
        if !mapped.is_empty() {
            let default_lvl = if mapped.contains(&"high") {
                "high"
            } else if mapped.contains(&"medium") {
                "medium"
            } else {
                mapped[0]
            };
            return (default_lvl, mapped);
        }
    }

    if id.starts_with("gpt-6")
        || id.starts_with("gpt-5")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id == "codex-auto-review"
    {
        let default_lvl = if id.ends_with("-sol") { "low" } else { "medium" };
        return (
            default_lvl,
            vec!["low", "medium", "high", "xhigh", "max"],
        );
    }

    ("medium", vec!["low", "medium", "high"])
}

/// Converts upstream OpenAI `/v1/models` + `/api/pricing` metadata into Codex's `ModelsResponse` schema.
pub fn build_codex_models_catalog(
    upstream_models: &Value,
    pricing: &PricingCache,
    cached_model_messages: Option<&Value>,
) -> Value {
    let raw_items = upstream_models
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut all_ids = HashSet::new();
    let mut candidate_ids = Vec::new();
    for item in &raw_items {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let eps = extract_endpoint_types(item);
        let owned_by = item
            .get("owned_by")
            .and_then(Value::as_str)
            .unwrap_or("");
        if is_client_excluded_model(id, owned_by) {
            continue;
        }
        let supports_chat_or_responses = eps.iter().any(|ep| {
            matches!(
                ep.as_str(),
                "openai-response" | "openai" | "anthropic" | "gemini"
            )
        });
        if !supports_chat_or_responses || is_non_chat_or_noise_model(id, &eps) {
            continue;
        }
        if all_ids.insert(id.to_string()) {
            candidate_ids.push(id.to_string());
        }
    }

    let mut ids: Vec<String> = candidate_ids
        .into_iter()
        .filter(|id| !is_duplicate_alias(id, &all_ids))
        .collect();

    ids.sort_by_key(|a| codex_model_sort_key(a));

    let valid_mm = cached_model_messages.filter(|mm| {
        mm.as_object()
            .map(|o| !o.is_empty())
            .unwrap_or(false)
    });

    let models: Vec<Value> = ids
        .into_iter()
        .map(|id| {
            let priority = codex_model_sort_key(&id).0;
            let visibility = if id == "codex-auto-review" {
                "hide"
            } else {
                "list"
            };
            let (default_lvl, levels) = codex_efforts_for_model(&id, pricing);
            let supported_reasoning_levels: Vec<Value> = levels
                .into_iter()
                .map(|eff| {
                    json!({
                        "effort": eff,
                        "description": effort_description(eff),
                    })
                })
                .collect();
            let description = concise_model_description(
                &id,
                pricing.descriptions.get(&id).map(String::as_str),
            );
            let ctx_window = model_context_window(&id);

            let mut obj = Map::new();
            obj.insert("slug".to_string(), Value::String(id.clone()));
            obj.insert(
                "display_name".to_string(),
                Value::String(pretty_model_display_name(&id)),
            );
            obj.insert("description".to_string(), Value::String(description));
            obj.insert(
                "default_reasoning_level".to_string(),
                Value::String(default_lvl.to_string()),
            );
            obj.insert(
                "supported_reasoning_levels".to_string(),
                Value::Array(supported_reasoning_levels),
            );
            obj.insert(
                "shell_type".to_string(),
                Value::String("unified_exec".to_string()),
            );
            obj.insert(
                "visibility".to_string(),
                Value::String(visibility.to_string()),
            );
            obj.insert("supported_in_api".to_string(), Value::Bool(true));
            obj.insert("priority".to_string(), json!(priority));
            obj.insert("prefer_websockets".to_string(), Value::Bool(false));
            obj.insert(
                "base_instructions".to_string(),
                Value::String(DEFAULT_CODEX_BASE_INSTRUCTIONS.to_string()),
            );
            obj.insert(
                "input_modalities".to_string(),
                json!(["text", "image"]),
            );
            obj.insert(
                "truncation_policy".to_string(),
                json!({"mode": "tokens", "limit": 10000}),
            );
            obj.insert(
                "apply_patch_tool_type".to_string(),
                Value::String("freeform".to_string()),
            );
            obj.insert(
                "web_search_tool_type".to_string(),
                Value::String("text_and_image".to_string()),
            );
            obj.insert("context_window".to_string(), json!(ctx_window));
            obj.insert("effective_context_window_percent".to_string(), json!(95));
            obj.insert("support_verbosity".to_string(), Value::Bool(true));
            obj.insert("default_verbosity".to_string(), Value::String("low".to_string()));
            obj.insert(
                "default_reasoning_summary".to_string(),
                Value::String("none".to_string()),
            );
            obj.insert("supports_parallel_tool_calls".to_string(), Value::Bool(true));
            obj.insert("experimental_supported_tools".to_string(), json!([]));
            if let Some(mm) = valid_mm {
                obj.insert("model_messages".to_string(), mm.clone());
            }
            Value::Object(obj)
        })
        .collect();

    json!({ "models": models })
}

pub fn filter_codex_models_catalog(full_catalog: &Value, filter: &ClientModelFilter) -> Value {
    let raw_models = full_catalog
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut selectable = Vec::new();
    let mut hidden = Vec::new();
    for item in raw_models {
        let vis = item
            .get("visibility")
            .and_then(Value::as_str)
            .unwrap_or("list");
        if vis == "hide" {
            hidden.push(item);
        } else {
            selectable.push(item);
        }
    }

    let allowed = normalized_filter_set(AppId::Codex, filter);
    let mut kept_selectable: Vec<Value> = match allowed {
        None => selectable,
        Some(ref set) => {
            let matched: Vec<Value> = selectable
                .iter()
                .filter(|item| {
                    item.get("slug")
                        .and_then(Value::as_str)
                        .map(|slug| set.contains(&slug.trim().to_ascii_lowercase()))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if matched.is_empty() && !selectable.is_empty() {
                selectable.into_iter().take(3).collect()
            } else {
                matched
            }
        }
    };

    kept_selectable.extend(hidden);
    json!({ "models": kept_selectable })
}

fn parse_claude_tier_and_version(id: &str) -> (Option<&'static str>, u32, u32) {
    let lower = id.to_ascii_lowercase();
    let no_1m = lower.strip_suffix("[1m]").unwrap_or(&lower);
    let trimmed = strip_dated_snapshot_suffix(no_1m).unwrap_or(no_1m);

    let tier = ["opus", "sonnet", "fable", "haiku", "mythos"]
        .into_iter()
        .find(|t| trimmed.split('-').any(|seg| seg == *t) || trimmed.contains(t));

    let nums: Vec<u32> = trimmed
        .split('-')
        .filter_map(|seg| {
            if !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit()) {
                seg.parse::<u32>().ok()
            } else {
                None
            }
        })
        .collect();

    let major = nums.first().copied().unwrap_or(0);
    let minor = nums.get(1).copied().unwrap_or(0);
    (tier, major, minor)
}

fn claude_tier_rank(tier: Option<&str>) -> u8 {
    match tier {
        Some("opus") => 0,
        Some("sonnet") => 1,
        Some("fable") => 2,
        Some("haiku") => 3,
        Some("mythos") => 4,
        _ => 5,
    }
}

fn claude_display_name(id: &str, tier: Option<&str>, major: u32, minor: u32) -> String {
    if let Some(t) = tier {
        let tier_title = title_case_token(t);
        if major > 0 && minor > 0 {
            return format!("Claude {tier_title} {major}.{minor}");
        }
        if major > 0 {
            return format!("Claude {tier_title} {major}");
        }
        return format!("Claude {tier_title}");
    }
    id.split('-')
        .map(format_general_token)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Claude Desktop treats `maxEffort` on `inferenceModels` entries strictly as an upper-bound cap
/// ("Claude Desktop hides higher levels and never requests them"). For Claude models and reasoning
/// models, return `"max"` so Desktop exposes all native effort levels supported by the model
/// (`low / medium / high / xhigh / max`) instead of capping them to `low / medium / high`.
fn claude_max_effort_level(real_id: &str, pricing: &PricingCache) -> &'static str {
    let lower = real_id.to_ascii_lowercase();
    if lower.starts_with("claude-")
        || lower.starts_with("gpt-6")
        || lower.starts_with("gpt-5")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        return "max";
    }
    if let Some(levels) = effective_effort_levels(real_id, pricing) {
        if let Some(level) = ALLOWED_EFFORTS
            .iter()
            .rev()
            .find(|l| levels.iter().any(|x| x == *l))
        {
            return level;
        }
    }
    "high"
}

/// Converts upstream `/v1/models` + `/api/pricing` into Claude Desktop's Anthropic `/v1/models` format.
pub fn build_claude_models_catalog(upstream_models: &Value, pricing: &PricingCache) -> Value {
    let raw_items = upstream_models
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut all_ids = HashSet::new();
    let mut candidate_ids: Vec<String> = Vec::new();
    for item in &raw_items {
        let Some(id) = item
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if id == "codex-auto-review" {
            continue;
        }
        let owned_by = item
            .get("owned_by")
            .and_then(Value::as_str)
            .unwrap_or("");
        if is_client_excluded_model(id, owned_by) {
            continue;
        }
        let eps = extract_endpoint_types(item);
        if is_non_chat_or_noise_model(id, &eps) {
            continue;
        }
        let is_claude =
            id.to_ascii_lowercase().starts_with("claude-") || owned_by.eq_ignore_ascii_case("claude");
        if is_claude && all_ids.insert(id.to_string()) {
            candidate_ids.push(id.to_string());
        }
    }

    struct ClaudeCatalogItem {
        id: String,
        tier: Option<&'static str>,
        major: u32,
        minor: u32,
    }

    let mut parsed: Vec<ClaudeCatalogItem> = candidate_ids
        .into_iter()
        .filter(|id| !is_duplicate_alias(id, &all_ids))
        .map(|id| {
            let (tier, major, minor) = parse_claude_tier_and_version(&id);
            ClaudeCatalogItem {
                id,
                tier,
                major,
                minor,
            }
        })
        .collect();

    parsed.sort_by(|a, b| {
        claude_tier_rank(a.tier)
            .cmp(&claude_tier_rank(b.tier))
            .then_with(|| Reverse(a.major).cmp(&Reverse(b.major)))
            .then_with(|| Reverse(a.minor).cmp(&Reverse(b.minor)))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut seen_tiers: HashSet<&'static str> = HashSet::new();
    let data: Vec<Value> = parsed
        .into_iter()
        .map(|item| {
            let is_family_default = item.tier.map(|t| seen_tiers.insert(t)).unwrap_or(false);
            let tier_for_gateway = item.tier.unwrap_or("sonnet");
            let max_input_tokens = model_context_window(&item.id);
            let display_name = pretty_model_display_name(&item.id);
            let description = concise_model_description(
                &item.id,
                pricing.descriptions.get(&item.id).map(String::as_str),
            );

            let mut obj = Map::new();
            obj.insert("id".to_string(), Value::String(item.id.clone()));
            obj.insert("type".to_string(), Value::String("model".to_string()));
            obj.insert("display_name".to_string(), Value::String(display_name));
            obj.insert(
                "created_at".to_string(),
                Value::String("2026-01-01T00:00:00Z".to_string()),
            );
            obj.insert(
                "anthropic_family_tier".to_string(),
                Value::String(tier_for_gateway.to_string()),
            );
            if is_family_default {
                obj.insert("is_family_default".to_string(), Value::Bool(true));
            }
            // Always set supports_1m = false so Claude Desktop does not duplicate every 1M model row.
            obj.insert("supports_1m".to_string(), Value::Bool(false));
            obj.insert("max_input_tokens".to_string(), json!(max_input_tokens));
            obj.insert("description".to_string(), Value::String(description));
            obj.insert(
                "max_effort_level".to_string(),
                Value::String(claude_max_effort_level(&item.id, pricing).to_string()),
            );
            Value::Object(obj)
        })
        .collect();

    let first_id = data
        .first()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    let last_id = data
        .last()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);

    json!({
        "data": data,
        "has_more": false,
        "first_id": first_id,
        "last_id": last_id,
    })
}

/// Claude Desktop takes a gateway model's effort levels and limits from `inferenceModels`
/// in the 3p profile; the fields it discovers over `/v1/models` are ignored for that.
pub fn build_claude_inference_models(catalog: &Value, filter: &ClientModelFilter) -> Value {
    let filtered = filter_claude_models_catalog(catalog, filter);
    let models: Vec<Value> = filtered
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let name = m.get("id").and_then(Value::as_str)?.to_string();
            let name_lower = name.to_ascii_lowercase();
            let mut entry = Map::new();
            entry.insert("name".to_string(), Value::String(name));
            if let Some(label) = m.get("display_name").and_then(Value::as_str) {
                entry.insert("labelOverride".to_string(), Value::String(label.to_string()));
            }
            if let Some(tier) = m.get("anthropic_family_tier").and_then(Value::as_str) {
                entry.insert(
                    "anthropicFamilyTier".to_string(),
                    Value::String(tier.to_string()),
                );
            }
            if m.get("is_family_default").and_then(Value::as_bool) == Some(true) {
                entry.insert("isFamilyDefault".to_string(), Value::Bool(true));
            }
            let effective_max_effort = if name_lower.starts_with("claude-")
                || name_lower.starts_with("gpt-6")
                || name_lower.starts_with("gpt-5")
                || name_lower.starts_with("o1")
                || name_lower.starts_with("o3")
                || name_lower.starts_with("o4")
            {
                Some("max")
            } else {
                m.get("max_effort_level").and_then(Value::as_str)
            };
            if let Some(effort) = effective_max_effort {
                entry.insert("maxEffort".to_string(), Value::String(effort.to_string()));
            }
            Some(Value::Object(entry))
        })
        .collect();
    Value::Array(models)
}

fn claude_catalog_cache_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.switch_dir().join("claude-catalog.json")
}

/// Refreshes the cached Claude catalog so the adapter can write `inferenceModels`
/// without doing network I/O itself.
pub async fn refresh_claude_catalog_cache(paths: &AppPaths) -> Result<(), String> {
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let api_key = read_main_api_key(paths)
        .ok_or_else(|| "未登录 Tako 账号或缺少 API Key".to_string())?;
    let client = apply_proxy_policy(
        Client::builder().timeout(Duration::from_secs(12)),
        state.bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("创建模型请求客户端失败: {e}"))?;

    let (models_res, pricing_res) = tokio::join!(
        client
            .get(format!("{base_url}/v1/models"))
            .bearer_auth(api_key)
            .send(),
        client
            .get(format!("{base_url}/api/pricing"))
            .timeout(Duration::from_secs(5))
            .send()
    );

    let resp = models_res.map_err(|e| format!("拉取上游模型列表失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("上游模型列表返回 HTTP {}", resp.status().as_u16()));
    }
    let json_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析上游模型列表失败: {e}"))?;
    let pricing = match pricing_res {
        Ok(r) if r.status().is_success() => r
            .json::<Value>()
            .await
            .map(|v| parse_pricing_metadata(&v))
            .unwrap_or_default(),
        _ => PricingCache::default(),
    };

    let full = build_claude_models_catalog(&json_body, &pricing);
    let filtered = filter_claude_models_catalog(&full, &state.model_filters.claude);
    let bytes = serde_json::to_vec(&filtered).map_err(|e| e.to_string())?;
    fs::write(claude_catalog_cache_path(paths), bytes)
        .map_err(|e| format!("写入 Claude 模型缓存失败: {e}"))
}

pub fn load_claude_inference_models(
    paths: &AppPaths,
    filter: &ClientModelFilter,
) -> Option<Value> {
    let bytes = fs::read(claude_catalog_cache_path(paths)).ok()?;
    let catalog: Value = serde_json::from_slice(&bytes).ok()?;
    catalog.get("data").and_then(Value::as_array)?;
    Some(build_claude_inference_models(&catalog, filter))
}

pub fn filter_claude_models_catalog(full_catalog: &Value, filter: &ClientModelFilter) -> Value {
    let raw_data = full_catalog
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let allowed = normalized_filter_set(AppId::Claude, filter);
    let mut kept: Vec<Value> = match allowed {
        None => raw_data.clone(),
        Some(ref set) => raw_data
            .iter()
            .filter(|item| {
                item.get("id")
                    .and_then(Value::as_str)
                    .map(|id| set.contains(&catalog_filter_key(AppId::Claude, id)))
                    .unwrap_or(false)
            })
            .cloned()
            .collect(),
    };

    if kept.is_empty() && !raw_data.is_empty() {
        let mut fallback_tiers: HashSet<&'static str> = HashSet::new();
        for item in &raw_data {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("");
            if let (Some(t), _, _) = parse_claude_tier_and_version(id) {
                if fallback_tiers.insert(t) {
                    kept.push(item.clone());
                }
            }
        }
        if kept.is_empty() {
            kept.push(raw_data[0].clone());
        }
    }

    let mut seen_tiers: HashSet<&'static str> = HashSet::new();
    for item in &mut kept {
        if let Some(obj) = item.as_object_mut() {
            obj.remove("is_family_default");
            let id = obj.get("id").and_then(Value::as_str).unwrap_or("");
            if let (Some(t), _, _) = parse_claude_tier_and_version(id) {
                if seen_tiers.insert(t) {
                    obj.insert("is_family_default".to_string(), Value::Bool(true));
                }
            }
        }
    }

    let first_id = kept
        .first()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);
    let last_id = kept
        .last()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(|s| Value::String(s.to_string()))
        .unwrap_or(Value::Null);

    json!({
        "data": kept,
        "has_more": false,
        "first_id": first_id,
        "last_id": last_id,
    })
}

/// Strips `[1m]` context suffix from `"model"` in Claude Messages/count_tokens requests.
pub fn normalize_claude_request_body(body_bytes: &[u8]) -> Vec<u8> {
    let Ok(mut root) = serde_json::from_slice::<Value>(body_bytes) else {
        return body_bytes.to_vec();
    };
    let Some(obj) = root.as_object_mut() else {
        return body_bytes.to_vec();
    };
    let Some(model) = obj.get("model").and_then(Value::as_str).map(str::to_string) else {
        return body_bytes.to_vec();
    };

    let lower = model.to_ascii_lowercase();
    let base = lower
        .strip_suffix("[1m]")
        .map(|prefix| model[..prefix.len()].to_string())
        .unwrap_or_else(|| model.clone());
    if base == model {
        return body_bytes.to_vec();
    }
    obj.insert("model".to_string(), Value::String(base));
    serde_json::to_vec(&root).unwrap_or_else(|_| body_bytes.to_vec())
}

fn should_strip_req_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "accept-encoding"
            | "keep-alive"
            | "proxy-connection"
            | "te"
            | "trailers"
            | "upgrade"
    )
}

fn should_strip_resp_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-length"
            | "transfer-encoding"
            | "connection"
            | "keep-alive"
            | "proxy-connection"
            | "te"
            | "trailers"
            | "upgrade"
    )
}

fn full_body(bytes: impl Into<Bytes>) -> ProxyBody {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

fn json_error_response(status: StatusCode, message: &str) -> Response<ProxyBody> {
    let payload = json!({
        "error": {
            "code": status.as_u16(),
            "message": message,
            "status": "PROXY_ERROR"
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(full_body(bytes))
        .unwrap_or_else(|_| Response::new(full_body(Bytes::new())))
}

async fn get_pricing_cache(
    state: &ProxyState,
    base_url: &str,
    bypass_system_proxy: bool,
) -> PricingCache {
    {
        let guard = state.pricing_cache.read().await;
        if let Some((ref cached_base, fetched_at, ref cached)) = *guard {
            if cached_base == base_url && fetched_at.elapsed() < PRICING_CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let url = format!("{base_url}/api/pricing");
    let client = state.upstream_client(bypass_system_proxy);
    let fetched = tokio::time::timeout(Duration::from_secs(5), client.get(&url).send()).await;

    if let Ok(Ok(resp)) = fetched {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<Value>().await {
                let parsed = parse_pricing_metadata(&body);
                let mut guard = state.pricing_cache.write().await;
                *guard = Some((base_url.to_string(), Instant::now(), parsed.clone()));
                return parsed;
            }
        }
    }

    let guard = state.pricing_cache.read().await;
    guard
        .as_ref()
        .filter(|(cached_base, _, _)| cached_base == base_url)
        .map(|(_, _, c)| c.clone())
        .unwrap_or_default()
}

async fn get_codex_model_messages(state: &ProxyState) -> Option<Value> {
    {
        let guard = state.codex_model_messages.read().await;
        if let Some(ref cached) = *guard {
            return Some(cached.clone());
        }
    }
    let cache_path = state.paths.codex_dir.join("models_cache.json");
    let extracted = fs::read(&cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|val| {
            val.get("models")
                .and_then(Value::as_array)
                .and_then(|models| {
                    models.iter().find_map(|m| {
                        let mm = m.get("model_messages")?;
                        let obj = mm.as_object()?;
                        if !obj.is_empty() && obj.contains_key("instructions_template") {
                            Some(mm.clone())
                        } else {
                            None
                        }
                    })
                })
        });
    if let Some(ref mm) = extracted {
        let mut guard = state.codex_model_messages.write().await;
        *guard = Some(mm.clone());
    }
    extracted
}

fn build_upstream_headers(
    paths: &AppPaths,
    incoming: &hyper::HeaderMap,
    kind: ClientKind,
) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    let agy_key = if kind == ClientKind::Antigravity {
        effective_agy_api_key(paths)
    } else {
        None
    };
    let main_key = if matches!(kind, ClientKind::Codex | ClientKind::Claude) {
        read_main_api_key(paths)
    } else {
        None
    };

    for (name, val) in incoming {
        let name_str = name.as_str();
        if should_strip_req_header(name_str) {
            continue;
        }
        match kind {
            ClientKind::Antigravity
                if agy_key.is_some()
                    && (name_str.eq_ignore_ascii_case("x-goog-api-key")
                        || name_str.eq_ignore_ascii_case("authorization")) =>
            {
                continue;
            }
            ClientKind::Codex | ClientKind::Claude
                if main_key.is_some()
                    && (name_str.eq_ignore_ascii_case("authorization")
                        || name_str.eq_ignore_ascii_case("x-api-key")) =>
            {
                continue;
            }
            _ => {}
        }
        if let (Ok(r_name), Ok(r_val)) = (
            reqwest::header::HeaderName::from_bytes(name_str.as_bytes()),
            reqwest::header::HeaderValue::from_bytes(val.as_bytes()),
        ) {
            out.append(r_name, r_val);
        }
    }

    match kind {
        ClientKind::Antigravity => {
            if let Some(key) = agy_key {
                if let Ok(r_val) = reqwest::header::HeaderValue::from_str(&key) {
                    out.insert(
                        reqwest::header::HeaderName::from_static("x-goog-api-key"),
                        r_val,
                    );
                }
            }
        }
        ClientKind::Codex | ClientKind::Claude => {
            if let Some(key) = main_key {
                if let Ok(r_val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                    out.insert(
                        reqwest::header::HeaderName::from_static("authorization"),
                        r_val,
                    );
                }
            }
        }
    }
    out
}

fn copy_response_headers(
    upstream: &reqwest::header::HeaderMap,
    builder: hyper::http::response::Builder,
) -> hyper::http::response::Builder {
    let mut b = builder;
    for (name, val) in upstream {
        let name_str = name.as_str();
        if should_strip_resp_header(name_str) {
            continue;
        }
        if let (Ok(h_name), Ok(h_val)) = (
            HeaderName::from_bytes(name_str.as_bytes()),
            HeaderValue::from_bytes(val.as_bytes()),
        ) {
            b = b.header(h_name, h_val);
        }
    }
    b
}

fn resolve_model_metrics(
    metric_id: &str,
    pricing: &PricingCache,
    perf: &HashMap<String, ModelPerfInfo>,
) -> (ModelPricingInfo, ModelPerfInfo) {
    let key = metric_id.trim().to_ascii_lowercase();
    let price_info = pricing.prices.get(&key).cloned().unwrap_or_default();
    let perf_info = perf.get(&key).cloned().unwrap_or_default();
    (price_info, perf_info)
}

pub fn build_claude_model_options(
    upstream_models: &Value,
    pricing: &PricingCache,
    perf: &HashMap<String, ModelPerfInfo>,
    filter: &ClientModelFilter,
) -> ClientModelsResponse {
    let full = build_claude_models_catalog(upstream_models, pricing);
    let filtered = filter_claude_models_catalog(&full, filter);
    let visible_ids: HashSet<String> = filtered
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(Value::as_str).map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let models: Vec<ClientModelOption> = full
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(Value::as_str)?.to_string();
                    let display_name = m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let description = m
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("Tako 托管模型")
                        .to_string();
                    let context_window = m.get("max_input_tokens").and_then(Value::as_u64);
                    let visible = visible_ids.contains(&id.to_ascii_lowercase());
                    let family = model_family_label(AppId::Claude, &id).to_string();
                    let (price_info, perf_info) =
                        resolve_model_metrics(&id, pricing, perf);
                    let capabilities = if price_info.capabilities.is_empty() {
                        derive_model_capabilities(&id, None, &price_info.tags, &description, false)
                    } else {
                        price_info.capabilities
                    };
                    let promo_ends_on = price_info
                        .promo_ends_on
                        .or_else(|| resolve_promo_ends_on(&id, None));
                    let vendor_name = price_info
                        .vendor_name
                        .or_else(|| infer_vendor_name(&id, None));
                    let resolved_ctx = price_info.context_window.or(context_window);
                    Some(ClientModelOption {
                        family,
                        recommended: is_recommended_model(AppId::Claude, &id),
                        id,
                        display_name,
                        description,
                        context_window: resolved_ctx,
                        visible,
                        effort_levels: Vec::new(),
                        is_free: price_info.is_free,
                        is_dynamic_pricing: price_info.is_dynamic_pricing,
                        standard_load_usd_per_m: price_info.standard_load_usd_per_m,
                        standard_load_max_usd_per_m: price_info.standard_load_max_usd_per_m,
                        realized_usd_per_m: price_info.realized_usd_per_m,
                        cache_hit_rate: price_info.cache_hit_rate,
                        input_usd_per_m: price_info.input_usd_per_m,
                        cache_usd_per_m: price_info.cache_usd_per_m,
                        output_usd_per_m: price_info.output_usd_per_m,
                        request_usd: price_info.request_usd,
                        quota_type: price_info.quota_type,
                        model_ratio: price_info.model_ratio,
                        official_multiplier: price_info.official_multiplier,
                        promo_ends_on,
                        capabilities,
                        tags: price_info.tags,
                        vendor_name,
                        channels: price_info.channels,
                        avg_tps: perf_info.avg_tps,
                        avg_latency_ms: perf_info.avg_latency_ms,
                        success_rate: perf_info.success_rate,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let visible_count = models.iter().filter(|m| m.visible).count();
    let total_count = models.len();
    ClientModelsResponse {
        app_id: AppId::Claude,
        filter: filter.clone(),
        recommended_ids: ClientModelFilter::recommended_ids_for(AppId::Claude)
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        visible_count,
        total_count,
        models,
    }
}

pub fn build_codex_model_options(
    upstream_models: &Value,
    pricing: &PricingCache,
    perf: &HashMap<String, ModelPerfInfo>,
    filter: &ClientModelFilter,
) -> ClientModelsResponse {
    let full = build_codex_models_catalog(upstream_models, pricing, None);
    let filtered = filter_codex_models_catalog(&full, filter);
    let visible_ids: HashSet<String> = filtered
        .get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|m| m.get("visibility").and_then(Value::as_str) != Some("hide"))
                .filter_map(|m| {
                    m.get("slug")
                        .and_then(Value::as_str)
                        .map(|s| s.to_ascii_lowercase())
                })
                .collect()
        })
        .unwrap_or_default();

    let models: Vec<ClientModelOption> = full
        .get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter(|m| m.get("visibility").and_then(Value::as_str) != Some("hide"))
                .filter_map(|m| {
                    let id = m.get("slug").and_then(Value::as_str)?.to_string();
                    let display_name = m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let description = m
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("Tako 托管模型")
                        .to_string();
                    let context_window = m.get("context_window").and_then(Value::as_u64);
                    let effort_levels: Vec<String> = m
                        .get("supported_reasoning_levels")
                        .and_then(Value::as_array)
                        .map(|lvls| {
                            lvls.iter()
                                .filter_map(|lvl| {
                                    lvl.get("effort").and_then(Value::as_str).map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let visible = visible_ids.contains(&id.to_ascii_lowercase());
                    let (price_info, perf_info) = resolve_model_metrics(&id, pricing, perf);
                    let capabilities = if price_info.capabilities.is_empty() {
                        derive_model_capabilities(
                            &id,
                            None,
                            &price_info.tags,
                            &description,
                            !effort_levels.is_empty(),
                        )
                    } else {
                        price_info.capabilities
                    };
                    let promo_ends_on = price_info
                        .promo_ends_on
                        .or_else(|| resolve_promo_ends_on(&id, None));
                    let vendor_name = price_info
                        .vendor_name
                        .or_else(|| infer_vendor_name(&id, None));
                    let resolved_ctx = price_info.context_window.or(context_window);
                    Some(ClientModelOption {
                        family: model_family_label(AppId::Codex, &id).to_string(),
                        recommended: is_recommended_model(AppId::Codex, &id),
                        id,
                        display_name,
                        description,
                        context_window: resolved_ctx,
                        visible,
                        effort_levels,
                        is_free: price_info.is_free,
                        is_dynamic_pricing: price_info.is_dynamic_pricing,
                        standard_load_usd_per_m: price_info.standard_load_usd_per_m,
                        standard_load_max_usd_per_m: price_info.standard_load_max_usd_per_m,
                        realized_usd_per_m: price_info.realized_usd_per_m,
                        cache_hit_rate: price_info.cache_hit_rate,
                        input_usd_per_m: price_info.input_usd_per_m,
                        cache_usd_per_m: price_info.cache_usd_per_m,
                        output_usd_per_m: price_info.output_usd_per_m,
                        request_usd: price_info.request_usd,
                        quota_type: price_info.quota_type,
                        model_ratio: price_info.model_ratio,
                        official_multiplier: price_info.official_multiplier,
                        promo_ends_on,
                        capabilities,
                        tags: price_info.tags,
                        vendor_name,
                        channels: price_info.channels,
                        avg_tps: perf_info.avg_tps,
                        avg_latency_ms: perf_info.avg_latency_ms,
                        success_rate: perf_info.success_rate,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let visible_count = models.iter().filter(|m| m.visible).count();
    let total_count = models.len();
    ClientModelsResponse {
        app_id: AppId::Codex,
        filter: filter.clone(),
        recommended_ids: ClientModelFilter::recommended_ids_for(AppId::Codex)
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        visible_count,
        total_count,
        models,
    }
}

pub fn build_antigravity_model_options(
    upstream_models: &Value,
    pricing: &PricingCache,
    perf: &HashMap<String, ModelPerfInfo>,
    filter: &ClientModelFilter,
) -> ClientModelsResponse {
    let candidates = collect_antigravity_base_candidates(upstream_models, pricing);
    let selected = resolve_antigravity_selected_base_ids(&candidates, filter);

    let models: Vec<ClientModelOption> = candidates
        .into_iter()
        .map(|cand| {
            let visible = selected.contains(&cand.base_id.to_ascii_lowercase());
            let description = concise_model_description(
                &cand.base_id,
                pricing.descriptions.get(&cand.base_id).map(String::as_str),
            );
            let context_window = Some(model_context_window(&cand.base_id));
            let recommended = is_recommended_model(AppId::Antigravity, &cand.base_id)
                || is_recommended_model(AppId::Antigravity, &cand.variant_prefix);
            let (price_info, perf_info) = resolve_model_metrics(&cand.base_id, pricing, perf);
            let capabilities = if price_info.capabilities.is_empty() {
                derive_model_capabilities(
                    &cand.base_id,
                    None,
                    &price_info.tags,
                    &description,
                    !cand.effort_levels.is_empty(),
                )
            } else {
                price_info.capabilities
            };
            let promo_ends_on = price_info
                .promo_ends_on
                .or_else(|| resolve_promo_ends_on(&cand.base_id, None));
            let vendor_name = price_info
                .vendor_name
                .or_else(|| infer_vendor_name(&cand.base_id, None));
            let resolved_ctx = price_info.context_window.or(context_window);
            ClientModelOption {
                family: model_family_label(AppId::Antigravity, &cand.base_id).to_string(),
                id: cand.base_id,
                display_name: cand.display_name,
                description,
                context_window: resolved_ctx,
                recommended,
                visible,
                effort_levels: cand.effort_levels,
                is_free: price_info.is_free,
                is_dynamic_pricing: price_info.is_dynamic_pricing,
                standard_load_usd_per_m: price_info.standard_load_usd_per_m,
                standard_load_max_usd_per_m: price_info.standard_load_max_usd_per_m,
                realized_usd_per_m: price_info.realized_usd_per_m,
                cache_hit_rate: price_info.cache_hit_rate,
                input_usd_per_m: price_info.input_usd_per_m,
                cache_usd_per_m: price_info.cache_usd_per_m,
                output_usd_per_m: price_info.output_usd_per_m,
                request_usd: price_info.request_usd,
                quota_type: price_info.quota_type,
                model_ratio: price_info.model_ratio,
                official_multiplier: price_info.official_multiplier,
                promo_ends_on,
                capabilities,
                tags: price_info.tags,
                vendor_name,
                channels: price_info.channels,
                avg_tps: perf_info.avg_tps,
                avg_latency_ms: perf_info.avg_latency_ms,
                success_rate: perf_info.success_rate,
            }
        })
        .collect();

    let visible_count = models.iter().filter(|m| m.visible).count();
    let total_count = models.len();
    ClientModelsResponse {
        app_id: AppId::Antigravity,
        filter: filter.clone(),
        recommended_ids: ClientModelFilter::recommended_ids_for(AppId::Antigravity)
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        visible_count,
        total_count,
        models,
    }
}

fn plaza_family_label(id: &str, capabilities: &[String], vendor: Option<&str>) -> String {
    let base_fam = model_family_label(AppId::Codex, id);
    if base_fam != "其他" {
        return base_fam.to_string();
    }
    if capabilities.iter().any(|c| c == "image") && !capabilities.iter().any(|c| c == "chat") {
        return "生图".to_string();
    }
    if capabilities.iter().any(|c| c == "video") {
        return "视频".to_string();
    }
    if capabilities.iter().any(|c| c == "audio") && !capabilities.iter().any(|c| c == "chat") {
        return "语音".to_string();
    }
    if capabilities.iter().any(|c| c == "embed") {
        return "向量".to_string();
    }
    if let Some(v) = vendor.filter(|s| !s.is_empty()) {
        return v.to_string();
    }
    "其他".to_string()
}

fn is_plaza_recommended(id: &str) -> bool {
    is_recommended_model(AppId::Codex, id)
        || is_recommended_model(AppId::Claude, id)
        || is_recommended_model(AppId::Antigravity, id)
        || matches!(
            id.trim().to_ascii_lowercase().as_str(),
            "gemini-4-carbon" | "gemini-4-argon"
        )
}

fn plaza_sort_key(m: &ClientModelOption) -> (u8, u16, String) {
    let promo_rank = if m.promo_ends_on.is_some() {
        0
    } else if m.recommended {
        1
    } else if m.capabilities.iter().any(|c| c == "chat") {
        2
    } else {
        3
    };
    let fam_prio = antigravity_model_sort_key(&m.id).0;
    (promo_rank, fam_prio, m.id.to_ascii_lowercase())
}

pub fn build_plaza_model_options(
    pricing_body: &Value,
    perf: &HashMap<String, ModelPerfInfo>,
) -> ClientModelsResponse {
    let pricing = parse_pricing_metadata(pricing_body);
    let raw_items = pricing_body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut seen = HashSet::new();
    let mut models = Vec::with_capacity(raw_items.len());
    let mut recommended_ids = Vec::new();

    for item in &raw_items {
        let Some(id_raw) = item
            .get("model_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let lower = id_raw.to_ascii_lowercase();
        if lower == "codex-auto-review" || lower.starts_with("jev-") {
            continue;
        }
        if !seen.insert(lower.clone()) {
            continue;
        }

        let (price_info, perf_info) = resolve_model_metrics(id_raw, &pricing, perf);
        let raw_desc = item.get("description").and_then(Value::as_str);
        let description = if let Some(d) = raw_desc.map(str::trim).filter(|s| !s.is_empty()) {
            let concise = concise_model_description(id_raw, Some(d));
            if concise == "Tako 托管模型" {
                d.chars().take(48).collect()
            } else {
                concise
            }
        } else {
            concise_model_description(id_raw, None)
        };
        let display_name = display_base_name(
            id_raw,
            item.get("display_name")
                .or_else(|| item.get("displayName"))
                .and_then(Value::as_str),
        );
        let effort_levels = pricing
            .reasoning_levels
            .get(id_raw)
            .cloned()
            .unwrap_or_default();
        let recommended = is_plaza_recommended(id_raw);
        if recommended {
            recommended_ids.push(id_raw.to_string());
        }
        let family = plaza_family_label(
            id_raw,
            &price_info.capabilities,
            price_info.vendor_name.as_deref(),
        );

        models.push(ClientModelOption {
            id: id_raw.to_string(),
            display_name,
            family,
            description,
            context_window: price_info.context_window,
            recommended,
            visible: true,
            effort_levels,
            is_free: price_info.is_free,
            is_dynamic_pricing: price_info.is_dynamic_pricing,
            standard_load_usd_per_m: price_info.standard_load_usd_per_m,
            standard_load_max_usd_per_m: price_info.standard_load_max_usd_per_m,
            realized_usd_per_m: price_info.realized_usd_per_m,
            cache_hit_rate: price_info.cache_hit_rate,
            input_usd_per_m: price_info.input_usd_per_m,
            cache_usd_per_m: price_info.cache_usd_per_m,
            output_usd_per_m: price_info.output_usd_per_m,
            request_usd: price_info.request_usd,
            quota_type: price_info.quota_type,
            model_ratio: price_info.model_ratio,
            official_multiplier: price_info.official_multiplier,
            promo_ends_on: price_info.promo_ends_on,
            capabilities: price_info.capabilities,
            tags: price_info.tags,
            vendor_name: price_info.vendor_name,
            channels: price_info.channels,
            avg_tps: perf_info.avg_tps,
            avg_latency_ms: perf_info.avg_latency_ms,
            success_rate: perf_info.success_rate,
        });
    }

    models.sort_by_key(plaza_sort_key);
    let total_count = models.len();
    ClientModelsResponse {
        app_id: AppId::DshTako,
        filter: ClientModelFilter {
            mode: ModelFilterMode::All,
            enabled_ids: Vec::new(),
        },
        recommended_ids,
        visible_count: total_count,
        total_count,
        models,
    }
}

pub async fn fetch_plaza_models_for_ui(paths: &AppPaths) -> Result<ClientModelsResponse, String> {
    let switch_state = read_switch_state(paths);
    let base_url = switch_state.normalized_base_url();
    let client = apply_proxy_policy(
        Client::builder().timeout(Duration::from_secs(12)),
        switch_state.bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("创建模型广场客户端失败: {e}"))?;

    let pricing_url = format!("{base_url}/api/pricing");
    let perf_url = format!("{base_url}/api/perf-metrics/summary");
    let (pricing_res, perf_res) = tokio::join!(
        client.get(&pricing_url).send(),
        client
            .get(&perf_url)
            .timeout(Duration::from_secs(6))
            .send()
    );

    let resp = pricing_res.map_err(|e| format!("拉取模型广场定价失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        let snippet: String = body_text.chars().take(160).collect();
        return Err(format!(
            "模型广场接口返回 HTTP {}: {snippet}",
            status.as_u16()
        ));
    }
    let pricing_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析模型广场定价数据失败: {e}"))?;

    let perf = match perf_res {
        Ok(r) if r.status().is_success() => r
            .json::<Value>()
            .await
            .map(|v| parse_perf_metrics_summary(&v))
            .unwrap_or_default(),
        _ => HashMap::new(),
    };

    Ok(build_plaza_model_options(&pricing_body, &perf))
}

pub async fn fetch_client_models_for_ui(
    paths: &AppPaths,
    app_id: AppId,
) -> Result<ClientModelsResponse, String> {
    if app_id == AppId::DshTako {
        return fetch_plaza_models_for_ui(paths).await;
    }

    let switch_state = read_switch_state(paths);
    let base_url = switch_state.normalized_base_url();
    let client = apply_proxy_policy(
        Client::builder().timeout(Duration::from_secs(12)),
        switch_state.bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("创建模型请求客户端失败: {e}"))?;

    let models_req = match app_id {
        AppId::DshTako => unreachable!(),
        AppId::Claude | AppId::Codex => {
            let api_key = read_main_api_key(paths).ok_or_else(|| {
                "未登录 Tako 账号或缺少 API Key，无法拉取模型列表".to_string()
            })?;
            client
                .get(format!("{base_url}/v1/models"))
                .bearer_auth(api_key)
        }
        AppId::Antigravity => {
            let agy_key = effective_agy_api_key(paths).ok_or_else(|| {
                "未登录 Tako 账号或缺少 Antigravity 专用 Key，无法拉取模型列表".to_string()
            })?;
            client
                .get(format!("{base_url}/v1beta/models"))
                .header("x-goog-api-key", agy_key)
        }
    };

    let pricing_url = format!("{base_url}/api/pricing");
    let perf_url = format!("{base_url}/api/perf-metrics/summary");
    let (models_res, pricing_res, perf_res) = tokio::join!(
        models_req.send(),
        client
            .get(&pricing_url)
            .timeout(Duration::from_secs(5))
            .send(),
        client
            .get(&perf_url)
            .timeout(Duration::from_secs(5))
            .send()
    );

    let pricing = match pricing_res {
        Ok(resp) if resp.status().is_success() => resp
            .json::<Value>()
            .await
            .map(|v| parse_pricing_metadata(&v))
            .unwrap_or_default(),
        _ => PricingCache::default(),
    };

    let perf = match perf_res {
        Ok(resp) if resp.status().is_success() => resp
            .json::<Value>()
            .await
            .map(|v| parse_perf_metrics_summary(&v))
            .unwrap_or_default(),
        _ => HashMap::new(),
    };

    let resp = models_res.map_err(|e| format!("拉取上游模型列表失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        let snippet: String = body_text.chars().take(160).collect();
        return Err(format!("上游返回 HTTP {}: {snippet}", status.as_u16()));
    }

    let json_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析上游模型列表失败: {e}"))?;

    if app_id == AppId::Claude {
        let full = build_claude_models_catalog(&json_body, &pricing);
        let filtered = filter_claude_models_catalog(&full, &switch_state.model_filters.claude);
        if let Ok(bytes) = serde_json::to_vec(&filtered) {
            let _ = fs::write(claude_catalog_cache_path(paths), bytes);
        }
    }

    Ok(match app_id {
        AppId::DshTako => unreachable!(),
        AppId::Claude => build_claude_model_options(
            &json_body,
            &pricing,
            &perf,
            &switch_state.model_filters.claude,
        ),
        AppId::Codex => build_codex_model_options(
            &json_body,
            &pricing,
            &perf,
            &switch_state.model_filters.codex,
        ),
        AppId::Antigravity => build_antigravity_model_options(
            &json_body,
            &pricing,
            &perf,
            &switch_state.model_filters.antigravity,
        ),
    })
}

async fn handle_models_list(
    state: Arc<ProxyState>,
    path_and_query: &str,
    incoming_headers: &hyper::HeaderMap,
) -> Response<ProxyBody> {
    let switch_state = read_switch_state(&state.paths);
    let base_url = switch_state.normalized_base_url();
    let bypass_proxy = switch_state.bypass_system_proxy;
    let target_url = format!("{base_url}{path_and_query}");
    let headers = build_upstream_headers(&state.paths, incoming_headers, ClientKind::Antigravity);
    let client = state.upstream_client(bypass_proxy);

    let (upstream_res, pricing) = tokio::join!(
        client.get(&target_url).headers(headers).send(),
        get_pricing_cache(&state, &base_url, bypass_proxy)
    );

    let resp = match upstream_res {
        Ok(r) => r,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("上游模型列表请求失败: {err}"),
            );
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_headers = resp.headers().clone();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("读取上游模型列表失败: {err}"),
            );
        }
    };

    if status == StatusCode::OK {
        if let Ok(json_body) = serde_json::from_slice::<Value>(&bytes) {
            if json_body.get("models").and_then(Value::as_array).is_some() {
                let (expanded_body, routes) = expand_models_catalog_with_filter(
                    json_body,
                    &pricing,
                    Some(&switch_state.model_filters.antigravity),
                );
                {
                    let mut guard = state.synthetic_routes.write().await;
                    *guard = routes;
                }
                if let Ok(out_bytes) = serde_json::to_vec(&expanded_body) {
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json; charset=utf-8")
                        .body(full_body(out_bytes))
                        .unwrap_or_else(|_| {
                            json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed")
                        });
                }
            }
        }
    }

    let builder = copy_response_headers(&resp_headers, Response::builder().status(status));
    builder
        .body(full_body(bytes))
        .unwrap_or_else(|_| json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed"))
}

async fn handle_codex_models(
    state: Arc<ProxyState>,
    incoming_headers: &hyper::HeaderMap,
) -> Response<ProxyBody> {
    let switch_state = read_switch_state(&state.paths);
    let base_url = switch_state.normalized_base_url();
    let bypass_proxy = switch_state.bypass_system_proxy;
    let target_url = format!("{base_url}/v1/models");
    let headers = build_upstream_headers(&state.paths, incoming_headers, ClientKind::Codex);
    let client = state.upstream_client(bypass_proxy);

    let (upstream_res, pricing, cached_mm) = tokio::join!(
        client.get(&target_url).headers(headers).send(),
        get_pricing_cache(&state, &base_url, bypass_proxy),
        get_codex_model_messages(&state)
    );

    let resp = match upstream_res {
        Ok(r) => r,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("获取 Codex 上游模型列表失败: {err}"),
            );
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_headers = resp.headers().clone();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("读取 Codex 上游模型列表失败: {err}"),
            );
        }
    };

    if status == StatusCode::OK {
        if let Ok(json_body) = serde_json::from_slice::<Value>(&bytes) {
            if json_body.get("data").and_then(Value::as_array).is_some() {
                let full_catalog =
                    build_codex_models_catalog(&json_body, &pricing, cached_mm.as_ref());
                let catalog =
                    filter_codex_models_catalog(&full_catalog, &switch_state.model_filters.codex);
                if let Ok(out_bytes) = serde_json::to_vec(&catalog) {
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json; charset=utf-8")
                        .body(full_body(out_bytes))
                        .unwrap_or_else(|_| {
                            json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed")
                        });
                }
            }
        }
    }

    let builder = copy_response_headers(&resp_headers, Response::builder().status(status));
    builder
        .body(full_body(bytes))
        .unwrap_or_else(|_| json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed"))
}

async fn handle_claude_models(
    state: Arc<ProxyState>,
    incoming_headers: &hyper::HeaderMap,
) -> Response<ProxyBody> {
    let switch_state = read_switch_state(&state.paths);
    let base_url = switch_state.normalized_base_url();
    let bypass_proxy = switch_state.bypass_system_proxy;
    let target_url = format!("{base_url}/v1/models");
    let headers = build_upstream_headers(&state.paths, incoming_headers, ClientKind::Claude);
    let client = state.upstream_client(bypass_proxy);

    let (upstream_res, pricing) = tokio::join!(
        client.get(&target_url).headers(headers).send(),
        get_pricing_cache(&state, &base_url, bypass_proxy)
    );

    let resp = match upstream_res {
        Ok(r) => r,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("获取 Claude 上游模型列表失败: {err}"),
            );
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let resp_headers = resp.headers().clone();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(err) => {
            return json_error_response(
                StatusCode::BAD_GATEWAY,
                &format!("读取 Claude 上游模型列表失败: {err}"),
            );
        }
    };

    if status == StatusCode::OK {
        if let Ok(json_body) = serde_json::from_slice::<Value>(&bytes) {
            if json_body.get("data").and_then(Value::as_array).is_some() {
                let full_catalog = build_claude_models_catalog(&json_body, &pricing);
                let catalog = filter_claude_models_catalog(
                    &full_catalog,
                    &switch_state.model_filters.claude,
                );
                if let Ok(out_bytes) = serde_json::to_vec(&catalog) {
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json; charset=utf-8")
                        .body(full_body(out_bytes))
                        .unwrap_or_else(|_| {
                            json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed")
                        });
                }
            }
        }
    }

    let builder = copy_response_headers(&resp_headers, Response::builder().status(status));
    builder
        .body(full_body(bytes))
        .unwrap_or_else(|_| json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response failed"))
}

fn strip_route_prefix(path_and_query: &str, prefix: &str) -> String {
    let rest = path_and_query.strip_prefix(prefix).unwrap_or(path_and_query);
    if rest.is_empty() {
        "/".to_string()
    } else if rest.starts_with('?') {
        format!("/{rest}")
    } else {
        rest.to_string()
    }
}

async fn handle_request(
    state: Arc<ProxyState>,
    downstream_tcp: Option<Arc<std::sync::Mutex<TokioTcpStream>>>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    let path = req.uri().path().to_string();
    let query = req.uri().query().map(str::to_string);
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| path.clone());

    if path == AGY_PROXY_HEALTH_PATH {
        let payload = json!({
            "ok": true,
            "service": "tako-switch-gateway",
            "version": env!("CARGO_PKG_VERSION"),
            "routes": ["antigravity", "codex", "claude"]
        });
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(full_body(bytes))
            .unwrap_or_else(|_| {
                json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "health error")
            }));
    }

    if (*req.method() == hyper::Method::GET || *req.method() == hyper::Method::HEAD)
        && (path == "/" || path.is_empty())
    {
        let payload = json!({
            "ok": true,
            "service": "tako-switch-gateway"
        });
        let body = if *req.method() == hyper::Method::HEAD {
            full_body(Bytes::new())
        } else {
            full_body(serde_json::to_vec(&payload).unwrap_or_default())
        };
        return Ok(Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(body)
            .unwrap_or_else(|_| {
                json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "probe error")
            }));
    }

    if *req.method() == hyper::Method::GET && (path == "/v1beta/models" || path == "/v1beta/models/")
    {
        return Ok(handle_models_list(state, &path_and_query, req.headers()).await);
    }

    if *req.method() == hyper::Method::GET && (path == "/codex/models" || path == "/codex/models/")
    {
        return Ok(handle_codex_models(state, req.headers()).await);
    }

    let is_claude_models = *req.method() == hyper::Method::GET
        && ((path == "/claude/v1/models" || path == "/claude/v1/models/")
            || ((path == "/v1/models" || path == "/v1/models/")
                && req.headers().contains_key("anthropic-version")));
    if is_claude_models {
        return Ok(handle_claude_models(state, req.headers()).await);
    }

    let method = match Method::from_bytes(req.method().as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return Ok(json_error_response(
                StatusCode::BAD_REQUEST,
                "不支持的 HTTP Method",
            ));
        }
    };
    let incoming_headers = req.headers().clone();
    let stream_untouched = req
        .body()
        .size_hint()
        .upper()
        .map(|u| u > PASSTHROUGH_STREAM_THRESHOLD_BYTES)
        .unwrap_or(true);
    let incoming_body = req.into_body();

    let switch_state = read_switch_state(&state.paths);
    let base_url = switch_state.normalized_base_url();
    let bypass_proxy = switch_state.bypass_system_proxy;

    let collect_body = |body: Incoming| async move {
        body.collect()
            .await
            .map(|c| c.to_bytes())
            .map_err(|err| json_error_response(StatusCode::BAD_REQUEST, &format!("读取请求体失败: {err}")))
    };

    let passthrough_body = |body: Incoming| async move {
        if stream_untouched {
            let signal = BodyDoneSignal::new(false);
            Ok((wrap_incoming_body_stream(body, signal.clone()), signal))
        } else {
            let bytes = collect_body(body).await?;
            Ok((reqwest::Body::from(bytes), BodyDoneSignal::new(true)))
        }
    };

    let (client_kind, target_path_and_query, outbound_body, body_done) =
        if path == "/codex" || path.starts_with("/codex/") {
            let stripped = strip_route_prefix(&path_and_query, "/codex");
            let (body, sig) = match passthrough_body(incoming_body).await {
                Ok(pair) => pair,
                Err(resp) => return Ok(resp),
            };
            (ClientKind::Codex, stripped, body, sig)
        } else if path == "/claude" || path.starts_with("/claude/") {
            let stripped_path = path.strip_prefix("/claude").unwrap_or(&path);
            let target_pq = strip_route_prefix(&path_and_query, "/claude");
            if method == Method::POST
                && (stripped_path == "/v1/messages"
                    || stripped_path == "/v1/messages/count_tokens")
            {
                let bytes = match collect_body(incoming_body).await {
                    Ok(b) => b,
                    Err(resp) => return Ok(resp),
                };
                let normalized = Bytes::from(normalize_claude_request_body(&bytes));
                (
                    ClientKind::Claude,
                    target_pq,
                    reqwest::Body::from(normalized),
                    BodyDoneSignal::new(true),
                )
            } else {
                let (body, sig) = match passthrough_body(incoming_body).await {
                    Ok(pair) => pair,
                    Err(resp) => return Ok(resp),
                };
                (ClientKind::Claude, target_pq, body, sig)
            }
        } else if path.starts_with("/v1/messages")
            || incoming_headers.contains_key("anthropic-version")
        {
            if method == Method::POST
                && (path == "/v1/messages" || path == "/v1/messages/count_tokens")
            {
                let bytes = match collect_body(incoming_body).await {
                    Ok(b) => b,
                    Err(resp) => return Ok(resp),
                };
                let normalized = Bytes::from(normalize_claude_request_body(&bytes));
                (
                    ClientKind::Claude,
                    path_and_query,
                    reqwest::Body::from(normalized),
                    BodyDoneSignal::new(true),
                )
            } else {
                let (body, sig) = match passthrough_body(incoming_body).await {
                    Ok(pair) => pair,
                    Err(resp) => return Ok(resp),
                };
                (ClientKind::Claude, path_and_query, body, sig)
            }
        } else {
            let mut next_path = path_and_query;
            let mut rewrite_effort: Option<String> = None;
            if let Some((model, action)) = parse_gemini_action_path(&path) {
                let cached_route = {
                    let routes = state.synthetic_routes.read().await;
                    routes.get(&model).cloned()
                };
                let route = if let Some(hit) = cached_route {
                    Some(hit)
                } else {
                    let pricing = get_pricing_cache(&state, &base_url, bypass_proxy).await;
                    let routes = state.synthetic_routes.read().await;
                    resolve_synthetic_route(&model, &routes, &pricing)
                };

                if let Some(route) = route {
                    next_path = match query.as_deref() {
                        Some(q) if !q.is_empty() => {
                            format!("/v1beta/models/{}:{action}?{q}", route.base_model)
                        }
                        _ => format!("/v1beta/models/{}:{action}", route.base_model),
                    };
                    if action == "generateContent" || action == "streamGenerateContent" {
                        rewrite_effort = Some(route.effort);
                    }
                }
            }

            if let Some(effort) = rewrite_effort {
                let bytes = match collect_body(incoming_body).await {
                    Ok(b) => b,
                    Err(resp) => return Ok(resp),
                };
                let rewritten = Bytes::from(inject_thinking_effort(&bytes, &effort));
                (
                    ClientKind::Antigravity,
                    next_path,
                    reqwest::Body::from(rewritten),
                    BodyDoneSignal::new(true),
                )
            } else {
                let (body, sig) = match passthrough_body(incoming_body).await {
                    Ok(pair) => pair,
                    Err(resp) => return Ok(resp),
                };
                (ClientKind::Antigravity, next_path, body, sig)
            }
        };

    let target_url = format!("{base_url}{target_path_and_query}");
    let upstream_headers = build_upstream_headers(&state.paths, &incoming_headers, client_kind);
    let client = state.upstream_client(bypass_proxy);

    let send_fut = client
        .request(method, &target_url)
        .headers(upstream_headers)
        .body(outbound_body)
        .send();

    let resp = tokio::select! {
        res = send_fut => match res {
            Ok(r) => r,
            Err(err) => {
                return Ok(json_error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("转发上游请求失败: {err}"),
                ));
            }
        },
        () = wait_for_client_disconnect(downstream_tcp, body_done) => {
            return Ok(json_error_response(
                StatusCode::REQUEST_TIMEOUT,
                "下游客户端连接已关闭，已取消上游转发",
            ));
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let builder = copy_response_headers(resp.headers(), Response::builder().status(status));
    let stream = resp
        .bytes_stream()
        .map(|chunk| chunk.map(Frame::data).map_err(std::io::Error::other));
    let body: ProxyBody = http_body_util::BodyExt::boxed(StreamBody::new(stream));
    Ok(builder.body(body).unwrap_or_else(|_| {
        json_error_response(StatusCode::INTERNAL_SERVER_ERROR, "构建响应失败")
    }))
}

/// Serves the local unified Tako HTTP/SSE gateway on an already-bound `TcpListener`.
pub async fn serve_proxy_on_listener(listener: TcpListener, paths: AppPaths) -> Result<(), String> {
    let direct_client = Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建直连 HTTP 客户端失败: {e}"))?;
    let system_proxy_client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| direct_client.clone());

    let state = Arc::new(ProxyState {
        paths,
        direct_client,
        system_proxy_client,
        pricing_cache: RwLock::new(None),
        synthetic_routes: RwLock::new(HashMap::new()),
        codex_model_messages: RwLock::new(None),
    });

    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| format!("接受代理连接失败: {e}"))?;
        let state_clone = state.clone();
        tokio::spawn(async move {
            let shared_tcp = Arc::new(std::sync::Mutex::new(stream));
            let io = TokioIo::new(SharedClientTcp {
                inner: shared_tcp.clone(),
            });
            let svc = service_fn(move |req| {
                handle_request(state_clone.clone(), Some(shared_tcp.clone()), req)
            });
            let _ = http1::Builder::new().serve_connection(io, svc).await;
        });
    }
}

/// Runs the local unified Tako HTTP/SSE gateway on `127.0.0.1:<port>`.
pub async fn run_proxy_server(port: u16) -> Result<(), String> {
    let paths = AppPaths::detect()?;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("绑定 {addr} 失败: {e}"))?;
    serve_proxy_on_listener(listener, paths).await
}

fn probe_health_text(port: u16) -> Option<String> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(350)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(350)));
    let req = format!(
        "GET {AGY_PROXY_HEALTH_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = [0u8; 512];
    let n = stream.read(&mut buf).ok()?;
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Probes `127.0.0.1:<port>/__tako_proxy_health` to verify our local gateway daemon is responsive.
pub fn is_proxy_healthy(port: u16) -> bool {
    let Some(text) = probe_health_text(port) else {
        return false;
    };
    text.contains("200 OK")
        && (text.contains("tako-switch-gateway") || text.contains("tako-switch-agy-proxy"))
}

/// Verifies the running local gateway matches the current crate version (for seamless self-update reloads).
pub fn is_proxy_healthy_current_version(port: u16) -> bool {
    let Some(text) = probe_health_text(port) else {
        return false;
    };
    let expected_ver = format!("\"version\":\"{}\"", env!("CARGO_PKG_VERSION"));
    text.contains("200 OK") && text.contains("tako-switch-gateway") && text.contains(&expected_ver)
}

pub fn ensure_in_process_proxy_running() {
    if is_proxy_healthy(LOCAL_GATEWAY_PORT) {
        return;
    }
    if IN_PROCESS_PROXY_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        if let Err(err) = run_proxy_server(LOCAL_GATEWAY_PORT).await {
            eprintln!("tako-switch in-process gateway stopped: {err}");
            IN_PROCESS_PROXY_STARTED.store(false, Ordering::SeqCst);
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(not(target_os = "windows"))]
fn render_gateway_launch_agent_plist(exe_path: &str) -> String {
    let escaped_exe = xml_escape(exe_path);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>tech.shiroha.tako-switch.gateway</string>
  <key>ProgramArguments</key>
  <array>
    <string>{escaped_exe}</string>
    <string>gateway-proxy</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardErrorPath</key>
  <string>/tmp/tako-switch-gateway.err.log</string>
</dict>
</plist>
"#
    )
}

#[cfg(not(target_os = "windows"))]
pub fn gateway_plist_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.launch_agents_dir.join(GATEWAY_PLIST_NAME)
}

#[cfg(target_os = "macos")]
fn bootout_macos_label(uid: &str, label: &str) {
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["bootout", &format!("gui/{uid}/{label}")])
        .output();
}

#[cfg(target_os = "macos")]
fn apply_macos_gateway_launch_agent(plist_path: &std::path::Path) {
    if let Ok(uid_out) = std::process::Command::new("/usr/bin/id").arg("-u").output() {
        let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
        if !uid.is_empty() {
            bootout_macos_label(&uid, LEGACY_AGY_PROXY_PLIST_LABEL);
            bootout_macos_label(&uid, GATEWAY_PLIST_LABEL);
            let _ = std::process::Command::new("/bin/launchctl")
                .args([
                    "bootstrap",
                    &format!("gui/{uid}"),
                    &plist_path.to_string_lossy(),
                ])
                .output();
        }
    }
}

/// Ensures the shared local gateway daemon (macOS LaunchAgent / Windows in-process server) is running.
pub fn ensure_gateway_service(paths: &AppPaths) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let exe = paths.mcp_server_exe();
        let plist = render_gateway_launch_agent_plist(&exe.to_string_lossy());
        let plist_path = gateway_plist_path(paths);
        let legacy_path = paths.launch_agents_dir.join(LEGACY_AGY_PROXY_PLIST_NAME);

        if !paths.is_test
            && is_proxy_healthy_current_version(LOCAL_GATEWAY_PORT)
            && !legacy_path.exists()
            && fs::read_to_string(&plist_path).ok().as_deref() == Some(plist.as_str())
        {
            return Ok(());
        }

        if legacy_path.exists() {
            let _ = fs::remove_file(&legacy_path);
        }
        write_atomic(&plist_path, plist.as_bytes(), false)?;

        #[cfg(target_os = "macos")]
        if !paths.is_test {
            apply_macos_gateway_launch_agent(&plist_path);
            for _ in 0..10 {
                if is_proxy_healthy(LOCAL_GATEWAY_PORT) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(150));
            }
            ensure_in_process_proxy_running();
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        if !paths.is_test {
            ensure_in_process_proxy_running();
        }
        Ok(())
    }
}

/// Stops the local gateway daemon only when all gateway-backed desktop apps have returned to Official mode.
pub fn stop_gateway_service_if_unused(paths: &AppPaths) -> Result<(), String> {
    let any_tako = AppId::all()
        .iter()
        .copied()
        .filter(|id| id.requires_local_gateway())
        .any(|id| get_adapter(id).read_mode(paths) == AppMode::Tako);
    if any_tako {
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let plist_path = gateway_plist_path(paths);
        if plist_path.exists() {
            fs::remove_file(&plist_path)
                .map_err(|e| format!("删除 {} 失败: {e}", plist_path.display()))?;
        }
        let legacy_path = paths.launch_agents_dir.join(LEGACY_AGY_PROXY_PLIST_NAME);
        if legacy_path.exists() {
            let _ = fs::remove_file(&legacy_path);
        }
        #[cfg(target_os = "macos")]
        if !paths.is_test {
            if let Ok(uid_out) = std::process::Command::new("/usr/bin/id").arg("-u").output() {
                let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
                if !uid.is_empty() {
                    bootout_macos_label(&uid, GATEWAY_PLIST_LABEL);
                    bootout_macos_label(&uid, LEGACY_AGY_PROXY_PLIST_LABEL);
                }
            }
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pricing_metadata_extracts_reasoning_levels_and_image_flag() {
        let pricing_json = json!({
            "data": [
                {
                    "model_name": "gemini-3.8-flash",
                    "reasoning_levels": ["HIGH", "low", "medium", "low", "unknown"]
                },
                {
                    "model_name": "swe-2",
                    "reasoning_levels": ["max", "medium", "high"],
                    "description": "SWE coding model"
                },
                {
                    "model_name": "gemini-3.1-flash-image"
                },
                {
                    "model_name": "gpt-6.2-sol",
                    "reasoning_levels": null
                }
            ]
        });

        let parsed = parse_pricing_metadata(&pricing_json);
        assert!(parsed.has_flash_image);
        assert_eq!(
            parsed.descriptions.get("swe-2").map(String::as_str),
            Some("SWE coding model")
        );
        assert_eq!(
            parsed.reasoning_levels.get("gemini-3.8-flash"),
            Some(&vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ])
        );
        assert_eq!(
            parsed.reasoning_levels.get("swe-2"),
            Some(&vec![
                "medium".to_string(),
                "high".to_string(),
                "max".to_string()
            ])
        );
        assert!(!parsed.reasoning_levels.contains_key("gpt-6.2-sol"));
    }

    #[test]
    fn expand_models_catalog_creates_effort_variants_and_routes() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {"model_name": "gemini-3.8-flash", "reasoning_levels": ["low", "medium", "high"]},
                {"model_name": "swe-2", "reasoning_levels": ["medium", "high", "max"]},
                {"model_name": "gemini-3.1-flash-image"}
            ]
        }));

        let upstream = json!({
            "models": [
                {
                    "name": "models/gemini-3.8-flash",
                    "displayName": "gemini-3.8-flash",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                },
                {
                    "name": "models/gemini-3.1-pro-preview",
                    "displayName": "gemini-3.1-pro-preview",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                },
                {
                    "name": "models/swe-2",
                    "displayName": "swe-2",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                },
                {
                    "name": "models/claude-opus-4-6",
                    "displayName": "claude-opus-4-6",
                    "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                }
            ]
        });

        let (expanded, routes) = expand_models_catalog(upstream, &pricing);
        let list = expanded["models"].as_array().unwrap();
        let names: Vec<&str> = list
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();
        let displays: Vec<&str> = list
            .iter()
            .map(|m| m["displayName"].as_str().unwrap())
            .collect();

        assert!(names.contains(&"models/gemini-3.8-flash-low"));
        assert!(names.contains(&"models/gemini-3.8-flash-medium"));
        assert!(names.contains(&"models/gemini-3.8-flash-high"));
        assert!(names.contains(&"models/gemini-3.1-pro-low"));
        assert!(names.contains(&"models/gemini-3.1-pro-high"));
        assert!(names.contains(&"models/swe-2-medium"));
        assert!(names.contains(&"models/swe-2-high"));
        assert!(names.contains(&"models/swe-2-max"));
        assert!(names.contains(&"models/claude-opus-4-6"));
        assert!(names.contains(&"models/gemini-3.1-flash-image"));

        assert!(displays.contains(&"Gemini 3.8 Flash (Low)"));
        assert!(displays.contains(&"Gemini 3.8 Flash (Medium)"));
        assert!(displays.contains(&"Gemini 3.8 Flash (High)"));
        assert!(displays.contains(&"Gemini 3.1 Pro (Low)"));
        assert!(displays.contains(&"Gemini 3.1 Pro (High)"));
        assert!(displays.contains(&"SWE-2 (Max)"));
        assert!(displays.contains(&"Claude Opus 4.6"));

        assert_eq!(
            routes.get("gemini-3.8-flash-high"),
            Some(&SyntheticRoute {
                base_model: "gemini-3.8-flash".to_string(),
                effort: "high".to_string()
            })
        );
        assert_eq!(
            routes.get("gemini-3.1-pro-low"),
            Some(&SyntheticRoute {
                base_model: "gemini-3.1-pro-preview".to_string(),
                effort: "low".to_string()
            })
        );
        assert_eq!(
            routes.get("gemini-3.1-pro-preview-low"),
            Some(&SyntheticRoute {
                base_model: "gemini-3.1-pro-preview".to_string(),
                effort: "low".to_string()
            })
        );
    }

    #[test]
    fn expand_models_catalog_prettifies_gemini4_and_enforces_32_custom_cap() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {"model_name": "gemini-4-argon", "reasoning_levels": ["low", "medium", "high"]},
                {"model_name": "gemini-4-carbon", "reasoning_levels": ["low", "medium", "high"]},
                {"model_name": "gemini-4-hydrogen", "reasoning_levels": ["low", "medium", "high", "xhigh"]},
                {"model_name": "gemini-4-neon", "reasoning_levels": ["low", "medium", "high", "xhigh"]},
                {"model_name": "claude-opus-5-5", "reasoning_levels": ["low", "medium", "high"]},
                {"model_name": "claude-sonnet-5-5", "reasoning_levels": ["low", "medium", "high"]},
                {"model_name": "gemini-3.1-flash-image"}
            ]
        }));

        let mut upstream_models = vec![
            json!({"name": "models/gemini-4-argon", "displayName": "gemini-4-argon"}),
            json!({"name": "models/gemini-4-carbon", "displayName": "gemini-4-carbon"}),
            json!({"name": "models/gemini-4-hydrogen", "displayName": "gemini-4-hydrogen"}),
            json!({"name": "models/gemini-4-neon", "displayName": "gemini-4-neon"}),
            json!({"name": "models/gemini-3.1-pro-preview", "displayName": "gemini-3.1-pro-preview"}),
            json!({"name": "models/claude-opus-5-5", "displayName": "claude-opus-5-5"}),
            json!({"name": "models/claude-sonnet-5-5", "displayName": "claude-sonnet-5-5"}),
            json!({"name": "models/claude-opus-4-8", "displayName": "claude-opus-4-8"}),
            json!({"name": "models/claude-haiku-4-5", "displayName": "claude-haiku-4-5"}),
            json!({"name": "models/claude-haiku-4-5-20251001", "displayName": "claude-haiku-4-5-20251001"}),
            json!({"name": "models/kimi-k3", "displayName": "kimi-k3"}),
            json!({"name": "models/k3", "displayName": "k3"}),
            json!({"name": "models/deepseek-v4.1-flash", "displayName": "deepseek-v4.1-flash"}),
            json!({"name": "models/deepseek-v4.1-flash-1017", "displayName": "deepseek-v4.1-flash-1017"}),
            json!({"name": "models/gpt-6.2-sol", "displayName": "gpt-6.2-sol"}),
            json!({"name": "models/hy4flash-free", "displayName": "hy4flash-free"}),
        ];
        for i in 0..38 {
            upstream_models.push(json!({
                "name": format!("models/z-extra-model-{i:02}"),
                "displayName": format!("z-extra-model-{i:02}")
            }));
        }

        let (expanded, routes) = expand_models_catalog(json!({ "models": upstream_models }), &pricing);
        let list = expanded["models"].as_array().unwrap();
        let names: Vec<&str> = list.iter().map(|m| m["name"].as_str().unwrap()).collect();
        let displays: Vec<&str> = list
            .iter()
            .map(|m| m["displayName"].as_str().unwrap())
            .collect();

        assert!(displays.contains(&"Gemini 4 Argon (Low)"));
        assert!(displays.contains(&"Gemini 4 Carbon (High)"));
        assert!(displays.contains(&"Gemini 4 Hydrogen (XHigh)"));
        assert!(displays.contains(&"Gemini 4 Neon (XHigh)"));
        assert!(displays.contains(&"Claude Opus 5.5 (High)"));
        assert!(displays.contains(&"Claude Sonnet 5.5 (Medium)"));
        assert!(displays.contains(&"Claude Opus 4.8"));
        assert!(displays.contains(&"GPT-6.2 Sol"));
        assert!(displays.contains(&"DeepSeek V4.1 Flash"));

        assert!(!names.contains(&"models/claude-haiku-4-5-20251001"));
        assert!(!names.contains(&"models/deepseek-v4.1-flash-1017"));
        assert!(!names.contains(&"models/k3"));
        assert!(!names.contains(&"models/hy4flash-free"));
        assert!(names.contains(&"models/gemini-3.1-flash-image"));
        assert!(names.contains(&"models/gemini-3.1-pro-low"));

        let custom_count = names
            .iter()
            .filter(|n| !is_antigravity_builtin_model(n.strip_prefix("models/").unwrap_or(n)))
            .count();
        assert!(
            custom_count <= AGY_MAX_CUSTOM_PLACEHOLDERS,
            "expected <= 32 custom models, got {custom_count}"
        );
        assert_eq!(custom_count, AGY_MAX_CUSTOM_PLACEHOLDERS);

        assert_eq!(
            routes.get("gemini-4-neon-xhigh"),
            Some(&SyntheticRoute {
                base_model: "gemini-4-neon".to_string(),
                effort: "xhigh".to_string()
            })
        );
    }

    #[test]
    fn inject_thinking_effort_sets_level_and_removes_conflicting_budget() {
        let raw = br#"{
            "contents": [{"role":"user","parts":[{"text":"hi"}]}],
            "generationConfig": {
                "maxOutputTokens": 128,
                "thinkingConfig": {
                    "thinkingBudget": 8192,
                    "thinking_budget": 4096,
                    "thinking_level": "LOW"
                }
            }
        }"#;

        let out_bytes = inject_thinking_effort(raw, "high");
        let parsed: Value = serde_json::from_slice(&out_bytes).unwrap();
        let tc = parsed["generationConfig"]["thinkingConfig"]
            .as_object()
            .unwrap();
        assert_eq!(tc.get("thinkingLevel").and_then(Value::as_str), Some("HIGH"));
        assert_eq!(tc.get("includeThoughts").and_then(Value::as_bool), Some(true));
        assert!(!tc.contains_key("thinkingBudget"));
        assert!(!tc.contains_key("thinking_budget"));
        assert!(!tc.contains_key("thinking_level"));
    }

    #[test]
    fn resolve_synthetic_route_handles_restart_fallback() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {"model_name": "gemini-3.8-flash", "reasoning_levels": ["low", "medium", "high"]}
            ]
        }));
        let empty_routes = HashMap::new();

        assert_eq!(
            resolve_synthetic_route("gemini-3.8-flash-medium", &empty_routes, &pricing),
            Some(SyntheticRoute {
                base_model: "gemini-3.8-flash".to_string(),
                effort: "medium".to_string()
            })
        );
        assert_eq!(
            resolve_synthetic_route("gemini-3.1-pro-preview-high", &empty_routes, &pricing),
            Some(SyntheticRoute {
                base_model: "gemini-3.1-pro-preview".to_string(),
                effort: "high".to_string()
            })
        );
        assert_eq!(
            resolve_synthetic_route("gemini-3.1-pro-low", &empty_routes, &pricing),
            Some(SyntheticRoute {
                base_model: "gemini-3.1-pro-preview".to_string(),
                effort: "low".to_string()
            })
        );
        assert_eq!(
            resolve_synthetic_route("gemini-3.8-flash-max", &empty_routes, &pricing),
            None
        );
        assert_eq!(
            resolve_synthetic_route("claude-opus-4-6", &empty_routes, &pricing),
            None
        );
    }

    #[test]
    fn build_codex_models_catalog_maps_openai_response_models_and_efforts() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {"model_name": "swe-2", "reasoning_levels": ["medium", "high", "max"]},
                {"model_name": "deepseek-v4-pro", "description": "DeepSeek V4 Pro。开源 MoE 旗舰，长程编码与智能体。 上下文 1M。"}
            ]
        }));
        let upstream = json!({
            "data": [
                {"id": "claude-opus-4-8", "supported_endpoint_types": ["anthropic", "openai"]},
                {"id": "gemini-3.8-flash", "supported_endpoint_types": ["gemini", "openai", "anthropic"]},
                {"id": "deepseek-v4-pro", "supported_endpoint_types": ["openai", "openai-response", "anthropic"]},
                {"id": "deepseek-v4.1-flash", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "deepseek-v4.1-flash-1017", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "kimi-k3", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "k3", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "hy4flash-free", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "gpt-image-2", "supported_endpoint_types": ["openai-response"]},
                {"id": "codex-auto-review", "supported_endpoint_types": ["openai-response"]},
                {"id": "swe-2", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "gpt-6.2-sol", "supported_endpoint_types": ["openai", "openai-response"]}
            ]
        });

        let catalog = build_codex_models_catalog(&upstream, &pricing, None);
        let models = catalog["models"].as_array().unwrap();
        let slugs: Vec<&str> = models.iter().map(|m| m["slug"].as_str().unwrap()).collect();

        assert_eq!(
            slugs,
            vec![
                "gpt-6.2-sol",
                "swe-2",
                "claude-opus-4-8",
                "deepseek-v4.1-flash",
                "kimi-k3",
                "codex-auto-review"
            ]
        );
        assert!(!slugs.contains(&"deepseek-v4.1-flash-1017"));
        assert!(!slugs.contains(&"k3"));
        assert!(!slugs.contains(&"hy4flash-free"));
        assert!(!slugs.contains(&"gpt-image-2"));

        let sol = &models[0];
        assert_eq!(sol["display_name"].as_str(), Some("GPT-6.2 Sol"));
        assert_eq!(sol["description"].as_str(), Some("1M · 6.2 编程与智能体主力"));
        assert_eq!(sol["context_window"].as_u64(), Some(1_000_000));
        assert_eq!(sol["visibility"].as_str(), Some("list"));
        assert_eq!(sol["experimental_supported_tools"], json!([]));
        assert!(!sol["base_instructions"].as_str().unwrap().is_empty());

        let swe = &models[1];
        assert_eq!(swe["context_window"].as_u64(), Some(262_000));
        let swe_efforts: Vec<&str> = swe["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["effort"].as_str().unwrap())
            .collect();
        assert_eq!(swe_efforts, vec!["medium", "high", "max"]);

        let ds = &models[3];
        assert_eq!(ds["display_name"].as_str(), Some("DeepSeek V4.1 Flash"));

        let review = models.last().unwrap();
        assert_eq!(review["visibility"].as_str(), Some("hide"));
    }

    #[test]
    fn build_claude_models_catalog_filters_to_real_claude_models() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {
                    "model_name": "claude-opus-5",
                    "description": "Claude Opus 5。仅支持 Claude Code 客户端使用，其他客户端上游会报错。反代渠道，价格更便宜。最强 Claude 档的反代入口。 上下文 1M。"
                }
            ]
        }));
        let upstream = json!({
            "data": [
                {"id": "gpt-6.2-sol", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "deepseek-v4-pro", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "deepseek-v4.1-flash", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "deepseek-v4.1-flash-1017", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "gemini-3.8-flash", "owned_by": "google gemini", "supported_endpoint_types": ["gemini", "anthropic"]},
                {"id": "kimi-k3", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "k3", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "gpt-image-2", "owned_by": "openai", "supported_endpoint_types": ["image-generation", "anthropic"]},
                {"id": "jev-latest", "owned_by": "systemone", "supported_endpoint_types": ["systemone", "anthropic"]},
                {"id": "hy4flash-free", "owned_by": "openai", "supported_endpoint_types": ["openai", "anthropic"]},
                {"id": "claude-haiku-4-5-20251001", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-haiku-4-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-haiku-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-sonnet-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-opus-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-opus-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-fable-5-2", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]}
            ]
        });

        let catalog = build_claude_models_catalog(&upstream, &pricing);
        let data = catalog["data"].as_array().unwrap();
        let ids: Vec<&str> = data.iter().map(|m| m["id"].as_str().unwrap()).collect();

        assert_eq!(
            ids,
            vec![
                "claude-opus-5-5",
                "claude-opus-5",
                "claude-sonnet-5-5",
                "claude-fable-5-2",
                "claude-haiku-5-5",
                "claude-haiku-4-5"
            ]
        );
        assert!(!ids.contains(&"claude-haiku-4-5-20251001"));
        assert!(!ids.contains(&"deepseek-v4-pro"));
        assert!(!ids.contains(&"deepseek-v4.1-flash"));
        assert!(!ids.contains(&"gpt-6.2-sol"));
        assert!(!ids.contains(&"gemini-3.8-flash"));
        assert!(!ids.contains(&"kimi-k3"));
        assert!(!ids.contains(&"gpt-image-2"));
        assert!(!ids.contains(&"jev-latest"));
        assert!(!ids.contains(&"hy4flash-free"));

        // Every entry sets supports_1m = false to eliminate duplicate "[1m]" rows in Claude Desktop.
        assert!(data.iter().all(|m| m["supports_1m"].as_bool() == Some(false)));
        // Every entry advertises an effort ceiling so Claude Desktop renders the effort selector.
        assert!(data
            .iter()
            .all(|m| matches!(m["max_effort_level"].as_str(), Some("low" | "medium" | "high" | "xhigh" | "max"))));

        assert_eq!(data[0]["display_name"].as_str(), Some("Claude Opus 5.5"));
        assert_eq!(data[0]["is_family_default"].as_bool(), Some(true));
        assert_eq!(data[0]["max_input_tokens"].as_u64(), Some(1_000_000));
        assert_eq!(data[1]["description"].as_str(), Some("1M · 旗舰推理与编码"));
        assert!(data[1].get("is_family_default").is_none());
        assert_eq!(data[2]["is_family_default"].as_bool(), Some(true));
        assert_eq!(data[5]["max_input_tokens"].as_u64(), Some(200_000));
    }

    #[test]
    fn filter_claude_models_catalog_handles_recommended_custom_all_and_fallback() {
        let pricing = PricingCache::default();
        let upstream = json!({
            "data": [
                {"id": "claude-opus-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-opus-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-sonnet-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-fable-5-2", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-haiku-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "claude-haiku-4-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]}
            ]
        });

        let full = build_claude_models_catalog(&upstream, &pricing);

        let rec = filter_claude_models_catalog(&full, &ClientModelFilter::default_claude());
        let rec_data = rec["data"].as_array().unwrap();
        let rec_ids: Vec<&str> = rec_data.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(
            rec_ids,
            vec!["claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-2"]
        );
        assert!(rec_data
            .iter()
            .all(|m| m["is_family_default"].as_bool() == Some(true)));
        assert_eq!(rec["first_id"].as_str(), Some("claude-opus-5-5"));
        assert_eq!(rec["last_id"].as_str(), Some("claude-fable-5-2"));

        let custom = filter_claude_models_catalog(
            &full,
            &ClientModelFilter {
                mode: ModelFilterMode::Custom,
                enabled_ids: vec!["claude-opus-5".to_string(), "claude-haiku-4-5".to_string()],
            },
        );
        let custom_data = custom["data"].as_array().unwrap();
        let custom_ids: Vec<&str> = custom_data.iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(custom_ids, vec!["claude-opus-5", "claude-haiku-4-5"]);
        assert_eq!(custom_data[0]["is_family_default"].as_bool(), Some(true));
        assert_eq!(custom_data[1]["is_family_default"].as_bool(), Some(true));

        let fallback = filter_claude_models_catalog(
            &full,
            &ClientModelFilter {
                mode: ModelFilterMode::Custom,
                enabled_ids: vec!["claude-nonexistent".to_string()],
            },
        );
        let fallback_ids: Vec<&str> = fallback["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            fallback_ids,
            vec![
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "claude-fable-5-2",
                "claude-haiku-5-5"
            ]
        );

        let all = filter_claude_models_catalog(
            &full,
            &ClientModelFilter {
                mode: ModelFilterMode::All,
                enabled_ids: vec![],
            },
        );
        let all_data = all["data"].as_array().unwrap();
        assert_eq!(all_data.len(), 6);
        assert!(all_data[1].get("is_family_default").is_none());

        let empty_perf = HashMap::new();
        let ui_opts = build_claude_model_options(
            &upstream,
            &pricing,
            &empty_perf,
            &ClientModelFilter::default_claude(),
        );
        assert_eq!(ui_opts.visible_count, 3);
        assert_eq!(ui_opts.total_count, 6);
        assert_eq!(ui_opts.models[0].family, "Opus");
        assert!(ui_opts.models[0].visible);
        assert!(!ui_opts.models[1].visible);
    }

    #[test]
    fn claude_inference_models_carry_tier_and_effort() {
        let pricing = PricingCache::default();
        let upstream = json!({
            "data": [
                {"id": "claude-opus-5-5", "owned_by": "claude", "supported_endpoint_types": ["anthropic"]},
                {"id": "gpt-6.2-sol", "owned_by": "openai", "supported_endpoint_types": ["anthropic"]}
            ]
        });
        let full = build_claude_models_catalog(&upstream, &pricing);
        let models = build_claude_inference_models(&full, &ClientModelFilter::default_claude());
        let arr = models.as_array().unwrap();
        assert_eq!(arr.len(), 1);

        let opus = &arr[0];
        assert_eq!(opus["name"].as_str(), Some("claude-opus-5-5"));
        assert_eq!(opus["labelOverride"].as_str(), Some("Claude Opus 5.5"));
        assert_eq!(opus["anthropicFamilyTier"].as_str(), Some("opus"));
        assert_eq!(opus["isFamilyDefault"].as_bool(), Some(true));
        assert_eq!(opus["maxEffort"].as_str(), Some("max"));
    }

    #[test]
    fn filter_codex_models_catalog_preserves_hidden_review_and_filters_visible() {
        let pricing = PricingCache::default();
        let upstream = json!({
            "data": [
                {"id": "gpt-6.2-sol", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "swe-2", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "claude-opus-4-8", "supported_endpoint_types": ["anthropic", "openai"]},
                {"id": "deepseek-v4-pro", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "deepseek-v4.1-flash", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "gemini-3.8-flash", "supported_endpoint_types": ["gemini", "openai"]},
                {"id": "kimi-k3", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "codex-auto-review", "supported_endpoint_types": ["openai-response"]}
            ]
        });

        let full = build_codex_models_catalog(&upstream, &pricing, None);
        let rec_filter = ClientModelFilter {
            mode: ModelFilterMode::Recommended,
            enabled_ids: vec![],
        };
        let rec = filter_codex_models_catalog(&full, &rec_filter);
        let rec_slugs: Vec<&str> = rec["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["slug"].as_str().unwrap())
            .collect();
        assert_eq!(
            rec_slugs,
            vec![
                "gpt-6.2-sol",
                "swe-2",
                "deepseek-v4.1-flash",
                "codex-auto-review"
            ]
        );
        assert_eq!(
            rec["models"].as_array().unwrap().last().unwrap()["visibility"].as_str(),
            Some("hide")
        );

        let custom = filter_codex_models_catalog(
            &full,
            &ClientModelFilter {
                mode: ModelFilterMode::Custom,
                enabled_ids: vec!["kimi-k3".to_string()],
            },
        );
        let custom_slugs: Vec<&str> = custom["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["slug"].as_str().unwrap())
            .collect();
        assert_eq!(custom_slugs, vec!["kimi-k3", "codex-auto-review"]);

        let fallback = filter_codex_models_catalog(
            &full,
            &ClientModelFilter {
                mode: ModelFilterMode::Custom,
                enabled_ids: vec!["missing-model".to_string()],
            },
        );
        let fallback_slugs: Vec<&str> = fallback["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["slug"].as_str().unwrap())
            .collect();
        assert_eq!(
            fallback_slugs,
            vec!["gpt-6.2-sol", "swe-2", "claude-opus-4-8", "codex-auto-review"]
        );

        let empty_perf = HashMap::new();
        let ui_opts = build_codex_model_options(&upstream, &pricing, &empty_perf, &rec_filter);
        assert_eq!(ui_opts.visible_count, 3);
        assert_eq!(ui_opts.total_count, 5);
        assert!(!ui_opts.models.iter().any(|m| m.id == "codex-auto-review"));
    }

    #[test]
    fn expand_models_catalog_with_filter_filters_before_placeholder_cap() {
        let pricing = parse_pricing_metadata(&json!({
            "data": [
                {"model_name": "gemini-4-neon", "reasoning_levels": ["low", "medium", "high", "xhigh"]},
                {"model_name": "swe-2", "reasoning_levels": ["medium", "high", "max"]}
            ]
        }));

        let mut upstream_models = vec![
            json!({"name": "models/gemini-4-neon", "displayName": "gemini-4-neon"}),
            json!({"name": "models/deepseek-v4.1-flash", "displayName": "deepseek-v4.1-flash"}),
            json!({"name": "models/swe-2", "displayName": "swe-2"}),
        ];
        for i in 0..38 {
            upstream_models.push(json!({
                "name": format!("models/z-extra-model-{i:02}"),
                "displayName": format!("z-extra-model-{i:02}")
            }));
        }
        let upstream = json!({ "models": upstream_models });

        let custom_filter = ClientModelFilter {
            mode: ModelFilterMode::Custom,
            enabled_ids: vec!["deepseek-v4.1-flash".to_string(), "swe-2".to_string()],
        };
        let (filtered, routes) =
            expand_models_catalog_with_filter(upstream.clone(), &pricing, Some(&custom_filter));
        let names: Vec<&str> = filtered["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect();

        assert_eq!(
            names,
            vec![
                "models/deepseek-v4.1-flash",
                "models/swe-2-medium",
                "models/swe-2-high",
                "models/swe-2-max",
            ]
        );
        assert!(routes.contains_key("gemini-4-neon-xhigh"));

        let empty_perf = HashMap::new();
        let ui_opts =
            build_antigravity_model_options(&upstream, &pricing, &empty_perf, &custom_filter);
        assert_eq!(ui_opts.visible_count, 2);
        assert_eq!(ui_opts.total_count, 41);
        assert_eq!(
            ui_opts
                .models
                .iter()
                .find(|m| m.id == "gemini-4-neon")
                .map(|m| m.effort_levels.clone()),
            Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "xhigh".to_string()
            ])
        );
    }

    #[test]
    fn snapshot_suffix_and_concise_descriptions_behave_cleanly() {
        assert_eq!(
            strip_dated_snapshot_suffix("claude-haiku-4-5-20251001"),
            Some("claude-haiku-4-5")
        );
        assert_eq!(
            strip_dated_snapshot_suffix("deepseek-v4.1-flash-1017"),
            Some("deepseek-v4.1-flash")
        );
        assert_eq!(strip_dated_snapshot_suffix("gpt-6.2-sol"), None);
        assert_eq!(strip_dated_snapshot_suffix("model-9999"), None);

        let sanitized = concise_model_description(
            "future-model-x",
            Some("Future Model X。仅支持 Claude Code 客户端使用，其他客户端上游会报错。反代渠道，价格更便宜。专注超快代码重构与单测生成。上下文 1M。"),
        );
        assert_eq!(sanitized, "专注超快代码重构与单测生成");
        assert_eq!(concise_model_description("unknown-empty", None), "Tako 托管模型");
    }

    #[test]
    fn normalize_claude_request_body_strips_1m_suffix() {
        let with_1m = br#"{"model":"claude-opus-4-8[1m]","max_tokens":64}"#;
        let out_1m: Value =
            serde_json::from_slice(&normalize_claude_request_body(with_1m)).unwrap();
        assert_eq!(out_1m["model"].as_str(), Some("claude-opus-4-8"));

        let plain = br#"{"model":"claude-sonnet-5-5","max_tokens":64}"#;
        assert_eq!(normalize_claude_request_body(plain), plain);
    }

    #[test]
    fn parse_pricing_and_perf_metrics_populates_price_and_speed_on_options() {
        let pricing = parse_pricing_metadata(&json!({
            "group_ratio": { "default": 1.0, "agy": 1.0 },
            "data": [
                {
                    "model_name": "claude-opus-5-5",
                    "quota_type": 0,
                    "model_ratio": 2.0,
                    "completion_ratio": 5.0,
                    "model_price": 0.0,
                    "standard_load_price_per_million": 0.664,
                    "realized_price_per_million": 178.166,
                    "cache_hit_rate": 0.9278,
                    "enable_groups": ["default", "agy"],
                    "group_channel_ratio_min": { "default": 3.6, "agy": 0.05 },
                    "group_channel_ratio_max": { "default": 3.6, "agy": 0.05 },
                    "channels": [
                        {
                            "channel_name": "Antigravity反代池-Claude5.5-0.05x",
                            "channel_ratio": 0.05,
                            "standard_load_price_per_million": 0.0332
                        },
                        {
                            "channel_name": "CC Max-官方订阅-0.15x",
                            "channel_ratio": 3.6,
                            "standard_load_price_per_million": 2.3904
                        }
                    ]
                },
                {
                    "model_name": "gemma-4-31b-it",
                    "quota_type": 0,
                    "model_ratio": 0.0,
                    "completion_ratio": 1.0,
                    "model_price": 0.0,
                    "enable_groups": ["default"],
                    "group_channel_ratio_min": { "default": 5.25 }
                },
                {
                    "model_name": "tako-auto",
                    "quota_type": 1,
                    "model_ratio": 0.0,
                    "completion_ratio": 0.0,
                    "model_price": 0.0,
                    "enable_groups": ["default"]
                },
                {
                    "model_name": "grok-4.6",
                    "quota_type": 0,
                    "billing_mode": "tiered_expr",
                    "billing_expr": "len <= 200000 ? tier(\"standard\", p * 2 + c * 6 + cr * 0.5) : tier(\"long\", p * 4 + c * 12)",
                    "model_ratio": 0.0,
                    "completion_ratio": 3.0,
                    "model_price": 0.0,
                    "realized_price_per_million": 0.919,
                    "cache_hit_rate": 0.372,
                    "enable_groups": ["default"],
                    "group_channel_ratio_min": { "default": 0.53 }
                },
                {
                    "model_name": "deepseek-v4.1-flash",
                    "quota_type": 0,
                    "model_ratio": 0.15,
                    "completion_ratio": 4.0,
                    "model_price": 0.0,
                    "standard_load_price_per_million": 0.03852,
                    "enable_groups": ["default"],
                    "group_channel_ratio_min": { "default": 2.63 }
                }
            ]
        }));

        let perf = parse_perf_metrics_summary(&json!({
            "success": true,
            "data": {
                "models": [
                    {
                        "model_name": "claude-opus-5-5",
                        "avg_tps": 119.6,
                        "avg_latency_ms": 10416,
                        "success_rate": 100.0
                    },
                    {
                        "model_name": "deepseek-v4.1-flash",
                        "avg_tps": 297.57,
                        "avg_latency_ms": 4274,
                        "success_rate": 99.77
                    },
                    {
                        "model_name": "tako-auto",
                        "avg_tps": 0.0,
                        "avg_latency_ms": 1200,
                        "success_rate": 100.0
                    }
                ]
            }
        }));

        let upstream = json!({
            "data": [
                {"id": "claude-opus-5-5", "supported_endpoint_types": ["anthropic", "openai"]},
                {"id": "deepseek-v4.1-flash", "supported_endpoint_types": ["openai", "openai-response"]},
                {"id": "gemma-4-31b-it", "supported_endpoint_types": ["gemini", "openai"]},
                {"id": "grok-4.6", "supported_endpoint_types": ["openai"]},
                {"id": "tako-auto", "supported_endpoint_types": ["openai"]}
            ]
        });

        let resp = build_codex_model_options(
            &upstream,
            &pricing,
            &perf,
            &ClientModelFilter::default_codex(),
        );
        let opus = resp.models.iter().find(|m| m.id == "claude-opus-5-5").unwrap();
        assert!(!opus.is_free);
        assert!(!opus.is_dynamic_pricing);
        assert!((opus.standard_load_usd_per_m.unwrap() - 0.0332).abs() < 1e-6);
        assert!((opus.standard_load_max_usd_per_m.unwrap() - 2.3904).abs() < 1e-6);
        assert!((opus.realized_usd_per_m.unwrap() - 178.166).abs() < 1e-6);
        assert!((opus.cache_hit_rate.unwrap() - 0.9278).abs() < 1e-6);
        assert_eq!(opus.channels.len(), 2);
        assert!((opus.input_usd_per_m.unwrap() - 0.2).abs() < 1e-6);
        assert!((opus.output_usd_per_m.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(opus.avg_tps, Some(119.6));
        assert_eq!(opus.avg_latency_ms, Some(10416.0));
        assert_eq!(opus.success_rate, Some(100.0));

        // Gemini-family models never reach the Codex catalog.
        assert!(!resp.models.iter().any(|m| m.id == "gemma-4-31b-it"));
        assert!(pricing.prices.get("gemma-4-31b-it").unwrap().is_free);

        let auto = resp.models.iter().find(|m| m.id == "tako-auto").unwrap();
        assert!(!auto.is_free);
        assert_eq!(auto.avg_tps, None);
        assert_eq!(auto.avg_latency_ms, Some(1200.0));

        let grok = resp.models.iter().find(|m| m.id == "grok-4.6").unwrap();
        assert!(!grok.is_free);
        assert!(grok.is_dynamic_pricing);
        assert_eq!(grok.standard_load_usd_per_m, None);
        assert!((grok.realized_usd_per_m.unwrap() - 0.919).abs() < 1e-6);
        assert!((grok.cache_hit_rate.unwrap() - 0.372).abs() < 1e-6);

        let ds = resp
            .models
            .iter()
            .find(|m| m.id == "deepseek-v4.1-flash")
            .unwrap();
        assert!(!ds.is_free);
        assert!((ds.standard_load_usd_per_m.unwrap() - (0.03852 * 2.63)).abs() < 1e-6);
        assert_eq!(ds.standard_load_max_usd_per_m, None);
        assert_eq!(ds.avg_tps, Some(297.57));
    }

    #[test]
    fn plaza_model_options_syncs_pr296_pricing_multipliers_promos_and_capabilities() {
        let pricing_body = json!({
            "group_ratio": { "default": 1.0 },
            "data": [
                {
                    "model_name": "gemini-4-carbon",
                    "description": "Google 旗舰多模态编程推理模型，支持 2M 上下文。",
                    "quota_type": 0,
                    "model_ratio": 1.5,
                    "completion_ratio": 4.0,
                    "cache_ratio": 0.1,
                    "enable_groups": ["default"],
                    "reasoning_levels": ["low", "medium", "high"],
                    "tags": ["限时特惠", "编程"],
                    "channels": [
                        { "channel_name": "official-pool", "channel_ratio": 1.224, "standard_load_price_per_million": 0.42 },
                        { "channel_name": "backup", "channel_ratio": 2.04, "standard_load_price_per_million": 0.70 }
                    ]
                },
                {
                    "model_name": "claude-opus-5-5",
                    "description": "旗舰长程智能体",
                    "quota_type": 0,
                    "billing_mode": "tiered_expr",
                    "tiered_prices": [
                        { "inputPrice": 5.0, "cacheReadPrice": 0.5, "outputPrice": 25.0 }
                    ],
                    "enable_groups": ["default"],
                    "tags": ["1M", "智能体"],
                    "channels": [
                        { "channel_name": "tiered", "channel_ratio": 2.0 }
                    ]
                },
                {
                    "model_name": "flux-1.1-pro",
                    "description": "高质量文生图模型",
                    "quota_type": 1,
                    "model_price": 0.12,
                    "enable_groups": ["default"],
                    "supported_endpoint_types": ["image"]
                }
            ]
        });
        let perf = parse_perf_metrics_summary(&json!({
            "data": {
                "models": [
                    { "model_name": "gemini-4-carbon", "avg_tps": 168.5, "avg_latency_ms": 2100.0, "success_rate": 99.8 }
                ]
            }
        }));

        let resp = build_plaza_model_options(&pricing_body, &perf);
        assert_eq!(resp.app_id, AppId::DshTako);
        assert_eq!(resp.total_count, 3);

        let carbon = resp
            .models
            .iter()
            .find(|m| m.id == "gemini-4-carbon")
            .unwrap();
        assert_eq!(carbon.vendor_name.as_deref(), Some("Google"));
        assert_eq!(carbon.context_window, Some(2_000_000));
        assert!((carbon.input_usd_per_m.unwrap() - 3.0).abs() < 1e-6);
        assert!((carbon.cache_usd_per_m.unwrap() - 0.3).abs() < 1e-6);
        assert!((carbon.output_usd_per_m.unwrap() - 12.0).abs() < 1e-6);
        assert!((carbon.official_multiplier.unwrap() - 0.06).abs() < 1e-6);
        assert_eq!(
            resolve_promo_ends_on_at_date("gemini-4-carbon", None, "2026-11-22").as_deref(),
            Some("2026-11-30")
        );
        assert_eq!(
            resolve_promo_ends_on_at_date("gemini-4-carbon", None, "2026-12-01"),
            None
        );
        assert!(carbon.capabilities.contains(&"chat".to_string()));
        assert!(carbon.capabilities.contains(&"reason".to_string()));
        assert!(carbon.capabilities.contains(&"vision".to_string()));
        assert!(carbon.capabilities.contains(&"code".to_string()));
        assert_eq!(carbon.avg_tps, Some(168.5));

        let opus = resp
            .models
            .iter()
            .find(|m| m.id == "claude-opus-5-5")
            .unwrap();
        assert!(opus.is_dynamic_pricing);
        assert_eq!(opus.official_multiplier, None);
        assert!((opus.cache_usd_per_m.unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(opus.context_window, Some(1_000_000));

        let flux = resp
            .models
            .iter()
            .find(|m| m.id == "flux-1.1-pro")
            .unwrap();
        assert_eq!(flux.quota_type, 1);
        assert!((flux.request_usd.unwrap() - 0.12).abs() < 1e-6);
        assert_eq!(flux.official_multiplier, None);
        assert!(flux.capabilities.contains(&"image".to_string()));
        assert!(!flux.capabilities.contains(&"chat".to_string()));
        assert_eq!(flux.context_window, None);
    }
}
