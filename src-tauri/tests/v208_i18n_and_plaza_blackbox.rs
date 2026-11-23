use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tako_switch_lib::adapters::antigravity::AntigravityAdapter;
use tako_switch_lib::adapters::claude::ClaudeAdapter;
use tako_switch_lib::adapters::codex::CodexAdapter;
use tako_switch_lib::adapters::{
    get_adapter, inspect_antigravity_ui_patch, patch_antigravity_ui_for_paths,
    restore_antigravity_ui_for_paths, snapshot_app, AppId, AppMode, UiPatchInfo, UiPatchStatus,
};
use tako_switch_lib::agy_proxy::{
    build_plaza_model_options, resolve_promo_ends_on_at_date, ModelPerfInfo,
};
use tako_switch_lib::config::AppPaths;
use tempfile::TempDir;
use toml_edit::DocumentMut;

const BLOCK_START: &str = "<!-- tako-switch:chinese-ui:start -->";
const BLOCK_END: &str = "<!-- tako-switch:chinese-ui:end -->";
const AGY_ZH_PATCH_GUARD: &str = "/* __TAKO_AGY_ZH_PATCH_V1__ */";
const ASAR_BLOCK_SIZE: usize = 4 * 1024 * 1024;

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn setup_sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create sandbox tempdir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(
        paths.is_test,
        "all v2.0.8 blackbox tests must run with paths.is_test == true"
    );
    (tmp, paths)
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

