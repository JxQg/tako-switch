use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tako_switch_lib::adapters::{
    antigravity::AntigravityAdapter, claude::ClaudeAdapter, codex::CodexAdapter,
    sync_managed_chinese_markdown, TAKO_CHINESE_BLOCK_END, TAKO_CHINESE_BLOCK_START,
};
use tako_switch_lib::config::{read_backup, write_atomic, AppPaths};
use tako_switch_lib::extensions::mcp::*;
use tako_switch_lib::extensions::prompts::*;
use tako_switch_lib::extensions::{
    get_extensions_summary_for_paths, mcp_config_paths, prompt_path, read_json_store, skills_dir,
    ExtPlatform,
};
use tempfile::TempDir;
use toml_edit::DocumentMut;

fn sandbox() -> (TempDir, AppPaths) {
    let tmp = TempDir::new().unwrap();
    let paths = AppPaths::for_test(tmp.path());
    assert!(paths.is_test);
    (tmp, paths)
}

fn write(path: &Path, raw: &str) {
    write_atomic(path, raw.as_bytes(), false).unwrap();
}

fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn entry(id: &str) -> McpEntry {
    McpEntry {
        id: id.into(),
        name: format!("Server {id}"),
        spec: McpSpec {
            command: Some("npx".into()),
            args: vec!["-y".into(), "example-mcp".into()],
            ..McpSpec::default()
        },
        platforms: ExtPlatform::all().to_vec(),
        ..McpEntry::default()
    }
}

fn preset(id: &str, content: &str) -> PromptPreset {
    PromptPreset {
        id: id.into(),
        name: format!("Preset {id}"),
        content: content.into(),
    }
}

fn managed(raw: &str) -> &str {
    let start = raw.find(TAKO_CHINESE_BLOCK_START).unwrap();
    &raw[start..]
}

fn assert_body(path: &Path, expected: &str) {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let body = raw.split(TAKO_CHINESE_BLOCK_START).next().unwrap();
    assert_eq!(
        body.trim_end_matches(['\r', '\n']),
        expected.trim_end_matches(['\r', '\n'])
    );
}

#[test]
fn tc_p1_02_adopts_legacy_entries_without_rewriting_or_exposing_builtins() {
    let (_tmp, paths) = sandbox();
    let claude = paths.claude_1p_dir.join("claude_desktop_config.json");
    let codex = paths.codex_dir.join("config.toml");
    let agy = paths.agy_gemini_config_dir.join("mcp_config.json");
    let c_raw = r#"{"theme":"user","mcpServers":{"shared":{"command":"npx","args":["old"],"timeout":120},"tako":{"command":"builtin"},"tako-web-page":{"command":"builtin"}}}"#;
    let x_raw = "# keep this comment\nmodel = \"example-model\"\n[mcp_servers.shared]\ncommand = \"npx\"\nargs = [\"new\"]\n[mcp_servers.node_repl]\ncommand = \"internal\"\n[mcp_servers.tako]\ncommand = \"builtin\"\n[mcp_servers.tako-web-page]\ncommand = \"builtin\"\n[mcp_servers.remote]\nurl = \"https://example.com/mcp\"\nhttp_headers = { Authorization = \"Bearer PLACEHOLDER\" }\n";
    let a_raw = r#"{"mcpServers":{"only-agy":{"httpUrl":"https://example.com/tools"},"tako":{"command":"builtin"},"tako-web-page":{"command":"builtin"}}}"#;
    write(&claude, c_raw);
    write(&codex, x_raw);
    write(&agy, a_raw);
    write(
        &paths.home.join(".claude.json"),
        r#"{"mcpServers":{"cli-only":{"command":"uvx"}},"onboarding":true}"#,
    );
    let snapshot = get_mcp_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(snapshot.entries.len(), 4);
    assert!(snapshot
        .entries
        .iter()
        .all(|item| !is_reserved_mcp_id(&item.id)));
    assert!(
        snapshot
            .entries
            .iter()
            .find(|item| item.id == "shared")
            .unwrap()
            .has_diff
    );
    assert_eq!(
        snapshot
            .entries
            .iter()
            .find(|item| item.id == "remote")
            .unwrap()
            .spec
            .headers["Authorization"],
        "Bearer PLACEHOLDER"
    );
    assert_eq!(fs::read_to_string(&claude).unwrap(), c_raw);
    assert_eq!(fs::read_to_string(&codex).unwrap(), x_raw);
    assert_eq!(fs::read_to_string(&agy).unwrap(), a_raw);
    let persisted: McpStore = read_json_store(&mcp_store_path(&paths));
    assert_eq!(persisted.entries.len(), 4);
    for id in ["tako", "tako-web-page", "node_repl", "TAKO"] {
        assert!(upsert_mcp_entry_for_paths(&paths, entry(id)).is_err());
        assert!(delete_mcp_entry_for_paths(&paths, id).is_err());
    }
    assert!(!paths.dsh_tako_dir.exists());
}

