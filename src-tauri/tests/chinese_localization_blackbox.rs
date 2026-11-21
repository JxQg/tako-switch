use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
use tako_switch_lib::adapters::{get_adapter, AppId, AppMode};
use tako_switch_lib::config::{
    read_switch_state, save_switch_auth, save_switch_state, AppPaths,
};
use tempfile::TempDir;
use toml_edit::DocumentMut;

const BLOCK_START: &str = "<!-- tako-switch:chinese-ui:start -->";
const BLOCK_END: &str = "<!-- tako-switch:chinese-ui:end -->";

fn setup_logged_in_paths(prefer_chinese_ui: bool) -> (TempDir, AppPaths) {
    let tmp = TempDir::new().expect("create temp dir");
    let paths = AppPaths::for_test(tmp.path());
    assert!(paths.is_test, "blackbox tests must run in is_test sandbox");
    save_switch_auth(&paths, Some("cr_blackbox_test_token"), Some("1"), Some("oauth"))
        .expect("save test auth token");
    set_prefer_chinese_ui(&paths, prefer_chinese_ui);
    (tmp, paths)
}

fn set_prefer_chinese_ui(paths: &AppPaths, enabled: bool) {
    let mut state = read_switch_state(paths);
    state.prefer_chinese_ui = enabled;
    save_switch_state(paths, &state).expect("save switch state");
}

fn read_toml_doc(path: &Path) -> DocumentMut {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    raw.parse::<DocumentMut>()
        .unwrap_or_else(|e| panic!("parse toml {}: {e}", path.display()))
}

fn read_json_map(path: &Path) -> serde_json::Map<String, JsonValue> {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let val: JsonValue = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse json {}: {e}", path.display()));
    val.as_object()
        .cloned()
        .unwrap_or_else(|| panic!("{} must be a JSON object", path.display()))
}

fn assert_single_managed_chinese_block(content: &str, context: &str) {
    assert_eq!(
        content.matches(BLOCK_START).count(),
        1,
        "{context}: expected exactly 1 start marker in:\n{content}"
    );
    assert_eq!(
        content.matches(BLOCK_END).count(),
        1,
        "{context}: expected exactly 1 end marker in:\n{content}"
    );
    let start_idx = content.find(BLOCK_START).expect("start marker index");
    let end_idx = content.find(BLOCK_END).expect("end marker index");
    assert!(
        start_idx < end_idx,
        "{context}: start marker must appear before end marker"
    );
    let inner = &content[start_idx + BLOCK_START.len()..end_idx];
    assert!(
        inner.contains("简体中文"),
        "{context}: managed block must contain 简体中文, got:\n{inner}"
    );
}

fn assert_no_managed_chinese_block(content: &str, context: &str) {
    assert!(
        !content.contains(BLOCK_START),
        "{context}: start marker still present in:\n{content}"
    );
    assert!(
        !content.contains(BLOCK_END),
        "{context}: end marker still present in:\n{content}"
    );
}

#[test]
fn tc_codex_01_chinese_ui_enabled_preserves_desktop_and_agents_idempotently() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.codex_dir).unwrap();
    fs::write(
        paths.codex_dir.join("config.toml"),
        r#"model = "gpt-5.4"

[desktop]
reviewDelivery = "inline"
"#,
    )
    .unwrap();
    let user_agents = "# User Custom Rules\nAlways explain architecture decisions concisely.\n";
    fs::write(paths.codex_dir.join("AGENTS.md"), user_agents).unwrap();

    let adapter = get_adapter(AppId::Codex);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    // Repeated apply_mode(Tako) must be idempotent and never stack duplicate blocks.
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let doc = read_toml_doc(&paths.codex_dir.join("config.toml"));
    let desktop = doc
        .get("desktop")
        .and_then(|i| i.as_table_like())
        .expect("[desktop] table must exist");
    assert_eq!(
        desktop.get("localeOverride").and_then(|v| v.as_str()),
        Some("zh-CN")
    );
    assert_eq!(
        desktop.get("reviewDelivery").and_then(|v| v.as_str()),
        Some("inline")
    );
    assert_eq!(doc.get("model").and_then(|v| v.as_str()), Some("gpt-5.4"));

    let agents_raw = fs::read_to_string(paths.codex_dir.join("AGENTS.md")).unwrap();
    assert!(agents_raw.contains("# User Custom Rules"));
    assert!(agents_raw.contains("Always explain architecture decisions concisely."));
    assert_single_managed_chinese_block(&agents_raw, "TC-CODEX-01");

    let diag = adapter.diagnose(&paths);
    assert!(
        diag.notes.iter().any(|n| n.contains("中文增强: 已启用")),
        "expected diagnose notes to report 中文增强: 已启用, got: {:?}",
        diag.notes
    );
}

