//! Independent blackbox integration tests for Tako Switch v2.2.0.
//!
//! Strictly adheres to the shared contract in `docs/dispatch/v2.2.0.md`:
//! - Module A (`TC-EO-*`): EdgeOne `Tako Web Page` MCP config compatibility,
//!   entry generation, multi-client sync/cleanup, and Node.js runtime detection.
//! - Module B (`TC-AGY-*`): Antigravity full Chinese dictionary integrity,
//!   ASAR preload engine injection (dict + dynamic rules + Shadow DOM + code
//!   skip guards), idempotency, upgrade awareness, and 100% SHA-256 restore.
//!
//! Safety guarantees:
//! - Every test operates strictly inside a `tempfile::TempDir` with
//!   `AppPaths::for_test(tmp.path())` (`paths.is_test == true`).
//! - Never touches host `/Applications/Claude.app`, `/Applications/Antigravity.app`,
//!   or real user home configs.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tako_switch_lib::adapters::antigravity::AntigravityAdapter;
use tako_switch_lib::adapters::antigravity_i18n::PATCH_GUARD_COMMENT;
use tako_switch_lib::adapters::claude::ClaudeAdapter;
use tako_switch_lib::adapters::codex::CodexAdapter;
use tako_switch_lib::adapters::dsh_tako::DshTakoAdapter;
use tako_switch_lib::adapters::{
    build_edgeone_mcp_json_entry, detect_node_runtime, inspect_antigravity_ui_patch,
    patch_antigravity_ui_for_paths, restore_antigravity_ui_for_paths, AppMode, DesktopAppAdapter,
    UiPatchStatus, EDGEONE_MCP_SERVER_KEY,
};
use tako_switch_lib::config::{
    read_switch_state, save_switch_auth, save_switch_state, AppPaths, McpToolsConfig, SwitchState,
};
use tempfile::TempDir;
use toml_edit::DocumentMut;

const ASAR_BLOCK_SIZE: usize = 4 * 1024 * 1024;
const EXPECTED_EDGEONE_ARGS: &[&str] = &["-y", "@edgeone/makers-mcp@latest", "--region", "china"];

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn setup_sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create sandbox tempdir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(
        paths.is_test,
        "all v2.2.0 blackbox tests must run with paths.is_test == true"
    );
    (tmp, paths)
}

fn setup_logged_in_sandbox() -> (TempDir, AppPaths) {
    let (tmp, paths) = setup_sandbox();
    save_switch_auth(
        &paths,
        Some("sk-tako-v220-blackbox-test-key"),
        Some("api-id-v220"),
        Some("manual"),
    )
    .expect("save sandbox auth");
    (tmp, paths)
}

fn assert_is_npx_command(cmd: &str, context: &str) {
    let trimmed = cmd.trim();
    assert!(!trimmed.is_empty(), "{context}: command must not be empty");
    let is_npx = trimmed == "npx"
        || Path::new(trimmed)
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| stem.eq_ignore_ascii_case("npx"));
    assert!(
        is_npx,
        "{context}: command must be 'npx' or an absolute path to npx(.cmd), got '{trimmed}'"
    );
}

fn assert_json_edgeone_entry(
    entry: &Value,
    expected_token: Option<&str>,
    expected_project: Option<&str>,
    context: &str,
) {
    let obj = entry
        .as_object()
        .unwrap_or_else(|| panic!("{context}: edgeone entry must be a JSON object, got {entry}"));

    let cmd = obj
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: missing string 'command' field"));
    assert_is_npx_command(cmd, context);

    let args: Vec<&str> = obj
        .get("args")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: missing array 'args' field"))
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("{context}: args element must be string, got {v}"))
        })
        .collect();
    assert_eq!(
        args, EXPECTED_EDGEONE_ARGS,
        "{context}: unexpected EdgeOne MCP args"
    );

    match (expected_token, expected_project) {
        (None, None) => {
            assert!(
                !obj.contains_key("env"),
                "{context}: must NOT contain 'env' key when both token and project are empty, got {entry}"
            );
        }
        (tok, proj) => {
            let env = obj
                .get("env")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{context}: expected 'env' object, got {entry}"));
            match tok {
                Some(expected) => {
                    assert_eq!(
                        env.get("EDGEONE_PAGES_API_TOKEN").and_then(Value::as_str),
                        Some(expected),
                        "{context}: mismatch on EDGEONE_PAGES_API_TOKEN"
                    );
                }
                None => {
                    assert!(
                        !env.contains_key("EDGEONE_PAGES_API_TOKEN"),
                        "{context}: must NOT write EDGEONE_PAGES_API_TOKEN when empty, got {env:?}"
                    );
                }
            }
            match proj {
                Some(expected) => {
                    assert_eq!(
                        env.get("EDGEONE_PAGES_PROJECT_NAME").and_then(Value::as_str),
                        Some(expected),
                        "{context}: mismatch on EDGEONE_PAGES_PROJECT_NAME"
                    );
                }
                None => {
                    assert!(
                        !env.contains_key("EDGEONE_PAGES_PROJECT_NAME"),
                        "{context}: must NOT write EDGEONE_PAGES_PROJECT_NAME when empty, got {env:?}"
                    );
                }
            }
        }
    }
}

fn contains_cjk(s: &str) -> bool {
    s.chars()
        .any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c))
}

