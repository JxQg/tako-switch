use crate::adapters::AppId;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const UI_PATCH_BLOCK_PREFIX: &str = "[TAKO_UI_PATCH_BLOCK]";
const ELECTRON_FUSE_V1_SENTINEL: &[u8; 32] = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiPatchBlockKind {
    MacosAppManagement,
    WindowsUac,
    WindowsFileLock,
    WindowsMsixReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchPermissionPrompt {
    pub app_id: AppId,
    pub app_name: String,
    pub action: String, // "patch" | "restore"
    pub kind: UiPatchBlockKind,
    pub target_path: Option<String>,
    pub message: String,
}

pub fn format_ui_patch_block_error(prompt: &UiPatchPermissionPrompt) -> String {
    let json = serde_json::to_string(prompt).unwrap_or_else(|_| {
        format!(
            r#"{{"appId":"{}","appName":"{}","action":"{}","kind":"macos_app_management","targetPath":null,"message":"权限受限"}}"#,
            prompt.app_id.as_str(),
            prompt.app_name,
            prompt.action
        )
    });
    format!("{UI_PATCH_BLOCK_PREFIX}{json}|{}", prompt.message)
}

pub fn parse_ui_patch_block_error(err: &str) -> Option<UiPatchPermissionPrompt> {
    let idx = err.find(UI_PATCH_BLOCK_PREFIX)?;
    let rest = err[idx + UI_PATCH_BLOCK_PREFIX.len()..].trim_start();
    if !rest.starts_with('{') {
        return None;
    }

    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut end_idx = None;

    for (i, ch) in rest.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end_idx = Some(i + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }

    let json_slice = &rest[..end_idx?];
    serde_json::from_str::<UiPatchPermissionPrompt>(json_slice).ok()
}

pub fn make_msix_readonly_prompt(
    app_id: AppId,
    action: &str,
    target_path: Option<&Path>,
) -> UiPatchPermissionPrompt {
    let app_name = app_id.display_name().to_string();
    let target_str = target_path.map(|p| p.to_string_lossy().to_string());
    UiPatchPermissionPrompt {
        app_id,
        app_name: app_name.clone(),
        action: action.to_string(),
        kind: UiPatchBlockKind::WindowsMsixReadOnly,
        target_path: target_str,
        message: format!(
            "检测到 Microsoft Store (MSIX) 版 {app_name}：WindowsApps 目录受系统内核只读保护，无法注入界面资源补丁。配置文件级简体中文与中文回复已生效；如需完整界面汉化，建议改用官方 EXE 安装版。"
        ),
    }
}

pub fn classify_io_error_for_os(
    os: &str,
    app_id: AppId,
    action: &str,
    target_path: &Path,
    err: &std::io::Error,
) -> Option<UiPatchPermissionPrompt> {
    let os_norm = os.trim().to_ascii_lowercase();
    let app_name = app_id.display_name().to_string();
    let path_str = target_path.to_string_lossy().to_string();
    let action_label = if action == "restore" {
        "还原原版界面"
    } else {
        "注入界面汉化补丁"
    };

    if path_str.to_ascii_lowercase().contains("windowsapps")
        && (err.kind() == ErrorKind::PermissionDenied
            || matches!(err.raw_os_error(), Some(1 | 5 | 13)))
    {
        return Some(make_msix_readonly_prompt(app_id, action, Some(target_path)));
    }

    match os_norm.as_str() {
        "macos" => {
            if err.kind() == ErrorKind::PermissionDenied
                || matches!(err.raw_os_error(), Some(1 | 13))
            {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::MacosAppManagement,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "macOS 拦截了对 {app_name} 安装包的写入操作（{action_label}：{path_str}）。请前往「系统设置 -> 隐私与安全性 -> App 管理」开启 Tako Switch 授权后点击重试；配置文件级简体中文已正常生效。"
                    ),
                });
            }
            None
        }
        "windows" => {
            if err.raw_os_error() == Some(32) {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::WindowsFileLock,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "{app_name} 进程正在运行并占用资源包（{path_str}），无法{action_label}。请先退出 {app_name} 后重试；配置文件级简体中文已正常生效。"
                    ),
                });
            }
            if err.kind() == ErrorKind::PermissionDenied || err.raw_os_error() == Some(5) {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::WindowsUac,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "{action_label}需要 Windows 管理员权限（UAC）才能写入 {app_name} 安装目录（{path_str}）。请在授权弹窗中允许管理员权限或以管理员身份运行后重试；配置文件级简体中文已正常生效。"
                    ),
                });
            }
            None
        }
        _ => {
            if err.raw_os_error() == Some(32) {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::WindowsFileLock,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "{app_name} 进程正在占用资源包（{path_str}），无法{action_label}。"
                    ),
                });
            }
            if err.kind() == ErrorKind::PermissionDenied
                || matches!(err.raw_os_error(), Some(1 | 13))
            {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::MacosAppManagement,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "系统权限限制了对 {app_name} 安装包的写入操作（{action_label}：{path_str}）。请前往「系统设置 -> 隐私与安全性 -> App 管理」开启 Tako Switch 授权后重试；配置文件级简体中文已正常生效。"
                    ),
                });
            }
            if err.raw_os_error() == Some(5) {
                return Some(UiPatchPermissionPrompt {
                    app_id,
                    app_name: app_name.clone(),
                    action: action.to_string(),
                    kind: UiPatchBlockKind::WindowsUac,
                    target_path: Some(path_str.clone()),
                    message: format!(
                        "{action_label}需要管理员权限才能写入 {app_name} 安装目录（{path_str}）。"
                    ),
                });
            }
            None
        }
    }
}

