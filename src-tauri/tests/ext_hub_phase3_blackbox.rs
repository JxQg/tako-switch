use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tako_switch_lib::config::{
    read_backup, read_switch_state, save_switch_state, write_atomic, AppPaths,
};
use tako_switch_lib::extensions::plugins::{
    detect_plugin_cli_for_paths, detect_plugin_cli_with_overrides,
    execute_plugin_action_with_overrides, extract_cli_json, get_plugins_hub_snapshot_for_paths,
    get_plugins_hub_snapshot_with_overrides, parse_plugin_jsonc, PluginAction, PluginCliSource,
    PluginTestOverrides,
};
use tako_switch_lib::extensions::ExtPlatform;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

fn sandbox() -> (TempDir, AppPaths) {
    let temporary = tempfile::Builder::new()
        .prefix("phase3 sandbox ")
        .tempdir()
        .unwrap();
    let paths = AppPaths::for_test(temporary.path());
    assert!(paths.is_test);
    (temporary, paths)
}

fn write(path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
    write_atomic(path.as_ref(), content.as_ref(), false).unwrap();
}

fn write_json(path: impl AsRef<Path>, value: &Value) {
    write(path, serde_json::to_vec(value).unwrap());
}

fn fake_cli(directory: &Path, name: &str, installed: &Value) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    write_json(directory.join("installed.json"), installed);
    write_json(
        directory.join("available.json"),
        &json!({"installed": [], "available": []}),
    );
    write_json(directory.join("marketplaces.json"), &json!([]));
    #[cfg(windows)]
    let (file_name, script) = (
        format!("{name}.cmd"),
        r#"@echo off
setlocal
set trace=%~2
if "%~2"=="marketplace" set trace=marketplace-%~3
>>"%~dp0calls-%trace%.log" echo %*
>"%~dp0environment-%trace%.log" echo %HOME%
>>"%~dp0environment-%trace%.log" echo %CODEX_HOME%
>>"%~dp0environment-%trace%.log" echo %CLAUDE_CONFIG_DIR%
>>"%~dp0environment-%trace%.log" echo %GEMINI_CONFIG_DIR%
if exist "%~dp0error.txt" (
  type "%~dp0error.txt" 1>&2
  exit /b 7
)
if not "%~1"=="plugin" exit /b 90
if "%~2"=="marketplace" (
  if "%~3"=="list" type "%~dp0marketplaces.json"
  exit /b 0
)
if "%~2"=="list" (
  if "%~3"=="--available" (
    echo Checking marketplace catalogs...
    type "%~dp0available.json"
  ) else (
    echo [info] Reading installed plugins...
    type "%~dp0installed.json"
  )
  exit /b 0
)
if "%~2"=="enable" if exist "%~dp0enabled.json" copy /y "%~dp0enabled.json" "%~dp0installed.json" >nul
if "%~2"=="disable" if exist "%~dp0disabled.json" copy /y "%~dp0disabled.json" "%~dp0installed.json" >nul
if "%~2"=="install" if exist "%~dp0after-install.json" copy /y "%~dp0after-install.json" "%~dp0installed.json" >nul
echo Operation completed
exit /b 0
"#,
    );
    #[cfg(not(windows))]
    let (file_name, script) = (
        name.to_string(),
        r#"#!/bin/sh
base="${0%/*}"
trace="$2"
if [ "$2" = marketplace ]; then trace="marketplace-$3"; fi
printf '%s\n' "$*" >> "$base/calls-$trace.log"
printf '%s\n' "$HOME" "$CODEX_HOME" "$CLAUDE_CONFIG_DIR" "$GEMINI_CONFIG_DIR" > "$base/environment-$trace.log"
if [ -f "$base/error.txt" ]; then /bin/cat "$base/error.txt" >&2; exit 7; fi
if [ "$1" != plugin ]; then exit 90; fi
if [ "$2" = marketplace ]; then
  if [ "$3" = list ]; then /bin/cat "$base/marketplaces.json"; fi
  exit 0
fi
if [ "$2" = list ]; then
  printf '%s\n' '[info] Reading plugin catalog...'
  if [ "$3" = --available ]; then /bin/cat "$base/available.json"; else /bin/cat "$base/installed.json"; fi
  exit 0
fi
if [ "$2" = enable ] && [ -f "$base/enabled.json" ]; then /bin/cp "$base/enabled.json" "$base/installed.json"; fi
if [ "$2" = disable ] && [ -f "$base/disabled.json" ]; then /bin/cp "$base/disabled.json" "$base/installed.json"; fi
if [ "$2" = install ] && [ -f "$base/after-install.json" ]; then /bin/cp "$base/after-install.json" "$base/installed.json"; fi
printf '%s\n' 'Operation completed'
"#,
    );
    let path = directory.join(file_name);
    write(
        &path,
        script.replace('\n', if cfg!(windows) { "\r\n" } else { "\n" }),
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

fn overrides(cli: PathBuf) -> PluginTestOverrides {
    PluginTestOverrides {
        builtin_cli_paths: vec![cli],
        ..Default::default()
    }
}

fn calls(cli: &Path) -> String {
    let mut logs: Vec<_> = fs::read_dir(cli.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("calls-"))
        .map(|entry| entry.path())
        .collect();
    logs.sort();
    logs.iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        .replace('"', "")
}

#[tokio::test]
async fn tc_p3_01_no_cli_returns_normal_empty_snapshot_without_host_path_fallback() {
    let (_temporary, paths) = sandbox();
    for platform in ExtPlatform::all() {
        let snapshot = get_plugins_hub_snapshot_for_paths(&paths, *platform, true)
            .await
            .unwrap();
        assert_eq!(snapshot.platform, *platform);
        assert!(!snapshot.cli_available);
        assert!(snapshot.cli_path.is_none());
        assert!(snapshot.cli_source.is_none());
        assert!(
            snapshot.installed.is_empty()
                && snapshot.available.is_empty()
                && snapshot.marketplaces.is_empty()
        );
        assert_eq!(
            snapshot.marketplaces_supported,
            *platform != ExtPlatform::Antigravity
        );
    }
    assert!(!paths.tako_dir.exists());
}

#[tokio::test]
async fn tc_p3_01_builtin_beats_user_cli_and_path_for_all_platforms() {
    let (_temporary, paths) = sandbox();
    for (platform, name) in [
        (ExtPlatform::Claude, "claude"),
        (ExtPlatform::Codex, "codex"),
        (ExtPlatform::Antigravity, "agy"),
    ] {
        let builtin = fake_cli(
            &paths.home.join(name).join("builtin with spaces"),
            name,
            &json!([]),
        );
        let user = fake_cli(&paths.home.join(name).join("user"), name, &json!([]));
        let path_cli = fake_cli(&paths.home.join(name).join("path"), name, &json!([]));
        let mut options = PluginTestOverrides {
            builtin_cli_paths: vec![builtin.clone()],
            user_cli_paths: vec![user.clone()],
            path_dirs: vec![path_cli.parent().unwrap().to_path_buf()],
            ..Default::default()
        };
        assert_eq!(
            detect_plugin_cli_with_overrides(&paths, platform, &options)
                .unwrap()
                .unwrap()
                .path,
            builtin
        );
        let snapshot = get_plugins_hub_snapshot_with_overrides(&paths, platform, false, &options)
            .await
            .unwrap();
        assert_eq!(snapshot.cli_source, Some(PluginCliSource::Builtin));
        assert!(calls(&builtin).contains("list"));
        assert!(calls(&path_cli).is_empty());
        options.builtin_cli_paths.clear();
        let detected_user = detect_plugin_cli_with_overrides(&paths, platform, &options)
            .unwrap()
            .unwrap();
        assert_eq!(detected_user.path, user);
        assert_eq!(detected_user.source, PluginCliSource::Path);
        options.user_cli_paths.clear();
        assert_eq!(
            detect_plugin_cli_with_overrides(&paths, platform, &options)
                .unwrap()
                .unwrap()
                .path,
            path_cli
        );
    }
}

#[test]
fn tc_p3_01_sandbox_native_install_locations_and_app_override_are_probed() {
    let (_temporary, mut paths) = sandbox();
    let app = paths.home.join("Antigravity Desktop");
    let builtin = fake_cli(&app.join("resources/app/bin"), "agy", &json!([]));
    paths.agy_app_override_dir = Some(app);
    assert_eq!(
        detect_plugin_cli_for_paths(&paths, ExtPlatform::Antigravity)
            .unwrap()
            .unwrap()
            .path,
        builtin
    );
    let user_cli = fake_cli(&paths.home.join(".local/bin"), "claude", &json!([]));
    assert_eq!(
        detect_plugin_cli_for_paths(&paths, ExtPlatform::Claude)
            .unwrap()
            .unwrap()
            .path,
        user_cli
    );
}

#[tokio::test]
async fn tc_p3_02_claude_lists_manifests_scope_actions_and_marketplaces() {
    let (_temporary, paths) = sandbox();
    let install_path = paths.home.join("installed/alpha");
    write_json(
        install_path.join(".claude-plugin/plugin.json"),
        &json!({
            "name": "alpha", "displayName": "Alpha Display", "description": "Manifest description", "category": "Development", "version": "1.2.3"
        }),
    );
    let installed = json!([
        {"id": "alpha@official", "enabled": true, "scope": "user", "installPath": install_path},
        {"id": "project-only@official", "enabled": true, "scope": "project"},
        {"id": "legacy@official", "enabled": false}
    ]);
    let cli = fake_cli(&paths.home.join("fake claude"), "claude", &installed);
    let base = cli.parent().unwrap();
    write_json(
        base.join("available.json"),
        &json!({"installed": installed, "available": [
            {"id": "alpha@official"}, {"name": "beta", "marketplace": "official", "description": "New plugin"}
        ]}),
    );
    write_json(
        base.join("marketplaces.json"),
        &json!({"marketplaces": [
            {"name": "official", "source": {"source": "github", "repo": "example/plugins"}}
        ]}),
    );
    let options = overrides(cli.clone());
    let snapshot =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Claude, true, &options)
            .await
            .unwrap();
    let alpha = snapshot
        .installed
        .iter()
        .find(|entry| entry.name == "alpha")
        .unwrap();
    assert_eq!(alpha.display_name, "Alpha Display");
    assert_eq!(alpha.description, "Manifest description");
    assert_eq!(alpha.category.as_deref(), Some("Development"));
    assert_eq!(alpha.version.as_deref(), Some("1.2.3"));
    assert!(alpha.manageable && alpha.removable);
    let project = snapshot
        .installed
        .iter()
        .find(|entry| entry.name == "project-only")
        .unwrap();
    assert!(!project.manageable && !project.removable);
    assert!(
        snapshot
            .installed
            .iter()
            .find(|entry| entry.name == "legacy")
            .unwrap()
            .manageable
    );
    assert_eq!(snapshot.available.len(), 1);
    assert_eq!(snapshot.available[0].id, "beta@official");
    assert_eq!(snapshot.marketplaces[0].source, "example/plugins");

    for action in [
        PluginAction::Toggle(false),
        PluginAction::Toggle(true),
        PluginAction::Update,
        PluginAction::Uninstall,
    ] {
        execute_plugin_action_with_overrides(
            &paths,
            ExtPlatform::Claude,
            action,
            "alpha@official",
            &options,
        )
        .await
        .unwrap();
    }
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::Install,
        "beta@official",
        &options,
    )
    .await
    .unwrap();
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::AddMarketplace,
        "example/plugins",
        &options,
    )
    .await
    .unwrap();
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::RemoveMarketplace,
        "official",
        &options,
    )
    .await
    .unwrap();
    let trace = calls(&cli);
    for expected in [
        "plugin disable",
        "plugin enable",
        "plugin update",
        "plugin uninstall",
        "plugin install",
        "marketplace add",
        "marketplace remove",
    ] {
        assert!(trace.contains(expected), "missing {expected}: {trace}");
    }
    let before = trace;
    let error = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::Uninstall,
        "project-only@official",
        &options,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_SCOPE_READ_ONLY");
    assert_eq!(
        calls(&cli).matches("plugin uninstall").count(),
        before.matches("plugin uninstall").count()
    );
    let environment = fs::read_to_string(base.join("environment-list.log")).unwrap();
    assert!(environment.contains(paths.home.to_str().unwrap()));
    assert!(environment.contains(paths.home.join(".claude").to_str().unwrap()));
}