/// Decode `\uXXXX` JavaScript unicode escapes so assertions treat UTF-8 literals
/// and escaped literals equivalently.
fn decode_js_unicode_escapes(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 5 < bytes.len() && bytes[i + 1] == b'u' {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 2..i + 6]) {
                if let Ok(code) = u16::from_str_radix(hex_str, 16) {
                    if let Some(ch) = char::from_u32(code as u32) {
                        out.push(ch);
                        i += 6;
                        continue;
                    }
                }
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn make_asar_file_entry(data: &[u8], payload: &mut Vec<u8>) -> Value {
    let offset = payload.len().to_string();
    let size = data.len() as u64;
    let hash = sha256_hex(data);
    let blocks: Vec<String> = if data.is_empty() {
        vec![hash.clone()]
    } else {
        data.chunks(ASAR_BLOCK_SIZE).map(sha256_hex).collect()
    };
    payload.extend_from_slice(data);
    json!({
        "size": size,
        "offset": offset,
        "integrity": {
            "algorithm": "SHA256",
            "hash": hash,
            "blockSize": ASAR_BLOCK_SIZE as u64,
            "blocks": blocks,
        }
    })
}

fn build_clean_antigravity_asar(app_version: &str) -> (Vec<u8>, String) {
    let mut payload = Vec::new();

    let pkg_json = format!(
        r#"{{"name":"antigravity","version":"{app_version}","main":"dist/main.js"}}"#
    );
    let menu_js = format!(
        "// Antigravity {app_version} menu\nconst {{ Menu }} = require(\"electron\");\nmodule.exports = function buildMenu() {{\n  return [{{ label: \"File\" }}, {{ label: \"Edit\" }}, {{ label: \"View\" }}, {{ label: \"Window\" }}, {{ label: \"Help\" }}, {{ label: \"Settings\" }}];\n}};\n"
    );
    let tray_js = format!(
        "// Antigravity {app_version} tray\nmodule.exports = function buildTray() {{\n  return [{{ label: \"Open Antigravity\" }}, {{ label: \"Quit\" }}];\n}};\n"
    );
    let updater_js = format!(
        "// Antigravity {app_version} updater\nmodule.exports = {{ checkLabel: \"Check for Updates...\" }};\n"
    );
    let preload_js = format!(
        "// Antigravity {app_version} preload\nconst {{ contextBridge }} = require(\"electron\");\nwindow.__AGY_RELEASE__ = \"{app_version}\";\n"
    );

    let pkg_node = make_asar_file_entry(pkg_json.as_bytes(), &mut payload);
    let menu_node = make_asar_file_entry(menu_js.as_bytes(), &mut payload);
    let tray_node = make_asar_file_entry(tray_js.as_bytes(), &mut payload);
    let updater_node = make_asar_file_entry(updater_js.as_bytes(), &mut payload);
    let preload_node = make_asar_file_entry(preload_js.as_bytes(), &mut payload);

    let header_value = json!({
        "files": {
            "package.json": pkg_node,
            "dist": {
                "files": {
                    "menu.js": menu_node,
                    "tray.js": tray_node,
                    "updater.js": updater_node,
                    "preload.js": preload_node,
                }
            },
            "node_modules": {
                "files": {
                    "node-pty": {
                        "files": {
                            "pty.node": {
                                "size": 4096,
                                "unpacked": true
                            }
                        }
                    }
                }
            }
        }
    });

    let header_json = serde_json::to_string(&header_value).expect("serialize ASAR header");
    let header_bytes = header_json.as_bytes();
    let header_sha256 = sha256_hex(header_bytes);

    let strlen = header_bytes.len() as u32;
    let padded_len = (strlen + 3) & !3;
    let u0: u32 = 4;
    let u1: u32 = 8 + padded_len;
    let u2: u32 = 4 + padded_len;

    let mut asar = Vec::with_capacity(16 + padded_len as usize + payload.len());
    asar.extend_from_slice(&u0.to_le_bytes());
    asar.extend_from_slice(&u1.to_le_bytes());
    asar.extend_from_slice(&u2.to_le_bytes());
    asar.extend_from_slice(&strlen.to_le_bytes());
    asar.extend_from_slice(header_bytes);
    asar.resize(16 + padded_len as usize, 0u8);
    asar.extend_from_slice(&payload);

    (asar, header_sha256)
}

fn render_macos_info_plist(app_version: &str, header_sha256: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.google.antigravity</string>
    <key>CFBundleName</key>
    <string>Antigravity</string>
    <key>CFBundleShortVersionString</key>
    <string>{app_version}</string>
    <key>ElectronAsarIntegrity</key>
    <dict>
        <key>Resources/app.asar</key>
        <dict>
            <key>algorithm</key>
            <string>SHA256</string>
            <key>hash</key>
            <string>{header_sha256}</string>
        </dict>
    </dict>
</dict>
</plist>
"#
    )
}

fn write_fake_antigravity_bundle(
    app_dir: &Path,
    app_version: &str,
) -> (PathBuf, PathBuf, Vec<u8>, Vec<u8>, String) {
    let contents_dir = app_dir.join("Contents");
    let resources_dir = contents_dir.join("Resources");
    fs::create_dir_all(&resources_dir).expect("create fake Antigravity Resources dir");

    let (asar_bytes, header_sha256) = build_clean_antigravity_asar(app_version);
    let plist_bytes = render_macos_info_plist(app_version, &header_sha256).into_bytes();

    let asar_path = resources_dir.join("app.asar");
    let plist_path = contents_dir.join("Info.plist");
    fs::write(&asar_path, &asar_bytes).expect("write clean app.asar");
    fs::write(&plist_path, &plist_bytes).expect("write clean Info.plist");

    (asar_path, plist_path, asar_bytes, plist_bytes, header_sha256)
}