#[test]
fn tc_p1_03_three_claude_files_toggle_and_all_off_keeps_central_record() {
    let (_tmp, paths) = sandbox();
    fs::create_dir_all(&paths.claude_3p_dir).unwrap();
    let cli = paths.home.join(".claude.json");
    let original = r#"{"oauthAccount":{"user":"kept"},"projects":{"/work":{"trusted":true}},"mcpServers":{"tako":{"command":"builtin"}}}"#;
    write(&cli, original);
    write(
        &paths.codex_dir.join("config.toml"),
        "# custom comment\nmodel = \"kept\"\n[mcp_servers.node_repl]\ncommand = \"internal\"\n",
    );
    upsert_mcp_entry_for_paths(&paths, entry("docs")).unwrap();
    for path in mcp_config_paths(&paths, ExtPlatform::Claude) {
        assert_eq!(json_file(&path)["mcpServers"]["docs"]["command"], "npx");
    }
    assert_eq!(json_file(&cli)["oauthAccount"]["user"], "kept");
    assert_eq!(json_file(&cli)["projects"]["/work"]["trusted"], true);
    assert_eq!(
        read_backup(&paths, "claude", "claude_code_mcp.json").unwrap(),
        original.as_bytes()
    );
    assert!(fs::read_to_string(paths.codex_dir.join("config.toml"))
        .unwrap()
        .contains("# custom comment"));
    for &platform in ExtPlatform::all() {
        toggle_mcp_platform_for_paths(&paths, "docs", platform, false).unwrap();
    }
    let snapshot = get_mcp_hub_snapshot_for_paths(&paths).unwrap();
    let docs = snapshot
        .entries
        .iter()
        .find(|item| item.id == "docs")
        .unwrap();
    assert!(docs.platforms.is_empty());
    for path in mcp_config_paths(&paths, ExtPlatform::Claude)
        .into_iter()
        .chain(mcp_config_paths(&paths, ExtPlatform::Antigravity))
    {
        assert!(json_file(&path)["mcpServers"].get("docs").is_none());
    }
    let document = fs::read_to_string(paths.codex_dir.join("config.toml"))
        .unwrap()
        .parse::<DocumentMut>()
        .unwrap();
    assert!(document["mcp_servers"].get("docs").is_none());
    assert!(document["mcp_servers"].get("node_repl").is_some());
    toggle_mcp_platform_for_paths(&paths, "docs", ExtPlatform::Claude, true).unwrap();
    assert!(json_file(&cli)["mcpServers"].get("docs").is_some());
    delete_mcp_entry_for_paths(&paths, "docs").unwrap();
    assert!(get_mcp_hub_snapshot_for_paths(&paths)
        .unwrap()
        .entries
        .is_empty());
    assert_eq!(json_file(&cli)["mcpServers"]["tako"]["command"], "builtin");
    assert!(!paths.dsh_tako_dir.exists());
}

