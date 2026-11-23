use crate::adapters::asar_patch::{
    make_msix_readonly_prompt, probe_bundle_write_access, update_info_plist_asar_key_hash,
    write_bundle_file_checked,
};
pub use crate::adapters::asar_patch::{
    classify_io_error_for_os, format_ui_patch_block_error, inspect_electron_asar_integrity_fuse,
    parse_ui_patch_block_error, patch_windows_pe_asar_integrity, UiPatchBlockKind,
    UiPatchPermissionPrompt, UI_PATCH_BLOCK_PREFIX,
};
use crate::adapters::{AppId, DesktopAppAdapter};
use crate::config::{write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const PATCH_MARKER_FILE: &str = "__tako_i18n_patch.json";
pub const PATCH_GUARD_COMMENT: &str = "/* __TAKO_AGY_ZH_PATCH_V1__ */";
pub const CURRENT_UI_PATCH_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const ANTIGRAVITY_ZH_DICT_JSON: &str = include_str!("antigravity_zh_dict.json");
const ASAR_INTEGRITY_BLOCK_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiPatchStatus {
    NotApplicable,
    NotInstalled,
    Unpatched,
    Patched,
    OutdatedAfterUpgrade,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchInfo {
    pub status: UiPatchStatus,
    pub patch_version: Option<String>,
    pub has_valid_backup: bool,
    pub target_asar_path: Option<String>,
    pub note: Option<String>,
}

impl UiPatchInfo {
    pub fn not_applicable() -> Self {
        Self {
            status: UiPatchStatus::NotApplicable,
            patch_version: None,
            has_valid_backup: false,
            target_asar_path: None,
            note: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchMarkerPayload {
    schema: u32,
    patch_version: String,
    original_asar_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchMeta {
    schema: u32,
    patch_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
    original_asar_sha256: String,
    patched_asar_sha256: String,
}

#[derive(Debug, Clone)]
struct ResolvedAgyTarget {
    asar_path: PathBuf,
    bundle_root: Option<PathBuf>,
    info_plist_path: Option<PathBuf>,
    macos_bin_path: Option<PathBuf>,
    code_resources_path: Option<PathBuf>,
    app_version_hint: Option<String>,
}

enum TargetResolution {
    NotInstalled,
    MissingAsar { root: PathBuf },
    Found(ResolvedAgyTarget),
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn ui_patch_backup_dir(paths: &AppPaths) -> PathBuf {
    paths.backups_dir("antigravity").join("ui_patch")
}

fn resolve_target_from_dir(root: &Path, version_hint: Option<String>) -> TargetResolution {
    if !root.exists() {
        return TargetResolution::NotInstalled;
    }

    let is_macos_bundle = root
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        || root.join("Contents").join("Resources").join("app.asar").exists()
        || root.join("Contents").is_dir();

    if is_macos_bundle {
        let asar_path = root.join("Contents").join("Resources").join("app.asar");
        if !asar_path.is_file() {
            return TargetResolution::MissingAsar {
                root: root.to_path_buf(),
            };
        }
        return TargetResolution::Found(ResolvedAgyTarget {
            asar_path,
            bundle_root: Some(root.to_path_buf()),
            info_plist_path: Some(root.join("Contents").join("Info.plist")),
            macos_bin_path: Some(root.join("Contents").join("MacOS").join("Antigravity")),
            code_resources_path: Some(
                root.join("Contents")
                    .join("_CodeSignature")
                    .join("CodeResources"),
            ),
            app_version_hint: version_hint,
        });
    }

    let resources_asar = root.join("resources").join("app.asar");
    let direct_asar = root.join("app.asar");
    let asar_path = if resources_asar.is_file() {
        resources_asar
    } else if direct_asar.is_file() {
        direct_asar
    } else {
        return TargetResolution::MissingAsar {
            root: root.to_path_buf(),
        };
    };

    TargetResolution::Found(ResolvedAgyTarget {
        asar_path,
        bundle_root: None,
        info_plist_path: None,
        macos_bin_path: None,
        code_resources_path: None,
        app_version_hint: version_hint,
    })
}

fn resolve_antigravity_target(paths: &AppPaths) -> TargetResolution {
    if let Some(ref override_dir) = paths.agy_app_override_dir {
        return resolve_target_from_dir(override_dir, None);
    }

    // Never touch host /Applications/Antigravity.app in test sandboxes without explicit override.
    if paths.is_test {
        return TargetResolution::NotInstalled;
    }

    let install = super::antigravity::AntigravityAdapter.detect_install(paths);
    if !install.installed {
        return TargetResolution::NotInstalled;
    }
    let Some(raw_path) = install.install_path else {
        return TargetResolution::NotInstalled;
    };
    let path_buf = PathBuf::from(&raw_path);
    let base_dir = if path_buf.is_file() {
        path_buf
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(path_buf)
    } else {
        path_buf
    };
    resolve_target_from_dir(&base_dir, install.version)
}

/// Pure-Rust Electron ASAR v1 reader and writer with Chromium Pickle 4-byte alignment.
#[derive(Debug, Clone)]
pub struct AsarArchive {
    header: Value,
    packed_files: HashMap<String, Vec<u8>>,
    modified_files: HashSet<String>,
}

impl AsarArchive {
    #[cfg(test)]
    pub(crate) fn new_empty() -> Self {
        Self {
            header: json!({ "files": {} }),
            packed_files: HashMap::new(),
            modified_files: HashSet::new(),
        }
    }

    pub fn file_paths(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.packed_files.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 16 {
            return Err("ASAR 文件过短，不足 16 字节头部".to_string());
        }
        let u0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let u1 = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let _u2 = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let strlen = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;

        if u0 != 4 {
            return Err(format!("不支持的 ASAR Pickle 头部标识: {u0}"));
        }
        let data_offset = 8usize
            .checked_add(u1 as usize)
            .ok_or_else(|| "ASAR 头部长度溢出".to_string())?;
        let json_end = 16usize
            .checked_add(strlen)
            .ok_or_else(|| "ASAR JSON 长度溢出".to_string())?;
        if json_end > data_offset || data_offset > bytes.len() {
            return Err("ASAR 头部偏移越界或文件已截断".to_string());
        }

        let header_str = std::str::from_utf8(&bytes[16..json_end])
            .map_err(|e| format!("ASAR 头部 JSON 非合法 UTF-8: {e}"))?;
        let header: Value = serde_json::from_str(header_str)
            .map_err(|e| format!("解析 ASAR 头部 JSON 失败: {e}"))?;
        if header.get("files").and_then(Value::as_object).is_none() {
            return Err("ASAR 头部缺少根 files 对象".to_string());
        }

        let mut packed_files = HashMap::new();
        Self::collect_packed_files(&header, "", bytes, data_offset, &mut packed_files)?;

        Ok(Self {
            header,
            packed_files,
            modified_files: HashSet::new(),
        })
    }

    fn collect_packed_files(
        node: &Value,
        prefix: &str,
        archive_bytes: &[u8],
        data_offset: usize,
        out: &mut HashMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let Some(files_obj) = node.get("files").and_then(Value::as_object) else {
            return Ok(());
        };
        for (name, child) in files_obj {
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if child.get("files").is_some() {
                Self::collect_packed_files(child, &rel, archive_bytes, data_offset, out)?;
                continue;
            }
            if child.get("unpacked").and_then(Value::as_bool) == Some(true)
                || child.get("link").is_some()
            {
                continue;
            }
            let size_u64 = child
                .get("size")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("ASAR 条目 {rel} 缺少有效 size"))?;
            let offset_u64 = match child.get("offset") {
                Some(Value::String(s)) => s
                    .parse::<u64>()
                    .map_err(|e| format!("ASAR 条目 {rel} offset 非法: {e}"))?,
                Some(Value::Number(n)) => n
                    .as_u64()
                    .ok_or_else(|| format!("ASAR 条目 {rel} offset 非法数字"))?,
                _ => return Err(format!("ASAR 条目 {rel} 缺少 offset")),
            };
            let start = data_offset
                .checked_add(offset_u64 as usize)
                .ok_or_else(|| format!("ASAR 条目 {rel} 起始偏移溢出"))?;
            let end = start
                .checked_add(size_u64 as usize)
                .ok_or_else(|| format!("ASAR 条目 {rel} 结束偏移溢出"))?;
            if end > archive_bytes.len() {
                return Err(format!(
                    "ASAR 条目 {rel} 数据越界 ({end} > {})",
                    archive_bytes.len()
                ));
            }
            out.insert(rel, archive_bytes[start..end].to_vec());
        }
        Ok(())
    }

    pub fn get_file(&self, rel_path: &str) -> Option<&[u8]> {
        let norm = rel_path.trim_matches('/');
        self.packed_files.get(norm).map(Vec::as_slice)
    }

    pub fn set_file(&mut self, rel_path: &str, content: Vec<u8>) -> Result<(), String> {
        let norm = rel_path.trim_matches('/');
        let segments: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err("ASAR 文件路径不能为空".to_string());
        }

        let mut cursor = &mut self.header;
        for dir_name in &segments[..segments.len() - 1] {
            let files_map = cursor
                .get_mut("files")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| "ASAR 目录节点缺少 files 对象".to_string())?;
            cursor = files_map
                .entry((*dir_name).to_string())
                .or_insert_with(|| json!({ "files": {} }));
            if cursor.get("files").is_none() {
                *cursor = json!({ "files": {} });
            }
        }

        let leaf_name = segments[segments.len() - 1];
        let files_map = cursor
            .get_mut("files")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "ASAR 目标父目录缺少 files 对象".to_string())?;
        let existing_exec = files_map
            .get(leaf_name)
            .and_then(|n| n.get("executable"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut file_meta = Map::new();
        file_meta.insert("size".to_string(), json!(content.len() as u64));
        file_meta.insert("offset".to_string(), json!("0"));
        if existing_exec {
            file_meta.insert("executable".to_string(), json!(true));
        }
        file_meta.insert("integrity".to_string(), compute_asar_integrity(&content));
        files_map.insert(leaf_name.to_string(), Value::Object(file_meta));

        self.packed_files.insert(norm.to_string(), content);
        self.modified_files.insert(norm.to_string());
        Ok(())
    }

    pub fn pack(&self) -> Result<(Vec<u8>, String), String> {
        let mut header_clone = self.header.clone();
        let mut current_offset: u64 = 0;
        let mut ordered_paths: Vec<String> = Vec::with_capacity(self.packed_files.len());

        Self::rebuild_offsets_and_integrity(
            &mut header_clone,
            "",
            &self.packed_files,
            &self.modified_files,
            &mut current_offset,
            &mut ordered_paths,
        )?;

        let header_json = serde_json::to_string(&header_clone)
            .map_err(|e| format!("序列化 ASAR 头部 JSON 失败: {e}"))?;
        let strlen = header_json.len() as u32;
        let padded_len = (strlen + 3) & !3;
        let u0: u32 = 4;
        let u1: u32 = 8 + padded_len;
        let u2: u32 = 4 + padded_len;

        let total_len = 16usize + (padded_len as usize) + (current_offset as usize);
        let mut out = Vec::with_capacity(total_len);
        out.extend_from_slice(&u0.to_le_bytes());
        out.extend_from_slice(&u1.to_le_bytes());
        out.extend_from_slice(&u2.to_le_bytes());
        out.extend_from_slice(&strlen.to_le_bytes());
        out.extend_from_slice(header_json.as_bytes());
        let pad_bytes = (padded_len - strlen) as usize;
        if pad_bytes > 0 {
            out.resize(out.len() + pad_bytes, 0u8);
        }
        for rel in &ordered_paths {
            let bytes = self
                .packed_files
                .get(rel)
                .ok_or_else(|| format!("打包缺失文件载荷: {rel}"))?;
            out.extend_from_slice(bytes);
        }

        Ok((out, header_json))
    }

    fn rebuild_offsets_and_integrity(
        node: &mut Value,
        prefix: &str,
        packed_files: &HashMap<String, Vec<u8>>,
        modified_files: &HashSet<String>,
        current_offset: &mut u64,
        ordered_paths: &mut Vec<String>,
    ) -> Result<(), String> {
        let Some(files_obj) = node.get_mut("files").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        for (name, child) in files_obj.iter_mut() {
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if child.get("files").is_some() {
                Self::rebuild_offsets_and_integrity(
                    child,
                    &rel,
                    packed_files,
                    modified_files,
                    current_offset,
                    ordered_paths,
                )?;
                continue;
            }
            if child.get("unpacked").and_then(Value::as_bool) == Some(true)
                || child.get("link").is_some()
            {
                if let Some(obj) = child.as_object_mut() {
                    obj.remove("offset");
                }
                continue;
            }
            let bytes = packed_files
                .get(&rel)
                .ok_or_else(|| format!("ASAR 缺少已登记文件数据: {rel}"))?;
            let obj = child
                .as_object_mut()
                .ok_or_else(|| format!("ASAR 文件节点 {rel} 非对象"))?;
            obj.insert("size".to_string(), json!(bytes.len() as u64));
            obj.insert("offset".to_string(), json!(current_offset.to_string()));
            if modified_files.contains(&rel) {
                obj.insert("integrity".to_string(), compute_asar_integrity(bytes));
            }
            *current_offset += bytes.len() as u64;
            ordered_paths.push(rel);
        }
        Ok(())
    }
}

fn compute_asar_integrity(data: &[u8]) -> Value {
    let full_hash = sha256_hex(data);
    let blocks: Vec<String> = if data.len() <= ASAR_INTEGRITY_BLOCK_SIZE {
        vec![full_hash.clone()]
    } else {
        data.chunks(ASAR_INTEGRITY_BLOCK_SIZE)
            .map(sha256_hex)
            .collect()
    };
    json!({
        "algorithm": "SHA256",
        "hash": full_hash,
        "blockSize": ASAR_INTEGRITY_BLOCK_SIZE,
        "blocks": blocks,
    })
}

const MENU_LEAF_TRANSLATIONS: &[(&str, &str)] = &[
    ("New Window", "新建窗口"),
    ("New Tab", "新建标签页"),
    ("New Conversation", "新建对话"),
    ("Open...", "打开…"),
    ("Open Folder...", "打开文件夹…"),
    ("Open Workspace...", "打开工作区…"),
    ("Open Recent", "打开最近使用"),
    ("Clear Recently Opened", "清除最近打开记录"),
    ("Close Tab", "关闭标签页"),
    ("Close Window", "关闭窗口"),
    ("Settings...", "设置…"),
    ("Preferences...", "偏好设置…"),
    ("Undo", "撤销"),
    ("Redo", "重做"),
    ("Cut", "剪切"),
    ("Copy", "复制"),
    ("Paste", "粘贴"),
    ("Select All", "全选"),
    ("Actual Size", "实际大小"),
    ("Reset Zoom", "重置缩放"),
    ("Zoom In", "放大"),
    ("Zoom Out", "缩小"),
    ("Toggle Full Screen", "切换全屏"),
    ("Toggle Developer Tools", "切换开发者工具"),
    ("Reload", "重新加载"),
    ("Force Reload", "强制重新加载"),
    ("Minimize", "最小化"),
    ("Bring All to Front", "前置全部窗口"),
    ("Hide Antigravity", "隐藏 Antigravity"),
    ("Hide Others", "隐藏其他"),
    ("Show All", "显示全部"),
    ("Services", "服务"),
    ("Docs", "官方文档"),
    ("Documentation", "官方文档"),
    ("Release Notes", "发行说明"),
    ("Report Issue", "反馈问题"),
    ("Report an Issue...", "反馈问题…"),
    ("About Antigravity", "关于 Antigravity"),
    ("Connect to WSL", "连接到 WSL"),
    ("Connect to SSH...", "连接到 SSH…"),
    ("Connect to Remote...", "连接到远程主机…"),
    ("Reopen Locally", "在本地重新打开"),
    ("No agents running", "暂无运行中的智能体"),
    ("Show Antigravity", "显示 Antigravity"),
    ("Open Antigravity", "打开 Antigravity"),
    ("Quit Antigravity", "退出 Antigravity"),
    ("Quit", "退出"),
    ("Check for Updates", "检查更新"),
    ("Check for Updates...", "检查更新…"),
    ("Checking for Updates...", "正在检查更新…"),
    ("Download Update", "下载更新"),
    ("Downloading Update...", "正在下载更新…"),
    ("Restart to Update", "重启以更新"),
    ("Restart to Install Update", "重启以更新"),
    ("Up to Date", "已是最新版本"),
];

fn replace_menu_label_literals(src: &str) -> String {
    let mut out = src.to_string();
    for &(en, zh) in MENU_LEAF_TRANSLATIONS {
        // Replace `label: 'En'` and `label: "En"` without touching top-level 'File'|'Edit'|'View'|'Window'|'Help'
        for quote in ['\'', '"'] {
            let from_spaced = format!("label: {quote}{en}{quote}");
            let to_spaced = format!("label: {quote}{zh}{quote}");
            out = out.replace(&from_spaced, &to_spaced);

            let from_compact = format!("label:{quote}{en}{quote}");
            let to_compact = format!("label:{quote}{zh}{quote}");
            out = out.replace(&from_compact, &to_compact);
        }
    }
    // Also translate standalone status strings like 'No agents running' when not followed by ':'
    for &(en, zh) in &[
        ("No agents running", "暂无运行中的智能体"),
        ("Connect to WSL", "连接到 WSL"),
        ("Reopen Locally", "在本地重新打开"),
    ] {
        for quote in ['\'', '"'] {
            let lit = format!("{quote}{en}{quote}");
            let rep = format!("{quote}{zh}{quote}");
            out = replace_non_property_key_literal(&out, &lit, &rep);
        }
    }
    out
}

fn patch_updater_dialog_literals(src: &str) -> String {
    let mut out = src.to_string();
    for &(from, to) in &[
        ("title: 'Check for Updates'", "title: '检查更新'"),
        ("title: \"Check for Updates\"", "title: \"检查更新\""),
        ("message: 'No updates available'", "message: '暂无可用更新'"),
        (
            "message: \"No updates available\"",
            "message: \"暂无可用更新\"",
        ),
        ("buttons: ['OK']", "buttons: ['确定']"),
        ("buttons: [\"OK\"]", "buttons: [\"确定\"]"),
    ] {
        out = out.replace(from, to);
    }
    out
}

fn patch_tray_dynamic_count(src: &str) -> String {
    const TRAY_COUNT_EXPR: &str = "(count > 0 ? `${count}` : 'No') +\n                    ' agent' +\n                    (count === 1 ? '' : 's') +\n                    ' running'";
    const TRAY_COUNT_ZH: &str = "count > 0 ? `${count} 个智能体运行中` : '暂无运行中的智能体'";
    let out = src.replace(TRAY_COUNT_EXPR, TRAY_COUNT_ZH);
    let compact_expr = "(count > 0 ? `${count}` : 'No') + ' agent' + (count === 1 ? '' : 's') + ' running'";
    out.replace(compact_expr, TRAY_COUNT_ZH)
}

fn replace_non_property_key_literal(src: &str, needle: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(src.len() + 64);
    let mut rest = src;
    while let Some(pos) = rest.find(needle) {
        result.push_str(&rest[..pos]);
        let after = &rest[pos + needle.len()..];
        let trimmed_after = after.trim_start_matches([' ', '\t']);
        if trimmed_after.starts_with(':') {
            result.push_str(needle);
        } else {
            result.push_str(replacement);
        }
        rest = after;
    }
    result.push_str(rest);
    result
}

fn find_matching_js_brace(src: &str, open_idx: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if bytes.get(open_idx).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open_idx;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(q) = quote {
            if ch == b'\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == b'\'' || ch == b'"' || ch == b'`' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if ch == b'{' {
            depth += 1;
        } else if ch == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn wrap_update_actions_objects(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 128);
    let mut cursor = 0usize;
    let marker = "updateActions";

    while let Some(rel) = src[cursor..].find(marker) {
        let m_idx = cursor + rel;
        let after_name = m_idx + marker.len();
        let rest = &src[after_name..];
        let ws1 = rest.len() - rest.trim_start_matches([' ', '\t', '\r', '\n']).len();
        let after_ws1 = after_name + ws1;

        if src.as_bytes().get(after_ws1).copied() == Some(b'=')
            && src.as_bytes().get(after_ws1 + 1).copied() != Some(b'=')
        {
            let rest2 = &src[after_ws1 + 1..];
            let ws2 = rest2.len() - rest2.trim_start_matches([' ', '\t', '\r', '\n']).len();
            let brace_idx = after_ws1 + 1 + ws2;
            if let Some(close_idx) = find_matching_js_brace(src, brace_idx) {
                out.push_str(&src[cursor..brace_idx]);
                out.push_str("__takoBindZhUpdateActions(");
                out.push_str(&src[brace_idx..=close_idx]);
                out.push(')');
                cursor = close_idx + 1;
                continue;
            }
        }

        out.push_str(&src[cursor..after_name]);
        cursor = after_name;
    }
    out.push_str(&src[cursor..]);
    out
}

fn build_electron_menu_runtime_hook() -> String {
    let mut map_entries = String::new();
    for &(en, zh) in MENU_LEAF_TRANSLATIONS {
        map_entries.push_str(&format!("    {:?}: {:?},\n", en, zh));
    }
    format!(
        r#"{PATCH_GUARD_COMMENT}
function __takoBindZhUpdateActions(obj) {{
  if (!obj || typeof obj !== "object") return obj;
  var pairs = [
    ["Check for Updates", "检查更新"],
    ["Check for Updates...", "检查更新…"],
    ["Checking for Updates...", "正在检查更新…"],
    ["Download Update", "下载更新"],
    ["Downloading Update...", "正在下载更新…"],
    ["Restart to Update", "重启以更新"],
    ["Restart to Install Update", "重启以更新"],
    ["Up to Date", "已是最新版本"]
  ];
  for (var i = 0; i < pairs.length; i++) {{
    var en = pairs[i][0], zh = pairs[i][1];
    if (Object.prototype.hasOwnProperty.call(obj, en) && !Object.prototype.hasOwnProperty.call(obj, zh)) {{
      obj[zh] = obj[en];
    }}
    if (Object.prototype.hasOwnProperty.call(obj, zh) && !Object.prototype.hasOwnProperty.call(obj, en)) {{
      obj[en] = obj[zh];
    }}
  }}
  return obj;
}}
;(function () {{
  try {{
    var electron = typeof require === "function" ? require("electron") : null;
    if (!electron || !electron.Menu || electron.Menu.__takoZhHooked) return;
    var TOP_KEEP = {{ File: 1, Edit: 1, View: 1, Window: 1, Help: 1 }};
    var MAP = Object.assign(Object.create(null), {{
{map_entries}    }});
    function translateTemplate(items, isTop) {{
      if (!Array.isArray(items)) return items;
      return items.map(function (item) {{
        if (!item || typeof item !== "object") return item;
        var copy = Object.assign({{}}, item);
        if (typeof copy.label === "string" && !(isTop && TOP_KEEP[copy.label])) {{
          var trimmed = copy.label.trim();
          if (MAP[trimmed]) copy.label = MAP[trimmed];
        }}
        if (Array.isArray(copy.submenu)) {{
          copy.submenu = translateTemplate(copy.submenu, false);
        }}
        return copy;
      }});
    }}
    var origBuild = electron.Menu.buildFromTemplate.bind(electron.Menu);
    electron.Menu.buildFromTemplate = function (template) {{
      return origBuild(translateTemplate(template, true));
    }};
    electron.Menu.__takoZhHooked = true;
  }} catch (_) {{}}
}})();
"#
    )
}

fn patch_main_process_js(raw_js: &str, is_updater: bool) -> String {
    if raw_js.contains(PATCH_GUARD_COMMENT) {
        return raw_js.to_string();
    }
    let mut transformed = replace_menu_label_literals(raw_js);
    transformed = patch_tray_dynamic_count(&transformed);
    if is_updater {
        transformed = patch_updater_dialog_literals(&transformed);
        transformed = wrap_update_actions_objects(&transformed);
    }
    let hook = build_electron_menu_runtime_hook();
    format!("{hook}\n{transformed}\n")
}

fn patch_preload_js(raw_js: &str) -> String {
    if raw_js.contains(PATCH_GUARD_COMMENT) {
        return raw_js.to_string();
    }
    let iife = build_preload_iife();
    if raw_js.ends_with('\n') {
        format!("{raw_js}\n{iife}\n")
    } else {
        format!("{raw_js}\n\n{iife}\n")
    }
}

fn build_preload_iife() -> String {
    let safe_dict_json = ANTIGRAVITY_ZH_DICT_JSON
        .trim()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
        .replace("</script", "<\\/script");

    let mut out = String::with_capacity(safe_dict_json.len() + 12_000);
    out.push_str(PATCH_GUARD_COMMENT);
    out.push_str(
        r#"
;(function () {
  try {
    if (typeof window === "undefined" || typeof document === "undefined") return;
    var host = (window.location && window.location.hostname) || "";
    if (host !== "127.0.0.1" && host !== "localhost") return;
    if (window.__TAKO_AGY_ZH_PATCHED__) return;
    window.__TAKO_AGY_ZH_PATCHED__ = true;

    var DICT = Object.assign(Object.create(null), "#,
    );
    out.push_str(&safe_dict_json);
    out.push_str(
        r#");

    var SKIP_TAGS = {
      SCRIPT: 1,
      STYLE: 1,
      CODE: 1,
      PRE: 1,
      TEXTAREA: 1,
      INPUT: 1,
      KBD: 1,
      SAMP: 1,
      SVG: 1,
      MATH: 1,
      NOSCRIPT: 1
    };
    var SKIP_TEXT_SELECTOR =
      'script, style, code, pre, textarea, input, kbd, samp, svg, math, noscript, [contenteditable="true"], [contenteditable=""], [role="textbox"], [data-monaco-editor], .monaco-editor, .xterm, [data-no-i18n]';
    var SKIP_ATTR_SELECTOR =
      'script, style, code, pre, kbd, samp, svg, math, noscript, [data-monaco-editor], .monaco-editor, .xterm, [data-no-i18n]';
    var CHAT_BODY_SELECTOR =
      '.markdown-body, .prose, [data-message-role], [data-testid*="message-content"], [data-testid*="markdown"], .message-content, .chat-markdown';
    var ATTRS = ["placeholder", "title", "aria-label"];
    var MAX_RAW_LEN = 560;
    var MAX_TRIMMED_LEN = 520;
    // Ultra-short single words that frequently collide with split React text nodes
    // (e.g. `<p>{"No "}{plural}{" yet"}</p>` -> "否 automations yet") when translated in isolation.
    var AMBIGUOUS_SINGLE_WORDS = {
      No: 1,
      Yes: 1,
      On: 1,
      Off: 1,
      All: 1,
      Any: 1,
      To: 1,
      New: 1,
      Add: 1,
      Run: 1,
      Key: 1,
      Tag: 1,
      Low: 1,
      Max: 1,
      Raw: 1,
      Now: 1,
      Pin: 1,
      Top: 1,
      Cut: 1,
      Ask: 1,
      Pro: 1,
      End: 1,
      Use: 1,
      See: 1,
      Hold: 1,
      This: 1
    };

    function hasCjk(str) {
      return /[\u4e00-\u9fff]/.test(str);
    }

    function looksLikeCodeOrPath(str) {
      if (!str) return false;
      if (/https?:\/\/|file:\/\/|ssh:\/\//i.test(str)) return true;
      if (/^(\/|[A-Za-z]:\\|~\/)[^\s]+/.test(str)) return true;
      if (/\b(?:function|const|let|var|import|export|return|class|interface|struct|impl|def)\b\s+\w/.test(str)) {
        return true;
      }
      if (/=>|::|<\/?[a-zA-Z][\w-]*[\s>]|^\s*[\{\[\(].*[\}\]\)]\s*$/.test(str)) return true;
      if (/[\\;]/.test(str) && !/\s/.test(str)) return true;
      return false;
    }

    function lookupDict(trimmed) {
      var direct = DICT[trimmed];
      if (direct) return direct;
      if (trimmed.length > 2) {
        if (trimmed.endsWith(":") && !trimmed.endsWith("://")) {
          var baseColon = trimmed.slice(0, -1).trim();
          if (baseColon && DICT[baseColon]) return DICT[baseColon] + "：";
        } else if (trimmed.endsWith("...")) {
          var baseDots = trimmed.slice(0, -3).trim();
          if (baseDots && DICT[baseDots]) return DICT[baseDots] + "...";
        } else if (trimmed.endsWith("…")) {
          var baseEllipsis = trimmed.slice(0, -1).trim();
          if (baseEllipsis && DICT[baseEllipsis]) return DICT[baseEllipsis] + "…";
        }
      }
      return null;
    }

    function translateShortNoun(phrase) {
      var trimmed = phrase.trim();
      if (!trimmed) return phrase;
      var exact = DICT[trimmed];
      if (exact) return exact;
      var cap = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
      if (DICT[cap]) return DICT[cap];
      return trimmed;
    }

    var DYNAMIC_RULES = [
      // Timing & thinking status
      [/^Thinking for\s+([0-9]+(?:\.[0-9]+)?\s*(?:ms|s|m|h|sec|secs|second|seconds|min|mins|minute|minutes)(?:\s+[0-9]+\s*(?:s|m))?)$/i, function (m) { return "正在思考 " + m[1]; }],
      [/^Thought for\s+([0-9]+(?:\.[0-9]+)?\s*(?:ms|s|m|h|sec|secs|second|seconds|min|mins|minute|minutes)(?:\s+[0-9]+\s*(?:s|m))?)$/i, function (m) { return "已思考 " + m[1]; }],
      [/^Worked for\s+([0-9]+(?:\.[0-9]+)?\s*(?:ms|s|m|h|sec|secs|second|seconds|min|mins|minute|minutes)(?:\s+[0-9]+\s*(?:s|m))?)$/i, function (m) { return "已执行 " + m[1]; }],
      [/^Running for\s+([0-9]+(?:\.[0-9]+)?\s*(?:ms|s|m|h|sec|secs|second|seconds|min|mins|minute|minutes)(?:\s+[0-9]+\s*(?:s|m))?)$/i, function (m) { return "已运行 " + m[1]; }],
      [/^Still thinking,\s*(\d+)\s+seconds?$/i, function (m) { return "仍在思考，已用 " + m[1] + " 秒"; }],

      // Relative time
      [/^(\d+)\s*s\s+ago$/i, function (m) { return m[1] + " 秒前"; }],
      [/^(\d+)\s*m\s+ago$/i, function (m) { return m[1] + " 分钟前"; }],
      [/^(\d+)\s*h\s+ago$/i, function (m) { return m[1] + " 小时前"; }],
      [/^(\d+)\s*d\s+ago$/i, function (m) { return m[1] + " 天前"; }],
      [/^(\d+)\s+seconds?\s+ago$/i, function (m) { return m[1] + " 秒前"; }],
      [/^(\d+)\s+minutes?\s+ago$/i, function (m) { return m[1] + " 分钟前"; }],
      [/^(\d+)\s+hours?\s+ago$/i, function (m) { return m[1] + " 小时前"; }],
      [/^(\d+)\s+days?\s+ago$/i, function (m) { return m[1] + " 天前"; }],
      [/^(\d+)\s+weeks?\s+ago$/i, function (m) { return m[1] + " 周前"; }],
      [/^(\d+)\s+months?\s+ago$/i, function (m) { return m[1] + " 个月前"; }],
      [/^(\d+)\s+years?\s+ago$/i, function (m) { return m[1] + " 年前"; }],
      [/^Resets in\s+([<>0-9a-zA-Z\s]{1,20})$/, function (m) { return "将在 " + m[1].trim() + " 后重置"; }],

      // File & diff statistics
      [/^(\d+)\s+files?\s+changed$/i, function (m) { return "已变更 " + m[1] + " 个文件"; }],
      [/^(\d+)\s+lines?\s+added$/i, function (m) { return "新增 " + m[1] + " 行"; }],
      [/^(\d+)\s+lines?\s+(?:removed|deleted)$/i, function (m) { return "删除 " + m[1] + " 行"; }],
      [/^Showing lines\s+(\d+)\s*[-–]\s*(\d+)\s+of\s+(\d+)$/i, function (m) { return "显示第 " + m[1] + "-" + m[2] + " 行（共 " + m[3] + " 行）"; }],
      [/^Unfold lines\s+(\d+)\s*[-–]\s*(\d+)$/i, function (m) { return "展开第 " + m[1] + "-" + m[2] + " 行"; }],
      [/^Fold lines\s+(\d+)\s*[-–]\s*(\d+)$/i, function (m) { return "折叠第 " + m[1] + "-" + m[2] + " 行"; }],
      [/^Showing first\s+(\d+)\s+references?$/i, function (m) { return "显示前 " + m[1] + " 条引用"; }],
      [/^Showing latest\s+(\d+)\s+browser steps?$/i, function (m) { return "显示最近 " + m[1] + " 个浏览器步骤"; }],

      // Steps, turns & pagination
      [/^Plan,\s*(\d+)\s+of\s+(\d+)\s+steps?\s+done$/i, function (m) { return "计划：已完成 " + m[1] + " / " + m[2] + " 步"; }],
      [/^Step\s+(\d+)\s+of\s+(\d+)$/i, function (m) { return "第 " + m[1] + " / " + m[2] + " 步"; }],
      [/^Turn\s+(\d+)\s+of\s+(\d+)$/i, function (m) { return "第 " + m[1] + " / " + m[2] + " 轮"; }],
      [/^Go to turn\s+(\d+)\s+of\s+(\d+)$/i, function (m) { return "跳转到第 " + m[1] + " / " + m[2] + " 轮"; }],
      [/^Page\s+(\d+)\s+of\s+(\d+)$/i, function (m) { return "第 " + m[1] + " / " + m[2] + " 页"; }],
      [/^Show\s+(\d+)\s+earlier\s+steps?$/i, function (m) { return "展开更早的 " + m[1] + " 个步骤"; }],
      [/^Show\s+(\d+)\s+more\s+subagents?$/i, function (m) { return "展开另外 " + m[1] + " 个子智能体"; }],
      [/^(?:Show|See)\s+(\d+)\s+more$/i, function (m) { return "显示另外 " + m[1] + " 项"; }],

      // Counts & badges
      [/^(\d+)\s+tool calls?$/i, function (m) { return m[1] + " 次工具调用"; }],
      [/^(\d+)\s+tools?$/i, function (m) { return m[1] + " 个工具"; }],
      [/^(\d+)\s+tasks?$/i, function (m) { return m[1] + " 个任务"; }],
      [/^(\d+)\s+conversations?$/i, function (m) { return m[1] + " 个对话"; }],
      [/^(\d+)\s+servers?$/i, function (m) { return m[1] + " 个服务"; }],
      [/^(\d+)\s+subagents?$/i, function (m) { return m[1] + " 个子智能体"; }],
      [/^(\d+)\s+skills?$/i, function (m) { return m[1] + " 项技能"; }],
      [/^(\d+)\s+rules?$/i, function (m) { return m[1] + " 条规则"; }],
      [/^(\d+)\s+matches?$/i, function (m) { return m[1] + " 个匹配"; }],
      [/^(\d+)\s+results?$/i, function (m) { return m[1] + " 条结果"; }],
      [/^(\d+)\s+selected$/i, function (m) { return "已选 " + m[1] + " 项"; }],
      [/^(\d+)\s+unread$/i, function (m) { return m[1] + " 条未读"; }],
      [/^(\d+)\s+active$/i, function (m) { return m[1] + " 个活跃"; }],
      [/^(\d+)\s+running$/i, function (m) { return m[1] + " 个运行中"; }],
      [/^(\d+)\s+agents?\s+running$/i, function (m) { return m[1] + " 个智能体运行中"; }],
      [/^(\d+)\s+plugins?$/i, function (m) { return m[1] + " 个插件"; }],
      [/^(\d+)\s+hooks?$/i, function (m) { return m[1] + " 个钩子"; }],
      [/^(\d+)\s+comments?$/i, function (m) { return m[1] + " 条评论"; }],
      [/^(\d+)\s+files?$/i, function (m) { return m[1] + " 个文件"; }],
      [/^(\d+)\s+workspaces?$/i, function (m) { return m[1] + " 个工作区"; }],
      [/^(\d+)\s+errors?$/i, function (m) { return m[1] + " 个错误"; }],
      [/^(\d+)\s+warnings?$/i, function (m) { return m[1] + " 条警告"; }],

      // Action summaries & safe short verb phrases
      [/^Edited\s+(\d+)\s+files?$/i, function (m) { return "已编辑 " + m[1] + " 个文件"; }],
      [/^Read\s+(\d+)\s+files?$/i, function (m) { return "已读取 " + m[1] + " 个文件"; }],
      [/^Created\s+(\d+)\s+files?$/i, function (m) { return "已创建 " + m[1] + " 个文件"; }],
      [/^Deleted\s+(\d+)\s+files?$/i, function (m) { return "已删除 " + m[1] + " 个文件"; }],
      [/^Marked\s+(\d+)\s+conversations?\s+as\s+read$/i, function (m) { return "已将 " + m[1] + " 个对话标为已读"; }],
      [/^Stop\s+(\d+)\s+selected$/i, function (m) { return "停止已选的 " + m[1] + " 项"; }],
      [/^Search\s+([^\n]{1,40})(\.{3}|…)$/, function (m) {
        if (looksLikeCodeOrPath(m[1])) return null;
        return "搜索 " + translateShortNoun(m[1]) + m[2];
      }],
      [/^Running\s+([^\n]{1,40})(\.{3}|…)$/, function (m) {
        if (looksLikeCodeOrPath(m[1])) return null;
        return "正在运行 " + translateShortNoun(m[1]) + m[2];
      }],
      [/^Open Subagent:\s*([^\n]{1,48})$/, function (m) {
        if (looksLikeCodeOrPath(m[1])) return null;
        return "打开子智能体：" + m[1].trim();
      }],
      [/^Used MCP tool:\s*([^\n]{1,48})$/, function (m) {
        return "已调用 MCP 工具：" + m[1].trim();
      }],
      [/^Used tool:\s*([^\n]{1,48})$/, function (m) {
        return "已调用工具：" + m[1].trim();
      }],
      [/^Invoked subagent:\s*([^\n]{1,48})$/, function (m) {
        return "已调用子智能体：" + m[1].trim();
      }],
      [/^Forked from\s+([^\n]{1,48})$/, function (m) {
        if (looksLikeCodeOrPath(m[1])) return null;
        return "分支自 " + m[1].trim();
      }],
      [/^No\s+([a-zA-Z][a-zA-Z0-9\s-]{1,36})\s+yet$/i, function (m) {
        if (looksLikeCodeOrPath(m[1])) return null;
        var noun = translateShortNoun(m[1]);
        return noun === m[1].trim() ? "暂无 " + noun : "暂无" + noun;
      }],
      [/^No\s+sites\s+match\s+[“"]([^”"\n]{1,80})[”"]$/i, function (m) {
        return "没有站点匹配“" + m[1] + "”";
      }],
      [/^Couldn't\s+delete\s+cookies\s+for\s+([^\n]{1,80})$/i, function (m) {
        return "无法删除 " + m[1].trim() + " 的 Cookie";
      }],
      [/^Site\s+data\s+cleared\s+for\s+([^\n]{1,80})$/i, function (m) {
        return "已清除 " + m[1].trim() + " 的站点数据";
      }],
      [/^Could\s+not\s+clear\s+site\s+data\s+for\s+([^\n]{1,80})$/i, function (m) {
        return "无法清除 " + m[1].trim() + " 的站点数据";
      }]
    ];

    function translateText(trimmed) {
      if (!trimmed || trimmed.length > MAX_TRIMMED_LEN) return null;
      var exact = lookupDict(trimmed);
      if (exact) return exact;
      if (hasCjk(trimmed)) return null;
      if (looksLikeCodeOrPath(trimmed)) return null;
      for (var i = 0; i < DYNAMIC_RULES.length; i++) {
        var rule = DYNAMIC_RULES[i];
        var m = rule[0].exec(trimmed);
        if (m) {
          var out = rule[1](m);
          if (out) return out;
        }
      }
      return null;
    }

    function isSkippedForText(el) {
      if (!el || el.nodeType !== 1) return true;
      if (SKIP_TAGS[el.tagName]) return true;
      if (typeof el.closest === "function") {
        if (el.closest(SKIP_TEXT_SELECTOR)) return true;
        if (el.closest(CHAT_BODY_SELECTOR) && !el.closest('button, [role="button"]')) {
          return true;
        }
      }
      return false;
    }

    function isSkippedForAttrs(el) {
      if (!el || el.nodeType !== 1) return true;
      if (SKIP_TAGS[el.tagName] && el.tagName !== "INPUT" && el.tagName !== "TEXTAREA") return true;
      if (typeof el.closest === "function") {
        if (el.closest(SKIP_ATTR_SELECTOR)) return true;
        if (el.closest(CHAT_BODY_SELECTOR) && !el.closest('button, [role="button"]')) {
          return true;
        }
      }
      return false;
    }

    function hasNonEmptySibling(node, parent) {
      var children = parent.childNodes;
      if (!children || children.length <= 1) return false;
      for (var i = 0; i < children.length; i++) {
        var sib = children[i];
        if (sib === node) continue;
        if (sib.nodeType === 3) {
          if (sib.nodeValue && sib.nodeValue.trim().length > 0) return true;
        } else if (sib.nodeType === 1) {
          var tag = sib.tagName;
          if (tag !== "SVG" && tag !== "IMG" && tag !== "BR") {
            if ((sib.textContent || "").trim().length > 0) return true;
          }
        }
      }
      return false;
    }

    // React often splits interpolated strings like `<p>{"No "}{plural}{" yet"}</p>` into multiple
    // adjacent Text nodes. Translate the combined sentence while keeping every Text node attached
    // so React Fiber reconciliation never throws NotFoundError on future updates.
    function translateCombinedTextNodes(el) {
      if (!el || el.nodeType !== 1 || isSkippedForText(el)) return false;
      var children = el.childNodes;
      if (!children || children.length < 2) return false;
      var combined = "";
      var firstNonEmpty = null;
      for (var i = 0; i < children.length; i++) {
        var c = children[i];
        if (c.nodeType !== 3) return false;
        var val = c.nodeValue || "";
        combined += val;
        if (!firstNonEmpty && val.trim().length > 0) {
          firstNonEmpty = c;
        }
      }
      if (!firstNonEmpty || combined.length > MAX_RAW_LEN) return false;
      var trimmed = combined.trim();
      if (!trimmed || trimmed.length > MAX_TRIMMED_LEN) return false;
      var zh = translateText(trimmed);
      if (!zh) return false;
      var idx = combined.indexOf(trimmed);
      if (idx < 0) return false;
      var fullNext = combined.slice(0, idx) + zh + combined.slice(idx + trimmed.length);
      if (fullNext === combined) return true;
      for (var j = 0; j < children.length; j++) {
        var tn = children[j];
        var targetVal = tn === firstNonEmpty ? fullNext : "";
        if (tn.nodeValue !== targetVal) {
          tn.nodeValue = targetVal;
        }
      }
      return true;
    }

    function translateTextNode(node) {
      if (!node || node.nodeType !== 3) return;
      var parent = node.parentElement;
      if (!parent || isSkippedForText(parent)) return;
      if (parent.childNodes && parent.childNodes.length >= 2 && translateCombinedTextNodes(parent)) {
        return;
      }
      var raw = node.nodeValue;
      if (!raw || raw.length > MAX_RAW_LEN) return;
      var trimmed = raw.trim();
      if (!trimmed || trimmed.length > MAX_TRIMMED_LEN) return;
      if (AMBIGUOUS_SINGLE_WORDS[trimmed] && hasNonEmptySibling(node, parent)) {
        return;
      }
      var zh = translateText(trimmed);
      if (!zh) return;
      var idx = raw.indexOf(trimmed);
      if (idx < 0) return;
      var next = raw.slice(0, idx) + zh + raw.slice(idx + trimmed.length);
      if (next !== raw) {
        node.nodeValue = next;
      }
    }

    function translateElementAttrs(el) {
      if (!el || el.nodeType !== 1 || isSkippedForAttrs(el)) return;
      for (var i = 0; i < ATTRS.length; i++) {
        var attr = ATTRS[i];
        var val = el.getAttribute(attr);
        if (!val || val.length > MAX_RAW_LEN) continue;
        var trimmed = val.trim();
        var zh = translateText(trimmed);
        if (zh && zh !== val) {
          el.setAttribute(attr, zh);
        }
      }
    }

    var observedRoots = typeof WeakSet === "function" ? new WeakSet() : null;
    var pendingRoots = new Set();
    var scheduled = false;

    function flushPending() {
      scheduled = false;
      try {
        pendingRoots.forEach(function (n) {
          walkSubtree(n);
        });
      } catch (_) {
      } finally {
        pendingRoots.clear();
      }
    }

    function scheduleNode(n) {
      if (!n) return;
      pendingRoots.add(n);
      if (!scheduled) {
        scheduled = true;
        if (typeof window.requestAnimationFrame === "function") {
          window.requestAnimationFrame(flushPending);
        } else {
          setTimeout(flushPending, 16);
        }
      }
    }

    function observeRoot(root) {
      if (!root) return;
      if (observedRoots) {
        if (observedRoots.has(root)) return;
        observedRoots.add(root);
      }
      try {
        var obs = new MutationObserver(function (mutations) {
          try {
            for (var i = 0; i < mutations.length; i++) {
              var m = mutations[i];
              if (m.type === "childList") {
                for (var j = 0; j < m.addedNodes.length; j++) {
                  scheduleNode(m.addedNodes[j]);
                }
              } else if (m.type === "characterData") {
                scheduleNode(m.target);
              } else if (m.type === "attributes") {
                translateElementAttrs(m.target);
              }
            }
          } catch (_) {}
        });
        obs.observe(root, {
          childList: true,
          subtree: true,
          characterData: true,
          attributes: true,
          attributeFilter: ATTRS
        });
      } catch (_) {}
    }

    function tryAttachIframe(iframeEl) {
      try {
        var doc = iframeEl.contentDocument;
        if (!doc || !doc.body) return;
        var h = (doc.location && doc.location.hostname) || "";
        if (h !== "127.0.0.1" && h !== "localhost") return;
        observeRoot(doc.body);
        walkSubtree(doc.body);
      } catch (_) {}
    }

    function walkSubtree(root) {
      if (!root) return;
      if (root.nodeType === 3) {
        translateTextNode(root);
        return;
      }
      var isShadow = root.nodeType === 11;
      if (!isShadow && root.nodeType !== 1) return;
      if (root.nodeType === 1) {
        if (root.tagName === "IFRAME") {
          tryAttachIframe(root);
          return;
        }
        if (root.shadowRoot) {
          observeRoot(root.shadowRoot);
          walkSubtree(root.shadowRoot);
        }
        translateElementAttrs(root);
        if (isSkippedForText(root)) return;
      }

      var ownerDoc = root.ownerDocument || document;
      var walker = ownerDoc.createTreeWalker(
        root,
        NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
        {
          acceptNode: function (n) {
            if (n.nodeType === 1) {
              if (n.tagName === "IFRAME") return NodeFilter.FILTER_ACCEPT;
              if (SKIP_TAGS[n.tagName]) {
                if (n.tagName === "INPUT" || n.tagName === "TEXTAREA") {
                  translateElementAttrs(n);
                }
                return NodeFilter.FILTER_REJECT;
              }
              if (
                n.getAttribute("contenteditable") === "true" ||
                n.getAttribute("contenteditable") === "" ||
                n.getAttribute("role") === "textbox" ||
                n.hasAttribute("data-monaco-editor") ||
                n.hasAttribute("data-no-i18n") ||
                (n.classList &&
                  (n.classList.contains("monaco-editor") || n.classList.contains("xterm")))
              ) {
                return NodeFilter.FILTER_REJECT;
              }
              return NodeFilter.FILTER_ACCEPT;
            }
            return NodeFilter.FILTER_ACCEPT;
          }
        }
      );
      var cur;
      while ((cur = walker.nextNode())) {
        if (cur.nodeType === 3) {
          translateTextNode(cur);
        } else if (cur.nodeType === 1) {
          if (cur.tagName === "IFRAME") {
            tryAttachIframe(cur);
          } else {
            if (cur.shadowRoot) {
              observeRoot(cur.shadowRoot);
              walkSubtree(cur.shadowRoot);
            }
            translateElementAttrs(cur);
          }
        }
      }
    }

    function startObserver() {
      try {
        if (!document.body) return;
        observeRoot(document.body);
        walkSubtree(document.body);
      } catch (_) {}
    }

    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", startObserver, { once: true });
    } else {
      startObserver();
    }
  } catch (_) {}
})();"#,
    );
    out
}

fn read_archive_app_version(archive: &AsarArchive) -> Option<String> {
    let pkg_bytes = archive.get_file("package.json")?;
    let val: Value = serde_json::from_slice(pkg_bytes).ok()?;
    val.get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_patch_meta(paths: &AppPaths) -> Option<PatchMeta> {
    let meta_path = ui_patch_backup_dir(paths).join("patch-meta.json");
    let raw = fs::read_to_string(meta_path).ok()?;
    serde_json::from_str::<PatchMeta>(&raw).ok()
}

fn verify_clean_backup_asar(paths: &AppPaths, expected_sha256: &str) -> Result<Vec<u8>, String> {
    if expected_sha256.trim().is_empty() {
        return Err("备份元数据缺少 originalAsarSha256".to_string());
    }
    let orig_path = ui_patch_backup_dir(paths).join("app.asar.orig");
    if !orig_path.is_file() {
        return Err("缺少 app.asar.orig 原始备份文件".to_string());
    }
    let bytes = fs::read(&orig_path)
        .map_err(|e| format!("读取原始备份 {} 失败: {e}", orig_path.display()))?;
    let actual_sha = sha256_hex(&bytes);
    if !actual_sha.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(format!(
            "原始备份 SHA-256 校验不一致（期望 {}，实际 {}）",
            expected_sha256, actual_sha
        ));
    }
    let parsed = AsarArchive::parse(&bytes)?;
    if parsed.get_file(PATCH_MARKER_FILE).is_some() {
        return Err("备份文件异常包含补丁标识，拒绝使用已污染备份".to_string());
    }
    Ok(bytes)
}

pub fn update_info_plist_asar_hash(plist_xml: &str, new_header_sha256: &str) -> Option<String> {
    update_info_plist_asar_key_hash(plist_xml, "Resources/app.asar", new_header_sha256)
}

fn update_target_info_plist_integrity(
    info_plist_path: &Path,
    new_header_sha256: &str,
    is_test: bool,
) -> Result<(), String> {
    if !info_plist_path.is_file() {
        return Ok(());
    }
    let raw_bytes = fs::read(info_plist_path)
        .map_err(|e| format!("读取 {} 失败: {e}", info_plist_path.display()))?;

    if let Ok(xml_text) = std::str::from_utf8(&raw_bytes) {
        if let Some(updated_xml) = update_info_plist_asar_hash(xml_text, new_header_sha256) {
            return write_bundle_file_checked(
                AppId::Antigravity,
                "patch",
                info_plist_path,
                updated_xml.as_bytes(),
                false,
            );
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    if !is_test && raw_bytes.starts_with(b"bplist00") {
        let _ = std::process::Command::new("/usr/bin/plutil")
            .args([
                "-replace",
                r"ElectronAsarIntegrity.Resources/app\.asar.hash",
                "-string",
                new_header_sha256,
            ])
            .arg(info_plist_path)
            .output();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = is_test;

    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn is_restricted_adhoc_entitlement_key(key: &str) -> bool {
    matches!(
        key,
        "com.apple.application-identifier"
            | "com.apple.developer.team-identifier"
            | "keychain-access-groups"
            | "com.apple.security.application-groups"
            | "aps-environment"
            | "com.apple.security.associated-domains"
    ) || key.starts_with("com.apple.developer.")
        || key.starts_with("com.apple.private.")
        || key.starts_with("com.apple.security.icloud")
}

/// Skip one XML plist value element starting at `start` (after optional whitespace).
/// Returns the byte offset immediately after the value element, or `None` if malformed.
#[cfg(any(target_os = "macos", test))]
fn find_plist_value_end(xml: &str, start: usize) -> Option<usize> {
    let bytes = xml.as_bytes();
    let mut i = start;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'<' {
        return None;
    }
    let tag_close_rel = xml[i..].find('>')?;
    let tag_close = i + tag_close_rel;
    let open_tag = xml[i + 1..tag_close].trim();
    if open_tag.ends_with('/') {
        // Self-closing tag such as <true/>, <false/>, <string/>, <array/>, <dict/>
        return Some(tag_close + 1);
    }
    let tag_name = open_tag.split_whitespace().next()?;
    let open_prefix = format!("<{tag_name}");
    let close_tag = format!("</{tag_name}>");

    let mut depth = 1usize;
    let mut cursor = tag_close + 1;
    while cursor < xml.len() {
        let next_lt = xml[cursor..].find('<')? + cursor;
        if xml[next_lt..].starts_with(&close_tag) {
            depth -= 1;
            cursor = next_lt + close_tag.len();
            if depth == 0 {
                return Some(cursor);
            }
            continue;
        }
        if xml[next_lt..].starts_with(&open_prefix) {
            let after_name = next_lt + open_prefix.len();
            let boundary_ok = xml
                .as_bytes()
                .get(after_name)
                .is_some_and(|&b| b == b'>' || b == b'/' || b.is_ascii_whitespace());
            if boundary_ok {
                let end_gt = xml[next_lt..].find('>')? + next_lt;
                let inner = xml[next_lt + 1..end_gt].trim();
                if !inner.ends_with('/') {
                    depth += 1;
                }
                cursor = end_gt + 1;
                continue;
            }
        }
        cursor = next_lt + 1;
    }
    None
}

/// Strip provisioning-profile-bound / restricted entitlements that cause macOS AMFI
/// to immediately SIGKILL (exit 137) an ad-hoc (`--sign -`) re-signed application
/// (notably Claude Desktop's `com.apple.application-identifier`,
/// `com.apple.developer.team-identifier`, and `keychain-access-groups`), while
/// preserving safe runtime capabilities such as JIT, audio/camera, AppleEvents, and virtualization.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn sanitize_macos_adhoc_entitlements_plist(xml: &str) -> Option<String> {
    let dict_open = xml.find("<dict>")?;
    let dict_body_start = dict_open + "<dict>".len();
    let dict_close = xml.rfind("</dict>")?;
    if dict_close < dict_body_start {
        return None;
    }

    let body = &xml[dict_body_start..dict_close];
    let mut kept_entries = String::new();
    let mut kept_count = 0usize;
    let mut cursor = 0usize;

    while cursor < body.len() {
        let rest = &body[cursor..];
        let trimmed_ws_len = rest.len() - rest.trim_start().len();
        cursor += trimmed_ws_len;
        if cursor >= body.len() {
            break;
        }
        if !body[cursor..].starts_with("<key>") {
            // Unexpected plist structure: fall back to omitting --entitlements rather than
            // risking writing corrupted XML or retaining restricted entitlements.
            return None;
        }
        let key_start = cursor + "<key>".len();
        let key_end_rel = body[key_start..].find("</key>")?;
        let key_end = key_start + key_end_rel;
        let key_name = body[key_start..key_end].trim();
        let after_key = key_end + "</key>".len();
        let val_end = find_plist_value_end(body, after_key)?;

        if !is_restricted_adhoc_entitlement_key(key_name) {
            kept_entries.push_str(&body[cursor..val_end]);
            kept_count += 1;
        }
        cursor = val_end;
    }

    if kept_count == 0 {
        return None;
    }

    let mut out = String::with_capacity(xml.len());
    out.push_str(&xml[..dict_body_start]);
    out.push_str(&kept_entries);
    out.push_str(&xml[dict_close..]);
    Some(out)
}

#[cfg(target_os = "macos")]
pub(crate) fn resign_macos_outer_bundle_for(
    app_id: AppId,
    action: &str,
    bundle_root: &Path,
    backup_dir: &Path,
) -> Result<(), String> {
    fs::create_dir_all(backup_dir)
        .map_err(|e| format!("创建临时目录 {} 失败: {e}", backup_dir.display()))?;
    let tmp_ent_path = backup_dir.join(format!(".entitlements.{}.plist", std::process::id()));

    let mut has_entitlements = false;
    if let Ok(ent_out) = std::process::Command::new("/usr/bin/codesign")
        .args(["-d", "--entitlements", ":-"])
        .arg(bundle_root)
        .output()
    {
        if ent_out.status.success() && !ent_out.stdout.is_empty() {
            let stdout_str = String::from_utf8_lossy(&ent_out.stdout);
            if stdout_str.contains("<plist") {
                if let Some(sanitized) = sanitize_macos_adhoc_entitlements_plist(&stdout_str) {
                    fs::write(&tmp_ent_path, sanitized.as_bytes())
                        .map_err(|e| format!("写入临时 entitlements 失败: {e}"))?;
                    has_entitlements = true;
                }
            }
        }
    }

    // Never pass `--deep` so inner Electron Framework, Helpers, and language_server keep original signatures.
    let mut cmd = std::process::Command::new("/usr/bin/codesign");
    cmd.arg("--force");
    if has_entitlements {
        cmd.arg("--entitlements").arg(&tmp_ent_path);
    }
    cmd.args(["--sign", "-"]).arg(bundle_root);

    let sign_res = cmd.output();
    if tmp_ent_path.exists() {
        let _ = fs::remove_file(&tmp_ent_path);
    }

    let out = sign_res.map_err(|e| format!("调用 codesign 失败: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let lower = stderr.to_ascii_lowercase();
        if lower.contains("operation not permitted") || lower.contains("permission denied") {
            let prompt = UiPatchPermissionPrompt {
                app_id,
                app_name: app_id.display_name().to_string(),
                action: action.to_string(),
                kind: UiPatchBlockKind::MacosAppManagement,
                target_path: Some(bundle_root.to_string_lossy().to_string()),
                message: format!(
                    "macOS 拦截了对 {} 安装包的本地重签操作。请前往「系统设置 -> 隐私与安全性 -> App 管理」开启 Tako Switch 授权后点击重试；配置文件级简体中文已正常生效。",
                    app_id.display_name()
                ),
            };
            return Err(format_ui_patch_block_error(&prompt));
        }
        return Err(format!(
            "本地重签 {} 失败: {stderr}",
            app_id.display_name()
        ));
    }
    Ok(())
}

fn is_patchable_asar(archive: &AsarArchive) -> bool {
    archive.get_file("dist/preload.js").is_some()
        || archive.get_file("dist/menu.js").is_some()
        || archive.get_file("dist/tray.js").is_some()
        || archive.get_file("dist/updater.js").is_some()
}

pub fn inspect_antigravity_ui_patch(paths: &AppPaths) -> UiPatchInfo {
    let target = match resolve_antigravity_target(paths) {
        TargetResolution::NotInstalled => {
            return UiPatchInfo {
                status: UiPatchStatus::NotInstalled,
                patch_version: None,
                has_valid_backup: false,
                target_asar_path: None,
                note: None,
            };
        }
        TargetResolution::MissingAsar { root } => {
            return UiPatchInfo {
                status: UiPatchStatus::Unsupported,
                patch_version: None,
                has_valid_backup: false,
                target_asar_path: None,
                note: Some(format!(
                    "未在 {} 中找到可修补的 app.asar 资源包",
                    root.display()
                )),
            };
        }
        TargetResolution::Found(t) => t,
    };

    let asar_path_str = target.asar_path.to_string_lossy().to_string();
    let Ok(asar_bytes) = fs::read(&target.asar_path) else {
        return UiPatchInfo {
            status: UiPatchStatus::Unsupported,
            patch_version: None,
            has_valid_backup: false,
            target_asar_path: Some(asar_path_str),
            note: Some("无法读取 app.asar 文件".to_string()),
        };
    };

    let archive = match AsarArchive::parse(&asar_bytes) {
        Ok(a) => a,
        Err(err) => {
            return UiPatchInfo {
                status: UiPatchStatus::Unsupported,
                patch_version: None,
                has_valid_backup: false,
                target_asar_path: Some(asar_path_str),
                note: Some(err),
            };
        }
    };

    let meta = read_patch_meta(paths);
    if let Some(marker_bytes) = archive.get_file(PATCH_MARKER_FILE) {
        let marker = serde_json::from_slice::<PatchMarkerPayload>(marker_bytes).ok();
        let patch_version = marker
            .as_ref()
            .map(|m| m.patch_version.clone())
            .or_else(|| meta.as_ref().map(|m| m.patch_version.clone()));
        let expected_sha = meta
            .as_ref()
            .map(|m| m.original_asar_sha256.as_str())
            .or_else(|| marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
            .unwrap_or("");
        let has_valid_backup = verify_clean_backup_asar(paths, expected_sha).is_ok();

        return UiPatchInfo {
            status: UiPatchStatus::Patched,
            patch_version,
            has_valid_backup,
            target_asar_path: Some(asar_path_str),
            note: Some("Antigravity 菜单与主界面简体中文补丁已生效".to_string()),
        };
    }

    if !is_patchable_asar(&archive) {
        return UiPatchInfo {
            status: UiPatchStatus::Unsupported,
            patch_version: None,
            has_valid_backup: false,
            target_asar_path: Some(asar_path_str),
            note: Some("当前 Antigravity 安装包未包含受支持的入口脚本".to_string()),
        };
    }

    if let Some(old_meta) = meta {
        return UiPatchInfo {
            status: UiPatchStatus::OutdatedAfterUpgrade,
            patch_version: Some(old_meta.patch_version),
            has_valid_backup: false,
            target_asar_path: Some(asar_path_str),
            note: Some("检测到 Antigravity 已升级至新版本，点击即可基于新版重新注入汉化补丁".to_string()),
        };
    }

    UiPatchInfo {
        status: UiPatchStatus::Unpatched,
        patch_version: None,
        has_valid_backup: false,
        target_asar_path: Some(asar_path_str),
        note: None,
    }
}

pub fn patch_antigravity_ui_for_paths(paths: &AppPaths) -> Result<UiPatchInfo, String> {
    let target = match resolve_antigravity_target(paths) {
        TargetResolution::NotInstalled => {
            return Err("未检测到 Google Antigravity 安装目录".to_string());
        }
        TargetResolution::MissingAsar { root } => {
            return Err(format!(
                "未在 {} 中找到可修补的 app.asar 资源包",
                root.display()
            ));
        }
        TargetResolution::Found(t) => t,
    };

    let current_bytes = fs::read(&target.asar_path)
        .map_err(|e| format!("读取 {} 失败: {e}", target.asar_path.display()))?;
    if target
        .asar_path
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("windowsapps")
    {
        return Err(format_ui_patch_block_error(&make_msix_readonly_prompt(
            AppId::Antigravity,
            "patch",
            Some(&target.asar_path),
        )));
    }
    let current_archive = AsarArchive::parse(&current_bytes)?;
    let backup_dir = ui_patch_backup_dir(paths);

    // Determine clean base archive & ensure original backup is NEVER overwritten by a patched ASAR.
    let (mut base_archive, orig_sha256) =
        if let Some(marker_bytes) = current_archive.get_file(PATCH_MARKER_FILE) {
            let marker: PatchMarkerPayload = serde_json::from_slice(marker_bytes)
                .map_err(|e| format!("解析补丁标识失败: {e}"))?;
            let expected_orig_sha = read_patch_meta(paths)
                .map(|m| m.original_asar_sha256)
                .unwrap_or_else(|| marker.original_asar_sha256.clone());

            if marker.patch_version == CURRENT_UI_PATCH_VERSION
                && verify_clean_backup_asar(paths, &expected_orig_sha).is_ok()
            {
                return Ok(inspect_antigravity_ui_patch(paths));
            }

            probe_bundle_write_access(AppId::Antigravity, "patch", &target.asar_path)?;
            if let Some(ref plist_path) = target.info_plist_path {
                if plist_path.is_file() {
                    probe_bundle_write_access(AppId::Antigravity, "patch", plist_path)?;
                }
            }
            let clean_orig_bytes = verify_clean_backup_asar(paths, &expected_orig_sha)?;
            let clean_archive = AsarArchive::parse(&clean_orig_bytes)?;
            (clean_archive, expected_orig_sha)
        } else {
            if !is_patchable_asar(&current_archive) {
                return Err("当前 Antigravity 版本目录结构不受支持，已安全跳过修改".to_string());
            }
            probe_bundle_write_access(AppId::Antigravity, "patch", &target.asar_path)?;
            if let Some(ref plist_path) = target.info_plist_path {
                if plist_path.is_file() {
                    probe_bundle_write_access(AppId::Antigravity, "patch", plist_path)?;
                }
            }
            let clean_sha = sha256_hex(&current_bytes);
            // Current ASAR is guaranteed unpatched (no __tako_i18n_patch.json), safe to write/refresh .orig backups.
            write_atomic(&backup_dir.join("app.asar.orig"), &current_bytes, false)?;
            if let Some(ref plist_path) = target.info_plist_path {
                if plist_path.is_file() {
                    let plist_bytes = fs::read(plist_path)
                        .map_err(|e| format!("读取 {} 失败: {e}", plist_path.display()))?;
                    write_atomic(&backup_dir.join("Info.plist.orig"), &plist_bytes, false)?;
                }
            }
            if let Some(ref bin_path) = target.macos_bin_path {
                if bin_path.is_file() {
                    let bin_bytes = fs::read(bin_path)
                        .map_err(|e| format!("读取 {} 失败: {e}", bin_path.display()))?;
                    write_atomic(
                        &backup_dir.join("MacOS_Antigravity.orig"),
                        &bin_bytes,
                        false,
                    )?;
                }
            }
            if let Some(ref cr_path) = target.code_resources_path {
                if cr_path.is_file() {
                    let cr_bytes = fs::read(cr_path)
                        .map_err(|e| format!("读取 {} 失败: {e}", cr_path.display()))?;
                    write_atomic(&backup_dir.join("CodeResources.orig"), &cr_bytes, false)?;
                }
            }
            (current_archive, clean_sha)
        };

    let app_version = read_archive_app_version(&base_archive).or(target.app_version_hint.clone());

    for &(rel_path, is_updater) in &[
        ("dist/menu.js", false),
        ("dist/tray.js", false),
        ("dist/updater.js", true),
    ] {
        if let Some(raw_bytes) = base_archive.get_file(rel_path) {
            let raw_str = std::str::from_utf8(raw_bytes)
                .map_err(|e| format!("{rel_path} 非合法 UTF-8: {e}"))?;
            let patched = patch_main_process_js(raw_str, is_updater);
            base_archive.set_file(rel_path, patched.into_bytes())?;
        }
    }

    if let Some(preload_bytes) = base_archive.get_file("dist/preload.js") {
        let preload_str = std::str::from_utf8(preload_bytes)
            .map_err(|e| format!("dist/preload.js 非合法 UTF-8: {e}"))?;
        let patched_preload = patch_preload_js(preload_str);
        base_archive.set_file("dist/preload.js", patched_preload.into_bytes())?;
    }

    let marker_payload = PatchMarkerPayload {
        schema: 1,
        patch_version: CURRENT_UI_PATCH_VERSION.to_string(),
        original_asar_sha256: orig_sha256.clone(),
        app_version: app_version.clone(),
    };
    let marker_json = serde_json::to_vec_pretty(&marker_payload)
        .map_err(|e| format!("序列化补丁标识失败: {e}"))?;
    base_archive.set_file(PATCH_MARKER_FILE, marker_json)?;

    let (patched_asar_bytes, header_json) = base_archive.pack()?;
    let patched_asar_sha256 = sha256_hex(&patched_asar_bytes);
    let header_sha256 = sha256_hex(header_json.as_bytes());

    write_bundle_file_checked(
        AppId::Antigravity,
        "patch",
        &target.asar_path,
        &patched_asar_bytes,
        false,
    )?;

    if let Some(ref plist_path) = target.info_plist_path {
        update_target_info_plist_integrity(plist_path, &header_sha256, paths.is_test)?;
    }

    let meta = PatchMeta {
        schema: 1,
        patch_version: CURRENT_UI_PATCH_VERSION.to_string(),
        app_version,
        original_asar_sha256: orig_sha256,
        patched_asar_sha256,
    };
    let meta_bytes =
        serde_json::to_vec_pretty(&meta).map_err(|e| format!("序列化 patch-meta.json 失败: {e}"))?;
    write_atomic(&backup_dir.join("patch-meta.json"), &meta_bytes, false)?;

    #[cfg(target_os = "macos")]
    if !paths.is_test {
        if let Some(ref bundle_root) = target.bundle_root {
            resign_macos_outer_bundle_for(AppId::Antigravity, "patch", bundle_root, &backup_dir)?;
        }
    }

    Ok(inspect_antigravity_ui_patch(paths))
}

pub fn restore_antigravity_ui_for_paths(paths: &AppPaths) -> Result<UiPatchInfo, String> {
    let target = match resolve_antigravity_target(paths) {
        TargetResolution::NotInstalled => {
            return Err("未检测到 Google Antigravity 安装目录".to_string());
        }
        TargetResolution::MissingAsar { root } => {
            return Err(format!(
                "未在 {} 中找到 app.asar 资源包",
                root.display()
            ));
        }
        TargetResolution::Found(t) => t,
    };

    if target
        .asar_path
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("windowsapps")
    {
        return Err(format_ui_patch_block_error(&make_msix_readonly_prompt(
            AppId::Antigravity,
            "restore",
            Some(&target.asar_path),
        )));
    }

    let current_bytes = fs::read(&target.asar_path)
        .map_err(|e| format!("读取 {} 失败: {e}", target.asar_path.display()))?;
    let current_archive = AsarArchive::parse(&current_bytes)?;
    let backup_dir = ui_patch_backup_dir(paths);
    let meta_path = backup_dir.join("patch-meta.json");

    // If current ASAR is unpatched (e.g. upgraded by official updater), NEVER overwrite with stale backup.
    let Some(marker_bytes) = current_archive.get_file(PATCH_MARKER_FILE) else {
        if meta_path.exists() {
            fs::remove_file(&meta_path)
                .map_err(|e| format!("清理过期 {} 失败: {e}", meta_path.display()))?;
        }
        return Ok(inspect_antigravity_ui_patch(paths));
    };

    let marker = serde_json::from_slice::<PatchMarkerPayload>(marker_bytes).ok();
    let expected_orig_sha = read_patch_meta(paths)
        .map(|m| m.original_asar_sha256)
        .or_else(|| marker.map(|m| m.original_asar_sha256))
        .unwrap_or_default();

    let clean_orig_bytes = verify_clean_backup_asar(paths, &expected_orig_sha)?;
    probe_bundle_write_access(AppId::Antigravity, "restore", &target.asar_path)?;
    write_bundle_file_checked(
        AppId::Antigravity,
        "restore",
        &target.asar_path,
        &clean_orig_bytes,
        false,
    )?;

    let mut restored_full_signature = false;
    if target.bundle_root.is_some() {
        let orig_plist = backup_dir.join("Info.plist.orig");
        if orig_plist.is_file() {
            if let Some(ref dst_plist) = target.info_plist_path {
                let bytes = fs::read(&orig_plist)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_plist.display()))?;
                write_bundle_file_checked(AppId::Antigravity, "restore", dst_plist, &bytes, false)?;
            }
        }

        let orig_bin = backup_dir.join("MacOS_Antigravity.orig");
        let orig_cr = backup_dir.join("CodeResources.orig");
        let mut restored_bin = false;
        let mut restored_cr = false;

        if orig_bin.is_file() {
            if let Some(ref dst_bin) = target.macos_bin_path {
                let bytes = fs::read(&orig_bin)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_bin.display()))?;
                write_bundle_file_checked(AppId::Antigravity, "restore", dst_bin, &bytes, true)?;
                restored_bin = true;
            }
        }

        if orig_cr.is_file() {
            if let Some(ref dst_cr) = target.code_resources_path {
                let bytes = fs::read(&orig_cr)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_cr.display()))?;
                write_bundle_file_checked(AppId::Antigravity, "restore", dst_cr, &bytes, false)?;
                restored_cr = true;
            }
        }

        restored_full_signature = orig_plist.is_file() && restored_bin && restored_cr;
    }

    if meta_path.exists() {
        fs::remove_file(&meta_path)
            .map_err(|e| format!("清理 {} 失败: {e}", meta_path.display()))?;
    }

    #[cfg(target_os = "macos")]
    if !paths.is_test && !restored_full_signature {
        if let Some(ref bundle_root) = target.bundle_root {
            resign_macos_outer_bundle_for(AppId::Antigravity, "restore", bundle_root, &backup_dir)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = restored_full_signature;

    Ok(inspect_antigravity_ui_patch(paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn build_synthetic_asar(
        files: &[(&str, &[u8])],
        include_unpacked_entry: bool,
    ) -> (Vec<u8>, String) {
        let mut root = json!({ "files": {} });
        if include_unpacked_entry {
            root["files"]["native_addon"] = json!({
                "files": {
                    "pty.node": {
                        "size": 4096,
                        "unpacked": true
                    }
                }
            });
        }
        let mut archive = AsarArchive {
            header: root,
            packed_files: HashMap::new(),
            modified_files: HashSet::new(),
        };
        for &(rel, content) in files {
            archive.set_file(rel, content.to_vec()).unwrap();
        }
        archive.pack().unwrap()
    }

    fn setup_fake_macos_agy_bundle(tmp: &Path, version: &str) -> (AppPaths, PathBuf) {
        let mut paths = AppPaths::for_test(tmp);
        let app_dir = tmp.join("FakeApps").join("Antigravity.app");
        paths.agy_app_override_dir = Some(app_dir.clone());

        let pkg_json = format!(r#"{{"name":"antigravity","version":"{version}"}}"#);
        let menu_js = r#"
const { Menu } = require('electron');
function setupMenu() {
  const menu = Menu.buildFromTemplate([
    { label: 'File', submenu: [{ label: 'New Window' }] },
    { label: 'View', submenu: [{ label: 'Actual Size' }, { label: 'Zoom In' }, { label: 'Zoom Out' }] },
    { label: 'Help', submenu: [{ label: 'Docs' }] }
  ]);
  const viewMenu = menu.items?.find(i => i.label === 'View');
  addItemToSubmenu(menu, 'File', { label: 'Connect to WSL' });
  addItemToSubmenu(menu, 'File', { label: 'Reopen Locally' });
  return { menu, viewMenu };
}
"#;
        let tray_js = r#"
const status = agents.length > 0 ? 'Active' : 'No agents running';
const trayItems = [{ label: 'No agents running', enabled: false }, { label: 'New Window' }, { label: 'Quit' }];
"#;
        let updater_js = r#"
const updateActions = {
  'Check for Updates...': () => triggerUpdateCheck(),
  'Restart to Update': () => applyUpdateNow(),
};
addItemToSubmenu(menu, 'Help', { label: 'Check for Updates...', click: (item) => updateActions[item.label]?.() });
"#;
        let preload_js = r#"const { contextBridge } = require('electron');"#;

        let (asar_bytes, header_json) = build_synthetic_asar(
            &[
                ("package.json", pkg_json.as_bytes()),
                ("dist/menu.js", menu_js.as_bytes()),
                ("dist/tray.js", tray_js.as_bytes()),
                ("dist/updater.js", updater_js.as_bytes()),
                ("dist/preload.js", preload_js.as_bytes()),
            ],
            true,
        );
        let orig_header_sha = sha256_hex(header_json.as_bytes());
        let info_plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.google.antigravity</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>ElectronAsarIntegrity</key>
  <dict>
    <key>Resources/app.asar</key>
    <dict>
      <key>algorithm</key>
      <string>SHA256</string>
      <key>hash</key>
      <string>{orig_header_sha}</string>
    </dict>
  </dict>
</dict>
</plist>
"#
        );

        write_atomic(
            &app_dir.join("Contents").join("Resources").join("app.asar"),
            &asar_bytes,
            false,
        )
        .unwrap();
        write_atomic(
            &app_dir.join("Contents").join("Info.plist"),
            info_plist.as_bytes(),
            false,
        )
        .unwrap();
        write_atomic(
            &app_dir
                .join("Contents")
                .join("MacOS")
                .join("Antigravity"),
            format!("MACH_O_BIN_{version}").as_bytes(),
            false,
        )
        .unwrap();
        write_atomic(
            &app_dir
                .join("Contents")
                .join("_CodeSignature")
                .join("CodeResources"),
            format!("CODE_SIG_{version}").as_bytes(),
            false,
        )
        .unwrap();

        (paths, app_dir)
    }

    #[test]
    fn asar_pickle_alignment_and_unpacked_entries_roundtrip() {
        let (asar_bytes, header_json) = build_synthetic_asar(
            &[
                ("package.json", br#"{"version":"2.0.0"}"#),
                ("dist/preload.js", b"console.log('hello');"),
            ],
            true,
        );
        let u0 = u32::from_le_bytes([asar_bytes[0], asar_bytes[1], asar_bytes[2], asar_bytes[3]]);
        let u1 = u32::from_le_bytes([asar_bytes[4], asar_bytes[5], asar_bytes[6], asar_bytes[7]]);
        let u2 =
            u32::from_le_bytes([asar_bytes[8], asar_bytes[9], asar_bytes[10], asar_bytes[11]]);
        let strlen =
            u32::from_le_bytes([asar_bytes[12], asar_bytes[13], asar_bytes[14], asar_bytes[15]]);

        let padded = (strlen + 3) & !3;
        assert_eq!(u0, 4);
        assert_eq!(u2, 4 + padded);
        assert_eq!(u1, 8 + padded);
        assert_eq!(strlen as usize, header_json.len());

        let parsed = AsarArchive::parse(&asar_bytes).unwrap();
        assert_eq!(
            parsed.get_file("dist/preload.js"),
            Some(&b"console.log('hello');"[..])
        );
        let native_entry = &parsed.header["files"]["native_addon"]["files"]["pty.node"];
        assert_eq!(native_entry["unpacked"], json!(true));
        assert!(native_entry.get("offset").is_none());
    }

    #[test]
    fn patch_idempotency_backup_protection_and_byte_exact_restore() {
        let tmp = TempDir::new().unwrap();
        let (paths, app_dir) = setup_fake_macos_agy_bundle(tmp.path(), "2.1.0");
        let asar_path = app_dir.join("Contents").join("Resources").join("app.asar");
        let plist_path = app_dir.join("Contents").join("Info.plist");
        let bin_path = app_dir.join("Contents").join("MacOS").join("Antigravity");
        let cr_path = app_dir
            .join("Contents")
            .join("_CodeSignature")
            .join("CodeResources");

        let orig_asar = fs::read(&asar_path).unwrap();
        let orig_plist = fs::read(&plist_path).unwrap();
        let orig_bin = fs::read(&bin_path).unwrap();
        let orig_cr = fs::read(&cr_path).unwrap();

        let initial = inspect_antigravity_ui_patch(&paths);
        assert_eq!(initial.status, UiPatchStatus::Unpatched);

        let patched_info = patch_antigravity_ui_for_paths(&paths).unwrap();
        assert_eq!(patched_info.status, UiPatchStatus::Patched);
        assert_eq!(
            patched_info.patch_version.as_deref(),
            Some(CURRENT_UI_PATCH_VERSION)
        );
        assert!(patched_info.has_valid_backup);

        // Verify injected files and menu/updater invariants
        let patched_bytes = fs::read(&asar_path).unwrap();
        let patched_archive = AsarArchive::parse(&patched_bytes).unwrap();
        assert!(patched_archive.get_file(PATCH_MARKER_FILE).is_some());
        let menu_patched =
            std::str::from_utf8(patched_archive.get_file("dist/menu.js").unwrap()).unwrap();
        assert!(menu_patched.contains("i.label === 'View'"));
        assert!(menu_patched.contains("addItemToSubmenu(menu, 'File'"));
        assert!(menu_patched.contains("新建窗口"));
        assert!(menu_patched.contains("连接到 WSL"));

        let updater_patched =
            std::str::from_utf8(patched_archive.get_file("dist/updater.js").unwrap()).unwrap();
        assert!(updater_patched.contains("__takoBindZhUpdateActions"));
        assert!(updater_patched.contains("'Check for Updates...'"));
        assert!(updater_patched.contains("检查更新…"));

        let preload_patched =
            std::str::from_utf8(patched_archive.get_file("dist/preload.js").unwrap()).unwrap();
        assert!(preload_patched.contains(PATCH_GUARD_COMMENT));
        assert!(preload_patched.contains("\"New Conversation\": \"新建对话\""));
        assert!(preload_patched.contains(".monaco-editor"));

        // Verify Info.plist ElectronAsarIntegrity hash matches new header JSON SHA256
        let strlen = u32::from_le_bytes([
            patched_bytes[12],
            patched_bytes[13],
            patched_bytes[14],
            patched_bytes[15],
        ]) as usize;
        let expected_header_hash = sha256_hex(&patched_bytes[16..16 + strlen]);
        let updated_plist = fs::read_to_string(&plist_path).unwrap();
        assert!(updated_plist.contains(&expected_header_hash));

        // Second patch call must be idempotent and NEVER overwrite app.asar.orig with patched bytes
        let second_info = patch_antigravity_ui_for_paths(&paths).unwrap();
        assert_eq!(second_info.status, UiPatchStatus::Patched);
        let backup_orig =
            fs::read(ui_patch_backup_dir(&paths).join("app.asar.orig")).unwrap();
        assert_eq!(backup_orig, orig_asar);

        // Restore must recover byte-for-byte identical files
        let restored = restore_antigravity_ui_for_paths(&paths).unwrap();
        assert_eq!(restored.status, UiPatchStatus::Unpatched);
        assert_eq!(fs::read(&asar_path).unwrap(), orig_asar);
        assert_eq!(fs::read(&plist_path).unwrap(), orig_plist);
        assert_eq!(fs::read(&bin_path).unwrap(), orig_bin);
        assert_eq!(fs::read(&cr_path).unwrap(), orig_cr);
    }

    #[test]
    fn upgrade_detection_never_downgrades_on_restore_and_refreshes_backup_on_repatch() {
        let tmp = TempDir::new().unwrap();
        let (paths, app_dir) = setup_fake_macos_agy_bundle(tmp.path(), "2.1.0");
        patch_antigravity_ui_for_paths(&paths).unwrap();

        // Simulate official Antigravity self-upgrade to 2.2.0 overwriting bundle files
        let (_, _) = setup_fake_macos_agy_bundle(tmp.path(), "2.2.0");
        let asar_path = app_dir.join("Contents").join("Resources").join("app.asar");
        let upgraded_clean_asar = fs::read(&asar_path).unwrap();

        let after_upgrade = inspect_antigravity_ui_patch(&paths);
        assert_eq!(after_upgrade.status, UiPatchStatus::OutdatedAfterUpgrade);

        // Calling restore after upgrade must NOT overwrite 2.2.0 with 2.1.0 backup!
        let restored = restore_antigravity_ui_for_paths(&paths).unwrap();
        assert_eq!(restored.status, UiPatchStatus::Unpatched);
        assert_eq!(fs::read(&asar_path).unwrap(), upgraded_clean_asar);

        // Re-apply patch on 2.1.0 -> upgrade to 2.2.0 -> re-patch must refresh .orig to 2.2.0!
        patch_antigravity_ui_for_paths(&paths).unwrap();
        let (_, _) = setup_fake_macos_agy_bundle(tmp.path(), "2.3.0");
        let v230_clean_asar = fs::read(&asar_path).unwrap();
        assert_eq!(
            inspect_antigravity_ui_patch(&paths).status,
            UiPatchStatus::OutdatedAfterUpgrade
        );

        let repatched = patch_antigravity_ui_for_paths(&paths).unwrap();
        assert_eq!(repatched.status, UiPatchStatus::Patched);
        let refreshed_orig =
            fs::read(ui_patch_backup_dir(&paths).join("app.asar.orig")).unwrap();
        assert_eq!(refreshed_orig, v230_clean_asar);
    }

    #[test]
    fn zh_dict_json_is_valid_non_empty_and_meets_scale_target() {
        let parsed: Value = serde_json::from_str(ANTIGRAVITY_ZH_DICT_JSON)
            .expect("antigravity_zh_dict.json 必须是合法 JSON");
        let map = parsed
            .as_object()
            .expect("antigravity_zh_dict.json 顶层必须是 JSON 对象");
        assert!(
            map.len() >= 1200,
            "词典有效词条应 >= 1200 条，实际为 {}",
            map.len()
        );
        for (k, v) in map {
            assert!(!k.trim().is_empty(), "存在空英文键");
            let zh = v
                .as_str()
                .unwrap_or_else(|| panic!("键 `{k}` 对应值必须是字符串"));
            assert!(!zh.trim().is_empty(), "键 `{k}` 对应中文译文为空");
            assert_ne!(k, zh, "键 `{k}` 不应为无意义恒等映射");
        }
        // Core Antigravity 2.x modules coverage spot-checks
        for required_key in [
            "New Conversation",
            "Scheduled Tasks",
            "Skills & Customizations",
            "MCP Servers",
            "Reasoning Effort",
            "Terminal Sandbox",
            "Non-Workspace File Access",
            "Internet Access Policy",
            "Implementation Plan",
            "Walkthrough",
            "Swarm Board",
            "Remote Control",
            "Dev Containers",
            "Port Forwarding",
            "Connect to WSL",
            "Create with Prompt",
            "No automations yet",
            "Run a prompt on a schedule, like a morning summary of yesterday's changes. Runs show up here.",
            "Not in Project",
            "Give Feedback",
            "Show system notifications when your action is needed or the agent finishes.",
            "Play a sound when your action is needed or the agent finishes.",
            "Set the volume of agent sounds.",
        ] {
            assert!(
                map.contains_key(required_key),
                "词典缺少核心词条: {required_key}"
            );
        }
    }

    #[test]
    fn sanitize_macos_adhoc_entitlements_strips_restricted_keys_and_preserves_safe_caps() {
        let claude_entitlements = r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>com.apple.application-identifier</key><string>Q6L2SF6YDW.com.anthropic.claudefordesktop</string><key>com.apple.developer.team-identifier</key><string>Q6L2SF6YDW</string><key>com.apple.security.automation.apple-events</key><true/><key>com.apple.security.cs.allow-jit</key><true/><key>com.apple.security.virtualization</key><true/><key>keychain-access-groups</key><array><string>Q6L2SF6YDW.com.anthropic.claude.webauthn</string><string>Q6L2SF6YDW.com.anthropic.claude.hwkey</string></array></dict></plist>"#;

        let sanitized = sanitize_macos_adhoc_entitlements_plist(claude_entitlements)
            .expect("must retain safe runtime entitlements");
        assert!(!sanitized.contains("com.apple.application-identifier"));
        assert!(!sanitized.contains("com.apple.developer.team-identifier"));
        assert!(!sanitized.contains("keychain-access-groups"));
        assert!(!sanitized.contains("Q6L2SF6YDW"));
        assert!(sanitized.contains("com.apple.security.automation.apple-events"));
        assert!(sanitized.contains("com.apple.security.cs.allow-jit"));
        assert!(sanitized.contains("com.apple.security.virtualization"));

        // If all entitlements are restricted, return None so codesign omits --entitlements
        let only_restricted = r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>com.apple.application-identifier</key><string>TEAM.app</string><key>com.apple.developer.icloud-services</key><array><string>CloudKit</string></array></dict></plist>"#;
        assert!(sanitize_macos_adhoc_entitlements_plist(only_restricted).is_none());
    }

    #[test]
    fn preload_iife_contains_upgraded_engine_and_is_idempotent() {
        let raw = "const { contextBridge } = require('electron');\n";
        let once = patch_preload_js(raw);
        let twice = patch_preload_js(&once);
        assert_eq!(once, twice, "preload 注入必须幂等");
        assert_eq!(once.matches(PATCH_GUARD_COMMENT).count(), 1);

        // Dictionary, punctuation normalization, dynamic rules & split-text-node protection
        assert!(once.contains("\"New Conversation\": \"新建对话\""));
        assert!(once.contains("\"Create with Prompt\": \"用提示词创建\""));
        assert!(once.contains("\"No automations yet\": \"暂无自动化任务\""));
        assert!(once.contains("AMBIGUOUS_SINGLE_WORDS"));
        assert!(once.contains("translateCombinedTextNodes"));
        assert!(once.contains("hasNonEmptySibling"));
        assert!(once.contains("DYNAMIC_RULES"));
        assert!(once.contains("Thinking for"));
        assert!(once.contains("Thought for"));
        assert!(once.contains("Worked for"));
        assert!(once.contains("files?\\s+changed"));
        assert!(once.contains("Showing lines"));
        assert!(once.contains("lookupDict"));

        // Shadow DOM & same-origin localhost iframe support
        assert!(once.contains("shadowRoot"));
        assert!(once.contains("contentDocument"));
        assert!(once.contains("WeakSet"));
        assert!(once.contains("host !== \"127.0.0.1\" && host !== \"localhost\""));
        assert!(
            !once.contains("window.top !== window.self"),
            "不应一刀切禁止同源本地 iframe 翻译"
        );

        // Strict code & chat body safety guards
        for guard in [
            "INPUT: 1",
            "SAMP: 1",
            "TEXTAREA: 1",
            "CODE: 1",
            "PRE: 1",
            "SVG: 1",
            "MATH: 1",
            "[data-monaco-editor]",
            "[role=\"textbox\"]",
            ".monaco-editor",
            ".xterm",
            ".markdown-body",
            ".prose",
            "[data-message-role]",
        ] {
            assert!(once.contains(guard), "preload 注入缺少安全跳过守卫: {guard}");
        }
    }

    #[test]
    fn main_process_static_patches_preserve_top_menus_and_translate_tray_updater() {
        let tray_src = r#"
function updateTrayAgentCount(count) {
    countItem.label =
        (count > 0 ? `${count}` : 'No') +
                    ' agent' +
                    (count === 1 ? '' : 's') +
                    ' running';
}
"#;
        let tray_patched = patch_main_process_js(tray_src, false);
        assert!(tray_patched.contains("个智能体运行中"));
        assert!(tray_patched.contains("暂无运行中的智能体"));
        assert_eq!(patch_main_process_js(&tray_patched, false), tray_patched);

        let updater_src = r#"
MenuUpdateStep["CheckForUpdates"] = "Check for Updates";
exports.updateActions = {
  [MenuUpdateStep.CheckForUpdates]: () => checkForUpdates(true),
};
const options = {
  type: 'info',
  title: 'Check for Updates',
  message: 'No updates available',
  buttons: ['OK'],
};
"#;
        let updater_patched = patch_main_process_js(updater_src, true);
        assert!(updater_patched.contains("__takoBindZhUpdateActions"));
        assert!(updater_patched.contains("[\"Check for Updates\", \"检查更新\"]"));
        assert!(updater_patched.contains("title: '检查更新'"));
        assert!(updater_patched.contains("message: '暂无可用更新'"));
        assert!(updater_patched.contains("buttons: ['确定']"));
        assert_eq!(
            patch_main_process_js(&updater_patched, true),
            updater_patched
        );
    }
}