struct ParsedAsar {
    header: Value,
    header_sha256: String,
    data_section: Vec<u8>,
}

fn parse_and_verify_asar_pickle(bytes: &[u8]) -> ParsedAsar {
    assert!(bytes.len() >= 16, "ASAR archive must be at least 16 bytes");
    let u0 = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let u1 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let u2 = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let strlen = u32::from_le_bytes(bytes[12..16].try_into().unwrap());

    let padded_len = (strlen + 3) & !3;
    assert_eq!(u0, 4, "Pickle outer header size u0 must be 4");
    assert_eq!(u2, 4 + padded_len, "Pickle inner size u2 must be 4 + padded_len");
    assert_eq!(u1, 8 + padded_len, "Pickle outer value u1 must be 8 + padded_len");

    let data_offset = (16 + padded_len) as usize;
    assert!(
        bytes.len() >= data_offset,
        "ASAR archive truncated before data section"
    );
    let json_end = 16 + strlen as usize;
    for &pad_byte in &bytes[json_end..data_offset] {
        assert_eq!(pad_byte, 0, "ASAR Pickle alignment padding bytes must be 0");
    }

    let header_slice = &bytes[16..json_end];
    let header_sha256 = sha256_hex(header_slice);
    let header: Value =
        serde_json::from_slice(header_slice).expect("ASAR header must be valid UTF-8 JSON");

    ParsedAsar {
        header,
        header_sha256,
        data_section: bytes[data_offset..].to_vec(),
    }
}

fn lookup_asar_node<'a>(header: &'a Value, segments: &[&str]) -> Option<&'a Value> {
    let mut cur = header;
    for seg in segments {
        cur = cur.get("files")?.get(*seg)?;
    }
    Some(cur)
}

fn extract_packed_asar_file(asar: &ParsedAsar, segments: &[&str]) -> Vec<u8> {
    let label = segments.join("/");
    let node = lookup_asar_node(&asar.header, segments)
        .unwrap_or_else(|| panic!("missing ASAR entry: {label}"));
    let size = node
        .get("size")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{label}: missing numeric size")) as usize;
    let offset_str = node
        .get("offset")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{label}: missing string offset"));
    let offset: usize = offset_str
        .parse()
        .unwrap_or_else(|e| panic!("{label}: invalid offset '{offset_str}': {e}"));
    assert!(
        offset + size <= asar.data_section.len(),
        "{label}: slice [{offset}..{}] exceeds ASAR data section ({})",
        offset + size,
        asar.data_section.len()
    );
    asar.data_section[offset..offset + size].to_vec()
}

// ============================================================================
// Module A: EdgeOne `Tako Web Page` MCP Backend Sync (TC-EO-01 .. TC-EO-08)
// ============================================================================

#[test]
fn tc_eo_01_legacy_state_json_backward_compat_and_roundtrip() {
    let (_tmp, paths) = setup_sandbox();

    // Legacy v2.1.1 state.json without `webPage`, `edgeoneApiToken`, `edgeoneProjectName`
    let legacy_json = json!({
        "baseUrl": "https://tako.shiroha.tech",
        "manifestUrl": "https://example.com/manifest.json",
        "fallbackManifestUrl": "https://example.com/fallback.json",
        "autoCheckUpdates": true,
        "codexPreserveOfficialAuth": false,
        "gatewayDisplayName": "Tako",
        "gatewayDisplaySubtitle": "Tako AI Gateway",
        "preferChineseUi": true,
        "showTrayBalance": true,
        "bypassSystemProxy": true,
        "mcp": {
            "image": false,
            "video": true,
            "audio": true,
            "search": false,
            "consult": true,
            "outputDir": "~/Downloads/TakoCustom",
            "consultModel": "swe-2"
        }
    });
    let raw = serde_json::to_string_pretty(&legacy_json).unwrap();

    // Direct deserialization must succeed (not silently fall back to defaults)
    let direct: SwitchState =
        serde_json::from_str(&raw).expect("legacy state.json must deserialize cleanly");
    assert!(!direct.mcp.web_page, "web_page must default to false");
    assert_eq!(direct.mcp.edgeone_api_token, None);
    assert_eq!(direct.mcp.edgeone_project_name, None);
    assert!(!direct.mcp.image, "legacy image=false must be preserved");
    assert!(!direct.mcp.search, "legacy search=false must be preserved");
    assert_eq!(direct.mcp.consult_model, "swe-2");

    // Disk persistence roundtrip via read_switch_state / save_switch_state
    fs::create_dir_all(paths.switch_dir()).unwrap();
    fs::write(paths.state_path(), &raw).unwrap();
    let mut loaded = read_switch_state(&paths);
    assert!(!loaded.mcp.web_page);
    assert_eq!(loaded.mcp.edgeone_api_token, None);
    assert_eq!(loaded.mcp.edgeone_project_name, None);
    assert!(!loaded.mcp.image);

    loaded.mcp.web_page = true;
    loaded.mcp.edgeone_api_token = Some("eo-token-roundtrip".to_string());
    loaded.mcp.edgeone_project_name = Some("tako-landing".to_string());
    save_switch_state(&paths, &loaded).expect("save updated v2.2.0 state");

    let saved_val: Value =
        serde_json::from_str(&fs::read_to_string(paths.state_path()).unwrap()).unwrap();
    let mcp_obj = saved_val.get("mcp").and_then(Value::as_object).unwrap();
    assert_eq!(mcp_obj.get("webPage").and_then(Value::as_bool), Some(true));
    assert_eq!(
        mcp_obj.get("edgeoneApiToken").and_then(Value::as_str),
        Some("eo-token-roundtrip")
    );
    assert_eq!(
        mcp_obj.get("edgeoneProjectName").and_then(Value::as_str),
        Some("tako-landing")
    );

    let reloaded = read_switch_state(&paths);
    assert!(reloaded.mcp.web_page);
    assert_eq!(
        reloaded.mcp.edgeone_api_token.as_deref(),
        Some("eo-token-roundtrip")
    );
    assert_eq!(
        reloaded.mcp.edgeone_project_name.as_deref(),
        Some("tako-landing")
    );
}

