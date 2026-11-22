use crate::config::{
    read_main_api_key, read_switch_state, resolve_output_dir, write_atomic, AppPaths,
    McpToolsConfig,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::multipart;
use reqwest::redirect::Policy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MAX_VOICE_SAMPLE_BYTES: usize = 10 * 1024 * 1024;
const MAX_LIVE_DEMO_PREVIEW_BYTES: u64 = 15 * 1024 * 1024;

fn build_api_client(timeout_secs: u64, bypass_system_proxy: bool) -> Result<Client, String> {
    crate::config::apply_proxy_policy(
        Client::builder()
            .redirect(Policy::none())
            .timeout(Duration::from_secs(timeout_secs)),
        bypass_system_proxy,
    )
    .build()
    .map_err(|e| format!("初始化 Tako API HTTP 客户端失败: {e}"))
}

fn build_cdn_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("初始化媒体下载客户端失败: {e}"))
}

fn timestamp_slug() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

fn expand_user_path(paths: &AppPaths, raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        paths.home.join(rest)
    } else {
        PathBuf::from(trimmed)
    }
}

fn detect_image_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/jpeg",
    }
}

fn detect_audio_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("ogg") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        _ => "audio/wav",
    }
}

fn detect_image_format_from_bytes(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(("png", "image/png"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("jpg", "image/jpeg"));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(("webp", "image/webp"));
    }
    None
}

fn is_valid_mp4(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[4..8] == b"ftyp"
}

fn resolve_target_file(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    explicit_path: Option<&str>,
    prefix: &str,
    ext: &str,
) -> PathBuf {
    if let Some(p) = explicit_path.map(str::trim).filter(|s| !s.is_empty()) {
        return expand_user_path(paths, p);
    }
    let dir = resolve_output_dir(paths, &cfg.output_dir);
    dir.join(format!("{prefix}-{}.{ext}", timestamp_slug()))
}

pub fn build_tools_catalog(cfg: &McpToolsConfig) -> Vec<Value> {
    let mut tools = Vec::new();
    if cfg.image {
        tools.push(json!({
            "name": "tako_generate_image",
            "description": "Generate or edit an image via Tako and save it directly to local disk (per tako-skill). Supports gpt-image-2 (default), gpt-image-2.5-flare, gpt-image-2.5-sunburst, gemini-3.1-flash-image, gemini-3-pro-image, grok-imagine-image-quality, wan2.7-image, wan2.7-image-pro. Preserve the user's explicit model choice and NEVER auto-retry paid calls on failure or timeout.",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Detailed prompt describing the image to generate or edit." },
                    "model": {
                        "type": "string",
                        "description": "Image model ID (default: gpt-image-2). Common options: gpt-image-2, gpt-image-2.5-flare, gpt-image-2.5-sunburst, gemini-3.1-flash-image, gemini-3-pro-image, grok-imagine-image-quality, wan2.7-image, wan2.7-image-pro."
                    },
                    "size": {
                        "type": "string",
                        "enum": ["1024x1024", "1536x1024", "1024x1536", "1792x1024", "1024x1792", "auto"],
                        "description": "Output size for OpenAI/Grok/WAN image models (or mapped to Gemini aspectRatio when aspect_ratio is omitted)."
                    },
                    "aspect_ratio": {
                        "type": "string",
                        "enum": ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
                        "description": "Target aspect ratio (mapped to Gemini aspectRatio or OpenAI size when size is omitted)."
                    },
                    "input_image_path": { "type": "string", "description": "Optional single local reference image path for image editing." },
                    "input_image_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional multiple local reference image paths for multi-reference editing (Grok supports up to 3 references)."
                    },
                    "output_path": { "type": "string", "description": "Optional local output file path (defaults to ~/Downloads/Tako/tako-image-<ts>.<ext>)." }
                },
                "required": ["prompt"]
            }
        }));
    }
    if cfg.video {
        tools.push(json!({
            "name": "tako_generate_video",
            "description": "Generate a video via Tako POST /v1/videos/generations (async 3-step: JSON submit -> poll -> download verified MP4 to local disk, per tako-skill). grok-imagine-video-1.5 is the only video model (legacy grok-imagine-video maps to it). Per-second billing: NEVER auto-resubmit on timeout or failure; pass resume_task_id to recover an existing job.",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Video prompt (required unless resume_task_id is provided)." },
                    "model": { "type": "string", "enum": ["grok-imagine-video-1.5", "grok-imagine-video"], "description": "Default: grok-imagine-video-1.5 (legacy grok-imagine-video maps to grok-imagine-video-1.5)." },
                    "duration": { "type": "integer", "minimum": 1, "maximum": 15, "description": "Duration in seconds (integer 1-15, default 5)." },
                    "resolution": { "type": "string", "enum": ["480p", "720p", "1080p"], "description": "Default: 480p (per tako-skill recommendation; also supports 720p and 1080p)." },
                    "aspect_ratio": { "type": "string", "enum": ["16:9", "3:2", "4:3", "1:1", "3:4", "2:3", "9:16"] },
                    "input_reference_path": { "type": "string", "description": "Optional local image path or public URL / data URI for image-to-video." },
                    "resume_task_id": { "type": "string", "description": "Existing video task ID to resume polling and download without creating a new paid job." },
                    "wait_timeout_secs": { "type": "integer", "minimum": 5, "maximum": 300, "description": "Max seconds to poll before returning task_id (default 120)." },
                    "output_path": { "type": "string", "description": "Optional local MP4 path (defaults to ~/Downloads/Tako/tako-video-<ts>.mp4)." }
                }
            }
        }));
    }
    if cfg.audio {
        tools.push(json!({
            "name": "tako_generate_audio",
            "description": "Synthesize speech via Tako POST /v1/audio/speech and save the audio file to local disk. Supports mimo-v2.5-tts (default), speech_tts, mimo-v2.5-tts-voicedesign (style-guided design without voice), and mimo-v2.5-tts-voiceclone (with voice_sample_path).",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "Text content to synthesize into speech (optional only when model is mimo-v2.5-tts-voicedesign with instructions)." },
                    "model": {
                        "type": "string",
                        "description": "TTS model: mimo-v2.5-tts (default), speech_tts, mimo-v2.5-tts-voicedesign, or mimo-v2.5-tts-voiceclone."
                    },
                    "voice": { "type": "string", "description": "Preset voice name (mimo_default, 冰糖, 茉莉, 苏打, 白桦, Mia, Chloe, Milo, Dean; default 冰糖). Do NOT send for voicedesign." },
                    "instructions": { "type": "string", "description": "Optional speaking style or voice design description." },
                    "voice_sample_path": { "type": "string", "description": "Local audio sample file path (<=10MB) for mimo-v2.5-tts-voiceclone." },
                    "response_format": { "type": "string", "enum": ["wav", "mp3", "pcm"], "description": "Output format: wav (default), mp3, or pcm. (opus is unsupported by Tako Speech API.)" },
                    "output_path": { "type": "string", "description": "Optional local audio file path." }
                }
            }
        }));
    }
    if cfg.search {
        tools.push(json!({
            "name": "tako_web_search",
            "description": "Search the live web via Tako POST /v1/search and return answer summary + ranked results. Always cite actual results[].url from the response and never fabricate URLs when results is empty.",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query string." },
                    "count": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Number of results (1-20, default 5)." },
                    "provider": {
                        "type": "string",
                        "enum": ["deepseek", "groq", "grok", "grok_x"],
                        "description": "Optional search provider family (deepseek, groq, grok, grok_x); omit for automatic routing."
                    }
                },
                "required": ["query"]
            }
        }));
    }
    if cfg.consult {
        tools.push(json!({
            "name": "tako_consult",
            "description": "Consult another frontier model via Tako POST /v1/chat/completions for a second opinion, architecture critique, or verification (e.g. gpt-6.2-sol, gpt-6.2-astra, deepseek-v4.1-flash, swe-2, claude-opus-5-5, deepseek-v4-pro, gemini-3.8-flash).",
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "Question or decision prompt to consult on." },
                    "context": { "type": "string", "description": "Optional relevant code/logs/constraints context." },
                    "model": { "type": "string", "description": "Model ID (default from Tako Switch settings, e.g. gpt-6.2-sol / gpt-6.2-astra / deepseek-v4.1-flash / swe-2 / claude-opus-5-5 / deepseek-v4-pro / gemini-3.8-flash)." },
                    "system_prompt": { "type": "string", "description": "Optional advisor system instruction." }
                },
                "required": ["question"]
            }
        }));
    }
    tools.push(json!({
        "name": "tako_account_status",
        "description": "Query the current Tako account balance (USD remaining and used), plan/subscription quota (if any), and gateway status in real time.",
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": true
        },
        "inputSchema": {
            "type": "object",
            "properties": {},
            "required": []
        }
    }));
    tools
}