/// Build a valid Chromium Pickle 4-byte aligned Electron ASAR v1 archive:
/// - `u0 = 4`
/// - `u1 = 8 + padded_len`
/// - `u2 = 4 + padded_len`
/// - `strlen = json_len`
///
/// Followed by `padded_len` bytes (UTF-8 JSON + zero padding) and packed file payloads.
fn build_clean_antigravity_asar(app_version: &str) -> (Vec<u8>, String) {
    let mut payload = Vec::new();

    let pkg_json = format!(
        r#"{{"name":"antigravity","version":"{app_version}","main":"dist/main.js"}}"#
    );
    let menu_js = format!(
        "// Antigravity {app_version} menu\nconst {{ Menu }} = require(\"electron\");\nmodule.exports = function buildMenu() {{\n  return [{{ label: \"File\" }}, {{ label: \"Edit\" }}, {{ label: \"View\" }}, {{ label: \"Window\" }}, {{ label: \"Help\" }}];\n}};\n"
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

    let header_json = serde_json::to_string(&header_value).expect("serialize ASAR header JSON");
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
    let plist_str = render_macos_info_plist(app_version, &header_sha256);
    let plist_bytes = plist_str.into_bytes();

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
    assert_eq!(
        u2,
        4 + padded_len,
        "Pickle inner payload size u2 must equal 4 + padded_len"
    );
    assert_eq!(
        u1,
        8 + padded_len,
        "Pickle outer value u1 must equal 8 + padded_len"
    );

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
    let path_label = segments.join("/");
    let node = lookup_asar_node(&asar.header, segments)
        .unwrap_or_else(|| panic!("missing ASAR entry: {path_label}"));
    let size = node
        .get("size")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{path_label}: missing numeric size")) as usize;
    let offset_str = node
        .get("offset")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{path_label}: packed ASAR entry must have string offset"));
    let offset: usize = offset_str
        .parse()
        .unwrap_or_else(|e| panic!("{path_label}: invalid offset '{offset_str}': {e}"));

    assert!(
        offset + size <= asar.data_section.len(),
        "{path_label}: slice [{offset}..{}] out of bounds (data_len={})",
        offset + size,
        asar.data_section.len()
    );
    let data = asar.data_section[offset..offset + size].to_vec();

    if let Some(integrity) = node.get("integrity") {
        let expected_hash = integrity
            .get("hash")
            .and_then(Value::as_str)
            .expect("integrity.hash string");
        assert_eq!(
            expected_hash.to_ascii_lowercase(),
            sha256_hex(&data),
            "{path_label}: SHA-256 integrity.hash mismatch"
        );
    }
    data
}

fn read_json_object(path: &Path) -> serde_json::Map<String, Value> {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let val: Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    val.as_object()
        .cloned()
        .unwrap_or_else(|| panic!("{} must be a JSON object", path.display()))
}

fn assert_single_chinese_block(content: &str, ctx: &str) {
    assert_eq!(
        content.matches(BLOCK_START).count(),
        1,
        "{ctx}: expected exactly one {BLOCK_START} in:\n{content}"
    );
    assert_eq!(
        content.matches(BLOCK_END).count(),
        1,
        "{ctx}: expected exactly one {BLOCK_END} in:\n{content}"
    );
    let s = content.find(BLOCK_START).unwrap();
    let e = content.find(BLOCK_END).unwrap();
    assert!(s < e, "{ctx}: start marker must precede end marker");
    assert!(
        content[s..e].contains("简体中文"),
        "{ctx}: managed block must mention 简体中文"
    );
}

#[test]
fn tc_v208_01_antigravity_patch_not_installed_and_unsupported_states() {
    let (tmp, mut paths) = setup_sandbox();

    // 1) Non-existent app directory -> NotInstalled
    paths.agy_app_override_dir = Some(tmp.path().join("MissingAntigravity.app"));
    let info_missing: UiPatchInfo = inspect_antigravity_ui_patch(&paths);
    assert_eq!(info_missing.status, UiPatchStatus::NotInstalled);
    assert!(!info_missing.has_valid_backup);

    // 2) Installed app directory missing app.asar -> Unsupported
    let broken_app = tmp.path().join("BrokenAntigravity.app");
    fs::create_dir_all(broken_app.join("Contents").join("Resources")).unwrap();
    fs::write(
        broken_app.join("Contents").join("Info.plist"),
        render_macos_info_plist("2.1.0", &"0".repeat(64)),
    )
    .unwrap();
    paths.agy_app_override_dir = Some(broken_app);

    let info_unsupported = inspect_antigravity_ui_patch(&paths);
    assert_eq!(info_unsupported.status, UiPatchStatus::Unsupported);

    // 3) Non-ASAR-patched snapshots (DshTako, Codex) report NotApplicable, Claude reports NotInstalled when absent, and DshTako is natively Chinese
    for app_id in [AppId::DshTako, AppId::Codex] {
        let snap = snapshot_app(&paths, app_id);
        assert_eq!(snap.ui_patch.status, UiPatchStatus::NotApplicable);
        if app_id == AppId::DshTako {
            assert!(
                snap.chinese_configured,
                "dsh-tako must always report chinese_configured == true"
            );
        }
    }
    let claude_snap = snapshot_app(&paths, AppId::Claude);
    assert_eq!(claude_snap.ui_patch.status, UiPatchStatus::NotInstalled);
}

#[test]
fn tc_v208_02_to_05_asar_patch_idempotency_unpacked_plist_and_byte_exact_restore() {
    let (tmp, mut paths) = setup_sandbox();
    let fake_app = tmp.path().join("FakeApps").join("Antigravity.app");
    let (asar_path, plist_path, orig_asar_bytes, orig_plist_bytes, orig_header_sha256) =
        write_fake_antigravity_bundle(&fake_app, "2.1.0");
    let orig_asar_sha256 = sha256_hex(&orig_asar_bytes);
    paths.agy_app_override_dir = Some(fake_app);

    // Pre-check: clean bundle is Unpatched
    let initial_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(initial_info.status, UiPatchStatus::Unpatched);

    // (b) First patch
    patch_antigravity_ui_for_paths(&paths).expect("first patch_antigravity_ui_for_paths");
    let patched_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(patched_info.status, UiPatchStatus::Patched);
    assert!(
        patched_info.has_valid_backup,
        "Patched state must report has_valid_backup == true"
    );
    assert_eq!(
        patched_info.patch_version.as_deref(),
        Some(tako_switch_lib::adapters::antigravity_i18n::CURRENT_UI_PATCH_VERSION)
    );
    assert_eq!(
        snapshot_app(&paths, AppId::Antigravity).ui_patch.status,
        UiPatchStatus::Patched
    );

    // Verify backup `app.asar.orig` is byte-for-byte identical to original clean archive
    let ui_backup_dir = paths.backups_dir("antigravity").join("ui_patch");
    let orig_backup_path = ui_backup_dir.join("app.asar.orig");
    assert!(
        orig_backup_path.exists(),
        "app.asar.orig must exist in {:?}",
        ui_backup_dir
    );
    assert_eq!(
        fs::read(&orig_backup_path).unwrap(),
        orig_asar_bytes,
        "app.asar.orig must match original unpatched bytes"
    );

    // Inspect patched ASAR structure, Pickle alignment, guard markers, unpacked node & Info.plist hash
    let patched_asar_bytes = fs::read(&asar_path).unwrap();
    let parsed = parse_and_verify_asar_pickle(&patched_asar_bytes);
    assert_ne!(
        parsed.header_sha256, orig_header_sha256,
        "patched ASAR header SHA-256 must change after injecting i18n files"
    );

    let marker_bytes = extract_packed_asar_file(&parsed, &["__tako_i18n_patch.json"]);
    let marker_json: Value =
        serde_json::from_slice(&marker_bytes).expect("__tako_i18n_patch.json must be valid JSON");
    assert_eq!(marker_json.get("schema").and_then(Value::as_u64), Some(1));
    assert_eq!(
        marker_json.get("patchVersion").and_then(Value::as_str),
        Some(tako_switch_lib::adapters::antigravity_i18n::CURRENT_UI_PATCH_VERSION)
    );
    assert_eq!(
        marker_json
            .get("originalAsarSha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
        Some(orig_asar_sha256.clone())
    );

    for rel in [
        &["dist", "preload.js"][..],
        &["dist", "menu.js"][..],
        &["dist", "tray.js"][..],
        &["dist", "updater.js"][..],
    ] {
        let js_content =
            String::from_utf8(extract_packed_asar_file(&parsed, rel)).expect("UTF-8 JS");
        assert_eq!(
            js_content.matches(AGY_ZH_PATCH_GUARD).count(),
            1,
            "{:?} must contain exactly one guard marker",
            rel
        );
    }

    // Verify `unpacked: true` native module node is preserved without `offset`
    let unpacked_node = lookup_asar_node(
        &parsed.header,
        &["node_modules", "node-pty", "pty.node"],
    )
    .expect("unpacked native module node must be preserved");
    assert_eq!(
        unpacked_node.get("unpacked").and_then(Value::as_bool),
        Some(true)
    );
    assert!(
        unpacked_node.get("offset").is_none(),
        "unpacked node must NOT be assigned an offset: {unpacked_node}"
    );

    // Verify Info.plist ElectronAsarIntegrity hash updated to new ASAR header JSON SHA-256
    let updated_plist_str = fs::read_to_string(&plist_path).unwrap();
    assert!(
        updated_plist_str.contains(&parsed.header_sha256),
        "Info.plist must contain new ASAR header SHA-256 ({}), got:\n{}",
        parsed.header_sha256,
        updated_plist_str
    );
    assert!(
        !updated_plist_str.contains(&orig_header_sha256),
        "Info.plist must no longer contain stale pre-patch header SHA-256"
    );

    // (c) Idempotent repeated patch must NEVER overwrite `app.asar.orig` with patched archive
    patch_antigravity_ui_for_paths(&paths).expect("second idempotent patch_antigravity_ui_for_paths");
    let second_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(second_info.status, UiPatchStatus::Patched);
    assert!(second_info.has_valid_backup);
    assert_eq!(
        fs::read(&orig_backup_path).unwrap(),
        orig_asar_bytes,
        "idempotent re-patch must NEVER pollute app.asar.orig"
    );
    let reparsed = parse_and_verify_asar_pickle(&fs::read(&asar_path).unwrap());
    let preload_after_second =
        String::from_utf8(extract_packed_asar_file(&reparsed, &["dist", "preload.js"])).unwrap();
    assert_eq!(
        preload_after_second.matches(AGY_ZH_PATCH_GUARD).count(),
        1,
        "repeated patch must not stack duplicate guard blocks"
    );

    // (d) Restore must recover 100% byte-identical app.asar and Info.plist
    restore_antigravity_ui_for_paths(&paths).expect("restore_antigravity_ui_for_paths");
    assert_eq!(
        fs::read(&asar_path).unwrap(),
        orig_asar_bytes,
        "restored app.asar must be 100% byte-identical to original"
    );
    assert_eq!(
        fs::read(&plist_path).unwrap(),
        orig_plist_bytes,
        "restored Info.plist must be 100% byte-identical to original"
    );
    let restored_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(restored_info.status, UiPatchStatus::Unpatched);
}

#[test]
fn tc_v208_06_client_upgrade_restore_never_downgrades_new_asar() {
    let (tmp, mut paths) = setup_sandbox();
    let fake_app = tmp.path().join("UpgradeRestore").join("Antigravity.app");
    let (_asar_path, _plist_path, v210_asar_bytes, _v210_plist_bytes, _) =
        write_fake_antigravity_bundle(&fake_app, "2.1.0");
    paths.agy_app_override_dir = Some(fake_app.clone());

    patch_antigravity_ui_for_paths(&paths).expect("patch v2.1.0");
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Patched
    );

    // Simulate official Antigravity self-upgrade replacing bundle with clean v2.2.0
    let (asar_path, plist_path, v220_asar_bytes, v220_plist_bytes, _) =
        write_fake_antigravity_bundle(&fake_app, "2.2.0");
    assert_ne!(v210_asar_bytes, v220_asar_bytes);

    let after_upgrade = inspect_antigravity_ui_patch(&paths);
    assert_eq!(
        after_upgrade.status,
        UiPatchStatus::OutdatedAfterUpgrade,
        "clean v2.2.0 with stale patch-meta must report OutdatedAfterUpgrade"
    );

    // Calling restore in OutdatedAfterUpgrade must NEVER overwrite v2.2.0 with old v2.1.0 backup!
    restore_antigravity_ui_for_paths(&paths)
        .expect("restore on OutdatedAfterUpgrade should clean stale metadata safely");
    assert_eq!(
        fs::read(&asar_path).unwrap(),
        v220_asar_bytes,
        "restore after upgrade must NEVER downgrade v2.2.0 app.asar to v2.1.0"
    );
    assert_eq!(
        fs::read(&plist_path).unwrap(),
        v220_plist_bytes,
        "restore after upgrade must keep v2.2.0 Info.plist intact"
    );
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Unpatched
    );
}