#[test]
fn tc_eo_02_build_edgeone_mcp_json_entry_without_credentials_and_whitespace_cleanup() {
    assert_eq!(
        EDGEONE_MCP_SERVER_KEY, "tako-web-page",
        "EDGEONE_MCP_SERVER_KEY contract mismatch"
    );

    // Case 1: None / None -> must omit `env` completely
    let mcp_empty = McpToolsConfig {
        web_page: true,
        edgeone_api_token: None,
        edgeone_project_name: None,
        ..Default::default()
    };
    let entry_empty = build_edgeone_mcp_json_entry(&mcp_empty);
    assert_json_edgeone_entry(&entry_empty, None, None, "no-credentials");

    // Case 2: Empty / whitespace-only strings -> sanitized away, must omit `env`
    let mcp_blank = McpToolsConfig {
        web_page: true,
        edgeone_api_token: Some("   \t  ".to_string()),
        edgeone_project_name: Some("\n  \r\n".to_string()),
        ..Default::default()
    };
    let entry_blank = build_edgeone_mcp_json_entry(&mcp_blank);
    assert_json_edgeone_entry(&entry_blank, None, None, "whitespace-only-credentials");
}

#[test]
fn tc_eo_03_build_edgeone_mcp_json_entry_with_token_and_project_name() {
    // Only token (with surrounding whitespace), project blank
    let mcp_tok = McpToolsConfig {
        web_page: true,
        edgeone_api_token: Some("  eo_tok_abc123 \n".to_string()),
        edgeone_project_name: Some("   ".to_string()),
        ..Default::default()
    };
    let entry_tok = build_edgeone_mcp_json_entry(&mcp_tok);
    assert_json_edgeone_entry(&entry_tok, Some("eo_tok_abc123"), None, "token-only");

    // Only project (with surrounding whitespace), token None
    let mcp_proj = McpToolsConfig {
        web_page: true,
        edgeone_api_token: None,
        edgeone_project_name: Some("  demo-site-cn  ".to_string()),
        ..Default::default()
    };
    let entry_proj = build_edgeone_mcp_json_entry(&mcp_proj);
    assert_json_edgeone_entry(&entry_proj, None, Some("demo-site-cn"), "project-only");

    // Both token and project set
    let mcp_both = McpToolsConfig {
        web_page: true,
        edgeone_api_token: Some("  eo_tok_xyz789 ".to_string()),
        edgeone_project_name: Some(" prod-web ".to_string()),
        ..Default::default()
    };
    let entry_both = build_edgeone_mcp_json_entry(&mcp_both);
    assert_json_edgeone_entry(
        &entry_both,
        Some("eo_tok_xyz789"),
        Some("prod-web"),
        "token-and-project",
    );
}

#[test]
fn tc_eo_04_claude_1p_and_3p_edgeone_mcp_sync_and_cleanup() {
    let (_tmp, paths) = setup_logged_in_sandbox();
    let cfg_1p = paths.claude_1p_dir.join("claude_desktop_config.json");
    let cfg_3p = paths.claude_3p_dir.join("claude_desktop_config.json");
    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();

    // Seed user-owned MCP servers in both 1p and 3p configs
    fs::write(
        &cfg_1p,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "user-1p-fs": { "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"] }
            },
            "userPreference": "keep-1p"
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &cfg_3p,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "user-3p-db": { "command": "uvx", "args": ["mcp-server-sqlite"] }
            },
            "userPreference": "keep-3p"
        }))
        .unwrap(),
    )
    .unwrap();

    // Enable web_page with credentials & apply AppMode::Tako
    let mut state = SwitchState::default_for(&paths);
    state.mcp.web_page = true;
    state.mcp.edgeone_api_token = Some("eo-claude-tok".to_string());
    state.mcp.edgeone_project_name = Some("claude-site".to_string());
    save_switch_state(&paths, &state).unwrap();

    ClaudeAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Claude apply_mode(Tako) with web_page=true");

    for (label, path, preserved_key, preserved_pref) in [
        ("claude-1p", &cfg_1p, "user-1p-fs", "keep-1p"),
        ("claude-3p", &cfg_3p, "user-3p-db", "keep-3p"),
    ] {
        let doc: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            doc.get("userPreference").and_then(Value::as_str),
            Some(preserved_pref),
            "{label}: unrelated top-level key must be preserved"
        );
        let servers = doc
            .get("mcpServers")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{label}: missing mcpServers object"));
        assert!(
            servers.contains_key(preserved_key),
            "{label}: user server '{preserved_key}' must be preserved"
        );
        let edgeone = servers
            .get(EDGEONE_MCP_SERVER_KEY)
            .unwrap_or_else(|| panic!("{label}: missing '{EDGEONE_MCP_SERVER_KEY}' in Tako+web_page=true"));
        assert_json_edgeone_entry(edgeone, Some("eo-claude-tok"), Some("claude-site"), label);
    }

    // Disable web_page while staying in Tako -> must remove tako-web-page and preserve user servers
    state.mcp.web_page = false;
    save_switch_state(&paths, &state).unwrap();
    ClaudeAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Claude apply_mode(Tako) with web_page=false");

    for (label, path, preserved_key) in [
        ("claude-1p-disabled", &cfg_1p, "user-1p-fs"),
        ("claude-3p-disabled", &cfg_3p, "user-3p-db"),
    ] {
        let doc: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let servers = doc.get("mcpServers").and_then(Value::as_object).unwrap();
        assert!(
            !servers.contains_key(EDGEONE_MCP_SERVER_KEY),
            "{label}: '{EDGEONE_MCP_SERVER_KEY}' must be removed when web_page=false"
        );
        assert!(
            servers.contains_key(preserved_key),
            "{label}: user server '{preserved_key}' must still exist"
        );
    }

    // Re-enable web_page in Tako, then switch back to Official -> must remove tako-web-page cleanly
    state.mcp.web_page = true;
    state.mcp.edgeone_api_token = None;
    state.mcp.edgeone_project_name = None;
    save_switch_state(&paths, &state).unwrap();
    ClaudeAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Claude re-apply Tako with web_page=true");
    ClaudeAdapter
        .apply_mode(&paths, AppMode::Official)
        .expect("Claude apply_mode(Official)");

    for (label, path, preserved_key) in [
        ("claude-1p-official", &cfg_1p, "user-1p-fs"),
        ("claude-3p-official", &cfg_3p, "user-3p-db"),
    ] {
        let doc: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let servers = doc.get("mcpServers").and_then(Value::as_object).unwrap();
        assert!(
            !servers.contains_key(EDGEONE_MCP_SERVER_KEY),
            "{label}: '{EDGEONE_MCP_SERVER_KEY}' must be removed in Official mode"
        );
        assert!(
            servers.contains_key(preserved_key),
            "{label}: user server '{preserved_key}' must be preserved in Official mode"
        );
    }
}