#[test]
fn tc_codex_02_disable_chinese_ui_and_official_removes_injected_locale_and_block() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.codex_dir).unwrap();
    fs::write(
        paths.codex_dir.join("config.toml"),
        r#"model = "gpt-5.4"

[desktop]
reviewDelivery = "inline"
"#,
    )
    .unwrap();
    let user_agents = "# Keep My Codex Rules\nDo not touch this paragraph.\n";
    fs::write(paths.codex_dir.join("AGENTS.md"), user_agents).unwrap();

    let adapter = get_adapter(AppId::Codex);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    // Case 1: prefer_chinese_ui = false while remaining in Tako mode.
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let doc_disabled = read_toml_doc(&paths.codex_dir.join("config.toml"));
    let desktop_disabled = doc_disabled
        .get("desktop")
        .and_then(|i| i.as_table_like())
        .expect("[desktop] must remain because reviewDelivery was present");
    assert!(desktop_disabled.get("localeOverride").is_none());
    assert_eq!(
        desktop_disabled.get("reviewDelivery").and_then(|v| v.as_str()),
        Some("inline")
    );
    let agents_disabled = fs::read_to_string(paths.codex_dir.join("AGENTS.md")).unwrap();
    assert_no_managed_chinese_block(&agents_disabled, "TC-CODEX-02 prefer_chinese_ui=false");
    assert!(agents_disabled.contains("# Keep My Codex Rules"));
    assert!(agents_disabled.contains("Do not touch this paragraph."));

    // Case 2: Re-enable Chinese UI, then switch back to Official.
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();

    let doc_off = read_toml_doc(&paths.codex_dir.join("config.toml"));
    let desktop_off = doc_off
        .get("desktop")
        .and_then(|i| i.as_table_like())
        .expect("[desktop] must keep reviewDelivery");
    assert!(desktop_off.get("localeOverride").is_none());
    assert_eq!(
        desktop_off.get("reviewDelivery").and_then(|v| v.as_str()),
        Some("inline")
    );
    let agents_off = fs::read_to_string(paths.codex_dir.join("AGENTS.md")).unwrap();
    assert_no_managed_chinese_block(&agents_off, "TC-CODEX-02 Official");
    assert!(agents_off.contains("# Keep My Codex Rules"));
    assert!(agents_off.contains("Do not touch this paragraph."));
}

#[test]
fn tc_codex_03_restores_original_ja_jp_locale_override() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.codex_dir).unwrap();
    fs::write(
        paths.codex_dir.join("config.toml"),
        r#"[desktop]
localeOverride = "ja-JP"
reviewDelivery = "inline"
"#,
    )
    .unwrap();

    let adapter = get_adapter(AppId::Codex);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    let doc_tako = read_toml_doc(&paths.codex_dir.join("config.toml"));
    assert_eq!(
        doc_tako["desktop"]["localeOverride"].as_str(),
        Some("zh-CN")
    );

    // Disable Chinese UI -> restores ja-JP
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    let doc_restored_tako = read_toml_doc(&paths.codex_dir.join("config.toml"));
    assert_eq!(
        doc_restored_tako["desktop"]["localeOverride"].as_str(),
        Some("ja-JP")
    );
    assert_eq!(
        doc_restored_tako["desktop"]["reviewDelivery"].as_str(),
        Some("inline")
    );

    // Re-enable Chinese UI then switch Official -> restores ja-JP
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();
    let doc_official = read_toml_doc(&paths.codex_dir.join("config.toml"));
    assert_eq!(
        doc_official["desktop"]["localeOverride"].as_str(),
        Some("ja-JP")
    );
    assert_eq!(
        doc_official["desktop"]["reviewDelivery"].as_str(),
        Some("inline")
    );
}