#[test]
fn tc_p1_03_no_third_party_claude_directory_is_created_in_official_mode() {
    let (_tmp, paths) = sandbox();
    let mut item = entry("only-claude");
    item.platforms = vec![ExtPlatform::Claude];
    upsert_mcp_entry_for_paths(&paths, item).unwrap();
    assert!(!paths.claude_3p_dir.exists());
    assert!(paths.home.join(".claude.json").exists());
}

#[test]
fn tc_p1_04_parse_json_toml_and_restore_backup_to_all_platforms() {
    let (_tmp, paths) = sandbox();
    let snippets = [
        r#"{"mcpServers":{"context7":{"command":"npx","args":["-y","@upstash/context7-mcp"],"env":{"DEBUG":"1"}}}}"#,
        r#"{"id":"remote","url":"https://example.com/mcp","headers":{"Authorization":"Bearer PLACEHOLDER"}}"#,
        "[mcp_servers.fetch]\ncommand = \"uvx\"\nargs = [\"mcp-server-fetch\"]\n[mcp_servers.fetch.env]\nDEBUG = \"1\"\n",
        r#"{"command":"uvx","args":["single-server"]}"#,
        "[mcp_servers.remote-toml]\nurl = \"https://example.com/mcp\"\nhttp_headers = { Authorization = \"Bearer PLACEHOLDER\" }\n",
    ];
    for snippet in snippets {
        for mut item in parse_mcp_snippet(snippet).unwrap() {
            item.platforms = ExtPlatform::all().to_vec();
            upsert_mcp_entry_for_paths(&paths, item).unwrap();
        }
    }
    let exported = export_mcp_backup_for_paths(&paths).unwrap();
    let ids = get_mcp_hub_snapshot_for_paths(&paths)
        .unwrap()
        .entries
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    for id in ids {
        delete_mcp_entry_for_paths(&paths, &id).unwrap();
    }
    let restored = import_mcp_backup_for_paths(&paths, &exported).unwrap();
    assert_eq!(restored.entries.len(), 5);
    assert!(restored
        .entries
        .iter()
        .all(|item| item.platforms.len() == 3 && !item.has_diff));
    let codex = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
    assert!(codex.contains("http_headers"));
    assert!(codex.contains("PLACEHOLDER"));
    for invalid in [
        "{}",
        "not toml",
        r#"{"mcpServers":{"tako":{"command":"x"}}}"#,
        r#"{"command":"x","args":[1]}"#,
    ] {
        assert!(parse_mcp_snippet(invalid).is_err(), "{invalid}");
    }
    assert!(import_mcp_backup_for_paths(&paths, "{}").is_err());
    assert!(!paths.dsh_tako_dir.exists());
}

#[test]
fn tc_p1_04_single_platform_toggle_preserves_other_platform_diff_and_unknown_fields() {
    let (_tmp, paths) = sandbox();
    let claude = paths.claude_1p_dir.join("claude_desktop_config.json");
    write(
        &claude,
        r#"{"mcpServers":{"shared":{"command":"claude-command","customTimeout":123}}}"#,
    );
    write(
        &paths.codex_dir.join("config.toml"),
        "# keep\n[mcp_servers.shared]\ncommand = \"codex-command\"\nstartup_timeout_sec = 45\n",
    );
    get_mcp_hub_snapshot_for_paths(&paths).unwrap();
    toggle_mcp_platform_for_paths(&paths, "shared", ExtPlatform::Antigravity, true).unwrap();
    assert!(fs::read_to_string(paths.codex_dir.join("config.toml"))
        .unwrap()
        .contains("codex-command"));
    let aligned = align_mcp_entry_for_paths(&paths, "shared", Some(ExtPlatform::Codex)).unwrap();
    assert!(!aligned.entries[0].has_diff);
    assert_eq!(
        json_file(&claude)["mcpServers"]["shared"]["command"],
        "codex-command"
    );
    assert_eq!(
        json_file(&claude)["mcpServers"]["shared"]["startup_timeout_sec"],
        45
    );
    assert_eq!(
        json_file(&claude)["mcpServers"]["shared"]["customTimeout"],
        123
    );
}