#[test]
fn tc_eo_05_codex_config_toml_edgeone_mcp_sync_and_cleanup() {
    let (_tmp, paths) = setup_logged_in_sandbox();
    fs::create_dir_all(&paths.codex_dir).unwrap();
    let cfg_path = paths.codex_dir.join("config.toml");

    fs::write(
        &cfg_path,
        r#"model = "gpt-5.4"

[mcp_servers.user_custom]
command = "uvx"
args = ["mcp-server-git"]
"#,
    )
    .unwrap();

    // 1) Tako + web_page=true (no credentials -> no env table)
    let mut state = SwitchState::default_for(&paths);
    state.mcp.web_page = true;
    state.mcp.edgeone_api_token = None;
    state.mcp.edgeone_project_name = None;
    save_switch_state(&paths, &state).unwrap();

    CodexAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Codex apply_mode(Tako) with web_page=true");

    let doc: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let servers = doc
        .get("mcp_servers")
        .and_then(|i| i.as_table_like())
        .expect("codex config.toml must contain [mcp_servers]");
    assert!(
        servers.contains_key("user_custom"),
        "codex must preserve user_custom mcp server"
    );
    assert!(
        servers.contains_key("tako"),
        "codex must keep built-in tako mcp server in Tako mode"
    );
    let eo_tbl = servers
        .get(EDGEONE_MCP_SERVER_KEY)
        .and_then(|i| i.as_table_like())
        .expect("codex must write [mcp_servers.\"tako-web-page\"] when web_page=true");
    let cmd = eo_tbl
        .get("command")
        .and_then(|i| i.as_str())
        .expect("codex tako-web-page missing command");
    assert_is_npx_command(cmd, "codex-no-creds");
    let args: Vec<&str> = eo_tbl
        .get("args")
        .and_then(|i| i.as_value())
        .and_then(|v| v.as_array())
        .expect("codex tako-web-page missing args array")
        .iter()
        .map(|v| v.as_str().expect("codex arg must be string"))
        .collect();
    assert_eq!(args, EXPECTED_EDGEONE_ARGS);
    assert!(
        eo_tbl.get("env").is_none(),
        "codex tako-web-page must NOT write env table when credentials are empty"
    );

    // 2) Add token + project name and re-sync -> env subtable populated
    state.mcp.edgeone_api_token = Some("eo-codex-tok".to_string());
    state.mcp.edgeone_project_name = Some("codex-web".to_string());
    save_switch_state(&paths, &state).unwrap();
    CodexAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Codex re-apply Tako with credentials");

    let doc_creds: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let eo_env = doc_creds
        .get("mcp_servers")
        .and_then(|i| i.as_table_like())
        .and_then(|s| s.get(EDGEONE_MCP_SERVER_KEY))
        .and_then(|i| i.as_table_like())
        .and_then(|t| t.get("env"))
        .and_then(|i| i.as_table_like())
        .expect("codex tako-web-page must write env table when credentials are configured");
    assert_eq!(
        eo_env.get("EDGEONE_PAGES_API_TOKEN").and_then(|i| i.as_str()),
        Some("eo-codex-tok")
    );
    assert_eq!(
        eo_env
            .get("EDGEONE_PAGES_PROJECT_NAME")
            .and_then(|i| i.as_str()),
        Some("codex-web")
    );

    // 3) Disable web_page=false while staying in Tako -> remove only tako-web-page
    state.mcp.web_page = false;
    save_switch_state(&paths, &state).unwrap();
    CodexAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Codex apply_mode(Tako) with web_page=false");

    let doc_off: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let servers_off = doc_off
        .get("mcp_servers")
        .and_then(|i| i.as_table_like())
        .expect("mcp_servers table should remain for tako and user_custom");
    assert!(
        !servers_off.contains_key(EDGEONE_MCP_SERVER_KEY),
        "tako-web-page must be removed when web_page=false"
    );
    assert!(servers_off.contains_key("tako"));
    assert!(servers_off.contains_key("user_custom"));

    // 4) Re-enable web_page=true then switch back to Official -> remove tako-web-page & tako, keep user_custom
    state.mcp.web_page = true;
    save_switch_state(&paths, &state).unwrap();
    CodexAdapter.apply_mode(&paths, AppMode::Tako).unwrap();
    CodexAdapter
        .apply_mode(&paths, AppMode::Official)
        .expect("Codex apply_mode(Official)");

    let doc_official: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let servers_official = doc_official
        .get("mcp_servers")
        .and_then(|i| i.as_table_like())
        .expect("user_custom mcp_servers must remain in Official mode");
    assert!(!servers_official.contains_key(EDGEONE_MCP_SERVER_KEY));
    assert!(!servers_official.contains_key("tako"));
    assert!(servers_official.contains_key("user_custom"));
}

