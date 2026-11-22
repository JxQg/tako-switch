use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tako_switch_lib::adapters::antigravity::AntigravityAdapter;
use tako_switch_lib::adapters::claude::ClaudeAdapter;
use tako_switch_lib::adapters::codex::CodexAdapter;
use tako_switch_lib::apply_prefer_chinese_ui_for_paths;
use tako_switch_lib::config::{read_switch_state, AppPaths};
use tempfile::TempDir;
use toml_edit::DocumentMut;

mod sym {
    #[allow(ambiguous_glob_reexports, unused_imports)]
    pub use tako_switch_lib::adapters::*;
    #[allow(ambiguous_glob_reexports, unused_imports)]
    pub use tako_switch_lib::adapters::claude_i18n::*;
}

use sym::{
    classify_io_error_for_os, format_ui_patch_block_error, get_adapter, inspect_claude_ui_patch,
    inspect_electron_asar_integrity_fuse, parse_ui_patch_block_error, patch_claude_ui_for_paths,
    patch_windows_pe_asar_integrity, restore_claude_ui_for_paths, snapshot_app, AppId, AppMode,
    UiPatchBlockKind, UiPatchInfo, UiPatchPermissionPrompt, UiPatchStatus,
    CLAUDE_PATCH_GUARD_COMMENT, UI_PATCH_BLOCK_PREFIX,
};

const BLOCK_START: &str = "<!-- tako-switch:chinese-ui:start -->";
const BLOCK_END: &str = "<!-- tako-switch:chinese-ui:end -->";
const ASAR_BLOCK_SIZE: usize = 4 * 1024 * 1024;
const ELECTRON_FUSE_SENTINEL: &[u8; 32] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";

const SHELL_MISSING_8_KEYS: &[(&str, &str)] = &[
    (
        "3KbALjoPbx",
        "Claude needs to fix file ownership so auto-updates can work correctly.",
    ),
    (
        "DLwCsMd5iw",
        "list the sites you\u{2019}re signed into in {browser}",
    ),
    (
        "KWmT+sHQrn",
        "sign in to {host} as \u{201c}{login}\u{201d}",
    ),
    (
        "LEublMzjXI",
        "Claude will {reason}. macOS may also ask for your login password to unlock that browser\u{2019}s saved data.",
    ),
    (
        "PbhDLsWQf4",
        "list the sites you\u{2019}re signed into in {browser}, to bring them into Cowork\u{2019}s browser",
    ),
    ("QoBk0vsDtW", "sign in to {host}"),
    ("dnkjLC8+rK", "verify your identity on $1"),
    ("wNsAXQmnVH", "Claude will {reason}."),
];

const DYNAMIC_51_KEYS: &[(&str, &str)] = &[
    ("0gtp5o1nEE", "Complex, detailed work"),
    (
        "29OLd9bgWw",
        "Opus consumes usage limits faster than other models",
    ),
    ("2EcGZn58SU", "Auto thinking"),
    ("2qTZeZCPue", "Analysis tool"),
    ("4EGZ/Kv9qH", "Most efficient for everyday tasks"),
    ("4N8a6wReLf", "Best for math and coding challenges"),
    ("4RgMF5iUDm", "Extended"),
    ("6EhWLLZwj4", "Enable fast mode"),
    ("6eNkKSj+Er", "Web search"),
    ("8iuHf335Ey", "Always uses deep reasoning"),
    ("B4Nw7coGbc", "Adaptive"),
    ("BDqDM65MnU", "Adaptive thinking"),
    ("DvRKnnUSOG", "Thinking"),
    ("EGP6HJPOg+", "Most efficient for simpler tasks"),
    ("EpijVevLzK", "Fast mode"),
    (
        "GoDyM43G4X",
        "Claude uses its access to the web to improve answers when appropriate.",
    ),
    ("HoZQmASaw3", "Extra"),
    ("LCyVJcnXTZ", "Most capable for ambitious work"),
    ("MnpOJUobxY", "Thinks for more complex tasks"),
    (
        "N+VJI9Z0SU",
        "Upload CSVs for Claude to analyze quantitative data with high accuracy and create interactive data visualizations.",
    ),
    ("NKydG2KArY", "Extended"),
    ("PLBs+fqKwX", "Off"),
    ("QQ2UzmdmNN", "Light, casual tasks"),
    ("QimMWHgE5g", "Respond right away"),
    (
        "RaZ/cp/eey",
        "May use excessive tokens resulting in long response times and may hit token limits. Use sparingly for the hardest tasks.",
    ),
    ("U0mtsR78v2", "Medium"),
    ("XBURx2a572", "For complex work and everyday tasks"),
    ("XPiJIxyAVt", "Recommended"),
    ("Xads8RoR+/", "Default"),
    ("Xft0R8xbIB", "Low"),
    ("Yrtr74Rv+h", "For your toughest challenges"),
    ("Zk4EtKw26K", "Max"),
    ("Ztf14WwmNM", "Balanced for everyday work"),
    ("Zyu/cxL2/B", "Uses usage credits"),
    ("aWuhH8qI9/", "Think longer for complex tasks"),
    ("c42Iq1UBqb", "Extended"),
    (
        "dapWFrcWv4",
        "Higher effort means more thorough responses, but takes longer and uses your limits faster.",
    ),
    ("eAZhflTduv", "Match thinking to complexity"),
    ("g8BKYKpvDv", "Can think for more complex tasks"),
    ("gRa9I5BFc2", "Opus draws down usage faster than other models"),
    (
        "hahKOhzAYF",
        "Claude can search the internet to provide more up-to-date and relevant responses. Claude will automatically determine when to use web search if the topic requires current information. Web search is only available when using Claude 3.7 Sonnet.",
    ),
    ("mQRd+O9jEI", "Org default"),
    ("n6XZvuhMVT", "Fastest for quick answers"),
    ("oAk189A0OS", "Fast"),
    ("oLD1YKsL8P", "The hardest problems. Takes longest."),
    (
        "sA9kR2u6VC",
        "Claude can write and run code to process data, run analysis, and produce data visualizations in real time.",
    ),
    ("u8fFNy1Mu4", "Instant"),
    ("u8n5HzNZ2R", "Extended thinking"),
    ("vv9L/MN3/H", "High"),
    ("w+qTzeepxX", "Best for most use cases"),
    ("zdSx1K4VL2", "Quick replies to simple questions"),
];

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn contains_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

fn unescape_js_unicode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 6 <= bytes.len() && bytes[i] == b'\\' && bytes[i + 1] == b'u' {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 2..i + 6]) {
                if let Ok(code) = u32::from_str_radix(hex_str, 16) {
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                        i += 6;
                        continue;
                    }
                }
            }
        }
        let ch = raw[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn extract_top_level_icu_vars(text: &str) -> Result<BTreeSet<String>, String> {
    let mut vars = BTreeSet::new();
    let mut depth = 0i32;
    let mut top_start: Option<usize> = None;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    top_start = Some(idx + 1);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!("unbalanced closing brace in: {text}"));
                }
                if depth == 0 {
                    if let Some(start) = top_start.take() {
                        let inner = &text[start..idx];
                        let name = inner
                            .split([',', '}'])
                            .next()
                            .unwrap_or("")
                            .trim();
                        if !name.is_empty() {
                            vars.insert(name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(format!("unclosed brace (depth={depth}) in: {text}"));
    }
    Ok(vars)
}

fn setup_sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create v2.0.9 sandbox tempdir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(
        paths.is_test,
        "all v2.0.9 blackbox tests must run with paths.is_test == true"
    );
    (tmp, paths)
}

struct ChmodRestoreGuard {
    dirs: Vec<PathBuf>,
}

impl ChmodRestoreGuard {
    fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    fn set_mode(&self, mode: u32) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for dir in &self.dirs {
                if dir.exists() {
                    fs::set_permissions(dir, fs::Permissions::from_mode(mode))
                        .unwrap_or_else(|e| panic!("chmod {mode:#o} on {}: {e}", dir.display()));
                }
            }
        }
        #[cfg(not(unix))]
        let _ = mode;
    }

    fn restore_writable(&self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for dir in &self.dirs {
                if dir.exists() {
                    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
                }
            }
        }
    }
}