fn map_bundle_io_err(
    app_id: AppId,
    action: &str,
    target_path: &Path,
    context: &str,
    err: std::io::Error,
) -> String {
    if let Some(prompt) = classify_io_error_for_os(std::env::consts::OS, app_id, action, target_path, &err) {
        return format_ui_patch_block_error(&prompt);
    }
    format!("{context} {} 失败: {err}", target_path.display())
}

/// Read `target_path` from a target bundle while classifying OS permission / sharing-violation errors
/// into structured `UI_PATCH_BLOCK_PREFIX` prompts.
pub(crate) fn read_bundle_file_checked(
    app_id: AppId,
    action: &str,
    target_path: &Path,
) -> Result<Vec<u8>, String> {
    fs::read(target_path)
        .map_err(|e| map_bundle_io_err(app_id, action, target_path, "读取目标文件", e))
}

/// Pre-flight check that both `target_path` (if it exists) and its parent directory are writable
/// before creating backups or mutating multi-file app bundles.
pub(crate) fn probe_bundle_write_access(
    app_id: AppId,
    action: &str,
    target_path: &Path,
) -> Result<(), String> {
    if target_path.exists() {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(target_path)
            .map_err(|e| map_bundle_io_err(app_id, action, target_path, "检查写入权限", e))?;
        drop(file);
    }
    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() && parent.exists() {
            let nonce: u32 = rand::random();
            let probe = parent.join(format!(".tako-perm-probe.{}.{nonce:08x}", std::process::id()));
            let f = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)
                .map_err(|e| map_bundle_io_err(app_id, action, target_path, "检查目录写入权限", e))?;
            drop(f);
            let _ = fs::remove_file(&probe);
        }
    }
    Ok(())
}