#[test]
fn tc_p1_04_invalid_target_is_rejected_before_any_client_file_is_written() {
    let (_tmp, paths) = sandbox();
    let claude = paths.claude_1p_dir.join("claude_desktop_config.json");
    let original = r#"{"keep":true}"#;
    write(&claude, original);
    write(&paths.codex_dir.join("config.toml"), "[broken");
    assert!(upsert_mcp_entry_for_paths(&paths, entry("fail-safely")).is_err());
    assert_eq!(fs::read_to_string(claude).unwrap(), original);
    assert!(!paths.home.join(".claude.json").exists());
    assert!(!get_mcp_hub_snapshot_for_paths(&paths)
        .unwrap()
        .entries
        .iter()
        .any(|item| item.id == "fail-safely"));
}

#[test]
fn tc_p1_04_backup_restores_distinct_platform_parameters_without_forcing_alignment() {
    let (_tmp, paths) = sandbox();
    write(
        &paths.claude_1p_dir.join("claude_desktop_config.json"),
        r#"{"mcpServers":{"shared":{"command":"claude-command"}}}"#,
    );
    write(
        &paths.codex_dir.join("config.toml"),
        "[mcp_servers.shared]\ncommand = \"codex-command\"\n",
    );
    let backup = export_mcp_backup_for_paths(&paths).unwrap();
    delete_mcp_entry_for_paths(&paths, "shared").unwrap();
    let restored = import_mcp_backup_for_paths(&paths, &backup).unwrap();
    assert!(restored.entries[0].has_diff);
    assert_eq!(
        restored.entries[0].variants[&ExtPlatform::Claude]
            .command
            .as_deref(),
        Some("claude-command")
    );
    assert_eq!(
        restored.entries[0].variants[&ExtPlatform::Codex]
            .command
            .as_deref(),
        Some("codex-command")
    );
}

#[test]
fn tc_p1_04_native_service_names_and_toml_comments_survive_updates() {
    let (_tmp, paths) = sandbox();
    let path = paths.codex_dir.join("config.toml");
    write(&path, "# top-level\n[mcp_servers.\"Docs service\"]\ncommand = \"npx\" # command-note\nargs = [\"old\"] # args-note\n[mcp_servers.\"Docs service\".env]\nDEBUG = \"1\" # env-note\n");
    let mut item = get_mcp_hub_snapshot_for_paths(&paths)
        .unwrap()
        .entries
        .remove(0);
    item.spec.args = vec!["new".into()];
    upsert_mcp_entry_for_paths(&paths, item).unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    for comment in ["# top-level", "# command-note", "# args-note", "# env-note"] {
        assert!(raw.contains(comment), "{comment}: {raw}");
    }
    assert!(raw.contains("[mcp_servers.\"Docs service\".env]"));
    let snapshot = get_mcp_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(snapshot.entries[0].spec.args, vec!["new"]);
}

#[test]
fn tc_p1_04_noop_central_operations_do_not_reformat_native_files() {
    let (_tmp, paths) = sandbox();
    let path = paths.claude_1p_dir.join("claude_desktop_config.json");
    let original = "{\"keep\": true, \"mcpServers\": {}}\n";
    write(&path, original);
    let mut item = entry("disabled");
    item.platforms.clear();
    upsert_mcp_entry_for_paths(&paths, item).unwrap();
    align_all_mcp_entries_for_paths(&paths).unwrap();
    delete_mcp_entry_for_paths(&paths, "disabled").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!paths.home.join(".claude.json").exists());
}

