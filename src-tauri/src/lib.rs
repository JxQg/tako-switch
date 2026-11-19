pub mod adapters;
pub mod auth;
pub mod config;
pub mod installer;
pub mod manifest;
pub mod mcp_server;

use adapters::{
    get_adapter, snapshot_app, AppId, AppMode, AppSnapshot, DiagnosticResult,
};
use auth::{
    fetch_account_status, login_with_key as auth_login_with_key, logout as auth_logout,
    open_external_url, start_oauth_login, AccountStatus,
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
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tokio::sync::Mutex;

const TRAY_ID: &str = "main-tray";

pub struct InstallGuard {
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardPayload {
    pub account: AccountStatus,
    pub state: SwitchState,
    pub apps: Vec<AppSnapshot>,
    pub cached_manifest: Option<Manifest>,
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
    pub mcp: Option<McpToolsConfig>,
}

fn build_tray_menu(app: &AppHandle, paths: &AppPaths) -> Result<Menu<tauri::Wry>, String> {
    let show = MenuItem::with_id(app, "tray:show", "打开 Tako Switch", true, None::<&str>)
        .map_err(|e| format!("创建托盘菜单失败: {e}"))?;

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
        let hyb = MenuItem::with_id(
            app,
            format!("mode:{}:hybrid", app_id.as_str()),
            format!("{}聚合 (保留官方模型 + Tako MCP)", mark(AppMode::Hybrid)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;
        let tako = MenuItem::with_id(
            app,
            format!("mode:{}:tako", app_id.as_str()),
            format!("{}全 Tako 网关", mark(AppMode::Tako)),
            true,
            None::<&str>,
        )
        .map_err(|e| format!("创建子菜单失败: {e}"))?;

        let sub = Submenu::with_items(
            app,
            format!("{} ({})", app_id.display_name(), mode_label(snap.mode)),
            true,
            &[&off, &hyb, &tako],
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

    Menu::with_items(
        app,
        &[
            &show,
            &sep1,
            &submenus[0],
            &submenus[1],
            &submenus[2],
            &sep2,
            &check_upd,
            &quit,
        ],
    )
    .map_err(|e| format!("组装托盘菜单失败: {e}"))
}

fn mode_label(mode: AppMode) -> &'static str {
    match mode {
        AppMode::Official => "官方",
        AppMode::Hybrid => "聚合",
        AppMode::Tako => "Tako",
    }
}

fn refresh_tray(app: &AppHandle, paths: &AppPaths) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, paths) {
            let _ = tray.set_menu(Some(menu));
        }
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
        "hybrid" => AppMode::Hybrid,
        "tako" => AppMode::Tako,
        _ => return None,
    };
    Some((app_id, mode))
}

#[tauri::command]
async fn get_dashboard(refresh_account: bool) -> Result<DashboardPayload, String> {
    let paths = AppPaths::detect()?;
    let account = if refresh_account {
        fetch_account_status(&paths).await
    } else {
        let key_opt = config::read_main_api_key(&paths);
        let st = read_switch_state(&paths);
        AccountStatus {
            logged_in: key_opt.is_some(),
            base_url: st.normalized_base_url(),
            masked_key: key_opt.as_deref().map(auth::mask_api_key),
            user_name: None,
            plan_name: None,
            wallet_remain: None,
            wallet_used: None,
            quota: None,
            error: None,
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
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
    })
}

#[tauri::command]
async fn login_oauth(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = start_oauth_login(&paths).await?;
    refresh_tray(&app, &paths);
    Ok(status)
}

#[tauri::command]
async fn login_with_key(app: AppHandle, key: String) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_login_with_key(&paths, &key).await?;
    refresh_tray(&app, &paths);
    Ok(status)
}

#[tauri::command]
async fn logout_account(app: AppHandle) -> Result<AccountStatus, String> {
    let paths = AppPaths::detect()?;
    let status = auth_logout(&paths).await?;
    refresh_tray(&app, &paths);
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
    if let Some(mcp) = input.mcp {
        state.mcp = mcp;
    }

    save_switch_state(&paths, &state)?;

    let endpoint_changed =
        prev_base != state.normalized_base_url() || prev_agy_key != state.agy_api_key;
    if endpoint_changed {
        for &app_id in AppId::all() {
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
async fn install_or_upgrade_app(
    app: AppHandle,
    guard: tauri::State<'_, InstallGuard>,
    app_id: AppId,
) -> Result<InstallOutcome, String> {
    let _lk = guard.lock.lock().await;
    let paths = AppPaths::detect()?;
    let manifest = if let Some(cached) = load_cached_manifest(&paths) {
        cached
    } else {
        fetch_manifest(&paths).await?.1
    };
    let (_, artifact) = resolve_platform_artifact(&manifest, app_id)
        .ok_or_else(|| format!("当前系统架构暂无 {} 的安装包条目", app_id.display_name()))?;
    let outcome = installer::install_or_upgrade_app(Some(&app), &paths, app_id, artifact).await?;
    refresh_tray(&app, &paths);
    Ok(outcome)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(InstallGuard {
            lock: Mutex::new(()),
        })
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let paths = AppPaths::detect().map_err(std::io::Error::other)?;
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            let menu = build_tray_menu(app.handle(), &paths).map_err(std::io::Error::other)?;

            let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "tray:show" {
                        show_main_window(app);
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
            {
                builder = builder.icon_as_template(true);
            }

            builder.build(app)?;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tako Switch");
}
