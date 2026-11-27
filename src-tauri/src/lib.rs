pub mod adapters;
pub mod agy_proxy;
pub mod auth;
pub mod client_monitor;
pub mod config;
pub mod extensions;
pub mod installer;
pub mod manifest;
pub mod mcp_server;
pub mod telemetry;

use adapters::{
    get_adapter, snapshot_app, AppId, AppMode, AppSnapshot, DesktopAppAdapter, DiagnosticResult,
};
use auth::{
    add_saved_key as auth_add_saved_key, build_local_account_status, fetch_account_status,
    format_usd, list_saved_key_summaries, login_with_key as auth_login_with_key,
    logout as auth_logout, open_external_url, read_balance_cache,
    remove_saved_key as auth_remove_saved_key, rename_saved_key_alias as auth_rename_saved_key_alias,
    start_oauth_login, switch_saved_key as auth_switch_saved_key, AccountStatus,
};
use config::{
    read_switch_state, save_switch_state, AppPaths, McpToolsConfig, SwitchState,
};
use installer::InstallOutcome;
use manifest::{
    compute_updates, fetch_manifest, load_cached_manifest, resolve_platform_artifact, Manifest,
    ManifestCheckResult,
};
use serde::{Deserialize, Serialize};
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_updater::UpdaterExt;
use tokio::sync::Mutex;

const TRAY_ID: &str = "main-tray";
const SELF_UPDATE_OSS_ENDPOINT: &str =
    "https://dl.lven.asia/tako-switch/latest.json";
const SELF_UPDATE_GITHUB_ENDPOINT: &str =
    "https://github.com/tako-dev/tako-switch/releases/latest/download/latest.json";
const SELF_UPDATE_BG_INTERVAL_SECS: u64 = 1800;
const UI_PATCH_IDLE_SYNC_INTERVAL_SECS: u64 = 20;

static UI_PATCH_OP_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

