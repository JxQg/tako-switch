// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{IsTerminal, Read};

fn main() {
    let subcmd = std::env::args().nth(1);
    if subcmd.as_deref() == Some("mcp-server") {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime for Tako MCP server");
        if let Err(err) = rt.block_on(tako_switch_lib::mcp_server::run_stdio_server()) {
            eprintln!("tako-switch mcp-server error: {err}");
            std::process::exit(1);
        }
        return;
    }

    if subcmd.as_deref() == Some("status-line") {
        let Ok(paths) = tako_switch_lib::config::AppPaths::detect() else {
            println!("Tako · 无法定位配置目录");
            return;
        };
        let mut stdin_buf = None;
        if !std::io::stdin().is_terminal() {
            let mut buf = String::new();
            let mut limited = std::io::stdin().take(16 * 1024);
            if limited.read_to_string(&mut buf).is_ok() && !buf.trim().is_empty() {
                stdin_buf = Some(buf);
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime for status-line");
        let line = rt.block_on(tako_switch_lib::auth::render_status_line(
            &paths,
            stdin_buf.as_deref(),
        ));
        println!("{line}");
        return;
    }

    if matches!(subcmd.as_deref(), Some("gateway-proxy") | Some("agy-proxy")) {
        let port = std::env::args()
            .skip(2)
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|w| {
                if w[0] == "--port" {
                    w[1].parse::<u16>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(tako_switch_lib::agy_proxy::LOCAL_GATEWAY_PORT);
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("failed to build Tokio runtime for Tako gateway-proxy");
        if let Err(err) = rt.block_on(tako_switch_lib::agy_proxy::run_proxy_server(port)) {
            eprintln!("tako-switch gateway-proxy error: {err}");
            std::process::exit(1);
        }
        return;
    }

    if matches!(
        subcmd.as_deref(),
        Some("ui-patch-worker") | Some("--ui-patch-worker")
    ) {
        std::env::set_var("TAKO_SWITCH_ELEVATED", "1");
        let args: Vec<String> = std::env::args().skip(2).collect();
        let mut app_arg: Option<String> = None;
        let mut action_arg: Option<String> = None;
        let mut idx = 0usize;
        while idx < args.len() {
            match args[idx].as_str() {
                "--app" => {
                    if let Some(val) = args.get(idx + 1) {
                        app_arg = Some(val.clone());
                        idx += 2;
                        continue;
                    }
                }
                "--action" => {
                    if let Some(val) = args.get(idx + 1) {
                        action_arg = Some(val.clone());
                        idx += 2;
                        continue;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        let Ok(paths) = tako_switch_lib::config::AppPaths::detect() else {
            eprintln!("tako-switch ui-patch-worker error: failed to detect AppPaths");
            std::process::exit(1);
        };
        let res = match (app_arg.as_deref(), action_arg.as_deref()) {
            (Some("claude"), Some("patch")) => {
                tako_switch_lib::adapters::patch_claude_ui_for_paths(&paths).map(|_| ())
            }
            (Some("claude"), Some("restore")) => {
                tako_switch_lib::adapters::restore_claude_ui_for_paths(&paths).map(|_| ())
            }
            (Some("antigravity"), Some("patch")) => {
                tako_switch_lib::adapters::patch_antigravity_ui_for_paths(&paths).map(|_| ())
            }
            (Some("antigravity"), Some("restore")) => {
                tako_switch_lib::adapters::restore_antigravity_ui_for_paths(&paths).map(|_| ())
            }
            _ => Err("invalid arguments; expected --app <claude|antigravity> --action <patch|restore>".to_string()),
        };
        if let Err(err) = res {
            eprintln!("tako-switch ui-patch-worker error: {err}");
            std::process::exit(1);
        }
        return;
    }

    tako_switch_lib::run();
}
