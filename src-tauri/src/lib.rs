pub mod adapters;
pub mod agy_proxy;
pub mod auth;
pub mod config;
pub mod installer;
pub mod manifest;
pub mod mcp_server;

use adapters::{
    get_adapter, snapshot_app, AppId, AppMode, AppSnapshot, DesktopAppAdapter, DiagnosticResult,
};
use auth::{
    fetch_account_status, format_usd, login_with_key as auth_login_with_key,
    logout as auth_logout, open_external_url, quota_units_to_usd, read_balance_cache,
    start_oauth_login, AccountStatus,
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
    "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/tako-switch/latest.json";
const SELF_UPDATE_GITHUB_ENDPOINT: &str =
    "https://github.com/tako-dev/tako-switch/releases/latest/download/latest.json";
const SELF_UPDATE_BG_INTERVAL_SECS: u64 = 1800;

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
}

#[derive(Debug, Clone, Deserialize)]
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
    pub mcp: Option<McpToolsConfig>,
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
    let has_key = config::read_main_api_key(paths).is_some();
    let acc_opt = if has_key {
        cached_acc.filter(|a| a.logged_in)
    } else {
        None
    };

    let summary_text = if let Some(ref acc) = acc_opt {
        if let Some(rem) = acc.wallet_remain_usd {
            if let Some(used) = acc.wallet_used_usd {
                format!("Tako 余额: {}  (已用 {})", format_usd(rem), format_usd(used))
            } else {
                format!("Tako 余额: {}", format_usd(rem))
            }
        } else {
            "Tako: 已登录 (刷新获取余额)".to_string()
        }
    } else if has_key {
        "Tako: 已登录 (刷新获取余额)".to_string()
    } else {
        "Tako: 未登录 (点击打开登录)".to_string()
    };

    let acct_summary = MenuItem::with_id(app, "tray:account-summary", summary_text, false, None::<&str>)
        .map_err(|e| format!("创建账户摘要项失败: {e}"))?;

    let plan_summary = acc_opt.as_ref().and_then(|acc| {
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

    let sep0 = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let show = MenuItem::with_id(app, "tray:show", "打开 Tako Switch", true, None::<&str>)
        .map_err(|e| format!("创建托盘菜单失败: {e}"))?;
    let refresh_bal = MenuItem::with_id(app, "tray:refresh-balance", "刷新余额", true, None::<&str>)
        .map_err(|e| format!("创建刷新余额菜单失败: {e}"))?;

    let mut submenus = Vec::new();
    for &app_id in AppId::all() {
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
    let account = if refresh_account {
        let status = fetch_account_status(&paths).await;
        update_account_and_tray(&app, &paths, status.clone(), false);
        status
    } else {
        let key_opt = config::read_main_api_key(&paths);
        let st = read_switch_state(&paths);
        let masked = key_opt.as_deref().map(auth::mask_api_key);
        let cached = key_opt.as_ref().and_then(|_| {
            current_cached_account(&app, &paths).filter(|c| c.logged_in && c.masked_key == masked)
        });
        if let Some(c) = cached {
            AccountStatus {
                base_url: st.normalized_base_url(),
                wallet_remain_usd: c.wallet_remain_usd.or_else(|| c.wallet_remain.map(quota_units_to_usd)),
                wallet_used_usd: c.wallet_used_usd.or_else(|| c.wallet_used.map(quota_units_to_usd)),
                ..c
            }
        } else {
            AccountStatus {
                logged_in: key_opt.is_some(),
                base_url: st.normalized_base_url(),
                masked_key: masked,
                user_name: None,
                plan_name: None,
                wallet_remain: None,
                wallet_used: None,
                wallet_remain_usd: None,
                wallet_used_usd: None,
                quota: None,
                error: None,
            }
        }
    };
    let (state, apps, cached_manifest, local_gateway_healthy) =
        tauri::async_runtime::spawn_blocking(move || {
            (
                read_switch_state(&paths),
                AppId::all()
                    .iter()
                    .map(|&id| snapshot_app(&paths, id))
                    .collect(),
                load_cached_manifest(&paths),
                agy_proxy::is_proxy_healthy(agy_proxy::LOCAL_GATEWAY_PORT),
            )
        })
        .await
        .map_err(|err| format!("读取客户端状态失败: {err}"))?;

    Ok(DashboardPayload {
        account,
        state,
        apps,
        cached_manifest,
        local_gateway_healthy,
        local_gateway_url: agy_proxy::LOCAL_GATEWAY_URL.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
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

#[tauri::command]
async fn login_oauth(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = start_oauth_login(&paths).await?;
    sync_after_account_login(&paths);
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
async fn login_with_key(app: AppHandle, key: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_login_with_key(&paths, &key).await?;
    sync_after_account_login(&paths);
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
async fn logout_account(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_logout(&paths).await?;
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    open_external_url(&url)
}

#[tauri::command]
async fn save_settings(app: AppHandle, input: SettingsPatch) -> Result<SwitchState, String> {
    let paths = AppPaths::detect()?;
    let mut state = read_switch_state(&paths);
    let prev_base = state.normalized_base_url();
    let prev_agy_key = state.agy_api_key.clone();
    let prev_codex_preserve = state.codex_preserve_official_auth;
    let prev_name = state.normalized_gateway_name();
    let prev_subtitle = state.normalized_gateway_subtitle();
    let prev_zh = state.prefer_chinese_ui;

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
    if let Some(mcp) = input.mcp {
        state.mcp = mcp;
    }

    save_switch_state(&paths, &state)?;

    let endpoint_or_profile_changed = prev_base != state.normalized_base_url()
        || prev_agy_key != state.agy_api_key
        || prev_name != state.normalized_gateway_name()
        || prev_subtitle != state.normalized_gateway_subtitle()
        || prev_zh != state.prefer_chinese_ui;
    let codex_auth_mode_changed = prev_codex_preserve != state.codex_preserve_official_auth;
    if endpoint_or_profile_changed || codex_auth_mode_changed {
        for &app_id in AppId::all() {
            if !app_id.requires_local_gateway() {
                continue;
            }
            if !endpoint_or_profile_changed && app_id != AppId::Codex {
                continue;
            }
            let adapter = get_adapter(app_id);
            if adapter.read_mode(&paths) == AppMode::Tako {
                let _ = adapter.apply_mode(&paths, AppMode::Tako);
            }
        }
    }
    if prev_zh != state.prefer_chinese_ui {
        sync_standalone_chinese_for_installed(&paths, state.prefer_chinese_ui);
    }

    refresh_tray(&app, &paths);
    Ok(state)
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
        AppId::DshTako => false,
        AppId::Codex => paths.codex_dir.exists(),
        AppId::Claude => {
            paths.claude_1p_dir.exists()
                || paths.claude_3p_dir.exists()
                || paths.home.join(".claude").exists()
        }
        AppId::Antigravity => paths.home.join(".gemini").exists(),
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

#[tauri::command]
async fn set_prefer_chinese_ui(
    app: AppHandle,
    enable: bool,
    patch_antigravity: bool,
    auto_restart: bool,
) -> Result<DashboardPayload, String> {
    let paths_for_sync = AppPaths::detect()?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut state = read_switch_state(&paths_for_sync);
        state.prefer_chinese_ui = enable;
        save_switch_state(&paths_for_sync, &state)?;

        let agy_adapter = get_adapter(AppId::Antigravity);
        let agy_was_running = agy_adapter.is_running(&paths_for_sync);

        let should_patch_agy = patch_antigravity
            && enable
            && matches!(
                adapters::inspect_antigravity_ui_patch(&paths_for_sync).status,
                adapters::UiPatchStatus::Unpatched
                    | adapters::UiPatchStatus::OutdatedAfterUpgrade
                    | adapters::UiPatchStatus::Patched
            );

        if should_patch_agy && auto_restart && agy_was_running {
            let _ = agy_adapter.quit_app(&paths_for_sync);
            std::thread::sleep(std::time::Duration::from_millis(450));
        }

        for &app_id in &[AppId::Codex, AppId::Claude, AppId::Antigravity] {
            let adapter = get_adapter(app_id);
            if adapter.read_mode(&paths_for_sync) == AppMode::Tako
                && adapter.apply_mode(&paths_for_sync, AppMode::Tako).is_err()
            {
                sync_standalone_chinese_for_installed(&paths_for_sync, enable);
            }
        }
        sync_standalone_chinese_for_installed(&paths_for_sync, enable);

        if should_patch_agy {
            adapters::patch_antigravity_ui_for_paths(&paths_for_sync)?;
        }

        if auto_restart {
            for &app_id in &[AppId::Codex, AppId::Claude, AppId::Antigravity] {
                let adapter = get_adapter(app_id);
                if app_id == AppId::Antigravity && should_patch_agy && agy_was_running {
                    let _ = adapter.launch_app(&paths_for_sync);
                    continue;
                }
                if adapter.is_running(&paths_for_sync) {
                    let _ = adapter.quit_app(&paths_for_sync);
                    std::thread::sleep(std::time::Duration::from_millis(450));
                    let _ = adapter.launch_app(&paths_for_sync);
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|err| format!("同步中文设置失败: {err}"))??;

    if let Ok(paths) = AppPaths::detect() {
        refresh_tray(&app, &paths);
    }
    get_dashboard(app, false).await
}

#[tauri::command]
async fn patch_antigravity_ui(app: AppHandle, auto_restart: bool) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let paths_clone = paths.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || -> Result<AppSnapshot, String> {
        let adapter = get_adapter(AppId::Antigravity);
        let was_running = adapter.is_running(&paths_clone);
        if auto_restart && was_running {
            adapter.quit_app(&paths_clone)?;
            std::thread::sleep(std::time::Duration::from_millis(450));
        }
        adapters::patch_antigravity_ui_for_paths(&paths_clone)?;
        if read_switch_state(&paths_clone).prefer_chinese_ui {
            let _ = adapters::antigravity::AntigravityAdapter::sync_standalone_chinese_locale(
                &paths_clone,
                true,
            );
        }
        if auto_restart && was_running {
            adapter.launch_app(&paths_clone)?;
        }
        Ok(snapshot_app(&paths_clone, AppId::Antigravity))
    })
    .await
    .map_err(|err| format!("执行界面汉化失败: {err}"))??;

    refresh_tray(&app, &paths);
    Ok(snapshot)
}

#[tauri::command]
async fn restore_antigravity_ui(
    app: AppHandle,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let paths_clone = paths.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || -> Result<AppSnapshot, String> {
        let adapter = get_adapter(AppId::Antigravity);
        let was_running = adapter.is_running(&paths_clone);
        if auto_restart && was_running {
            adapter.quit_app(&paths_clone)?;
            std::thread::sleep(std::time::Duration::from_millis(450));
        }
        adapters::restore_antigravity_ui_for_paths(&paths_clone)?;
        if auto_restart && was_running {
            adapter.launch_app(&paths_clone)?;
        }
        Ok(snapshot_app(&paths_clone, AppId::Antigravity))
    })
    .await
    .map_err(|err| format!("还原原版界面失败: {err}"))??;

    refresh_tray(&app, &paths);
    Ok(snapshot)
}

#[tauri::command]
async fn list_plaza_models() -> Result<agy_proxy::ClientModelsResponse, String> {
    let paths = AppPaths::detect()?;
    agy_proxy::fetch_plaza_models_for_ui(&paths).await
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

#[tauri::command]
async fn set_app_mode(
    app: AppHandle,
    app_id: AppId,
    mode: AppMode,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    if app_id == AppId::Claude && mode == AppMode::Tako {
        let _ = crate::agy_proxy::refresh_claude_catalog_cache(&paths).await;
    }
    let adapter = get_adapter(app_id);
    adapter.apply_mode(&paths, mode)?;
    if auto_restart && adapter.is_running(&paths) {
        adapter.quit_app(&paths)?;
        std::thread::sleep(std::time::Duration::from_millis(450));
        adapter.launch_app(&paths)?;
    }
    refresh_tray(&app, &paths);
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn restart_app(app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let adapter = get_adapter(app_id);
    if adapter.is_running(&paths) {
        adapter.quit_app(&paths)?;
        std::thread::sleep(std::time::Duration::from_millis(450));
    }
    adapter.launch_app(&paths)?;
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn close_app(app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let adapter = get_adapter(app_id);
    if adapter.is_running(&paths) {
        adapter.quit_app(&paths)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn launch_app(app_id: AppId) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
    let adapter = get_adapter(app_id);
    adapter.launch_app(&paths)?;
    Ok(snapshot_app(&paths, app_id))
}

#[tauri::command]
fn diagnose_app(app_id: AppId) -> Result<DiagnosticResult, String> {
    let paths = AppPaths::detect()?;
    Ok(get_adapter(app_id).diagnose(&paths))
}

#[tauri::command]
async fn list_client_models(app_id: AppId) -> Result<agy_proxy::ClientModelsResponse, String> {
    let paths = AppPaths::detect()?;
    agy_proxy::fetch_client_models_for_ui(&paths, app_id).await
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
    }
    let _ = app.emit("state-changed", ());
    Ok(state)
}

#[tauri::command]
async fn check_app_updates() -> Result<ManifestCheckResult, String> {
    let paths = AppPaths::detect()?;
    let (source_url, manifest) = fetch_manifest(&paths).await?;
    let updates = compute_updates(&paths, &manifest);
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
    let outcome = async {
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
        refresh_tray(&app, &paths);
        Ok(outcome)
    }
    .await;

    if let Ok(mut map) = guard.cancels.lock() {
        map.remove(&app_id);
    }
    outcome
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
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
                    } else if let Some((app_id, mode)) = parse_tray_mode_id(id) {
                        if let Ok(paths) = AppPaths::detect() {
                            if get_adapter(app_id).apply_mode(&paths, mode).is_ok() {
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
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(
                        SELF_UPDATE_BG_INTERVAL_SECS,
                    ))
                    .await;
                }
            });

            if let Some(win) = app.get_webview_window("main") {
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
            logout_account,
            open_url,
            save_settings,
            set_prefer_chinese_ui,
            patch_antigravity_ui,
            restore_antigravity_ui,
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
}