fn ui_patch_op_lock() -> &'static std::sync::Mutex<()> {
    UI_PATCH_OP_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoUiSyncReport {
    pub updated: Vec<AppId>,
    pub deferred_running: Vec<AppId>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchAutoSyncEvent {
    pub updated_apps: Vec<AppId>,
    pub deferred_running_apps: Vec<AppId>,
}

pub struct InstallGuard {
    lock: Mutex<()>,
    cancels: std::sync::Mutex<
        std::collections::HashMap<AppId, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
}

pub struct CachedAccount(std::sync::Mutex<Option<AccountStatus>>);

pub struct PendingSelfUpdate(Mutex<Option<tauri_plugin_updater::Update>>);

pub struct TrayUpdateNotice(std::sync::Mutex<Option<String>>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfUpdateCheckResult {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchProgressEvent {
    pub scope: String,
    pub action: String,
    pub stage: String,
    pub percent: u8,
    pub message: String,
    pub current_app: Option<AppId>,
}

fn ui_patch_app_label(app_id: AppId) -> &'static str {
    match app_id {
        AppId::Claude => "Claude Desktop",
        AppId::Codex => "Codex",
        AppId::Antigravity => "Antigravity",
        AppId::DshTako => "DSH Tako",
        AppId::WorkBuddy => "WorkBuddy",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfUpdateProgressEvent {
    stage: String,
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPayload {
    pub account: AccountStatus,
    pub state: SwitchState,
    pub apps: Vec<AppSnapshot>,
    pub cached_manifest: Option<Manifest>,
    pub local_gateway_healthy: bool,
    pub local_gateway_url: String,
    pub app_version: String,
    pub os: String,
    #[serde(default)]
    pub ui_patch_prompts: Vec<adapters::UiPatchPermissionPrompt>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub base_url: Option<String>,
    pub agy_api_key: Option<String>,
    pub manifest_url: Option<String>,
    pub fallback_manifest_url: Option<String>,
    pub auto_check_updates: Option<bool>,
    pub codex_preserve_official_auth: Option<bool>,
    pub gateway_display_name: Option<String>,
    pub gateway_display_subtitle: Option<String>,
    pub prefer_chinese_ui: Option<bool>,
    pub show_tray_balance: Option<bool>,
    pub bypass_system_proxy: Option<bool>,
    pub telemetry_enabled: Option<bool>,
    pub ui_density: Option<config::UiDensity>,
    pub theme_pref: Option<config::ThemePref>,
    pub mcp: Option<McpToolsConfig>,
    pub hidden_apps: Option<Vec<AppId>>,
    pub workbuddy_default_model: Option<String>,
    pub claude_workspace_mode: Option<config::ClaudeWorkspaceMode>,
}

fn current_cached_account(app: &AppHandle, paths: &AppPaths) -> Option<AccountStatus> {
    if let Some(cache) = app.try_state::<CachedAccount>() {
        if let Ok(guard) = cache.0.lock() {
            if let Some(ref st) = *guard {
                return Some(st.clone());
            }
        }
    }
    read_balance_cache(paths).map(|c| c.status)
}

fn current_tray_update_version(app: &AppHandle) -> Option<String> {
    app.try_state::<TrayUpdateNotice>()
        .and_then(|st| st.0.lock().ok().and_then(|g| g.clone()))
        .filter(|v| v != env!("CARGO_PKG_VERSION"))
}

fn set_tray_update_version(app: &AppHandle, version: Option<String>) {
    if let Some(st) = app.try_state::<TrayUpdateNotice>() {
        if let Ok(mut guard) = st.0.lock() {
            *guard = version;
        }
    }
}

fn parse_semver_tuple(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?.trim();
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn choose_self_update_endpoints(
    oss_ver: Option<(u64, u64, u64)>,
    gh_ver: Option<(u64, u64, u64)>,
) -> (&'static str, Vec<&'static str>) {
    match (oss_ver, gh_ver) {
        (Some(o), Some(g)) if g > o => (
            "github",
            vec![SELF_UPDATE_GITHUB_ENDPOINT, SELF_UPDATE_OSS_ENDPOINT],
        ),
        (None, Some(_)) => (
            "github",
            vec![SELF_UPDATE_GITHUB_ENDPOINT, SELF_UPDATE_OSS_ENDPOINT],
        ),
        _ => (
            "oss",
            vec![SELF_UPDATE_OSS_ENDPOINT, SELF_UPDATE_GITHUB_ENDPOINT],
        ),
    }
}

async fn probe_release_version(client: &reqwest::Client, url: &str) -> Option<(u64, u64, u64)> {
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        client.get(url).send(),
    )
    .await
    .ok()?
    .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.json::<serde_json::Value>().await.ok()?;
    body.get("version")
        .and_then(|v| v.as_str())
        .and_then(parse_semver_tuple)
}

async fn perform_self_update_check(app: &AppHandle) -> Result<SelfUpdateCheckResult, String> {
    let probe_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .user_agent(concat!("tako-switch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| format!("创建更新探测客户端失败: {e}"))?;

    let (oss_ver, gh_ver) = tokio::join!(
        probe_release_version(&probe_client, SELF_UPDATE_OSS_ENDPOINT),
        probe_release_version(&probe_client, SELF_UPDATE_GITHUB_ENDPOINT),
    );
    let (source, ordered_urls) = choose_self_update_endpoints(oss_ver, gh_ver);
    let endpoints: Vec<tauri::Url> = ordered_urls
        .iter()
        .filter_map(|u| tauri::Url::parse(u).ok())
        .collect();

    let updater = app
        .updater_builder()
        .endpoints(endpoints)
        .map_err(|e| format!("配置软件更新源失败: {e}"))?
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("初始化软件更新器失败: {e}"))?;

    let update_opt = updater
        .check()
        .await
        .map_err(|e| format!("检查 Tako Switch 更新失败: {e}"))?;

    if let Some(pending) = app.try_state::<PendingSelfUpdate>() {
        let mut guard = pending.0.lock().await;
        *guard = update_opt.clone();
    }

    let result = match update_opt {
        Some(ref upd) => {
            set_tray_update_version(app, Some(upd.version.clone()));
            SelfUpdateCheckResult {
                available: true,
                version: Some(upd.version.clone()),
                notes: upd.body.clone(),
                source: source.to_string(),
            }
        }
        None => {
            set_tray_update_version(app, None);
            SelfUpdateCheckResult {
                available: false,
                version: None,
                notes: None,
                source: source.to_string(),
            }
        }
    };

    if let Ok(paths) = AppPaths::detect() {
        refresh_tray(app, &paths);
        telemetry::track_event(
            &paths,
            "self_update_check",
            "tako-switch",
            true,
            0,
            serde_json::json!({
                "available": result.available,
                "version": result.version,
                "source": result.source,
            }),
        );
    }

    Ok(result)
}

fn pct_round(used: f64, limit: f64) -> u64 {
    if limit <= 0.0 || used <= 0.0 {
        return 0;
    }
    ((used / limit) * 100.0).round().clamp(0.0, 100.0) as u64
}

fn build_tray_menu(app: &AppHandle, paths: &AppPaths) -> Result<Menu<tauri::Wry>, String> {
    let cached_acc = current_cached_account(app, paths);
    let local_acc = build_local_account_status(paths, cached_acc);
    let has_key = local_acc.logged_in;
    let acc_opt = if has_key { Some(&local_acc) } else { None };

    let active_key_item = acc_opt
        .and_then(|acc| acc.display_label.as_deref())
        .and_then(|label| {
            let text = match acc_opt.and_then(|a| a.active_key_alias.as_deref()) {
                Some(alias) if !alias.trim().is_empty() => format!("当前密钥: {label} [{alias}]"),
                _ => format!("当前密钥: {label}"),
            };
            MenuItem::with_id(app, "tray:active-key", text, false, None::<&str>).ok()
        });

    let summary_text = if let Some(acc) = acc_opt {
        if let Some(rem) = acc.wallet_remain_usd {
            if let Some(used) = acc.wallet_used_usd {
                format!("Tako 余额: {}  (已用 {})", format_usd(rem), format_usd(used))
            } else {
                format!("Tako 余额: {}", format_usd(rem))
            }
        } else {
            "Tako: 已登录 (刷新获取余额)".to_string()
        }
    } else {
        "Tako: 未登录 (点击打开登录)".to_string()
    };

    let acct_summary = MenuItem::with_id(app, "tray:account-summary", summary_text, false, None::<&str>)
        .map_err(|e| format!("创建账户摘要项失败: {e}"))?;

    let plan_summary = acc_opt.and_then(|acc| {
        acc.quota
            .as_ref()
            .filter(|q| q.window_cost_limit > 0.0 || q.weekly_cost_limit > 0.0)
            .and_then(|q| {
                let p5h = pct_round(q.window_used, q.window_cost_limit);
                let p7d = pct_round(q.weekly_used, q.weekly_cost_limit);
                let plan_label = acc
                    .plan_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("套餐");
                MenuItem::with_id(
                    app,
                    "tray:plan-summary",
                    format!("{plan_label} · 5h {p5h}% / 7d {p7d}%"),
                    false,
                    None::<&str>,
                )
                .ok()
            })
    });

    let saved_keys = list_saved_key_summaries(paths);
    let key_switch_submenu = if saved_keys.len() >= 2 || (!has_key && !saved_keys.is_empty()) {
        let mut key_items = Vec::with_capacity(saved_keys.len());
        for item in &saved_keys {
            let mark = if item.active { "● " } else { "○ " };
            let label = match item.alias.as_deref() {
                Some(alias) if !alias.trim().is_empty() => {
                    format!("{mark}{} [{alias}]", item.display_label)
                }
                _ => format!("{mark}{}", item.display_label),
            };
            let menu_item = MenuItem::with_id(
                app,
                format!("key:{}", item.id),
                label,
                true,
                None::<&str>,
            )
            .map_err(|e| format!("创建密钥切换菜单项失败: {e}"))?;
            key_items.push(menu_item);
        }
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = key_items
            .iter()
            .map(|m| m as &dyn IsMenuItem<tauri::Wry>)
            .collect();
        Some(
            Submenu::with_items(
                app,
                format!("切换密钥 ({})", saved_keys.len()),
                true,
                &refs,
            )
            .map_err(|e| format!("创建密钥切换子菜单失败: {e}"))?,
        )
    } else {
        None
    };

    let sep0 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let show = MenuItem::with_id(app, "tray:show", "打开 Tako Switch", true, None::<&str>)
        .map_err(|e| format!("创建托盘菜单失败: {e}"))?;
    let refresh_bal = MenuItem::with_id(app, "tray:refresh-balance", "刷新余额", true, None::<&str>)
        .map_err(|e| format!("创建刷新余额菜单失败: {e}"))?;

    let switch_state = read_switch_state(paths);
    let visible_tray_apps: Vec<AppId> = {
        let filtered: Vec<AppId> = AppId::all()
            .iter()
            .copied()
            .filter(|id| !switch_state.is_app_hidden(*id))
            .collect();
        if filtered.is_empty() {
            AppId::all().to_vec()
        } else {
            filtered
        }
    };
    let mut submenus = Vec::new();
    for app_id in visible_tray_apps {
        let mode = get_adapter(app_id).read_mode(paths);
        let mark = |m: AppMode| if mode == m { "● " } else { "○ " };
        let (off_label, tako_label) = if app_id == AppId::DshTako {
            ("独立登录", "同步密钥 (免二次登录)")
        } else {
            ("官方直连", "Tako 全接管 (免官方登录)")
        };
        let off = MenuItem::with_id(
            app,
            format!("mode:{}:official", app_id.as_str()),
            format!("{}{off_label}", mark(AppMode::Official)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;
        let tako = MenuItem::with_id(
            app,
            format!("mode:{}:tako", app_id.as_str()),
            format!("{}{tako_label}", mark(AppMode::Tako)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;

        let sub = Submenu::with_items(
            app,
            format!(
                "{} ({})",
                app_id.display_name(),
                mode_label_for(app_id, mode)
            ),
            true,
            &[&off, &tako],
        )
        .map_err(|e| format!("创建应用子菜单失败: {e}"))?;
        submenus.push(sub);
    }

    let pending_update_ver = current_tray_update_version(app);
    let update_banner_item = pending_update_ver.as_deref().and_then(|ver| {
        MenuItem::with_id(
            app,
            "tray:updates",
            format!("⬆ 发现新版 Tako Switch v{ver} · 点击更新"),
            true,
            None::<&str>,
        )
        .ok()
    });
    let check_upd = MenuItem::with_id(app, "tray:updates-check", "检查更新", true, None::<&str>)
        .map_err(|e| format!("创建菜单项失败: {e}"))?;
    let quit = MenuItem::with_id(app, "tray:quit", "退出 Tako Switch", true, None::<&str>)
        .map_err(|e| format!("创建退出菜单失败: {e}"))?;

    let sep1 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let sep2 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;

    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = Vec::new();
    if let Some(ref key_item) = active_key_item {
        items.push(key_item);
    }
    items.push(&acct_summary);
    if let Some(ref p_item) = plan_summary {
        items.push(p_item);
    }
    items.push(&sep0);
    if let Some(ref upd_item) = update_banner_item {
        items.push(upd_item);
    }
    items.push(&show);
    items.push(&refresh_bal);
    if let Some(ref ks_sub) = key_switch_submenu {
        items.push(ks_sub);
    }
    items.push(&sep1);
    for sub in &submenus {
        items.push(sub);
    }
    items.push(&sep2);
    items.push(&check_upd);
    items.push(&quit);

    Menu::with_items(app, &items).map_err(|e| format!("组装托盘菜单失败: {e}"))
}

fn mode_label_for(app_id: AppId, mode: AppMode) -> &'static str {
    if app_id == AppId::DshTako {
        return match mode {
            AppMode::Official => "独立",
            AppMode::Tako => "同步",
        };
    }
    match mode {
        AppMode::Official => "官方",
        AppMode::Tako => "Tako",
    }
}

fn refresh_tray(app: &AppHandle, paths: &AppPaths) {
    update_tray(app, paths, true);
}

fn update_tray(app: &AppHandle, paths: &AppPaths, rebuild_menu: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if rebuild_menu {
            if let Ok(menu) = build_tray_menu(app, paths) {
                let _ = tray.set_menu(Some(menu));
            }
        }
        let st = read_switch_state(paths);
        let has_key = config::read_main_api_key(paths).is_some();
        let rem_opt = if has_key {
            current_cached_account(app, paths)
                .filter(|a| a.logged_in)
                .and_then(|a| a.wallet_remain_usd)
        } else {
            None
        };
        let update_ver = current_tray_update_version(app);

        #[cfg(target_os = "macos")]
        {
            if st.show_tray_balance {
                if let Some(rem) = rem_opt {
                    let _ = tray.set_title(Some(format!(" {}", format_usd(rem))));
                } else {
                    let _ = tray.set_title(Some(""));
                }
            } else {
                let _ = tray.set_title(Some(""));
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = st.show_tray_balance;

        let tip = match (update_ver.as_deref(), rem_opt) {
            (Some(v), Some(rem)) => format!("Tako Switch (可更新 v{v}) · 余额 {}", format_usd(rem)),
            (Some(v), None) => format!("Tako Switch · 发现新版 v{v}"),
            (None, Some(rem)) => format!("Tako Switch · 余额 {}", format_usd(rem)),
            (None, None) => "Tako Switch".to_string(),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn update_account_and_tray(
    app: &AppHandle,
    paths: &AppPaths,
    status: AccountStatus,
    emit_event: bool,
) {
    let prev_formatted = if let Some(cache) = app.try_state::<CachedAccount>() {
        if let Ok(mut guard) = cache.0.lock() {
            let prev = guard
                .as_ref()
                .and_then(|s| s.wallet_remain_usd)
                .map(format_usd);
            *guard = Some(status.clone());
            prev
        } else {
            None
        }
    } else {
        None
    };

    let next_formatted = status.wallet_remain_usd.map(format_usd);
    if prev_formatted != next_formatted
        && status.logged_in
        && next_formatted.is_some()
        && adapters::claude::ClaudeAdapter.read_mode(paths) == AppMode::Tako
    {
        let _ = adapters::claude::ClaudeAdapter.apply_mode(paths, AppMode::Tako);
    }

    refresh_tray(app, paths);
    if emit_event {
        let _ = app.emit("account-updated", &status);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn parse_tray_mode_id(id: &str) -> Option<(AppId, AppMode)> {
    let mut parts = id.split(':');
    if parts.next()? != "mode" {
        return None;
    }
    let app_id = match parts.next()? {
        "dsh-tako" => AppId::DshTako,
        "codex" => AppId::Codex,
        "claude" => AppId::Claude,
        "antigravity" => AppId::Antigravity,
        "workbuddy" => AppId::WorkBuddy,
        _ => return None,
    };
    let mode = match parts.next()? {
        "official" => AppMode::Official,
        "tako" => AppMode::Tako,
        _ => return None,
    };
    Some((app_id, mode))
}

#[tauri::command]
async fn get_dashboard(app: AppHandle, refresh_account: bool) -> Result<DashboardPayload, String> {
    let paths = AppPaths::detect()?;
    let paths_for_local = paths.clone();
    let local_task = tauri::async_runtime::spawn_blocking(move || {
        (
            read_switch_state(&paths_for_local),
            AppId::all()
                .iter()
                .map(|&id| snapshot_app(&paths_for_local, id))
                .collect(),
            load_cached_manifest(&paths_for_local),
            agy_proxy::is_proxy_healthy(agy_proxy::LOCAL_GATEWAY_PORT),
        )
    });

    let account_task = async {
        if refresh_account {
            let status = fetch_account_status(&paths).await;
            update_account_and_tray(&app, &paths, status.clone(), false);
            status
        } else {
            build_local_account_status(&paths, current_cached_account(&app, &paths))
        }
    };

    let (account, local_res) = tokio::join!(account_task, local_task);
    let (state, apps, cached_manifest, local_gateway_healthy) =
        local_res.map_err(|err| format!("读取客户端状态失败: {err}"))?;

    Ok(DashboardPayload {
        account,
        state,
        apps,
        cached_manifest,
        local_gateway_healthy,
        local_gateway_url: agy_proxy::LOCAL_GATEWAY_URL.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        ui_patch_prompts: Vec::new(),
    })
}

fn sync_after_account_login(paths: &AppPaths) {
    for &app_id in AppId::all() {
        let adapter = get_adapter(app_id);
        let should_sync = adapter.read_mode(paths) == AppMode::Tako
            || (app_id == AppId::DshTako
                && adapter.detect_install(paths).installed
                && !adapters::dsh_tako::has_any_dsh_tako_key(paths));
        if should_sync {
            let _ = adapter.apply_mode(paths, AppMode::Tako);
        }
    }
}

async fn sync_after_key_change(paths: &AppPaths) {
    if config::read_main_api_key(paths).is_some() {
        let claude_tako = get_adapter(AppId::Claude).read_mode(paths) == AppMode::Tako;
        let wb_tako = get_adapter(AppId::WorkBuddy).read_mode(paths) == AppMode::Tako;
        if claude_tako && wb_tako {
            let _ = tokio::join!(
                agy_proxy::refresh_claude_catalog_cache(paths),
                agy_proxy::refresh_workbuddy_catalog_cache(paths),
            );
        } else if claude_tako {
            let _ = agy_proxy::refresh_claude_catalog_cache(paths).await;
        } else if wb_tako {
            let _ = agy_proxy::refresh_workbuddy_catalog_cache(paths).await;
        }
        sync_after_account_login(paths);
    }
}

#[tauri::command]
async fn login_oauth(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = start_oauth_login(&paths).await?;
    sync_after_key_change(&paths).await;
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
async fn login_with_key(app: AppHandle, key: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_login_with_key(&paths, &key).await?;
    sync_after_key_change(&paths).await;
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
async fn add_saved_key(
    app: AppHandle,
    key: String,
    alias: Option<String>,
    activate: Option<bool>,
) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let should_activate = activate.unwrap_or(true);
    let status = auth_add_saved_key(&paths, &key, alias.as_deref(), should_activate).await?;
    if should_activate || status.logged_in {
        sync_after_key_change(&paths).await;
    }
    telemetry::track_event(
        &paths,
        "key_added",
        "tako-switch",
        true,
        0,
        serde_json::json!({
            "saved_keys_count": status.saved_keys.len(),
            "activated": should_activate,
        }),
    );
    update_account_and_tray(&app, &paths, status.clone(), true);
    Ok(status)
}

#[tauri::command]
async fn switch_saved_key(app: AppHandle, key_id: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_switch_saved_key(&paths, &key_id).await?;
    sync_after_key_change(&paths).await;
    telemetry::track_event(
        &paths,
        "key_switch",
        "tako-switch",
        true,
        0,
        serde_json::json!({
            "saved_keys_count": status.saved_keys.len(),
            "via": "ui",
        }),
    );
    update_account_and_tray(&app, &paths, status.clone(), true);
    Ok(status)
}

#[tauri::command]
async fn remove_saved_key(app: AppHandle, key_id: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let prev_active = config::read_switch_auth(&paths).active_key_id;
    let status = auth_remove_saved_key(&paths, &key_id).await?;
    if prev_active.as_deref() == Some(key_id.trim()) && status.logged_in {
        sync_after_key_change(&paths).await;
    }
    telemetry::track_event(
        &paths,
        "key_removed",
        "tako-switch",
        true,
        0,
        serde_json::json!({
            "saved_keys_count": status.saved_keys.len(),
        }),
    );
    update_account_and_tray(&app, &paths, status.clone(), true);
    Ok(status)
}

#[tauri::command]
async fn rename_saved_key_alias(
    app: AppHandle,
    key_id: String,
    alias: Option<String>,
) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_rename_saved_key_alias(&paths, &key_id, alias.as_deref()).await?;
    update_account_and_tray(&app, &paths, status.clone(), true);
    Ok(status)
}

#[tauri::command]
async fn logout_account(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_logout(&paths).await?;
    update_account_and_tray(&app, &paths, status.clone(), true);
    Ok(status)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    open_external_url(&url)
}

pub fn apply_settings_patch_for_paths(
    paths: &AppPaths,
    input: SettingsPatch,
) -> Result<SwitchState, String> {
    let mut state = read_switch_state(paths);
    let prev_base = state.normalized_base_url();
    let prev_agy_key = state.agy_api_key.clone();
    let prev_codex_preserve = state.codex_preserve_official_auth;
    let prev_workbuddy_default_model = state.workbuddy_default_model.clone();
    let prev_claude_workspace_mode = state.claude_workspace_mode;
    let prev_name = state.normalized_gateway_name();
    let prev_subtitle = state.normalized_gateway_subtitle();
    let prev_zh = state.prefer_chinese_ui;
    let prev_mcp_flags = (
        state.mcp.image,
        state.mcp.video,
        state.mcp.audio,
        state.mcp.search,
        state.mcp.consult,
        state.mcp.web_page,
    );
    let prev_edgeone = (
        state.mcp.web_page,
        state.mcp.normalized_edgeone_api_token().map(str::to_string),
        state
            .mcp
            .normalized_edgeone_project_name()
            .map(str::to_string),
    );

    if let Some(url) = input.base_url {
        let trimmed = url.trim().trim_end_matches('/').to_string();
        if !trimmed.is_empty() {
            state.base_url = trimmed;
        }
    }
    if let Some(agy_key_raw) = input.agy_api_key {
        let trimmed = agy_key_raw.trim().to_string();
        state.agy_api_key = (!trimmed.is_empty()).then_some(trimmed);
    }
    if let Some(murl) = input.manifest_url {
        if !murl.trim().is_empty() {
            state.manifest_url = murl.trim().to_string();
        }
    }
    if let Some(furl) = input.fallback_manifest_url {
        if !furl.trim().is_empty() {
            state.fallback_manifest_url = furl.trim().to_string();
        }
    }
    if let Some(auto_chk) = input.auto_check_updates {
        state.auto_check_updates = auto_chk;
    }
    if let Some(preserve) = input.codex_preserve_official_auth {
        state.codex_preserve_official_auth = preserve;
    }
    if let Some(name) = input.gateway_display_name {
        let trimmed = name.trim();
        state.gateway_display_name = if trimmed.is_empty() {
            crate::config::DEFAULT_GATEWAY_DISPLAY_NAME.to_string()
        } else {
            trimmed.chars().take(60).collect()
        };
    }
    if let Some(subtitle) = input.gateway_display_subtitle {
        state.gateway_display_subtitle = subtitle.trim().chars().take(60).collect();
    }
    if let Some(prefer_zh) = input.prefer_chinese_ui {
        state.prefer_chinese_ui = prefer_zh;
    }
    if let Some(show_tray) = input.show_tray_balance {
        state.show_tray_balance = show_tray;
    }
    if let Some(bypass_proxy) = input.bypass_system_proxy {
        state.bypass_system_proxy = bypass_proxy;
    }
    if let Some(telemetry_on) = input.telemetry_enabled {
        state.telemetry_enabled = telemetry_on;
    }
    if let Some(density) = input.ui_density {
        state.ui_density = density;
    }
    if let Some(theme) = input.theme_pref {
        state.theme_pref = theme;
    }
    if let Some(mut mcp) = input.mcp {
        mcp.normalize();
        state.mcp = mcp;
    }
    if let Some(raw_hidden) = input.hidden_apps {
        let mut deduped = Vec::with_capacity(raw_hidden.len());
        for id in raw_hidden {
            if !deduped.contains(&id) {
                deduped.push(id);
            }
        }
        if AppId::all().iter().all(|id| deduped.contains(id)) {
            return Err("至少需要保留一个可见的桌面工具".to_string());
        }
        state.hidden_apps = deduped;
    }
    if let Some(raw_wb_default) = input.workbuddy_default_model {
        let trimmed = raw_wb_default.trim();
        let clean = trimmed
            .strip_prefix(adapters::workbuddy::WORKBUDDY_CUSTOM_LOCAL_PREFIX)
            .unwrap_or(trimmed)
            .trim();
        state.workbuddy_default_model = (!clean.is_empty()).then(|| clean.to_string());
    }
    if let Some(claude_ws) = input.claude_workspace_mode {
        state.claude_workspace_mode = claude_ws;
    }

    save_switch_state(paths, &state)?;

    let next_mcp_flags = (
        state.mcp.image,
        state.mcp.video,
        state.mcp.audio,
        state.mcp.search,
        state.mcp.consult,
        state.mcp.web_page,
    );
    if prev_mcp_flags != next_mcp_flags {
        telemetry::track_event(
            paths,
            "mcp_toggle",
            "tako-switch",
            true,
            0,
            serde_json::json!({
                "enabled_mcps": telemetry::enabled_mcp_names(&state),
                "web_page": state.mcp.web_page,
            }),
        );
    }

    let next_edgeone = (
        state.mcp.web_page,
        state.mcp.normalized_edgeone_api_token().map(str::to_string),
        state
            .mcp
            .normalized_edgeone_project_name()
            .map(str::to_string),
    );
    let edgeone_mcp_changed = prev_edgeone != next_edgeone;
    let endpoint_or_profile_changed = prev_base != state.normalized_base_url()
        || prev_agy_key != state.agy_api_key
        || prev_name != state.normalized_gateway_name()
        || prev_subtitle != state.normalized_gateway_subtitle()
        || prev_zh != state.prefer_chinese_ui
        || edgeone_mcp_changed;
    let codex_auth_mode_changed = prev_codex_preserve != state.codex_preserve_official_auth;
    let workbuddy_default_changed = prev_workbuddy_default_model != state.workbuddy_default_model;
    let claude_workspace_changed = prev_claude_workspace_mode != state.claude_workspace_mode;
    if endpoint_or_profile_changed || codex_auth_mode_changed || claude_workspace_changed {
        for &app_id in AppId::all() {
            if !app_id.requires_local_gateway() {
                continue;
            }
            let targeted_change = (codex_auth_mode_changed && app_id == AppId::Codex)
                || (claude_workspace_changed && app_id == AppId::Claude);
            if !endpoint_or_profile_changed && !targeted_change {
                continue;
            }
            let adapter = get_adapter(app_id);
            if adapter.read_mode(paths) == AppMode::Tako {
                let _ = adapter.apply_mode(paths, AppMode::Tako);
            }
        }
    }
    if workbuddy_default_changed
        && get_adapter(AppId::WorkBuddy).read_mode(paths) == AppMode::Tako
    {
        let _ = adapters::workbuddy::WorkBuddyAdapter::apply_default_model_preference(paths);
    }
    if prev_zh != state.prefer_chinese_ui {
        sync_standalone_chinese_for_installed(paths, state.prefer_chinese_ui);
    }

    Ok(state)
}

#[tauri::command]
async fn save_settings(app: AppHandle, input: SettingsPatch) -> Result<SwitchState, String> {
    let paths = AppPaths::detect()?;
    let theme_changed = input.theme_pref.is_some();
    let state = apply_settings_patch_for_paths(&paths, input)?;
    if theme_changed {
        if let Some(win) = app.get_webview_window("main") {
            apply_native_window_appearance(&win, state.theme_pref, None);
        }
    }
    refresh_tray(&app, &paths);
    Ok(state)
}

#[cfg(target_os = "windows")]
fn apply_windows_dwm_frame_attributes(win: &tauri::WebviewWindow, is_dark: bool) {
    let Ok(hwnd_raw) = win.hwnd() else { return };
    let hwnd = hwnd_raw.0;
    if hwnd.is_null() {
        return;
    }
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: u32 = 2;
    const DWMWA_BORDER_COLOR: u32 = 34;

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut std::ffi::c_void,
            attribute: u32,
            pv_attribute: *const std::ffi::c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    let corner_pref: u32 = DWMWCP_ROUND;
    // COLORREF uses 0x00BBGGRR: light #e1e5ec -> 0x00ECE5E1, dark #242b3d -> 0x003D2B24
    let border_color: u32 = if is_dark { 0x003D_2B24 } else { 0x00EC_E5E1 };

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&corner_pref as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            (&border_color as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

fn apply_native_window_appearance(
    win: &tauri::WebviewWindow,
    theme_pref: config::ThemePref,
    resolved_dark: Option<bool>,
) {
    let native_theme = match theme_pref {
        config::ThemePref::Light => Some(tauri::Theme::Light),
        config::ThemePref::Dark => Some(tauri::Theme::Dark),
        config::ThemePref::System => None,
    };
    let _ = win.set_theme(native_theme);

    let is_dark = match theme_pref {
        config::ThemePref::Dark => true,
        config::ThemePref::Light => false,
        config::ThemePref::System => {
            resolved_dark.unwrap_or_else(|| matches!(win.theme(), Ok(tauri::Theme::Dark)))
        }
    };
    let bg_color = if is_dark {
        tauri::window::Color(15, 18, 25, 255)
    } else {
        tauri::window::Color(247, 248, 251, 255)
    };
    let _ = win.set_background_color(Some(bg_color));

    #[cfg(target_os = "windows")]
    apply_windows_dwm_frame_attributes(win, is_dark);
}

#[tauri::command]
fn sync_window_theme(app: AppHandle, theme_pref: config::ThemePref, is_dark: bool) {
    if let Some(win) = app.get_webview_window("main") {
        apply_native_window_appearance(&win, theme_pref, Some(is_dark));
    }
}

#[tauri::command]
fn show_window_system_menu(window: tauri::WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        let Ok(hwnd_raw) = window.hwnd() else { return };
        let hwnd = hwnd_raw.0;
        if hwnd.is_null() {
            return;
        }
        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }
        const TPM_LEFTALIGN: u32 = 0x0000;
        const TPM_TOPALIGN: u32 = 0x0000;
        const TPM_RIGHTBUTTON: u32 = 0x0002;
        const TPM_RETURNCMD: u32 = 0x0100;
        const WM_SYSCOMMAND: u32 = 0x0112;
        const MF_BYCOMMAND: u32 = 0x0000;
        const MF_ENABLED: u32 = 0x0000;
        const MF_GRAYED: u32 = 0x0001;
        const SC_RESTORE: u32 = 0xF120;
        const SC_MOVE: u32 = 0xF010;
        const SC_SIZE: u32 = 0xF000;
        const SC_MINIMIZE: u32 = 0xF020;
        const SC_MAXIMIZE: u32 = 0xF030;
        const SC_CLOSE: u32 = 0xF060;

        #[link(name = "user32")]
        extern "system" {
            fn GetSystemMenu(hwnd: *mut std::ffi::c_void, revert: i32) -> *mut std::ffi::c_void;
            fn EnableMenuItem(hmenu: *mut std::ffi::c_void, id: u32, enable: u32) -> i32;
            fn GetCursorPos(point: *mut Point) -> i32;
            fn SetForegroundWindow(hwnd: *mut std::ffi::c_void) -> i32;
            fn TrackPopupMenu(
                hmenu: *mut std::ffi::c_void,
                flags: u32,
                x: i32,
                y: i32,
                reserved: i32,
                hwnd: *mut std::ffi::c_void,
                rect: *const std::ffi::c_void,
            ) -> i32;
            fn PostMessageW(
                hwnd: *mut std::ffi::c_void,
                msg: u32,
                wparam: usize,
                lparam: isize,
            ) -> i32;
        }

        let maximized = window.is_maximized().unwrap_or(false);
        unsafe {
            let hmenu = GetSystemMenu(hwnd, 0);
            if hmenu.is_null() {
                return;
            }
            let _ = EnableMenuItem(
                hmenu,
                SC_RESTORE,
                MF_BYCOMMAND | if maximized { MF_ENABLED } else { MF_GRAYED },
            );
            let _ = EnableMenuItem(
                hmenu,
                SC_MOVE,
                MF_BYCOMMAND | if maximized { MF_GRAYED } else { MF_ENABLED },
            );
            let _ = EnableMenuItem(
                hmenu,
                SC_SIZE,
                MF_BYCOMMAND | if maximized { MF_GRAYED } else { MF_ENABLED },
            );
            let _ = EnableMenuItem(hmenu, SC_MINIMIZE, MF_BYCOMMAND | MF_ENABLED);
            let _ = EnableMenuItem(
                hmenu,
                SC_MAXIMIZE,
                MF_BYCOMMAND | if maximized { MF_GRAYED } else { MF_ENABLED },
            );
            let _ = EnableMenuItem(hmenu, SC_CLOSE, MF_BYCOMMAND | MF_ENABLED);

            let mut pt = Point { x: 0, y: 0 };
            if GetCursorPos(&mut pt) == 0 {
                return;
            }
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(
                hmenu,
                TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
                pt.x,
                pt.y,
                0,
                hwnd,
                std::ptr::null(),
            );
            if cmd != 0 {
                let _ = PostMessageW(hwnd, WM_SYSCOMMAND, cmd as usize, 0);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
    }
}

#[tauri::command]
async fn check_node_runtime() -> Result<adapters::NodeRuntimeInfo, String> {
    Ok(tokio::task::spawn_blocking(adapters::detect_node_runtime)
        .await
        .unwrap_or_else(|_| adapters::NodeRuntimeInfo::unavailable()))
}

fn should_sync_standalone_locale(paths: &AppPaths, app_id: AppId, enable: bool) -> bool {
    if !enable {
        return true;
    }
    let adapter = get_adapter(app_id);
    if adapter.detect_install(paths).installed || adapter.read_mode(paths) == AppMode::Tako {
        return true;
    }
    match app_id {
        AppId::DshTako | AppId::WorkBuddy => false,
        AppId::Codex => paths.codex_dir.exists(),
        AppId::Claude => {
            paths.claude_1p_dir.exists()
                || paths.claude_3p_dir.exists()
                || paths.home.join(".claude").exists()
                || paths.claude_app_override_dir.is_some()
        }
        AppId::Antigravity => {
            paths.home.join(".gemini").exists() || paths.agy_app_override_dir.is_some()
        }
    }
}

pub fn sync_standalone_chinese_for_installed(paths: &AppPaths, enable: bool) {
    if should_sync_standalone_locale(paths, AppId::Codex, enable) {
        let _ = adapters::codex::CodexAdapter::sync_standalone_chinese_locale(paths, enable);
    }
    if should_sync_standalone_locale(paths, AppId::Claude, enable) {
        let _ = adapters::claude::ClaudeAdapter::sync_standalone_chinese_locale(paths, enable);
    }
    if should_sync_standalone_locale(paths, AppId::Antigravity, enable) {
        let _ =
            adapters::antigravity::AntigravityAdapter::sync_standalone_chinese_locale(paths, enable);
    }
}

fn inspect_ui_patch_for_app(paths: &AppPaths, app_id: AppId) -> adapters::UiPatchInfo {
    match app_id {
        AppId::Claude => adapters::inspect_claude_ui_patch(paths),
        AppId::Antigravity => adapters::inspect_antigravity_ui_patch(paths),
        _ => adapters::UiPatchInfo::not_applicable(),
    }
}

fn run_raw_ui_patch_action(
    paths: &AppPaths,
    app_id: AppId,
    action: &str,
) -> Result<adapters::UiPatchInfo, String> {
    match (app_id, action) {
        (AppId::Claude, "patch") => adapters::patch_claude_ui_for_paths(paths),
        (AppId::Claude, "restore") => adapters::restore_claude_ui_for_paths(paths),
        (AppId::Antigravity, "patch") => adapters::patch_antigravity_ui_for_paths(paths),
        (AppId::Antigravity, "restore") => adapters::restore_antigravity_ui_for_paths(paths),
        (_, "patch" | "restore") => Err(format!("{} 不支持界面资源包补丁操作", app_id.display_name())),
        _ => Err(format!("不支持的界面补丁动作: {action}")),
    }
}

#[cfg(target_os = "windows")]
fn try_windows_elevated_ui_patch(
    paths: &AppPaths,
    app_id: AppId,
    action: &str,
) -> Result<bool, String> {
    if paths.is_test || std::env::var("TAKO_SWITCH_ELEVATED").as_deref() == Ok("1") {
        return Ok(false);
    }
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = std::env::current_exe().map_err(|e| format!("无法定位当前程序路径: {e}"))?;
    let exe_escaped = exe.to_string_lossy().replace('\'', "''");
    let app_str = app_id.as_str();
    let script = format!(
        "$p = Start-Process -FilePath '{exe_escaped}' -ArgumentList 'ui-patch-worker','--app','{app_str}','--action','{action}' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; if ($null -eq $p) {{ exit 1 }}; exit $p.ExitCode"
    );
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("拉起管理员提权进程失败: {e}"))?;
    Ok(out.status.success())
}

fn quit_app_and_wait_for_ui_patch(
    paths: &AppPaths,
    app_id: AppId,
    force_on_windows: bool,
    auto_restart: bool,
) -> Result<bool, String> {
    if paths.is_test {
        return Ok(false);
    }
    let adapter = get_adapter(app_id);
    let was_running = adapter.is_running(paths);
    if !was_running {
        return Ok(false);
    }
    let should_quit = auto_restart || (cfg!(target_os = "windows") && force_on_windows);
    if !should_quit {
        return Ok(false);
    }
    adapter.quit_app(paths)?;
    let max_polls = if cfg!(target_os = "windows") { 20 } else { 3 };
    let poll_ms = if cfg!(target_os = "windows") { 200 } else { 150 };
    for _ in 0..max_polls {
        if !adapter.is_running(paths) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(poll_ms));
    }
    Ok(true)
}

fn execute_ui_patch_action_for_paths(
    paths: &AppPaths,
    app_id: AppId,
    action: &str,
    auto_restart: bool,
) -> Result<adapters::UiPatchInfo, String> {
    let _op_guard = ui_patch_op_lock().lock().unwrap_or_else(|e| e.into_inner());
    let quit_for_patch = quit_app_and_wait_for_ui_patch(paths, app_id, true, auto_restart)?;

    let res = {
        #[cfg(target_os = "windows")]
        {
            match run_raw_ui_patch_action(paths, app_id, action) {
                Ok(info) => Ok(info),
                Err(err) => {
                    if !paths.is_test
                        && adapters::parse_ui_patch_block_error(&err)
                            .is_some_and(|p| p.kind == adapters::UiPatchBlockKind::WindowsUac)
                    {
                        if let Ok(true) = try_windows_elevated_ui_patch(paths, app_id, action) {
                            Ok(inspect_ui_patch_for_app(paths, app_id))
                        } else {
                            Err(err)
                        }
                    } else {
                        Err(err)
                    }
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            run_raw_ui_patch_action(paths, app_id, action)
        }
    };

    if quit_for_patch && !paths.is_test {
        let _ = client_monitor::launch_and_monitor(paths, app_id, "ui_patch_relaunch");
    }
    res
}

/// Silently upgrades outdated / post-upgrade UI localization patches for installed apps
/// when `prefer_chinese_ui` is enabled. Never kills a running client; running apps are
/// recorded in `deferred_running` so they can be synced automatically once closed.
pub fn auto_sync_pending_ui_patches_for_paths(
    paths: &AppPaths,
    include_unpatched_for: Option<AppId>,
) -> AutoUiSyncReport {
    auto_sync_pending_ui_patches_for_paths_with_progress(paths, include_unpatched_for, |_| {})
}

pub fn auto_sync_pending_ui_patches_for_paths_with_progress<F>(
    paths: &AppPaths,
    include_unpatched_for: Option<AppId>,
    mut on_progress: F,
) -> AutoUiSyncReport
where
    F: FnMut(UiPatchProgressEvent),
{
    let mut report = AutoUiSyncReport::default();
    if !read_switch_state(paths).prefer_chinese_ui {
        return report;
    }
    let Ok(_guard) = ui_patch_op_lock().try_lock() else {
        return report;
    };

    let mut pending_targets: Vec<AppId> = Vec::new();
    for &app_id in &[AppId::Claude, AppId::Antigravity] {
        let info = inspect_ui_patch_for_app(paths, app_id);
        let needs_sync = info.status == adapters::UiPatchStatus::OutdatedAfterUpgrade
            || (info.status == adapters::UiPatchStatus::Patched && info.update_available)
            || (include_unpatched_for == Some(app_id)
                && info.status == adapters::UiPatchStatus::Unpatched);
        if !needs_sync {
            continue;
        }
        if !paths.is_test && get_adapter(app_id).is_running(paths) {
            report.deferred_running.push(app_id);
            continue;
        }
        pending_targets.push(app_id);
    }

    let total = pending_targets.len();
    for (idx, app_id) in pending_targets.into_iter().enumerate() {
        let label = ui_patch_app_label(app_id);
        let start_pct = 18u8
            .saturating_add(((idx as u16) * 72 / (total.max(1) as u16)) as u8)
            .min(86);
        on_progress(UiPatchProgressEvent {
            scope: "auto".into(),
            action: "patch".into(),
            stage: "patching".into(),
            percent: start_pct,
            message: format!("检测到汉化版本更新，正在自动为 {label} 汉化中..."),
            current_app: Some(app_id),
        });
        if run_raw_ui_patch_action(paths, app_id, "patch").is_ok() {
            match app_id {
                AppId::Claude => {
                    let _ = adapters::claude::ClaudeAdapter::sync_standalone_chinese_locale(
                        paths, true,
                    );
                }
                AppId::Antigravity => {
                    let _ =
                        adapters::antigravity::AntigravityAdapter::sync_standalone_chinese_locale(
                            paths, true,
                        );
                }
                _ => {}
            }
            report.updated.push(app_id);
        }
    }

    if !report.updated.is_empty() {
        let names = report
            .updated
            .iter()
            .map(|id| ui_patch_app_label(*id))
            .collect::<Vec<_>>()
            .join("、");
        on_progress(UiPatchProgressEvent {
            scope: "auto".into(),
            action: "patch".into(),
            stage: "done".into(),
            percent: 100,
            message: format!("汉化版本已自动更新完成（{names}）"),
            current_app: None,
        });
    }

    report
}

async fn run_background_ui_patch_auto_sync(app: &AppHandle, fetch_remote_packs: bool) {
    let Ok(paths) = AppPaths::detect() else {
        return;
    };
    if !read_switch_state(&paths).prefer_chinese_ui {
        return;
    }
    let fetched_packs = if fetch_remote_packs {
        adapters::refresh_remote_i18n_packs(&paths).await
    } else {
        Vec::new()
    };

    let paths_for_sync = paths.clone();
    let app_for_progress = app.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        auto_sync_pending_ui_patches_for_paths_with_progress(&paths_for_sync, None, |evt| {
            let _ = app_for_progress.emit("ui-patch-progress", &evt);
        })
    })
    .await
    .unwrap_or_default();

    if !report.updated.is_empty() || !fetched_packs.is_empty() {
        refresh_tray(app, &paths);
        let _ = app.emit("state-changed", ());
    }
    if !report.updated.is_empty() || !report.deferred_running.is_empty() {
        let _ = app.emit(
            "ui-patch-auto-synced",
            &UiPatchAutoSyncEvent {
                updated_apps: report.updated,
                deferred_running_apps: report.deferred_running,
            },
        );
    }
}

/// Decoupled Chinese UI toggle & ASAR synchronization for tests and runtime commands.
/// Step 1 & 2 (config-layer Chinese localization) ALWAYS commit first; Step 3/4 ASAR
/// permission blocks are collected as `UiPatchPermissionPrompt` without rolling back config.
pub fn apply_prefer_chinese_ui_for_paths(
    paths: &AppPaths,
    enable: bool,
    patch_ui: bool,
    auto_restart: bool,
) -> Result<Vec<adapters::UiPatchPermissionPrompt>, String> {
    apply_prefer_chinese_ui_for_paths_with_progress(paths, enable, patch_ui, auto_restart, |_| {})
}

pub fn apply_prefer_chinese_ui_for_paths_with_progress<F>(
    paths: &AppPaths,
    enable: bool,
    patch_ui: bool,
    auto_restart: bool,
    mut on_progress: F,
) -> Result<Vec<adapters::UiPatchPermissionPrompt>, String>
where
    F: FnMut(UiPatchProgressEvent),
{
    let action = if enable { "patch" } else { "restore" };
    on_progress(UiPatchProgressEvent {
        scope: "all".into(),
        action: action.into(),
        stage: "config".into(),
        percent: 12,
        message: if enable {
            "正在同步 Codex / Claude / Antigravity 简体中文配置...".into()
        } else {
            "正在恢复 Codex / Claude / Antigravity 原版语言配置...".into()
        },
        current_app: None,
    });

    let mut state = read_switch_state(paths);
    state.prefer_chinese_ui = enable;
    save_switch_state(paths, &state)?;

    for &app_id in &[AppId::Codex, AppId::Claude, AppId::Antigravity] {
        let adapter = get_adapter(app_id);
        if adapter.read_mode(paths) == AppMode::Tako {
            let _ = adapter.apply_mode(paths, AppMode::Tako);
        }
    }
    sync_standalone_chinese_for_installed(paths, enable);

    let mut prompts: Vec<adapters::UiPatchPermissionPrompt> = Vec::new();
    let mut handled_restarts: std::collections::HashSet<AppId> = std::collections::HashSet::new();

    if patch_ui {
        let mut targets: Vec<AppId> = Vec::new();
        for &app_id in &[AppId::Claude, AppId::Antigravity] {
            let info = inspect_ui_patch_for_app(paths, app_id);
            if enable {
                let needs_patch = matches!(
                    info.status,
                    adapters::UiPatchStatus::Unpatched
                        | adapters::UiPatchStatus::OutdatedAfterUpgrade
                ) || (info.status == adapters::UiPatchStatus::Patched && info.update_available);
                if needs_patch {
                    targets.push(app_id);
                }
            } else if info.status == adapters::UiPatchStatus::Patched {
                targets.push(app_id);
            }
        }

        let total = targets.len();
        if total == 0 {
            on_progress(UiPatchProgressEvent {
                scope: "all".into(),
                action: action.into(),
                stage: "finalizing".into(),
                percent: 88,
                message: "配置已同步，正在刷新客户端状态...".into(),
                current_app: None,
            });
        } else {
            for (idx, &app_id) in targets.iter().enumerate() {
                let label = ui_patch_app_label(app_id);
                let start_pct = 24u8
                    .saturating_add((idx * 64).checked_div(total).unwrap_or(0) as u8);
                let done_pct = 24u8
                    .saturating_add(((idx + 1) * 64).checked_div(total).unwrap_or(64) as u8);
                let step_tag = if total > 1 {
                    format!("（{}/{}）", idx + 1, total)
                } else {
                    String::new()
                };
                on_progress(UiPatchProgressEvent {
                    scope: "all".into(),
                    action: action.into(),
                    stage: if enable { "patching" } else { "restoring" }.into(),
                    percent: start_pct,
                    message: if enable {
                        format!("正在注入 {label} 简体中文词典并更新签名{step_tag}...")
                    } else {
                        format!("正在无损还原 {label} 原版界面与签名{step_tag}...")
                    },
                    current_app: Some(app_id),
                });

                match execute_ui_patch_action_for_paths(paths, app_id, action, auto_restart) {
                    Ok(_) => {
                        handled_restarts.insert(app_id);
                        on_progress(UiPatchProgressEvent {
                            scope: "all".into(),
                            action: action.into(),
                            stage: if enable { "patching" } else { "restoring" }.into(),
                            percent: done_pct,
                            message: if enable {
                                format!("{label} 简体中文注入完成{step_tag}")
                            } else {
                                format!("{label} 原版界面还原完成{step_tag}")
                            },
                            current_app: Some(app_id),
                        });
                    }
                    Err(err) => {
                        if let Some(prompt) = adapters::parse_ui_patch_block_error(&err) {
                            prompts.push(prompt);
                            on_progress(UiPatchProgressEvent {
                                scope: "all".into(),
                                action: action.into(),
                                stage: "finalizing".into(),
                                percent: done_pct,
                                message: format!("{label} 需要系统授权后继续"),
                                current_app: Some(app_id),
                            });
                        }
                    }
                }
            }
        }
    }

    if auto_restart && !paths.is_test {
        on_progress(UiPatchProgressEvent {
            scope: "all".into(),
            action: action.into(),
            stage: "finalizing".into(),
            percent: 94,
            message: "正在重启运行中的客户端...".into(),
            current_app: None,
        });
        for &app_id in &[AppId::Codex, AppId::Claude, AppId::Antigravity] {
            if handled_restarts.contains(&app_id) {
                continue;
            }
            let adapter = get_adapter(app_id);
            if adapter.is_running(paths) {
                let _ = adapter.quit_app(paths);
                std::thread::sleep(std::time::Duration::from_millis(450));
                let _ = client_monitor::launch_and_monitor(paths, app_id, "chinese_toggle_restart");
            }
        }
    }

    on_progress(UiPatchProgressEvent {
        scope: "all".into(),
        action: action.into(),
        stage: "done".into(),
        percent: 100,
        message: if !prompts.is_empty() {
            "配置已同步，部分客户端需系统授权后完成界面处理".into()
        } else if enable {
            "三端简体中文汉化已完成".into()
        } else {
            "三端原版配置与界面已恢复".into()
        },
        current_app: None,
    });

    Ok(prompts)
}

#[tauri::command]
async fn set_prefer_chinese_ui(
    app: AppHandle,
    enable: bool,
    patch_antigravity: bool,
    auto_restart: bool,
) -> Result<DashboardPayload, String> {
    let paths_for_sync = AppPaths::detect()?;
    let app_for_emit = app.clone();
    let res = tauri::async_runtime::spawn_blocking(move || {
        apply_prefer_chinese_ui_for_paths_with_progress(
            &paths_for_sync,
            enable,
            patch_antigravity,
            auto_restart,
            |ev| {
                let _ = app_for_emit.emit("ui-patch-progress", &ev);
            },
        )
    })
    .await
    .map_err(|err| format!("同步中文设置失败: {err}"))?;

    let prompts = match res {
        Ok(p) => p,
        Err(err) => {
            let _ = app.emit(
                "ui-patch-progress",
                UiPatchProgressEvent {
                    scope: "all".into(),
                    action: if enable { "patch" } else { "restore" }.into(),
                    stage: "error".into(),
                    percent: 100,
                    message: err.clone(),
                    current_app: None,
                },
            );
            return Err(err);
        }
    };

    if let Ok(paths) = AppPaths::detect() {
        refresh_tray(&app, &paths);
    }
    let mut dashboard = get_dashboard(app, false).await?;
    dashboard.ui_patch_prompts = prompts;
    Ok(dashboard)
}

#[tauri::command]
async fn patch_app_ui(
    app: AppHandle,
    app_id: AppId,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let started_at = std::time::Instant::now();
    let paths_clone = paths.clone();
    let app_for_emit = app.clone();
    let scope = app_id.as_str().to_string();
    let label = ui_patch_app_label(app_id);

    let _ = app.emit(
        "ui-patch-progress",
        UiPatchProgressEvent {
            scope: scope.clone(),
            action: "patch".into(),
            stage: "preparing".into(),
            percent: 14,
            message: format!("正在校验 {label} 安装包与原始备份..."),
            current_app: Some(app_id),
        },
    );

    let scope_worker = scope.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<AppSnapshot, String> {
        let _ = app_for_emit.emit(
            "ui-patch-progress",
            UiPatchProgressEvent {
                scope: scope_worker.clone(),
                action: "patch".into(),
                stage: "patching".into(),
                percent: 38,
                message: format!("正在解包并注入 {label} 简体中文词典与签名..."),
                current_app: Some(app_id),
            },
        );
        execute_ui_patch_action_for_paths(&paths_clone, app_id, "patch", auto_restart)?;
        let _ = app_for_emit.emit(
            "ui-patch-progress",
            UiPatchProgressEvent {
                scope: scope_worker.clone(),
                action: "patch".into(),
                stage: "finalizing".into(),
                percent: 88,
                message: format!("正在同步 {label} 语言配置并验证状态..."),
                current_app: Some(app_id),
            },
        );
        if read_switch_state(&paths_clone).prefer_chinese_ui {
            match app_id {
                AppId::Antigravity => {
                    let _ = adapters::antigravity::AntigravityAdapter::sync_standalone_chinese_locale(
                        &paths_clone,
                        true,
                    );
                }
                AppId::Claude => {
                    let _ = adapters::claude::ClaudeAdapter::sync_standalone_chinese_locale(
                        &paths_clone,
                        true,
                    );
                }
                _ => {}
            }
        }
        Ok(snapshot_app(&paths_clone, app_id))
    })
    .await
    .map_err(|err| format!("执行界面汉化失败: {err}"))?;

    let snapshot = match res {
        Ok(snap) => {
            let _ = app.emit(
                "ui-patch-progress",
                UiPatchProgressEvent {
                    scope,
                    action: "patch".into(),
                    stage: "done".into(),
                    percent: 100,
                    message: format!("{label} 完整简体中文已就绪"),
                    current_app: Some(app_id),
                },
            );
            telemetry::track_event(
                &paths,
                "ui_patch_apply",
                app_id.as_str(),
                true,
                started_at.elapsed().as_millis() as u64,
                serde_json::json!({
                    "auto_restart": auto_restart,
                    "patch_version": snap.ui_patch.patch_version,
                }),
            );
            snap
        }
        Err(err) => {
            telemetry::track_event(
                &paths,
                "ui_patch_apply",
                app_id.as_str(),
                false,
                started_at.elapsed().as_millis() as u64,
                serde_json::json!({
                    "auto_restart": auto_restart,
                    "permission_block": err.contains("[TAKO_UI_PATCH_BLOCK]"),
                }),
            );
            if !err.contains("[TAKO_UI_PATCH_BLOCK]") {
                telemetry::report_error(
                    &paths,
                    telemetry::ErrorReport {
                        source: "rust_error",
                        category: "ui_patch",
                        error_type: "UiPatchApplyFailed",
                        message: &err,
                        location: "lib.rs:patch_app_ui",
                        stack: "",
                        context: serde_json::json!({"app_id": app_id.as_str()}),
                    },
                );
            }
            let _ = app.emit(
                "ui-patch-progress",
                UiPatchProgressEvent {
                    scope,
                    action: "patch".into(),
                    stage: "error".into(),
                    percent: 100,
                    message: format!("{label} 汉化未完成"),
                    current_app: Some(app_id),
                },
            );
            return Err(err);
        }
    };

    refresh_tray(&app, &paths);
    Ok(snapshot)
}

#[tauri::command]
async fn restore_app_ui(
    app: AppHandle,
    app_id: AppId,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let started_at = std::time::Instant::now();
    let paths_clone = paths.clone();
    let app_for_emit = app.clone();
    let scope = app_id.as_str().to_string();
    let label = ui_patch_app_label(app_id);

    let _ = app.emit(
        "ui-patch-progress",
        UiPatchProgressEvent {
            scope: scope.clone(),
            action: "restore".into(),
            stage: "preparing".into(),
            percent: 16,
            message: format!("正在校验 {label} 无损备份..."),
            current_app: Some(app_id),
        },
    );

    let scope_worker = scope.clone();
    let res = tauri::async_runtime::spawn_blocking(move || -> Result<AppSnapshot, String> {
        let _ = app_for_emit.emit(
            "ui-patch-progress",
            UiPatchProgressEvent {
                scope: scope_worker.clone(),
                action: "restore".into(),
                stage: "restoring".into(),
                percent: 45,
                message: format!("正在无损还原 {label} 原版文件与签名..."),
                current_app: Some(app_id),
            },
        );
        execute_ui_patch_action_for_paths(&paths_clone, app_id, "restore", auto_restart)?;
        let _ = app_for_emit.emit(
            "ui-patch-progress",
            UiPatchProgressEvent {
                scope: scope_worker.clone(),
                action: "restore".into(),
                stage: "finalizing".into(),
                percent: 90,
                message: format!("正在校验 {label} 还原结果..."),
                current_app: Some(app_id),
            },
        );
        Ok(snapshot_app(&paths_clone, app_id))
    })
    .await
    .map_err(|err| format!("还原原版界面失败: {err}"))?;

    let snapshot = match res {
        Ok(snap) => {
            let _ = app.emit(
                "ui-patch-progress",
                UiPatchProgressEvent {
                    scope,
                    action: "restore".into(),
                    stage: "done".into(),
                    percent: 100,
                    message: format!("{label} 已无损还原原版"),
                    current_app: Some(app_id),
                },
            );
            telemetry::track_event(
                &paths,
                "ui_patch_restore",
                app_id.as_str(),
                true,
                started_at.elapsed().as_millis() as u64,
                serde_json::json!({"auto_restart": auto_restart}),
            );
            snap
        }
        Err(err) => {
            telemetry::track_event(
                &paths,
                "ui_patch_restore",
                app_id.as_str(),
                false,
                started_at.elapsed().as_millis() as u64,
                serde_json::json!({
                    "auto_restart": auto_restart,
                    "permission_block": err.contains("[TAKO_UI_PATCH_BLOCK]"),
                }),
            );
            if !err.contains("[TAKO_UI_PATCH_BLOCK]") {
                telemetry::report_error(
                    &paths,
                    telemetry::ErrorReport {
                        source: "rust_error",
                        category: "ui_patch",
                        error_type: "UiPatchRestoreFailed",
                        message: &err,
                        location: "lib.rs:restore_app_ui",
                        stack: "",
                        context: serde_json::json!({"app_id": app_id.as_str()}),
                    },
                );
            }
            let _ = app.emit(
                "ui-patch-progress",
                UiPatchProgressEvent {
                    scope,
                    action: "restore".into(),
                    stage: "error".into(),
                    percent: 100,
                    message: format!("{label} 还原未完成"),
                    current_app: Some(app_id),
                },
            );
            return Err(err);
        }
    };

    refresh_tray(&app, &paths);
    Ok(snapshot)
}

#[tauri::command]
async fn patch_antigravity_ui(app: AppHandle, auto_restart: bool) -> Result<AppSnapshot, String> {
    patch_app_ui(app, AppId::Antigravity, auto_restart).await
}

#[tauri::command]
async fn restore_antigravity_ui(
    app: AppHandle,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    restore_app_ui(app, AppId::Antigravity, auto_restart).await
}

#[tauri::command]
fn open_macos_app_management_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles")
            .output()
            .map_err(|e| format!("打开系统设置失败: {e}"))?;
        if out.status.success() {
            return Ok(());
        }
        let fallback = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy")
            .output()
            .map_err(|e| format!("打开隐私与安全性设置失败: {e}"))?;
        if fallback.status.success() {
            Ok(())
        } else {
            Err(format!(
                "无法打开系统设置: {}",
                String::from_utf8_lossy(&fallback.stderr).trim()
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("仅支持在 macOS 上打开「隐私与安全性 -> App 管理」设置".to_string())
    }
}

#[tauri::command]
async fn run_elevated_ui_patch_worker(
    app: AppHandle,
    app_id: AppId,
    action: String,
) -> Result<AppSnapshot, String> {
    let normalized_action = match action.trim() {
        "patch" => "patch",
        "restore" => "restore",
        other => return Err(format!("不支持的界面补丁动作: {other}")),
    };
    let paths = AppPaths::detect()?;
    let paths_clone = paths.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || -> Result<AppSnapshot, String> {
        #[cfg(target_os = "windows")]
        {
            let quit_for_patch = quit_app_and_wait_for_ui_patch(&paths_clone, app_id, true, false)?;
            let elevated_ok = try_windows_elevated_ui_patch(&paths_clone, app_id, normalized_action)?;
            if quit_for_patch && !paths_clone.is_test {
                let _ = client_monitor::launch_and_monitor(
                    &paths_clone,
                    app_id,
                    "elevated_ui_patch_relaunch",
                );
            }
            if !elevated_ok {
                return Err("管理员授权被取消或执行失败".to_string());
            }
            Ok(snapshot_app(&paths_clone, app_id))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (paths_clone, app_id, normalized_action);
            Err("管理员提权补丁模式仅适用于 Windows".to_string())
        }
    })
    .await
    .map_err(|err| format!("执行管理员提权补丁失败: {err}"))??;

    refresh_tray(&app, &paths);
    Ok(snapshot)
}

#[tauri::command]
async fn list_plaza_models(
    force_refresh: Option<bool>,
) -> Result<agy_proxy::ClientModelsResponse, String> {
    let paths = AppPaths::detect()?;
    agy_proxy::fetch_plaza_models_for_ui_with_refresh(&paths, force_refresh.unwrap_or(false))
        .await
}

#[tauri::command]
async fn get_running_states() -> Result<std::collections::BTreeMap<String, bool>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(target_os = "windows")]
        {
            adapters::windows_process::running_states()
        }
        #[cfg(not(target_os = "windows"))]
        {
            let paths = AppPaths::detect()?;
            Ok(AppId::all()
                .iter()
                .map(|&id| (id.as_str().to_string(), get_adapter(id).is_running(&paths)))
                .collect())
        }
    })
    .await
    .map_err(|err| format!("读取运行状态失败: {err}"))?
}

async fn ensure_default_tako_mode_before_launch(
    paths: &AppPaths,
    app_id: AppId,
) -> Result<bool, String> {
    if config::read_main_api_key(paths).is_none() {
        return Ok(false);
    }
    let state = read_switch_state(paths);
    if state.is_explicit_official(app_id) {
        return Ok(false);
    }
    let is_official = get_adapter(app_id).read_mode(paths) == AppMode::Official;
    if !is_official {
        return Ok(false);
    }
    if app_id == AppId::Claude {
        let _ = crate::agy_proxy::refresh_claude_catalog_cache(paths).await;
    } else if app_id == AppId::WorkBuddy {
        let _ = crate::agy_proxy::refresh_workbuddy_catalog_cache(paths).await;
    }
    get_adapter(app_id).apply_mode(paths, AppMode::Tako)?;
    Ok(true)
}

#[tauri::command]
async fn set_app_mode(
    app: AppHandle,
    app_id: AppId,
    mode: AppMode,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let mut state = read_switch_state(&paths);
    state.set_explicit_official(app_id, mode == AppMode::Official);
    save_switch_state(&paths, &state)?;
    if mode == AppMode::Tako {
        if app_id == AppId::Claude {
            let _ = crate::agy_proxy::refresh_claude_catalog_cache(&paths).await;
        } else if app_id == AppId::WorkBuddy {
            let _ = crate::agy_proxy::refresh_workbuddy_catalog_cache(&paths).await;
        }
    }
    let adapter = get_adapter(app_id);
    let mode_str = match mode {
        AppMode::Tako => "tako",
        AppMode::Official => "official",
    };
    if let Err(err) = adapter.apply_mode(&paths, mode) {
        telemetry::track_event(
            &paths,
            "client_switch",
            app_id.as_str(),
            false,
            0,
            serde_json::json!({
                "mode": mode_str,
                "auto_restart": auto_restart,
            }),
        );
        telemetry::report_error(
            &paths,
            telemetry::ErrorReport {
                source: "rust_error",
                category: "switch",
                error_type: "ClientSwitchFailed",
                message: &err,
                location: "lib.rs:set_app_mode",
                stack: "",
                context: serde_json::json!({
                    "app_id": app_id.as_str(),
                    "mode": mode_str,
                }),
            },
        );
        return Err(err);
    }
    telemetry::track_event(
        &paths,
        "client_switch",
        app_id.as_str(),
        true,
        0,
        serde_json::json!({
            "mode": mode_str,
            "auto_restart": auto_restart,
        }),
    );
    if auto_restart && adapter.is_running(&paths) {
        adapter.quit_app(&paths)?;
        std::thread::sleep(std::time::Duration::from_millis(450));
        if app_id == AppId::WorkBuddy {
            let _ = adapters::workbuddy::WorkBuddyAdapter::flush_after_stop_or_before_launch(
                &paths,
            );
        }
        client_monitor::launch_and_monitor(&paths, app_id, "mode_switch_restart")?;
    }
    refresh_tray(&app, &paths);
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
async fn restart_app(app: AppHandle, app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    {
        let adapter = get_adapter(app_id);
        if adapter.is_running(&paths) {
            adapter.quit_app(&paths)?;
            std::thread::sleep(std::time::Duration::from_millis(450));
        }
    }
    let switched = ensure_default_tako_mode_before_launch(&paths, app_id).await?;
    if app_id == AppId::Claude {
        let _ = adapters::claude::ClaudeAdapter::ensure_tako_profile_up_to_date(&paths);
    }
    if app_id == AppId::WorkBuddy {
        let _ = adapters::workbuddy::WorkBuddyAdapter::flush_after_stop_or_before_launch(&paths);
    }
    client_monitor::launch_and_monitor(&paths, app_id, "restart")?;
    if switched {
        refresh_tray(&app, &paths);
    }
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn close_app(app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let adapter = get_adapter(app_id);
    if adapter.is_running(&paths) {
        adapter.quit_app(&paths)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        if app_id == AppId::WorkBuddy {
            let _ = adapters::workbuddy::WorkBuddyAdapter::flush_after_stop_or_before_launch(
                &paths,
            );
        }
    }
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
async fn launch_app(app: AppHandle, app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let was_running = get_adapter(app_id).is_running(&paths);
    // Only apply lazy default Tako switch when the client is NOT currently running,
    // so an active official session is never disrupted mid-conversation.
    let switched = if !was_running {
        ensure_default_tako_mode_before_launch(&paths, app_id).await?
    } else {
        false
    };
    if !was_running && app_id == AppId::Claude {
        let _ = adapters::claude::ClaudeAdapter::ensure_tako_profile_up_to_date(&paths);
    }
    if !was_running && app_id == AppId::WorkBuddy {
        let _ = adapters::workbuddy::WorkBuddyAdapter::flush_after_stop_or_before_launch(&paths);
    }
    client_monitor::launch_and_monitor(&paths, app_id, "launch")?;
    if switched {
        refresh_tray(&app, &paths);
    }
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn diagnose_app(app_id: AppId) -> Result<DiagnosticResult, String> {
    let paths = AppPaths::detect()?;
    Ok(get_adapter(app_id).diagnose(&paths))
}

#[tauri::command]
async fn list_client_models(
    app_id: AppId,
    force_refresh: Option<bool>,
) -> Result<agy_proxy::ClientModelsResponse, String> {
    let paths = AppPaths::detect()?;
    agy_proxy::fetch_client_models_for_ui_with_refresh(
        &paths,
        app_id,
        force_refresh.unwrap_or(false),
    )
    .await
}

#[tauri::command]
async fn save_client_model_filter(
    app: AppHandle,
    app_id: AppId,
    filter: config::ClientModelFilter,
) -> Result<SwitchState, String> {
    let paths = AppPaths::detect()?;
    let normalized = filter.normalized();
    if normalized.mode == config::ModelFilterMode::Custom && normalized.enabled_ids.is_empty() {
        return Err("至少保留 1 个展示模型".to_string());
    }
    let mut state = read_switch_state(&paths);
    *state.model_filters.for_app_mut(app_id) = normalized;
    save_switch_state(&paths, &state)?;
    if app_id == AppId::Claude {
        let _ = crate::agy_proxy::refresh_claude_catalog_cache(&paths).await;
        let adapter = get_adapter(app_id);
        if adapter.read_mode(&paths) == AppMode::Tako {
            adapter.apply_mode(&paths, AppMode::Tako)?;
        }
    } else if app_id == AppId::WorkBuddy {
        let _ = crate::agy_proxy::refresh_workbuddy_catalog_cache(&paths).await;
        let adapter = get_adapter(app_id);
        if adapter.read_mode(&paths) == AppMode::Tako {
            adapter.apply_mode(&paths, AppMode::Tako)?;
        }
    }
    let _ = app.emit("state-changed", ());
    Ok(state)
}

#[tauri::command]
async fn check_app_updates(app: AppHandle) -> Result<ManifestCheckResult, String> {
    let paths = AppPaths::detect()?;
    let (source_url, manifest) = fetch_manifest(&paths).await?;
    let updates = compute_updates(&paths, &manifest);
    let app_for_i18n = app.clone();
    tauri::async_runtime::spawn(async move {
        run_background_ui_patch_auto_sync(&app_for_i18n, true).await;
    });
    Ok(ManifestCheckResult {
        source_url,
        manifest,
        updates,
    })
}

#[tauri::command]
fn cancel_app_install(guard: tauri::State<'_, InstallGuard>, app_id: AppId) {
    if let Ok(map) = guard.cancels.lock() {
        if let Some(flag) = map.get(&app_id) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[tauri::command]
fn ensure_local_gateway() -> Result<bool, String> {
    let paths = AppPaths::detect()?;
    agy_proxy::ensure_gateway_service(&paths)?;
    Ok(agy_proxy::is_proxy_healthy(agy_proxy::LOCAL_GATEWAY_PORT))
}

#[tauri::command]
async fn run_mcp_live_demo(
    input: mcp_server::LiveDemoRequest,
) -> Result<mcp_server::LiveDemoResult, String> {
    let paths = AppPaths::detect()?;
    mcp_server::execute_live_demo_for_paths(&paths, input).await
}

use extensions::with_extension_paths;

#[tauri::command]
async fn get_extensions_summary() -> Result<extensions::ExtensionsSummary, String> {
    with_extension_paths(extensions::get_extensions_summary_for_paths).await
}

#[tauri::command]
async fn get_mcp_hub_snapshot() -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(extensions::mcp::get_mcp_hub_snapshot_for_paths).await
}

#[tauri::command]
async fn upsert_mcp_entry(
    entry: extensions::mcp::McpEntry,
) -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(move |paths| extensions::mcp::upsert_mcp_entry_for_paths(paths, entry)).await
}

#[tauri::command]
async fn toggle_mcp_platform(
    id: String,
    platform: extensions::ExtPlatform,
    enabled: bool,
) -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::mcp::toggle_mcp_platform_for_paths(paths, &id, platform, enabled)
    })
    .await
}

#[tauri::command]
async fn delete_mcp_entry(id: String) -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(move |paths| extensions::mcp::delete_mcp_entry_for_paths(paths, &id)).await
}

#[tauri::command]
async fn align_mcp_entry(
    id: String,
    source: Option<extensions::ExtPlatform>,
) -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::mcp::align_mcp_entry_for_paths(paths, &id, source)
    })
    .await
}

#[tauri::command]
async fn align_all_mcp_entries() -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(extensions::mcp::align_all_mcp_entries_for_paths).await
}

#[tauri::command]
fn parse_mcp_snippet(snippet: String) -> Result<Vec<extensions::mcp::McpEntry>, String> {
    extensions::mcp::parse_mcp_snippet(&snippet)
}

#[tauri::command]
async fn export_mcp_backup() -> Result<String, String> {
    with_extension_paths(extensions::mcp::export_mcp_backup_for_paths).await
}

#[tauri::command]
async fn import_mcp_backup(backup: String) -> Result<extensions::mcp::McpHubSnapshot, String> {
    with_extension_paths(move |paths| extensions::mcp::import_mcp_backup_for_paths(paths, &backup)).await
}

#[tauri::command]
async fn get_prompts_hub_snapshot() -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(extensions::prompts::get_prompts_hub_snapshot_for_paths).await
}

#[tauri::command]
async fn upsert_prompt_preset(
    platform: extensions::ExtPlatform,
    preset: extensions::prompts::PromptPreset,
) -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::prompts::upsert_prompt_preset_for_paths(paths, platform, preset)
    })
    .await
}

#[tauri::command]
async fn activate_prompt_preset(
    platform: extensions::ExtPlatform,
    id: Option<String>,
) -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::prompts::activate_prompt_preset_for_paths(paths, platform, id.as_deref())
    })
    .await
}