#[tokio::test]
async fn tc_p3_02_codex_enabled_uses_lossless_toml_and_one_time_backup() {
    let (_temporary, paths) = sandbox();
    let original = "# user header\nmodel = \"example-model\"\n\n[plugins.\"alpha@official\"]\n# keep plugin comment\nenabled = true # keep inline comment\ncustom = \"unchanged\"\n\n[mcp_servers.custom]\ncommand = \"custom-mcp\" # keep mcp comment\n";
    write(paths.codex_dir.join("config.toml"), original);
    let cli = fake_cli(
        &paths.home.join("fake codex"),
        "codex",
        &json!({"installed": [{"id": "alpha@official", "enabled": false}]}),
    );
    write_json(
        cli.parent().unwrap().join("available.json"),
        &json!({"installed": [{"id": "alpha@official"}], "available": [{"id": "beta@official"}]}),
    );
    write_json(
        cli.parent().unwrap().join("marketplaces.json"),
        &json!([{"name": "official", "source": "https://example.com/plugins.json"}]),
    );
    let options = overrides(cli.clone());
    let snapshot =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Codex, true, &options)
            .await
            .unwrap();
    assert!(
        snapshot.installed[0].enabled,
        "TOML must override stale CLI state"
    );
    assert_eq!(snapshot.available[0].id, "beta@official");
    assert_eq!(snapshot.marketplaces[0].name, "official");
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Codex,
        PluginAction::Toggle(false),
        "alpha@official",
        &options,
    )
    .await
    .unwrap();
    let disabled = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
    assert_eq!(
        disabled,
        original.replacen("enabled = true", "enabled = false", 1)
    );
    assert_eq!(
        read_backup(&paths, "codex", "config.toml").unwrap(),
        original.as_bytes()
    );
    assert!(
        !get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Codex, false, &options)
            .await
            .unwrap()
            .installed[0]
            .enabled
    );
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Codex,
        PluginAction::Toggle(true),
        "alpha@official",
        &options,
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap(),
        original
    );
    assert_eq!(
        read_backup(&paths, "codex", "config.toml").unwrap(),
        original.as_bytes()
    );
    assert!(!calls(&cli).contains("plugin enable") && !calls(&cli).contains("plugin disable"));
    let environment =
        fs::read_to_string(cli.parent().unwrap().join("environment-list.log")).unwrap();
    assert!(environment
        .lines()
        .any(|line| line.trim_matches('"') == paths.codex_dir.to_str().unwrap()));
    for (action, argument) in [
        (PluginAction::Install, "beta@official"),
        (PluginAction::Update, "alpha@official"),
        (PluginAction::Uninstall, "alpha@official"),
        (
            PluginAction::AddMarketplace,
            "https://example.com/catalog.json",
        ),
        (PluginAction::RemoveMarketplace, "official"),
    ] {
        execute_plugin_action_with_overrides(
            &paths,
            ExtPlatform::Codex,
            action,
            argument,
            &options,
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn tc_p3_02_codex_inline_table_new_plugin_and_invalid_toml_are_preserved() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(
        &paths.home.join("codex"),
        "codex",
        &json!([{"id": "alpha@official"}, {"id": "beta@official"}]),
    );
    let options = overrides(cli);
    write(
        paths.codex_dir.join("config.toml"),
        "# header\nplugins = { \"alpha@official\" = { enabled = true, extra = 7 } } # inline\n",
    );
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Codex,
        PluginAction::Toggle(false),
        "alpha@official",
        &options,
    )
    .await
    .unwrap();
    let updated = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
    assert!(updated.contains("enabled = false, extra = 7") && updated.contains("# inline"));
    write(
        paths.codex_dir.join("config.toml"),
        "# fresh config\nmodel = \"example\"\n",
    );
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Codex,
        PluginAction::Toggle(true),
        "beta@official",
        &options,
    )
    .await
    .unwrap();
    let created = fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
    assert!(created.contains("# fresh config") && created.contains("[plugins.\"beta@official\"]"));
    let invalid = "# invalid config must not be overwritten\n[plugins";
    write(paths.codex_dir.join("config.toml"), invalid);
    assert!(execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Codex,
        PluginAction::Toggle(true),
        "beta@official",
        &options
    )
    .await
    .is_err());
    assert_eq!(
        fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap(),
        invalid
    );
}

