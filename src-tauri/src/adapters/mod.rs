pub mod antigravity;
pub mod antigravity_i18n;
pub mod asar_patch;
pub mod claude;
pub mod claude_i18n;
pub mod codex;
pub mod dsh_tako;
pub mod i18n_packs;
#[cfg(target_os = "windows")]
pub(crate) mod windows_process;

pub use antigravity_i18n::{
    inspect_antigravity_ui_patch, patch_antigravity_ui_for_paths,
    restore_antigravity_ui_for_paths, UiPatchInfo, UiPatchStatus,
};
pub use asar_patch::{
    classify_io_error_for_os, format_ui_patch_block_error, inspect_electron_asar_integrity_fuse,
    parse_ui_patch_block_error, patch_windows_pe_asar_integrity, UiPatchBlockKind,
    UiPatchPermissionPrompt, UI_PATCH_BLOCK_PREFIX,
};
pub use claude_i18n::{
    inspect_claude_ui_patch, patch_claude_ui_for_paths, restore_claude_ui_for_paths,
    validate_icu_compatibility, CLAUDE_PATCH_GUARD_COMMENT,
};
pub use i18n_packs::{
    effective_ui_patch_version, install_i18n_pack_bytes, load_i18n_overlay_dict,
    merge_json_dict_with_overlay, refresh_remote_i18n_packs,
};

use crate::config::{write_atomic, AppPaths, McpToolsConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppId {
    #[serde(rename = "dsh-tako")]
    DshTako,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "antigravity")]
    Antigravity,
}

impl AppId {
    pub fn all() -> &'static [AppId] {
        &[
            AppId::DshTako,
            AppId::Codex,
            AppId::Claude,
            AppId::Antigravity,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AppId::DshTako => "dsh-tako",
            AppId::Codex => "codex",
            AppId::Claude => "claude",
            AppId::Antigravity => "antigravity",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            AppId::DshTako => "DSH Tako",
            AppId::Codex => "Codex Desktop",
            AppId::Claude => "Claude Desktop",
            AppId::Antigravity => "Google Antigravity",
        }
    }

    pub fn requires_local_gateway(self) -> bool {
        !matches!(self, AppId::DshTako)
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
    pub chinese_configured: bool,
    pub ui_patch: UiPatchInfo,
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
        AppId::DshTako => Box::new(dsh_tako::DshTakoAdapter),
        AppId::Codex => Box::new(codex::CodexAdapter),
        AppId::Claude => Box::new(claude::ClaudeAdapter),
        AppId::Antigravity => Box::new(antigravity::AntigravityAdapter),
    }
}

pub fn snapshot_app(paths: &AppPaths, id: AppId) -> AppSnapshot {
    let adapter = get_adapter(id);
    let mode = adapter.read_mode(paths);
    let chinese_configured = match id {
        AppId::DshTako => true,
        AppId::Codex => codex::CodexAdapter::is_chinese_configured(paths),
        AppId::Claude => claude::ClaudeAdapter::is_chinese_configured(paths),
        AppId::Antigravity => antigravity::AntigravityAdapter::is_chinese_configured(paths),
    };
    let ui_patch = match id {
        AppId::Claude => inspect_claude_ui_patch(paths),
        AppId::Antigravity => inspect_antigravity_ui_patch(paths),
        _ => UiPatchInfo::not_applicable(),
    };
    AppSnapshot {
        id,
        name: adapter.id().display_name().to_string(),
        mode,
        mcp_enabled: adapter.read_mcp_enabled(paths, mode),
        has_backup: crate::config::has_backup(paths, id.as_str()),
        running: adapter.is_running(paths),
        install: adapter.detect_install(paths),
        diagnostic: adapter.diagnose(paths),
        chinese_configured,
        ui_patch,
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

pub const EDGEONE_MCP_SERVER_KEY: &str = "tako-web-page";
pub const EDGEONE_MCP_NPX_ARGS: &[&str] =
    &["-y", "@edgeone/makers-mcp@latest", "--region", "china"];
pub const EDGEONE_MCP_MIN_NODE_MAJOR: u32 = 18;

pub fn build_edgeone_mcp_json_entry(mcp: &McpToolsConfig) -> Value {
    let mut entry = Map::new();
    entry.insert("command".to_string(), Value::String("npx".to_string()));
    entry.insert(
        "args".to_string(),
        Value::Array(
            EDGEONE_MCP_NPX_ARGS
                .iter()
                .map(|arg| Value::String((*arg).to_string()))
                .collect(),
        ),
    );
    let mut env = Map::new();
    if let Some(token) = mcp.normalized_edgeone_api_token() {
        env.insert(
            "EDGEONE_PAGES_API_TOKEN".to_string(),
            Value::String(token.to_string()),
        );
    }
    if let Some(project) = mcp.normalized_edgeone_project_name() {
        env.insert(
            "EDGEONE_PAGES_PROJECT_NAME".to_string(),
            Value::String(project.to_string()),
        );
    }
    if !env.is_empty() {
        entry.insert("env".to_string(), Value::Object(env));
    }
    Value::Object(entry)
}

pub fn merge_edgeone_mcp_server(
    path: &Path,
    mcp: &McpToolsConfig,
    enable: bool,
) -> Result<(), String> {
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
            map.insert(
                EDGEONE_MCP_SERVER_KEY.to_string(),
                build_edgeone_mcp_json_entry(mcp),
            );
        }
    } else if let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) {
        servers.remove(EDGEONE_MCP_SERVER_KEY);
    }

    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(path, &bytes, false)
}

