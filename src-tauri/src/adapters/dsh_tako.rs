use crate::adapters::{AppId, AppMode, DesktopAppAdapter, DiagnosticResult, InstallInfo};
use crate::auth::mask_api_key;
use crate::config::{backup_file_once, read_backup, read_main_api_key, write_atomic, AppPaths};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use crate::adapters::{
    find_macos_app, is_macos_bundle_id_running, launch_macos_app, quit_macos_bundle_id,
    read_macos_app_bundle_version,
};

#[cfg(target_os = "macos")]
const DSH_TAKO_MACOS_BUNDLE_ID: &str = "tech.shiroha.dsh-tako";

pub struct DshTakoAdapter;

impl DshTakoAdapter {
    pub fn credentials_path(paths: &AppPaths) -> PathBuf {
        paths.dsh_tako_dir.join(".credentials.yaml")
    }

    pub fn synced_marker_path(paths: &AppPaths) -> PathBuf {
        paths.switch_dir().join("dsh-tako-synced")
    }
}

pub fn has_any_dsh_tako_key(paths: &AppPaths) -> bool {
    let path = DshTakoAdapter::credentials_path(paths);
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    read_dsh_tako_api_key(&raw).is_some()
}

fn format_yaml_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let plain_safe = !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'));
    if plain_safe {
        trimmed.to_string()
    } else {
        let escaped = trimmed.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

fn strip_unquoted_comment(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(idx) = trimmed.find(" #") {
        trimmed[..idx].trim()
    } else {
        trimmed
    }
}

fn unquote_yaml_scalar(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(inner) = trimmed.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = inner.chars();
        let mut closed = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(next) = chars.next() {
                        match next {
                            'n' => out.push('\n'),
                            't' => out.push('\t'),
                            'r' => out.push('\r'),
                            other => out.push(other),
                        }
                    }
                }
                '"' => {
                    closed = true;
                    break;
                }
                other => out.push(other),
            }
        }
        let val = if closed { out } else { trimmed.to_string() };
        let val = val.trim().to_string();
        return (!val.is_empty()).then_some(val);
    }
    if let Some(inner) = trimmed.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    out.push('\'');
                } else {
                    break;
                }
            } else {
                out.push(c);
            }
        }
        let val = out.trim().to_string();
        return (!val.is_empty()).then_some(val);
    }
    let clean = strip_unquoted_comment(trimmed);
    (!clean.is_empty()).then(|| clean.to_string())
}

fn is_refs_key_line(trimmed: &str) -> bool {
    trimmed == "refs:"
        || trimmed.starts_with("refs: ")
        || trimmed.starts_with("refs:#")
        || trimmed.starts_with("refs:{")
}

fn parse_tako_api_key_line(trimmed: &str) -> Option<&str> {
    for prefix in ["TAKO_API_KEY:", "\"TAKO_API_KEY\":", "'TAKO_API_KEY':"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest.trim());
        }
    }
    None
}

pub fn read_dsh_tako_api_key(raw: &str) -> Option<String> {
    let mut in_refs = false;
    let mut refs_child_indent: Option<usize> = None;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent == 0 {
            refs_child_indent = None;
            if is_refs_key_line(trimmed) {
                if trimmed == "refs:" || trimmed.starts_with("refs: #") {
                    in_refs = true;
                } else {
                    in_refs = false;
                    // Best-effort inline flow map support (`refs: { TAKO_API_KEY: ... }`)
                    if let Some(body) = trimmed
                        .strip_prefix("refs:")
                        .map(str::trim)
                        .and_then(|s| s.strip_prefix('{'))
                        .and_then(|s| s.strip_suffix('}'))
                    {
                        for part in body.split(',') {
                            if let Some(val_raw) = parse_tako_api_key_line(part.trim()) {
                                return unquote_yaml_scalar(val_raw);
                            }
                        }
                    }
                }
            } else {
                in_refs = false;
            }
            continue;
        }

        if in_refs {
            let expected_indent = *refs_child_indent.get_or_insert(indent);
            if indent == expected_indent {
                if let Some(val_raw) = parse_tako_api_key_line(trimmed) {
                    return unquote_yaml_scalar(val_raw);
                }
            }
        }
    }
    None
}

