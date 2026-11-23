use crate::adapters::antigravity_i18n::sha256_hex;
use crate::adapters::{read_json_object_or_empty, AppMode, DesktopAppAdapter};
use crate::config::{write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncPlatform {
    Claude,
    Codex,
    Antigravity,
}

impl SyncPlatform {
    pub fn all() -> &'static [SyncPlatform] {
        &[
            SyncPlatform::Claude,
            SyncPlatform::Codex,
            SyncPlatform::Antigravity,
        ]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SyncPlatform::Claude => "Claude",
            SyncPlatform::Codex => "Codex",
            SyncPlatform::Antigravity => "Antigravity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedMcpSpec {
    pub transport: String, // "stdio" | "http"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub summary: String,
}

impl SyncedMcpSpec {
    fn fingerprint(&self) -> String {
        let mut s = String::new();
        s.push_str(&self.transport);
        s.push('|');
        if let Some(ref c) = self.command {
            s.push_str(c);
        }
        s.push('|');
        for a in &self.args {
            s.push_str(a);
            s.push('\x1f');
        }
        s.push('|');
        for (k, v) in &self.env {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push('\x1f');
        }
        s.push('|');
        if let Some(ref u) = self.url {
            s.push_str(u);
        }
        s.push('|');
        for (k, v) in &self.headers {
            s.push_str(k);
            s.push('=');
            s.push_str(v);
            s.push('\x1f');
        }
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformMcpItem {
    pub name: String,
    pub managed_by_tako: bool,
    pub platforms: Vec<SyncPlatform>,
    pub variants: HashMap<SyncPlatform, SyncedMcpSpec>,
    pub has_diff: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSkillItem {
    pub name: String,
    pub title: String,
    pub description: String,
    pub platforms: Vec<SyncPlatform>,
    pub file_counts: HashMap<SyncPlatform, usize>,
    pub modified_ms: HashMap<SyncPlatform, u64>,
    pub has_diff: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSyncPaths {
    pub mcp_path: String,
    pub skills_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiPlatformSyncSnapshot {
    pub mcp_items: Vec<PlatformMcpItem>,
    pub skill_items: Vec<PlatformSkillItem>,
    pub platform_paths: HashMap<SyncPlatform, PlatformSyncPaths>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchSyncOutcome {
    pub synced_items: usize,
    pub synced_writes: usize,
    pub snapshot: MultiPlatformSyncSnapshot,
}

fn is_builtin_tako_mcp(name: &str) -> bool {
    matches!(name, "tako" | "tako-web-page")
}

fn is_internal_platform_mcp(platform: SyncPlatform, name: &str) -> bool {
    platform == SyncPlatform::Codex && name == "node_repl"
}

fn active_claude_mcp_config_paths(paths: &AppPaths) -> Vec<PathBuf> {
    let mut out = vec![paths.claude_1p_dir.join("claude_desktop_config.json")];
    let p3 = paths.claude_3p_dir.join("claude_desktop_config.json");
    if p3.exists()
        || crate::adapters::claude::ClaudeAdapter.read_mode(paths) == AppMode::Tako
    {
        out.push(p3);
    }
    out
}

fn primary_mcp_path(paths: &AppPaths, platform: SyncPlatform) -> PathBuf {
    match platform {
        SyncPlatform::Claude => {
            if crate::adapters::claude::ClaudeAdapter.read_mode(paths) == AppMode::Tako {
                let p3 = paths.claude_3p_dir.join("claude_desktop_config.json");
                if p3.exists() {
                    return p3;
                }
            }
            paths.claude_1p_dir.join("claude_desktop_config.json")
        }
        SyncPlatform::Codex => paths.codex_dir.join("config.toml"),
        SyncPlatform::Antigravity => paths.agy_gemini_config_dir.join("mcp_config.json"),
    }
}

fn primary_skills_dir(paths: &AppPaths, platform: SyncPlatform) -> PathBuf {
    match platform {
        SyncPlatform::Claude => paths.home.join(".claude").join("skills"),
        SyncPlatform::Codex => paths.codex_dir.join("skills"),
        SyncPlatform::Antigravity => paths.agy_gemini_config_dir.join("skills"),
    }
}

fn extra_antigravity_skills_dirs(paths: &AppPaths) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let cli_dir = paths.home.join(".gemini").join("antigravity-cli").join("skills");
    if cli_dir.is_dir() {
        dirs.push(cli_dir);
    }
    let agy_dir = paths.home.join(".gemini").join("antigravity").join("skills");
    if agy_dir.is_dir() {
        dirs.push(agy_dir);
    }
    dirs
}

fn parse_json_mcp_spec(val: &Value) -> Option<SyncedMcpSpec> {
    let obj = val.as_object()?;
    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let url = obj
        .get("url")
        .or_else(|| obj.get("serverUrl"))
        .or_else(|| obj.get("httpUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let args: Vec<String> = obj
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut env = BTreeMap::new();
    if let Some(env_obj) = obj.get("env").and_then(Value::as_object) {
        for (k, v) in env_obj {
            if let Some(s) = v.as_str() {
                env.insert(k.clone(), s.to_string());
            } else if v.is_number() || v.is_boolean() {
                env.insert(k.clone(), v.to_string());
            }
        }
    }

    let mut headers = BTreeMap::new();
    if let Some(h_obj) = obj
        .get("headers")
        .or_else(|| obj.get("httpHeaders"))
        .and_then(Value::as_object)
    {
        for (k, v) in h_obj {
            if let Some(s) = v.as_str() {
                headers.insert(k.clone(), s.to_string());
            }
        }
    }

    if let Some(ref cmd) = command {
        let summary = if args.is_empty() {
            cmd.clone()
        } else {
            format!("{cmd} {}", args.join(" "))
        };
        Some(SyncedMcpSpec {
            transport: "stdio".to_string(),
            command,
            args,
            env,
            url: None,
            headers: BTreeMap::new(),
            summary,
        })
    } else {
        url.map(|u| SyncedMcpSpec {
            transport: "http".to_string(),
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            summary: u.clone(),
            url: Some(u),
            headers,
        })
    }
}

fn spec_to_json_value(spec: &SyncedMcpSpec) -> Value {
    let mut obj = Map::new();
    if spec.transport == "http" {
        if let Some(ref u) = spec.url {
            obj.insert("url".to_string(), Value::String(u.clone()));
        }
        if !spec.headers.is_empty() {
            let mut h = Map::new();
            for (k, v) in &spec.headers {
                h.insert(k.clone(), Value::String(v.clone()));
            }
            obj.insert("headers".to_string(), Value::Object(h));
        }
    } else {
        if let Some(ref cmd) = spec.command {
            obj.insert("command".to_string(), Value::String(cmd.clone()));
        }
        obj.insert(
            "args".to_string(),
            Value::Array(spec.args.iter().cloned().map(Value::String).collect()),
        );
        if !spec.env.is_empty() {
            let mut e = Map::new();
            for (k, v) in &spec.env {
                e.insert(k.clone(), Value::String(v.clone()));
            }
            obj.insert("env".to_string(), Value::Object(e));
        }
    }
    Value::Object(obj)
}

fn read_json_mcp_servers(path: &Path) -> BTreeMap<String, SyncedMcpSpec> {
    let mut out = BTreeMap::new();
    let Ok(root) = read_json_object_or_empty(path) else {
        return out;
    };
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return out;
    };
    for (name, val) in servers {
        if let Some(spec) = parse_json_mcp_spec(val) {
            out.insert(name.clone(), spec);
        }
    }
    out
}

fn read_codex_mcp_servers(path: &Path) -> BTreeMap<String, SyncedMcpSpec> {
    let mut out = BTreeMap::new();
    let Ok(raw) = fs::read_to_string(path) else {
        return out;
    };
    let Ok(doc) = raw.parse::<DocumentMut>() else {
        return out;
    };
    let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table) else {
        return out;
    };
    for (name, item) in servers.iter() {
        let Some(tbl) = item.as_table() else {
            continue;
        };
        let command = tbl
            .get("command")
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let url = tbl
            .get("url")
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let args: Vec<String> = tbl
            .get("args")
            .and_then(Item::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(TomlValue::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        let mut env = BTreeMap::new();
        if let Some(env_tbl) = tbl.get("env").and_then(Item::as_table) {
            for (k, v) in env_tbl.iter() {
                if let Some(s) = v.as_str() {
                    env.insert(k.to_string(), s.to_string());
                }
            }
        }
        let mut headers = BTreeMap::new();
        if let Some(h_tbl) = tbl
            .get("http_headers")
            .or_else(|| tbl.get("headers"))
            .and_then(Item::as_table)
        {
            for (k, v) in h_tbl.iter() {
                if let Some(s) = v.as_str() {
                    headers.insert(k.to_string(), s.to_string());
                }
            }
        }

        if let Some(ref cmd) = command {
            let summary = if args.is_empty() {
                cmd.clone()
            } else {
                format!("{cmd} {}", args.join(" "))
            };
            out.insert(
                name.to_string(),
                SyncedMcpSpec {
                    transport: "stdio".to_string(),
                    command,
                    args,
                    env,
                    url: None,
                    headers: BTreeMap::new(),
                    summary,
                },
            );
        } else if let Some(ref u) = url {
            out.insert(
                name.to_string(),
                SyncedMcpSpec {
                    transport: "http".to_string(),
                    command: None,
                    args: Vec::new(),
                    env: BTreeMap::new(),
                    url: Some(u.clone()),
                    headers,
                    summary: u.clone(),
                },
            );
        }
    }
    out
}

fn read_platform_mcps(paths: &AppPaths, platform: SyncPlatform) -> BTreeMap<String, SyncedMcpSpec> {
    match platform {
        SyncPlatform::Claude => {
            let mut merged = read_json_mcp_servers(&paths.claude_1p_dir.join("claude_desktop_config.json"));
            let p3 = paths.claude_3p_dir.join("claude_desktop_config.json");
            if p3.exists() {
                for (k, v) in read_json_mcp_servers(&p3) {
                    merged.entry(k).or_insert(v);
                }
            }
            merged
        }
        SyncPlatform::Codex => read_codex_mcp_servers(&paths.codex_dir.join("config.toml")),
        SyncPlatform::Antigravity => {
            read_json_mcp_servers(&paths.agy_gemini_config_dir.join("mcp_config.json"))
        }
    }
}

fn upsert_json_mcp_server(path: &Path, name: &str, spec: &SyncedMcpSpec) -> Result<(), String> {
    let mut root = if path.exists() {
        read_json_object_or_empty(path)?
    } else {
        Map::new()
    };
    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        *servers = Value::Object(Map::new());
    }
    if let Some(map) = servers.as_object_mut() {
        map.insert(name.to_string(), spec_to_json_value(spec));
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(path, &bytes, false)
}

fn remove_json_mcp_server(path: &Path, name: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_json_object_or_empty(path)?;
    let removed = root
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .and_then(|m| m.remove(name))
        .is_some();
    if !removed {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(&Value::Object(root))
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    write_atomic(path, &bytes, false)
}

fn upsert_codex_mcp_server(path: &Path, name: &str, spec: &SyncedMcpSpec) -> Result<(), String> {
    let raw = if path.exists() {
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?
    } else {
        String::new()
    };
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|e| format!("解析 {} TOML 失败: {e}", path.display()))?;
    if !doc.contains_key("mcp_servers") || !doc["mcp_servers"].is_table() {
        let mut t = Table::new();
        t.set_implicit(true);
        doc["mcp_servers"] = Item::Table(t);
    }

    let mut srv = Table::new();
    if spec.transport == "http" {
        if let Some(ref u) = spec.url {
            srv["url"] = toml_edit::value(u.clone());
        }
        if !spec.headers.is_empty() {
            let mut h_tbl = Table::new();
            for (k, v) in &spec.headers {
                h_tbl[k] = toml_edit::value(v.clone());
            }
            srv["http_headers"] = Item::Table(h_tbl);
        }
    } else {
        if let Some(ref cmd) = spec.command {
            srv["command"] = toml_edit::value(cmd.clone());
        }
        let mut arr = Array::new();
        for a in &spec.args {
            arr.push(a.as_str());
        }
        srv["args"] = toml_edit::value(arr);
        if !spec.env.is_empty() {
            let mut env_tbl = Table::new();
            for (k, v) in &spec.env {
                env_tbl[k] = toml_edit::value(v.clone());
            }
            srv["env"] = Item::Table(env_tbl);
        }
    }
    doc["mcp_servers"][name] = Item::Table(srv);
    write_atomic(path, doc.to_string().as_bytes(), false)
}

fn remove_codex_mcp_server(path: &Path, name: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let mut doc = raw
        .parse::<DocumentMut>()
        .map_err(|e| format!("解析 {} TOML 失败: {e}", path.display()))?;
    if let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) {
        if servers.remove(name).is_some() {
            write_atomic(path, doc.to_string().as_bytes(), false)?;
        }
    }
    Ok(())
}

fn write_platform_mcp(
    paths: &AppPaths,
    platform: SyncPlatform,
    name: &str,
    spec: &SyncedMcpSpec,
) -> Result<(), String> {
    match platform {
        SyncPlatform::Claude => {
            for cfg_path in active_claude_mcp_config_paths(paths) {
                upsert_json_mcp_server(&cfg_path, name, spec)?;
            }
            Ok(())
        }
        SyncPlatform::Codex => {
            upsert_codex_mcp_server(&paths.codex_dir.join("config.toml"), name, spec)
        }
        SyncPlatform::Antigravity => upsert_json_mcp_server(
            &paths.agy_gemini_config_dir.join("mcp_config.json"),
            name,
            spec,
        ),
    }
}

fn remove_platform_mcp(paths: &AppPaths, platform: SyncPlatform, name: &str) -> Result<(), String> {
    if is_builtin_tako_mcp(name) {
        return Err("Tako 内置 MCP 请在「Tako 内置工具」标签页中开关管理".to_string());
    }
    match platform {
        SyncPlatform::Claude => {
            for cfg_path in active_claude_mcp_config_paths(paths) {
                remove_json_mcp_server(&cfg_path, name)?;
            }
            Ok(())
        }
        SyncPlatform::Codex => {
            remove_codex_mcp_server(&paths.codex_dir.join("config.toml"), name)
        }
        SyncPlatform::Antigravity => {
            remove_json_mcp_server(&paths.agy_gemini_config_dir.join("mcp_config.json"), name)
        }
    }
}

#[derive(Debug, Clone)]
struct ScannedSkill {
    dir_path: PathBuf,
    title: String,
    description: String,
    file_count: usize,
    modified_ms: u64,
    skill_md_sha: String,
}

fn parse_skill_frontmatter(raw: &str, fallback_name: &str) -> (String, String) {
    let mut title = fallback_name.to_string();
    let mut desc = String::new();
    let trimmed = raw.trim_start();
    if let Some(rest) = trimmed.strip_prefix("---") {
        if let Some(end_idx) = rest.find("\n---") {
            let fm = &rest[..end_idx];
            for line in fm.lines() {
                let l = line.trim();
                if let Some(val) = l.strip_prefix("name:") {
                    let v = val.trim().trim_matches('"').trim_matches('\'').trim();
                    if !v.is_empty() {
                        title = v.to_string();
                    }
                } else if let Some(val) = l.strip_prefix("description:") {
                    let v = val.trim().trim_matches('"').trim_matches('\'').trim();
                    if !v.is_empty() {
                        desc = v.to_string();
                    }
                }
            }
        }
    }
    if desc.is_empty() {
        for line in raw.lines() {
            let l = line.trim();
            if l.is_empty() || l == "---" || l.starts_with('#') || l.contains(':') && l.len() < 40 {
                continue;
            }
            desc = l.chars().take(120).collect();
            break;
        }
    }
    (title, desc)
}

fn count_files_and_mtime(dir: &Path, depth: usize) -> (usize, u64) {
    if depth > 6 {
        return (0, 0);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut count = 0usize;
    let mut max_ms = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(mtime) = meta.modified() {
                if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                    max_ms = max_ms.max(dur.as_millis() as u64);
                }
            }
            if meta.is_dir() {
                let (c, m) = count_files_and_mtime(&path, depth + 1);
                count += c;
                max_ms = max_ms.max(m);
            } else if meta.is_file() {
                count += 1;
            }
        }
    }
    (count, max_ms)
}

fn scan_skills_dir(dir: &Path) -> BTreeMap<String, ScannedSkill> {
    let mut out = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let entry_path = entry.path();
        // Follow symlinks so symlinked skills (like ego-browser / chen-worker skills) are scanned cleanly
        let Ok(meta) = fs::metadata(&entry_path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let skill_md_path = entry_path.join("SKILL.md");
        if !skill_md_path.is_file() {
            continue;
        }
        let Ok(skill_bytes) = fs::read(&skill_md_path) else {
            continue;
        };
        let skill_text = String::from_utf8_lossy(&skill_bytes);
        let (title, description) = parse_skill_frontmatter(&skill_text, &name);
        let (file_count, modified_ms) = count_files_and_mtime(&entry_path, 0);
        out.insert(
            name,
            ScannedSkill {
                dir_path: entry_path,
                title,
                description,
                file_count,
                modified_ms,
                skill_md_sha: sha256_hex(&skill_bytes),
            },
        );
    }
    out
}

fn scan_platform_skills(paths: &AppPaths, platform: SyncPlatform) -> BTreeMap<String, ScannedSkill> {
    let mut merged = scan_skills_dir(&primary_skills_dir(paths, platform));
    if platform == SyncPlatform::Antigravity {
        for extra in extra_antigravity_skills_dirs(paths) {
            for (k, v) in scan_skills_dir(&extra) {
                merged.entry(k).or_insert(v);
            }
        }
    }
    merged
}

fn copy_dir_recursive_safe(src: &Path, dst: &Path, depth: usize) -> Result<(), String> {
    if depth > 8 {
        return Err("Skill 目录嵌套过深，已中止复制".to_string());
    }
    let src_canon = fs::canonicalize(src)
        .map_err(|e| format!("无法解析源 Skill 路径 {}: {e}", src.display()))?;
    if dst.exists() {
        if let Ok(dst_canon) = fs::canonicalize(dst) {
            if dst_canon == src_canon {
                // Already pointing to the exact same canonical directory
                return Ok(());
            }
        }
    }

    fs::create_dir_all(dst).map_err(|e| format!("创建目标目录 {} 失败: {e}", dst.display()))?;
    let entries = fs::read_dir(&src_canon)
        .map_err(|e| format!("读取源目录 {} 失败: {e}", src_canon.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".git" || name_str == "node_modules" || name_str == ".DS_Store" {
            continue;
        }
        let child_src = entry.path();
        let child_dst = dst.join(&name);
        let Ok(meta) = fs::metadata(&child_src) else {
            continue;
        };
        if meta.is_dir() {
            copy_dir_recursive_safe(&child_src, &child_dst, depth + 1)?;
        } else if meta.is_file() {
            fs::copy(&child_src, &child_dst).map_err(|e| {
                format!(
                    "复制 Skill 文件 {} -> {} 失败: {e}",
                    child_src.display(),
                    child_dst.display()
                )
            })?;
        }
    }
    Ok(())
}

fn validate_skill_name(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('.')
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err(format!("非法 Skill 名称: {name}"));
    }
    Ok(trimmed)
}

fn sync_skill_dir_to_target(src_dir: &Path, dst_dir: &Path) -> Result<(), String> {
    let src_canon = fs::canonicalize(src_dir)
        .map_err(|e| format!("无法解析源 Skill 路径 {}: {e}", src_dir.display()))?;
    if dst_dir.exists() {
        if let Ok(dst_canon) = fs::canonicalize(dst_dir) {
            if dst_canon == src_canon {
                return Ok(());
            }
        }
    }
    let parent = dst_dir
        .parent()
        .ok_or_else(|| format!("目标路径无效: {}", dst_dir.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("创建 Skills 目录 {} 失败: {e}", parent.display()))?;

    let tmp_dir = parent.join(format!(
        ".tako-sync-tmp-{}-{}",
        dst_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill"),
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&tmp_dir);
    copy_dir_recursive_safe(&src_canon, &tmp_dir, 0)?;

    if dst_dir.symlink_metadata().is_ok() {
        let meta = dst_dir.symlink_metadata().ok();
        if meta.is_some_and(|m| m.file_type().is_symlink() || m.is_file()) {
            fs::remove_file(dst_dir)
                .map_err(|e| format!("替换旧 Skill 符号链接 {} 失败: {e}", dst_dir.display()))?;
        } else {
            fs::remove_dir_all(dst_dir)
                .map_err(|e| format!("清理旧 Skill 目录 {} 失败: {e}", dst_dir.display()))?;
        }
    }
    fs::rename(&tmp_dir, dst_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&tmp_dir);
        format!("写入目标 Skill {} 失败: {e}", dst_dir.display())
    })
}

pub fn build_multi_platform_sync_snapshot(paths: &AppPaths) -> MultiPlatformSyncSnapshot {
    // 1. Aggregate MCPs across all 3 platforms
    let mut mcp_names: BTreeSet<String> = BTreeSet::new();
    let mut per_platform_mcp: HashMap<SyncPlatform, BTreeMap<String, SyncedMcpSpec>> =
        HashMap::new();
    for &p in SyncPlatform::all() {
        let map = read_platform_mcps(paths, p);
        for name in map.keys() {
            if !is_internal_platform_mcp(p, name) {
                mcp_names.insert(name.clone());
            }
        }
        per_platform_mcp.insert(p, map);
    }

    let mut mcp_items = Vec::new();
    for name in mcp_names {
        let mut platforms = Vec::new();
        let mut variants = HashMap::new();
        let mut fingerprints = BTreeSet::new();
        for &p in SyncPlatform::all() {
            if is_internal_platform_mcp(p, &name) {
                continue;
            }
            if let Some(spec) = per_platform_mcp.get(&p).and_then(|m| m.get(&name)) {
                platforms.push(p);
                fingerprints.insert(spec.fingerprint());
                variants.insert(p, spec.clone());
            }
        }
        if platforms.is_empty() {
            continue;
        }
        mcp_items.push(PlatformMcpItem {
            managed_by_tako: is_builtin_tako_mcp(&name),
            name,
            platforms,
            variants,
            has_diff: fingerprints.len() > 1,
        });
    }
    // Sort user MCPs first, then built-in Tako MCPs
    mcp_items.sort_by(|a, b| a.managed_by_tako.cmp(&b.managed_by_tako).then(a.name.cmp(&b.name)));

    // 2. Aggregate Skills across all 3 platforms
    let mut skill_names: BTreeSet<String> = BTreeSet::new();
    let mut per_platform_skills: HashMap<SyncPlatform, BTreeMap<String, ScannedSkill>> =
        HashMap::new();
    for &p in SyncPlatform::all() {
        let map = scan_platform_skills(paths, p);
        for name in map.keys() {
            skill_names.insert(name.clone());
        }
        per_platform_skills.insert(p, map);
    }

    let mut skill_items = Vec::new();
    for name in skill_names {
        let mut platforms = Vec::new();
        let mut file_counts = HashMap::new();
        let mut modified_ms = HashMap::new();
        let mut shas = BTreeSet::new();
        let mut title = name.clone();
        let mut description = String::new();
        let mut best_mtime = 0u64;

        for &p in SyncPlatform::all() {
            if let Some(sk) = per_platform_skills.get(&p).and_then(|m| m.get(&name)) {
                platforms.push(p);
                file_counts.insert(p, sk.file_count);
                modified_ms.insert(p, sk.modified_ms);
                shas.insert(sk.skill_md_sha.clone());
                if sk.modified_ms >= best_mtime || description.is_empty() {
                    best_mtime = sk.modified_ms;
                    title = sk.title.clone();
                    if !sk.description.is_empty() {
                        description = sk.description.clone();
                    }
                }
            }
        }

        skill_items.push(PlatformSkillItem {
            name,
            title,
            description,
            platforms,
            file_counts,
            modified_ms,
            has_diff: shas.len() > 1,
        });
    }

    let mut platform_paths = HashMap::new();
    for &p in SyncPlatform::all() {
        platform_paths.insert(
            p,
            PlatformSyncPaths {
                mcp_path: primary_mcp_path(paths, p).to_string_lossy().to_string(),
                skills_dir: primary_skills_dir(paths, p).to_string_lossy().to_string(),
            },
        );
    }

    MultiPlatformSyncSnapshot {
        mcp_items,
        skill_items,
        platform_paths,
    }
}

pub fn sync_mcp_to_platforms_for_paths(
    paths: &AppPaths,
    name: &str,
    source: SyncPlatform,
    targets: &[SyncPlatform],
) -> Result<MultiPlatformSyncSnapshot, String> {
    if is_builtin_tako_mcp(name) {
        return Err("Tako 内置 MCP 由「Tako 内置工具」自动管理，无需手动同步".to_string());
    }
    let src_map = read_platform_mcps(paths, source);
    let spec = src_map.get(name).ok_or_else(|| {
        format!(
            "在 {} 中未找到名为 `{name}` 的 MCP 配置",
            source.display_name()
        )
    })?;
    for &target in targets {
        if target == source {
            continue;
        }
        write_platform_mcp(paths, target, name, spec)?;
    }
    Ok(build_multi_platform_sync_snapshot(paths))
}

pub fn sync_all_mcps_for_paths(
    paths: &AppPaths,
    targets: &[SyncPlatform],
    preferred_source: Option<SyncPlatform>,
) -> Result<BatchSyncOutcome, String> {
    let snapshot = build_multi_platform_sync_snapshot(paths);
    let target_list: Vec<SyncPlatform> = if targets.is_empty() {
        SyncPlatform::all().to_vec()
    } else {
        targets.to_vec()
    };

    let mut synced_items = 0usize;
    let mut synced_writes = 0usize;
    for item in &snapshot.mcp_items {
        if item.managed_by_tako {
            continue;
        }
        let source = preferred_source
            .filter(|s| item.variants.contains_key(s))
            .or_else(|| item.platforms.first().copied());
        let Some(src) = source else {
            continue;
        };
        let Some(spec) = item.variants.get(&src) else {
            continue;
        };
        let src_fp = spec.fingerprint();
        let mut item_changed = false;
        for &t in &target_list {
            if t == src {
                continue;
            }
            let already_same = item
                .variants
                .get(&t)
                .is_some_and(|existing| existing.fingerprint() == src_fp);
            if already_same {
                continue;
            }
            write_platform_mcp(paths, t, &item.name, spec)?;
            synced_writes += 1;
            item_changed = true;
        }
        if item_changed {
            synced_items += 1;
        }
    }

    Ok(BatchSyncOutcome {
        synced_items,
        synced_writes,
        snapshot: build_multi_platform_sync_snapshot(paths),
    })
}

pub fn sync_skill_to_platforms_for_paths(
    paths: &AppPaths,
    name: &str,
    source: SyncPlatform,
    targets: &[SyncPlatform],
) -> Result<MultiPlatformSyncSnapshot, String> {
    let clean_name = validate_skill_name(name)?;
    let src_skills = scan_platform_skills(paths, source);
    let src_skill = src_skills.get(clean_name).ok_or_else(|| {
        format!(
            "在 {} 中未找到名为 `{clean_name}` 的 Skill",
            source.display_name()
        )
    })?;

    for &target in targets {
        if target == source {
            continue;
        }
        let dst_dir = primary_skills_dir(paths, target).join(clean_name);
        sync_skill_dir_to_target(&src_skill.dir_path, &dst_dir)?;
        if target == SyncPlatform::Antigravity {
            // Also keep Antigravity CLI skills directory in sync if it exists on disk
            for extra in extra_antigravity_skills_dirs(paths) {
                let extra_dst = extra.join(clean_name);
                let _ = sync_skill_dir_to_target(&src_skill.dir_path, &extra_dst);
            }
        }
    }

    Ok(build_multi_platform_sync_snapshot(paths))
}

pub fn sync_all_skills_for_paths(
    paths: &AppPaths,
    targets: &[SyncPlatform],
    preferred_source: Option<SyncPlatform>,
) -> Result<BatchSyncOutcome, String> {
    let mut per_platform_skills: HashMap<SyncPlatform, BTreeMap<String, ScannedSkill>> =
        HashMap::new();
    let mut all_names: BTreeSet<String> = BTreeSet::new();
    for &p in SyncPlatform::all() {
        let map = scan_platform_skills(paths, p);
        for k in map.keys() {
            all_names.insert(k.clone());
        }
        per_platform_skills.insert(p, map);
    }

    let target_list: Vec<SyncPlatform> = if targets.is_empty() {
        SyncPlatform::all().to_vec()
    } else {
        targets.to_vec()
    };

    let mut synced_items = 0usize;
    let mut synced_writes = 0usize;

    for name in all_names {
        let Ok(clean_name) = validate_skill_name(&name) else {
            continue;
        };
        let source = preferred_source
            .filter(|s| {
                per_platform_skills
                    .get(s)
                    .is_some_and(|m| m.contains_key(clean_name))
            })
            .or_else(|| {
                // Default: pick the platform with the newest modification timestamp
                SyncPlatform::all()
                    .iter()
                    .copied()
                    .filter_map(|p| {
                        per_platform_skills
                            .get(&p)
                            .and_then(|m| m.get(clean_name))
                            .map(|sk| (p, sk.modified_ms))
                    })
                    .max_by_key(|(_, m)| *m)
                    .map(|(p, _)| p)
            });

        let Some(src) = source else {
            continue;
        };
        let Some(src_skill) = per_platform_skills
            .get(&src)
            .and_then(|m| m.get(clean_name))
        else {
            continue;
        };

        let mut item_changed = false;
        for &t in &target_list {
            if t == src {
                continue;
            }
            let same_sha = per_platform_skills
                .get(&t)
                .and_then(|m| m.get(clean_name))
                .is_some_and(|existing| existing.skill_md_sha == src_skill.skill_md_sha);
            if same_sha {
                continue;
            }
            let dst_dir = primary_skills_dir(paths, t).join(clean_name);
            sync_skill_dir_to_target(&src_skill.dir_path, &dst_dir)?;
            if t == SyncPlatform::Antigravity {
                for extra in extra_antigravity_skills_dirs(paths) {
                    let _ = sync_skill_dir_to_target(&src_skill.dir_path, &extra.join(clean_name));
                }
            }
            synced_writes += 1;
            item_changed = true;
        }
        if item_changed {
            synced_items += 1;
        }
    }

    Ok(BatchSyncOutcome {
        synced_items,
        synced_writes,
        snapshot: build_multi_platform_sync_snapshot(paths),
    })
}

pub fn remove_synced_item_for_paths(
    paths: &AppPaths,
    kind: &str,
    name: &str,
    platform: SyncPlatform,
) -> Result<MultiPlatformSyncSnapshot, String> {
    match kind {
        "mcp" => {
            remove_platform_mcp(paths, platform, name)?;
        }
        "skill" => {
            let clean_name = validate_skill_name(name)?;
            let mut candidates = vec![primary_skills_dir(paths, platform).join(clean_name)];
            if platform == SyncPlatform::Antigravity {
                for extra in extra_antigravity_skills_dirs(paths) {
                    candidates.push(extra.join(clean_name));
                }
            }
            for target in candidates {
                if let Ok(meta) = target.symlink_metadata() {
                    if meta.file_type().is_symlink() || meta.is_file() {
                        fs::remove_file(&target)
                            .map_err(|e| format!("移除 Skill {} 失败: {e}", target.display()))?;
                    } else if meta.is_dir() {
                        fs::remove_dir_all(&target)
                            .map_err(|e| format!("移除 Skill {} 失败: {e}", target.display()))?;
                    }
                }
            }
        }
        other => return Err(format!("不支持的同步对象类型: {other}")),
    }
    Ok(build_multi_platform_sync_snapshot(paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn mcp_cross_platform_sync_converts_json_and_toml_and_preserves_other_entries() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        // Seed a custom stdio MCP in Claude and a custom http MCP in Codex
        let claude_cfg = paths.claude_1p_dir.join("claude_desktop_config.json");
        write_atomic(
            &claude_cfg,
            br#"{"mcpServers":{"filesystem":{"command":"npx","args":["-y","@modelcontextprotocol/server-filesystem","/tmp"],"env":{"DEBUG":"1"}},"tako":{"command":"tako","args":["mcp-server"]}}}"#,
            false,
        )
        .unwrap();

        let codex_cfg = paths.codex_dir.join("config.toml");
        write_atomic(
            &codex_cfg,
            b"model = \"gpt-6.2-sol\"\n\n[mcp_servers.remote_docs]\nurl = \"https://docs.example.com/mcp\"\n\n[mcp_servers.remote_docs.http_headers]\nAuthorization = \"Bearer test\"\n",
            false,
        )
        .unwrap();

        // Batch sync all custom MCPs across Claude, Codex, Antigravity
        let outcome = sync_all_mcps_for_paths(&paths, SyncPlatform::all(), None).unwrap();
        assert_eq!(outcome.synced_items, 2);
        assert_eq!(outcome.synced_writes, 4);

        // Verify Codex got `filesystem` without losing `model = "gpt-6.2-sol"` or `remote_docs`
        let codex_text = fs::read_to_string(&codex_cfg).unwrap();
        assert!(codex_text.contains("model = \"gpt-6.2-sol\""));
        assert!(codex_text.contains("[mcp_servers.filesystem]"));
        assert!(codex_text.contains("DEBUG = \"1\""));

        // Verify Antigravity got both custom MCPs, NOT built-in `tako`
        let agy_mcps = read_platform_mcps(&paths, SyncPlatform::Antigravity);
        assert!(agy_mcps.contains_key("filesystem"));
        assert!(agy_mcps.contains_key("remote_docs"));
        assert!(!agy_mcps.contains_key("tako"));
        assert_eq!(
            agy_mcps["remote_docs"].headers.get("Authorization").map(String::as_str),
            Some("Bearer test")
        );

        // Remove `filesystem` from Antigravity
        let after_rm = remove_synced_item_for_paths(
            &paths,
            "mcp",
            "filesystem",
            SyncPlatform::Antigravity,
        )
        .unwrap();
        let fs_item = after_rm.mcp_items.iter().find(|i| i.name == "filesystem").unwrap();
        assert!(!fs_item.platforms.contains(&SyncPlatform::Antigravity));
        assert!(fs_item.platforms.contains(&SyncPlatform::Claude));
        assert!(fs_item.platforms.contains(&SyncPlatform::Codex));
    }

    #[test]
    fn skill_cross_platform_sync_copies_files_and_detects_parity() {
        let tmp = TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        let src_skill = paths.home.join(".claude").join("skills").join("code-review");
        fs::create_dir_all(src_skill.join("scripts")).unwrap();
        fs::write(
            src_skill.join("SKILL.md"),
            "---\nname: code-review\ndescription: Review pull requests cleanly\n---\n# Code Review\n",
        )
        .unwrap();
        fs::write(src_skill.join("scripts").join("lint.sh"), "#!/bin/sh\necho ok\n").unwrap();

        let snap0 = build_multi_platform_sync_snapshot(&paths);
        assert_eq!(snap0.skill_items.len(), 1);
        assert_eq!(snap0.skill_items[0].platforms, vec![SyncPlatform::Claude]);
        assert_eq!(snap0.skill_items[0].description, "Review pull requests cleanly");

        let synced = sync_skill_to_platforms_for_paths(
            &paths,
            "code-review",
            SyncPlatform::Claude,
            &[SyncPlatform::Codex, SyncPlatform::Antigravity],
        )
        .unwrap();
        let item = synced.skill_items.iter().find(|s| s.name == "code-review").unwrap();
        assert_eq!(item.platforms.len(), 3);
        assert!(!item.has_diff);
        assert!(paths
            .codex_dir
            .join("skills")
            .join("code-review")
            .join("scripts")
            .join("lint.sh")
            .is_file());
        assert!(paths
            .agy_gemini_config_dir
            .join("skills")
            .join("code-review")
            .join("SKILL.md")
            .is_file());
    }
}