fn normalize_tool_arguments(raw: Option<&Value>) -> Value {
    let mut cur = raw.cloned().unwrap_or_else(|| json!({}));
    for _ in 0..4 {
        match cur {
            Value::String(ref s) => {
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    cur = json!({});
                    break;
                }
                if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
                    cur = parsed;
                } else {
                    break;
                }
            }
            Value::Object(ref map) if map.len() == 1 => {
                if let Some(inner) = map.get("Arguments").or_else(|| map.get("arguments")) {
                    if inner.is_string() || inner.is_object() {
                        cur = inner.clone();
                        continue;
                    }
                }
                break;
            }
            _ => break,
        }
    }
    if cur.is_object() {
        cur
    } else {
        json!({})
    }
}

pub async fn handle_jsonrpc_message(paths: &AppPaths, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

    if method.starts_with("notifications/") || id.is_none() {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2024-11-05");
            Ok(json!({
                "protocolVersion": requested,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "tako-switch-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "instructions": "Prefer Tako MCP tools whenever the user asks for image generation/editing (tako_generate_image, default gpt-image-2), video generation (tako_generate_video, grok-imagine-video-1.5), speech synthesis (tako_generate_audio, default mimo-v2.5-tts), live web search (tako_web_search), a second-model opinion (tako_consult), or Tako balance/quota checks (tako_account_status). Follow tako-skill guardrails: preserve the user's explicit model choice; NEVER auto-retry paid image or video calls on timeout/error (check tako_account_status first, and pass resume_task_id for video recovery); cite actual results[].url on web search without fabricating URLs; never delegate image tasks to built-in `image-generator` subagents when Tako MCP is mounted."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let state = read_switch_state(paths);
            Ok(json!({
                "tools": build_tools_catalog(&state.mcp)
            }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = normalize_tool_arguments(params.get("arguments"));
            match call_tool(paths, name, args).await {
                Ok(text) => Ok(json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                })),
                Err(err) => Ok(json!({
                    "content": [{ "type": "text", "text": format!("Error: {err}") }],
                    "isError": true
                })),
            }
        }
        other => Err((-32601, format!("Method not found: {other}"))),
    };

    match result {
        Ok(res) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": res
        })),
        Err((code, message)) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        })),
    }
}

pub async fn run_stdio_server() -> Result<(), String> {
    let paths = AppPaths::detect()?;
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|e| format!("读取 stdin 失败: {e}"))?
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            let err_resp = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": "Parse error" }
            });
            let out_line = format!("{err_resp}\n");
            let _ = stdout.write_all(out_line.as_bytes()).await;
            let _ = stdout.flush().await;
            continue;
        };

        if let Some(resp) = handle_jsonrpc_message(&paths, msg).await {
            let out_line = format!("{resp}\n");
            stdout
                .write_all(out_line.as_bytes())
                .await
                .map_err(|e| format!("写入 stdout 失败: {e}"))?;
            stdout
                .flush()
                .await
                .map_err(|e| format!("flush stdout 失败: {e}"))?;
        }
    }
    Ok(())
}

async fn call_tool(paths: &AppPaths, name: &str, args: Value) -> Result<String, String> {
    if name == "tako_account_status" {
        return exec_account_status(paths).await;
    }
    let state = read_switch_state(paths);
    let enabled = match name {
        "tako_generate_image" => state.mcp.image,
        "tako_generate_video" => state.mcp.video,
        "tako_generate_audio" => state.mcp.audio,
        "tako_web_search" => state.mcp.search,
        "tako_consult" => state.mcp.consult,
        _ => return Err(format!("未知工具: {name}")),
    };
    if !enabled {
        return Err(format!("工具 `{name}` 当前已在 Tako Switch 中关闭"));
    }

    let api_key = read_main_api_key(paths)
        .ok_or_else(|| "未检测到 Tako API Key，请先在 Tako Switch 登录或粘贴 cr_ Key".to_string())?;
    let base_url = state.normalized_base_url();

    match name {
        "tako_generate_image" => {
            exec_generate_image(paths, &state.mcp, &base_url, &api_key, args).await
        }
        "tako_generate_video" => {
            exec_generate_video(paths, &state.mcp, &base_url, &api_key, args).await
        }
        "tako_generate_audio" => {
            exec_generate_audio(paths, &state.mcp, &base_url, &api_key, args).await
        }
        "tako_web_search" => {
            exec_web_search(&base_url, &api_key, state.bypass_system_proxy, args).await
        }
        "tako_consult" => {
            exec_consult(&state.mcp, &base_url, &api_key, state.bypass_system_proxy, args).await
        }
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageRequestMode {
    GeminiNative,
    JsonGeneration,
    MultipartEdit,
    JsonEdits,
}

fn collect_reference_images(paths: &AppPaths, args: &Value) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(single) = args
        .get("input_image_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let resolved = expand_user_path(paths, single);
        if seen.insert(resolved.clone()) {
            out.push(resolved);
        }
    }
    if let Some(arr) = args.get("input_image_paths").and_then(Value::as_array) {
        for item in arr {
            if let Some(raw) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                let resolved = expand_user_path(paths, raw);
                if seen.insert(resolved.clone()) {
                    out.push(resolved);
                }
            }
        }
    }
    out
}

fn aspect_ratio_for_gemini(aspect_ratio: Option<&str>, size: Option<&str>) -> Option<&'static str> {
    if let Some(ar) = aspect_ratio.map(str::trim).filter(|s| !s.is_empty()) {
        return match ar {
            "1:1" => Some("1:1"),
            "16:9" => Some("16:9"),
            "9:16" => Some("9:16"),
            "4:3" => Some("4:3"),
            "3:4" => Some("3:4"),
            "3:2" => Some("3:2"),
            "2:3" => Some("2:3"),
            _ => None,
        };
    }
    match size.map(str::trim) {
        Some("1024x1024") => Some("1:1"),
        Some("1536x1024") => Some("3:2"),
        Some("1024x1536") => Some("2:3"),
        Some("1792x1024") => Some("16:9"),
        Some("1024x1792") => Some("9:16"),
        _ => None,
    }
}

fn size_for_openai_image(size: Option<&str>, aspect_ratio: Option<&str>) -> Option<String> {
    if let Some(sz) = size.map(str::trim).filter(|s| !s.is_empty() && *s != "auto") {
        return Some(sz.to_string());
    }
    match aspect_ratio.map(str::trim) {
        Some("16:9") => Some("1792x1024".to_string()),
        Some("9:16") => Some("1024x1792".to_string()),
        Some("3:2" | "4:3") => Some("1536x1024".to_string()),
        Some("2:3" | "3:4") => Some("1024x1536".to_string()),
        Some("1:1") => Some("1024x1024".to_string()),
        _ => None,
    }
}

fn image_request_mode(model: &str, ref_count: usize) -> Result<ImageRequestMode, String> {
    let lower = model.trim().to_ascii_lowercase();
    if lower.starts_with("gemini-") {
        return Ok(ImageRequestMode::GeminiNative);
    }
    if ref_count == 0 {
        return Ok(ImageRequestMode::JsonGeneration);
    }
    if lower.starts_with("grok-") {
        if ref_count > 3 {
            return Err("Grok 图像编辑目前最多支持 3 张参考图".to_string());
        }
        return Ok(ImageRequestMode::JsonEdits);
    }
    if ref_count > 1 {
        return Ok(ImageRequestMode::JsonEdits);
    }
    Ok(ImageRequestMode::MultipartEdit)
}