impl Drop for ChmodRestoreGuard {
    fn drop(&mut self) {
        self.restore_writable();
    }
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

fn pack_asar_from_header_and_payload(header_value: &Value, payload: &[u8]) -> (Vec<u8>, String) {
    let header_json = serde_json::to_string(header_value).expect("serialize ASAR header JSON");
    let header_bytes = header_json.as_bytes();
    let header_sha256 = sha256_hex(header_bytes);

    let json_len = header_bytes.len() as u32;
    let padded_len = (json_len + 3) & !3;
    let u0: u32 = 4;
    let u1: u32 = 8 + padded_len;
    let u2: u32 = 4 + padded_len;
    let strlen: u32 = json_len;

    let mut asar = Vec::with_capacity(16 + padded_len as usize + payload.len());
    asar.extend_from_slice(&u0.to_le_bytes());
    asar.extend_from_slice(&u1.to_le_bytes());
    asar.extend_from_slice(&u2.to_le_bytes());
    asar.extend_from_slice(&strlen.to_le_bytes());
    asar.extend_from_slice(header_bytes);
    asar.resize(16 + padded_len as usize, 0u8);
    asar.extend_from_slice(payload);

    (asar, header_sha256)
}

fn build_clean_claude_app_asar(app_version: &str) -> (Vec<u8>, String) {
    let mut payload = Vec::new();

    let pkg_json = serde_json::to_string_pretty(&json!({
        "name": "claude-desktop",
        "productName": "Claude",
        "version": app_version,
        "main": ".vite/build/index.js"
    }))
    .unwrap();

    let mut shell_en = Map::new();
    shell_en.insert("ShSettings01".to_string(), json!("Settings"));
    shell_en.insert("ShRestart002".to_string(), json!("Restart Claude"));
    shell_en.insert(
        "ShWorking003".to_string(),
        json!("Claude is still working"),
    );
    shell_en.insert(
        "ShAllowTool4".to_string(),
        json!("Allow Claude to use {toolName}?"),
    );
    shell_en.insert("ShWindow0005".to_string(), json!("Window"));
    shell_en.insert("ShReload0006".to_string(), json!("Reload"));
    for &(k, v) in SHELL_MISSING_8_KEYS {
        shell_en.insert(k.to_string(), json!(v));
    }

    // Partial zh-Hans.json: has existing shell translations, intentionally missing SHELL_MISSING_8_KEYS
    let mut shell_zh = Map::new();
    shell_zh.insert("ShSettings01".to_string(), json!("设置"));
    shell_zh.insert("ShRestart002".to_string(), json!("重启 Claude"));
    shell_zh.insert("ShWorking003".to_string(), json!("Claude 仍在处理中"));
    shell_zh.insert(
        "ShAllowTool4".to_string(),
        json!("允许 Claude 使用 {toolName} 吗？"),
    );
    shell_zh.insert("ShWindow0005".to_string(), json!("窗口"));
    shell_zh.insert("ShReload0006".to_string(), json!("重新加载"));

    let index_js = format!(
        "// Claude Desktop {app_version} main entry\nrequire(\"./index.chunk-abc123.js\");\n"
    );
    let index_chunk_js = format!(
        "// Claude Desktop {app_version} DesktopIntl chunk\nconst QVe={{en:\"en-US\",de:\"de-DE\",fr:\"fr-FR\",ko:\"ko-KR\",ja:\"ja-JP\",\"es-419\":\"es-419\",es:\"es-ES\",it:\"it-IT\",hi:\"hi-IN\",pt:\"pt-BR\",id:\"id-ID\"}};\nfunction aHe(){{return[\"en-US.json\",\"zh-Hans.json\",\"es-419.json\"]}}\nfunction sHe(){{let e=aHe(),t=/[a-z]{{2}}-[A-Z]{{2}}/;return e.filter((e=>t.test(e)))}}\nmodule.exports={{QVe,sHe}};\n"
    );

    let pkg_node = make_asar_file_entry(pkg_json.as_bytes(), &mut payload);
    let en_node = make_asar_file_entry(
        serde_json::to_string_pretty(&Value::Object(shell_en))
            .unwrap()
            .as_bytes(),
        &mut payload,
    );
    let zh_node = make_asar_file_entry(
        serde_json::to_string_pretty(&Value::Object(shell_zh))
            .unwrap()
            .as_bytes(),
        &mut payload,
    );
    let index_node = make_asar_file_entry(index_js.as_bytes(), &mut payload);
    let chunk_node = make_asar_file_entry(index_chunk_js.as_bytes(), &mut payload);

    let header_value = json!({
        "files": {
            "package.json": pkg_node,
            "resources": {
                "files": {
                    "i18n": {
                        "files": {
                            "en-US.json": en_node,
                            "zh-Hans.json": zh_node
                        }
                    }
                }
            },
            ".vite": {
                "files": {
                    "build": {
                        "files": {
                            "index.js": index_node,
                            "index.chunk-abc123.js": chunk_node
                        }
                    }
                }
            },
            "node_modules": {
                "files": {
                    "@ant": {
                        "files": {
                            "claude-native": {
                                "files": {
                                    "claude-native-binding.node": {
                                        "size": 8192,
                                        "unpacked": true
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    pack_asar_from_header_and_payload(&header_value, &payload)
}

fn build_clean_claude_ion_asar(
    app_version: &str,
    include_multiblock_blob: bool,
) -> (Vec<u8>, String) {
    let mut payload = Vec::new();

    let index_html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Claude {app_version}</title></head><body><div id=\"root\"></div><script type=\"module\" src=\"/assets/v1/c2c114516-test.js\"></script></body></html>"
    );

    let ion_en_entries: &[(&str, &str)] = &[
        // Chat
        ("ChatNew001", "New chat"),
        ("ChatAll002", "All chats"),
        ("ChatAsk003", "Ask Claude\u{2026}"),
        ("ChatPrj004", "New chat in {projectName}"),
        // Claude Code desktop workbench
        ("CodeSes001", "Sessions"),
        ("CodeRep002", "Repositories"),
        ("CodeBrn003", "Branches"),
        ("CodeWkt004", "Worktrees"),
        ("CodePrm005", "Permissions"),
        ("CodeAcc006", "Accept edits"),
        ("CodeAlw007", "Always ask before making changes"),
        // Cowork
        ("CowkSch001", "Scheduled tasks"),
        ("CowkTsk002", "Cowork tasks"),
        (
            "CowkCon003",
            "Connect Claude to your favorite apps and data sources.",
        ),
        // Settings
        ("SetMain001", "Settings"),
        ("SetApr002", "Appearance"),
        ("SetLng003", "Language"),
        ("SetAcc004", "Account"),
        ("SetUsg005", "Usage"),
        ("SetCon006", "Connectors"),
        ("SetEml007", "Logged in as {email}"),
        // Overlap with app.asar shell official zh-Hans
        ("OvlAllow01", "Allow Claude to use {toolName}?"),
        ("OvlWork002", "Claude is still working"),
        // ICU plural & multi-placeholder strings
        (
            "IcuPlrRun1",
            "{count, plural, one {# running} other {# running}}",
        ),
        (
            "IcuPlrFld2",
            "{count, plural, one {# failed} other {# failed}}",
        ),
        ("IcuArcCnt3", "Archive all ({count})"),
        ("IcuNamEml4", "{name} ({email})"),
        // Unknown future English string to verify 100% key parity fallback + placeholder preservation
        (
            "FutUnkKey9",
            "Experimental telemetry channel for {deviceId}",
        ),
    ];

    let mut ion_en = Map::new();
    for &(k, v) in ion_en_entries {
        ion_en.insert(k.to_string(), json!(v));
    }

    let mut dynamic_en = Map::new();
    assert_eq!(DYNAMIC_51_KEYS.len(), 51, "fixture must include all 51 dynamic keys");
    for &(k, v) in DYNAMIC_51_KEYS {
        dynamic_en.insert(k.to_string(), json!(v));
    }

    let shared_73_js = format!(
        "// ion-dist {app_version} shared-73\nexport const SUPPORTED_LOCALES=[\"en-US\",\"de-DE\",\"fr-FR\",\"ko-KR\",\"ja-JP\",\"es-419\",\"es-ES\",\"it-IT\",\"hi-IN\",\"pt-BR\",\"id-ID\"];\n"
    );
    let shared_107_js = format!(
        "// ion-dist {app_version} shared-107\nexport const DYNAMIC_LOCALES=[\"en-US\", \"de-DE\", \"fr-FR\", \"ko-KR\", \"ja-JP\", \"es-419\", \"es-ES\", \"it-IT\", \"hi-IN\", \"pt-BR\", \"id-ID\"];\n"
    );
    let c2c_main_js = format!(
        "// ion-dist {app_version} main bundle\nconst UI_LOCALES=[\"en-US\",\"de-DE\",\"fr-FR\",\"ko-KR\",\"ja-JP\",\"es-419\",\"es-ES\",\"it-IT\",\"hi-IN\",\"pt-BR\",\"id-ID\"];\nconst LOCALE_DISPLAY_MAP={{\"en-US\":{{name:\"English (United States)\",localName:\"English (United States)\"}},\"de-DE\":{{name:\"German (Germany)\",localName:\"Deutsch (Deutschland)\"}},\"fr-FR\":{{name:\"French (France)\",localName:\"Fran\\u00e7ais (France)\"}},\"ko-KR\":{{name:\"Korean (South Korea)\",localName:\"\\ud55c\\uad6d\\uc5b4 (\\ub300\\ud55c\\ubbfc\\uad6d)\"}},\"ja-JP\":{{name:\"Japanese (Japan)\",localName:\"\\u65e5\\u672c\\u8a9e (\\u65e5\\u672c)\"}},\"es-419\":{{name:\"Spanish (Latin America)\",localName:\"Espa\\u00f1ol (Latinoam\\u00e9rica)\"}},\"es-ES\":{{name:\"Spanish (Spain)\",localName:\"Espa\\u00f1ol (Espa\\u00f1a)\"}},\"it-IT\":{{name:\"Italian (Italy)\",localName:\"Italiano (Italia)\"}},\"hi-IN\":{{name:\"Hindi (India)\",localName:\"\\u0939\\u093f\\u0928\\u094d\\u0926\\u0940 (\\u092d\\u093e\\u0930\\u0924)\"}},\"pt-BR\":{{name:\"Portuguese (Brazil)\",localName:\"Portugu\\u00eas (Brasil)\"}},\"id-ID\":{{name:\"Indonesian (Indonesia)\",localName:\"Indonesia (Indonesia)\"}}}};\nexport{{UI_LOCALES,LOCALE_DISPLAY_MAP}};\n"
    );

    let index_html_node = make_asar_file_entry(index_html.as_bytes(), &mut payload);
    let ion_en_node = make_asar_file_entry(
        serde_json::to_string_pretty(&Value::Object(ion_en))
            .unwrap()
            .as_bytes(),
        &mut payload,
    );
    let dyn_en_node = make_asar_file_entry(
        serde_json::to_string_pretty(&Value::Object(dynamic_en))
            .unwrap()
            .as_bytes(),
        &mut payload,
    );
    let s73_node = make_asar_file_entry(shared_73_js.as_bytes(), &mut payload);
    let s107_node = make_asar_file_entry(shared_107_js.as_bytes(), &mut payload);
    let c2c_node = make_asar_file_entry(c2c_main_js.as_bytes(), &mut payload);

    let mut v1_files = Map::new();
    v1_files.insert("shared-73-test.js".to_string(), s73_node);
    v1_files.insert("shared-107-test.js".to_string(), s107_node);
    v1_files.insert("c2c114516-test.js".to_string(), c2c_node);
    if include_multiblock_blob {
        // 4 MiB + 2048 bytes -> spans 2 SHA-256 4MB integrity blocks
        let mut blob = vec![0x5Au8; ASAR_BLOCK_SIZE + 2048];
        blob[0..8].copy_from_slice(b"CLAUDE29");
        blob[ASAR_BLOCK_SIZE..ASAR_BLOCK_SIZE + 8].copy_from_slice(b"BLOCKTWO");
        let blob_node = make_asar_file_entry(&blob, &mut payload);
        v1_files.insert("wasm-worker-bundle.bin".to_string(), blob_node);
    }

    let header_value = json!({
        "files": {
            "index.html": index_html_node,
            "i18n": {
                "files": {
                    "en-US.json": ion_en_node,
                    "dynamic": {
                        "files": {
                            "en-US.json": dyn_en_node
                        }
                    }
                }
            },
            "assets": {
                "files": {
                    "v1": {
                        "files": Value::Object(v1_files)
                    },
                    "native": {
                        "files": {
                            "ion-helper.node": {
                                "size": 4096,
                                "unpacked": true
                            }
                        }
                    }
                }
            }
        }
    });

    pack_asar_from_header_and_payload(&header_value, &payload)
}

fn render_claude_macos_info_plist(
    app_version: &str,
    app_header_sha256: &str,
    ion_header_sha256: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.anthropic.claudefordesktop</string>
    <key>CFBundleName</key>
    <string>Claude</string>
    <key>CFBundleShortVersionString</key>
    <string>{app_version}</string>
    <key>ElectronAsarIntegrity</key>
    <dict>
        <key>Resources/app.asar</key>
        <dict>
            <key>algorithm</key>
            <string>SHA256</string>
            <key>hash</key>
            <string>{app_header_sha256}</string>
        </dict>
        <key>Resources/ion-dist.asar</key>
        <dict>
            <key>algorithm</key>
            <string>SHA256</string>
            <key>hash</key>
            <string>{ion_header_sha256}</string>
        </dict>
    </dict>
</dict>
</plist>
"#
    )
}

struct ClaudeBundleFixture {
    app_asar_path: PathBuf,
    ion_asar_path: PathBuf,
    info_plist_path: PathBuf,
    macos_bin_path: PathBuf,
    code_resources_path: PathBuf,
    orig_app_asar: Vec<u8>,
    orig_ion_asar: Vec<u8>,
    orig_info_plist: Vec<u8>,
    orig_macos_bin: Vec<u8>,
    orig_code_resources: Vec<u8>,
    orig_app_header_sha256: String,
    orig_ion_header_sha256: String,
}

fn write_fake_claude_macos_bundle(
    app_dir: &Path,
    app_version: &str,
    include_multiblock_blob: bool,
) -> ClaudeBundleFixture {
    let contents_dir = app_dir.join("Contents");
    let resources_dir = contents_dir.join("Resources");
    let macos_dir = contents_dir.join("MacOS");
    let codesig_dir = contents_dir.join("_CodeSignature");
    fs::create_dir_all(&resources_dir).unwrap();
    fs::create_dir_all(&macos_dir).unwrap();
    fs::create_dir_all(&codesig_dir).unwrap();

    let (orig_app_asar, orig_app_header_sha256) = build_clean_claude_app_asar(app_version);
    let (orig_ion_asar, orig_ion_header_sha256) =
        build_clean_claude_ion_asar(app_version, include_multiblock_blob);
    let orig_info_plist = render_claude_macos_info_plist(
        app_version,
        &orig_app_header_sha256,
        &orig_ion_header_sha256,
    )
    .into_bytes();
    let mut orig_macos_bin = vec![0xCF, 0xFA, 0xED, 0xFE];
    orig_macos_bin.extend_from_slice(format!("claude-macho-stub-{app_version}").as_bytes());
    let orig_code_resources = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>version</key><string>{app_version}</string></dict></plist>\n"
    )
    .into_bytes();

    let app_asar_path = resources_dir.join("app.asar");
    let ion_asar_path = resources_dir.join("ion-dist.asar");
    let info_plist_path = contents_dir.join("Info.plist");
    let macos_bin_path = macos_dir.join("Claude");
    let code_resources_path = codesig_dir.join("CodeResources");

    fs::write(&app_asar_path, &orig_app_asar).unwrap();
    fs::write(&ion_asar_path, &orig_ion_asar).unwrap();
    fs::write(&info_plist_path, &orig_info_plist).unwrap();
    fs::write(&macos_bin_path, &orig_macos_bin).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&macos_bin_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(&code_resources_path, &orig_code_resources).unwrap();

    ClaudeBundleFixture {
        app_asar_path,
        ion_asar_path,
        info_plist_path,
        macos_bin_path,
        code_resources_path,
        orig_app_asar,
        orig_ion_asar,
        orig_info_plist,
        orig_macos_bin,
        orig_code_resources,
        orig_app_header_sha256,
        orig_ion_header_sha256,
    }
}

fn build_clean_antigravity_bundle(app_dir: &Path, app_version: &str) {
    let contents_dir = app_dir.join("Contents");
    let resources_dir = contents_dir.join("Resources");
    fs::create_dir_all(&resources_dir).unwrap();

    let mut payload = Vec::new();
    let pkg = format!(r#"{{"name":"antigravity","version":"{app_version}","main":"dist/main.js"}}"#);
    let menu = "// menu\nmodule.exports = [{ label: \"File\" }, { label: \"View\" }];\n";
    let preload = "// preload\nwindow.__AGY__ = true;\n";
    let pkg_node = make_asar_file_entry(pkg.as_bytes(), &mut payload);
    let menu_node = make_asar_file_entry(menu.as_bytes(), &mut payload);
    let preload_node = make_asar_file_entry(preload.as_bytes(), &mut payload);
    let header_val = json!({
        "files": {
            "package.json": pkg_node,
            "dist": {
                "files": {
                    "menu.js": menu_node,
                    "preload.js": preload_node
                }
            }
        }
    });
    let (asar_bytes, header_sha) = pack_asar_from_header_and_payload(&header_val, &payload);
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.google.antigravity</string>
    <key>CFBundleShortVersionString</key>
    <string>{app_version}</string>
    <key>ElectronAsarIntegrity</key>
    <dict>
        <key>Resources/app.asar</key>
        <dict>
            <key>algorithm</key>
            <string>SHA256</string>
            <key>hash</key>
            <string>{header_sha}</string>
        </dict>
    </dict>
</dict>
</plist>
"#
    );
    fs::write(resources_dir.join("app.asar"), asar_bytes).unwrap();
    fs::write(contents_dir.join("Info.plist"), plist).unwrap();
}

struct ParsedAsar {
    header: Value,
    header_sha256: String,
    data_section: Vec<u8>,
}

fn verify_asar_tree_integrity(node: &Value, data_section: &[u8], prefix: &str) {
    let Some(files) = node.get("files").and_then(Value::as_object) else {
        return;
    };
    for (name, child) in files {
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if child.get("files").is_some() {
            verify_asar_tree_integrity(child, data_section, &rel);
            continue;
        }
        if child.get("unpacked").and_then(Value::as_bool) == Some(true) {
            assert!(
                child.get("offset").is_none(),
                "unpacked entry {rel} must NOT carry an offset field: {child}"
            );
            continue;
        }
        let size = child
            .get("size")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| panic!("{rel}: missing size")) as usize;
        let offset: usize = child
            .get("offset")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{rel}: missing string offset"))
            .parse()
            .unwrap_or_else(|e| panic!("{rel}: invalid offset: {e}"));
        assert!(
            offset + size <= data_section.len(),
            "{rel}: slice [{offset}..{}] exceeds ASAR data section ({})",
            offset + size,
            data_section.len()
        );
        let bytes = &data_section[offset..offset + size];
        let integrity = child
            .get("integrity")
            .unwrap_or_else(|| panic!("{rel}: packed file must include SHA256 integrity metadata"));
        assert_eq!(
            integrity.get("algorithm").and_then(Value::as_str),
            Some("SHA256"),
            "{rel}: integrity.algorithm must be SHA256"
        );
        assert_eq!(
            integrity.get("blockSize").and_then(Value::as_u64),
            Some(ASAR_BLOCK_SIZE as u64),
            "{rel}: integrity.blockSize must be 4 MiB"
        );
        let actual_hash = sha256_hex(bytes);
        let recorded_hash = integrity
            .get("hash")
            .and_then(Value::as_str)
            .expect("integrity.hash string")
            .to_ascii_lowercase();
        assert_eq!(recorded_hash, actual_hash, "{rel}: integrity.hash mismatch");

        let expected_blocks: Vec<String> = if bytes.is_empty() {
            vec![actual_hash]
        } else {
            bytes.chunks(ASAR_BLOCK_SIZE).map(sha256_hex).collect()
        };
        let recorded_blocks: Vec<String> = integrity
            .get("blocks")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{rel}: missing integrity.blocks array"))
            .iter()
            .map(|v| v.as_str().expect("block hash string").to_ascii_lowercase())
            .collect();
        if bytes.is_empty() && recorded_blocks.is_empty() {
            // Empty files with [] or [empty_sha256] are both valid in Electron ASAR
        } else {
            assert_eq!(
                recorded_blocks, expected_blocks,
                "{rel}: 4MB SHA-256 block integrity mismatch"
            );
        }
    }
}

fn parse_and_verify_asar_pickle(bytes: &[u8]) -> ParsedAsar {
    assert!(bytes.len() >= 16, "ASAR archive shorter than 16-byte Pickle header");
    let u0 = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let u1 = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let u2 = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let strlen = u32::from_le_bytes(bytes[12..16].try_into().unwrap());

    let padded_len = (strlen + 3) & !3;
    assert_eq!(u0, 4, "Pickle u0 must be 4");
    assert_eq!(u2, 4 + padded_len, "Pickle u2 must equal 4 + padded_len");
    assert_eq!(u1, 8 + padded_len, "Pickle u1 must equal 8 + padded_len");

    let data_offset = (16 + padded_len) as usize;
    let json_end = 16 + strlen as usize;
    assert!(bytes.len() >= data_offset, "ASAR truncated before data payload");
    for &b in &bytes[json_end..data_offset] {
        assert_eq!(b, 0, "ASAR Pickle alignment padding must be zero bytes");
    }

    let header_slice = &bytes[16..json_end];
    let header_sha256 = sha256_hex(header_slice);
    let header: Value =
        serde_json::from_slice(header_slice).expect("ASAR header must be valid UTF-8 JSON");
    let data_section = bytes[data_offset..].to_vec();

    verify_asar_tree_integrity(&header, &data_section, "");

    ParsedAsar {
        header,
        header_sha256,
        data_section,
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
    let size = node.get("size").and_then(Value::as_u64).unwrap() as usize;
    let offset: usize = node
        .get("offset")
        .and_then(Value::as_str)
        .unwrap()
        .parse()
        .unwrap();
    asar.data_section[offset..offset + size].to_vec()
}

fn extract_plist_asar_hash(plist_xml: &str, resource_key: &str) -> Option<String> {
    let key_tag = format!("<key>{resource_key}</key>");
    let key_pos = plist_xml.find(&key_tag)?;
    let after_key = &plist_xml[key_pos + key_tag.len()..];
    let dict_end = after_key.find("</dict>")?;
    let dict_body = &after_key[..dict_end];
    let hash_pos = dict_body.find("<key>hash</key>")?;
    let after_hash = &dict_body[hash_pos + "<key>hash</key>".len()..];
    let s_open = after_hash.find("<string>")? + "<string>".len();
    let s_close = after_hash[s_open..].find("</string>")?;
    Some(after_hash[s_open..s_open + s_close].trim().to_string())
}

/// Construct a valid synthetic PE32+ (`0x20b`) executable image with:
/// - `MZ` header at `0x00` and `e_lfanew = 0x80`
/// - `PE\0\0` + 20-byte `IMAGE_FILE_HEADER` + 240-byte `IMAGE_OPTIONAL_HEADER64` (`opt_hdr = 0x98`)
/// - Non-zero `CheckSum` (`0xDEAD_BEEF` at `opt_hdr + 64`)
/// - `DataDirectory[4]` (`IMAGE_DIRECTORY_ENTRY_SECURITY` at `opt_hdr + 144`) pointing to a trailing 64-byte Authenticode blob
/// - Optional Electron FuseV1 sentinel (`enable_integrity_fuse: Option<bool>`)
/// - Embedded `ELECTRONASARINTEGRITY` JSON resource containing `app_hash` and `ion_hash`
fn build_synthetic_pe32_plus_exe(
    enable_integrity_fuse: Option<bool>,
    app_hash: &str,
    ion_hash: &str,
    uppercase_embedded_hashes: bool,
) -> (Vec<u8>, usize, usize) {
    const PE_OFFSET: usize = 0x80;
    const OPT_HDR_OFFSET: usize = PE_OFFSET + 24; // 0x98
    const OPT_HDR_SIZE: usize = 240; // 0xF0 for PE32+
    const SEC_HDR_OFFSET: usize = OPT_HDR_OFFSET + OPT_HDR_SIZE; // 0x188

    let mut exe = vec![0u8; 0x200];
    // DOS header
    exe[0..2].copy_from_slice(b"MZ");
    exe[0x3C..0x40].copy_from_slice(&(PE_OFFSET as u32).to_le_bytes());

    // NT headers: "PE\0\0"
    exe[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
    // COFF FileHeader
    exe[PE_OFFSET + 4..PE_OFFSET + 6].copy_from_slice(&0x8664u16.to_le_bytes()); // Machine = AMD64
    exe[PE_OFFSET + 6..PE_OFFSET + 8].copy_from_slice(&1u16.to_le_bytes()); // NumberOfSections = 1
    exe[PE_OFFSET + 20..PE_OFFSET + 22].copy_from_slice(&(OPT_HDR_SIZE as u16).to_le_bytes());
    exe[PE_OFFSET + 22..PE_OFFSET + 24].copy_from_slice(&0x0022u16.to_le_bytes());

    // OptionalHeader64 (PE32+ Magic = 0x020B)
    exe[OPT_HDR_OFFSET..OPT_HDR_OFFSET + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
    exe[OPT_HDR_OFFSET + 64..OPT_HDR_OFFSET + 68].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes()); // CheckSum
    exe[OPT_HDR_OFFSET + 108..OPT_HDR_OFFSET + 112].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes

    // Single .rdata section header (40 bytes)
    exe[SEC_HDR_OFFSET..SEC_HDR_OFFSET + 6].copy_from_slice(b".rdata");

    // Append .rdata body starting at 0x200
    if let Some(fuse_on) = enable_integrity_fuse {
        exe.extend_from_slice(ELECTRON_FUSE_SENTINEL);
        exe.push(1u8); // FuseV1 version
        exe.push(8u8); // 8 fuse slots
        let mut fuses = *b"11000000";
        fuses[4] = if fuse_on { b'1' } else { b'0' }; // index 4 = EnableEmbeddedAsarIntegrityValidation
        exe.extend_from_slice(&fuses);
    }

    let app_h = if uppercase_embedded_hashes {
        app_hash.to_ascii_uppercase()
    } else {
        app_hash.to_ascii_lowercase()
    };
    let ion_h = if uppercase_embedded_hashes {
        ion_hash.to_ascii_uppercase()
    } else {
        ion_hash.to_ascii_lowercase()
    };
    let integrity_rc = format!(
        r#"[{{"file":"resources\\app.asar","alg":"SHA256","value":"{app_h}"}},{{"file":"resources\\ion-dist.asar","alg":"SHA256","value":"{ion_h}"}}]"#
    );
    exe.extend_from_slice(b"\0\0ELECTRONASARINTEGRITY\0\0");
    exe.extend_from_slice(integrity_rc.as_bytes());
    while exe.len() % 8 != 0 {
        exe.push(0);
    }

    // Trailing WIN_CERTIFICATE Authenticode signature table (64 bytes)
    let cert_file_offset = exe.len();
    let cert_size: usize = 64;
    exe[OPT_HDR_OFFSET + 144..OPT_HDR_OFFSET + 148]
        .copy_from_slice(&(cert_file_offset as u32).to_le_bytes());
    exe[OPT_HDR_OFFSET + 148..OPT_HDR_OFFSET + 152]
        .copy_from_slice(&(cert_size as u32).to_le_bytes());

    let mut win_cert = vec![0xA5u8; cert_size];
    win_cert[0..4].copy_from_slice(&(cert_size as u32).to_le_bytes()); // dwLength
    win_cert[4..6].copy_from_slice(&0x0200u16.to_le_bytes()); // wRevision = WIN_CERT_REVISION_2_0
    win_cert[6..8].copy_from_slice(&0x0002u16.to_le_bytes()); // wCertificateType = PKCS_SIGNED_DATA
    exe.extend_from_slice(&win_cert);

    (exe, OPT_HDR_OFFSET, cert_file_offset)
}

fn read_json_object(path: &Path) -> Map<String, Value> {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let val: Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    val.as_object().cloned().expect("expected JSON object")
}

#[test]
fn tc_209_01_macos_claude_bundle_lifecycle_and_snapshot() {
    let (tmp, mut paths) = setup_sandbox();
    assert_eq!(env!("CARGO_PKG_VERSION"), "2.0.9");

    // 1) Non-existent Claude.app -> NotInstalled
    paths.claude_app_override_dir = Some(tmp.path().join("MissingClaude.app"));
    let not_installed: UiPatchInfo = inspect_claude_ui_patch(&paths);
    assert_eq!(not_installed.status, UiPatchStatus::NotInstalled);
    assert!(!not_installed.has_valid_backup);
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::NotInstalled
    );

    // 2) Broken bundle missing ion-dist.asar -> Unsupported
    let missing_ion_app = tmp.path().join("MissingIonClaude.app");
    fs::create_dir_all(missing_ion_app.join("Contents").join("Resources")).unwrap();
    let (only_app_asar, _) = build_clean_claude_app_asar("6.28899.6");
    fs::write(
        missing_ion_app
            .join("Contents")
            .join("Resources")
            .join("app.asar"),
        only_app_asar,
    )
    .unwrap();
    paths.claude_app_override_dir = Some(missing_ion_app);
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::Unsupported
    );

    // 3) Broken bundle missing app.asar -> Unsupported
    let missing_app_asar_dir = tmp.path().join("MissingAppAsarClaude.app");
    fs::create_dir_all(missing_app_asar_dir.join("Contents").join("Resources")).unwrap();
    let (only_ion_asar, _) = build_clean_claude_ion_asar("6.28899.6", false);
    fs::write(
        missing_app_asar_dir
            .join("Contents")
            .join("Resources")
            .join("ion-dist.asar"),
        only_ion_asar,
    )
    .unwrap();
    paths.claude_app_override_dir = Some(missing_app_asar_dir);
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::Unsupported
    );

    // 4) Full clean Claude.app -> Unpatched -> patch_claude_ui_for_paths -> Patched
    let claude_app = tmp.path().join("Apps").join("Claude.app");
    write_fake_claude_macos_bundle(&claude_app, "6.28899.6", false);
    paths.claude_app_override_dir = Some(claude_app.clone());

    let unpatched = inspect_claude_ui_patch(&paths);
    assert_eq!(unpatched.status, UiPatchStatus::Unpatched);
    assert!(!unpatched.has_valid_backup);
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::Unpatched
    );

    let patched = patch_claude_ui_for_paths(&paths).expect("patch_claude_ui_for_paths must succeed");
    assert_eq!(patched.status, UiPatchStatus::Patched);
    assert!(patched.has_valid_backup);
    assert_eq!(patched.patch_version.as_deref(), Some("2.0.9"));
    let target_str = patched
        .target_asar_path
        .as_deref()
        .expect("target_asar_path must be populated");
    assert!(
        target_str.ends_with("app.asar") || target_str.ends_with("ion-dist.asar"),
        "unexpected target_asar_path: {target_str}"
    );

    let claude_snap = snapshot_app(&paths, AppId::Claude);
    assert_eq!(claude_snap.ui_patch.status, UiPatchStatus::Patched);
    assert!(claude_snap.ui_patch.has_valid_backup);

    // Codex and DshTako remain NotApplicable
    for other_id in [AppId::Codex, AppId::DshTako] {
        assert_eq!(
            snapshot_app(&paths, other_id).ui_patch.status,
            UiPatchStatus::NotApplicable
        );
    }
}

#[test]
fn tc_209_02_dual_asar_deep_binary_i18n_and_icu_invariants() {
    let (tmp, mut paths) = setup_sandbox();
    let claude_app = tmp.path().join("DeepInspect").join("Claude.app");
    let fixture = write_fake_claude_macos_bundle(&claude_app, "6.28899.6", true);
    let orig_app_sha256 = sha256_hex(&fixture.orig_app_asar);
    let orig_ion_sha256 = sha256_hex(&fixture.orig_ion_asar);
    paths.claude_app_override_dir = Some(claude_app);

    patch_claude_ui_for_paths(&paths).expect("patch dual ASARs");

    // Parse and validate Pickle alignment + 4MB SHA-256 block integrity on BOTH archives
    let parsed_app = parse_and_verify_asar_pickle(&fs::read(&fixture.app_asar_path).unwrap());
    let parsed_ion = parse_and_verify_asar_pickle(&fs::read(&fixture.ion_asar_path).unwrap());

    // --- Verify app.asar ---
    let app_marker: Value = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_app,
        &["__tako_i18n_patch.json"],
    ))
    .expect("app.asar __tako_i18n_patch.json");
    assert_eq!(app_marker.get("schema").and_then(Value::as_u64), Some(1));
    assert_eq!(
        app_marker.get("patchVersion").and_then(Value::as_str),
        Some("2.0.9")
    );
    assert_eq!(
        app_marker
            .get("originalAsarSha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
        Some(orig_app_sha256)
    );

    let chunk_js = String::from_utf8(extract_packed_asar_file(
        &parsed_app,
        &[".vite", "build", "index.chunk-abc123.js"],
    ))
    .unwrap();
    assert_eq!(
        chunk_js.matches(CLAUDE_PATCH_GUARD_COMMENT).count(),
        1,
        "index.chunk-abc123.js must contain CLAUDE_PATCH_GUARD_COMMENT exactly once"
    );
    assert!(
        chunk_js.contains("\"zh-Hans\":\"zh-Hans\"") || chunk_js.contains("\"zh-Hans\": \"zh-Hans\""),
        "DesktopIntl QVe map must register zh-Hans: {chunk_js}"
    );
    assert!(
        chunk_js.contains("zh-CN"),
        "DesktopIntl QVe map must map zh-CN -> zh-Hans: {chunk_js}"
    );
    assert!(
        !chunk_js.contains("/[a-z]{2}-[A-Z]{2}/"),
        "sHe() narrow regex /[a-z]{{2}}-[A-Z]{{2}}/ must be relaxed: {chunk_js}"
    );
    assert!(
        chunk_js.contains("[a-z]{2,3}") && chunk_js.contains("[A-Za-z0-9]{2,8}"),
        "sHe() relaxed regex must accept script subtags like zh-Hans.json: {chunk_js}"
    );

    let shell_en_map: Map<String, Value> = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_app,
        &["resources", "i18n", "en-US.json"],
    ))
    .unwrap();
    let shell_zh_map: Map<String, Value> = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_app,
        &["resources", "i18n", "zh-Hans.json"],
    ))
    .unwrap();
    // Existing zh-Hans translations preserved
    assert_eq!(
        shell_zh_map.get("ShSettings01").and_then(Value::as_str),
        Some("设置")
    );
    // All 8 missing shell keys filled and key parity with en-US.json achieved
    assert_eq!(
        shell_zh_map.keys().collect::<BTreeSet<_>>(),
        shell_en_map.keys().collect::<BTreeSet<_>>(),
        "app.asar resources/i18n/zh-Hans.json must cover 100% of en-US.json keys"
    );
    let mut translated_missing_count = 0usize;
    for &(missing_id, en_text) in SHELL_MISSING_8_KEYS {
        let zh_val = shell_zh_map
            .get(missing_id)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing shell key {missing_id} in zh-Hans.json"));
        assert_eq!(
            extract_top_level_icu_vars(zh_val).unwrap(),
            extract_top_level_icu_vars(en_text).unwrap(),
            "shell key {missing_id} ICU placeholder mismatch"
        );
        if contains_cjk(zh_val) {
            translated_missing_count += 1;
        }
    }
    assert_eq!(
        translated_missing_count,
        SHELL_MISSING_8_KEYS.len(),
        "all 8 missing shell keys in app.asar must be translated into Simplified Chinese"
    );

