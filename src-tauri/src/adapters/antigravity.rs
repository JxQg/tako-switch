use crate::adapters::{
    has_tako_mcp_server, merge_tako_mcp_server, read_json_object_or_empty, AppId, AppMode,
    DesktopAppAdapter, DiagnosticResult, InstallInfo,
};
use crate::agy_proxy::{
    ensure_gateway_service, is_proxy_healthy, stop_gateway_service_if_unused, LOCAL_GATEWAY_PORT,
    LOCAL_GATEWAY_URL,
};
use crate::config::{backup_file_once, effective_agy_api_key, write_atomic, AppPaths};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version,
};

#[cfg(not(target_os = "windows"))]
const AGY_ENV_PLIST_NAME: &str = "tech.shiroha.tako-switch.agy-env.plist";
#[cfg(target_os = "macos")]
const AGY_ENV_PLIST_LABEL: &str = "tech.shiroha.tako-switch.agy-env";
#[cfg(target_os = "macos")]
const AGY_MACOS_BUNDLE_ID: &str = "com.google.antigravity";
const AGY_TAKO_MCP_GRANT: &str = "mcp(tako/*)";

pub struct AntigravityAdapter;

impl AntigravityAdapter {
    fn mcp_path(paths: &AppPaths) -> PathBuf {
        paths.agy_gemini_config_dir.join("mcp_config.json")
    }

    fn config_path(paths: &AppPaths) -> PathBuf {
        paths.agy_gemini_config_dir.join("config.json")
    }

