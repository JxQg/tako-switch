mod backups;
mod commands;
mod config_paths;
mod env_vars;
mod models;
mod platforms;
mod providers;
mod redaction;
mod tools;
mod utils;

use tauri_plugin_deep_link::DeepLinkExt;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let mut handled_deeplink = false;
            for arg in args {
                if commands::handle_deeplink_url(app, &arg) {
                    handled_deeplink = true;
                    break;
                }
            }

            if !handled_deeplink {
                commands::focus_main_window(app);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            #[cfg(all(debug_assertions, windows))]
            {
                if let Err(err) = app.deep_link().register_all() {
                    eprintln!("注册 deep-link 协议失败：{err}");
                }
            }

            app.deep_link().on_open_url({
                let app_handle = app.handle().clone();
                move |event| {
                    for url in event.urls() {
                        commands::handle_deeplink_url(&app_handle, &url.to_string());
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::detect_tools,
            commands::load_current_configs,
            commands::load_latest_apply_result,
            commands::load_provider_catalog,
            commands::migrate_legacy_codex_config,
            commands::preview_changes,
            commands::apply_configs,
            commands::restore_backup,
            commands::open_external,
            providers::tako::tako_login,
            providers::tako::tako_apply_key,
            providers::tako::tako_current_identity,
            providers::tako::tako_logout,
            providers::tako::tako_usage,
            providers::tako::tako_list_models
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tako Switch 时出错");
}