async fn catalog_server() -> (
    String,
    oneshot::Sender<()>,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let fail_partners = Arc::new(AtomicBool::new(false));
    let failing = fail_partners.clone();
    let (stop, mut stopped) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = tokio::select! {
                accepted = listener.accept() => accepted.unwrap(),
                _ = &mut stopped => break,
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut request = [0; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let partners = request.starts_with("GET /partners");
            let status = if partners && failing.load(Ordering::SeqCst) {
                "503 Unavailable"
            } else {
                "200 OK"
            };
            let body = if partners {
                json!({"plugins": [{"name": "partner", "displayName": "Partner Plugin", "description": "Partner catalog"}]})
            } else {
                json!({"plugins": [{"name": "alpha"}, {"name": "new-plugin", "description": "Official catalog"}]})
            }.to_string();
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (url, stop, hits, fail_partners)
}

#[tokio::test]
async fn tc_p3_03_antigravity_jsonc_two_catalogs_offline_cache_and_authorization() {
    let (_temporary, paths) = sandbox();
    write(
        paths
            .agy_gemini_config_dir
            .join("plugins/-12345/.antigravity-plugin/plugin.json"),
        r#"{
      // Older installations keep a numeric directory.
      "name": "alpha",
      "displayName": "Alpha Desktop",
      "version": "2.0.0",
      "description": "https://example.com/path//literal /* string */",
      /* category metadata */ "category": "Productivity",
      "nested": [1, 2,],
    }"#,
    );
    let installed = json!({"plugins": [{"name": "-12345", "enabled": false, "marketplace": "antigravity-plugins-official"}]});
    let cli = fake_cli(&paths.home.join("fake agy"), "agy", &installed);
    let (base_url, stop, hits, fail_partners) = catalog_server().await;
    let options = PluginTestOverrides {
        builtin_cli_paths: vec![cli.clone()],
        catalog_urls: Some([
            format!("{base_url}/official"),
            format!("{base_url}/partners"),
        ]),
        ..Default::default()
    };
    let snapshot =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, true, &options)
            .await
            .unwrap();
    assert!(!snapshot.marketplaces_supported && snapshot.marketplaces.is_empty());
    assert!(!snapshot.catalog_from_cache);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    let alpha = &snapshot.installed[0];
    assert_eq!(alpha.id, "alpha@antigravity-plugins-official");
    assert_eq!(alpha.dir_name.as_deref(), Some("-12345"));
    assert_eq!(alpha.display_name, "Alpha Desktop");
    assert_eq!(alpha.version.as_deref(), Some("2.0.0"));
    assert_eq!(
        alpha.description,
        "https://example.com/path//literal /* string */"
    );
    assert_eq!(alpha.category.as_deref(), Some("Productivity"));
    assert!(alpha.needs_configuration);
    assert_eq!(
        snapshot.available.len(),
        2,
        "installed alpha must be excluded"
    );
    assert!(snapshot
        .available
        .iter()
        .any(|entry| entry.id == "partner@antigravity-plugins-partners"));
    assert!(paths
        .switch_dir()
        .join("extensions/plugin_cache.json")
        .exists());

    let outcome = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Install,
        &alpha.id,
        &options,
    )
    .await
    .unwrap();
    assert!(outcome.needs_configuration);
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Update,
        &alpha.id,
        &options,
    )
    .await
    .unwrap();
    write_json(
        cli.parent().unwrap().join("enabled.json"),
        &json!({"plugins": [{"name": "-12345", "enabled": true, "marketplace": "antigravity-plugins-official"}]}),
    );
    write_json(cli.parent().unwrap().join("disabled.json"), &installed);
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Toggle(true),
        "-12345",
        &options,
    )
    .await
    .unwrap();
    assert!(
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, false, &options)
            .await
            .unwrap()
            .installed[0]
            .enabled
    );
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Toggle(false),
        "-12345",
        &options,
    )
    .await
    .unwrap();
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Uninstall,
        "-12345",
        &options,
    )
    .await
    .unwrap();
    let trace = calls(&cli).replace('"', "");
    for expected in [
        "plugin enable -12345",
        "plugin disable -12345",
        "plugin uninstall -12345",
        "plugin install alpha@antigravity-plugins-official",
    ] {
        assert!(trace.contains(expected), "{trace}");
    }
    assert!(!trace.contains("plugin update"));
    let before = calls(&cli);
    assert_eq!(
        execute_plugin_action_with_overrides(
            &paths,
            ExtPlatform::Antigravity,
            PluginAction::AddMarketplace,
            "example/plugins",
            &options
        )
        .await
        .unwrap_err()
        .error_code,
        "PLUGIN_UNSUPPORTED"
    );
    assert_eq!(before, calls(&cli));

    fail_partners.store(true, Ordering::SeqCst);
    let partial =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, true, &options)
            .await
            .unwrap();
    assert!(partial.catalog_from_cache);
    assert_eq!(partial.available, snapshot.available);
    assert_eq!(
        partial.warnings.len(),
        1,
        "one failed catalog must keep its previous cache"
    );

    stop.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let offline =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, true, &options)
            .await
            .unwrap();
    assert!(offline.catalog_from_cache);
    assert_eq!(offline.available, snapshot.available);
    assert_eq!(offline.warnings.len(), 2);
    let mut state = read_switch_state(&paths);
    state.bypass_system_proxy = false;
    save_switch_state(&paths, &state).unwrap();
    assert!(!read_switch_state(&paths).bypass_system_proxy);
}