pub fn patch_dsh_credentials_yaml(existing: &str, api_key: Option<&str>) -> String {
    let normalized = existing.replace("\r\n", "\n");
    if normalized.trim().is_empty() {
        return match api_key {
            Some(key) => format!("version: 1\nrefs:\n  TAKO_API_KEY: {}\n", format_yaml_scalar(key)),
            None => String::new(),
        };
    }

    let lines: Vec<&str> = normalized.split('\n').collect();
    let had_trailing_newline = normalized.ends_with('\n');
    let src_len = if had_trailing_newline && lines.last() == Some(&"") {
        lines.len() - 1
    } else {
        lines.len()
    };

    let mut refs_header_idx: Option<usize> = None;
    let mut refs_end_idx: Option<usize> = None;
    let mut refs_child_indent = "  ".to_string();
    let mut refs_child_indent_len: Option<usize> = None;
    let mut existing_key_line_idx: Option<usize> = None;
    let mut refs_other_children = 0usize;
    let mut has_version_key = false;
    let mut in_refs = false;

    for (idx, line) in lines[..src_len].iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        let indent_len = line.len() - line.trim_start_matches(' ').len();
        if indent_len == 0 {
            if in_refs {
                refs_end_idx = Some(idx);
            }
            in_refs = false;
            if trimmed.starts_with("version:") {
                has_version_key = true;
            } else if is_refs_key_line(trimmed) {
                refs_header_idx = Some(idx);
                in_refs = trimmed == "refs:" || trimmed.starts_with("refs: #");
            }
            continue;
        }

        if in_refs {
            let expected = *refs_child_indent_len.get_or_insert_with(|| {
                refs_child_indent = line[..indent_len].to_string();
                indent_len
            });
            if indent_len == expected {
                if parse_tako_api_key_line(trimmed).is_some() {
                    if existing_key_line_idx.is_none() {
                        existing_key_line_idx = Some(idx);
                    }
                } else {
                    refs_other_children += 1;
                }
            }
        }
    }
    if in_refs && refs_end_idx.is_none() {
        refs_end_idx = Some(src_len);
    }

    let mut out: Vec<String> = Vec::with_capacity(src_len + 4);
    match api_key {
        Some(key) => {
            let entry = format!("{refs_child_indent}TAKO_API_KEY: {}", format_yaml_scalar(key));
            let mut inserted = false;

            for (idx, line) in lines[..src_len].iter().enumerate() {
                let trimmed = line.trim();
                if Some(idx) == existing_key_line_idx {
                    let indent = &line[..(line.len() - line.trim_start_matches(' ').len())];
                    out.push(format!("{indent}TAKO_API_KEY: {}", format_yaml_scalar(key)));
                    inserted = true;
                    continue;
                }
                if !inserted && refs_header_idx == Some(idx) && trimmed != "refs:" && !trimmed.starts_with("refs: #") {
                    // Handles `refs: {}` or inline `refs:` forms cleanly.
                    out.push("refs:".to_string());
                    out.push(entry.clone());
                    inserted = true;
                    continue;
                }
                if !inserted && refs_end_idx == Some(idx) {
                    out.push(entry.clone());
                    inserted = true;
                }
                out.push((*line).to_string());
            }

            if !inserted {
                if refs_header_idx.is_some() {
                    out.push(entry);
                } else {
                    if !has_version_key {
                        out.push("version: 1".to_string());
                    }
                    out.push("refs:".to_string());
                    out.push(format!("  TAKO_API_KEY: {}", format_yaml_scalar(key)));
                }
            }
        }
        None => {
            for (idx, line) in lines[..src_len].iter().enumerate() {
                if Some(idx) == existing_key_line_idx {
                    continue;
                }
                if refs_header_idx == Some(idx)
                    && refs_other_children == 0
                    && existing_key_line_idx.is_some()
                    && (line.trim() == "refs:" || line.trim().starts_with("refs: #"))
                {
                    out.push("refs: {}".to_string());
                    continue;
                }
                out.push((*line).to_string());
            }
        }
    }

    if out.is_empty() {
        return String::new();
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

impl DesktopAppAdapter for DshTakoAdapter {
    fn id(&self) -> AppId {
        AppId::DshTako
    }

    fn detect_install(&self, paths: &AppPaths) -> InstallInfo {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            if let Some(app_path) = find_macos_app(paths, &["DSH Tako.app"], DSH_TAKO_MACOS_BUNDLE_ID) {
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
            if let Some((ver, path, kind)) = detect_windows_dsh_tako(paths) {
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
            is_macos_bundle_id_running(DSH_TAKO_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            super::windows_process::is_running(AppId::DshTako)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        false
    }

    fn quit_app(&self, _paths: &AppPaths) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            quit_macos_bundle_id(DSH_TAKO_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            quit_windows_dsh_tako()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Ok(())
    }

    fn launch_app(&self, paths: &AppPaths) -> Result<(), String> {
        let _ = paths;
        #[cfg(target_os = "macos")]
        {
            let app_path = find_macos_app(paths, &["DSH Tako.app"], DSH_TAKO_MACOS_BUNDLE_ID);
            launch_macos_app(app_path.as_deref(), DSH_TAKO_MACOS_BUNDLE_ID)
        }
        #[cfg(target_os = "windows")]
        {
            launch_windows_dsh_tako(paths)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err("当前操作系统不支持拉起 DSH Tako".to_string())
    }

    fn read_mode(&self, paths: &AppPaths) -> AppMode {
        let cred_path = Self::credentials_path(paths);
        let Ok(raw) = fs::read_to_string(cred_path) else {
            return AppMode::Official;
        };
        let Some(cred_key) = read_dsh_tako_api_key(&raw) else {
            return AppMode::Official;
        };
        let marker_exists = Self::synced_marker_path(paths).exists();
        if marker_exists || read_main_api_key(paths).as_deref() == Some(cred_key.as_str()) {
            AppMode::Tako
        } else {
            AppMode::Official
        }
    }

    fn read_mcp_enabled(&self, _paths: &AppPaths, _mode: AppMode) -> bool {
        false
    }

    fn apply_mode(&self, paths: &AppPaths, mode: AppMode) -> Result<(), String> {
        let cred_path = Self::credentials_path(paths);
        let marker_path = Self::synced_marker_path(paths);

        match mode {
            AppMode::Tako => {
                let api_key = read_main_api_key(paths).ok_or_else(|| {
                    "请先在顶部登录 Tako 账号，再同步密钥至 DSH Tako".to_string()
                })?;
                backup_file_once(paths, "dsh-tako", "credentials.yaml", &cred_path)?;
                let existing = fs::read_to_string(&cred_path).unwrap_or_default();
                let patched = patch_dsh_credentials_yaml(&existing, Some(&api_key));
                write_atomic(&cred_path, patched.as_bytes(), true)?;
                write_atomic(&marker_path, b"1", false)?;
                Ok(())
            }
            AppMode::Official => {
                if marker_path.exists() {
                    let _ = fs::remove_file(&marker_path);
                }
                if let Some(backup) = read_backup(paths, "dsh-tako", "credentials.yaml") {
                    write_atomic(&cred_path, &backup, true)?;
                } else if cred_path.exists() {
                    let existing = fs::read_to_string(&cred_path).unwrap_or_default();
                    let patched = patch_dsh_credentials_yaml(&existing, None);
                    if patched.trim().is_empty() {
                        let _ = fs::remove_file(&cred_path);
                    } else {
                        write_atomic(&cred_path, patched.as_bytes(), true)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn diagnose(&self, paths: &AppPaths) -> DiagnosticResult {
        let mut issues = Vec::new();
        let cred_path = Self::credentials_path(paths);
        let mut notes = vec![format!("凭据文件: {}", cred_path.display())];

        let install = self.detect_install(paths);
        if let Some(path) = install.install_path {
            notes.push(format!("应用路径: {path}"));
        } else {
            notes.push("未检测到 DSH Tako 安装包，可点击「一键安装」下载并配置".to_string());
        }

        if cred_path.exists() {
            #[cfg(unix)]
            if let Ok(meta) = fs::metadata(&cred_path) {
                let mode = meta.permissions().mode() & 0o777;
                if (mode & 0o077) != 0 {
                    issues.push(format!(
                        "{} 权限过宽 ({:03o})，需保持仅所有者可读写 (0600)",
                        cred_path.display(),
                        mode
                    ));
                }
            }

            match fs::read_to_string(&cred_path) {
                Ok(raw) => {
                    if let Some(key) = read_dsh_tako_api_key(&raw) {
                        notes.push(format!("TAKO_API_KEY: 已写入 ({})", mask_api_key(&key)));
                    } else if self.read_mode(paths) == AppMode::Tako {
                        issues.push("处于同步密钥状态，但 refs.TAKO_API_KEY 为空".to_string());
                    } else {
                        notes.push("处于独立登录状态（未由 Tako Switch 同步密钥）".to_string());
                    }
                }
                Err(err) => {
                    issues.push(format!("读取 {} 失败: {err}", cred_path.display()));
                }
            }
        } else if self.read_mode(paths) == AppMode::Tako {
            issues.push("缺少 .credentials.yaml，重新切换「同步」可自动生成".to_string());
        } else {
            notes.push("尚无本地凭据文件：切换「同步」或在 DSH Tako 内登录即可写入".to_string());
        }

        notes.push("DSH Tako 原生内置 Tako 模型目录与联网工具，无需占用本地网关端口".to_string());
        if crate::config::read_switch_state(paths).mcp.web_page {
            notes.push(
                "网站上线 MCP (EdgeOne): DSH Tako 当前不读取外部 mcpServers JSON，已保持配置文件无改动"
                    .to_string(),
            );
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
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
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
fn read_windows_uninstall_info() -> Option<(Option<String>, Option<PathBuf>)> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let uninstall = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall")
        .ok()?;
    for name in uninstall.enum_keys().flatten() {
        let Ok(sub) = uninstall.open_subkey(&name) else {
            continue;
        };
        let Ok(display_name) = sub.get_value::<String, _>("DisplayName") else {
            continue;
        };
        if display_name.starts_with("DSH Tako") {
            let version = sub
                .get_value::<String, _>("DisplayVersion")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let loc = sub
                .get_value::<String, _>("InstallLocation")
                .ok()
                .map(|s| PathBuf::from(s.trim()));
            return Some((version, loc));
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn find_windows_dsh_tako_exe(paths: &AppPaths) -> (Option<String>, Option<PathBuf>) {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths.home.join("AppData").join("Local"));

    let mut reg_version = None;
    let mut candidates = vec![
        local_app_data
            .join("Programs")
            .join("DSH Tako")
            .join("DSH Tako.exe"),
        local_app_data
            .join("Programs")
            .join("dsh-tako")
            .join("DSH Tako.exe"),
    ];

    if let Some((ver, loc)) = read_windows_uninstall_info() {
        reg_version = ver;
        if let Some(dir) = loc {
            let exe = dir.join("DSH Tako.exe");
            if !candidates.contains(&exe) {
                candidates.insert(0, exe);
            }
        }
    }

    let found = candidates.into_iter().find(|p| p.exists());
    (reg_version, found)
}

#[cfg(target_os = "windows")]
fn detect_windows_dsh_tako(paths: &AppPaths) -> Option<(Option<String>, String, String)> {
    let (reg_ver, exe_opt) = find_windows_dsh_tako_exe(paths);
    let exe = exe_opt?;
    let ver = reg_ver.or_else(|| {
        let script = format!(
            "(Get-Item '{}').VersionInfo.ProductVersion",
            exe.to_string_lossy().replace('\'', "''")
        );
        ps_hidden(&script).filter(|s| !s.is_empty())
    });
    Some((ver, exe.to_string_lossy().to_string(), "exe".to_string()))
}

#[cfg(target_os = "windows")]
fn quit_windows_dsh_tako() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let _ = std::process::Command::new("taskkill.exe")
        .args(["/IM", "DSH Tako.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    for _ in 0..10 {
        if !super::windows_process::is_running(AppId::DshTako) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    let _ = std::process::Command::new("taskkill.exe")
        .args(["/F", "/IM", "DSH Tako.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    Ok(())
}

#[cfg(target_os = "windows")]
fn launch_windows_dsh_tako(paths: &AppPaths) -> Result<(), String> {
    let (_, exe_opt) = find_windows_dsh_tako_exe(paths);
    let exe = exe_opt.ok_or_else(|| "未找到 DSH Tako.exe 安装路径".to_string())?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| format!("启动 DSH Tako 失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_switch_auth;
    use tempfile::TempDir;

    #[test]
    fn patch_credentials_yaml_from_empty_writes_v1_schema() {
        let out = patch_dsh_credentials_yaml("", Some("cr_tako_demo_12345"));
        assert_eq!(out, "version: 1\nrefs:\n  TAKO_API_KEY: cr_tako_demo_12345\n");
        assert_eq!(
            read_dsh_tako_api_key(&out).as_deref(),
            Some("cr_tako_demo_12345")
        );
    }

    #[test]
    fn patch_credentials_yaml_preserves_records_comments_and_other_refs() {
        let initial = "\
# DSH credentials
version: 1
records:
  client-connection/browser-session:
    kind: oauth
    payload:
      version: 1
      secret: session_secret_token
refs:
  LOCAL_CHAT_API_KEY: \"local\"
  TAKO_API_KEY: cr_old_key_value
";
        let updated = patch_dsh_credentials_yaml(initial, Some("cr_new_key_value"));
        assert!(updated.contains("# DSH credentials"));
        assert!(updated.contains("secret: session_secret_token"));
        assert!(updated.contains("LOCAL_CHAT_API_KEY: \"local\""));
        assert!(updated.contains("TAKO_API_KEY: cr_new_key_value"));
        assert!(!updated.contains("cr_old_key_value"));
        assert_eq!(
            read_dsh_tako_api_key(&updated).as_deref(),
            Some("cr_new_key_value")
        );

        let removed = patch_dsh_credentials_yaml(&updated, None);
        assert!(removed.contains("LOCAL_CHAT_API_KEY: \"local\""));
        assert!(!removed.contains("TAKO_API_KEY"));
        assert!(read_dsh_tako_api_key(&removed).is_none());
    }

    #[test]
    fn patch_credentials_yaml_appends_refs_and_handles_empty_refs_on_remove() {
        let records_only = "version: 1\nrecords:\n  foo:\n    kind: k\n";
        let added = patch_dsh_credentials_yaml(records_only, Some("cr_added_key"));
        assert_eq!(
            read_dsh_tako_api_key(&added).as_deref(),
            Some("cr_added_key")
        );
        assert!(added.contains("records:\n  foo:\n    kind: k"));

        let removed = patch_dsh_credentials_yaml(&added, None);
        assert!(removed.contains("refs: {}"));
        assert!(!removed.contains("TAKO_API_KEY"));
        assert!(read_dsh_tako_api_key(&removed).is_none());

        let from_braces = patch_dsh_credentials_yaml("version: 1\nrefs: {}\n", Some("cr_braces"));
        assert_eq!(
            from_braces,
            "version: 1\nrefs:\n  TAKO_API_KEY: cr_braces\n"
        );
    }

    #[test]
    fn apply_mode_round_trip_preserves_0600_and_restores_backup() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        fs::create_dir_all(&paths.dsh_tako_dir).unwrap();
        let cred_path = DshTakoAdapter::credentials_path(&paths);
        let original = "\
version: 1
records:
  client-connection/browser-session:
    kind: token
refs:
  LOCAL_CHAT_API_KEY: \"local\"
  TAKO_API_KEY: cr_standalone_user_key
";
        write_atomic(&cred_path, original.as_bytes(), true).unwrap();
        save_switch_auth(&paths, Some("cr_switch_main_key"), Some("9"), Some("oauth")).unwrap();

        let adapter = DshTakoAdapter;
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);

        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Tako);
        assert!(has_any_dsh_tako_key(&paths));
        let synced_raw = fs::read_to_string(&cred_path).unwrap();
        assert_eq!(
            read_dsh_tako_api_key(&synced_raw).as_deref(),
            Some("cr_switch_main_key")
        );
        assert!(synced_raw.contains("LOCAL_CHAT_API_KEY: \"local\""));

        #[cfg(unix)]
        {
            let mode = fs::metadata(&cred_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        assert!(adapter.diagnose(&paths).ok);

        adapter.apply_mode(&paths, AppMode::Official).unwrap();
        assert_eq!(adapter.read_mode(&paths), AppMode::Official);
        let restored = fs::read_to_string(&cred_path).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn dsh_tako_edgeone_web_page_enabled_does_not_create_unsupported_mcp_files() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        save_switch_auth(&paths, Some("cr_switch_main_key"), Some("9"), Some("oauth")).unwrap();

        let mut st = crate::config::read_switch_state(&paths);
        st.mcp.web_page = true;
        crate::config::save_switch_state(&paths, &st).unwrap();

        let adapter = DshTakoAdapter;
        adapter.apply_mode(&paths, AppMode::Tako).unwrap();
        let diag = adapter.diagnose(&paths);
        assert!(diag.ok);
        assert!(diag
            .notes
            .iter()
            .any(|n| n.contains("EdgeOne") && n.contains("无改动")));
        assert!(!paths.dsh_tako_dir.join("mcp.json").exists());
        assert!(!paths.dsh_tako_dir.join("config.yaml").exists());
    }
}