#[tauri::command]
async fn delete_prompt_preset(
    platform: extensions::ExtPlatform,
    id: String,
) -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::prompts::delete_prompt_preset_for_paths(paths, platform, &id)
    })
    .await
}

#[tauri::command]
async fn sync_prompt_to_platforms(
    source: extensions::ExtPlatform,
    id: String,
    targets: Vec<extensions::ExtPlatform>,
    activate: bool,
) -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::prompts::sync_prompt_to_platforms_for_paths(paths, source, &id, &targets, activate)
    })
    .await
}

#[tauri::command]
async fn export_prompts_backup() -> Result<String, String> {
    with_extension_paths(extensions::prompts::export_prompts_backup_for_paths).await
}

#[tauri::command]
async fn import_prompts_backup(
    backup: String,
) -> Result<extensions::prompts::PromptsHubSnapshot, String> {
    with_extension_paths(move |paths| {
        extensions::prompts::import_prompts_backup_for_paths(paths, &backup)
    })
    .await
}

#[tauri::command]
async fn get_plugins_hub_snapshot(
    platform: extensions::ExtPlatform,
    include_available: Option<bool>,
) -> Result<extensions::plugins::PluginsHubSnapshot, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::get_plugins_hub_snapshot_for_paths(
        &paths,
        platform,
        include_available.unwrap_or(true),
    )
    .await
}