#[test]
fn tc_p3_03_jsonc_strings_escapes_comments_arrays_and_malformed_input() {
    let parsed = parse_plugin_jsonc(
        r#"{
      "url": "https://example.com//test", // comment
      "quoted": "say \"hello\" // literal",
      "slash": "a\\", /* comment */
      "array": [{"key": 1,}, 2,],
    }"#,
    )
    .unwrap();
    assert_eq!(parsed["url"], "https://example.com//test");
    assert_eq!(parsed["quoted"], "say \"hello\" // literal");
    assert_eq!(parsed["slash"], "a\\");
    assert_eq!(parsed["array"][0]["key"], 1);
    assert!(parse_plugin_jsonc("{ /* unterminated").is_err());
    assert!(parse_plugin_jsonc("{\"bad\": invalid}").is_err());
    assert!(parse_plugin_jsonc("{\"comma\": 1,,}").is_err());
}

#[tokio::test]
async fn tc_p3_04_invalid_arguments_are_rejected_before_any_cli_execution() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(&paths.home.join("fake"), "claude", &json!([]));
    let options = overrides(cli.clone());
    for platform in ExtPlatform::all() {
        for id in [
            "-malicious",
            "; rm -rf",
            "has space",
            "name|command",
            "name&command",
            "\"quoted\"",
            "../traversal",
            "name@-flag",
            "name@@market",
            "",
            "a%PATH%",
            "bad\nname",
        ] {
            for action in [
                PluginAction::Install,
                PluginAction::Update,
                PluginAction::Uninstall,
                PluginAction::Toggle(true),
                PluginAction::RemoveMarketplace,
            ] {
                let error =
                    execute_plugin_action_with_overrides(&paths, *platform, action, id, &options)
                        .await
                        .unwrap_err();
                assert_eq!(
                    error.error_code, "PLUGIN_INVALID_ARGUMENT",
                    "{platform:?} {action:?} {id:?}"
                );
            }
        }
    }
    for source in [
        "-flag",
        "owner/repo;evil",
        "https://example.com/a&b",
        "https://user:secret@example.com/repo",
        "file:///etc/config",
        "owner/../repo",
    ] {
        assert_eq!(
            execute_plugin_action_with_overrides(
                &paths,
                ExtPlatform::Claude,
                PluginAction::AddMarketplace,
                source,
                &options
            )
            .await
            .unwrap_err()
            .error_code,
            "PLUGIN_INVALID_ARGUMENT"
        );
    }
    assert!(calls(&cli).is_empty());
    assert!(!paths.switch_dir().exists());
}