    fn has_tako_mcp_permission(paths: &AppPaths) -> bool {
        let Ok(root) = read_json_object_or_empty(&Self::config_path(paths)) else {
            return false;
        };
        root.get("userSettings")
            .and_then(Value::as_object)
            .and_then(|us| us.get("globalPermissionGrants"))
            .and_then(Value::as_object)
            .and_then(|g| g.get("allow"))
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .any(|v| v.as_str() == Some(AGY_TAKO_MCP_GRANT))
            })
            .unwrap_or(false)
    }

    fn set_tako_mcp_permission(paths: &AppPaths, enable: bool) -> Result<(), String> {
        let path = Self::config_path(paths);
        if !enable && !path.exists() {
            return Ok(());
        }
        let mut root = read_json_object_or_empty(&path)?;
        if enable {
            let user_settings = root
                .entry("userSettings".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !user_settings.is_object() {
                *user_settings = Value::Object(Map::new());
            }
            let us_obj = user_settings
                .as_object_mut()
                .ok_or_else(|| "userSettings 必须为 JSON object".to_string())?;
            let grants = us_obj
                .entry("globalPermissionGrants".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !grants.is_object() {
                *grants = Value::Object(Map::new());
            }
            let grants_obj = grants
                .as_object_mut()
                .ok_or_else(|| "globalPermissionGrants 必须为 JSON object".to_string())?;
            let allow = grants_obj
                .entry("allow".to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if !allow.is_array() {
                *allow = Value::Array(Vec::new());
            }
            if let Some(arr) = allow.as_array_mut() {
                if !arr.iter().any(|v| v.as_str() == Some(AGY_TAKO_MCP_GRANT)) {
                    arr.push(Value::String(AGY_TAKO_MCP_GRANT.to_string()));
                }
            }
        } else if let Some(arr) = root
            .get_mut("userSettings")
            .and_then(Value::as_object_mut)
            .and_then(|us| us.get_mut("globalPermissionGrants"))
            .and_then(Value::as_object_mut)
            .and_then(|g| g.get_mut("allow"))
            .and_then(Value::as_array_mut)
        {
            arr.retain(|v| v.as_str() != Some(AGY_TAKO_MCP_GRANT));
        }

        let bytes = serde_json::to_vec_pretty(&Value::Object(root))
            .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
        write_atomic(&path, &bytes, false)
    }

    #[cfg(not(target_os = "windows"))]
    fn plist_path(paths: &AppPaths) -> PathBuf {
        paths.launch_agents_dir.join(AGY_ENV_PLIST_NAME)
    }

    #[cfg(target_os = "windows")]
    fn windows_test_env_path(paths: &AppPaths) -> PathBuf {
        paths.switch_dir().join("agy-test-env.json")
    }

    fn read_gateway_env(paths: &AppPaths) -> BTreeMap<String, String> {
        #[cfg(target_os = "windows")]
        {
            if !paths.is_test {
                return read_windows_hkcu_env();
            }
            fs::read_to_string(Self::windows_test_env_path(paths))
                .ok()
                .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(&raw).ok())
                .unwrap_or_default()
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut map = BTreeMap::new();
            let Ok(raw) = fs::read_to_string(Self::plist_path(paths)) else {
                return map;
            };
            for key in [
                "AGY_LLM_GATEWAY_URL",
                "AGY_LLM_GATEWAY_API_KEY",
                "AGY_LLM_GATEWAY_WIRE_PROTOCOL",
            ] {
                if let Some(val) = extract_launchctl_setenv(&raw, key) {
                    map.insert(key.to_string(), val);
                }
            }
            map
        }
    }

    fn set_gateway_env(paths: &AppPaths, base_url: &str, api_key: &str) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            if paths.is_test {
                let mut map = BTreeMap::new();
                map.insert("AGY_LLM_GATEWAY_URL".to_string(), base_url.to_string());
                map.insert("AGY_LLM_GATEWAY_API_KEY".to_string(), api_key.to_string());
                map.insert("AGY_LLM_GATEWAY_WIRE_PROTOCOL".to_string(), "genai".to_string());
                let bytes = serde_json::to_vec_pretty(&map)
                    .map_err(|e| format!("序列化测试环境失败: {e}"))?;
                return write_atomic(&Self::windows_test_env_path(paths), &bytes, true);
            }
            write_windows_hkcu_env(base_url, api_key)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let plist = render_launch_agent_plist(base_url, api_key);
            let plist_path = Self::plist_path(paths);
            write_atomic(&plist_path, plist.as_bytes(), true)?;
            #[cfg(target_os = "macos")]
            if !paths.is_test {
                apply_macos_launchctl_env(base_url, api_key);
            }
            Ok(())
        }
    }

    fn clear_gateway_env(paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "windows")]
        {
            if paths.is_test {
                let p = Self::windows_test_env_path(paths);
                if p.exists() {
                    let _ = fs::remove_file(p);
                }
                return Ok(());
            }
            clear_windows_hkcu_env()
        }
        #[cfg(not(target_os = "windows"))]
        {
            let plist_path = Self::plist_path(paths);
            if plist_path.exists() {
                fs::remove_file(&plist_path)
                    .map_err(|e| format!("删除 {} 失败: {e}", plist_path.display()))?;
            }
            #[cfg(target_os = "macos")]
            if !paths.is_test {
                clear_macos_launchctl_env();
            }
            Ok(())
        }
    }
}