    // Unpacked native addon preserved in app.asar without offset
    let app_unpacked = lookup_asar_node(
        &parsed_app.header,
        &[
            "node_modules",
            "@ant",
            "claude-native",
            "claude-native-binding.node",
        ],
    )
    .expect("app.asar unpacked native addon node must exist");
    assert_eq!(app_unpacked.get("unpacked").and_then(Value::as_bool), Some(true));
    assert!(app_unpacked.get("offset").is_none());

    // --- Verify ion-dist.asar ---
    let ion_marker: Value = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_ion,
        &["__tako_i18n_patch.json"],
    ))
    .expect("ion-dist.asar __tako_i18n_patch.json");
    assert_eq!(ion_marker.get("schema").and_then(Value::as_u64), Some(1));
    assert_eq!(
        ion_marker.get("patchVersion").and_then(Value::as_str),
        Some("2.0.9")
    );
    assert_eq!(
        ion_marker
            .get("originalAsarSha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
        Some(orig_ion_sha256)
    );

    for js_name in ["shared-73-test.js", "shared-107-test.js", "c2c114516-test.js"] {
        let js_text = String::from_utf8(extract_packed_asar_file(
            &parsed_ion,
            &["assets", "v1", js_name],
        ))
        .unwrap();
        assert_eq!(
            js_text.matches(CLAUDE_PATCH_GUARD_COMMENT).count(),
            1,
            "{js_name} must contain CLAUDE_PATCH_GUARD_COMMENT once"
        );
        assert!(
            js_text.contains("\"id-ID\",\"zh-Hans\"]")
                || js_text.contains("\"id-ID\", \"zh-Hans\"]"),
            "{js_name} 11-locale whitelist array must append \"zh-Hans\": {js_text}"
        );
        if js_name == "c2c114516-test.js" {
            let decoded = unescape_js_unicode(&js_text);
            assert!(
                decoded.contains("Chinese (Simplified)") && decoded.contains("中文（简体）"),
                "c2c114516-test.js LOCALE_DISPLAY_MAP must expose zh-Hans -> 中文（简体）, got: {decoded}"
            );
        }
    }

    // Verify 51 dynamic model/thinking/effort keys all translated into Simplified Chinese
    let dyn_zh_map: Map<String, Value> = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_ion,
        &["i18n", "dynamic", "zh-Hans.json"],
    ))
    .expect("i18n/dynamic/zh-Hans.json must be injected");
    assert_eq!(dyn_zh_map.len(), 51, "dynamic/zh-Hans.json must have all 51 keys");
    for &(dyn_id, dyn_en_text) in DYNAMIC_51_KEYS {
        let zh_text = dyn_zh_map
            .get(dyn_id)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing dynamic key {dyn_id}"));
        assert!(
            contains_cjk(zh_text),
            "dynamic key {dyn_id} ({dyn_en_text:?}) must be translated into Chinese, got {zh_text:?}"
        );
    }

    // Verify main i18n/zh-Hans.json 100% key parity, core domain translations & ICU placeholder safety
    let ion_en_map: Map<String, Value> = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_ion,
        &["i18n", "en-US.json"],
    ))
    .unwrap();
    let ion_zh_map: Map<String, Value> = serde_json::from_slice(&extract_packed_asar_file(
        &parsed_ion,
        &["i18n", "zh-Hans.json"],
    ))
    .expect("i18n/zh-Hans.json must be injected");
    assert_eq!(
        ion_zh_map.keys().collect::<BTreeSet<_>>(),
        ion_en_map.keys().collect::<BTreeSet<_>>(),
        "ion-dist i18n/zh-Hans.json must have 100% exact key parity with i18n/en-US.json"
    );

    // Core Chat / Claude Code / Cowork / Settings UI strings must be translated to Chinese
    for core_key in [
        "ChatNew001", // New chat
        "CodeSes001", // Sessions
        "CodeRep002", // Repositories
        "CodePrm005", // Permissions
        "CowkSch001", // Scheduled tasks
        "SetMain001", // Settings
        "SetApr002",  // Appearance
        "SetLng003",  // Language
        "SetEml007",  // Logged in as {email}
        "OvlAllow01", // Allow Claude to use {toolName}?
    ] {
        let en_s = ion_en_map.get(core_key).and_then(Value::as_str).unwrap();
        let zh_s = ion_zh_map.get(core_key).and_then(Value::as_str).unwrap();
        assert!(
            zh_s != en_s && contains_cjk(zh_s),
            "core UI string {core_key} ({en_s:?}) must be translated into Simplified Chinese, got {zh_s:?}"
        );
    }

    // Mandatory ICU placeholder & brace-balance invariant across EVERY key
    for (k, en_v) in &ion_en_map {
        let en_s = en_v.as_str().unwrap();
        let zh_s = ion_zh_map
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("missing key {k} in ion-dist zh-Hans.json"));
        let en_vars = extract_top_level_icu_vars(en_s)
            .unwrap_or_else(|e| panic!("fixture en-US ICU error on {k}: {e}"));
        let zh_vars = extract_top_level_icu_vars(zh_s)
            .unwrap_or_else(|e| panic!("patched zh-Hans ICU brace corruption on {k} ({zh_s:?}): {e}"));
        assert_eq!(
            zh_vars, en_vars,
            "ICU placeholder variable set mismatch on key {k}: en={en_s:?} vs zh={zh_s:?}"
        );
    }

    // Verify multi-block (>4MB) file preserved 2-block SHA-256 integrity & unpacked entry preserved
    let multiblock_node = lookup_asar_node(
        &parsed_ion.header,
        &["assets", "v1", "wasm-worker-bundle.bin"],
    )
    .expect("multi-block binary entry must be preserved");
    let blocks_arr = multiblock_node
        .get("integrity")
        .and_then(|i| i.get("blocks"))
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(blocks_arr.len(), 2, "4MiB+2KiB entry must have 2 SHA-256 blocks");

    let ion_unpacked = lookup_asar_node(&parsed_ion.header, &["assets", "native", "ion-helper.node"])
        .expect("ion-dist unpacked node must exist");
    assert_eq!(ion_unpacked.get("unpacked").and_then(Value::as_bool), Some(true));
    assert!(ion_unpacked.get("offset").is_none());
}