#[tokio::test]
async fn tc_p3_04_locked_files_and_terminal_confirmation_have_typed_chinese_errors() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(&paths.home.join("fake"), "claude", &json!([]));
    let options = overrides(cli.clone());
    for message in [
        "EBUSY: directory locked",
        "EPERM operation not permitted",
        "being used by another process",
        "Access is denied",
        "拒绝访问",
    ] {
        write(cli.parent().unwrap().join("error.txt"), message);
        let error = execute_plugin_action_with_overrides(
            &paths,
            ExtPlatform::Claude,
            PluginAction::Install,
            "alpha@official",
            &options,
        )
        .await
        .unwrap_err();
        assert_eq!(error.error_code, "PLUGIN_IN_USE");
        assert!(error.message.contains("关闭运行中的 Claude 客户端会话"));
        assert!(error.detail.unwrap().contains(message));
    }
    write(
        cli.parent().unwrap().join("error.txt"),
        "marketplace requires interactive confirmation; run in a terminal to trust source",
    );
    let error = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::Install,
        "alpha@official",
        &options,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_NEEDS_TERMINAL_CONFIRM");
    assert!(error.message.contains("终端"));
    let wire = serde_json::to_value(error).unwrap();
    assert!(wire.get("errorCode").is_some());
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetOEMCP() -> u32;
        }
        if unsafe { GetOEMCP() } == 936 {
            write(
                cli.parent().unwrap().join("error.txt"),
                [0xbe, 0xdc, 0xbe, 0xf8, 0xb7, 0xc3, 0xce, 0xca],
            );
            let error = execute_plugin_action_with_overrides(
                &paths,
                ExtPlatform::Claude,
                PluginAction::Install,
                "alpha@official",
                &options,
            )
            .await
            .unwrap_err();
            assert_eq!(error.error_code, "PLUGIN_IN_USE");
            assert!(error.detail.unwrap().contains("拒绝访问"));
        }
    }
}

