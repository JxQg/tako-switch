use crate::adapters::DesktopAppAdapter;
use crate::config::{write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub const PATCH_MARKER_FILE: &str = "__tako_i18n_patch.json";
pub const PATCH_GUARD_COMMENT: &str = "/* __TAKO_AGY_ZH_PATCH_V1__ */";
pub const CURRENT_UI_PATCH_VERSION: &str = "2.0.8";
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

fn sha256_hex(bytes: &[u8]) -> String {
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
            if modified_files.contains(&rel) || obj.contains_key("integrity") {
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
    let blocks: Vec<String> = if data.is_empty() {
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
    ("New Conversation", "新建对话"),
    ("Open...", "打开…"),
    ("Open Folder...", "打开文件夹…"),
    ("Open Recent", "打开最近使用"),
    ("Clear Recently Opened", "清除最近打开记录"),
    ("Close Window", "关闭窗口"),
    ("Settings...", "设置…"),
    ("Preferences...", "偏好设置…"),
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
    if is_updater {
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

fn build_preload_iife() -> &'static str {
    r#"/* __TAKO_AGY_ZH_PATCH_V1__ */
;(function () {
  try {
    if (typeof window === "undefined" || typeof document === "undefined") return;
    if (window.top !== window.self) return;
    var host = (window.location && window.location.hostname) || "";
    if (host !== "127.0.0.1" && host !== "localhost") return;
    if (window.__TAKO_AGY_ZH_PATCHED__) return;
    window.__TAKO_AGY_ZH_PATCHED__ = true;

    var DICT = Object.assign(Object.create(null), {
      // Sidebar & Primary Navigation
      "New Conversation": "新建对话",
      "Start New Conversation": "开始新对话",
      "Conversations": "对话列表",
      "All Conversations": "全部对话",
      "Recent Conversations": "最近对话",
      "Pinned": "已置顶",
      "Archive": "归档",
      "Archived": "已归档",
      "Inbox": "收件箱",
      "Projects": "项目",
      "New Project": "新建项目",
      "Workspaces": "工作区",
      "Open Workspace": "打开工作区",
      "Add Workspace": "添加工作区",
      "Select a workspace": "选择工作区",
      "No workspace selected": "未选择工作区",
      "Scheduled Tasks": "定时任务",
      "New Scheduled Task": "新建定时任务",
      "Skills & Customizations": "技能与定制",
      "Customizations": "定制",
      "Agents": "智能体",
      "Skills": "技能",
      "New Skill": "新建技能",
      "Rules": "规则",
      "New Rule": "新建规则",
      "User Rules": "全局规则",
      "Workspace Rules": "工作区规则",
      "Memory": "记忆",
      "Memories": "记忆库",
      "MCP Servers": "MCP 服务",
      "Add MCP Server": "添加 MCP 服务",
      "Configure MCP": "配置 MCP",
      "History": "历史记录",
      "Search": "搜索",
      "Settings": "设置",

      // Settings & Security Policies
      "Global Settings": "全局设置",
      "Project-Level Settings": "项目级设置",
      "Project Settings": "项目设置",
      "App Settings": "应用设置",
      "General": "通用",
      "Appearance": "外观",
      "Theme": "主题",
      "Light": "浅色",
      "Dark": "深色",
      "System": "跟随系统",
      "Language": "语言",
      "Shortcuts": "快捷键",
      "Keyboard Shortcuts": "键盘快捷键",
      "Notifications": "通知",
      "Account": "账号",
      "Updates": "更新",
      "Advanced": "高级",
      "About": "关于",
      "Model Selection": "模型选择",
      "Default Model": "默认模型",
      "Reasoning Effort": "推理强度",
      "Tool Execution Policy": "工具执行策略",
      "Terminal Sandbox": "终端沙盒",
      "Non-Workspace File Access": "工作区外文件访问",
      "Internet Access Policy": "联网访问策略",
      "Permission Grants": "权限授权",
      "Global Permission Grants": "全局权限授权",
      "Command Allowlist": "命令白名单",
      "Command Denylist": "命令黑名单",
      "Browser Allowlist": "浏览器白名单",
      "Browser Settings": "浏览器设置",
      "Editor Settings": "编辑器设置",
      "Auto": "自动",
      "Always Ask": "每次询问",
      "Ask": "询问",
      "Always Allow": "始终允许",
      "Allow": "允许",
      "Deny": "拒绝",
      "Enabled": "已启用",
      "Disabled": "已禁用",
      "On": "开启",
      "Off": "关闭",
      "Default": "默认",
      "Custom": "自定义",
      "Strict": "严格",
      "Permissive": "宽松",
      "Read Only": "只读",
      "Read & Write": "读写",

      // Panels, Tabs & Artifacts
      "Subagents": "子智能体",
      "Background Tasks": "后台任务",
      "Artifacts": "产物",
      "Files Changed": "变更文件",
      "Changed Files": "变更文件",
      "Changes": "变更",
      "Diff": "差异对比",
      "Terminals": "终端",
      "Terminal": "终端",
      "New Terminal": "新建终端",
      "Browser": "浏览器",
      "Files": "文件",
      "Explorer": "文件浏览",
      "Output": "输出",
      "Problems": "问题",
      "Console": "控制台",
      "Logs": "日志",
      "Preview": "预览",
      "Code": "代码",
      "Plan": "实现计划",
      "Implementation Plan": "实现计划",
      "Task List": "任务清单",
      "Walkthrough": "验收说明",
      "Timeline": "时间线",
      "Outline": "大纲",

      // Actions & Conversation Controls
      "Send": "发送",
      "Stop": "停止",
      "Retry": "重试",
      "Regenerate": "重新生成",
      "Continue": "继续",
      "Approve": "批准",
      "Reject": "拒绝",
      "Proceed": "继续执行",
      "Review": "审查",
      "Accept": "接受",
      "Accept All": "全部接受",
      "Discard": "放弃",
      "Discard All": "全部放弃",
      "Cancel": "取消",
      "Confirm": "确认",
      "Save": "保存",
      "Apply": "应用",
      "Reset": "重置",
      "Close": "关闭",
      "Open": "打开",
      "Open in Editor": "在编辑器中打开",
      "Open in Terminal": "在终端中打开",
      "Reveal in Finder": "在访达中显示",
      "Show in Explorer": "在资源管理器中显示",
      "Copy": "复制",
      "Copied": "已复制",
      "Copied!": "已复制！",
      "Copy Path": "复制路径",
      "Paste": "粘贴",
      "Cut": "剪切",
      "Edit": "编辑",
      "Rename": "重命名",
      "Duplicate": "创建副本",
      "Delete": "删除",
      "Remove": "移除",
      "Clear": "清空",
      "Refresh": "刷新",
      "Reload": "重新加载",
      "Add": "添加",
      "Create": "创建",
      "New": "新建",
      "Import": "导入",
      "Export": "导出",
      "Install": "安装",
      "Uninstall": "卸载",
      "Update": "更新",
      "Check for Updates": "检查更新",
      "Check for Updates...": "检查更新…",
      "Back": "返回",
      "Next": "下一步",
      "Previous": "上一步",
      "Done": "完成",
      "Submit": "提交",
      "Expand": "展开",
      "Collapse": "收起",
      "Show More": "显示更多",
      "Show Less": "收起",
      "Details": "详情",
      "View Details": "查看详情",
      "Filter": "筛选",
      "Sort": "排序",
      "Run": "运行",
      "Run Now": "立即运行",
      "Pause": "暂停",
      "Resume": "恢复",

      // Auth, Statuses & Empty States
      "Sign in": "登录",
      "Sign In": "登录",
      "Log in": "登录",
      "Sign out": "退出登录",
      "Sign Out": "退出登录",
      "Log out": "退出登录",
      "Switch Account": "切换账号",
      "Loading...": "加载中…",
      "Thinking...": "思考中…",
      "Thought": "思考过程",
      "Running...": "运行中…",
      "Running": "运行中",
      "Completed": "已完成",
      "Failed": "失败",
      "Pending": "等待中",
      "Queued": "排队中",
      "Cancelled": "已取消",
      "Canceled": "已取消",
      "Idle": "空闲",
      "Connected": "已连接",
      "Disconnected": "已断开",
      "Online": "在线",
      "Offline": "离线",
      "Recent": "最近",
      "Today": "今天",
      "Yesterday": "昨天",
      "No results found": "未找到匹配结果",
      "No conversations yet": "暂无对话",
      "No scheduled tasks": "暂无定时任务",
      "No background tasks": "暂无后台任务",
      "No files changed": "暂无变更文件",
      "No artifacts": "暂无产物",
      "No agents running": "暂无运行中的智能体",
      "Ask anything...": "输入问题或指令…",
      "Ask Antigravity...": "向 Antigravity 提问…",
      "Type a message...": "输入消息…",
      "Search...": "搜索…",
      "Search conversations...": "搜索对话…",
      "Search settings...": "搜索设置…",
      "Restart Required": "需重启生效",
      "Restart Now": "立即重启",
      "Later": "稍后",
      "Learn more": "了解更多",
      "Documentation": "官方文档",
      "Docs": "官方文档",
      "Feedback": "意见反馈",
      "Version": "版本",
      "Status": "状态",
      "Actions": "操作",
      "Name": "名称",
      "Description": "描述",
      "Command": "命令",
      "Arguments": "参数",
      "Environment Variables": "环境变量",
      "Working Directory": "工作目录",
      "Schedule": "调度计划"
    });

    var SKIP_TAGS = {
      CODE: 1,
      PRE: 1,
      KBD: 1,
      SCRIPT: 1,
      STYLE: 1,
      TEXTAREA: 1,
      NOSCRIPT: 1,
      SVG: 1,
      MATH: 1
    };
    var SKIP_SELECTOR =
      'code, pre, kbd, script, style, textarea, [contenteditable="true"], [contenteditable=""], .monaco-editor, .xterm, [data-no-i18n]';
    var ATTRS = ["placeholder", "title", "aria-label"];

    function isSkippedElement(el) {
      if (!el || el.nodeType !== 1) return true;
      if (SKIP_TAGS[el.tagName]) return true;
      if (typeof el.closest === "function" && el.closest(SKIP_SELECTOR)) return true;
      return false;
    }

    function translateTextNode(node) {
      if (!node || node.nodeType !== 3) return;
      var parent = node.parentElement;
      if (!parent || isSkippedElement(parent)) return;
      var raw = node.nodeValue;
      if (!raw || raw.length > 120) return;
      var trimmed = raw.trim();
      if (!trimmed || trimmed.length > 96) return;
      var zh = DICT[trimmed];
      if (!zh) return;
      var idx = raw.indexOf(trimmed);
      if (idx < 0) return;
      var next = raw.slice(0, idx) + zh + raw.slice(idx + trimmed.length);
      if (next !== raw) {
        node.nodeValue = next;
      }
    }

    function translateElementAttrs(el) {
      if (!el || el.nodeType !== 1 || isSkippedElement(el)) return;
      for (var i = 0; i < ATTRS.length; i++) {
        var attr = ATTRS[i];
        var val = el.getAttribute(attr);
        if (!val || val.length > 120) continue;
        var trimmed = val.trim();
        var zh = DICT[trimmed];
        if (zh && zh !== val) {
          el.setAttribute(attr, zh);
        }
      }
    }

    function walkSubtree(root) {
      if (!root) return;
      if (root.nodeType === 3) {
        translateTextNode(root);
        return;
      }
      if (root.nodeType !== 1 || isSkippedElement(root)) return;
      translateElementAttrs(root);
      var walker = document.createTreeWalker(
        root,
        NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
        {
          acceptNode: function (n) {
            if (n.nodeType === 1) {
              if (SKIP_TAGS[n.tagName]) return NodeFilter.FILTER_REJECT;
              if (
                n.getAttribute("contenteditable") === "true" ||
                n.getAttribute("contenteditable") === "" ||
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
          translateElementAttrs(cur);
        }
      }
    }

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

    function startObserver() {
      try {
        if (!document.body) return;
        walkSubtree(document.body);
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
        obs.observe(document.body, {
          childList: true,
          subtree: true,
          characterData: true,
          attributes: true,
          attributeFilter: ATTRS
        });
      } catch (_) {}
    }

    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", startObserver, { once: true });
    } else {
      startObserver();
    }
  } catch (_) {}
})();"#
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
    let asar_key = "<key>Resources/app.asar</key>";
    let key_pos = plist_xml.find(asar_key)?;
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
            return write_atomic(info_plist_path, updated_xml.as_bytes(), false);
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

#[cfg(target_os = "macos")]
fn resign_macos_outer_bundle(bundle_root: &Path, backup_dir: &Path) -> Result<(), String> {
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
                fs::write(&tmp_ent_path, &ent_out.stdout)
                    .map_err(|e| format!("写入临时 entitlements 失败: {e}"))?;
                has_entitlements = true;
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
        return Err(format!(
            "本地重签 Antigravity.app 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
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

            let clean_orig_bytes = verify_clean_backup_asar(paths, &expected_orig_sha)?;
            let clean_archive = AsarArchive::parse(&clean_orig_bytes)?;
            (clean_archive, expected_orig_sha)
        } else {
            if !is_patchable_asar(&current_archive) {
                return Err("当前 Antigravity 版本目录结构不受支持，已安全跳过修改".to_string());
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

    write_atomic(&target.asar_path, &patched_asar_bytes, false)?;

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
            resign_macos_outer_bundle(bundle_root, &backup_dir)?;
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
    write_atomic(&target.asar_path, &clean_orig_bytes, false)?;

    let mut restored_full_signature = false;
    if target.bundle_root.is_some() {
        let orig_plist = backup_dir.join("Info.plist.orig");
        if orig_plist.is_file() {
            if let Some(ref dst_plist) = target.info_plist_path {
                let bytes = fs::read(&orig_plist)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_plist.display()))?;
                write_atomic(dst_plist, &bytes, false)?;
            }
        }

        let orig_bin = backup_dir.join("MacOS_Antigravity.orig");
        let orig_cr = backup_dir.join("CodeResources.orig");
        let mut restored_bin = false;
        let mut restored_cr = false;

        if orig_bin.is_file() {
            if let Some(ref dst_bin) = target.macos_bin_path {
                #[cfg(unix)]
                let prev_mode = fs::metadata(dst_bin)
                    .map(|m| m.permissions().mode())
                    .unwrap_or(0o755);
                let bytes = fs::read(&orig_bin)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_bin.display()))?;
                write_atomic(dst_bin, &bytes, false)?;
                #[cfg(unix)]
                {
                    let mode = if prev_mode & 0o111 != 0 {
                        prev_mode
                    } else {
                        0o755
                    };
                    let _ = fs::set_permissions(dst_bin, fs::Permissions::from_mode(mode));
                }
                restored_bin = true;
            }
        }

        if orig_cr.is_file() {
            if let Some(ref dst_cr) = target.code_resources_path {
                let bytes = fs::read(&orig_cr)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_cr.display()))?;
                write_atomic(dst_cr, &bytes, false)?;
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
            resign_macos_outer_bundle(bundle_root, &backup_dir)?;
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
}