fn encode_image_data_uri(path: &Path) -> Result<(String, &'static str, Vec<u8>), String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("读取参考图 {} 失败: {e}", path.display()))?;
    let mime = detect_image_format_from_bytes(&bytes)
        .map(|(_, m)| m)
        .unwrap_or_else(|| detect_image_mime(path));
    let data_uri = format!("data:{mime};base64,{}", STANDARD.encode(&bytes));
    Ok((data_uri, mime, bytes))
}

async fn exec_generate_image(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    args: Value,
) -> Result<String, String> {
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少必需参数 prompt".to_string())?;
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("gpt-image-2");
    let aspect_ratio = args.get("aspect_ratio").and_then(Value::as_str);
    let size = args.get("size").and_then(Value::as_str);
    let output_path_arg = args.get("output_path").and_then(Value::as_str);
    let refs = collect_reference_images(paths, &args);
    let mode = image_request_mode(model, refs.len())?;

    let bypass_proxy = read_switch_state(paths).bypass_system_proxy;
    let api_client = build_api_client(240, bypass_proxy)?;

    match mode {
        ImageRequestMode::GeminiNative => {
            let mut parts = Vec::with_capacity(refs.len() + 1);
            for ref_path in &refs {
                let (_uri, mime, bytes) = encode_image_data_uri(ref_path)?;
                parts.push(json!({
                    "inlineData": {
                        "mimeType": mime,
                        "data": STANDARD.encode(&bytes),
                    }
                }));
            }
            parts.push(json!({ "text": prompt }));

            let mut gen_cfg = json!({
                "responseModalities": ["IMAGE", "TEXT"]
            });
            if let Some(ar) = aspect_ratio_for_gemini(aspect_ratio, size) {
                gen_cfg["imageConfig"] = json!({ "aspectRatio": ar });
            }

            let resp = api_client
                .post(format!("{base_url}/v1beta/models/{model}:generateContent"))
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&json!({
                    "contents": [{ "role": "user", "parts": parts }],
                    "generationConfig": gen_cfg,
                }))
                .send()
                .await
                .map_err(|e| format!("生图请求失败（请勿自动重试）: {e}"))?;

            let status = resp.status();
            let body: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析生图响应失败: {e}"))?;
            if !status.is_success() {
                return Err(format!("生图接口返回 {status}（请勿自动重试）: {body}"));
            }

            let (img_bytes, raw_mime, notes) = extract_gemini_image(&body)?;
            let (ext, mime) = detect_image_format_from_bytes(&img_bytes).unwrap_or_else(|| {
                if raw_mime.contains("jpeg") || raw_mime.contains("jpg") {
                    ("jpg", "image/jpeg")
                } else if raw_mime.contains("webp") {
                    ("webp", "image/webp")
                } else {
                    ("png", "image/png")
                }
            });
            let target = resolve_target_file(paths, cfg, output_path_arg, "tako-image", ext);
            write_atomic(&target, &img_bytes, false)?;
            let mut summary = json!({
                "saved_path": target.to_string_lossy(),
                "model": model,
                "mime_type": mime,
                "bytes": img_bytes.len(),
            });
            if let Some(t) = notes {
                summary["note"] = Value::String(t);
            }
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())
        }
        ImageRequestMode::JsonGeneration | ImageRequestMode::MultipartEdit | ImageRequestMode::JsonEdits => {
            let resolved_size = size_for_openai_image(size, aspect_ratio);
            let resp = match mode {
                ImageRequestMode::MultipartEdit => {
                    let ref_path = &refs[0];
                    let bytes = std::fs::read(ref_path)
                        .map_err(|e| format!("读取参考图 {} 失败: {e}", ref_path.display()))?;
                    let mime = detect_image_format_from_bytes(&bytes)
                        .map(|(_, m)| m)
                        .unwrap_or_else(|| detect_image_mime(ref_path));
                    let filename = ref_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("input.png")
                        .to_string();
                    let part = multipart::Part::bytes(bytes)
                        .file_name(filename)
                        .mime_str(mime)
                        .map_err(|e| e.to_string())?;
                    let mut form = multipart::Form::new()
                        .text("model", model.to_string())
                        .text("prompt", prompt.to_string())
                        .text("n", "1".to_string())
                        .text("response_format", "b64_json".to_string())
                        .part("image", part);
                    if let Some(ref sz) = resolved_size {
                        form = form.text("size", sz.clone());
                    }
                    api_client
                        .post(format!("{base_url}/v1/images/edits"))
                        .header("Authorization", format!("Bearer {api_key}"))
                        .multipart(form)
                        .send()
                        .await
                        .map_err(|e| format!("图像编辑请求失败（请勿自动重试）: {e}"))?
                }
                ImageRequestMode::JsonEdits => {
                    let mut images_arr = Vec::with_capacity(refs.len());
                    for ref_path in &refs {
                        let (data_uri, _, _) = encode_image_data_uri(ref_path)?;
                        images_arr.push(json!({ "image_url": data_uri }));
                    }
                    let mut payload = json!({
                        "model": model,
                        "prompt": prompt,
                        "n": 1,
                        "response_format": "b64_json",
                        "images": images_arr,
                    });
                    if let Some(ref sz) = resolved_size {
                        payload["size"] = Value::String(sz.clone());
                    }
                    api_client
                        .post(format!("{base_url}/v1/images/edits"))
                        .header("Authorization", format!("Bearer {api_key}"))
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|e| format!("图像编辑请求失败（请勿自动重试）: {e}"))?
                }
                ImageRequestMode::JsonGeneration => {
                    let mut payload = json!({
                        "model": model,
                        "prompt": prompt,
                        "n": 1,
                        "response_format": "b64_json"
                    });
                    if let Some(ref sz) = resolved_size {
                        payload["size"] = Value::String(sz.clone());
                    }
                    api_client
                        .post(format!("{base_url}/v1/images/generations"))
                        .header("Authorization", format!("Bearer {api_key}"))
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|e| format!("生图请求失败（请勿自动重试）: {e}"))?
                }
                ImageRequestMode::GeminiNative => unreachable!(),
            };

            let status = resp.status();
            let body: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析生图响应失败: {e}"))?;
            if !status.is_success() {
                return Err(format!("生图接口返回 {status}（请勿自动重试）: {body}"));
            }

            let img_bytes = if let Some(b64) = body.pointer("/data/0/b64_json").and_then(Value::as_str)
            {
                STANDARD
                    .decode(b64)
                    .map_err(|e| format!("解码图片 base64 失败: {e}"))?
            } else if let Some(url) = body.pointer("/data/0/url").and_then(Value::as_str) {
                let cdn_client = build_cdn_client(120)?;
                let dl = cdn_client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| format!("下载图片 URL 失败（请勿自动重试）: {e}"))?;
                if !dl.status().is_success() {
                    return Err(format!("图片 CDN 返回 HTTP {}", dl.status()));
                }
                dl.bytes()
                    .await
                    .map_err(|e| format!("读取图片字节失败: {e}"))?
                    .to_vec()
            } else {
                return Err(format!("响应未包含图片数据: {body}"));
            };

            let (ext, mime) = detect_image_format_from_bytes(&img_bytes)
                .ok_or_else(|| "下载内容未通过 PNG/JPEG/WebP 文件头校验，请勿自动重试".to_string())?;
            let target = resolve_target_file(paths, cfg, output_path_arg, "tako-image", ext);
            write_atomic(&target, &img_bytes, false)?;
            serde_json::to_string_pretty(&json!({
                "saved_path": target.to_string_lossy(),
                "model": model,
                "mime_type": mime,
                "bytes": img_bytes.len(),
                "references_used": refs.len(),
            }))
            .map_err(|e| e.to_string())
        }
    }
}

