pub mod antigravity;
pub mod claude;
pub mod codex;

use crate::config::{write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppId {
    Codex,
    Claude,
    Antigravity,
}

impl AppId {
    pub fn all() -> &'static [AppId] {
        &[AppId::Codex, AppId::Claude, AppId::Antigravity]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AppId::Codex => "codex",
            AppId::Claude => "claude",
            AppId::Antigravity => "antigravity",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            AppId::Codex => "Codex Desktop",
            AppId::Claude => "Claude Desktop",
            AppId::Antigravity => "Google Antigravity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    Official,
    Tako,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallInfo {
    pub installed: bool,
    pub version: Option<String>,
    pub install_path: Option<String>,
    pub install_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticResult {
    pub ok: bool,
    pub issues: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub id: AppId,
    pub name: String,
    pub mode: AppMode,
    pub mcp_enabled: bool,
    pub has_backup: bool,
    pub running: bool,
    pub install: InstallInfo,
    pub diagnostic: DiagnosticResult,
}

pub trait DesktopAppAdapter {
    fn id(&self) -> AppId;
    fn detect_install(&self, paths: &AppPaths) -> InstallInfo;
    fn is_running(&self, paths: &AppPaths) -> bool;
    fn quit_app(&self, paths: &AppPaths) -> Result<(), String>;
    fn launch_app(&self, paths: &AppPaths) -> Result<(), String>;
    fn read_mode(&self, paths: &AppPaths) -> AppMode;
    fn read_mcp_enabled(&self, paths: &AppPaths, mode: AppMode) -> bool;
    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String>;
    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult;
}

pub fn get_adapter(id: AppId) -> Box<dyn DesktopAppAdapter> {
    match id {
        AppId::Codex => Box::new(codex::CodexAdapter),
        AppId::Claude => Box::new(claude::ClaudeAdapter),
        AppId::Antigravity => Box::new(antigravity::AntigravityAdapter),
    }
}

pub fn snapshot_app(paths: &AppPaths, id: AppId) -> AppSnapshot {
    let adapter = get_adapter(id);
    let mode = adapter.read_mode(paths);
    AppSnapshot {
        id,
        name: adapter.id().display_name().to_string(),
        mode,
        mcp_enabled: adapter.read_mcp_enabled(paths, mode),
        has_backup: crate::config::has_backup(paths, id.as_str()),
        running: adapter.is_running(paths),
        install: adapter.detect_install(paths),
        diagnostic: adapter.diagnose(paths),
    }
}

pub const TAKO_MCP_TOOL_NAMES: &[&str] = &[
    "tako_generate_image",
    "tako_generate_video",
    "tako_generate_audio",
    "tako_web_search",
    "tako_consult",
    "tako_account_status",
];

pub fn build_stdio_mcp_json_entry(paths: &AppPaths) -> Value {
    let exe = paths.mcp_server_exe().to_string_lossy().to_string();
    serde_json::json!({
        "command": exe,
        "args": ["mcp-server"]
    })
}

pub fn read_json_object_or_empty(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(Map::new());
    }
    let val: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 {} 失败: {e}", path.display()))?;
    val.as_object()
        .cloned()
        .ok_or_else(|| format!("{} 必须为 JSON object", path.display()))
}

pub fn merge_tako_mcp_server(path: &Path, paths: &AppPaths, enable: bool) -> Result<(), String> {
    let mut root = if path.exists() {
        read_json_object_or_empty(path)?
    } else if !enable {
        return Ok(());
    } else {
        Map::new()
    };

    if enable {
        let servers = root
            .entry("mcpServers".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !servers.is_object() {
            *servers = Value::Object(Map::new());
        }
        if let Some(map) = servers.as_object_mut() {
            map.insert("tako".to_string(), build_stdio_mcp_json_entry(paths));
        }
    } else if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove("tako");
    }

    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(path, &bytes, false)
}

pub fn has_tako_mcp_server(path: &Path) -> bool {
    read_json_object_or_empty(path)
        .ok()
        .and_then(|m| {
            m.get("mcpServers")
                .and_then(Value::as_object)
                .map(|s| s.contains_key("tako"))
        })
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn read_macos_app_bundle_version(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents").join("Info.plist");
    if !plist.exists() {
        return None;
    }
    let out = std::process::Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(&plist)
        .arg("CFBundleShortVersionString")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!ver.is_empty()).then_some(ver)
}

#[cfg(target_os = "macos")]
pub fn read_macos_bundle_id(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents").join("Info.plist");
    if !plist.exists() {
        return None;
    }
    let out = std::process::Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(&plist)
        .arg("CFBundleIdentifier")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!id.is_empty()).then_some(id)
}

#[cfg(target_os = "macos")]
pub fn is_macos_bundle_id_running(bundle_id: &str) -> bool {
    if let Ok(out) = std::process::Command::new("/usr/bin/lsappinfo")
        .args(["find", &format!("bundleid={bundle_id}")])
        .output()
    {
        return out.status.success() && String::from_utf8_lossy(&out.stdout).contains("ASN:");
    }
    let script = format!("application id \"{bundle_id}\" is running");
    std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn quit_macos_bundle_id(bundle_id: &str) -> Result<(), String> {
    let script = format!("tell application id \"{bundle_id}\" to quit");
    let out = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("退出应用失败: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "退出应用失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    for _ in 0..20 {
        if !is_macos_bundle_id_running(bundle_id) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn launch_macos_app(app_path: Option<&Path>, fallback_bundle_id: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("/usr/bin/open");
    if let Some(path) = app_path {
        cmd.arg(path);
    } else {
        cmd.arg("-b").arg(fallback_bundle_id);
    }
    let out = cmd.output().map_err(|e| format!("拉起应用失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "拉起应用失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

pub fn find_macos_app(paths: &AppPaths, candidates: &[&str], expected_bundle_id: &str) -> Option<PathBuf> {
    let _ = (paths, candidates, expected_bundle_id);
    #[cfg(target_os = "macos")]
    {
        let roots = [
            PathBuf::from("/Applications"),
            paths.home.join("Applications"),
        ];
        for root in roots {
            for name in candidates {
                let p = root.join(name);
                if p.exists() {
                    if let Some(bid) = read_macos_bundle_id(&p) {
                        if bid == expected_bundle_id {
                            return Some(p);
                        }
                    } else {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