#[test]
fn tc_codex_04_absent_config_and_agents_cleaned_up_on_disable_and_official() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    let config_path = paths.codex_dir.join("config.toml");
    let agents_path = paths.codex_dir.join("AGENTS.md");
    assert!(!config_path.exists());
    assert!(!agents_path.exists());

    let adapter = get_adapter(AppId::Codex);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    let doc_tako = read_toml_doc(&config_path);
    assert_eq!(
        doc_tako["desktop"]["localeOverride"].as_str(),
        Some("zh-CN")
    );
    assert!(agents_path.exists());

    // Disable Chinese UI in Tako mode -> no desktop.localeOverride or empty desktop table; AGENTS.md removed or empty.
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    let doc_disabled = read_toml_doc(&config_path);
    assert!(
        doc_disabled.get("desktop").is_none(),
        "empty [desktop] table must not remain when originally absent"
    );
    assert!(
        !agents_path.exists()
            || fs::read_to_string(&agents_path)
                .unwrap()
                .trim()
                .is_empty(),
        "absent AGENTS.md must be deleted or cleaned up when Chinese UI disabled"
    );

    // Re-enable Chinese UI, then switch to Official -> temporary config.toml & AGENTS.md cleaned up.
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();

    if config_path.exists() {
        let doc_off = read_toml_doc(&config_path);
        assert!(
            doc_off.get("desktop").is_none() && doc_off.as_table().is_empty(),
            "temporary config.toml must not contain residual tables: {}",
            doc_off
        );
    }
    assert!(
        !agents_path.exists()
            || fs::read_to_string(&agents_path)
                .unwrap()
                .trim()
                .is_empty(),
        "absent AGENTS.md must be deleted or cleaned up on Official"
    );
}