#[tokio::test]
async fn tc_p3_04_sandbox_overrides_cannot_escape_or_use_external_catalogs() {
    let (_temporary, paths) = sandbox();
    let options = PluginTestOverrides {
        builtin_cli_paths: vec![paths.home.join("../outside.cmd")],
        ..Default::default()
    };
    assert_eq!(
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Claude, false, &options)
            .await
            .unwrap_err()
            .error_code,
        "PLUGIN_TEST_BOUNDARY"
    );
    let options = PluginTestOverrides {
        catalog_urls: Some([
            "https://example.com/catalog".into(),
            "https://example.com/partners".into(),
        ]),
        ..Default::default()
    };
    assert_eq!(
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, true, &options)
            .await
            .unwrap_err()
            .error_code,
        "PLUGIN_TEST_BOUNDARY"
    );
    let mut not_test = paths.clone();
    not_test.is_test = false;
    assert_eq!(
        detect_plugin_cli_with_overrides(
            &not_test,
            ExtPlatform::Codex,
            &PluginTestOverrides::default()
        )
        .unwrap_err()
        .error_code,
        "PLUGIN_TEST_BOUNDARY"
    );
}

#[tokio::test]
async fn tc_p3_04_cli_timeout_is_bounded_and_stdin_is_not_interactive() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(&paths.home.join("slow"), "claude", &json!([]));
    #[cfg(windows)]
    write(&cli, "@echo off\r\n:wait\r\ngoto wait\r\n");
    #[cfg(not(windows))]
    write(&cli, "#!/bin/sh\nwhile :; do :; done\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let options = PluginTestOverrides {
        short_timeout: Some(Duration::from_millis(150)),
        ..overrides(cli)
    };
    let started = std::time::Instant::now();
    let error =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Claude, false, &options)
            .await
            .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_CLI_TIMEOUT");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn tc_p3_02_cli_json_preamble_last_line_and_default_cache_upgrade() {
    for output in [
        "[{\"name\":\"alpha\"}]",
        "CLI notice\n[{\"name\":\"alpha\"}]",
        "[info] notice\n{\n \"plugins\": []\n}\ncompleted",
    ] {
        assert!(extract_cli_json(output).is_ok(), "{output}");
    }
    assert!(extract_cli_json("no json output").is_err());
    let pretty = extract_cli_json("Loading...\n{\n  \"installed\": [\n    {\"id\":\"alpha@official\"}\n  ],\n  \"available\": [\n    {\"id\":\"beta@official\"}\n  ]\n}\n").unwrap();
    assert_eq!(pretty["installed"][0]["id"], "alpha@official");
    assert_eq!(pretty["available"][0]["id"], "beta@official");
    let entry: tako_switch_lib::extensions::plugins::PluginEntry =
        serde_json::from_str("{}").unwrap();
    assert_eq!(entry, Default::default());
}

