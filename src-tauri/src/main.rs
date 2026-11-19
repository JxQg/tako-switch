// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("mcp-server") {
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

    tako_switch_lib::run();
}