fn extract_gemini_image(body: &Value) -> Result<(Vec<u8>, String, Option<String>), String> {
    let parts = body
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Gemini 响应缺少 candidates[0].content.parts: {body}"))?;

    let mut image_bytes = None;
    let mut mime = "image/png".to_string();
    let mut texts = Vec::new();

    for p in parts {
        let inline = p.get("inlineData").or_else(|| p.get("inline_data"));
        if let Some(obj) = inline {
            if let Some(data) = obj.get("data").and_then(Value::as_str) {
                mime = obj
                    .get("mimeType")
                    .or_else(|| obj.get("mime_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png")
                    .to_string();
                image_bytes = Some(
                    STANDARD
                        .decode(data)
                        .map_err(|e| format!("解码 Gemini 图片失败: {e}"))?,
                );
            }
        }
        if let Some(t) = p.get("text").and_then(Value::as_str) {
            if !t.trim().is_empty() {
                texts.push(t.trim().to_string());
            }
        }
    }

    let bytes = image_bytes.ok_or_else(|| {
        if texts.is_empty() {
            format!("响应未包含图片: {body}")
        } else {
            format!("模型未返回图片，文本输出: {}", texts.join("\n"))
        }
    })?;

    let note = (!texts.is_empty()).then(|| texts.join("\n"));
    Ok((bytes, mime, note))
}

fn normalize_video_model(raw: Option<&str>) -> Result<&'static str, String> {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("grok-imagine-video-1.5")
        || trimmed.eq_ignore_ascii_case("grok-imagine-video")
    {
        Ok("grok-imagine-video-1.5")
    } else {
        Err(format!(
            "不支持的视频模型 `{trimmed}`，目前仅支持 grok-imagine-video-1.5"
        ))
    }
}