#[test]
fn tc_v208_07_client_upgrade_repatch_refreshes_backup_to_new_version() {
    let (tmp, mut paths) = setup_sandbox();
    let fake_app = tmp.path().join("UpgradeRepatch").join("Antigravity.app");
    write_fake_antigravity_bundle(&fake_app, "2.1.0");
    paths.agy_app_override_dir = Some(fake_app.clone());

    patch_antigravity_ui_for_paths(&paths).expect("patch v2.1.0");

    // Simulate official upgrade to clean v2.2.0
    let (asar_path, plist_path, v220_asar_bytes, v220_plist_bytes, _) =
        write_fake_antigravity_bundle(&fake_app, "2.2.0");
    let v220_sha256 = sha256_hex(&v220_asar_bytes);
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::OutdatedAfterUpgrade
    );

    // Re-patching in OutdatedAfterUpgrade must refresh app.asar.orig to clean v2.2.0 first!
    patch_antigravity_ui_for_paths(&paths).expect("re-patch v2.2.0 after upgrade");
    let repatched_info = inspect_antigravity_ui_patch(&paths);
    assert_eq!(repatched_info.status, UiPatchStatus::Patched);
    assert!(repatched_info.has_valid_backup);

    let orig_backup_path = paths
        .backups_dir("antigravity")
        .join("ui_patch")
        .join("app.asar.orig");
    assert_eq!(
        fs::read(&orig_backup_path).unwrap(),
        v220_asar_bytes,
        "re-patching after upgrade must refresh app.asar.orig with clean v2.2.0 archive"
    );

    let parsed = parse_and_verify_asar_pickle(&fs::read(&asar_path).unwrap());
    let marker: Value = serde_json::from_slice(&extract_packed_asar_file(
        &parsed,
        &["__tako_i18n_patch.json"],
    ))
    .unwrap();
    assert_eq!(
        marker
            .get("originalAsarSha256")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase),
        Some(v220_sha256)
    );
    let pkg_after_repatch =
        String::from_utf8(extract_packed_asar_file(&parsed, &["package.json"])).unwrap();
    assert!(
        pkg_after_repatch.contains("2.2.0"),
        "patched archive must contain v2.2.0 payload, not old v2.1.0"
    );

    // Restoring now must recover v2.2.0 byte-for-byte
    restore_antigravity_ui_for_paths(&paths).expect("restore v2.2.0");
    assert_eq!(fs::read(&asar_path).unwrap(), v220_asar_bytes);
    assert_eq!(fs::read(&plist_path).unwrap(), v220_plist_bytes);
    assert_eq!(
        inspect_antigravity_ui_patch(&paths).status,
        UiPatchStatus::Unpatched
    );
}