#[test]
fn tc_209_03_macos_dual_info_plist_asar_integrity_hashes() {
    let (tmp, mut paths) = setup_sandbox();
    let claude_app = tmp.path().join("PlistDualHash").join("Claude.app");
    let fixture = write_fake_claude_macos_bundle(&claude_app, "6.28899.6", false);
    paths.claude_app_override_dir = Some(claude_app);

    patch_claude_ui_for_paths(&paths).expect("patch Claude.app");

    let parsed_app = parse_and_verify_asar_pickle(&fs::read(&fixture.app_asar_path).unwrap());
    let parsed_ion = parse_and_verify_asar_pickle(&fs::read(&fixture.ion_asar_path).unwrap());
    assert_ne!(parsed_app.header_sha256, fixture.orig_app_header_sha256);
    assert_ne!(parsed_ion.header_sha256, fixture.orig_ion_header_sha256);

    let updated_plist = fs::read_to_string(&fixture.info_plist_path).unwrap();
    let plist_app_hash = extract_plist_asar_hash(&updated_plist, "Resources/app.asar")
        .expect("Info.plist must retain Resources/app.asar integrity entry");
    let plist_ion_hash = extract_plist_asar_hash(&updated_plist, "Resources/ion-dist.asar")
        .expect("Info.plist must retain Resources/ion-dist.asar integrity entry");

    assert_eq!(
        plist_app_hash.to_ascii_lowercase(),
        parsed_app.header_sha256,
        "Info.plist Resources/app.asar hash must match repacked app.asar header SHA-256"
    );
    assert_eq!(
        plist_ion_hash.to_ascii_lowercase(),
        parsed_ion.header_sha256,
        "Info.plist Resources/ion-dist.asar hash must match repacked ion-dist.asar header SHA-256"
    );
}

