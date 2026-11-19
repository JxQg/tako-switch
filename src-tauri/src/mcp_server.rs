use crate::config::{
    read_main_api_key, read_switch_state, resolve_output_dir, write_atomic, AppPaths,
    McpToolsConfig,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::multipart;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn build_client(timeout_secs: u64) -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))
}

fn timestamp_slug() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

fn detect_mime(path: &Path) -> &'static str {
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

fn resolve_target_file(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    explicit_path: Option<&str>,
    prefix: &str,
    ext: &str,
) -> PathBuf {
    if let Some(p) = explicit_path.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(rest) = p.strip_prefix("~/") {
            return paths.home.join(rest);
        }
        return PathBuf::from(p);
    }
    let dir = resolve_output_dir(paths, &cfg.output_dir);
    dir.join(format!("{prefix}-{}.{ext}", timestamp_slug()))
}

pub fn build_tools_catalog(cfg: &McpToolsConfig) -> Vec<Value> {
    let mut tools = Vec::new();
    if cfg.image {
        tools.push(json!({
            "name": "tako_generate_image",
            "description": "Generate or edit an image via Tako and save the file directly to local disk (avoids context bloat). Supports Gemini image models (default: gemini-3.1-flash-image, gemini-3-pro-image) and OpenAI/Grok image models (gpt-image-2, grok-imagine-image-quality).",
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
                    "model": { "type": "string", "description": "Optional image model ID (default: gemini-3.1-flash-image)." },
                    "aspect_ratio": { "type": "string", "enum": ["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"] },
                    "input_image_path": { "type": "string", "description": "Optional local file path of an input reference image to edit." },
                    "output_path": { "type": "string", "description": "Optional local output file path (defaults to ~/Downloads/Tako/tako-image-<ts>.<ext>)." }
                },
                "required": ["prompt"]
            }
        }));
    }
    if cfg.video {
        tools.push(json!({
            "name": "tako_generate_video",
            "description": "Generate a video using Tako Grok Imagine Video (async 3-step: submit -> poll -> download MP4 to local disk). Video generation is billed by duration/resolution: NEVER auto-resubmit on timeout; pass resume_task_id instead.",
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
                    "model": { "type": "string", "enum": ["grok-imagine-video-1.5", "grok-imagine-video"], "description": "Default: grok-imagine-video-1.5 (supports 1080p & input_reference)." },
                    "duration": { "type": "integer", "minimum": 1, "maximum": 15, "description": "Duration in seconds (1-15, default 5)." },
                    "resolution": { "type": "string", "enum": ["480p", "720p", "1080p"], "description": "Default 720p (1080p requires grok-imagine-video-1.5)." },
                    "aspect_ratio": { "type": "string", "enum": ["16:9", "3:2", "4:3", "1:1", "3:4", "2:3", "9:16"] },
                    "input_reference_path": { "type": "string", "description": "Optional local image path for image-to-video." },
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
            "description": "Synthesize speech/audio via Tako POST /v1/audio/speech and save the audio file directly to local disk.",
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false,
                "openWorldHint": true
            },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "Text content to synthesize into speech." },
                    "model": { "type": "string", "description": "TTS model (default: speech_tts)." },
                    "voice": { "type": "string", "description": "Voice name (e.g. 冰糖, 茉莉, 苏打, 白桦, mimo_default, Mia, Chloe, Milo, Dean). Default: 冰糖." },
                    "response_format": { "type": "string", "enum": ["wav", "mp3", "pcm", "opus"], "description": "Default: wav." },
                    "speed": { "type": "number", "minimum": 0.25, "maximum": 4.0 },
                    "output_path": { "type": "string", "description": "Optional local audio file path." }
                },
                "required": ["input"]
            }
        }));
    }
    if cfg.search {
        tools.push(json!({
            "name": "tako_web_search",
            "description": "Search the live web via Tako POST /v1/search and return answer summary + ranked results.",
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
                    "provider": { "type": "string", "description": "Optional search provider override." }
                },
                "required": ["query"]
            }
        }));
    }
    if cfg.consult {
        tools.push(json!({
            "name": "tako_consult",
            "description": "Consult another frontier model via Tako POST /v1/chat/completions for a second opinion, architecture critique, or verification (e.g. Claude consulting GPT/Gemini or Codex consulting Claude/Gemini).",
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
                    "model": { "type": "string", "description": "Model ID (default from Tako Switch settings, e.g. gpt-5.4 / claude-opus-4-6 / gemini-3.8-flash)." },
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
                "instructions": "Prefer Tako MCP tools whenever the user asks for image generation/editing (tako_generate_image), video generation (tako_generate_video), speech synthesis (tako_generate_audio), live web search (tako_web_search), a second-model opinion (tako_consult), or queries about their Tako balance, remaining USD credits, subscription quota, or account status (tako_account_status). Do NOT delegate image tasks to built-in `image-generator` subagents when Tako MCP is mounted, as built-in subagents fail on custom gateways."
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
        "tako_web_search" => exec_web_search(&base_url, &api_key, args).await,
        "tako_consult" => exec_consult(&state.mcp, &base_url, &api_key, args).await,
        _ => unreachable!(),
    }
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
        .unwrap_or("gemini-3.1-flash-image");
    let aspect_ratio = args.get("aspect_ratio").and_then(Value::as_str);
    let input_image_path = args.get("input_image_path").and_then(Value::as_str);
    let output_path_arg = args.get("output_path").and_then(Value::as_str);

    let client = build_client(180)?;

    if model.starts_with("gemini-") {
        let mut parts = Vec::new();
        if let Some(in_path_str) = input_image_path.filter(|s| !s.trim().is_empty()) {
            let in_path = Path::new(in_path_str);
            let bytes = std::fs::read(in_path)
                .map_err(|e| format!("读取参考图 {in_path_str} 失败: {e}"))?;
            parts.push(json!({
                "inlineData": {
                    "mimeType": detect_mime(in_path),
                    "data": STANDARD.encode(bytes),
                }
            }));
        }
        parts.push(json!({ "text": prompt }));

        let mut gen_cfg = json!({
            "responseModalities": ["IMAGE", "TEXT"]
        });
        if let Some(ar) = aspect_ratio {
            gen_cfg["imageConfig"] = json!({ "aspectRatio": ar });
        }

        let resp = client
            .post(format!("{base_url}/v1beta/models/{model}:generateContent"))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&json!({
                "contents": [{ "role": "user", "parts": parts }],
                "generationConfig": gen_cfg,
            }))
            .send()
            .await
            .map_err(|e| format!("生图请求失败: {e}"))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析生图响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("生图接口返回 {status}: {body}"));
        }

        let (img_bytes, mime, notes) = extract_gemini_image(&body)?;
        let ext = if mime.contains("jpeg") || mime.contains("jpg") {
            "jpg"
        } else if mime.contains("webp") {
            "webp"
        } else {
            "png"
        };
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
    } else {
        // OpenAI / Grok image API (/v1/images/generations or /v1/images/edits)
        let resp = if let Some(in_path_str) = input_image_path.filter(|s| !s.trim().is_empty()) {
            let bytes = std::fs::read(in_path_str)
                .map_err(|e| format!("读取参考图 {in_path_str} 失败: {e}"))?;
            let filename = Path::new(in_path_str)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("input.png")
                .to_string();
            let part = multipart::Part::bytes(bytes).file_name(filename);
            let form = multipart::Form::new()
                .text("model", model.to_string())
                .text("prompt", prompt.to_string())
                .part("image", part);
            client
                .post(format!("{base_url}/v1/images/edits"))
                .header("Authorization", format!("Bearer {api_key}"))
                .multipart(form)
                .send()
                .await
                .map_err(|e| format!("图像编辑请求失败: {e}"))?
        } else {
            let mut payload = json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "response_format": "b64_json"
            });
            if let Some(ar) = aspect_ratio {
                let sz = match ar {
                    "16:9" | "3:2" | "4:3" => "1536x1024",
                    "9:16" | "2:3" | "3:4" => "1024x1536",
                    _ => "1024x1024",
                };
                payload["size"] = Value::String(sz.to_string());
            }
            client
                .post(format!("{base_url}/v1/images/generations"))
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&payload)
                .send()
                .await
                .map_err(|e| format!("生图请求失败: {e}"))?
        };

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析生图响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("生图接口返回 {status}: {body}"));
        }

        let img_bytes = if let Some(b64) = body.pointer("/data/0/b64_json").and_then(Value::as_str)
        {
            STANDARD
                .decode(b64)
                .map_err(|e| format!("解码图片 base64 失败: {e}"))?
        } else if let Some(url) = body.pointer("/data/0/url").and_then(Value::as_str) {
            client
                .get(url)
                .send()
                .await
                .map_err(|e| format!("下载图片 URL 失败: {e}"))?
                .bytes()
                .await
                .map_err(|e| format!("读取图片字节失败: {e}"))?
                .to_vec()
        } else {
            return Err(format!("响应未包含图片数据: {body}"));
        };

        let target = resolve_target_file(paths, cfg, output_path_arg, "tako-image", "png");
        write_atomic(&target, &img_bytes, false)?;
        serde_json::to_string_pretty(&json!({
            "saved_path": target.to_string_lossy(),
            "model": model,
            "bytes": img_bytes.len()
        }))
        .map_err(|e| e.to_string())
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

