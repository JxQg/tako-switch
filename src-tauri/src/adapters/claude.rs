use crate::adapters::{
    has_tako_mcp_server, merge_tako_mcp_server, read_json_object_or_empty, AppId, AppMode,
    DesktopAppAdapter, DiagnosticResult, InstallInfo,
};
use crate::config::{backup_file_once, read_backup, read_main_api_key, read_switch_state, write_atomic, AppPaths};
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version,
};

pub const TAKO_CLAUDE_3P_UUID: &str = "7a405017-c100-4a80-9000-7a4050170001";
#[cfg(target_os = "macos")]
const CLAUDE_MACOS_BUNDLE_ID: &str = "com.anthropic.claudefordesktop";

pub struct ClaudeAdapter;

impl ClaudeAdapter {
    fn mcp_1p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_1p_dir.join("claude_desktop_config.json")
    }

    fn mcp_3p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_3p_dir.join("claude_desktop_config.json")
    }

    fn meta_3p_path(paths: &AppPaths) -> PathBuf {
        paths.claude_3p_dir.join("configLibrary").join("_meta.json")
    }

    fn profile_3p_path(paths: &AppPaths) -> PathBuf {
        paths
            .claude_3p_dir
            .join("configLibrary")
            .join(format!("{TAKO_CLAUDE_3P_UUID}.json"))
    }

    fn deactivate_3p(paths: &AppPaths) -> Result<(), String> {
        let meta_path = Self::meta_3p_path(paths);
        if meta_path.exists() {
            let mut meta = read_json_object_or_empty(&meta_path)?;
            if meta.get("appliedId").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID) {
                let restored_applied = read_backup(paths, "claude", "3p_meta.json")
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|v| {
                        v.get("appliedId")
                            .and_then(Value::as_str)
                            .filter(|id| *id != TAKO_CLAUDE_3P_UUID)
                            .map(str::to_string)
                    });
                if let Some(prev) = restored_applied {
                    meta.insert("appliedId".to_string(), Value::String(prev));
                } else {
                    meta.remove("appliedId");
                }
                let bytes = serde_json::to_vec_pretty(&Value::Object(meta))
                    .map_err(|e| format!("序列化 Claude 3p _meta.json 失败: {e}"))?;
                write_atomic(&meta_path, &bytes, false)?;
            }
        }
        let prof_path = Self::profile_3p_path(paths);
        if prof_path.exists() {
            let _ = fs::remove_file(&prof_path);
        }
        Ok(())
    }
}