#[tokio::test]
async fn tc_p3_03_antigravity_all_manifest_locations_and_unmapped_numeric_directories() {
    let (_temporary, paths) = sandbox();
    let folders = [
        "",
        ".antigravity-plugin",
        ".gemini-plugin",
        ".claude-plugin",
    ];
    for (index, folder) in folders.into_iter().enumerate() {
        write_json(
            paths
                .agy_gemini_config_dir
                .join("plugins")
                .join(format!("plugin-{index}"))
                .join(folder)
                .join("plugin.json"),
            &json!({
                "name": format!("plugin-{index}"), "displayName": format!("Manifest {index}"), "version": "9.0.0"
            }),
        );
    }
    let installed = json!({"plugins": [
        {"name": "plugin-0"}, {"name": "plugin-1"}, {"name": "plugin-2"}, {"name": "plugin-3"}, {"name": "-99999", "enabled": false}
    ]});
    let cli = fake_cli(&paths.home.join("agy"), "agy", &installed);
    let options = overrides(cli);
    let snapshot =
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Antigravity, false, &options)
            .await
            .unwrap();
    for entry in snapshot
        .installed
        .iter()
        .filter(|entry| entry.name.starts_with("plugin-"))
    {
        assert!(entry.display_name.starts_with("Manifest "));
        assert_eq!(entry.version.as_deref(), Some("9.0.0"));
    }
    execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Toggle(true),
        "-99999",
        &options,
    )
    .await
    .unwrap();
    let error = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Toggle(true),
        "-88888",
        &options,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_NOT_INSTALLED");
    assert!(execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Antigravity,
        PluginAction::Install,
        "-99999",
        &options
    )
    .await
    .is_err());
}