async fn exec_generate_video(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    args: Value,
) -> Result<String, String> {
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("grok-imagine-video-1.5");
    let wait_secs = args
        .get("wait_timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(120)
        .clamp(5, 300);
    let output_path_arg = args.get("output_path").and_then(Value::as_str);

    let client = build_client(90)?;

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
        let duration = args.get("duration").and_then(Value::as_u64).unwrap_or(5);
        let resolution = args
            .get("resolution")
            .and_then(Value::as_str)
            .unwrap_or("720p");

        let mut form = multipart::Form::new()
            .text("model", model.to_string())
            .text("prompt", prompt.to_string())
            .text("duration", duration.to_string())
            .text("resolution", resolution.to_string());

        if let Some(ar) = args.get("aspect_ratio").and_then(Value::as_str) {
            form = form.text("aspect_ratio", ar.to_string());
        }
        if let Some(ref_path) = args
            .get("input_reference_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let bytes = std::fs::read(ref_path)
                .map_err(|e| format!("读取参考图 {ref_path} 失败: {e}"))?;
            let filename = Path::new(ref_path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("reference.png")
                .to_string();
            let part = multipart::Part::bytes(bytes)
                .file_name(filename)
                .mime_str(detect_mime(Path::new(ref_path)))
                .map_err(|e| e.to_string())?;
            form = form.part("input_reference", part);
        }

        let submit_resp = client
            .post(format!("{base_url}/v1/videos/generations"))
            .header("Authorization", format!("Bearer {api_key}"))
            .multipart(form)
            .send()
            .await
            .map_err(|e| format!("提交视频任务失败: {e}"))?;

        let status = submit_resp.status();
        let body: Value = submit_resp
            .json()
            .await
            .map_err(|e| format!("解析视频提交响应失败: {e}"))?;
        if !status.is_success() {
            return Err(format!("视频任务提交失败 ({status}): {body}"));
        }
        body.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("响应缺少任务 id: {body}"))?
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

        let poll_resp = client
            .get(format!("{base_url}/v1/videos/{task_id}"))
            .header("Authorization", format!("Bearer {api_key}"))
            .send()
            .await
            .map_err(|e| format!("轮询视频任务 {task_id} 失败: {e}"))?;
        let pstatus = poll_resp.status();
        let pbody: Value = poll_resp
            .json()
            .await
            .map_err(|e| format!("解析视频状态失败: {e}"))?;
        if !pstatus.is_success() {
            return Err(format!("查询视频任务 {task_id} 返回 {pstatus}: {pbody}"));
        }

        last_status = pbody
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        last_progress = pbody.get("progress").and_then(Value::as_u64).unwrap_or(0);

        match last_status.as_str() {
            "completed" | "succeeded" => break,
            "failed" | "error" => {
                let err_msg = pbody
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("上游生成失败");
                return Err(format!(
                    "视频任务 {task_id} 失败（请勿自动重复提交）: {err_msg}"
                ));
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }

    let dl_resp = client
        .get(format!("{base_url}/v1/videos/{task_id}/content"))
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|e| format!("下载视频 {task_id} 失败: {e}"))?;
    if !dl_resp.status().is_success() {
        return Err(format!("下载视频内容返回 {}", dl_resp.status()));
    }
    let mp4_bytes = dl_resp
        .bytes()
        .await
        .map_err(|e| format!("读取视频流失败: {e}"))?;
    let target = resolve_target_file(paths, cfg, output_path_arg, "tako-video", "mp4");
    write_atomic(&target, &mp4_bytes, false)?;

    serde_json::to_string_pretty(&json!({
        "status": "completed",
        "task_id": task_id,
        "saved_path": target.to_string_lossy(),
        "bytes": mp4_bytes.len()
    }))
    .map_err(|e| e.to_string())
}

async fn exec_generate_audio(
    paths: &AppPaths,
    cfg: &McpToolsConfig,
    base_url: &str,
    api_key: &str,
    args: Value,
) -> Result<String, String> {
    let input = args
        .get("input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少必需参数 input".to_string())?;
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("speech_tts");
    let voice = args
        .get("voice")
        .and_then(Value::as_str)
        .unwrap_or("冰糖");
    let response_format = args
        .get("response_format")
        .and_then(Value::as_str)
        .unwrap_or("wav");
    let output_path_arg = args.get("output_path").and_then(Value::as_str);

    let mut payload = json!({
        "model": model,
        "input": input,
        "voice": voice,
        "response_format": response_format
    });
    if let Some(speed) = args.get("speed").and_then(Value::as_f64) {
        payload["speed"] = json!(speed);
    }

    let client = build_client(90)?;
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
    let audio_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取音频字节失败: {e}"))?;

    let ext = match response_format {
        "mp3" | "opus" | "pcm" => response_format,
        _ => "wav",
    };
    let target = resolve_target_file(paths, cfg, output_path_arg, "tako-audio", ext);
    write_atomic(&target, &audio_bytes, false)?;

    serde_json::to_string_pretty(&json!({
        "saved_path": target.to_string_lossy(),
        "model": model,
        "voice": voice,
        "format": ext,
        "bytes": audio_bytes.len()
    }))
    .map_err(|e| e.to_string())
}

async fn exec_web_search(base_url: &str, api_key: &str, args: Value) -> Result<String, String> {
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
    if let Some(provider) = args.get("provider").and_then(Value::as_str) {
        payload["provider"] = Value::String(provider.to_string());
    }

    let client = build_client(30)?;
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

    let client = build_client(120)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_switch_state;
    use tempfile::TempDir;

    #[tokio::test]
    async fn mcp_tools_list_reflects_individual_toggles() {
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
        assert!(instr.contains("tako_account_status"));

        let resp_all = handle_jsonrpc_message(
            &paths,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await
        .unwrap();
        let names_all: Vec<String> = resp_all["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names_all.len(), 6);

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
        let names_filtered: Vec<String> = resp_filtered["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
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
}