fn resolve_video_image_input(paths: &AppPaths, args: &Value) -> Result<Option<String>, String> {
    let raw = args
        .get("input_reference_path")
        .or_else(|| args.get("image"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let Some(val) = raw else {
        return Ok(None);
    };
    if val.starts_with("http://") || val.starts_with("https://") || val.starts_with("data:") {
        return Ok(Some(val.to_string()));
    }
    let path = expand_user_path(paths, val);
    let (data_uri, _, _) = encode_image_data_uri(&path)?;
    Ok(Some(data_uri))
}

fn build_video_submit_payload(
    prompt: &str,
    model: &str,
    duration: u64,
    resolution: &str,
    aspect_ratio: Option<&str>,
    image: Option<&str>,
) -> Value {
    let mut payload = json!({
        "model": model,
        "prompt": prompt,
        "duration": duration,
        "resolution": resolution,
    });
    if let Some(ar) = aspect_ratio.map(str::trim).filter(|s| !s.is_empty()) {
        payload["aspect_ratio"] = Value::String(ar.to_string());
    }
    if let Some(img) = image.filter(|s| !s.trim().is_empty()) {
        payload["image"] = Value::String(img.to_string());
    }
    payload
}

fn extract_video_task_id(body: &Value) -> Option<String> {
    for ptr in [
        "/id",
        "/task_id",
        "/task/id",
        "/task/task_id",
        "/data/id",
        "/data/task_id",
    ] {
        if let Some(s) = body
            .pointer(ptr)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VideoPollOutcome {
    Completed,
    Failed(String),
    Pending,
}

fn classify_video_poll_state(body: &Value) -> (String, u64, VideoPollOutcome) {
    let status = [
        "/status",
        "/state",
        "/task_status",
        "/task/status",
        "/task/state",
        "/task/task_status",
        "/data/status",
        "/data/state",
        "/data/task_status",
    ]
    .into_iter()
    .find_map(|ptr| {
        body.pointer(ptr)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })
    .unwrap_or("unknown")
    .to_string();

    let progress = ["/progress", "/task/progress", "/data/progress"]
        .into_iter()
        .find_map(|ptr| body.pointer(ptr).and_then(Value::as_u64))
        .unwrap_or(0);

    let lower = status.to_ascii_lowercase();
    let outcome = match lower.as_str() {
        "success" | "succeeded" | "done" | "completed" | "finished" | "ok" => {
            VideoPollOutcome::Completed
        }
        "failure" | "failed" | "error" | "cancelled" | "canceled" | "rejected" | "expired" => {
            let reason = [
                "/fail_reason",
                "/error/message",
                "/error",
                "/message",
                "/task/fail_reason",
                "/data/fail_reason",
                "/task/error",
                "/data/error",
            ]
            .into_iter()
            .find_map(|ptr| {
                body.pointer(ptr)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("上游生成失败")
            .to_string();
            VideoPollOutcome::Failed(reason)
        }
        _ => VideoPollOutcome::Pending,
    };
    (status, progress, outcome)
}

async fn exec_generate_video(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    args: Value,
) -> Result<String, String> {
    let model = normalize_video_model(args.get("model").and_then(Value::as_str))?;
    let wait_secs = args
        .get("wait_timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(120)
        .clamp(5, 300);
    let output_path_arg = args.get("output_path").and_then(Value::as_str);

    let bypass_proxy = read_switch_state(paths).bypass_system_proxy;
    let api_client = build_api_client(90, bypass_proxy)?;

    let task_id = if let Some(existing) = args
        .get("resume_task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        existing.to_string()
    } else {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "缺少 prompt 或 resume_task_id".to_string())?;
        let duration = args
            .get("duration")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 15);
        let resolution = args
            .get("resolution")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("480p");
        let aspect_ratio = args.get("aspect_ratio").and_then(Value::as_str);
        let image_ref = resolve_video_image_input(paths, &args)?;
        let payload = build_video_submit_payload(
            prompt,
            model,
            duration,
            resolution,
            aspect_ratio,
            image_ref.as_deref(),
        );

        let submit_resp = api_client
            .post(format!("{base_url}/v1/videos/generations"))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("提交视频任务失败（请勿自动重试）: {e}"))?;

        let status = submit_resp.status();
        let body: Value = submit_resp
            .json()
            .await
            .map_err(|e| format!("解析视频提交响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("视频任务提交失败 ({status}，请勿自动重试): {body}"));
        }
        extract_video_task_id(&body).ok_or_else(|| format!("响应缺少任务 id: {body}"))?
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(wait_secs);
    let mut last_status = "queued".to_string();
    let mut last_progress = 0u64;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return serde_json::to_string_pretty(&json!({
                "status": last_status,
                "progress": last_progress,
                "task_id": task_id,
                "message": "轮询等待超时，视频仍在生成中。请勿重复付费提交新任务，稍后传入 resume_task_id 继续下载。"
            }))
            .map_err(|e| e.to_string());
        }

        let poll_resp = api_client
            .get(format!("{base_url}/v1/videos/{task_id}"))
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| {
                format!("轮询视频任务 {task_id} 失败（请传入 resume_task_id 恢复，切勿重新提交）: {e}")
            })?;
        let pstatus = poll_resp.status();
        let pbody: Value = poll_resp
            .json()
            .await
            .map_err(|e| format!("解析视频状态失败 (task_id={task_id}): {e}"))?;
        if !pstatus.is_success() {
            return Err(format!(
                "查询视频任务 {task_id} 返回 {pstatus}（请勿自动重复提交）: {pbody}"
            ));
        }

        let (st, prog, outcome) = classify_video_poll_state(&pbody);
        last_status = st;
        last_progress = prog;

        match outcome {
            VideoPollOutcome::Completed => break,
            VideoPollOutcome::Failed(reason) => {
                return Err(format!(
                    "视频任务 {task_id} 失败（请勿自动重复提交）: {reason}"
                ));
            }
            VideoPollOutcome::Pending => {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }

    let dl_resp = api_client
        .get(format!("{base_url}/v1/videos/{task_id}/content"))
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| {
            format!("下载视频 {task_id} 失败（请传 resume_task_id 恢复下载，切勿重新生成）: {e}")
        })?;
    if !dl_resp.status().is_success() {
        let code = dl_resp.status();
        let err_txt = dl_resp.text().await.unwrap_or_default();
        return Err(format!(
            "下载视频 {task_id} 返回 {code}（请传 resume_task_id 恢复）: {err_txt}"
        ));
    }

    let content_type = dl_resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let raw_bytes = dl_resp
        .bytes()
        .await
        .map_err(|e| format!("读取视频内容失败 (task_id={task_id}): {e}"))?
        .to_vec();

    let mp4_bytes = if content_type.contains("json") || raw_bytes.first() == Some(&b'{') {
        let v: Value = serde_json::from_slice(&raw_bytes)
            .map_err(|e| format!("解析视频下载 JSON 失败 (task_id={task_id}): {e}"))?;
        if let Some(b64) = ["/b64_json", "/video_base64", "/data/b64_json", "/data/0/b64_json"]
            .into_iter()
            .find_map(|p| v.pointer(p).and_then(Value::as_str))
        {
            STANDARD
                .decode(b64)
                .map_err(|e| format!("解码视频 base64 失败 (task_id={task_id}): {e}"))?
        } else if let Some(url) = ["/url", "/video_url", "/data/url", "/data/0/url"]
            .into_iter()
            .find_map(|p| v.pointer(p).and_then(Value::as_str))
        {
            let cdn_client = build_cdn_client(180)?;
            let cdn_resp = cdn_client
                .get(url)
                .send()
                .await
                .map_err(|e| format!("从视频 CDN 下载失败 (task_id={task_id}): {e}"))?;
            if !cdn_resp.status().is_success() {
                return Err(format!(
                    "视频 CDN 返回 {}（可传 resume_task_id 重试下载）",
                    cdn_resp.status()
                ));
            }
            cdn_resp
                .bytes()
                .await
                .map_err(|e| format!("读取视频 CDN 流失败 (task_id={task_id}): {e}"))?
                .to_vec()
        } else {
            return Err(format!(
                "视频下载返回 JSON 但缺少 MP4 url/b64_json (task_id={task_id}): {v}"
            ));
        }
    } else {
        raw_bytes
    };

    if !is_valid_mp4(&mp4_bytes) {
        return Err(format!(
            "下载的视频未通过 MP4 (ftyp) 文件头校验（task_id={task_id}，bytes={}，请传 resume_task_id 排查，切勿盲目重试提交）",
            mp4_bytes.len()
        ));
    }

    let target = resolve_target_file(paths, cfg, output_path_arg, "tako-video", "mp4");
    write_atomic(&target, &mp4_bytes, false)?;

    serde_json::to_string_pretty(&json!({
        "status": "completed",
        "task_id": task_id,
        "model": model,
        "saved_path": target.to_string_lossy(),
        "bytes": mp4_bytes.len()
    }))
    .map_err(|e| e.to_string())
}

fn normalize_audio_format(raw: Option<&str>) -> Result<&'static str, String> {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return Ok("wav");
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "wav" | "wave" => Ok("wav"),
        "mp3" => Ok("mp3"),
        "pcm" | "pcm16" => Ok("pcm"),
        other => Err(format!(
            "不支持的音频格式 `{other}`，Tako Speech API 仅支持 wav、mp3、pcm"
        )),
    }
}

fn build_audio_request_payload(
    paths: &AppPaths,
    args: &Value,
) -> Result<(Value, &'static str, String, Option<String>), String> {
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("mimo-v2.5-tts")
        .to_string();
    let format = normalize_audio_format(args.get("response_format").and_then(Value::as_str))?;
    let instructions = args
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let input = args
        .get("input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let is_voicedesign = model == "mimo-v2.5-tts-voicedesign";
    let is_voiceclone = model == "mimo-v2.5-tts-voiceclone";

    if input.is_none() && !(is_voicedesign && instructions.is_some()) {
        return Err("缺少必需参数 input（仅 mimo-v2.5-tts-voicedesign 提供 instructions 时可省略）".to_string());
    }

    let mut payload = json!({
        "model": model,
        "response_format": format,
    });
    if let Some(ref text) = input {
        payload["input"] = Value::String(text.clone());
    }
    if let Some(ref inst) = instructions {
        payload["instructions"] = Value::String(inst.clone());
    }

    let voice_sample_path = args
        .get("voice_sample_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let explicit_voice = args
        .get("voice")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let voice_used = if is_voicedesign {
        None
    } else if is_voiceclone {
        let data_uri = if let Some(sample_raw) = voice_sample_path {
            let sample_path = expand_user_path(paths, sample_raw);
            let bytes = std::fs::read(&sample_path)
                .map_err(|e| format!("读取声音克隆样本 {} 失败: {e}", sample_path.display()))?;
            if bytes.len() > MAX_VOICE_SAMPLE_BYTES {
                return Err(format!(
                    "声音克隆样本超过 10MB 上限（当前 {} 字节）",
                    bytes.len()
                ));
            }
            let mime = detect_audio_mime(&sample_path);
            format!("data:{mime};base64,{}", STANDARD.encode(&bytes))
        } else if let Some(v) = explicit_voice.filter(|v| v.starts_with("data:")) {
            v.to_string()
        } else {
            return Err("mimo-v2.5-tts-voiceclone 必须传入 voice_sample_path（或 data: URI 格式的 voice）".to_string());
        };
        payload["voice"] = Value::String(data_uri);
        Some("<voice_sample>".to_string())
    } else {
        if let Some(v) = explicit_voice {
            if v.starts_with("data:") {
                return Err("预设语音模型不支持 data: URI 作为 voice；请改用 mimo-v2.5-tts-voiceclone".to_string());
            }
        }
        let chosen = explicit_voice.unwrap_or("冰糖").to_string();
        payload["voice"] = Value::String(chosen.clone());
        Some(chosen)
    };

    Ok((payload, format, model, voice_used))
}

async fn exec_generate_audio(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    args: Value,
) -> Result<String, String> {
    let output_path_arg = args.get("output_path").and_then(Value::as_str);
    let (payload, ext, model, voice_used) = build_audio_request_payload(paths, &args)?;

    let bypass_proxy = read_switch_state(paths).bypass_system_proxy;
    let client = build_api_client(120, bypass_proxy)?;
    let resp = client
        .post(format!("{base_url}/v1/audio/speech"))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("语音合成请求失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let err_txt = resp.text().await.unwrap_or_default();
        return Err(format!("语音合成返回 {status}: {err_txt}"));
    }
    let preview = resp
        .headers()
        .get("x-speech-final-text-preview")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let audio_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取音频字节失败: {e}"))?;

    let target = resolve_target_file(paths, cfg, output_path_arg, "tako-audio", ext);
    write_atomic(&target, &audio_bytes, false)?;

    let mut summary = json!({
        "saved_path": target.to_string_lossy(),
        "model": model,
        "format": ext,
        "bytes": audio_bytes.len()
    });
    if let Some(v) = voice_used {
        summary["voice"] = Value::String(v);
    }
    if let Some(p) = preview {
        summary["final_text_preview"] = Value::String(p);
    }
    serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())
}

async fn exec_web_search(
    base_url: &str,
    api_key: &str,
    bypass_system_proxy: bool,
    args: Value,
) -> Result<String, String> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少必需参数 query".to_string())?;
    let count = args.get("count").and_then(Value::as_u64).unwrap_or(5).clamp(1, 20);
    let mut payload = json!({
        "query": query,
        "count": count
    });
    if let Some(provider) = args
        .get("provider")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        payload["provider"] = Value::String(provider.to_string());
    }

    let client = build_api_client(60, bypass_system_proxy)?;
    let resp = client
        .post(format!("{base_url}/v1/search"))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("联网搜索请求失败: {e}"))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析搜索结果失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("搜索接口返回 {status}: {body}"));
    }

    serde_json::to_string_pretty(&body).map_err(|e| e.to_string())
}

async fn exec_consult(
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    bypass_system_proxy: bool,
    args: Value,
) -> Result<String, String> {
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少必需参数 question".to_string())?;
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&cfg.consult_model);
    let sys_prompt = args
        .get("system_prompt")
        .and_then(Value::as_str)
        .unwrap_or("You are a rigorous technical & strategic second-opinion advisor. Give concise, candid analysis pointing out blind spots, edge cases, and better alternatives.");

    let user_msg = match args.get("context").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        Some(ctx) => format!("{question}\n\n<context>\n{ctx}\n</context>"),
        None => question.to_string(),
    };

    let client = build_api_client(120, bypass_system_proxy)?;
    let resp = client
        .post(format!("{base_url}/v1/chat/completions"))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&json!({
            "model": model,
            "messages": [
                { "role": "system", "content": sys_prompt },
                { "role": "user", "content": user_msg }
            ]
        }))
        .send()
        .await
        .map_err(|e| format!("多模型参谋请求失败: {e}"))?;

    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析参谋响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("参谋接口返回 {status}: {body}"));
    }

    let reply = body
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("参谋响应缺少 choices[0].message.content: {body}"))?;
    Ok(format!("[{model}]\n{reply}"))
}