#[tokio::test]
async fn tc_p3_03_missing_partial_or_corrupt_cache_does_not_break_installed_plugins() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(&paths.home.join("agy"), "agy", &json!([]));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let options = PluginTestOverrides {
        catalog_urls: Some([format!("{url}/official"), format!("{url}/partners")]),
        ..overrides(cli)
    };
    let cache = paths.switch_dir().join("extensions/plugin_cache.json");
    for legacy in [
        "{}",
        "{\"catalogs\":{\"antigravity-plugins-official\":{}}}",
        "not-json",
    ] {
        write(&cache, legacy);
        let snapshot = get_plugins_hub_snapshot_with_overrides(
            &paths,
            ExtPlatform::Antigravity,
            true,
            &options,
        )
        .await
        .unwrap();
        assert!(snapshot.cli_available);
        assert!(snapshot.available.is_empty());
        assert!(!snapshot.warnings.is_empty());
    }
}

#[tokio::test]
async fn tc_p3_04_readonly_duplicate_scope_cannot_modify_user_or_project_plugin() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(
        &paths.home.join("claude"),
        "claude",
        &json!([
            {"id": "alpha@official", "scope": "user"}, {"id": "alpha@official", "scope": "project"}
        ]),
    );
    let options = overrides(cli.clone());
    let error = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::Toggle(false),
        "alpha@official",
        &options,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_SCOPE_READ_ONLY");
    assert!(!calls(&cli).contains("plugin disable"));
}

#[tokio::test]
async fn tc_p3_04_child_stdin_is_closed_and_json_failures_are_not_success() {
    let (_temporary, paths) = sandbox();
    let cli = fake_cli(&paths.home.join("stdin"), "claude", &json!([]));
    #[cfg(windows)]
    write(
        &cli,
        "@echo off\r\nset /p answer=\r\nif defined answer exit /b 9\r\necho []\r\n",
    );
    #[cfg(not(windows))]
    write(
        &cli,
        "#!/bin/sh\nif read -r answer; then exit 9; fi\nprintf '[]'\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let options = PluginTestOverrides {
        short_timeout: Some(Duration::from_secs(2)),
        ..overrides(cli.clone())
    };
    assert!(
        get_plugins_hub_snapshot_with_overrides(&paths, ExtPlatform::Claude, false, &options)
            .await
            .unwrap()
            .installed
            .is_empty()
    );
    #[cfg(windows)]
    write(
        &cli,
        "@echo off\r\necho {\"success\":false,\"error\":\"EBUSY locked\"}\r\n",
    );
    #[cfg(not(windows))]
    write(
        &cli,
        "#!/bin/sh\nprintf '%s' '{\"success\":false,\"error\":\"EBUSY locked\"}'\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let error = execute_plugin_action_with_overrides(
        &paths,
        ExtPlatform::Claude,
        PluginAction::Install,
        "alpha@official",
        &options,
    )
    .await
    .unwrap_err();
    assert_eq!(error.error_code, "PLUGIN_IN_USE");
}