pub fn has_edgeone_mcp_server(path: &Path) -> bool {
    read_json_object_or_empty(path)
        .ok()
        .and_then(|m| {
            m.get("mcpServers")
                .and_then(Value::as_object)
                .map(|s| s.contains_key(EDGEONE_MCP_SERVER_KEY))
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub major: Option<u32>,
    pub meets_min_version: bool,
    pub node_path: Option<String>,
    pub npx_path: Option<String>,
}

impl NodeRuntimeInfo {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            version: None,
            major: None,
            meets_min_version: false,
            node_path: None,
            npx_path: None,
        }
    }
}

pub fn parse_node_version(raw: &str) -> Option<(String, u32)> {
    let token = raw.split_whitespace().next()?.trim();
    let stripped = token.strip_prefix('v').or_else(|| token.strip_prefix('V')).unwrap_or(token);
    if stripped.is_empty() {
        return None;
    }
    let core = stripped.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major_str = parts.next()?;
    let major = major_str.parse::<u32>().ok()?;
    for part in parts {
        if part.is_empty() || part.parse::<u32>().is_err() {
            return None;
        }
    }
    Some((format!("v{stripped}"), major))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && (meta.permissions().mode() & 0o111) != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

fn node_search_dirs() -> Vec<PathBuf> {
    let mut dirs_out = Vec::new();
    if let Some(path_os) = std::env::var_os("PATH") {
        dirs_out.extend(std::env::split_paths(&path_os));
    }

    #[cfg(not(windows))]
    {
        for fixed in ["/opt/homebrew/bin", "/usr/local/bin"] {
            dirs_out.push(PathBuf::from(fixed));
        }
        if let Some(home) = dirs::home_dir() {
            for rel in [
                ".volta/bin",
                ".fnm/current/bin",
                ".asdf/shims",
                ".mise/shims",
                ".nodenv/shims",
            ] {
                dirs_out.push(home.join(rel));
            }
            let nvm_dir = home.join(".nvm").join("versions").join("node");
            if let Ok(entries) = fs::read_dir(nvm_dir) {
                let mut versioned: Vec<((u32, u32, u32), PathBuf)> = entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name();
                        let name_str = name.to_string_lossy();
                        let stripped = name_str
                            .strip_prefix('v')
                            .or_else(|| name_str.strip_prefix('V'))
                            .unwrap_or(&name_str);
                        let core = stripped.split(['-', '+']).next()?;
                        let mut parts = core.split('.');
                        let major = parts.next()?.parse::<u32>().ok()?;
                        let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
                        let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
                        Some(((major, minor, patch), entry.path().join("bin")))
                    })
                    .collect();
                versioned.sort_by_key(|b| std::cmp::Reverse(b.0));
                for (_, bin_dir) in versioned {
                    dirs_out.push(bin_dir);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        dirs_out.push(PathBuf::from(r"C:\Program Files\nodejs"));
        dirs_out.push(PathBuf::from(r"C:\Program Files (x86)\nodejs"));
        if let Some(nvm_symlink) = std::env::var_os("NVM_SYMLINK") {
            dirs_out.push(PathBuf::from(nvm_symlink));
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            dirs_out.push(PathBuf::from(local_app_data).join("Volta").join("bin"));
        }
    }

    dirs_out
}

fn find_in_dirs(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn detect_node_runtime() -> NodeRuntimeInfo {
    #[cfg(windows)]
    let node_names: &[&str] = &["node.exe", "node.cmd"];
    #[cfg(not(windows))]
    let node_names: &[&str] = &["node"];

    #[cfg(windows)]
    let npx_names: &[&str] = &["npx.cmd", "npx.exe"];
    #[cfg(not(windows))]
    let npx_names: &[&str] = &["npx"];

    let search_dirs = node_search_dirs();
    let node_path = find_in_dirs(&search_dirs, node_names);
    let npx_path = node_path
        .as_deref()
        .and_then(Path::parent)
        .and_then(|parent| find_in_dirs(&[parent.to_path_buf()], npx_names))
        .or_else(|| find_in_dirs(&search_dirs, npx_names));

    let mut cmd = match &node_path {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("node"),
    };
    cmd.arg("-v");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let parsed = cmd
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| parse_node_version(&String::from_utf8_lossy(&out.stdout)));

    match parsed {
        Some((version, major)) => NodeRuntimeInfo {
            available: true,
            version: Some(version),
            major: Some(major),
            meets_min_version: major >= EDGEONE_MCP_MIN_NODE_MAJOR,
            node_path: node_path.map(|p| p.to_string_lossy().to_string()),
            npx_path: npx_path.map(|p| p.to_string_lossy().to_string()),
        },
        None => NodeRuntimeInfo {
            available: false,
            version: None,
            major: None,
            meets_min_version: false,
            node_path: node_path.map(|p| p.to_string_lossy().to_string()),
            npx_path: npx_path.map(|p| p.to_string_lossy().to_string()),
        },
    }
}

pub const TAKO_CHINESE_BLOCK_START: &str = "<!-- tako-switch:chinese-ui:start -->";
pub const TAKO_CHINESE_BLOCK_END: &str = "<!-- tako-switch:chinese-ui:end -->";

pub fn has_managed_chinese_block(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Some(start_idx) = raw.find(TAKO_CHINESE_BLOCK_START) else {
        return false;
    };
    let after = start_idx + TAKO_CHINESE_BLOCK_START.len();
    raw[after..].contains(TAKO_CHINESE_BLOCK_END)
}

fn strip_managed_chinese_blocks(raw: &str) -> (String, bool) {
    let mut current = raw.to_string();
    let mut found = false;

    while let Some(start_idx) = current.find(TAKO_CHINESE_BLOCK_START) {
        let search_from = start_idx + TAKO_CHINESE_BLOCK_START.len();
        let Some(end_rel) = current[search_from..].find(TAKO_CHINESE_BLOCK_END) else {
            break;
        };
        found = true;
        let end_idx = search_from + end_rel + TAKO_CHINESE_BLOCK_END.len();
        let before_trimmed = current[..start_idx].trim_end_matches(['\r', '\n']);
        let after_trimmed = current[end_idx..].trim_start_matches(['\r', '\n']);

        current = if before_trimmed.is_empty() && after_trimmed.is_empty() {
            String::new()
        } else if before_trimmed.is_empty() {
            after_trimmed.to_string()
        } else if after_trimmed.is_empty() {
            format!("{before_trimmed}\n")
        } else {
            format!("{before_trimmed}\n\n{after_trimmed}")
        };
    }

    (current, found)
}

pub fn sync_managed_chinese_markdown(
    paths: &AppPaths,
    app_key: &str,
    backup_name: &str,
    target_path: &Path,
    enable: bool,
    body: &str,
) -> Result<(), String> {
    if !enable && !target_path.exists() {
        return Ok(());
    }

    crate::config::backup_file_once(paths, app_key, backup_name, target_path)?;

    let existing = if target_path.exists() {
        fs::read_to_string(target_path)
            .map_err(|e| format!("读取 {} 失败: {e}", target_path.display()))?
    } else {
        String::new()
    };

    if enable {
        let block = format!(
            "{TAKO_CHINESE_BLOCK_START}\n{}\n{TAKO_CHINESE_BLOCK_END}",
            body.trim()
        );
        let next = if let Some(start_idx) = existing.find(TAKO_CHINESE_BLOCK_START) {
            let search_from = start_idx + TAKO_CHINESE_BLOCK_START.len();
            if let Some(end_rel) = existing[search_from..].find(TAKO_CHINESE_BLOCK_END) {
                let end_idx = search_from + end_rel + TAKO_CHINESE_BLOCK_END.len();
                let mut replaced = String::with_capacity(existing.len() + block.len());
                replaced.push_str(&existing[..start_idx]);
                replaced.push_str(&block);
                replaced.push_str(&existing[end_idx..]);
                if replaced.ends_with(TAKO_CHINESE_BLOCK_END) {
                    replaced.push('\n');
                }
                replaced
            } else if existing.trim().is_empty() {
                format!("{block}\n")
            } else if existing.ends_with("\n\n") {
                format!("{existing}{block}\n")
            } else if existing.ends_with('\n') {
                format!("{existing}\n{block}\n")
            } else {
                format!("{existing}\n\n{block}\n")
            }
        } else if existing.is_empty() {
            format!("{block}\n")
        } else if existing.ends_with("\n\n") {
            format!("{existing}{block}\n")
        } else if existing.ends_with('\n') {
            format!("{existing}\n{block}\n")
        } else {
            format!("{existing}\n\n{block}\n")
        };
        write_atomic(target_path, next.as_bytes(), false)
    } else {
        let (stripped, found) = strip_managed_chinese_blocks(&existing);
        if !found {
            return Ok(());
        }
        let absent_marker = paths
            .backups_dir(app_key)
            .join(format!("{backup_name}.absent"));
        if stripped.trim().is_empty() && absent_marker.exists() {
            if target_path.exists() {
                fs::remove_file(target_path)
                    .map_err(|e| format!("移除临时 {} 失败: {e}", target_path.display()))?;
            }
            Ok(())
        } else {
            write_atomic(target_path, stripped.as_bytes(), false)
        }
    }
}

#[cfg(target_os = "macos")]
pub fn sync_macos_apple_languages(paths: &AppPaths, bundle_id: &str, enable: bool) {
    if paths.is_test {
        return;
    }
    if enable {
        let _ = std::process::Command::new("/usr/bin/defaults")
            .args(["write", bundle_id, "AppleLanguages", "-array", "zh-Hans"])
            .output();
    } else {
        let _ = std::process::Command::new("/usr/bin/defaults")
            .args(["delete", bundle_id, "AppleLanguages"])
            .output();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sync_managed_chinese_markdown_roundtrip_and_middle_block() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let md_path = tmp.path().join("AGENTS.md");

        // Disabling on absent file must not create .absent or target file
        sync_managed_chinese_markdown(&paths, "codex", "AGENTS.md", &md_path, false, "").unwrap();
        assert!(!md_path.exists());
        assert!(!paths.backups_dir("codex").join("AGENTS.md.absent").exists());

        // Middle block replacement & clean stripping
        fs::write(
            &md_path,
            "# Top Rule\n\n<!-- tako-switch:chinese-ui:start -->\nold\n<!-- tako-switch:chinese-ui:end -->\n\n# Bottom Rule\n",
        )
        .unwrap();
        sync_managed_chinese_markdown(&paths, "codex", "AGENTS.md", &md_path, true, "new rule").unwrap();
        let updated = fs::read_to_string(&md_path).unwrap();
        assert_eq!(
            updated,
            "# Top Rule\n\n<!-- tako-switch:chinese-ui:start -->\nnew rule\n<!-- tako-switch:chinese-ui:end -->\n\n# Bottom Rule\n"
        );

        sync_managed_chinese_markdown(&paths, "codex", "AGENTS.md", &md_path, false, "").unwrap();
        let stripped = fs::read_to_string(&md_path).unwrap();
        assert_eq!(stripped, "# Top Rule\n\n# Bottom Rule\n");
    }

    #[test]
    fn edgeone_mcp_entry_omits_env_when_blank_and_includes_non_empty_fields() {
        let mut mcp = McpToolsConfig::default();
        let entry_empty = build_edgeone_mcp_json_entry(&mcp);
        assert_eq!(entry_empty["command"], "npx");
        assert_eq!(
            entry_empty["args"],
            serde_json::json!(["-y", "@edgeone/makers-mcp@latest", "--region", "china"])
        );
        assert!(entry_empty.get("env").is_none());

        mcp.edgeone_api_token = Some("   ".to_string());
        mcp.edgeone_project_name = Some("\n\t ".to_string());
        assert!(build_edgeone_mcp_json_entry(&mcp).get("env").is_none());

        mcp.edgeone_api_token = Some("  eo_token_abc  ".to_string());
        let entry_token_only = build_edgeone_mcp_json_entry(&mcp);
        let env_obj = entry_token_only["env"].as_object().unwrap();
        assert_eq!(env_obj.len(), 1);
        assert_eq!(env_obj["EDGEONE_PAGES_API_TOKEN"], "eo_token_abc");
        assert!(!env_obj.contains_key("EDGEONE_PAGES_PROJECT_NAME"));

        mcp.edgeone_project_name = Some(" landing-prod ".to_string());
        let entry_both = build_edgeone_mcp_json_entry(&mcp);
        let env_both = entry_both["env"].as_object().unwrap();
        assert_eq!(env_both.len(), 2);
        assert_eq!(env_both["EDGEONE_PAGES_API_TOKEN"], "eo_token_abc");
        assert_eq!(env_both["EDGEONE_PAGES_PROJECT_NAME"], "landing-prod");
    }

    #[test]
    fn merge_edgeone_mcp_server_preserves_other_servers_and_skips_absent_on_disable() {
        let tmp = TempDir::new().unwrap();
        let mcp_path = tmp.path().join("mcp_config.json");
        let mcp = McpToolsConfig {
            web_page: true,
            edgeone_api_token: Some("eo_tok".to_string()),
            ..McpToolsConfig::default()
        };

        // Disabling when absent must not create the file
        merge_edgeone_mcp_server(&mcp_path, &mcp, false).unwrap();
        assert!(!mcp_path.exists());
        assert!(!has_edgeone_mcp_server(&mcp_path));

        // Preserve existing servers (including tako and user servers)
        fs::write(
            &mcp_path,
            r#"{"mcpServers":{"tako":{"command":"tako-switch","args":["mcp-server"]},"custom":{"command":"my-mcp"}}}"#,
        )
        .unwrap();
        merge_edgeone_mcp_server(&mcp_path, &mcp, true).unwrap();
        assert!(has_edgeone_mcp_server(&mcp_path));
        assert!(has_tako_mcp_server(&mcp_path));
        let enabled_root = read_json_object_or_empty(&mcp_path).unwrap();
        let servers = enabled_root["mcpServers"].as_object().unwrap();
        assert!(servers.contains_key("tako"));
        assert!(servers.contains_key("custom"));
        assert_eq!(servers[EDGEONE_MCP_SERVER_KEY]["command"], "npx");
        assert_eq!(
            servers[EDGEONE_MCP_SERVER_KEY]["env"]["EDGEONE_PAGES_API_TOKEN"],
            "eo_tok"
        );

        // Disabling removes only tako-web-page
        merge_edgeone_mcp_server(&mcp_path, &mcp, false).unwrap();
        assert!(!has_edgeone_mcp_server(&mcp_path));
        assert!(has_tako_mcp_server(&mcp_path));
        let disabled_root = read_json_object_or_empty(&mcp_path).unwrap();
        let servers_after = disabled_root["mcpServers"].as_object().unwrap();
        assert!(servers_after.contains_key("tako"));
        assert!(servers_after.contains_key("custom"));
        assert!(!servers_after.contains_key(EDGEONE_MCP_SERVER_KEY));
    }

    #[test]
    fn parse_node_version_handles_v_prefix_prerelease_and_min_version_boundary() {
        assert_eq!(
            parse_node_version("v22.14.0\n"),
            Some(("v22.14.0".to_string(), 22))
        );
        assert_eq!(
            parse_node_version("v18.0.0"),
            Some(("v18.0.0".to_string(), 18))
        );
        assert_eq!(
            parse_node_version("v16.20.2"),
            Some(("v16.20.2".to_string(), 16))
        );
        assert_eq!(
            parse_node_version("20.11.1"),
            Some(("v20.11.1".to_string(), 20))
        );
        assert_eq!(
            parse_node_version("v23.1.0-nightly.20241101"),
            Some(("v23.1.0-nightly.20241101".to_string(), 23))
        );
        assert!(parse_node_version("v18.0.0").unwrap().1 >= EDGEONE_MCP_MIN_NODE_MAJOR);
        assert!(parse_node_version("v17.9.1").unwrap().1 < EDGEONE_MCP_MIN_NODE_MAJOR);
        assert_eq!(parse_node_version(""), None);
        assert_eq!(parse_node_version("not-a-version"), None);
        assert_eq!(parse_node_version("v18.abc"), None);
    }
}