#[test]
fn tc_209_04_idempotency_and_byte_for_byte_restore() {
    let (tmp, mut paths) = setup_sandbox();
    let claude_app = tmp.path().join("IdempotentRestore").join("Claude.app");
    let fixture = write_fake_claude_macos_bundle(&claude_app, "6.28899.6", false);
    paths.claude_app_override_dir = Some(claude_app);

    // First patch
    patch_claude_ui_for_paths(&paths).expect("first patch");
    // Second patch (idempotent)
    let second_info = patch_claude_ui_for_paths(&paths).expect("second idempotent patch");
    assert_eq!(second_info.status, UiPatchStatus::Patched);
    assert!(second_info.has_valid_backup);

    let backup_dir = paths.backups_dir("claude").join("ui_patch");
    assert_eq!(
        fs::read(backup_dir.join("app.asar.orig")).unwrap(),
        fixture.orig_app_asar,
        "second patch must NEVER overwrite app.asar.orig with patched bytes"
    );
    assert_eq!(
        fs::read(backup_dir.join("ion-dist.asar.orig")).unwrap(),
        fixture.orig_ion_asar,
        "second patch must NEVER overwrite ion-dist.asar.orig with patched bytes"
    );

    // Verify guard comments were not duplicated on second patch
    let reparsed_app = parse_and_verify_asar_pickle(&fs::read(&fixture.app_asar_path).unwrap());
    let chunk_js = String::from_utf8(extract_packed_asar_file(
        &reparsed_app,
        &[".vite", "build", "index.chunk-abc123.js"],
    ))
    .unwrap();
    assert_eq!(chunk_js.matches(CLAUDE_PATCH_GUARD_COMMENT).count(), 1);

    let reparsed_ion = parse_and_verify_asar_pickle(&fs::read(&fixture.ion_asar_path).unwrap());
    let c2c_js = String::from_utf8(extract_packed_asar_file(
        &reparsed_ion,
        &["assets", "v1", "c2c114516-test.js"],
    ))
    .unwrap();
    assert_eq!(c2c_js.matches(CLAUDE_PATCH_GUARD_COMMENT).count(), 1);

    // Simulate post-codesign state where MacOS_Claude.orig & CodeResources.orig exist and bundle signature files changed
    if !backup_dir.join("MacOS_Claude.orig").exists() {
        fs::write(backup_dir.join("MacOS_Claude.orig"), &fixture.orig_macos_bin).unwrap();
    }
    if !backup_dir.join("CodeResources.orig").exists() {
        fs::write(
            backup_dir.join("CodeResources.orig"),
            &fixture.orig_code_resources,
        )
        .unwrap();
    }
    fs::write(&fixture.macos_bin_path, b"adhoc-resigned-macho-bytes").unwrap();
    fs::write(&fixture.code_resources_path, b"<plist>adhoc-signed</plist>").unwrap();

    // Restore -> all 5 files must be 100% byte-identical to original
    let restored = restore_claude_ui_for_paths(&paths).expect("restore_claude_ui_for_paths");
    assert_eq!(restored.status, UiPatchStatus::Unpatched);
    assert!(!backup_dir.join("patch-meta.json").exists());

    assert_eq!(fs::read(&fixture.app_asar_path).unwrap(), fixture.orig_app_asar);
    assert_eq!(fs::read(&fixture.ion_asar_path).unwrap(), fixture.orig_ion_asar);
    assert_eq!(
        fs::read(&fixture.info_plist_path).unwrap(),
        fixture.orig_info_plist
    );
    assert_eq!(
        fs::read(&fixture.macos_bin_path).unwrap(),
        fixture.orig_macos_bin
    );
    assert_eq!(
        fs::read(&fixture.code_resources_path).unwrap(),
        fixture.orig_code_resources
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::Unpatched
    );
}