#[test]
fn tc_claude_01_chinese_ui_enabled_updates_1p_3p_and_claude_code_settings() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(paths.home.join(".claude")).unwrap();

    let cfg_3p = paths.claude_3p_dir.join("config.json");
    let cfg_1p = paths.claude_1p_dir.join("config.json");
    let cc_settings = paths.home.join(".claude").join("settings.json");

    fs::write(&cfg_3p, r#"{"darkMode":"dark","customFlag":true}"#).unwrap();
    fs::write(&cfg_1p, r#"{"darkMode":"dark","windowBounds":{"width":1200}}"#).unwrap();
    fs::write(
        &cc_settings,
        r#"{"model":"claude-sonnet-5-5","permissions":{"allow":["Bash(git status)"]}}"#,
    )
    .unwrap();

    let adapter = get_adapter(AppId::Claude);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let map_3p = read_json_map(&cfg_3p);
    assert_eq!(
        map_3p.get("locale").and_then(JsonValue::as_str),
        Some("zh-Hans")
    );
    assert_eq!(
        map_3p.get("darkMode").and_then(JsonValue::as_str),
        Some("dark")
    );
    assert_eq!(
        map_3p.get("customFlag").and_then(JsonValue::as_bool),
        Some(true)
    );

    let map_1p = read_json_map(&cfg_1p);
    assert_eq!(
        map_1p.get("locale").and_then(JsonValue::as_str),
        Some("zh-Hans")
    );
    assert_eq!(
        map_1p.get("darkMode").and_then(JsonValue::as_str),
        Some("dark")
    );
    assert_eq!(
        map_1p
            .get("windowBounds")
            .and_then(|v| v.get("width"))
            .and_then(JsonValue::as_u64),
        Some(1200)
    );

    let map_cc = read_json_map(&cc_settings);
    assert_eq!(
        map_cc.get("language").and_then(JsonValue::as_str),
        Some("Simplified Chinese")
    );
    assert_eq!(
        map_cc.get("model").and_then(JsonValue::as_str),
        Some("claude-sonnet-5-5")
    );
    let allow = map_cc
        .get("permissions")
        .and_then(JsonValue::as_object)
        .and_then(|p| p.get("allow"))
        .and_then(JsonValue::as_array)
        .expect("permissions.allow array must exist");
    assert!(allow.iter().any(|v| v.as_str() == Some("Bash(git status)")));
    assert!(allow.iter().any(|v| v.as_str() == Some("mcp__tako__*")));

    let diag = adapter.diagnose(&paths);
    assert!(
        diag.notes.iter().any(|n| n.contains("中文增强: 已启用")),
        "expected diagnose notes to report 中文增强: 已启用, got: {:?}",
        diag.notes
    );
}

#[test]
fn tc_claude_02_disable_and_official_removes_injected_keys_when_absent_in_backup() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(paths.home.join(".claude")).unwrap();

    let cfg_3p = paths.claude_3p_dir.join("config.json");
    let cfg_1p = paths.claude_1p_dir.join("config.json");
    let cc_settings = paths.home.join(".claude").join("settings.json");

    fs::write(&cfg_3p, r#"{"darkMode":"dark"}"#).unwrap();
    fs::write(&cfg_1p, r#"{"darkMode":"light"}"#).unwrap();
    fs::write(
        &cc_settings,
        r#"{"model":"claude-sonnet-5-5","permissions":{"allow":["Bash(git status)"]}}"#,
    )
    .unwrap();

    let adapter = get_adapter(AppId::Claude);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    // Toggle prefer_chinese_ui = false in Tako mode
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let map_3p_disabled = read_json_map(&cfg_3p);
    assert!(map_3p_disabled.get("locale").is_none());
    assert_eq!(
        map_3p_disabled.get("darkMode").and_then(JsonValue::as_str),
        Some("dark")
    );
    let map_1p_disabled = read_json_map(&cfg_1p);
    assert!(map_1p_disabled.get("locale").is_none());
    assert_eq!(
        map_1p_disabled.get("darkMode").and_then(JsonValue::as_str),
        Some("light")
    );
    let map_cc_disabled = read_json_map(&cc_settings);
    assert!(map_cc_disabled.get("language").is_none());
    assert_eq!(
        map_cc_disabled.get("model").and_then(JsonValue::as_str),
        Some("claude-sonnet-5-5")
    );
    let allow_disabled = map_cc_disabled["permissions"]["allow"].as_array().unwrap();
    assert!(allow_disabled.iter().any(|v| v.as_str() == Some("mcp__tako__*")));

    // Re-enable Chinese UI, then switch to Official
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();

    let map_3p_off = read_json_map(&cfg_3p);
    assert!(map_3p_off.get("locale").is_none());
    assert_eq!(
        map_3p_off.get("darkMode").and_then(JsonValue::as_str),
        Some("dark")
    );
    let map_1p_off = read_json_map(&cfg_1p);
    assert!(map_1p_off.get("locale").is_none());
    assert_eq!(
        map_1p_off.get("darkMode").and_then(JsonValue::as_str),
        Some("light")
    );
    let map_cc_off = read_json_map(&cc_settings);
    assert!(map_cc_off.get("language").is_none());
    let allow_off = map_cc_off["permissions"]["allow"].as_array().unwrap();
    assert!(allow_off.iter().any(|v| v.as_str() == Some("Bash(git status)")));
    assert!(!allow_off.iter().any(|v| v.as_str() == Some("mcp__tako__*")));
}

#[test]
fn tc_claude_03_restores_original_locale_and_language_values() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    fs::create_dir_all(&paths.claude_1p_dir).unwrap();
    fs::create_dir_all(paths.home.join(".claude")).unwrap();

    let cfg_3p = paths.claude_3p_dir.join("config.json");
    let cfg_1p = paths.claude_1p_dir.join("config.json");
    let cc_settings = paths.home.join(".claude").join("settings.json");

    fs::write(&cfg_3p, r#"{"locale":"en-US","darkMode":"light"}"#).unwrap();
    fs::write(&cfg_1p, r#"{"locale":"fr-FR","theme":"system"}"#).unwrap();
    fs::write(
        &cc_settings,
        r#"{"language":"Japanese","permissions":{"allow":["Bash(ls)"]}}"#,
    )
    .unwrap();

    let adapter = get_adapter(AppId::Claude);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    assert_eq!(read_json_map(&cfg_3p)["locale"].as_str(), Some("zh-Hans"));
    assert_eq!(read_json_map(&cfg_1p)["locale"].as_str(), Some("zh-Hans"));
    assert_eq!(
        read_json_map(&cc_settings)["language"].as_str(),
        Some("Simplified Chinese")
    );

    // Toggle prefer_chinese_ui = false in Tako mode -> exact original values restored
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let m3_tako_off = read_json_map(&cfg_3p);
    assert_eq!(m3_tako_off["locale"].as_str(), Some("en-US"));
    assert_eq!(m3_tako_off["darkMode"].as_str(), Some("light"));
    let m1_tako_off = read_json_map(&cfg_1p);
    assert_eq!(m1_tako_off["locale"].as_str(), Some("fr-FR"));
    assert_eq!(m1_tako_off["theme"].as_str(), Some("system"));
    let mcc_tako_off = read_json_map(&cc_settings);
    assert_eq!(mcc_tako_off["language"].as_str(), Some("Japanese"));

    // Re-enable Chinese UI, then switch to Official -> exact original values restored
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();

    assert_eq!(read_json_map(&cfg_3p)["locale"].as_str(), Some("en-US"));
    assert_eq!(read_json_map(&cfg_1p)["locale"].as_str(), Some("fr-FR"));
    assert_eq!(
        read_json_map(&cc_settings)["language"].as_str(),
        Some("Japanese")
    );
}

#[test]
fn tc_claude_04_absent_configs_created_on_tako_and_cleaned_on_official() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    let cfg_3p = paths.claude_3p_dir.join("config.json");
    let cfg_1p = paths.claude_1p_dir.join("config.json");
    let cc_settings = paths.home.join(".claude").join("settings.json");
    assert!(!cfg_3p.exists());
    assert!(!cfg_1p.exists());
    assert!(!cc_settings.exists());

    let adapter = get_adapter(AppId::Claude);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    assert_eq!(read_json_map(&cfg_3p)["locale"].as_str(), Some("zh-Hans"));
    assert_eq!(read_json_map(&cfg_1p)["locale"].as_str(), Some("zh-Hans"));
    assert_eq!(
        read_json_map(&cc_settings)["language"].as_str(),
        Some("Simplified Chinese")
    );

    adapter.apply_mode(&paths, AppMode::Official).unwrap();
    if cfg_3p.exists() {
        assert!(read_json_map(&cfg_3p).get("locale").is_none());
    }
    if cfg_1p.exists() {
        assert!(read_json_map(&cfg_1p).get("locale").is_none());
    }
    if cc_settings.exists() {
        assert!(read_json_map(&cc_settings).get("language").is_none());
    }
}

#[test]
fn tc_agy_01_chinese_ui_enabled_injects_agents_md_idempotently() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    let gemini_dir = paths.home.join(".gemini");
    fs::create_dir_all(&gemini_dir).unwrap();
    let agents_path = gemini_dir.join("AGENTS.md");
    let user_rules = "# My Gemini Rules\nPrefer concise diffs and strict tests.\n";
    fs::write(&agents_path, user_rules).unwrap();

    let adapter = get_adapter(AppId::Antigravity);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();

    let raw = fs::read_to_string(&agents_path).unwrap();
    assert!(raw.contains("# My Gemini Rules"));
    assert!(raw.contains("Prefer concise diffs and strict tests."));
    assert_single_managed_chinese_block(&raw, "TC-AGY-01");

    let diag = adapter.diagnose(&paths);
    assert!(
        diag.notes.iter().any(|n| n.contains("中文增强: 已启用")),
        "expected diagnose notes to report 中文增强: 已启用, got: {:?}",
        diag.notes
    );
}

#[test]
fn tc_agy_02_disable_and_official_strips_block_preserving_user_rules() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    let gemini_dir = paths.home.join(".gemini");
    fs::create_dir_all(&gemini_dir).unwrap();
    let agents_path = gemini_dir.join("AGENTS.md");
    let user_rules = "# Personal Instructions\nKeep existing behavior intact.\n";
    fs::write(&agents_path, user_rules).unwrap();

    let adapter = get_adapter(AppId::Antigravity);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    assert_single_managed_chinese_block(&fs::read_to_string(&agents_path).unwrap(), "TC-AGY-02 enable");

    // Disable Chinese UI while staying in Tako mode
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    let after_disable = fs::read_to_string(&agents_path).unwrap();
    assert_no_managed_chinese_block(&after_disable, "TC-AGY-02 prefer_chinese_ui=false");
    assert!(after_disable.contains("# Personal Instructions"));
    assert!(after_disable.contains("Keep existing behavior intact."));

    // Re-enable Chinese UI then switch to Official
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    adapter.apply_mode(&paths, AppMode::Official).unwrap();
    let after_official = fs::read_to_string(&agents_path).unwrap();
    assert_no_managed_chinese_block(&after_official, "TC-AGY-02 Official");
    assert!(after_official.contains("# Personal Instructions"));
    assert!(after_official.contains("Keep existing behavior intact."));
}