#[tauri::command]
async fn toggle_plugin_enabled(
    platform: extensions::ExtPlatform,
    id: String,
    enabled: bool,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::Toggle(enabled), &id,
    )
    .await
}

#[tauri::command]
async fn install_plugin_entry(
    platform: extensions::ExtPlatform,
    id: String,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::Install, &id,
    )
    .await
}

#[tauri::command]
async fn update_plugin_entry(
    platform: extensions::ExtPlatform,
    id: String,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::Update, &id,
    )
    .await
}

#[tauri::command]
async fn uninstall_plugin_entry(
    platform: extensions::ExtPlatform,
    id: String,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::Uninstall, &id,
    )
    .await
}

#[tauri::command]
async fn add_plugin_marketplace(
    platform: extensions::ExtPlatform,
    source: String,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::AddMarketplace, &source,
    )
    .await
}

#[tauri::command]
async fn remove_plugin_marketplace(
    platform: extensions::ExtPlatform,
    name: String,
) -> Result<extensions::plugins::PluginActionOutcome, extensions::plugins::PluginError> {
    let paths = AppPaths::detect()?;
    extensions::plugins::execute_plugin_action_for_paths(
        &paths, platform, extensions::plugins::PluginAction::RemoveMarketplace, &name,
    )
    .await
}