#[test]
fn tc_eo_06_antigravity_mcp_config_edgeone_sync_and_cleanup() {
    let (_tmp, paths) = setup_logged_in_sandbox();
    fs::create_dir_all(&paths.agy_gemini_config_dir).unwrap();
    let mcp_path = paths.agy_gemini_config_dir.join("mcp_config.json");

    fs::write(
        &mcp_path,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "user-agy": { "command": "uvx", "args": ["demo-tool"] }
            }
        }))
        .unwrap(),
    )
    .unwrap();

    // 1) Tako + web_page=true with token only
    let mut state = SwitchState::default_for(&paths);
    state.mcp.web_page = true;
    state.mcp.edgeone_api_token = Some("eo-agy-tok".to_string());
    state.mcp.edgeone_project_name = Some("   ".to_string());
    save_switch_state(&paths, &state).unwrap();

    AntigravityAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Antigravity apply_mode(Tako) with web_page=true");

    let doc: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let servers = doc.get("mcpServers").and_then(Value::as_object).unwrap();
    assert!(servers.contains_key("user-agy"), "must preserve user-agy");
    assert!(servers.contains_key("tako"), "must keep built-in tako");
    let eo = servers
        .get(EDGEONE_MCP_SERVER_KEY)
        .expect("Antigravity mcp_config.json must contain tako-web-page");
    assert_json_edgeone_entry(eo, Some("eo-agy-tok"), None, "antigravity-tako");

    // 2) web_page=false while staying in Tako
    state.mcp.web_page = false;
    save_switch_state(&paths, &state).unwrap();
    AntigravityAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("Antigravity apply_mode(Tako) with web_page=false");

    let doc_off: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let servers_off = doc_off.get("mcpServers").and_then(Value::as_object).unwrap();
    assert!(!servers_off.contains_key(EDGEONE_MCP_SERVER_KEY));
    assert!(servers_off.contains_key("tako"));
    assert!(servers_off.contains_key("user-agy"));

    // 3) Re-enable then switch to Official
    state.mcp.web_page = true;
    save_switch_state(&paths, &state).unwrap();
    AntigravityAdapter.apply_mode(&paths, AppMode::Tako).unwrap();
    AntigravityAdapter
        .apply_mode(&paths, AppMode::Official)
        .expect("Antigravity apply_mode(Official)");

    let doc_official: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let servers_official = doc_official
        .get("mcpServers")
        .and_then(Value::as_object)
        .unwrap();
    assert!(!servers_official.contains_key(EDGEONE_MCP_SERVER_KEY));
    assert!(!servers_official.contains_key("tako"));
    assert!(servers_official.contains_key("user-agy"));
}

#[test]
fn tc_eo_07_and_08_dsh_tako_safety_and_detect_node_runtime_contract() {
    let (_tmp, paths) = setup_logged_in_sandbox();
    fs::create_dir_all(&paths.dsh_tako_dir).unwrap();
    let creds_path = DshTakoAdapter::credentials_path(&paths);
    fs::write(
        &creds_path,
        "version: 1\nrefs:\n  OTHER_TOKEN: keep_me_intact\n",
    )
    .unwrap();

    let mut state = SwitchState::default_for(&paths);
    state.mcp.web_page = true;
    save_switch_state(&paths, &state).unwrap();

    DshTakoAdapter
        .apply_mode(&paths, AppMode::Tako)
        .expect("DshTako apply_mode(Tako) must stay safe when web_page=true");
    let diag = DshTakoAdapter.diagnose(&paths);
    assert!(
        diag.ok,
        "DshTako diagnose must remain healthy with web_page=true, issues={:?}",
        diag.issues
    );
    let creds_after = fs::read_to_string(&creds_path).unwrap();
    assert!(
        creds_after.contains("OTHER_TOKEN: keep_me_intact"),
        "DshTako must preserve unrelated credential keys"
    );
    DshTakoAdapter
        .apply_mode(&paths, AppMode::Official)
        .expect("DshTako apply_mode(Official) must succeed");

    // TC-EO-08: detect_node_runtime() structural contract & invariants (must never panic)
    let runtime = detect_node_runtime();
    if !runtime.available {
        assert!(
            !runtime.meets_min_version,
            "meets_min_version must be false when node runtime is unavailable"
        );
    }
    if runtime.meets_min_version {
        assert!(
            runtime.available,
            "available must be true when meets_min_version is true"
        );
        assert!(
            runtime.major.is_some_and(|m| m >= 18),
            "meets_min_version requires major >= 18, got {:?}",
            runtime.major
        );
    }
    if let Some(ref ver) = runtime.version {
        assert!(!ver.trim().is_empty(), "version must not be empty when Some");
    }

    let serialized = serde_json::to_value(&runtime).expect("NodeRuntimeInfo must serialize");
    let obj = serialized.as_object().expect("NodeRuntimeInfo must serialize as JSON object");
    for required_key in [
        "available",
        "version",
        "major",
        "meetsMinVersion",
        "nodePath",
        "npxPath",
    ] {
        assert!(
            obj.contains_key(required_key),
            "NodeRuntimeInfo JSON missing camelCase field '{required_key}': {serialized}"
        );
    }
}