#[test]
fn tc_p1_05_prompt_operations_preserve_managed_block_byte_for_byte_on_all_platforms() {
    let (_tmp, paths) = sandbox();
    let block = format!(
        "{TAKO_CHINESE_BLOCK_START}\r\ncustom managed rule\r\n{TAKO_CHINESE_BLOCK_END}\r\n"
    );
    for &platform in ExtPlatform::all() {
        let path = prompt_path(&paths, platform);
        write(&path, &block);
        for (id, body) in [
            ("first", "# First\nKeep this rule."),
            ("second", "# Second\n\u{4e2d}\u{6587} body."),
        ] {
            upsert_prompt_preset_for_paths(&paths, platform, preset(id, body)).unwrap();
            activate_prompt_preset_for_paths(&paths, platform, Some(id)).unwrap();
            assert_body(&path, body);
            assert_eq!(managed(&fs::read_to_string(&path).unwrap()), block);
        }
        upsert_prompt_preset_for_paths(
            &paths,
            platform,
            preset("second", "# Saved\nChanged body."),
        )
        .unwrap();
        assert_body(&path, "# Saved\nChanged body.");
        assert_eq!(managed(&fs::read_to_string(&path).unwrap()), block);
        activate_prompt_preset_for_paths(&paths, platform, None).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), block);
        activate_prompt_preset_for_paths(&paths, platform, Some("first")).unwrap();
        delete_prompt_preset_for_paths(&paths, platform, "first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), block);
    }
}

#[test]
fn tc_p1_05_locale_toggles_and_prompt_switches_preserve_each_others_regions() {
    let (_tmp, paths) = sandbox();
    for &platform in ExtPlatform::all() {
        let path = prompt_path(&paths, platform);
        upsert_prompt_preset_for_paths(
            &paths,
            platform,
            preset("active", "# User rules\nNever discard this content."),
        )
        .unwrap();
        activate_prompt_preset_for_paths(&paths, platform, Some("active")).unwrap();
        for enabled in [true, false, true, false, true] {
            match platform {
                ExtPlatform::Claude => {
                    ClaudeAdapter::sync_standalone_chinese_locale(&paths, enabled).unwrap()
                }
                ExtPlatform::Codex => {
                    CodexAdapter::sync_standalone_chinese_locale(&paths, enabled).unwrap()
                }
                ExtPlatform::Antigravity => {
                    AntigravityAdapter::sync_standalone_chinese_locale(&paths, enabled).unwrap()
                }
            }
            assert_body(&path, "# User rules\nNever discard this content.");
            if platform != ExtPlatform::Claude {
                assert_eq!(
                    fs::read_to_string(&path)
                        .unwrap()
                        .contains(TAKO_CHINESE_BLOCK_START),
                    enabled
                );
            }
            upsert_prompt_preset_for_paths(
                &paths,
                platform,
                preset("active", "# User rules\nNever discard this content."),
            )
            .unwrap();
        }
        sync_managed_chinese_markdown(
            &paths,
            "claude",
            "prompt-test.md",
            &path,
            true,
            "managed rule",
        )
        .unwrap();
        assert_body(&path, "# User rules\nNever discard this content.");
        sync_managed_chinese_markdown(&paths, "claude", "prompt-test.md", &path, false, "")
            .unwrap();
        assert_body(&path, "# User rules\nNever discard this content.");
    }
    assert!(!paths.dsh_tako_dir.exists());
}