#[tauri::command]
async fn check_self_update_smart(app: AppHandle) -> Result<SelfUpdateCheckResult, String> {
    perform_self_update_check(&app).await
}

#[cfg(target_os = "windows")]
fn stop_auxiliary_windows_gateway_processes() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let current_pid = std::process::id();
    let script = format!(
        "Get-CimInstance Win32_Process -Filter \"Name='tako-switch.exe'\" | Where-Object {{ $_.ProcessId -ne {current_pid} -and $_.CommandLine -match 'gateway-proxy|agy-proxy' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[tauri::command]
async fn install_self_update(
    app: AppHandle,
    pending: tauri::State<'_, PendingSelfUpdate>,
) -> Result<(), String> {
    let update = {
        let guard = pending.0.lock().await;
        guard.clone()
    }
    .ok_or_else(|| "当前没有可安装的新版本，请先检查更新".to_string())?;

    let app_for_chunk = app.clone();
    let app_for_finish = app.clone();
    let downloaded = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let total_len = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let started_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let dl_chunk = downloaded.clone();
    let tot_chunk = total_len.clone();
    let dl_fin = downloaded.clone();
    let tot_fin = total_len.clone();

    let bytes = update
        .download(
            move |chunk_len, content_len| {
                let tot = content_len.unwrap_or(0);
                if tot > 0 {
                    tot_chunk.store(tot, std::sync::atomic::Ordering::Relaxed);
                }
                if !started_flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    let _ = app_for_chunk.emit(
                        "self-update-progress",
                        SelfUpdateProgressEvent {
                            stage: "started".to_string(),
                            downloaded: 0,
                            total: (tot > 0).then_some(tot),
                        },
                    );
                }
                let prev = dl_chunk.fetch_add(chunk_len as u64, std::sync::atomic::Ordering::Relaxed);
                let cur = prev + chunk_len as u64;
                let cur_tot = tot_chunk.load(std::sync::atomic::Ordering::Relaxed);
                let _ = app_for_chunk.emit(
                    "self-update-progress",
                    SelfUpdateProgressEvent {
                        stage: "progress".to_string(),
                        downloaded: cur,
                        total: (cur_tot > 0).then_some(cur_tot),
                    },
                );
            },
            move || {
                let cur = dl_fin.load(std::sync::atomic::Ordering::Relaxed);
                let cur_tot = tot_fin.load(std::sync::atomic::Ordering::Relaxed);
                let _ = app_for_finish.emit(
                    "self-update-progress",
                    SelfUpdateProgressEvent {
                        stage: "finished".to_string(),
                        downloaded: cur,
                        total: (cur_tot > 0).then_some(cur_tot).or(Some(cur)),
                    },
                );
            },
        )
        .await
        .map_err(|e| format!("下载 Tako Switch 更新包失败: {e}"))?;

    #[cfg(target_os = "windows")]
    stop_auxiliary_windows_gateway_processes();

    update
        .install(&bytes)
        .map_err(|e| format!("安装 Tako Switch 更新包失败: {e}"))?;

    app.restart();
}