/// Atomically write `data` to `target_path` while classifying OS permission / sharing-violation errors
/// into structured `UI_PATCH_BLOCK_PREFIX` prompts.
pub(crate) fn write_bundle_file_checked(
    app_id: AppId,
    action: &str,
    target_path: &Path,
    data: &[u8],
    preserve_exec: bool,
) -> Result<(), String> {
    if target_path.exists() {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(target_path)
            .map_err(|e| map_bundle_io_err(app_id, action, target_path, "打开目标文件", e))?;
        drop(file);
    }

    #[cfg(unix)]
    let prev_mode = if preserve_exec {
        fs::metadata(target_path)
            .map(|m| m.permissions().mode())
            .ok()
    } else {
        None
    };
    #[cfg(not(unix))]
    let _ = preserve_exec;

    if let Some(parent) = target_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| map_bundle_io_err(app_id, action, target_path, "创建目标目录", e))?;
        }
    }

    let file_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("bundle.bin");
    let nonce: u32 = rand::random();
    let tmp_path = target_path.with_file_name(format!(
        ".{file_name}.tako-tmp.{}.{nonce:08x}",
        std::process::id()
    ));

    {
        let mut f = fs::File::create(&tmp_path)
            .map_err(|e| map_bundle_io_err(app_id, action, target_path, "创建临时文件", e))?;
        if let Err(e) = f.write_all(data).and_then(|()| f.sync_all()) {
            let _ = fs::remove_file(&tmp_path);
            return Err(map_bundle_io_err(app_id, action, target_path, "写入临时文件", e));
        }
    }

    #[cfg(unix)]
    if let Some(mode) = prev_mode {
        let exec_mode = if mode & 0o111 != 0 { mode } else { 0o755 };
        let _ = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(exec_mode));
    }

    if let Err(err) = fs::rename(&tmp_path, target_path) {
        #[cfg(windows)]
        {
            if target_path.exists() {
                if let Err(rm_err) = fs::remove_file(target_path) {
                    let _ = fs::remove_file(&tmp_path);
                    return Err(map_bundle_io_err(
                        app_id,
                        action,
                        target_path,
                        "覆盖目标文件",
                        rm_err,
                    ));
                }
                if fs::rename(&tmp_path, target_path).is_ok() {
                    return Ok(());
                }
            }
        }
        let _ = fs::remove_file(&tmp_path);
        return Err(map_bundle_io_err(
            app_id,
            action,
            target_path,
            "原子替换",
            err,
        ));
    }

    Ok(())
}