#[test]
fn tc_p1_06_archives_handwritten_rules_tracks_external_edits_and_syncs_selected_targets() {
    let (_tmp, paths) = sandbox();
    let original = "# Handwritten\n\u{4fdd}\u{7559}\u{539f}\u{59cb}\u{5185}\u{5bb9}\u{3002}\n";
    let source = prompt_path(&paths, ExtPlatform::Claude);
    write(&source, original);
    let initial = get_prompts_hub_snapshot_for_paths(&paths).unwrap();
    let imported = &initial.platforms[&ExtPlatform::Claude].presets[0];
    assert_eq!(imported.name, "导入的原始提示词备份");
    assert_eq!(imported.content, original);
    assert_eq!(
        read_backup(&paths, "claude", "extensions-original-prompt.md").unwrap(),
        original.as_bytes()
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    let id = imported.id.clone();
    let external = "# External change\n\u{5916}\u{90e8}\u{7f16}\u{8f91}\u{3002}\n";
    write(&source, external);
    let live = get_prompts_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(
        live.platforms[&ExtPlatform::Claude].presets[0].content,
        external
    );
    assert_eq!(
        live.platforms[&ExtPlatform::Claude].live_bytes,
        external.len()
    );
    let saved = sync_prompt_to_platforms_for_paths(
        &paths,
        ExtPlatform::Claude,
        &id,
        &[ExtPlatform::Codex],
        false,
    )
    .unwrap();
    assert_eq!(saved.platforms[&ExtPlatform::Codex].presets.len(), 1);
    assert!(saved.platforms[&ExtPlatform::Codex].active_id.is_none());
    assert!(!prompt_path(&paths, ExtPlatform::Codex).exists());
    assert!(saved.platforms[&ExtPlatform::Antigravity]
        .presets
        .is_empty());
    let synced = sync_prompt_to_platforms_for_paths(
        &paths,
        ExtPlatform::Claude,
        &id,
        &[ExtPlatform::Codex, ExtPlatform::Antigravity],
        true,
    )
    .unwrap();
    for &target in &[ExtPlatform::Codex, ExtPlatform::Antigravity] {
        assert_eq!(synced.platforms[&target].presets.len(), 1);
        assert!(synced.platforms[&target].active_id.is_some());
        assert_eq!(
            fs::read_to_string(prompt_path(&paths, target)).unwrap(),
            external
        );
    }
    let next = sync_prompt_to_platforms_for_paths(
        &paths,
        ExtPlatform::Claude,
        &id,
        &[ExtPlatform::Codex, ExtPlatform::Codex],
        false,
    )
    .unwrap();
    assert_eq!(next.platforms[&ExtPlatform::Codex].presets.len(), 1);
    let backup = export_prompts_backup_for_paths(&paths).unwrap();
    delete_prompt_preset_for_paths(&paths, ExtPlatform::Codex, &id).unwrap();
    let restored = import_prompts_backup_for_paths(&paths, &backup).unwrap();
    assert_eq!(
        restored.platforms[&ExtPlatform::Codex].presets[0].content,
        external
    );
    assert_eq!(
        fs::read_to_string(prompt_path(&paths, ExtPlatform::Codex)).unwrap(),
        external
    );
    assert!(!paths.dsh_tako_dir.exists());
}

#[test]
fn tc_p1_06_summary_scans_skills_and_stores_tolerate_missing_legacy_fields() {
    let (_tmp, paths) = sandbox();
    write(
        &mcp_store_path(&paths),
        r#"{"entries":[{"id":"legacy","spec":{"command":"uvx"}}]}"#,
    );
    write(&prompts_store_path(&paths), r#"{"platforms":{"codex":{}}}"#);
    write(
        &skills_dir(&paths, ExtPlatform::Claude)
            .join("sample")
            .join("SKILL.md"),
        "# Skill",
    );
    write(
        &skills_dir(&paths, ExtPlatform::Codex)
            .join("sample")
            .join("SKILL.md"),
        "# Skill",
    );
    let summary = get_extensions_summary_for_paths(&paths).unwrap();
    assert_eq!(summary.mcp_count, 1);
    assert_eq!(summary.skills_count, 1);
    assert_eq!(summary.plugins_count, 0);
    assert_eq!(summary.prompts_count, 0);
    assert_eq!(serde_json::to_value(&summary).unwrap()["mcpCount"], 1);
    write(&prompts_store_path(&paths), "not json");
    assert!(get_prompts_hub_snapshot_for_paths(&paths)
        .unwrap()
        .platforms
        .values()
        .all(|state| state.presets.is_empty()));
    assert!(
        parse_mcp_snippet(&json!({"mcpServers":{"node_repl":{"command":"x"}}}).to_string())
            .is_err()
    );
}