#[test]
fn tc_agy_03_absent_agents_md_auto_deleted_on_disable_and_official() {
    let (_tmp, paths) = setup_logged_in_paths(true);
    let agents_path = paths.home.join(".gemini").join("AGENTS.md");
    assert!(!agents_path.exists());

    let adapter = get_adapter(AppId::Antigravity);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    assert!(agents_path.exists());
    assert_single_managed_chinese_block(&fs::read_to_string(&agents_path).unwrap(), "TC-AGY-03 enable");

    // Disable Chinese UI in Tako mode -> absent file must be auto-deleted
    set_prefer_chinese_ui(&paths, false);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    assert!(
        !agents_path.exists(),
        "~/.gemini/AGENTS.md should be auto-deleted when originally absent and Chinese UI disabled"
    );

    // Re-enable Chinese UI then switch to Official -> absent file must be auto-deleted
    set_prefer_chinese_ui(&paths, true);
    adapter.apply_mode(&paths, AppMode::Tako).unwrap();
    assert!(agents_path.exists());
    adapter.apply_mode(&paths, AppMode::Official).unwrap();
    assert!(
        !agents_path.exists(),
        "~/.gemini/AGENTS.md should be auto-deleted when originally absent and switched to Official"
    );
}

#[test]
fn tc_cross_01_all_adapters_stay_within_test_sandbox() {
    let (tmp, paths) = setup_logged_in_paths(true);
    for app_id in [AppId::Codex, AppId::Claude, AppId::Antigravity] {
        let adapter = get_adapter(app_id);
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
    }
    assert!(paths.home.starts_with(tmp.path()));
    assert!(paths.tako_dir.starts_with(tmp.path()));
    assert!(paths.codex_dir.starts_with(tmp.path()));
    assert!(paths.claude_1p_dir.starts_with(tmp.path()));
    assert!(paths.claude_3p_dir.starts_with(tmp.path()));
    assert!(paths.agy_gemini_config_dir.starts_with(tmp.path()));
    assert!(paths.launch_agents_dir.starts_with(tmp.path()));
}