#[tauri::command]
async fn install_or_upgrade_app(
    app: AppHandle,
    guard: tauri::State<'_, InstallGuard>,
    app_id: AppId,
) -> Result<InstallOutcome, String> {
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut map) = guard.cancels.lock() {
        map.insert(app_id, cancel_flag.clone());
    }

    let _lk = guard.lock.lock().await;
    let started_at = std::time::Instant::now();
    let outcome: Result<InstallOutcome, String> = async {
        let paths = AppPaths::detect()?;
        let manifest = match fetch_manifest(&paths).await {
            Ok((_, m)) => m,
            Err(fetch_err) => load_cached_manifest(&paths).ok_or(fetch_err)?,
        };
        let (_, artifact) = resolve_platform_artifact(&manifest, app_id)
            .ok_or_else(|| format!("当前系统架构暂无 {} 的安装包条目", app_id.display_name()))?;
        let outcome =
            installer::install_or_upgrade_app(Some(&app), &paths, app_id, artifact, Some(cancel_flag))
                .await?;
        if read_switch_state(&paths).prefer_chinese_ui {
            let paths_for_zh = paths.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                sync_standalone_chinese_for_installed(&paths_for_zh, true);
                if matches!(app_id, AppId::Claude | AppId::Antigravity) {
                    let _ = auto_sync_pending_ui_patches_for_paths(&paths_for_zh, Some(app_id));
                }
            })
            .await;
        }
        refresh_tray(&app, &paths);
        Ok(outcome)
    }
    .await;

    if let Ok(paths) = AppPaths::detect() {
        let total_ms = started_at.elapsed().as_millis() as u64;
        match &outcome {
            Ok(res) => {
                telemetry::track_event(
                    &paths,
                    "client_install_done",
                    app_id.as_str(),
                    true,
                    res.download_duration_ms.unwrap_or(total_ms),
                    serde_json::json!({
                        "route": res.download_route,
                        "bytes": res.download_bytes,
                        "speed_kbps": res.download_speed_kbps,
                        "total_duration_ms": total_ms,
                    }),
                );
            }
            Err(err) if err != "已取消下载" => {
                telemetry::track_event(
                    &paths,
                    "client_install_done",
                    app_id.as_str(),
                    false,
                    total_ms,
                    serde_json::json!({
                        "error": err,
                    }),
                );
                telemetry::report_error(
                    &paths,
                    telemetry::ErrorReport {
                        source: "rust_error",
                        category: "installer",
                        error_type: "ClientInstallFailed",
                        message: err,
                        location: "lib.rs:install_or_upgrade_app",
                        stack: "",
                        context: serde_json::json!({"app_id": app_id.as_str()}),
                    },
                );
            }
            Err(_) => {}
        }
    }

    if let Ok(mut map) = guard.cancels.lock() {
        map.remove(&app_id);
    }
    outcome
}