/// Inspect Electron FuseV1 wire inside an executable (`Claude.exe` / macOS binary)
/// and return `Some(true)` if `EnableEmbeddedAsarIntegrityValidation` (index 4) is `'1'`,
/// `Some(false)` if `'0'`, or `None` if no FuseV1 sentinel is found.
pub fn inspect_electron_asar_integrity_fuse(exe_bytes: &[u8]) -> Option<bool> {
    if exe_bytes.len() < ELECTRON_FUSE_V1_SENTINEL.len() + 7 {
        return None;
    }
    let sentinel_len = ELECTRON_FUSE_V1_SENTINEL.len();
    for idx in 0..=exe_bytes.len().saturating_sub(sentinel_len + 7) {
        if &exe_bytes[idx..idx + sentinel_len] != ELECTRON_FUSE_V1_SENTINEL {
            continue;
        }
        let ver = exe_bytes[idx + sentinel_len];
        let len = exe_bytes[idx + sentinel_len + 1] as usize;
        let fuse_start = idx + sentinel_len + 2;
        let Some(fuse_end) = fuse_start.checked_add(len) else {
            continue;
        };
        if ver != 1 || len <= 4 || fuse_end > exe_bytes.len() {
            continue;
        }
        let fuse_slice = &exe_bytes[fuse_start..fuse_end];
        return match fuse_slice[4] {
            b'1' => Some(true),
            b'0' => Some(false),
            _ => None,
        };
    }
    None
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Patch embedded 64-char SHA-256 ASAR header hashes in a Windows PE executable (`Claude.exe`)
/// when `EnableEmbeddedAsarIntegrityValidation` fuse is active, while zeroing `OptionalHeader.CheckSum`
/// and stripping Authenticode `DataDirectory[4]` (+ trailing certificate table when at EOF) so
/// Windows loader never fails with `STATUS_INVALID_IMAGE_HASH`.
pub fn patch_windows_pe_asar_integrity(
    exe_bytes: &[u8],
    hash_replacements: &[(&str, &str)],
) -> Result<Option<Vec<u8>>, String> {
    if inspect_electron_asar_integrity_fuse(exe_bytes) != Some(true) {
        return Ok(None);
    }

    for &(old_hash, new_hash) in hash_replacements {
        if !is_valid_sha256_hex(old_hash) {
            return Err(format!("无效的原始 ASAR SHA-256 哈希长度或格式: {old_hash}"));
        }
        if !is_valid_sha256_hex(new_hash) {
            return Err(format!("无效的新 ASAR SHA-256 哈希长度或格式: {new_hash}"));
        }
    }

    if exe_bytes.len() < 0x40 || &exe_bytes[0..2] != b"MZ" {
        return Err("非合法的 Windows PE 可执行文件（缺少 MZ 头）".to_string());
    }
    let e_lfanew = u32::from_le_bytes([
        exe_bytes[0x3c],
        exe_bytes[0x3d],
        exe_bytes[0x3e],
        exe_bytes[0x3f],
    ]) as usize;
    if e_lfanew
        .checked_add(24)
        .is_none_or(|end| end > exe_bytes.len())
        || &exe_bytes[e_lfanew..e_lfanew + 4] != b"PE\0\0"
    {
        return Err("非合法的 Windows PE 可执行文件（缺少 PE\\0\\0 签名）".to_string());
    }

    let coff_hdr = e_lfanew + 4;
    let size_of_opt =
        u16::from_le_bytes([exe_bytes[coff_hdr + 16], exe_bytes[coff_hdr + 17]]) as usize;
    let opt_hdr = e_lfanew + 24;
    if size_of_opt < 68
        || opt_hdr
            .checked_add(size_of_opt)
            .is_none_or(|end| end > exe_bytes.len())
    {
        return Err("Windows PE OptionalHeader 长度不足或越界".to_string());
    }

    let magic = u16::from_le_bytes([exe_bytes[opt_hdr], exe_bytes[opt_hdr + 1]]);
    let (num_rva_off, sec_dir_off) = match magic {
        0x010b => (opt_hdr + 92, opt_hdr + 128), // PE32
        0x020b => (opt_hdr + 108, opt_hdr + 144), // PE32+
        other => {
            return Err(format!(
                "不支持的 Windows PE OptionalHeader Magic: 0x{other:04x}"
            ));
        }
    };

    let mut out = exe_bytes.to_vec();

    for &(old_hash, new_hash) in hash_replacements {
        let old_bytes = old_hash.as_bytes();
        let new_lower = new_hash.to_ascii_lowercase();
        let new_bytes = new_lower.as_bytes();
        let mut matched_positions = Vec::new();
        let mut i = 0usize;
        while i + 64 <= out.len() {
            if out[i..i + 64].eq_ignore_ascii_case(old_bytes) {
                matched_positions.push(i);
                i += 64;
            } else {
                i += 1;
            }
        }
        if matched_positions.is_empty() {
            return Err(format!(
                "未在 Windows PE 资源中找到待替换的 ASAR 完整性哈希: {old_hash}"
            ));
        }
        for pos in matched_positions {
            out[pos..pos + 64].copy_from_slice(new_bytes);
        }
    }

    // Zero OptionalHeader.CheckSum (offset 64 in both PE32 and PE32+)
    out[opt_hdr + 64..opt_hdr + 68].copy_from_slice(&[0u8; 4]);

    // Zero DataDirectory[4] (IMAGE_DIRECTORY_ENTRY_SECURITY) and strip trailing Authenticode blob
    let opt_end = opt_hdr + size_of_opt;
    if num_rva_off + 4 <= opt_end && sec_dir_off + 8 <= opt_end {
        let num_rva = u32::from_le_bytes([
            out[num_rva_off],
            out[num_rva_off + 1],
            out[num_rva_off + 2],
            out[num_rva_off + 3],
        ]);
        if num_rva > 4 {
            let cert_file_offset = u32::from_le_bytes([
                out[sec_dir_off],
                out[sec_dir_off + 1],
                out[sec_dir_off + 2],
                out[sec_dir_off + 3],
            ]) as usize;
            let cert_size = u32::from_le_bytes([
                out[sec_dir_off + 4],
                out[sec_dir_off + 5],
                out[sec_dir_off + 6],
                out[sec_dir_off + 7],
            ]) as usize;
            out[sec_dir_off..sec_dir_off + 8].copy_from_slice(&[0u8; 8]);
            if cert_file_offset > 0
                && cert_size > 0
                && cert_file_offset.checked_add(cert_size) == Some(out.len())
            {
                out.truncate(cert_file_offset);
            }
        }
    }

    Ok(Some(out))
}

pub fn update_info_plist_asar_key_hash(
    plist_xml: &str,
    asar_rel_key: &str,
    new_header_sha256: &str,
) -> Option<String> {
    let asar_key = format!("<key>{asar_rel_key}</key>");
    let key_pos = plist_xml.find(&asar_key)?;
    let after_asar_key = key_pos + asar_key.len();
    let dict_end_rel = plist_xml[after_asar_key..].find("</dict>")?;
    let dict_end = after_asar_key + dict_end_rel;
    let dict_slice = &plist_xml[after_asar_key..dict_end];

    let hash_key = "<key>hash</key>";
    let hash_key_rel = dict_slice.find(hash_key)?;
    let after_hash_key = after_asar_key + hash_key_rel + hash_key.len();
    let sub_slice = &plist_xml[after_hash_key..dict_end];

    let str_open = "<string>";
    let str_close = "</string>";
    let open_rel = sub_slice.find(str_open)?;
    let val_start = after_hash_key + open_rel + str_open.len();
    let close_rel = plist_xml[val_start..dict_end].find(str_close)?;
    let val_end = val_start + close_rel;

    let mut updated = String::with_capacity(plist_xml.len());
    updated.push_str(&plist_xml[..val_start]);
    updated.push_str(new_header_sha256);
    updated.push_str(&plist_xml[val_end..]);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn build_synthetic_pe(is_pe32_plus: bool, fuse_char: Option<u8>, hashes: &[&str], cert: &[u8]) -> Vec<u8> {
        let e_lfanew = 0x80usize;
        let size_of_opt = if is_pe32_plus { 240usize } else { 224usize };
        let mut buf = vec![0u8; e_lfanew + 24 + size_of_opt + 64];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3c..0x40].copy_from_slice(&(e_lfanew as u32).to_le_bytes());
        buf[e_lfanew..e_lfanew + 4].copy_from_slice(b"PE\0\0");
        let coff = e_lfanew + 4;
        buf[coff + 16..coff + 18].copy_from_slice(&(size_of_opt as u16).to_le_bytes());
        let opt = e_lfanew + 24;
        let magic: u16 = if is_pe32_plus { 0x020b } else { 0x010b };
        buf[opt..opt + 2].copy_from_slice(&magic.to_le_bytes());
        // Non-zero checksum
        buf[opt + 64..opt + 68].copy_from_slice(&0xdead_beefu32.to_le_bytes());

        let (num_rva_off, sec_off) = if is_pe32_plus {
            (opt + 108, opt + 144)
        } else {
            (opt + 92, opt + 128)
        };
        buf[num_rva_off..num_rva_off + 4].copy_from_slice(&16u32.to_le_bytes());

        if let Some(fc) = fuse_char {
            buf.extend_from_slice(ELECTRON_FUSE_V1_SENTINEL);
            buf.push(1); // version 1
            buf.push(8); // length 8
            buf.extend_from_slice(&[b'0', b'0', b'1', b'0', fc, b'0', b'0', b'0']);
        }

        for h in hashes {
            buf.extend_from_slice(br#"{"file":"resources\\asar","value":""#);
            buf.extend_from_slice(h.as_bytes());
            buf.extend_from_slice(b"\"}");
        }

        if !cert.is_empty() {
            let cert_off = buf.len() as u32;
            let cert_len = cert.len() as u32;
            buf[sec_off..sec_off + 4].copy_from_slice(&cert_off.to_le_bytes());
            buf[sec_off + 4..sec_off + 8].copy_from_slice(&cert_len.to_le_bytes());
            buf.extend_from_slice(cert);
        }

        buf
    }

    #[test]
    fn block_error_roundtrip_and_os_classification() {
        let p = PathBuf::from("/Applications/Claude.app/Contents/Resources/app.asar");
        let eperm = std::io::Error::from_raw_os_error(1);
        let prompt = classify_io_error_for_os("macos", AppId::Claude, "patch", &p, &eperm).unwrap();
        assert_eq!(prompt.kind, UiPatchBlockKind::MacosAppManagement);
        assert_eq!(prompt.app_id, AppId::Claude);

        let formatted = format_ui_patch_block_error(&prompt);
        assert!(formatted.starts_with(UI_PATCH_BLOCK_PREFIX));
        let parsed = parse_ui_patch_block_error(&formatted).unwrap();
        assert_eq!(parsed, prompt);

        // Also parses cleanly when wrapped with outer Tauri context prefix
        let wrapped = format!("执行界面汉化失败: {formatted}");
        assert_eq!(parse_ui_patch_block_error(&wrapped).unwrap(), prompt);

        let win_path = PathBuf::from(r"C:\Users\demo\AppData\Local\AnthropicClaude\app-1.0.0\resources\app.asar");
        let lock_err = std::io::Error::from_raw_os_error(32);
        let lock_prompt =
            classify_io_error_for_os("windows", AppId::Claude, "patch", &win_path, &lock_err)
                .unwrap();
        assert_eq!(lock_prompt.kind, UiPatchBlockKind::WindowsFileLock);

        let uac_err = std::io::Error::from_raw_os_error(5);
        let uac_prompt =
            classify_io_error_for_os("windows", AppId::Antigravity, "restore", &win_path, &uac_err)
                .unwrap();
        assert_eq!(uac_prompt.kind, UiPatchBlockKind::WindowsUac);
    }

    #[test]
    fn windows_pe_fuse_and_authenticode_strip_pe32_and_pe32_plus() {
        let h_old_1 = &"A".repeat(64);
        let h_old_2 = &"b".repeat(64);
        let h_new_1 = &"1".repeat(64);
        let h_new_2 = &"2".repeat(64);
        let fake_cert = vec![0x30u8; 48];

        // Fuse = '0' -> untouched None
        let exe_off = build_synthetic_pe(true, Some(b'0'), &[h_old_1], &fake_cert);
        assert_eq!(inspect_electron_asar_integrity_fuse(&exe_off), Some(false));
        assert_eq!(
            patch_windows_pe_asar_integrity(&exe_off, &[(h_old_1, h_new_1)]).unwrap(),
            None
        );

        // No fuse -> None
        let exe_nofuse = build_synthetic_pe(false, None, &[h_old_1], &fake_cert);
        assert_eq!(inspect_electron_asar_integrity_fuse(&exe_nofuse), None);
        assert_eq!(
            patch_windows_pe_asar_integrity(&exe_nofuse, &[(h_old_1, h_new_1)]).unwrap(),
            None
        );

        // PE32+ and PE32 with Fuse = '1' -> hashes replaced, CheckSum=0, DataDirectory[4]=0, trailing cert truncated
        for is_64 in [true, false] {
            let exe_on = build_synthetic_pe(is_64, Some(b'1'), &[h_old_1, h_old_2], &fake_cert);
            assert_eq!(inspect_electron_asar_integrity_fuse(&exe_on), Some(true));
            let patched = patch_windows_pe_asar_integrity(
                &exe_on,
                &[(h_old_1, h_new_1), (h_old_2, h_new_2)],
            )
            .unwrap()
            .expect("must patch when fuse is enabled");

            assert_eq!(patched.len(), exe_on.len() - fake_cert.len());
            let opt = 0x80 + 24;
            assert_eq!(&patched[opt + 64..opt + 68], &[0, 0, 0, 0]);
            let sec_off = if is_64 { opt + 144 } else { opt + 128 };
            assert_eq!(&patched[sec_off..sec_off + 8], &[0u8; 8]);
            let patched_str = String::from_utf8_lossy(&patched);
            assert!(patched_str.contains(h_new_1));
            assert!(patched_str.contains(h_new_2));
            assert!(!patched_str.contains(h_old_1));
        }
    }
}