#[test]
fn tc_v208_08_codex_standalone_chinese_locale_in_official_mode() {
    let (_tmp, paths) = setup_sandbox();
    fs::create_dir_all(&paths.codex_dir).unwrap();
    let cfg_path = paths.codex_dir.join("config.toml");
    let agents_path = paths.codex_dir.join("AGENTS.md");

    fs::write(
        &cfg_path,
        "model = \"gpt-5.4\"\n\n[desktop]\nlocaleOverride = \"en-US\"\nreviewDelivery = \"inline\"\n",
    )
    .unwrap();
    let user_agents = "# Custom Codex Rules\nPrefer deterministic tests.\n";
    fs::write(&agents_path, user_agents).unwrap();

    let adapter = get_adapter(AppId::Codex);
    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(!CodexAdapter::is_chinese_configured(&paths));

    // Enable standalone Chinese while staying in Official mode (twice for idempotency)
    CodexAdapter::sync_standalone_chinese_locale(&paths, true).expect("enable codex chinese");
    CodexAdapter::sync_standalone_chinese_locale(&paths, true)
        .expect("enable codex chinese idempotently");

    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(CodexAdapter::is_chinese_configured(&paths));
    assert!(snapshot_app(&paths, AppId::Codex).chinese_configured);

    let doc: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let desktop = doc.get("desktop").and_then(|i| i.as_table_like()).unwrap();
    assert_eq!(
        desktop.get("localeOverride").and_then(|v| v.as_str()),
        Some("zh-CN")
    );
    assert_eq!(
        desktop.get("reviewDelivery").and_then(|v| v.as_str()),
        Some("inline")
    );
    assert_eq!(doc.get("model").and_then(|v| v.as_str()), Some("gpt-5.4"));

    let enabled_agents = fs::read_to_string(&agents_path).unwrap();
    assert!(enabled_agents.contains("# Custom Codex Rules"));
    assert_single_chinese_block(&enabled_agents, "Codex standalone enable");

    // Disable standalone Chinese -> lossless restore
    CodexAdapter::sync_standalone_chinese_locale(&paths, false).expect("disable codex chinese");
    assert!(!CodexAdapter::is_chinese_configured(&paths));
    assert!(!snapshot_app(&paths, AppId::Codex).chinese_configured);

    let restored_doc: DocumentMut = fs::read_to_string(&cfg_path).unwrap().parse().unwrap();
    let restored_desktop = restored_doc
        .get("desktop")
        .and_then(|i| i.as_table_like())
        .unwrap();
    assert_eq!(
        restored_desktop
            .get("localeOverride")
            .and_then(|v| v.as_str()),
        Some("en-US")
    );
    assert_eq!(
        restored_desktop
            .get("reviewDelivery")
            .and_then(|v| v.as_str()),
        Some("inline")
    );
    let restored_agents = fs::read_to_string(&agents_path).unwrap();
    assert!(!restored_agents.contains(BLOCK_START));
    assert_eq!(restored_agents, user_agents);
}