#[tauri::command]
fn report_frontend_error(input: telemetry::FrontendErrorInput) -> Result<(), String> {
    if let Ok(paths) = AppPaths::detect() {
        telemetry::report_frontend_error(&paths, input);
    }
    Ok(())
}

#[tauri::command]
async fn submit_user_feedback(
    input: telemetry::FeedbackSubmission,
) -> Result<telemetry::FeedbackReceipt, String> {
    let paths = AppPaths::detect()?;
    telemetry::submit_feedback(&paths, input).await
}

#[tauri::command]
async fn get_sanitized_diagnostics_preview() -> Result<serde_json::Value, String> {
    let paths = AppPaths::detect()?;
    tauri::async_runtime::spawn_blocking(move || telemetry::build_sanitized_diagnostics(&paths))
        .await
        .map_err(|e| format!("生成脱敏诊断预览失败: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    telemetry::install_panic_hook();
    let initial_cache = AppPaths::detect()
        .ok()
        .and_then(|p| read_balance_cache(&p))
        .map(|c| c.status);

    tauri::Builder::default()
        .manage(InstallGuard {
            lock: Mutex::new(()),
            cancels: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
        .manage(CachedAccount(std::sync::Mutex::new(initial_cache)))
        .manage(PendingSelfUpdate(Mutex::new(None)))
        .manage(TrayUpdateNotice(std::sync::Mutex::new(None)))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let paths = AppPaths::detect().map_err(std::io::Error::other)?;
            #[cfg(target_os = "macos")]
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            #[cfg(not(target_os = "macos"))]
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/64x64.png"))?;
            let menu = build_tray_menu(app.handle(), &paths).map_err(std::io::Error::other)?;

            let builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "tray:show" {
                        show_main_window(app);
                    } else if id == "tray:refresh-balance" {
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Ok(paths) = AppPaths::detect() {
                                let status = fetch_account_status(&paths).await;
                                update_account_and_tray(&handle, &paths, status, true);
                            }
                        });
                    } else if id == "tray:updates" || id == "tray:updates-check" {
                        show_main_window(app);
                        let _ = app.emit("request-check-updates", ());
                    } else if id == "tray:quit" {
                        app.exit(0);
                    } else if let Some(key_id) = id.strip_prefix("key:") {
                        let key_id_owned = key_id.to_string();
                        let handle = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Ok(paths) = AppPaths::detect() {
                                if let Ok(status) =
                                    auth_switch_saved_key(&paths, &key_id_owned).await
                                {
                                    sync_after_key_change(&paths).await;
                                    telemetry::track_event(
                                        &paths,
                                        "key_switch",
                                        "tako-switch",
                                        true,
                                        0,
                                        serde_json::json!({
                                            "saved_keys_count": status.saved_keys.len(),
                                            "via": "tray",
                                        }),
                                    );
                                    update_account_and_tray(&handle, &paths, status, true);
                                    let _ = handle.emit("state-changed", ());
                                }
                            }
                        });
                    } else if let Some((app_id, mode)) = parse_tray_mode_id(id) {
                        if let Ok(paths) = AppPaths::detect() {
                            let mut st = read_switch_state(&paths);
                            st.set_explicit_official(app_id, mode == AppMode::Official);
                            let _ = save_switch_state(&paths, &st);
                            if get_adapter(app_id).apply_mode(&paths, mode).is_ok() {
                                let mode_str = match mode {
                                    AppMode::Tako => "tako",
                                    AppMode::Official => "official",
                                };
                                telemetry::track_event(
                                    &paths,
                                    "client_switch",
                                    app_id.as_str(),
                                    true,
                                    0,
                                    serde_json::json!({
                                        "mode": mode_str,
                                        "via": "tray",
                                    }),
                                );
                                refresh_tray(app, &paths);
                                let _ = app.emit("state-changed", ());
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });

            #[cfg(target_os = "macos")]
            let builder = builder.icon_as_template(true);

            builder.build(app)?;
            update_tray(app.handle(), &paths, false);

            let any_tako = AppId::all()
                .iter()
                .copied()
                .filter(|id| id.requires_local_gateway())
                .any(|id| get_adapter(id).read_mode(&paths) == AppMode::Tako);
            if any_tako {
                let _ = agy_proxy::ensure_gateway_service(&paths);
            }
            let _ = adapters::claude::ClaudeAdapter::ensure_tako_profile_up_to_date(&paths);

            // Refresh Claude 3p and WorkBuddy model catalog caches shortly after startup so
            // upgraded clients automatically pick up updated recommended models (e.g. swe-2-1)
            // and newly supported multi-vendor Tako models without manual re-saving.
            tauri::async_runtime::spawn(async move {
                if let Ok(sync_paths) = AppPaths::detect() {
                    if config::read_main_api_key(&sync_paths).is_some() {
                        let claude_tako =
                            adapters::claude::ClaudeAdapter.read_mode(&sync_paths) == AppMode::Tako;
                        let wb_tako =
                            adapters::workbuddy::WorkBuddyAdapter.read_mode(&sync_paths)
                                == AppMode::Tako;
                        if claude_tako
                            && crate::agy_proxy::refresh_claude_catalog_cache(&sync_paths)
                                .await
                                .is_ok()
                        {
                            let _ = adapters::claude::ClaudeAdapter
                                .apply_mode(&sync_paths, AppMode::Tako);
                        }
                        if wb_tako
                            && crate::agy_proxy::refresh_workbuddy_catalog_cache(&sync_paths)
                                .await
                                .is_ok()
                        {
                            let _ = adapters::workbuddy::WorkBuddyAdapter
                                .apply_mode(&sync_paths, AppMode::Tako);
                        }
                    }
                }
            });

            let bg_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                loop {
                    if let Ok(bg_paths) = AppPaths::detect() {
                        if config::read_main_api_key(&bg_paths).is_some() {
                            let status = fetch_account_status(&bg_paths).await;
                            update_account_and_tray(&bg_handle, &bg_paths, status, true);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            });

            // Background self-update watcher so tray badge updates even when window is hidden.
            let upd_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(SELF_UPDATE_BG_INTERVAL_SECS))
                    .await;
                loop {
                    if let Ok(upd_paths) = AppPaths::detect() {
                        if read_switch_state(&upd_paths).auto_check_updates {
                            if let Ok(res) = perform_self_update_check(&upd_handle).await {
                                let _ = upd_handle.emit("self-update-status", &res);
                            }
                            run_background_ui_patch_auto_sync(&upd_handle, true).await;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(
                        SELF_UPDATE_BG_INTERVAL_SECS,
                    ))
                    .await;
                }
            });

            // Startup & idle UI localization auto-sync:
            // 1. After startup, check remote i18n overlay packs and silently upgrade outdated patches.
            // 2. While running, periodically sync any deferred app once the user closes it.
            let i18n_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                run_background_ui_patch_auto_sync(&i18n_handle, true).await;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(
                        UI_PATCH_IDLE_SYNC_INTERVAL_SECS,
                    ))
                    .await;
                    run_background_ui_patch_auto_sync(&i18n_handle, false).await;
                }
            });

            // Startup anonymous launch heartbeat (runs once shortly after boot if telemetry is enabled).
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let _ = tauri::async_runtime::spawn_blocking(|| {
                    if let Ok(hb_paths) = AppPaths::detect() {
                        telemetry::track_launch_heartbeat(&hb_paths);
                    }
                })
                .await;
            });

            if let Some(win) = app.get_webview_window("main") {
                let init_state = read_switch_state(&paths);
                #[cfg(target_os = "windows")]
                {
                    let _ = win.set_decorations(false);
                    let _ = win.set_shadow(true);
                }
                apply_native_window_appearance(&win, init_state.theme_pref, None);

                let win_clone = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            get_running_states,
            login_oauth,
            login_with_key,
            add_saved_key,
            switch_saved_key,
            remove_saved_key,
            rename_saved_key_alias,
            logout_account,
            open_url,
            save_settings,
            sync_window_theme,
            show_window_system_menu,
            set_prefer_chinese_ui,
            patch_app_ui,
            restore_app_ui,
            patch_antigravity_ui,
            restore_antigravity_ui,
            open_macos_app_management_settings,
            run_elevated_ui_patch_worker,
            set_app_mode,
            restart_app,
            close_app,
            launch_app,
            diagnose_app,
            list_client_models,
            list_plaza_models,
            save_client_model_filter,
            check_app_updates,
            install_or_upgrade_app,
            cancel_app_install,
            ensure_local_gateway,
            check_self_update_smart,
            install_self_update,
            run_mcp_live_demo,
            check_node_runtime,
            get_extensions_summary,
            get_mcp_hub_snapshot,
            upsert_mcp_entry,
            toggle_mcp_platform,
            delete_mcp_entry,
            align_mcp_entry,
            align_all_mcp_entries,
            parse_mcp_snippet,
            export_mcp_backup,
            import_mcp_backup,
            get_prompts_hub_snapshot,
            upsert_prompt_preset,
            activate_prompt_preset,
            delete_prompt_preset,
            sync_prompt_to_platforms,
            export_prompts_backup,
            import_prompts_backup,
            extensions::skills::get_skills_hub_snapshot,
            extensions::skills::toggle_skill_platform,
            extensions::skills::align_skill_platforms,
            extensions::skills::uninstall_skill,
            extensions::skills::set_skill_sync_method,
            extensions::skills::set_skill_default_targets,
            extensions::skills::stage_skill_zip_chunk,
            extensions::skills::discard_skill_zip_upload,
            extensions::skills::import_local_skill,
            extensions::skills::scan_adopt_local_skills,
            extensions::skills::upsert_skill_repo,
            extensions::skills::remove_skill_repo,
            extensions::skills::discover_repo_skills,
            extensions::skills::install_discovered_skill,
            extensions::skills::update_installed_skill,
            get_plugins_hub_snapshot,
            toggle_plugin_enabled,
            install_plugin_entry,
            update_plugin_entry,
            uninstall_plugin_entry,
            add_plugin_marketplace,
            remove_plugin_marketplace,
            report_frontend_error,
            submit_user_feedback,
            get_sanitized_diagnostics_preview,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tako Switch");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_self_update_endpoints_prefers_github_when_ahead() {
        assert_eq!(parse_semver_tuple("v2.0.3"), Some((2, 0, 3)));
        assert_eq!(parse_semver_tuple("2.0.2-beta.1"), Some((2, 0, 2)));

        let (src, urls) = choose_self_update_endpoints(Some((2, 0, 2)), Some((2, 0, 3)));
        assert_eq!(src, "github");
        assert_eq!(urls[0], SELF_UPDATE_GITHUB_ENDPOINT);

        let (src_equal, urls_equal) =
            choose_self_update_endpoints(Some((2, 0, 3)), Some((2, 0, 3)));
        assert_eq!(src_equal, "oss");
        assert_eq!(urls_equal[0], SELF_UPDATE_OSS_ENDPOINT);

        let (src_oss_only, urls_oss_only) = choose_self_update_endpoints(Some((2, 0, 3)), None);
        assert_eq!(src_oss_only, "oss");
        assert_eq!(urls_oss_only[0], SELF_UPDATE_OSS_ENDPOINT);

        let (src_gh_only, urls_gh_only) = choose_self_update_endpoints(None, Some((2, 0, 3)));
        assert_eq!(src_gh_only, "github");
        assert_eq!(urls_gh_only[0], SELF_UPDATE_GITHUB_ENDPOINT);
    }

    #[test]
    fn save_settings_syncs_edgeone_mcp_to_tako_clients_only_and_normalizes_blank_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        config::save_switch_auth(&paths, Some("cr_main_key"), Some("1"), Some("oauth")).unwrap();

        // Put Claude & Codex in Tako mode; leave Antigravity in Official mode
        get_adapter(AppId::Claude)
            .apply_mode(&paths, AppMode::Tako)
            .unwrap();
        get_adapter(AppId::Codex)
            .apply_mode(&paths, AppMode::Tako)
            .unwrap();

        let mut next_mcp = read_switch_state(&paths).mcp;
        next_mcp.web_page = true;
        next_mcp.edgeone_api_token = Some("   ".to_string());
        next_mcp.edgeone_project_name = Some("  demo-site  ".to_string());

        let saved = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                mcp: Some(next_mcp),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(saved.mcp.web_page);
        assert_eq!(saved.mcp.edgeone_api_token, None);
        assert_eq!(saved.mcp.edgeone_project_name.as_deref(), Some("demo-site"));

        // Claude & Codex synced automatically (Claude 3p receives tako-web-page via managedMcpServers, not user config)
        assert!(adapters::has_edgeone_mcp_server(
            &paths.claude_1p_dir.join("claude_desktop_config.json")
        ));
        assert!(!adapters::has_edgeone_mcp_server(
            &paths.claude_3p_dir.join("claude_desktop_config.json")
        ));
        let claude_3p_profile = std::fs::read_to_string(
            paths
                .claude_3p_dir
                .join("configLibrary")
                .join("7a405017-c100-4a80-9000-7a4050170001.json"),
        )
        .unwrap();
        assert!(claude_3p_profile.contains(adapters::EDGEONE_MCP_SERVER_KEY));
        assert!(claude_3p_profile.contains("\"EDGEONE_PAGES_PROJECT_NAME\": \"demo-site\""));
        let codex_toml = std::fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(codex_toml.contains(adapters::EDGEONE_MCP_SERVER_KEY));
        assert!(codex_toml.contains("EDGEONE_PAGES_PROJECT_NAME = \"demo-site\""));
        assert!(!codex_toml.contains("EDGEONE_PAGES_API_TOKEN"));

        // Antigravity (Official) was NOT touched
        assert!(!adapters::has_edgeone_mcp_server(
            &paths.agy_gemini_config_dir.join("mcp_config.json")
        ));

        // Update token -> synced automatically
        let mut with_token = saved.mcp.clone();
        with_token.edgeone_api_token = Some("  eo_live_tok  ".to_string());
        let saved_tok = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                mcp: Some(with_token),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            saved_tok.mcp.edgeone_api_token.as_deref(),
            Some("eo_live_tok")
        );
        let codex_toml_tok = std::fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(codex_toml_tok.contains("EDGEONE_PAGES_API_TOKEN = \"eo_live_tok\""));

        // Disable web_page -> cleaned up automatically
        let mut disabled = saved_tok.mcp.clone();
        disabled.web_page = false;
        apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                mcp: Some(disabled),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!adapters::has_edgeone_mcp_server(
            &paths.claude_1p_dir.join("claude_desktop_config.json")
        ));
        assert!(!adapters::has_edgeone_mcp_server(
            &paths.claude_3p_dir.join("claude_desktop_config.json")
        ));
        let codex_toml_off = std::fs::read_to_string(paths.codex_dir.join("config.toml")).unwrap();
        assert!(!codex_toml_off.contains(adapters::EDGEONE_MCP_SERVER_KEY));
    }

    #[test]
    fn lazy_default_tako_switch_respects_explicit_official_opt_out() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        // Without login: must NOT auto-switch
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert!(!rt
            .block_on(ensure_default_tako_mode_before_launch(&paths, AppId::Codex))
            .unwrap());
        assert_eq!(get_adapter(AppId::Codex).read_mode(&paths), AppMode::Official);

        // After login: Codex defaults to Tako on launch
        config::save_switch_auth(&paths, Some("cr_lazy_test"), Some("1"), Some("oauth")).unwrap();
        assert!(rt
            .block_on(ensure_default_tako_mode_before_launch(&paths, AppId::Codex))
            .unwrap());
        assert_eq!(get_adapter(AppId::Codex).read_mode(&paths), AppMode::Tako);

        // If user explicitly chooses Official for Antigravity, launch must NOT override it
        let mut st = read_switch_state(&paths);
        st.set_explicit_official(AppId::Antigravity, true);
        save_switch_state(&paths, &st).unwrap();
        assert!(!rt
            .block_on(ensure_default_tako_mode_before_launch(
                &paths,
                AppId::Antigravity
            ))
            .unwrap());
        assert_eq!(
            get_adapter(AppId::Antigravity).read_mode(&paths),
            AppMode::Official
        );
    }

    #[test]
    fn ui_appearance_settings_patch_persists_without_touching_client_configs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        config::save_switch_auth(&paths, Some("cr_ui_test"), Some("1"), Some("oauth")).unwrap();

        // Put Codex in Tako mode, then mutate config.toml with a custom comment marker
        get_adapter(AppId::Codex)
            .apply_mode(&paths, AppMode::Tako)
            .unwrap();
        let codex_cfg_path = paths.codex_dir.join("config.toml");
        let mut raw_toml = std::fs::read_to_string(&codex_cfg_path).unwrap();
        raw_toml.push_str("\n# untouched-marker\n");
        std::fs::write(&codex_cfg_path, &raw_toml).unwrap();

        let saved = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                ui_density: Some(config::UiDensity::Comfortable),
                theme_pref: Some(config::ThemePref::Dark),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(saved.ui_density, config::UiDensity::Comfortable);
        assert_eq!(saved.theme_pref, config::ThemePref::Dark);

        // Codex config was not rewritten by pure UI appearance patch
        let after_toml = std::fs::read_to_string(&codex_cfg_path).unwrap();
        assert_eq!(after_toml, raw_toml);
    }

    #[test]
    fn hidden_apps_settings_patch_persists_deduped_and_blocks_hiding_all() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let saved = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                hidden_apps: Some(vec![AppId::Antigravity, AppId::Antigravity, AppId::DshTako]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.hidden_apps, vec![AppId::Antigravity, AppId::DshTako]);
        assert_eq!(
            read_switch_state(&paths).hidden_apps,
            vec![AppId::Antigravity, AppId::DshTako]
        );

        let err = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                hidden_apps: Some(AppId::all().to_vec()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("至少需要保留一个"));

        let restored = apply_settings_patch_for_paths(
            &paths,
            SettingsPatch {
                hidden_apps: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(restored.hidden_apps.is_empty());
    }
}
