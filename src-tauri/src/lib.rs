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
use tokio::sync::Mutex;

const TRAY_ID: &str = "main-tray";

pub struct InstallGuard {
    lock: Mutex<()>,
    cancels: std::sync::Mutex<
        std::collections::HashMap<AppId, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
}

pub struct CachedAccount(std::sync::Mutex<Option<AccountStatus>>);

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
        let snap = snapshot_app(paths, app_id);
        let mark = |m: AppMode| if snap.mode == m { "● " } else { "○ " };
        let off = MenuItem::with_id(
            app,
            format!("mode:{}:official", app_id.as_str()),
            format!("{}官方直连", mark(AppMode::Official)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;
        let tako = MenuItem::with_id(
            app,
            format!("mode:{}:tako", app_id.as_str()),
            format!("{}Tako 全接管 (免官方登录)", mark(AppMode::Tako)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;

        let sub = Submenu::with_items(
            app,
            format!("{} ({})", app_id.display_name(), mode_label(snap.mode)),
            true,
            &[&off, &tako],
        )
        .map_err(|e| format!("创建应用子菜单失败: {e}"))?;
        submenus.push(sub);
    }

    let check_upd = MenuItem::with_id(app, "tray:updates", "检查更新", true, None::<&str>)
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
    items.push(&show);
    items.push(&refresh_bal);
    items.push(&sep1);
    items.push(&submenus[0]);
    items.push(&submenus[1]);
    items.push(&submenus[2]);
    items.push(&sep2);
    items.push(&check_upd);
    items.push(&quit);

    Menu::with_items(app, &items).map_err(|e| format!("组装托盘菜单失败: {e}"))
}

fn mode_label(mode: AppMode) -> &'static str {
    match mode {
        AppMode::Official => "官方",
        AppMode::Tako => "Tako",
    }
}

fn refresh_tray(app: &AppHandle, paths: &AppPaths) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, paths) {
            let _ = tray.set_menu(Some(menu));
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

        let tip = match rem_opt {
            Some(rem) => format!("Tako Switch · 余额 {}", format_usd(rem)),
            None => "Tako Switch".to_string(),
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
    let state = read_switch_state(&paths);
    let apps = AppId::all()
        .iter()
        .map(|&id| snapshot_app(&paths, id))
        .collect();
    let cached_manifest = load_cached_manifest(&paths);

    Ok(DashboardPayload {
        account,
        state,
        apps,
        cached_manifest,
        local_gateway_healthy: agy_proxy::is_proxy_healthy(agy_proxy::LOCAL_GATEWAY_PORT),
        local_gateway_url: agy_proxy::LOCAL_GATEWAY_URL.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
    })
}

#[tauri::command]
async fn login_oauth(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = start_oauth_login(&paths).await?;
    update_account_and_tray(&app, &paths, status.clone(), false);
    Ok(status)
}

#[tauri::command]
async fn login_with_key(app: AppHandle, key: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_login_with_key(&paths, &key).await?;
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
            if !endpoint_or_profile_changed && app_id != AppId::Codex {
                continue;
            }
            let adapter = get_adapter(app_id);
            if adapter.read_mode(&paths) == AppMode::Tako {
                let _ = adapter.apply_mode(&paths, AppMode::Tako);
            }
        }
    }

    refresh_tray(&app, &paths);
    Ok(state)
}

#[tauri::command]
fn get_running_states() -> Result<std::collections::BTreeMap<String, bool>, String> {
    let paths = AppPaths::detect()?;
    let mut map = std::collections::BTreeMap::new();
    for &app_id in AppId::all() {
        map.insert(app_id.as_str().to_string(), get_adapter(app_id).is_running(&paths));
    }
    Ok(map)
}

#[tauri::command]
fn set_app_mode(
    app: AppHandle,
    app_id: AppId,
    mode: AppMode,
    auto_restart: bool,
) -> Result<AppSnapshot, String> {
    let paths = AppPaths::detect()?;
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
                    } else if id == "tray:updates" {
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
            refresh_tray(app.handle(), &paths);

            let any_tako = AppId::all()
                .iter()
                .any(|&id| get_adapter(id).read_mode(&paths) == AppMode::Tako);
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
            set_app_mode,
            restart_app,
            launch_app,
            diagnose_app,
            check_app_updates,
            install_or_upgrade_app,
            cancel_app_install,
            ensure_local_gateway,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tako Switch");
}