async fn exec_account_status(paths: &AppPaths) -> Result<String, String> {
    let status = crate::auth::fetch_account_status(paths).await;
    let payload = json!({
        "logged_in": status.logged_in,
        "user": status.user_name,
        "plan": status.plan_name,
        "wallet_remain_usd": status.wallet_remain_usd,
        "wallet_used_usd": status.wallet_used_usd,
        "wallet_remain_display": status.wallet_remain_usd.map(crate::auth::format_usd),
        "wallet_used_display": status.wallet_used_usd.map(crate::auth::format_usd),
        "quota": status.quota,
        "base_url": status.base_url,
        "error": status.error,
    });
    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveDemoRequest {
    pub tool: String, // "image" | "video" | "audio" | "search" | "consult"
    pub prompt: String,
    pub model: Option<String>,
    pub voice: Option<String>,
    pub instructions: Option<String>,
    pub aspect_ratio: Option<String>,
    pub provider: Option<String>,
    pub resume_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LiveDemoCitation {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub domain: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveDemoResult {
    pub tool: String,
    pub status: String, // "completed" | "pending"
    pub model: String,
    pub elapsed_ms: u64,
    pub saved_path: Option<String>,
    pub preview_data_url: Option<String>,
    pub text_output: Option<String>,
    pub task_id: Option<String>,
    pub citations: Vec<LiveDemoCitation>,
}

fn read_base64_preview(saved_path: Option<&str>, mime: &str) -> Option<String> {
    let path_str = saved_path?;
    let path = Path::new(path_str);
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_LIVE_DEMO_PREVIEW_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", STANDARD.encode(&bytes)))
}

fn extract_domain_from_url(raw_url: &str) -> String {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| trimmed.strip_prefix("//").unwrap_or(trimmed));
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };
    host.trim().to_ascii_lowercase()
}

fn parse_live_demo_search_result(
    body: &Value,
    provider: Option<String>,
    elapsed_ms: u64,
) -> LiveDemoResult {
    let text_output = ["/answer", "/summary", "/content", "/data/answer"]
        .into_iter()
        .find_map(|ptr| {
            body.pointer(ptr)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });

    let items = ["/results", "/data/results", "/data", "/organic"]
        .into_iter()
        .find_map(|ptr| body.pointer(ptr).and_then(Value::as_array));

    let mut citations = Vec::new();
    if let Some(arr) = items {
        for item in arr {
            let url = item
                .get("url")
                .or_else(|| item.get("link"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if url.is_empty() {
                continue;
            }
            let title = item
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_string();
            let snippet = ["snippet", "content", "description"]
                .into_iter()
                .find_map(|k| {
                    item.get(k)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("")
                .to_string();
            let domain = extract_domain_from_url(url);
            citations.push(LiveDemoCitation {
                title,
                url: url.to_string(),
                snippet,
                domain,
            });
        }
    }

    let model = provider
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "deepseek".to_string());

    LiveDemoResult {
        tool: "search".to_string(),
        status: "completed".to_string(),
        model,
        elapsed_ms,
        saved_path: None,
        preview_data_url: None,
        text_output,
        task_id: None,
        citations,
    }
}

fn strip_consult_model_prefix(raw: String, fallback_model: String) -> (String, String) {
    if let Some(rest) = raw.strip_prefix('[') {
        if let Some((m, body)) = rest.split_once("]\n") {
            let trimmed_model = m.trim();
            if !trimmed_model.is_empty() {
                return (trimmed_model.to_string(), body.to_string());
            }
        }
    }
    (fallback_model, raw)
}

pub async fn execute_live_demo_for_paths(
    paths: &AppPaths,
    input: LiveDemoRequest,
) -> Result<LiveDemoResult, String> {
    let api_key = read_main_api_key(paths).ok_or_else(|| {
        "未登录 Tako 账号，请先在右上角登录或填入 API Key 后再现场试跑".to_string()
    })?;
    let state = read_switch_state(paths);
    let base_url = state.normalized_base_url();
    let started = Instant::now();

    match input.tool.trim() {
        "image" => {
            let mut args = json!({ "prompt": input.prompt });
            if let Some(m) = input
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                args["model"] = Value::String(m.to_string());
            }
            if let Some(ar) = input
                .aspect_ratio
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                args["aspect_ratio"] = Value::String(ar.to_string());
            }

            let raw = exec_generate_image(paths, &state.mcp, &base_url, &api_key, args).await?;
            let v: Value =
                serde_json::from_str(&raw).map_err(|e| format!("解析生图结果失败: {e}"))?;
            let saved_path = v
                .get("saved_path")
                .and_then(Value::as_str)
                .map(str::to_string);
            let model = v
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    input
                        .model
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "gpt-image-2".to_string())
                });
            let note = v.get("note").and_then(Value::as_str).map(str::to_string);

            let preview_data_url = match saved_path.as_deref() {
                Some(p) => {
                    let path = Path::new(p);
                    match std::fs::metadata(path) {
                        Ok(meta) if meta.is_file() && meta.len() <= MAX_LIVE_DEMO_PREVIEW_BYTES => {
                            encode_image_data_uri(path).ok().map(|(uri, _, _)| uri)
                        }
                        _ => None,
                    }
                }
                None => None,
            };

            Ok(LiveDemoResult {
                tool: "image".to_string(),
                status: "completed".to_string(),
                model,
                elapsed_ms: started.elapsed().as_millis() as u64,
                saved_path,
                preview_data_url,
                text_output: note,
                task_id: None,
                citations: Vec::new(),
            })
        }
        "video" => {
            let mut args = json!({
                "prompt": input.prompt,
                "duration": 5,
                "resolution": "480p",
                "aspect_ratio": input
                    .aspect_ratio
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("1:1"),
                "wait_timeout_secs": 90,
            });
            if let Some(m) = input
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                args["model"] = Value::String(m.to_string());
            }
            if let Some(tid) = input
                .resume_task_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                args["resume_task_id"] = Value::String(tid.to_string());
            }

            let raw = exec_generate_video(paths, &state.mcp, &base_url, &api_key, args).await?;
            let v: Value =
                serde_json::from_str(&raw).map_err(|e| format!("解析视频结果失败: {e}"))?;
            let raw_status = v.get("status").and_then(Value::as_str).unwrap_or("");
            let task_id = v.get("task_id").and_then(Value::as_str).map(str::to_string);
            let model = v
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    input
                        .model
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "grok-imagine-video-1.5".to_string())
                });

            if raw_status == "completed" {
                let saved_path = v
                    .get("saved_path")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let preview_data_url = read_base64_preview(saved_path.as_deref(), "video/mp4");
                Ok(LiveDemoResult {
                    tool: "video".to_string(),
                    status: "completed".to_string(),
                    model,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    saved_path,
                    preview_data_url,
                    text_output: None,
                    task_id,
                    citations: Vec::new(),
                })
            } else {
                let message = v.get("message").and_then(Value::as_str).map(str::to_string);
                Ok(LiveDemoResult {
                    tool: "video".to_string(),
                    status: "pending".to_string(),
                    model,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    saved_path: None,
                    preview_data_url: None,
                    text_output: message,
                    task_id,
                    citations: Vec::new(),
                })
            }
        }
        "audio" => {
            let mut args = json!({
                "input": input.prompt,
                "voice": input
                    .voice
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("冰糖"),
                "model": input
                    .model
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("mimo-v2.5-tts"),
                "response_format": "mp3",
            });
            if let Some(inst) = input
                .instructions
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                args["instructions"] = Value::String(inst.to_string());
            }

            let raw = exec_generate_audio(paths, &state.mcp, &base_url, &api_key, args).await?;
            let v: Value =
                serde_json::from_str(&raw).map_err(|e| format!("解析音频结果失败: {e}"))?;
            let saved_path = v
                .get("saved_path")
                .and_then(Value::as_str)
                .map(str::to_string);
            let model = v
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    input
                        .model
                        .clone()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| "mimo-v2.5-tts".to_string())
                });
            let preview_data_url = read_base64_preview(saved_path.as_deref(), "audio/mpeg");
            let text_output = v
                .get("final_text_preview")
                .and_then(Value::as_str)
                .map(str::to_string);

            Ok(LiveDemoResult {
                tool: "audio".to_string(),
                status: "completed".to_string(),
                model,
                elapsed_ms: started.elapsed().as_millis() as u64,
                saved_path,
                preview_data_url,
                text_output,
                task_id: None,
                citations: Vec::new(),
            })
        }
        "search" => {
            let provider = input.provider.clone().filter(|s| !s.trim().is_empty());
            let raw = exec_web_search(
                &base_url,
                &api_key,
                state.bypass_system_proxy,
                json!({
                    "query": input.prompt,
                    "count": 4,
                    "provider": provider,
                }),
            )
            .await?;
            let body: Value =
                serde_json::from_str(&raw).map_err(|e| format!("解析搜索结果失败: {e}"))?;
            Ok(parse_live_demo_search_result(
                &body,
                provider,
                started.elapsed().as_millis() as u64,
            ))
        }
        "consult" => {
            let fallback_model = input
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&state.mcp.consult_model)
                .to_string();
            let raw = exec_consult(
                &state.mcp,
                &base_url,
                &api_key,
                state.bypass_system_proxy,
                json!({
                    "question": input.prompt,
                    "model": input.model,
                }),
            )
            .await?;
            let (model, text) = strip_consult_model_prefix(raw, fallback_model);

            Ok(LiveDemoResult {
                tool: "consult".to_string(),
                status: "completed".to_string(),
                model,
                elapsed_ms: started.elapsed().as_millis() as u64,
                saved_path: None,
                preview_data_url: None,
                text_output: Some(text),
                task_id: None,
                citations: Vec::new(),
            })
        }
        _ => Err(format!("不支持的现场试跑工具: {}", input.tool)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_switch_state;
    use tempfile::TempDir;

    #[tokio::test]
    async fn mcp_tools_list_reflects_toggles_and_tako_skill_schemas() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let init_resp = handle_jsonrpc_message(
            &paths,
            json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}),
        )
        .await
        .unwrap();
        let instr = init_resp["result"]["instructions"].as_str().unwrap_or("");
        assert!(instr.contains("tako_generate_image"));
        assert!(instr.contains("gpt-image-2"));
        assert!(instr.contains("resume_task_id"));
        assert!(instr.contains("tako_account_status"));

        let resp_all = handle_jsonrpc_message(
            &paths,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await
        .unwrap();
        let tools = resp_all["result"]["tools"].as_array().unwrap();
        let names_all: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names_all.len(), 6);

        let img_tool = tools.iter().find(|t| t["name"] == "tako_generate_image").unwrap();
        assert!(img_tool["description"].as_str().unwrap().contains("gpt-image-2"));
        assert!(img_tool["inputSchema"]["properties"].get("input_image_paths").is_some());
        assert!(img_tool["inputSchema"]["properties"].get("size").is_some());

        let vid_tool = tools.iter().find(|t| t["name"] == "tako_generate_video").unwrap();
        assert!(vid_tool["inputSchema"]["properties"]["resolution"]["description"]
            .as_str()
            .unwrap()
            .contains("480p"));

        let audio_tool = tools.iter().find(|t| t["name"] == "tako_generate_audio").unwrap();
        let audio_formats: Vec<&str> = audio_tool["inputSchema"]["properties"]["response_format"]
            ["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(audio_formats, vec!["wav", "mp3", "pcm"]);
        assert!(audio_tool["inputSchema"]["properties"].get("speed").is_none());
        assert!(audio_tool["inputSchema"]["properties"].get("instructions").is_some());
        assert!(audio_tool["inputSchema"]["properties"].get("voice_sample_path").is_some());

        let search_tool = tools.iter().find(|t| t["name"] == "tako_web_search").unwrap();
        let providers: Vec<&str> = search_tool["inputSchema"]["properties"]["provider"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(providers, vec!["deepseek", "groq", "grok", "grok_x"]);

        let mut st = read_switch_state(&paths);
        st.mcp.video = false;
        st.mcp.consult = false;
        save_switch_state(&paths, &st).unwrap();

        let resp_filtered = handle_jsonrpc_message(
            &paths,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .await
        .unwrap();
        let names_filtered: Vec<&str> = resp_filtered["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names_filtered,
            vec![
                "tako_generate_image",
                "tako_generate_audio",
                "tako_web_search",
                "tako_account_status"
            ]
        );
    }

    #[test]
    fn mcp_tools_call_normalizes_stringified_arguments() {
        let single = Value::String(r#"{"prompt":"cat","aspect_ratio":"3:4"}"#.to_string());
        assert_eq!(
            normalize_tool_arguments(Some(&single)),
            json!({"prompt": "cat", "aspect_ratio": "3:4"})
        );

        let double_encoded = Value::String(
            serde_json::to_string(&r#"{"prompt":"dog","model":"gpt-image-2"}"#).unwrap(),
        );
        assert_eq!(
            normalize_tool_arguments(Some(&double_encoded)),
            json!({"prompt": "dog", "model": "gpt-image-2"})
        );

        let wrapped = json!({"Arguments": r#"{"query":"hello","count":3}"#});
        assert_eq!(
            normalize_tool_arguments(Some(&wrapped)),
            json!({"query": "hello", "count": 3})
        );

        let plain = json!({"query": "plain"});
        assert_eq!(normalize_tool_arguments(Some(&plain)), plain);

        let invalid = Value::String("not-json".to_string());
        assert_eq!(normalize_tool_arguments(Some(&invalid)), json!({}));
    }

    #[test]
    fn video_payload_task_id_and_status_parsing_match_tako_skill() {
        assert_eq!(normalize_video_model(None).unwrap(), "grok-imagine-video-1.5");
        assert_eq!(
            normalize_video_model(Some("grok-imagine-video")).unwrap(),
            "grok-imagine-video-1.5"
        );
        assert!(normalize_video_model(Some("sora-2")).is_err());

        let payload = build_video_submit_payload(
            "a red apple spinning",
            "grok-imagine-video-1.5",
            5,
            "480p",
            Some("16:9"),
            Some("data:image/png;base64,AAAA"),
        );
        assert_eq!(payload["model"], json!("grok-imagine-video-1.5"));
        assert_eq!(payload["resolution"], json!("480p"));
        assert_eq!(payload["duration"], json!(5));
        assert_eq!(payload["image"], json!("data:image/png;base64,AAAA"));

        assert_eq!(extract_video_task_id(&json!({"id": "vid_1"})), Some("vid_1".to_string()));
        assert_eq!(
            extract_video_task_id(&json!({"data": {"task_id": "vid_2"}})),
            Some("vid_2".to_string())
        );

        assert_eq!(
            classify_video_poll_state(&json!({"status": "SUCCESS", "progress": 100})).2,
            VideoPollOutcome::Completed
        );
        assert_eq!(
            classify_video_poll_state(&json!({"state": "done"})).2,
            VideoPollOutcome::Completed
        );
        assert_eq!(
            classify_video_poll_state(&json!({"data": {"task_status": "FAILURE", "fail_reason": "nsfw"}})).2,
            VideoPollOutcome::Failed("nsfw".to_string())
        );
        assert_eq!(
            classify_video_poll_state(&json!({"status": "IN_PROGRESS"})).2,
            VideoPollOutcome::Pending
        );

        assert!(is_valid_mp4(b"\x00\x00\x00\x20ftypisom\x00\x00"));
        assert!(!is_valid_mp4(b"{\"error\":\"nope\"}"));
    }

    #[test]
    fn image_routing_magic_bytes_and_size_mapping_match_tako_skill() {
        assert_eq!(
            image_request_mode("gpt-image-2", 0).unwrap(),
            ImageRequestMode::JsonGeneration
        );
        assert_eq!(
            image_request_mode("gpt-image-2", 1).unwrap(),
            ImageRequestMode::MultipartEdit
        );
        assert_eq!(
            image_request_mode("gpt-image-2", 2).unwrap(),
            ImageRequestMode::JsonEdits
        );
        assert_eq!(
            image_request_mode("grok-imagine-image-quality", 1).unwrap(),
            ImageRequestMode::JsonEdits
        );
        assert!(image_request_mode("grok-imagine-image-quality", 4).is_err());
        assert_eq!(
            image_request_mode("gemini-3.1-flash-image", 2).unwrap(),
            ImageRequestMode::GeminiNative
        );

        assert_eq!(
            size_for_openai_image(None, Some("16:9")),
            Some("1792x1024".to_string())
        );
        assert_eq!(
            aspect_ratio_for_gemini(None, Some("1536x1024")),
            Some("3:2")
        );
        assert_eq!(
            detect_image_format_from_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(("png", "image/png"))
        );
        assert_eq!(
            detect_image_format_from_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(("jpg", "image/jpeg"))
        );
        assert_eq!(
            detect_image_format_from_bytes(b"RIFF\x10\x00\x00\x00WEBP"),
            Some(("webp", "image/webp"))
        );
        assert_eq!(detect_image_format_from_bytes(b"<html>err</html>"), None);
    }

    #[test]
    fn audio_payload_builder_enforces_speech_api_contract() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        // Rejects opus
        assert!(build_audio_request_payload(
            &paths,
            &json!({"input": "hello", "response_format": "opus"})
        )
        .is_err());

        // Standard TTS defaults to mimo-v2.5-tts + 冰糖 + wav
        let (p, ext, model, voice) =
            build_audio_request_payload(&paths, &json!({"input": "你好"})).unwrap();
        assert_eq!(ext, "wav");
        assert_eq!(model, "mimo-v2.5-tts");
        assert_eq!(voice.as_deref(), Some("冰糖"));
        assert_eq!(p["voice"], json!("冰糖"));

        // Voicedesign omits voice and allows instructions-only
        let (pd, _, _, vd_voice) = build_audio_request_payload(
            &paths,
            &json!({"model": "mimo-v2.5-tts-voicedesign", "instructions": "沉稳男声", "voice": "ignored"}),
        )
        .unwrap();
        assert!(vd_voice.is_none());
        assert!(pd.get("voice").is_none());
        assert_eq!(pd["instructions"], json!("沉稳男声"));

        // Voiceclone encodes local audio file into voice data URI
        let sample = tmp.path().join("sample.wav");
        std::fs::write(&sample, b"RIFFdemoWAVE").unwrap();
        let (pc, _, _, _) = build_audio_request_payload(
            &paths,
            &json!({
                "model": "mimo-v2.5-tts-voiceclone",
                "input": "测试克隆",
                "voice_sample_path": sample.to_string_lossy()
            }),
        )
        .unwrap();
        assert!(pc["voice"].as_str().unwrap().starts_with("data:audio/wav;base64,"));
    }

    #[tokio::test]
    async fn live_demo_requires_login_and_rejects_unknown_tool() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let err_unauth = execute_live_demo_for_paths(
            &paths,
            LiveDemoRequest {
                tool: "search".to_string(),
                prompt: "tauri v2".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            err_unauth,
            "未登录 Tako 账号，请先在右上角登录或填入 API Key 后再现场试跑"
        );

        crate::config::save_switch_auth(&paths, Some("cr_test_key"), None, None).unwrap();
        let err_unknown = execute_live_demo_for_paths(
            &paths,
            LiveDemoRequest {
                tool: "magic".to_string(),
                prompt: "test".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(err_unknown, "不支持的现场试跑工具: magic");
    }

    #[test]
    fn live_demo_search_extracts_citations_summary_and_domains() {
        assert_eq!(
            extract_domain_from_url("https://v2.tauri.app/start/create-project/?q=1#top"),
            "v2.tauri.app"
        );
        assert_eq!(
            extract_domain_from_url("http://user:pass@Docs.Example.org:8443/path"),
            "docs.example.org"
        );
        assert_eq!(extract_domain_from_url("https://[::1]:8080/api"), "::1");

        let body = json!({
            "data": {
                "answer": "Tauri 2.0 supports desktop and mobile.",
                "results": [
                    {
                        "title": "Tauri 2.0 Docs",
                        "url": "https://v2.tauri.app/start/",
                        "description": "Official Tauri v2 documentation"
                    },
                    {
                        "title": "Empty URL should be filtered",
                        "url": "   ",
                        "snippet": "ignored"
                    },
                    {
                        "title": "GitHub Repo",
                        "link": "https://github.com/tauri-apps/tauri",
                        "content": "Build smaller, faster desktop apps"
                    }
                ]
            }
        });
        let res = parse_live_demo_search_result(&body, None, 42);
        assert_eq!(res.tool, "search");
        assert_eq!(res.status, "completed");
        assert_eq!(res.model, "deepseek");
        assert_eq!(res.elapsed_ms, 42);
        assert_eq!(
            res.text_output.as_deref(),
            Some("Tauri 2.0 supports desktop and mobile.")
        );
        assert_eq!(
            res.citations,
            vec![
                LiveDemoCitation {
                    title: "Tauri 2.0 Docs".to_string(),
                    url: "https://v2.tauri.app/start/".to_string(),
                    snippet: "Official Tauri v2 documentation".to_string(),
                    domain: "v2.tauri.app".to_string(),
                },
                LiveDemoCitation {
                    title: "GitHub Repo".to_string(),
                    url: "https://github.com/tauri-apps/tauri".to_string(),
                    snippet: "Build smaller, faster desktop apps".to_string(),
                    domain: "github.com".to_string(),
                }
            ]
        );

        let organic_body = json!({
            "organic": [
                {
                    "title": "Rust Book",
                    "url": "https://doc.rust-lang.org/book/",
                    "snippet": "The Rust Programming Language"
                }
            ]
        });
        let res2 = parse_live_demo_search_result(&organic_body, Some("grok".to_string()), 15);
        assert_eq!(res2.model, "grok");
        assert!(res2.text_output.is_none());
        assert_eq!(res2.citations.len(), 1);
        assert_eq!(res2.citations[0].domain, "doc.rust-lang.org");
    }

    #[tokio::test]
    async fn live_demo_bypasses_mcp_tool_toggles_and_strips_consult_prefix() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.contains("POST /v1/search ") {
                    r#"{"summary":"Search summary","results":[{"title":"Example","url":"https://example.com/a","snippet":"hello"}]}"#
                } else {
                    r#"{"choices":[{"message":{"content":"Second opinion body"}}]}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        crate::config::save_switch_auth(&paths, Some("cr_test_key"), None, None).unwrap();
        let mut st = read_switch_state(&paths);
        st.base_url = format!("http://{addr}");
        st.mcp.search = false;
        st.mcp.consult = false;
        save_switch_state(&paths, &st).unwrap();

        let search_res = execute_live_demo_for_paths(
            &paths,
            LiveDemoRequest {
                tool: "search".to_string(),
                prompt: "hello".to_string(),
                provider: Some("grok_x".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(search_res.status, "completed");
        assert_eq!(search_res.model, "grok_x");
        assert_eq!(search_res.text_output.as_deref(), Some("Search summary"));
        assert_eq!(search_res.citations.len(), 1);
        assert_eq!(search_res.citations[0].domain, "example.com");

        let consult_res = execute_live_demo_for_paths(
            &paths,
            LiveDemoRequest {
                tool: "consult".to_string(),
                prompt: "review this".to_string(),
                model: Some("claude-opus-5-5".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(consult_res.status, "completed");
        assert_eq!(consult_res.model, "claude-opus-5-5");
        assert_eq!(consult_res.text_output.as_deref(), Some("Second opinion body"));

        server.await.unwrap();
    }
}