// ============================================================================
// Module B: Antigravity Full Chinese I18n Dictionary & ASAR Engine (TC-AGY-*)
// ============================================================================

fn load_antigravity_zh_dict() -> serde_json::Map<String, Value> {
    let dict_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("adapters")
        .join("antigravity_zh_dict.json");
    assert!(
        dict_path.is_file(),
        "missing standalone Antigravity Chinese dictionary at {}",
        dict_path.display()
    );
    let raw = fs::read_to_string(&dict_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dict_path.display()));
    let val: Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("{} must be valid JSON: {e}", dict_path.display()));
    val.as_object()
        .cloned()
        .expect("antigravity_zh_dict.json root must be a JSON object")
}

#[test]
fn tc_agy_01_antigravity_zh_dict_json_integrity_and_coverage() {
    let dict = load_antigravity_zh_dict();
    assert!(
        dict.len() >= 1200,
        "antigravity_zh_dict.json must contain >= 1200 entries, found {}",
        dict.len()
    );

    let mut cjk_count = 0usize;
    for (k, v) in &dict {
        assert!(!k.trim().is_empty(), "dictionary contains empty/whitespace key");
        let zh = v
            .as_str()
            .unwrap_or_else(|| panic!("dictionary value for '{k}' must be a string, got {v}"));
        assert!(
            !zh.trim().is_empty(),
            "dictionary value for key '{k}' must not be empty/whitespace"
        );
        if contains_cjk(zh) {
            cjk_count += 1;
        }
    }

    let cjk_ratio = (cjk_count as f64) / (dict.len() as f64);
    assert!(
        cjk_ratio >= 0.90,
        "at least 90% of dictionary values should contain CJK characters, got {cjk_count}/{} ({:.1}%)",
        dict.len(),
        cjk_ratio * 100.0
    );

    let required_core_keys = [
        "New Conversation",
        "Scheduled Tasks",
        "Skills & Customizations",
        "Global Settings",
        "MCP Servers",
    ];
    for key in required_core_keys {
        let zh = dict
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing required core dictionary key '{key}'"));
        assert!(
            contains_cjk(zh),
            "core dictionary key '{key}' must translate to Chinese, got '{zh}'"
        );
    }
}