#[test]
fn tc_209_05_client_self_upgrade_outdated_and_never_downgrades() {
    let (tmp, mut paths) = setup_sandbox();
    let claude_app = tmp.path().join("UpgradeSafety").join("Claude.app");
    let v1_fixture = write_fake_claude_macos_bundle(&claude_app, "6.28899.6", false);
    paths.claude_app_override_dir = Some(claude_app.clone());

    patch_claude_ui_for_paths(&paths).expect("patch 6.28899.6");
    let patched_v1_app = fs::read(&v1_fixture.app_asar_path).unwrap();
    let patched_v1_ion = fs::read(&v1_fixture.ion_asar_path).unwrap();

    let (clean_v2_app, v2_app_hdr_sha) = build_clean_claude_app_asar("6.29000.0");
    let (clean_v2_ion, v2_ion_hdr_sha) = build_clean_claude_ion_asar("6.29000.0", false);
    let clean_v2_plist =
        render_claude_macos_info_plist("6.29000.0", &v2_app_hdr_sha, &v2_ion_hdr_sha).into_bytes();

    // Case 1: Only app.asar upgraded to 6.29000.0 -> OutdatedAfterUpgrade
    fs::write(&v1_fixture.app_asar_path, &clean_v2_app).unwrap();
    fs::write(&v1_fixture.ion_asar_path, &patched_v1_ion).unwrap();
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    // Case 2: Only ion-dist.asar upgraded to 6.29000.0 -> OutdatedAfterUpgrade
    fs::write(&v1_fixture.app_asar_path, &patched_v1_app).unwrap();
    fs::write(&v1_fixture.ion_asar_path, &clean_v2_ion).unwrap();
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    // Case 3: Both ASARs + Info.plist upgraded to 6.29000.0 -> OutdatedAfterUpgrade
    fs::write(&v1_fixture.app_asar_path, &clean_v2_app).unwrap();
    fs::write(&v1_fixture.ion_asar_path, &clean_v2_ion).unwrap();
    fs::write(&v1_fixture.info_plist_path, &clean_v2_plist).unwrap();
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    // Calling restore in OutdatedAfterUpgrade MUST NEVER downgrade 6.29000.0 to 6.28899.6 .orig!
    let after_restore = restore_claude_ui_for_paths(&paths)
        .expect("restore on OutdatedAfterUpgrade must clean stale meta without downgrading");
    assert_eq!(after_restore.status, UiPatchStatus::Unpatched);
    assert_eq!(fs::read(&v1_fixture.app_asar_path).unwrap(), clean_v2_app);
    assert_eq!(fs::read(&v1_fixture.ion_asar_path).unwrap(), clean_v2_ion);
    assert_eq!(fs::read(&v1_fixture.info_plist_path).unwrap(), clean_v2_plist);

    // Case 4: Patch 6.28899.6 -> upgrade to 6.29000.0 -> re-patch refreshes .orig to 6.29000.0
    let claude_app_repatch = tmp.path().join("UpgradeRepatch").join("Claude.app");
    write_fake_claude_macos_bundle(&claude_app_repatch, "6.28899.6", false);
    paths.claude_app_override_dir = Some(claude_app_repatch.clone());
    patch_claude_ui_for_paths(&paths).expect("patch 6.28899.6 before upgrade");

    let v2_fixture = write_fake_claude_macos_bundle(&claude_app_repatch, "6.29000.0", false);
    assert_eq!(
        inspect_claude_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    let repatched = patch_claude_ui_for_paths(&paths).expect("re-patch upgraded 6.29000.0");
    assert_eq!(repatched.status, UiPatchStatus::Patched);
    assert!(repatched.has_valid_backup);

    let backup_dir = paths.backups_dir("claude").join("ui_patch");
    assert_eq!(
        fs::read(backup_dir.join("app.asar.orig")).unwrap(),
        v2_fixture.orig_app_asar,
        "re-patching after upgrade must refresh app.asar.orig to 6.29000.0"
    );
    assert_eq!(
        fs::read(backup_dir.join("ion-dist.asar.orig")).unwrap(),
        v2_fixture.orig_ion_asar,
        "re-patching after upgrade must refresh ion-dist.asar.orig to 6.29000.0"
    );

    restore_claude_ui_for_paths(&paths).expect("restore 6.29000.0");
    assert_eq!(
        fs::read(&v2_fixture.app_asar_path).unwrap(),
        v2_fixture.orig_app_asar
    );
    assert_eq!(
        fs::read(&v2_fixture.ion_asar_path).unwrap(),
        v2_fixture.orig_ion_asar
    );
}

#[test]
fn tc_209_06_windows_squirrel_msix_and_pe32_plus_asar_integrity() {
    let (tmp, mut paths) = setup_sandbox();

    // --- Part 1: Direct PE32+ fuse & integrity unit invariants ---
    let h1_old = "a".repeat(64);
    let h2_old = "b".repeat(64);
    let h1_new = "c".repeat(64);
    let h2_new = "d".repeat(64);

    let (exe_no_fuse, _, _) = build_synthetic_pe32_plus_exe(None, &h1_old, &h2_old, false);
    assert_eq!(inspect_electron_asar_integrity_fuse(&exe_no_fuse), None);
    assert_eq!(
        patch_windows_pe_asar_integrity(&exe_no_fuse, &[(&h1_old, &h1_new)]).unwrap(),
        None
    );

    let (exe_fuse_off, _, _) = build_synthetic_pe32_plus_exe(Some(false), &h1_old, &h2_old, false);
    assert_eq!(
        inspect_electron_asar_integrity_fuse(&exe_fuse_off),
        Some(false)
    );
    assert_eq!(
        patch_windows_pe_asar_integrity(&exe_fuse_off, &[(&h1_old, &h1_new), (&h2_old, &h2_new)])
            .unwrap(),
        None,
        "fuse=0 executable must remain untouched"
    );

    let (exe_fuse_on, opt_hdr, cert_offset) =
        build_synthetic_pe32_plus_exe(Some(true), &h1_old, &h2_old, true);
    assert_eq!(
        inspect_electron_asar_integrity_fuse(&exe_fuse_on),
        Some(true)
    );
    let patched_pe = patch_windows_pe_asar_integrity(
        &exe_fuse_on,
        &[(&h1_old, &h1_new.to_ascii_uppercase()), (&h2_old, &h2_new)],
    )
    .expect("patch_windows_pe_asar_integrity with fuse=1 must succeed")
    .expect("fuse=1 must return Some(patched_bytes)");

    assert_eq!(
        patched_pe.len(),
        cert_offset,
        "trailing 64-byte Authenticode certificate must be truncated"
    );
    assert_eq!(
        u32::from_le_bytes(patched_pe[opt_hdr + 64..opt_hdr + 68].try_into().unwrap()),
        0,
        "PE32+ OptionalHeader.CheckSum must be zeroed"
    );
    assert_eq!(
        u32::from_le_bytes(patched_pe[opt_hdr + 144..opt_hdr + 148].try_into().unwrap()),
        0,
        "PE32+ DataDirectory[4].VirtualAddress must be zeroed"
    );
    assert_eq!(
        u32::from_le_bytes(patched_pe[opt_hdr + 148..opt_hdr + 152].try_into().unwrap()),
        0,
        "PE32+ DataDirectory[4].Size must be zeroed"
    );
    let patched_pe_ascii = String::from_utf8_lossy(&patched_pe);
    assert!(
        patched_pe_ascii.contains(&h1_new) && patched_pe_ascii.contains(&h2_new),
        "patched PE32+ must contain lowercase 64-hex replacement hashes"
    );
    assert!(
        !patched_pe_ascii.to_ascii_lowercase().contains(&h1_old),
        "old hash must no longer appear in patched PE32+"
    );
    assert!(
        patch_windows_pe_asar_integrity(&exe_fuse_on, &[("short", &h1_new)]).is_err(),
        "invalid hash length must return Err"
    );

    // --- Part 2: Windows Squirrel multi-version layout (with Claude.exe stub + fuse=1 claude.exe) ---
    let squirrel_root = tmp.path().join("AnthropicClaude");
    fs::create_dir_all(&squirrel_root).unwrap();
    let root_stub_exe = squirrel_root.join("Claude.exe");
    fs::write(&root_stub_exe, b"MZ-squirrel-launcher-stub").unwrap();

    // Older versions (including 6.9999.0 to verify numeric semver component sorting over lexicographical!)
    for old_ver in ["6.9999.0", "6.28000.0"] {
        let res_dir = squirrel_root.join(format!("app-{old_ver}")).join("resources");
        fs::create_dir_all(&res_dir).unwrap();
        let (old_app, _) = build_clean_claude_app_asar(old_ver);
        let (old_ion, _) = build_clean_claude_ion_asar(old_ver, false);
        fs::write(res_dir.join("app.asar"), old_app).unwrap();
        fs::write(res_dir.join("ion-dist.asar"), old_ion).unwrap();
    }
    let old_28000_app_before = fs::read(
        squirrel_root
            .join("app-6.28000.0")
            .join("resources")
            .join("app.asar"),
    )
    .unwrap();

    // Latest version 6.28899.6 with fuse=1 PE32+ claude.exe
    let latest_dir = squirrel_root.join("app-6.28899.6");
    let latest_res = latest_dir.join("resources");
    fs::create_dir_all(&latest_res).unwrap();
    let (latest_app_bytes, latest_app_hdr_sha) = build_clean_claude_app_asar("6.28899.6");
    let (latest_ion_bytes, latest_ion_hdr_sha) = build_clean_claude_ion_asar("6.28899.6", false);
    let (latest_exe_bytes, latest_opt_hdr, latest_cert_off) = build_synthetic_pe32_plus_exe(
        Some(true),
        &latest_app_hdr_sha,
        &latest_ion_hdr_sha,
        false,
    );
    let latest_exe_path = latest_dir.join("claude.exe");
    let latest_app_path = latest_res.join("app.asar");
    let latest_ion_path = latest_res.join("ion-dist.asar");
    fs::write(&latest_exe_path, &latest_exe_bytes).unwrap();
    fs::write(&latest_app_path, &latest_app_bytes).unwrap();
    fs::write(&latest_ion_path, &latest_ion_bytes).unwrap();

    // Point claude_app_override_dir directly at the Squirrel launcher stub `Claude.exe`
    paths.claude_app_override_dir = Some(root_stub_exe);
    let sq_inspect = inspect_claude_ui_patch(&paths);
    assert_eq!(sq_inspect.status, UiPatchStatus::Unpatched);
    assert!(
        sq_inspect
            .target_asar_path
            .as_deref()
            .unwrap_or("")
            .contains("app-6.28899.6"),
        "Squirrel resolver must pick highest numeric version app-6.28899.6, got: {:?}",
        sq_inspect.target_asar_path
    );

    let sq_patched = patch_claude_ui_for_paths(&paths).expect("patch Squirrel 6.28899.6");
    assert_eq!(sq_patched.status, UiPatchStatus::Patched);

    // Older version directory untouched
    assert_eq!(
        fs::read(
            squirrel_root
                .join("app-6.28000.0")
                .join("resources")
                .join("app.asar")
        )
        .unwrap(),
        old_28000_app_before
    );

    // Verify claude.exe updated hashes, zeroed CheckSum & Security directory, truncated cert, and saved Claude.exe.orig
    let parsed_sq_app = parse_and_verify_asar_pickle(&fs::read(&latest_app_path).unwrap());
    let parsed_sq_ion = parse_and_verify_asar_pickle(&fs::read(&latest_ion_path).unwrap());
    let disk_patched_exe = fs::read(&latest_exe_path).unwrap();
    assert_eq!(disk_patched_exe.len(), latest_cert_off);
    assert_eq!(
        u32::from_le_bytes(
            disk_patched_exe[latest_opt_hdr + 64..latest_opt_hdr + 68]
                .try_into()
                .unwrap()
        ),
        0
    );
    let disk_exe_str = String::from_utf8_lossy(&disk_patched_exe);
    assert!(disk_exe_str.contains(&parsed_sq_app.header_sha256));
    assert!(disk_exe_str.contains(&parsed_sq_ion.header_sha256));
    assert!(
        paths
            .backups_dir("claude")
            .join("ui_patch")
            .join("Claude.exe.orig")
            .is_file(),
        "Claude.exe.orig backup must be created when fuse=1"
    );

    // Restore Squirrel -> byte-for-byte recovery of claude.exe, app.asar, and ion-dist.asar
    let sq_restored = restore_claude_ui_for_paths(&paths).expect("restore Squirrel 6.28899.6");
    assert_eq!(sq_restored.status, UiPatchStatus::Unpatched);
    assert_eq!(fs::read(&latest_exe_path).unwrap(), latest_exe_bytes);
    assert_eq!(fs::read(&latest_app_path).unwrap(), latest_app_bytes);
    assert_eq!(fs::read(&latest_ion_path).unwrap(), latest_ion_bytes);

    // Also verify fuse=0 Squirrel claude.exe is left 100% untouched on disk during patch
    let (fuse0_exe_bytes, _, _) = build_synthetic_pe32_plus_exe(
        Some(false),
        &latest_app_hdr_sha,
        &latest_ion_hdr_sha,
        false,
    );
    fs::write(&latest_exe_path, &fuse0_exe_bytes).unwrap();
    let _ = fs::remove_file(
        paths
            .backups_dir("claude")
            .join("ui_patch")
            .join("Claude.exe.orig"),
    );
    patch_claude_ui_for_paths(&paths).expect("patch Squirrel with fuse=0");
    assert_eq!(
        fs::read(&latest_exe_path).unwrap(),
        fuse0_exe_bytes,
        "claude.exe with fuse=0 must not be modified on disk"
    );
    restore_claude_ui_for_paths(&paths).expect("restore Squirrel after fuse=0 patch");

    // --- Part 3: MSIX WindowsApps read-only detection + WindowsMsixReadOnly block error ---
    let msix_dir = tmp
        .path()
        .join("Program Files")
        .join("WindowsApps")
        .join("AnthropicPBC.Claude_6.28899.6_x64__pzs8sxrjxfjjc");
    let msix_res = msix_dir.join("app").join("resources");
    fs::create_dir_all(&msix_res).unwrap();
    fs::write(msix_res.join("app.asar"), &latest_app_bytes).unwrap();
    fs::write(msix_res.join("ion-dist.asar"), &latest_ion_bytes).unwrap();

    paths.claude_app_override_dir = Some(msix_dir);
    let msix_info = inspect_claude_ui_patch(&paths);
    assert_eq!(msix_info.status, UiPatchStatus::Unsupported);
    assert!(
        msix_info
            .note
            .as_deref()
            .is_some_and(|n| contains_cjk(n) && (n.contains("MSIX") || n.contains("Store") || n.contains("微软商店"))),
        "MSIX Unsupported note should explain Store/MSIX read-only limitation in Chinese, got: {:?}",
        msix_info.note
    );

    let msix_err = patch_claude_ui_for_paths(&paths)
        .expect_err("patching MSIX WindowsApps install must return WindowsMsixReadOnly block error");
    let msix_prompt = parse_ui_patch_block_error(&msix_err)
        .unwrap_or_else(|| panic!("expected structured block error, got: {msix_err}"));
    assert_eq!(msix_prompt.app_id, AppId::Claude);
    assert_eq!(msix_prompt.kind, UiPatchBlockKind::WindowsMsixReadOnly);
}

#[test]
fn tc_209_07_cross_platform_permission_classification_and_roundtrip() {
    assert_eq!(UI_PATCH_BLOCK_PREFIX, "[TAKO_UI_PATCH_BLOCK]");

    let mac_target = Path::new("/Applications/Claude.app/Contents/Resources/ion-dist.asar");
    let win_target = Path::new(r"C:\Program Files\Claude\resources\app.asar");

    // macOS EPERM (1) & EACCES (13) -> MacosAppManagement
    for raw_code in [1, 13] {
        let err = io::Error::from_raw_os_error(raw_code);
        let p = classify_io_error_for_os("macos", AppId::Claude, "patch", mac_target, &err)
            .unwrap_or_else(|| panic!("macOS errno {raw_code} must classify as MacosAppManagement"));
        assert_eq!(p.app_id, AppId::Claude);
        assert_eq!(p.action, "patch");
        assert_eq!(p.kind, UiPatchBlockKind::MacosAppManagement);
        assert!(
            p.message.contains("App 管理") || p.message.contains("隐私与安全性"),
            "MacosAppManagement message must guide user to App Management settings: {}",
            p.message
        );
    }

    // Windows ERROR_ACCESS_DENIED (5) -> WindowsUac
    let win_uac_err = io::Error::from_raw_os_error(5);
    let p_uac = classify_io_error_for_os("windows", AppId::Claude, "patch", win_target, &win_uac_err)
        .expect("Windows error 5 must classify as WindowsUac");
    assert_eq!(p_uac.kind, UiPatchBlockKind::WindowsUac);
    assert_eq!(p_uac.action, "patch");

    // Windows ERROR_SHARING_VIOLATION (32) -> WindowsFileLock
    let win_lock_err = io::Error::from_raw_os_error(32);
    let p_lock = classify_io_error_for_os(
        "windows",
        AppId::Antigravity,
        "restore",
        win_target,
        &win_lock_err,
    )
    .expect("Windows error 32 must classify as WindowsFileLock");
    assert_eq!(p_lock.app_id, AppId::Antigravity);
    assert_eq!(p_lock.action, "restore");
    assert_eq!(p_lock.kind, UiPatchBlockKind::WindowsFileLock);

    // Unrelated NotFound error -> None
    let not_found = io::Error::new(io::ErrorKind::NotFound, "missing file");
    assert!(
        classify_io_error_for_os("macos", AppId::Claude, "patch", mac_target, &not_found).is_none()
    );

    // Format & parse roundtrip (both direct and wrapped in outer context!)
    let sample_prompt = UiPatchPermissionPrompt {
        app_id: AppId::Claude,
        app_name: "Claude Desktop".to_string(),
        action: "patch".to_string(),
        kind: UiPatchBlockKind::MacosAppManagement,
        target_path: Some(mac_target.to_string_lossy().to_string()),
        message: "请在系统设置 -> 隐私与安全性 -> App 管理中允许 Tako Switch".to_string(),
    };
    let encoded = format_ui_patch_block_error(&sample_prompt);
    assert!(
        encoded.starts_with(UI_PATCH_BLOCK_PREFIX),
        "encoded block error must start with UI_PATCH_BLOCK_PREFIX: {encoded}"
    );
    assert_eq!(
        parse_ui_patch_block_error(&encoded),
        Some(sample_prompt.clone())
    );

    let wrapped = format!("外层同步上下文: 无法写入资源包: {encoded} (等待用户授权后重试)");
    assert_eq!(
        parse_ui_patch_block_error(&wrapped),
        Some(sample_prompt),
        "parse_ui_patch_block_error must extract prompt even when wrapped"
    );
    assert_eq!(parse_ui_patch_block_error("普通网络超时错误"), None);
}

#[cfg(unix)]
#[test]
fn tc_209_08_config_vs_asar_decoupling_under_permission_denied() {
    let (tmp, mut paths) = setup_sandbox();

    // Prepare Claude.app & Antigravity.app inside TempDir
    let claude_app = tmp.path().join("ProtectedApps").join("Claude.app");
    let agy_app = tmp.path().join("ProtectedApps").join("Antigravity.app");
    write_fake_claude_macos_bundle(&claude_app, "6.28899.6", false);
    build_clean_antigravity_bundle(&agy_app, "2.1.0");
    paths.claude_app_override_dir = Some(claude_app.clone());
    paths.agy_app_override_dir = Some(agy_app.clone());

    // Prepare existing config directories for Codex, Claude (1p/3p/Claude Code), and Antigravity
    fs::create_dir_all(&paths.codex_dir).unwrap();
    fs::write(
        paths.codex_dir.join("config.toml"),
        "model = \"gpt-6.2-sol\"\n\n[desktop]\nlocaleOverride = \"en-US\"\n",
    )
    .unwrap();
    fs::write(paths.codex_dir.join("AGENTS.md"), "# User Codex Rules\n").unwrap();

    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    fs::create_dir_all(paths.home.join(".claude")).unwrap();
    fs::write(
        paths.claude_1p_dir.join("config.json"),
        r#"{"darkMode":"dark","locale":"en-US"}"#,
    )
    .unwrap();
    fs::write(
        paths.claude_3p_dir.join("config.json"),
        r#"{"darkMode":"dark"}"#,
    )
    .unwrap();
    fs::write(
        paths.home.join(".claude").join("settings.json"),
        r#"{"model":"claude-opus-5-5"}"#,
    )
    .unwrap();

    let gemini_dir = paths.home.join(".gemini");
    fs::create_dir_all(&gemini_dir).unwrap();
    fs::write(gemini_dir.join("AGENTS.md"), "# User Gemini Rules\n").unwrap();

    // Declare ChmodRestoreGuard AFTER `tmp` so its Drop restores 0o755 BEFORE TempDir cleans up!
    let claude_res_dir = claude_app.join("Contents").join("Resources");
    let agy_res_dir = agy_app.join("Contents").join("Resources");
    let chmod_guard = ChmodRestoreGuard::new(vec![claude_res_dir, agy_res_dir]);
    chmod_guard.set_mode(0o555);

    // 1) Direct patch call on read-only .app must surface structured MacosAppManagement block error
    let direct_err = patch_claude_ui_for_paths(&paths)
        .expect_err("direct patch on 0o555 Resources must fail with permission block");
    let direct_prompt = parse_ui_patch_block_error(&direct_err)
        .unwrap_or_else(|| panic!("expected structured permission prompt, got: {direct_err}"));
    assert_eq!(direct_prompt.app_id, AppId::Claude);
    assert_eq!(direct_prompt.kind, UiPatchBlockKind::MacosAppManagement);

    // 2) Batch toggle `apply_prefer_chinese_ui_for_paths(&paths, true, true, false)` MUST succeed (`Ok(prompts)`)
    //    returning MacosAppManagement prompts while still writing ALL config-level Chinese settings!
    let prompts = apply_prefer_chinese_ui_for_paths(&paths, true, true, false)
        .expect("apply_prefer_chinese_ui_for_paths must NOT abort config sync when ASAR is read-only");

    assert!(
        read_switch_state(&paths).prefer_chinese_ui,
        "SwitchState.prefer_chinese_ui must be persisted as true"
    );

    // Both read-only .app bundles must report MacosAppManagement prompts
    assert!(
        prompts.iter().any(|p| p.app_id == AppId::Claude
            && p.kind == UiPatchBlockKind::MacosAppManagement
            && p.action == "patch"),
        "expected Claude MacosAppManagement patch prompt in: {prompts:?}"
    );
    assert!(
        prompts.iter().any(|p| p.app_id == AppId::Antigravity
            && p.kind == UiPatchBlockKind::MacosAppManagement
            && p.action == "patch"),
        "expected Antigravity MacosAppManagement patch prompt in: {prompts:?}"
    );

    // Verify Codex config.toml + AGENTS.md were successfully written!
    assert_eq!(get_adapter(AppId::Codex).read_mode(&paths), AppMode::Official);
    assert!(CodexAdapter::is_chinese_configured(&paths));
    let codex_doc: DocumentMut = fs::read_to_string(paths.codex_dir.join("config.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        codex_doc["desktop"]["localeOverride"].as_str(),
        Some("zh-CN")
    );
    let codex_agents = fs::read_to_string(paths.codex_dir.join("AGENTS.md")).unwrap();
    assert!(codex_agents.contains(BLOCK_START) && codex_agents.contains(BLOCK_END));

    // Verify Claude 1p/3p config.json ("locale": "zh-Hans") + ~/.claude/settings.json ("language": "Simplified Chinese")!
    assert!(ClaudeAdapter::is_chinese_configured(&paths));
    assert_eq!(
        read_json_object(&paths.claude_1p_dir.join("config.json"))
            .get("locale")
            .and_then(Value::as_str),
        Some("zh-Hans")
    );
    assert_eq!(
        read_json_object(&paths.claude_3p_dir.join("config.json"))
            .get("locale")
            .and_then(Value::as_str),
        Some("zh-Hans")
    );
    assert_eq!(
        read_json_object(&paths.home.join(".claude").join("settings.json"))
            .get("language")
            .and_then(Value::as_str),
        Some("Simplified Chinese")
    );

    // Verify Antigravity ~/.gemini/AGENTS.md was successfully written!
    assert!(AntigravityAdapter::is_chinese_configured(&paths));
    let agy_agents = fs::read_to_string(gemini_dir.join("AGENTS.md")).unwrap();
    assert!(agy_agents.contains(BLOCK_START) && agy_agents.contains(BLOCK_END));

    // Read-only ASARs remain cleanly Unpatched
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::Unpatched
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Antigravity).ui_patch.status,
        UiPatchStatus::Unpatched
    );

    // 3) Simulate granting App Management permission (restore 0o755) and clicking Retry
    chmod_guard.set_mode(0o755);
    let retry_prompts = apply_prefer_chinese_ui_for_paths(&paths, true, true, false)
        .expect("retry after granting 0o755 must succeed");
    assert!(
        retry_prompts.is_empty(),
        "retry with writable Resources must produce zero prompts, got: {retry_prompts:?}"
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::Patched
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Antigravity).ui_patch.status,
        UiPatchStatus::Patched
    );

    // 4) Also verify disabling Chinese UI under 0o555 decouples config restore from ASAR restore
    chmod_guard.set_mode(0o555);
    let disable_prompts = apply_prefer_chinese_ui_for_paths(&paths, false, true, false)
        .expect("disabling Chinese UI under 0o555 must still restore configs cleanly");
    assert!(!read_switch_state(&paths).prefer_chinese_ui);
    assert!(!CodexAdapter::is_chinese_configured(&paths));
    assert!(!ClaudeAdapter::is_chinese_configured(&paths));
    assert!(!AntigravityAdapter::is_chinese_configured(&paths));
    assert!(
        disable_prompts
            .iter()
            .any(|p| p.app_id == AppId::Claude && p.action == "restore")
    );

    // Restore 0o755 explicitly before TempDir drops
    chmod_guard.restore_writable();
    let final_prompts =
        apply_prefer_chinese_ui_for_paths(&paths, false, true, false).expect("final clean restore");
    assert!(final_prompts.is_empty());
    assert_eq!(
        snapshot_app(&paths, AppId::Claude).ui_patch.status,
        UiPatchStatus::Unpatched
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Antigravity).ui_patch.status,
        UiPatchStatus::Unpatched
    );
}