impl DesktopAppAdapter for ClaudeAdapter {
    fn id(&self) -> AppId {
        AppId::Claude
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) = find_macos_app(paths, &["Claude.app"], CLAUDE_MACOS_BUNDLE_ID) {
                let version = read_macos_app_bundle_version(&app_path);
                return InstallInfo {
                    installed: true,
                    version,
                    install_path: Some(app_path.to_string_lossy().to_string()),
                    install_kind: Some("app_bundle".to_string()),
                };
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Some((ver, path, kind)) = detect_windows_claude(paths) {
                return InstallInfo {
                    installed: true,
                    version: ver,
                    install_path: Some(path),
                    install_kind: Some(kind),
                };
            }
        }
        InstallInfo {
            installed: false,
            version: None,
            install_path: None,
            install_kind: None,
        }
    }

    fn is_running(&self, _paths: &AppPaths) -> bool {
        #[cfg(target_os = "macos")]
        {
            is_macos_bundle_id_running(CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            is_windows_claude_running()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            quit_macos_bundle_id(CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_claude()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path = find_macos_app(paths, &["Claude.app"], CLAUDE_MACOS_BUNDLE_ID);
            launch_macos_app(app_path.as_deref(), CLAUDE_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_claude(paths)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 Claude Desktop".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        let meta_path = Self::meta_3p_path(paths);
        if let Ok(meta) = read_json_object_or_empty(&meta_path) {
            if meta.get("appliedId").and_then(Value::as_str) == Some(TAKO_CLAUDE_3P_UUID)
                && Self::profile_3p_path(paths).exists()
            {
                return AppMode::Tako;
            }
        }
        if has_tako_mcp_server(&Self::mcp_1p_path(paths)) {
            AppMode::Hybrid
        } else {
            AppMode::Official
        }
    }

    fn read_mcp_enabled(&self, paths: &AppPaths, mode: AppMode) -> bool {
        match mode {
            AppMode::Tako => has_tako_mcp_server(&Self::mcp_3p_path(paths)),
            AppMode::Hybrid | AppMode::Official => has_tako_mcp_server(&Self::mcp_1p_path(paths)),
        }
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let mcp_1p = Self::mcp_1p_path(paths);
        let mcp_3p = Self::mcp_3p_path(paths);
        let meta_3p = Self::meta_3p_path(paths);

        backup_file_once(paths, "claude", "1p_claude_desktop_config.json", &mcp_1p)?;
        backup_file_once(paths, "claude", "3p_claude_desktop_config.json", &mcp_3p)?;
        backup_file_once(paths, "claude", "3p_meta.json", &meta_3p)?;

        match mode {
            AppMode::Official => {
                Self::deactivate_3p(paths)?;
                merge_tako_mcp_server(&mcp_1p, paths, false)?;
                merge_tako_mcp_server(&mcp_3p, paths, false)?;
                Ok(())
            }
            AppMode::Hybrid => {
                Self::deactivate_3p(paths)?;
                merge_tako_mcp_server(&mcp_3p, paths, false)?;
                merge_tako_mcp_server(&mcp_1p, paths, true)?;
                Ok(())
            }
            AppMode::Tako => {
                let api_key = read_main_api_key(paths)
                    .ok_or_else(|| "请先登录 Tako 账号或填入 Key，再切换到全 Tako 模式".to_string())?;
                let base_url = read_switch_state(paths).normalized_base_url();

                let profile = serde_json::json!({
                    "id": TAKO_CLAUDE_3P_UUID,
                    "name": "Tako",
                    "description": "Tako Enterprise Gateway",
                    "inferenceProvider": "gateway",
                    "inferenceGatewayBaseUrl": base_url,
                    "inferenceGatewayAuthScheme": "bearer",
                    "inferenceGatewayApiKey": api_key,
                    "defaultModel": "claude-sonnet-4-6"
                });
                let prof_bytes = serde_json::to_vec_pretty(&profile)
                    .map_err(|e| format!("序列化 Claude 3p profile 失败: {e}"))?;
                write_atomic(&Self::profile_3p_path(paths), &prof_bytes, true)?;

                let mut meta = if meta_3p.exists() {
                    read_json_object_or_empty(&meta_3p)?
                } else {
                    Map::new()
                };
                meta.insert(
                    "appliedId".to_string(),
                    Value::String(TAKO_CLAUDE_3P_UUID.to_string()),
                );
                let meta_bytes = serde_json::to_vec_pretty(&Value::Object(meta))
                    .map_err(|e| format!("序列化 Claude 3p _meta.json 失败: {e}"))?;
                write_atomic(&meta_3p, &meta_bytes, false)?;

                merge_tako_mcp_server(&mcp_1p, paths, false)?;
                merge_tako_mcp_server(&mcp_3p, paths, true)?;
                Ok(())
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let mut notes = vec![
            format!("1p MCP 配置: {}", Self::mcp_1p_path(paths).display()),
            format!("3p 网关目录: {}", paths.claude_3p_dir.display()),
        ];
        let mode = self.read_mode(paths);
        if mode == AppMode::Tako {
            match read_json_object_or_empty(&Self::profile_3p_path(paths)) {
                Err(e) => issues.push(e),
                Ok(prof) => {
                    let url = prof
                        .get("inferenceGatewayBaseUrl")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if url.is_empty() {
                        issues.push("inferenceGatewayBaseUrl 为空".to_string());
                    } else if url.trim_end_matches('/').ends_with("/v1") {
                        issues.push(format!("Claude Desktop 网关 URL 不能带 /v1 后缀，当前为: {url}"));
                    }
                    let scheme = prof
                        .get("inferenceGatewayAuthScheme")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if scheme != "bearer" {
                        issues.push(format!("inferenceGatewayAuthScheme 应为 bearer，当前为: {scheme}"));
                    }
                    let key = prof
                        .get("inferenceGatewayApiKey")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if key.is_empty() {
                        issues.push("inferenceGatewayApiKey 为空".to_string());
                    }
                }
            }
            if !has_tako_mcp_server(&Self::mcp_3p_path(paths)) {
                notes.push("提示: Claude-3p 尚未注入 tako MCP".to_string());
            }
        }

        DiagnosticResult {
            ok: issues.is_empty(),
            issues,
            notes,
        }
    }
}

#[cfg(target_os = "windows")]
fn ps_hidden(script: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn detect_windows_claude(paths: &AppPaths) -> Option<(Option<String>, String, String)> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
    let exe = local_app_data.join("AnthropicClaude").join("Claude.exe");
    if exe.exists() {
        let script = format!(
            "(Get-Item '{}').VersionInfo.ProductVersion",
            exe.to_string_lossy().replace('\'', "''")
        );
        let ver = ps_hidden(&script).filter(|s| !s.is_empty());
        return Some((ver, exe.to_string_lossy().to_string(), "exe".to_string()));
    }
    let msix = ps_hidden(
        "$p = Get-AppxPackage | Where-Object { $_.Name -eq 'Claude' -or $_.Name -eq 'AnthropicPBC.Claude' } | Select-Object -First 1; if ($p) { Write-Output ($p.Version + '|' + $p.InstallLocation) }",
    )?;
    if msix.is_empty() {
        return None;
    }
    let mut parts = msix.splitn(2, '|');
    let ver = parts.next().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let loc = parts.next().map(str::trim).unwrap_or("").to_string();
    Some((ver, loc, "msix".to_string()))
}

#[cfg(target_os = "windows")]
fn is_windows_claude_running() -> bool {
    ps_hidden("if (Get-Process -Name 'Claude' -ErrorAction SilentlyContinue) { 'true' }").as_deref()
        == Some("true")
}

#[cfg(target_os = "windows")]
fn quit_windows_claude() -> Result<(), String> {
    let _ = ps_hidden("Stop-Process -Name 'Claude' -Force -ErrorAction SilentlyContinue");
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_claude(paths: &AppPaths) -> Result<(), String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
    let exe = local_app_data.join("AnthropicClaude").join("Claude.exe");
    if exe.exists() {
        std::process::Command::new(&exe)
            .spawn()
            .map_err(|e| format!("启动 Claude 失败: {e}"))?;
        return Ok(());
    }
    let app_id = ps_hidden(
        "(Get-StartApps | Where-Object { $_.AppID -like '*Claude*' } | Select-Object -First 1).AppID",
    )
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "Claude_pzs8sxrjxfjjc!Claude".to_string());
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{app_id}"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("启动 Claude 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_tako_cli_credentials;
    use tempfile::TempDir;

    #[test]
    fn claude_dual_slot_1p_3p_isolation() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.claude_1p_dir).unwrap();
        fs::write(
            paths.claude_1p_dir.join("claude_desktop_config.json"),
            r#"{"mcpServers":{"existing":{"command":"foo"}}}"#,
        )
        .unwrap();

        save_tako_cli_credentials(&paths, Some("cr_claude_secret"), Some("2")).unwrap();
        let adapter = ClaudeAdapter;

        adapter.apply_mode(&paths, AppMode::Hybrid).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Hybrid);
        assert!(adapter.read_mcp_enabled(&paths, AppMode::Hybrid));
        let p1 = read_json_object_or_empty(&paths.claude_1p_dir.join("claude_desktop_config.json")).unwrap();
        let servers = p1.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers.contains_key("existing"));
        assert!(servers.contains_key("tako"));

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        assert!(adapter.read_mcp_enabled(&paths, AppMode::Tako));
        assert!(adapter.diagnose(&paths).ok);
        let prof = read_json_object_or_empty(&ClaudeAdapter::profile_3p_path(&paths)).unwrap();
        assert_eq!(
            prof.get("inferenceGatewayBaseUrl").and_then(Value::as_str),
            Some("https://tako.shiroha.tech")
        );
        assert_eq!(
            prof.get("inferenceGatewayAuthScheme").and_then(Value::as_str),
            Some("bearer")
        );
        // 1p should no longer carry tako when 3p is active
        assert!(!has_tako_mcp_server(&paths.claude_1p_dir.join("claude_desktop_config.json")));

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        assert!(!ClaudeAdapter::profile_3p_path(&paths).exists());
        let p1_after =
            read_json_object_or_empty(&paths.claude_1p_dir.join("claude_desktop_config.json")).unwrap();
        let servers_after = p1_after.get("mcpServers").unwrap().as_object().unwrap();
        assert!(servers_after.contains_key("existing"));
        assert!(!servers_after.contains_key("tako"));
    }
}