#[test]
fn tc_agy_02_to_06_antigravity_full_i18n_patch_idempotency_restore_and_upgrade() {
    let dict = load_antigravity_zh_dict();
    let (tmp, mut paths) = setup_sandbox();
    let fake_app = tmp.path().join("FakeApps").join("Antigravity.app");
    let (asar_path, plist_path, orig_asar_bytes, orig_plist_bytes, orig_header_sha256) =
        write_fake_antigravity_bundle(&fake_app, "2.2.0");
    let orig_asar_sha256 = sha256_hex(&orig_asar_bytes);
    paths.agy_app_override_dir = Some(fake_app.clone());

    // Pre-check: clean bundle is Unpatched
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Unpatched
    );

    // 1) First patch
    patch_antigravity_ui_for_paths(&paths).expect("first patch_antigravity_ui_for_paths");
    let patched_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(patched_info.status, UiPatchStatus::Patched);
    assert!(
        patched_info.has_valid_backup,
        "Patched state must report has_valid_backup == true"
    );

    let patched_asar_bytes = fs::read(&asar_path).unwrap();
    let parsed = parse_and_verify_asar_pickle(&patched_asar_bytes);
    assert_ne!(
        parsed.header_sha256, orig_header_sha256,
        "patched ASAR header SHA-256 must change"
    );

    // Verify `unpacked: true` native module node preserved
    let unpacked_node =
        lookup_asar_node(&parsed.header, &["node_modules", "node-pty", "pty.node"])
            .expect("unpacked native module node must be preserved");
    assert_eq!(
        unpacked_node.get("unpacked").and_then(Value::as_bool),
        Some(true)
    );
    assert!(unpacked_node.get("offset").is_none());

    // Verify Info.plist ElectronAsarIntegrity hash updated
    let updated_plist = fs::read_to_string(&plist_path).unwrap();
    assert!(updated_plist.contains(&parsed.header_sha256));

    // Inspect injected dist/preload.js
    let preload_raw =
        String::from_utf8(extract_packed_asar_file(&parsed, &["dist", "preload.js"]))
            .expect("dist/preload.js must be valid UTF-8");
    let preload_decoded = decode_js_unicode_escapes(&preload_raw);
    let preload_lower = preload_decoded.to_ascii_lowercase();

    assert_eq!(
        preload_raw.matches(PATCH_GUARD_COMMENT).count(),
        1,
        "dist/preload.js must contain exactly one PATCH_GUARD_COMMENT"
    );

    // Must embed standalone dictionary entries (both keys and their actual Chinese translations)
    for core_key in [
        "New Conversation",
        "Scheduled Tasks",
        "Skills & Customizations",
        "Global Settings",
        "MCP Servers",
    ] {
        let expected_zh = dict.get(core_key).and_then(Value::as_str).unwrap();
        assert!(
            preload_decoded.contains(core_key) && preload_decoded.contains(expected_zh),
            "dist/preload.js must embed dictionary entry '{core_key}' -> '{expected_zh}'"
        );
    }
    assert!(
        preload_raw.len() > 25_000,
        "dist/preload.js should embed the >= 1200-entry standalone dictionary, got only {} bytes",
        preload_raw.len()
    );

    // Must include dynamic regex template rules (e.g. Thinking for / files changed / ago),
    // allowing standard regex spacing/quantifiers such as `\s+`, `(Thinking|Thought)`, or `files?`.
    let has_thinking_rule = preload_lower.contains("thinking for")
        || preload_lower.contains("thinking\\s+for")
        || (preload_lower.contains("thinking")
            && preload_lower.contains("thought")
            && preload_lower.contains("for"));
    assert!(
        has_thinking_rule,
        "dist/preload.js must include dynamic regex rule for 'Thinking for' / 'Thought for'"
    );
    let has_files_changed_rule = preload_lower.contains("files changed")
        || preload_lower.contains("file changed")
        || preload_lower.contains("files?\\s+changed")
        || preload_lower.contains("files? changed")
        || (preload_lower.contains("files?") && preload_lower.contains("changed"));
    assert!(
        has_files_changed_rule,
        "dist/preload.js must include dynamic regex rule for 'files changed'"
    );
    assert!(
        preload_lower.contains("ago"),
        "dist/preload.js must include dynamic regex rule for relative time ('ago')"
    );

    // Must support Shadow DOM and localhost/127.0.0.1 host guard
    assert!(
        preload_decoded.contains("shadowRoot"),
        "dist/preload.js must traverse open Shadow DOM (shadowRoot)"
    );
    assert!(
        preload_decoded.contains("127.0.0.1") && preload_decoded.contains("localhost"),
        "dist/preload.js must retain 127.0.0.1/localhost origin guard"
    );

    // Must include code/editor/input safety skip guards
    for guard_token in [
        "monaco-editor",
        "xterm",
        "CODE",
        "PRE",
        "SCRIPT",
        "STYLE",
        "TEXTAREA",
        "INPUT",
    ] {
        assert!(
            preload_decoded.contains(guard_token),
            "dist/preload.js missing mandatory skip guard '{guard_token}'"
        );
    }

    // 2) Idempotency: second patch call must not duplicate guards or corrupt archive
    patch_antigravity_ui_for_paths(&paths).expect("second (idempotent) patch call");
    let reparsed = parse_and_verify_asar_pickle(&fs::read(&asar_path).unwrap());
    let preload_after_second =
        String::from_utf8(extract_packed_asar_file(&reparsed, &["dist", "preload.js"])).unwrap();
    assert_eq!(
        preload_after_second, preload_raw,
        "idempotent second patch must keep dist/preload.js identical"
    );
    for rel in [
        &["dist", "preload.js"][..],
        &["dist", "menu.js"][..],
        &["dist", "tray.js"][..],
        &["dist", "updater.js"][..],
    ] {
        let content = String::from_utf8(extract_packed_asar_file(&reparsed, rel)).unwrap();
        assert_eq!(
            content.matches(PATCH_GUARD_COMMENT).count(),
            1,
            "{:?} must contain exactly one PATCH_GUARD_COMMENT after idempotent re-patch",
            rel
        );
    }

    // 3) 100% byte-for-byte & SHA-256 restore
    restore_antigravity_ui_for_paths(&paths).expect("restore_antigravity_ui_for_paths");
    let restored_asar_bytes = fs::read(&asar_path).unwrap();
    assert_eq!(
        sha256_hex(&restored_asar_bytes),
        orig_asar_sha256,
        "restored app.asar SHA-256 must match original backup 100%"
    );
    assert_eq!(
        restored_asar_bytes, orig_asar_bytes,
        "restored app.asar bytes must match original backup byte-for-byte"
    );
    assert_eq!(
        fs::read(&plist_path).unwrap(),
        orig_plist_bytes,
        "restored Info.plist must match original backup byte-for-byte"
    );
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Unpatched
    );

    // 4) Upgrade awareness: patch 2.2.0 -> overwrite with clean 2.3.0 -> OutdatedAfterUpgrade -> re-patch -> restore 2.3.0
    patch_antigravity_ui_for_paths(&paths).expect("patch 2.2.0 before upgrade");
    let (_, _, v230_asar_bytes, v230_plist_bytes, _) =
        write_fake_antigravity_bundle(&fake_app, "2.3.0");
    let v230_sha256 = sha256_hex(&v230_asar_bytes);
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    patch_antigravity_ui_for_paths(&paths).expect("re-patch 2.3.0 after upgrade");
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Patched
    );

    restore_antigravity_ui_for_paths(&paths).expect("restore clean 2.3.0");
    let restored_v230 = fs::read(&asar_path).unwrap();
    assert_eq!(sha256_hex(&restored_v230), v230_sha256);
    assert_eq!(restored_v230, v230_asar_bytes);
    assert_eq!(fs::read(&plist_path).unwrap(), v230_plist_bytes);
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Unpatched
    );
}