#[test]
fn tc_v208_09_claude_standalone_chinese_locale_in_official_mode() {
    let (_tmp, paths) = setup_sandbox();
    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    let claude_code_dir = paths.home.join(".claude");
    fs::create_dir_all(&claude_code_dir).unwrap();

    let cfg_1p = paths.claude_1p_dir.join("config.json");
    let cfg_3p = paths.claude_3p_dir.join("config.json");
    let settings_path = claude_code_dir.join("settings.json");

    fs::write(&cfg_1p, r#"{"darkMode":"dark","locale":"en-US"}"#).unwrap();
    fs::write(&cfg_3p, r#"{"darkMode":"dark","scale":1.0}"#).unwrap();
    fs::write(
        &settings_path,
        r#"{"model":"claude-sonnet-5-5","permissions":{"allow":["Bash(git status)"]}}"#,
    )
    .unwrap();

    let adapter = get_adapter(AppId::Claude);
    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(!ClaudeAdapter::is_chinese_configured(&paths));

    ClaudeAdapter::sync_standalone_chinese_locale(&paths, true).expect("enable claude chinese");
    ClaudeAdapter::sync_standalone_chinese_locale(&paths, true)
        .expect("enable claude chinese idempotently");

    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(ClaudeAdapter::is_chinese_configured(&paths));
    assert!(snapshot_app(&paths, AppId::Claude).chinese_configured);

    let map_1p = read_json_object(&cfg_1p);
    assert_eq!(map_1p.get("locale").and_then(Value::as_str), Some("zh-Hans"));
    assert_eq!(map_1p.get("darkMode").and_then(Value::as_str), Some("dark"));

    let map_3p = read_json_object(&cfg_3p);
    assert_eq!(map_3p.get("locale").and_then(Value::as_str), Some("zh-Hans"));
    assert_eq!(map_3p.get("darkMode").and_then(Value::as_str), Some("dark"));

    let map_cc = read_json_object(&settings_path);
    assert_eq!(
        map_cc.get("language").and_then(Value::as_str),
        Some("Simplified Chinese")
    );
    assert_eq!(
        map_cc.get("model").and_then(Value::as_str),
        Some("claude-sonnet-5-5")
    );

    // Disable -> lossless restore
    ClaudeAdapter::sync_standalone_chinese_locale(&paths, false).expect("disable claude chinese");
    assert!(!ClaudeAdapter::is_chinese_configured(&paths));
    assert!(!snapshot_app(&paths, AppId::Claude).chinese_configured);

    let restored_1p = read_json_object(&cfg_1p);
    assert_eq!(
        restored_1p.get("locale").and_then(Value::as_str),
        Some("en-US")
    );
    assert_eq!(
        restored_1p.get("darkMode").and_then(Value::as_str),
        Some("dark")
    );

    let restored_3p = read_json_object(&cfg_3p);
    assert!(
        restored_3p.get("locale").is_none(),
        "originally absent 3p locale must be removed, got: {restored_3p:?}"
    );
    assert_eq!(
        restored_3p.get("darkMode").and_then(Value::as_str),
        Some("dark")
    );

    let restored_cc = read_json_object(&settings_path);
    assert!(
        restored_cc.get("language").is_none(),
        "originally absent settings.json language must be removed, got: {restored_cc:?}"
    );
    assert_eq!(
        restored_cc.get("model").and_then(Value::as_str),
        Some("claude-sonnet-5-5")
    );
}

#[test]
fn tc_v208_10_antigravity_standalone_chinese_locale_in_official_mode() {
    let (_tmp, paths) = setup_sandbox();
    let agents_path = paths.home.join(".gemini").join("AGENTS.md");
    fs::create_dir_all(agents_path.parent().unwrap()).unwrap();
    let user_rules = "# Existing Antigravity Instructions\nKeep commits atomic.\n";
    fs::write(&agents_path, user_rules).unwrap();

    let adapter = get_adapter(AppId::Antigravity);
    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(!AntigravityAdapter::is_chinese_configured(&paths));

    AntigravityAdapter::sync_standalone_chinese_locale(&paths, true).expect("enable agy chinese");
    AntigravityAdapter::sync_standalone_chinese_locale(&paths, true)
        .expect("enable agy chinese idempotently");

    assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    assert!(AntigravityAdapter::is_chinese_configured(&paths));
    assert!(snapshot_app(&paths, AppId::Antigravity).chinese_configured);

    let after_enable = fs::read_to_string(&agents_path).unwrap();
    assert!(after_enable.contains("# Existing Antigravity Instructions"));
    assert_single_chinese_block(&after_enable, "Antigravity standalone enable");

    AntigravityAdapter::sync_standalone_chinese_locale(&paths, false)
        .expect("disable agy chinese");
    assert!(!AntigravityAdapter::is_chinese_configured(&paths));
    assert!(!snapshot_app(&paths, AppId::Antigravity).chinese_configured);
    assert_eq!(fs::read_to_string(&agents_path).unwrap(), user_rules);

    // Also verify originally absent ~/.gemini/AGENTS.md is cleaned up on disable
    let (_tmp2, paths2) = setup_sandbox();
    let absent_agents = paths2.home.join(".gemini").join("AGENTS.md");
    assert!(!absent_agents.exists());
    AntigravityAdapter::sync_standalone_chinese_locale(&paths2, true).unwrap();
    assert!(AntigravityAdapter::is_chinese_configured(&paths2));
    assert!(absent_agents.exists());
    AntigravityAdapter::sync_standalone_chinese_locale(&paths2, false).unwrap();
    assert!(!AntigravityAdapter::is_chinese_configured(&paths2));
    assert!(
        !absent_agents.exists()
            || fs::read_to_string(&absent_agents)
                .unwrap_or_default()
                .trim()
                .is_empty(),
        "originally absent AGENTS.md must be removed or empty after disabling"
    );
}

#[test]
fn tc_v208_12_to_14_plaza_pricing_multiplier_request_promo_and_capabilities() {
    // Promo date boundary checks
    assert_eq!(
        resolve_promo_ends_on_at_date("gemini-4-carbon", None, "2026-11-22"),
        Some("2026-11-30".to_string())
    );
    assert_eq!(
        resolve_promo_ends_on_at_date("gemini-4-argon", None, "2026-11-22"),
        Some("2026-11-30".to_string())
    );
    assert_eq!(
        resolve_promo_ends_on_at_date("gemini-4-carbon", None, "2026-12-01"),
        None
    );
    assert_eq!(
        resolve_promo_ends_on_at_date("gemini-4-argon", None, "2026-12-01"),
        None
    );
    assert_eq!(
        resolve_promo_ends_on_at_date("gpt-6.2-sol", None, "2026-11-22"),
        None
    );

    // Build Plaza models from synthetic `/api/pricing` payload
    let pricing_body = json!({
        "group_ratio": { "default": 1.0 },
        "data": [
            {
                "model_name": "swe-2",
                "description": "旗舰代码工程与编程智能体模型",
                "vendor_name": "OpenAI",
                "quota_type": 0,
                "model_ratio": 1.25,
                "completion_ratio": 4.0,
                "cache_ratio": 0.1,
                "context_length": 1048576,
                "enable_groups": ["default"],
                "group_ratio": { "default": 1.0 },
                "group_channel_ratio_min": { "default": 1.224 },
                "group_channel_ratio_max": { "default": 2.04 },
                "standard_load_price_per_million": 3.5,
                "realized_price_per_million": 2.1,
                "cache_hit_rate": 0.6,
                "tags": ["编程", "Agent", "编程"],
                "channels": [
                    {
                        "channel_name": "特惠高速池",
                        "channel_ratio": 1.224,
                        "standard_load_price_per_million": 4.284
                    },
                    {
                        "channel_name": "标准直连池",
                        "channel_ratio": 2.04,
                        "standard_load_price_per_million": 7.14
                    }
                ]
            },
            {
                "model_name": "gemini-3.1-flash-image",
                "description": "高清图像生成与图像编辑模型",
                "vendor_name": "Google",
                "quota_type": 1,
                "model_ratio": 0.0,
                "model_price": 0.12,
                "enable_groups": ["default"],
                "group_ratio": { "default": 1.0 },
                "tags": ["生图", "Image"],
                "channels": [
                    {
                        "channel_name": "生图专线",
                        "channel_ratio": 1.0
                    }
                ]
            },
            {
                "model_name": "special-schema-model",
                "description": "带自定义计费 schema 的多维模型",
                "quota_type": 0,
                "model_ratio": 2.0,
                "completion_ratio": 2.0,
                "enable_groups": ["default"],
                "group_ratio": { "default": 1.0 },
                "billing_usage_schema": {
                    "audio_seconds": 0.05
                },
                "channels": [
                    {
                        "channel_name": "多模态池",
                        "channel_ratio": 2.04
                    }
                ]
            }
        ]
    });

    let empty_perf: HashMap<String, ModelPerfInfo> = HashMap::new();
    let plaza_resp = build_plaza_model_options(&pricing_body, &empty_perf);
    assert_eq!(plaza_resp.app_id, AppId::DshTako);
    let models = &plaza_resp.models;

    let swe = models
        .iter()
        .find(|m| m.id == "swe-2")
        .expect("swe-2 must be present in plaza options");
    assert_eq!(swe.quota_type, 0);
    assert_eq!(swe.model_ratio, Some(1.25));
    let mult = swe
        .official_multiplier
        .expect("token model without usage schema must compute official_multiplier");
    let expected_mult = 1.224 / 20.4; // 0.06
    assert!(
        (mult - expected_mult).abs() < 1e-9,
        "expected official_multiplier == {expected_mult}, got {mult}"
    );
    assert!(
        swe.cache_usd_per_m.is_some_and(|c| c > 0.0),
        "cache_usd_per_m should be populated when cache_ratio > 0, got {:?}",
        swe.cache_usd_per_m
    );
    assert!(
        swe.capabilities.iter().any(|c| c == "code"),
        "swe-2 capabilities must include 'code', got {:?}",
        swe.capabilities
    );
    assert_eq!(
        swe.tags.iter().filter(|t| t.as_str() == "编程").count(),
        1,
        "duplicate tags must be deduplicated: {:?}",
        swe.tags
    );

    let img = models
        .iter()
        .find(|m| m.id == "gemini-3.1-flash-image")
        .expect("gemini-3.1-flash-image must be present in plaza options");
    assert_eq!(img.quota_type, 1);
    assert_eq!(
        img.official_multiplier, None,
        "per-request model (quota_type == 1) must have official_multiplier == None"
    );
    let req_usd = img
        .request_usd
        .expect("per-request model must populate request_usd");
    assert!(
        (req_usd - 0.12).abs() < 1e-9,
        "expected request_usd == 0.12, got {req_usd}"
    );
    assert!(
        img.capabilities.iter().any(|c| c == "image"),
        "gemini-3.1-flash-image capabilities must include 'image', got {:?}",
        img.capabilities
    );

    let schema_model = models
        .iter()
        .find(|m| m.id == "special-schema-model")
        .expect("special-schema-model must be present");
    assert_eq!(
        schema_model.official_multiplier, None,
        "models with non-empty billing_usage_schema must have official_multiplier == None"
    );
}