impl DesktopAppAdapter for AntigravityAdapter {
    fn id(&self) -> AppId {
        AppId::Antigravity
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) =
                find_macos_app(paths, &["Antigravity.app"], AGY_MACOS_BUNDLE_ID)
            {
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
            if let Some((ver, path)) = detect_windows_antigravity(paths) {
                return InstallInfo {
                    installed: true,
                    version: ver,
                    install_path: Some(path),
                    install_kind: Some("exe".to_string()),
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
            is_macos_bundle_id_running(AGY_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            is_windows_process_running("Antigravity")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            quit_macos_bundle_id(AGY_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_process("Antigravity")
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path = find_macos_app(paths, &["Antigravity.app"], AGY_MACOS_BUNDLE_ID);
            launch_macos_app(app_path.as_deref(), AGY_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_antigravity(paths)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 Google Antigravity".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        let env = Self::read_gateway_env(paths);
        if env.contains_key("AGY_LLM_GATEWAY_URL") && env.contains_key("AGY_LLM_GATEWAY_API_KEY") {
            AppMode::Tako
        } else {
            AppMode::Official
        }
    }

    fn read_mcp_enabled(&self, paths: &AppPaths, _mode: AppMode) -> bool {
        has_tako_mcp_server(&Self::mcp_path(paths))
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let mcp_path = Self::mcp_path(paths);
        let config_path = Self::config_path(paths);
        backup_file_once(paths, "antigravity", "mcp_config.json", &mcp_path)?;
        backup_file_once(paths, "antigravity", "config.json", &config_path)?;

        match mode {
            AppMode::Official => {
                Self::clear_gateway_env(paths)?;
                merge_tako_mcp_server(&mcp_path, paths, false)?;
                Self::set_tako_mcp_permission(paths, false)?;
                stop_gateway_service_if_unused(paths)
            }
            AppMode::Tako => {
                let api_key = effective_agy_api_key(paths).ok_or_else(|| {
                    "请先登录 Tako 账号或配置专用反重力 Key，再切换到全 Tako 模式".to_string()
                })?;
                ensure_gateway_service(paths)?;
                Self::set_gateway_env(paths, LOCAL_GATEWAY_URL, &api_key)?;
                merge_tako_mcp_server(&mcp_path, paths, true)?;
                Self::set_tako_mcp_permission(paths, true)
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let mut notes = vec![format!("MCP 配置: {}", Self::mcp_path(paths).display())];
        let mode = self.read_mode(paths);
        if mode == AppMode::Tako {
            let env = Self::read_gateway_env(paths);
            if env
                .get("AGY_LLM_GATEWAY_WIRE_PROTOCOL")
                .map(String::as_str)
                != Some("genai")
            {
                issues.push("AGY_LLM_GATEWAY_WIRE_PROTOCOL 必须为 genai".to_string());
            }
            if env.get("AGY_LLM_GATEWAY_URL").map(|s| s.is_empty()).unwrap_or(true) {
                issues.push("AGY_LLM_GATEWAY_URL 为空".to_string());
            } else if env.get("AGY_LLM_GATEWAY_URL").map(String::as_str) == Some(LOCAL_GATEWAY_URL)
                && !paths.is_test
            {
                if is_proxy_healthy(LOCAL_GATEWAY_PORT) {
                    notes.push(format!("本地统一网关: 运行中 (127.0.0.1:{LOCAL_GATEWAY_PORT})"));
                } else {
                    issues.push(format!(
                        "本地统一网关 (127.0.0.1:{LOCAL_GATEWAY_PORT}) 未运行，请重新点击 Tako 或重启 Tako Switch"
                    ));
                }
            }
            if env
                .get("AGY_LLM_GATEWAY_API_KEY")
                .map(|s| s.is_empty())
                .unwrap_or(true)
            {
                issues.push("AGY_LLM_GATEWAY_API_KEY 为空".to_string());
            }
            if Self::has_tako_mcp_permission(paths) {
                notes.push("MCP 自动放行: mcp(tako/*) 已写入 config.json".to_string());
            } else {
                notes.push(
                    "提示: 未检测到 mcp(tako/*) 自动放行规则，重新点击 Tako 可自动补全"
                        .to_string(),
                );
            }
            notes.push("提示: 新建 Antigravity 网关环境变量需完整退出并重启应用后生效".to_string());
        }

        DiagnosticResult {
            ok: issues.is_empty(),
            issues,
            notes,
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(not(target_os = "windows"))]
fn render_launch_agent_plist(base_url: &str, api_key: &str) -> String {
    let sh_url = shell_single_quote(base_url);
    let sh_key = shell_single_quote(api_key);
    let cmd = format!(
        "launchctl setenv AGY_LLM_GATEWAY_URL {sh_url} && launchctl setenv AGY_LLM_GATEWAY_API_KEY {sh_key} && launchctl setenv AGY_LLM_GATEWAY_WIRE_PROTOCOL genai"
    );
    let escaped_cmd = xml_escape(&cmd);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>tech.shiroha.tako-switch.agy-env</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-c</string>
    <string>{escaped_cmd}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

#[cfg(not(target_os = "windows"))]
fn shell_single_quote(val: &str) -> String {
    format!("'{}'", val.replace('\'', "'\"'\"'"))
}

#[cfg(not(target_os = "windows"))]
fn xml_unescape(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(not(target_os = "windows"))]
fn extract_launchctl_setenv(plist: &str, key: &str) -> Option<String> {
    let decoded = xml_unescape(plist);
    let needle = format!("launchctl setenv {key} ");
    let start = decoded.find(&needle)? + needle.len();
    let rest = &decoded[start..];
    if let Some(after_q) = rest.strip_prefix('\'') {
        let mut out = String::new();
        let mut cursor = after_q;
        while !cursor.is_empty() {
            if let Some(rem) = cursor.strip_prefix("'\"'\"'") {
                out.push('\'');
                cursor = rem;
            } else if let Some(ch) = cursor.chars().next() {
                if ch == '\'' {
                    return Some(out);
                }
                out.push(ch);
                cursor = &cursor[ch.len_utf8()..];
            }
        }
        return Some(out);
    }
    let end = rest.find([' ', '<', '\n']).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

#[cfg(target_os = "macos")]
fn apply_macos_launchctl_env(base_url: &str, api_key: &str) {
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["setenv", "AGY_LLM_GATEWAY_URL", base_url])
        .output();
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["setenv", "AGY_LLM_GATEWAY_API_KEY", api_key])
        .output();
    let _ = std::process::Command::new("/bin/launchctl")
        .args(["setenv", "AGY_LLM_GATEWAY_WIRE_PROTOCOL", "genai"])
        .output();
    // Ensure LaunchAgent is active for future logins without failing if GUI session is absent.
    if let Ok(uid_out) = std::process::Command::new("/usr/bin/id").arg("-u").output() {
        let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
        if !uid.is_empty() {
            let domain_label = format!("gui/{uid}/{AGY_ENV_PLIST_LABEL}");
            let _ = std::process::Command::new("/bin/launchctl")
                .args(["bootout", &domain_label])
                .output();
            if let Some(home) = dirs::home_dir() {
                let plist = home
                    .join("Library")
                    .join("LaunchAgents")
                    .join(AGY_ENV_PLIST_NAME);
                let _ = std::process::Command::new("/bin/launchctl")
                    .args(["bootstrap", &format!("gui/{uid}"), &plist.to_string_lossy()])
                    .output();
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn clear_macos_launchctl_env() {
    for key in [
        "AGY_LLM_GATEWAY_URL",
        "AGY_LLM_GATEWAY_API_KEY",
        "AGY_LLM_GATEWAY_WIRE_PROTOCOL",
    ] {
        let _ = std::process::Command::new("/bin/launchctl")
            .args(["unsetenv", key])
            .output();
    }
    if let Ok(uid_out) = std::process::Command::new("/usr/bin/id").arg("-u").output() {
        let uid = String::from_utf8_lossy(&uid_out.stdout).trim().to_string();
        if !uid.is_empty() {
            let _ = std::process::Command::new("/bin/launchctl")
                .args(["bootout", &format!("gui/{uid}/{AGY_ENV_PLIST_LABEL}")])
                .output();
        }
    }
}

#[cfg(target_os = "windows")]
fn read_windows_hkcu_env() -> BTreeMap<String, String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let mut map = BTreeMap::new();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env) = hkcu.open_subkey("Environment") {
        for key in [
            "AGY_LLM_GATEWAY_URL",
            "AGY_LLM_GATEWAY_API_KEY",
            "AGY_LLM_GATEWAY_WIRE_PROTOCOL",
        ] {
            if let Ok(val) = env.get_value::<String, _>(key) {
                map.insert(key.to_string(), val);
            }
        }
    }
    map
}

#[cfg(target_os = "windows")]
fn write_windows_hkcu_env(base_url: &str, api_key: &str) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env, _) = hkcu
        .create_subkey("Environment")
        .map_err(|e| format!("打开注册表 HKCU\\Environment 失败: {e}"))?;
    env.set_value("AGY_LLM_GATEWAY_URL", &base_url)
        .map_err(|e| format!("写入 AGY_LLM_GATEWAY_URL 失败: {e}"))?;
    env.set_value("AGY_LLM_GATEWAY_API_KEY", &api_key)
        .map_err(|e| format!("写入 AGY_LLM_GATEWAY_API_KEY 失败: {e}"))?;
    env.set_value("AGY_LLM_GATEWAY_WIRE_PROTOCOL", &"genai")
        .map_err(|e| format!("写入 AGY_LLM_GATEWAY_WIRE_PROTOCOL 失败: {e}"))?;

    // Also broadcast WM_SETTINGCHANGE via .NET SetEnvironmentVariable so Explorer picks up changes immediately.
    let ps = format!(
        "[Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_URL','{}','User'); \
         [Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_API_KEY','{}','User'); \
         [Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_WIRE_PROTOCOL','genai','User')",
        base_url.replace('\'', "''"),
        api_key.replace('\'', "''")
    );
    let _ = ps_hidden(&ps);
    Ok(())
}

#[cfg(target_os = "windows")]
fn clear_windows_hkcu_env() -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env) = hkcu.open_subkey_with_flags("Environment", KEY_SET_VALUE) {
        for key in [
            "AGY_LLM_GATEWAY_URL",
            "AGY_LLM_GATEWAY_API_KEY",
            "AGY_LLM_GATEWAY_WIRE_PROTOCOL",
        ] {
            let _ = env.delete_value(key);
        }
    }
    let _ = ps_hidden(
        "[Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_URL',$null,'User'); \
         [Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_API_KEY',$null,'User'); \
         [Environment]::SetEnvironmentVariable('AGY_LLM_GATEWAY_WIRE_PROTOCOL',$null,'User')",
    );
    Ok(())
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
fn detect_windows_antigravity(paths: &AppPaths) -> Option<(Option<String>, String)> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
    let candidates = [
        local_app_data
            .join("Programs")
            .join("Antigravity")
            .join("Antigravity.exe"),
        PathBuf::from(r"C:\Program Files\Antigravity\Antigravity.exe"),
    ];
    for exe in candidates {
        if exe.exists() {
            let script = format!(
                "(Get-Item '{}').VersionInfo.ProductVersion",
                exe.to_string_lossy().replace('\'', "''")
            );
            let ver = ps_hidden(&script).filter(|s| !s.is_empty());
            return Some((ver, exe.to_string_lossy().to_string()));
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn is_windows_process_running(name: &str) -> bool {
    let script = format!(
        "if (Get-Process -Name '{name}' -ErrorAction SilentlyContinue) {{ 'true' }}"
    );
    ps_hidden(&script).as_deref() == Some("true")
}

#[cfg(target_os = "windows")]
fn quit_windows_process(name: &str) -> Result<(), String> {
    let script = format!("Stop-Process -Name '{name}' -Force -ErrorAction SilentlyContinue");
    let _ = ps_hidden(&script);
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_antigravity(paths: &AppPaths) -> Result<(), String> {
    if let Some((_, exe)) = detect_windows_antigravity(paths) {
        let mut cmd = std::process::Command::new(exe);
        let gw_env = AntigravityAdapter::read_gateway_env(paths);
        for key in [
            "AGY_LLM_GATEWAY_URL",
            "AGY_LLM_GATEWAY_API_KEY",
            "AGY_LLM_GATEWAY_WIRE_PROTOCOL",
        ] {
            if let Some(val) = gw_env.get(key) {
                cmd.env(key, val);
            } else {
                cmd.env_remove(key);
            }
        }
        cmd.spawn()
            .map_err(|e| format!("启动 Antigravity 失败: {e}"))?;
        return Ok(());
    }
    Err("未找到 Antigravity 安装路径".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{read_switch_state, save_switch_auth, save_switch_state};
    use tempfile::TempDir;

    #[test]
    fn antigravity_uses_genai_wire_protocol_and_dedicated_key_override() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.agy_gemini_config_dir).unwrap();
        fs::write(
            paths.agy_gemini_config_dir.join("mcp_config.json"),
            r#"{"mcpServers":{"other":{"command":"npx"}}}"#,
        )
        .unwrap();
        fs::write(
            paths.agy_gemini_config_dir.join("config.json"),
            r#"{"plugins":{"swarm":{"enabled":true}},"userSettings":{"permissionPreset":"AGENT_PERMISSION_PRESET_TURBO","globalPermissionGrants":{"allow":["command(git status)"]}}}"#,
        )
        .unwrap();

        save_switch_auth(&paths, Some("cr_main_key"), Some("1"), Some("oauth")).unwrap();
        let mut state = read_switch_state(&paths);
        state.agy_api_key = Some("cr_agy_only".to_string());
        save_switch_state(&paths, &state).unwrap();

        let adapter = AntigravityAdapter;

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        // Applying Tako a second time must not duplicate mcp(tako/*)
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        let env = AntigravityAdapter::read_gateway_env(&paths);
        assert_eq!(
            env.get("AGY_LLM_GATEWAY_URL").map(String::as_str),
            Some(LOCAL_GATEWAY_URL)
        );
        #[cfg(not(target_os = "windows"))]
        assert!(crate::agy_proxy::gateway_plist_path(&paths).exists());
        assert_eq!(
            env.get("AGY_LLM_GATEWAY_API_KEY").map(String::as_str),
            Some("cr_agy_only")
        );
        assert_eq!(
            env.get("AGY_LLM_GATEWAY_WIRE_PROTOCOL").map(String::as_str),
            Some("genai")
        );
        assert!(AntigravityAdapter::has_tako_mcp_permission(&paths));
        let cfg_tako = read_json_object_or_empty(&AntigravityAdapter::config_path(&paths)).unwrap();
        assert!(cfg_tako.get("plugins").is_some());
        let us_tako = cfg_tako.get("userSettings").unwrap().as_object().unwrap();
        assert_eq!(
            us_tako.get("permissionPreset").and_then(Value::as_str),
            Some("AGENT_PERMISSION_PRESET_TURBO")
        );
        let allow_tako = us_tako
            .get("globalPermissionGrants")
            .unwrap()
            .as_object()
            .unwrap()
            .get("allow")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            allow_tako
                .iter()
                .filter(|v| v.as_str() == Some(AGY_TAKO_MCP_GRANT))
                .count(),
            1
        );
        assert!(allow_tako
            .iter()
            .any(|v| v.as_str() == Some("command(git status)")));
        assert!(adapter.diagnose(&paths).ok);

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        assert!(AntigravityAdapter::read_gateway_env(&paths).is_empty());
        #[cfg(not(target_os = "windows"))]
        assert!(!crate::agy_proxy::gateway_plist_path(&paths).exists());
        assert!(!adapter.read_mcp_enabled(&paths, AppMode::Official));
        assert!(!AntigravityAdapter::has_tako_mcp_permission(&paths));
        let cfg_off = read_json_object_or_empty(&AntigravityAdapter::config_path(&paths)).unwrap();
        assert!(cfg_off.get("plugins").is_some());
        let allow_off = cfg_off
            .get("userSettings")
            .unwrap()
            .as_object()
            .unwrap()
            .get("globalPermissionGrants")
            .unwrap()
            .as_object()
            .unwrap()
            .get("allow")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(allow_off
            .iter()
            .any(|v| v.as_str() == Some("command(git status)")));
    }
}
