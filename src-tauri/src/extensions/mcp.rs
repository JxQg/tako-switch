use super::{
    apply_file_updates, extension_store_path, mcp_config_paths, read_json_store,
    read_optional_file, store_lock, ExtPlatform, FileUpdate,
};
use crate::config::AppPaths;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value as TomlValue};

fn default_transport() -> String {
    "stdio".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSpec {
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for McpSpec {
    fn default() -> Self {
        Self {
            transport: default_transport(),
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub spec: McpSpec,
    #[serde(default)]
    pub platforms: Vec<ExtPlatform>,
    #[serde(default)]
    pub variants: BTreeMap<ExtPlatform, McpSpec>,
    #[serde(default)]
    pub has_diff: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStore {
    #[serde(default)]
    pub entries: Vec<McpEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHubSnapshot {
    #[serde(default)]
    pub entries: Vec<McpEntry>,
    #[serde(default)]
    pub platform_paths: BTreeMap<ExtPlatform, Vec<String>>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub fn mcp_store_path(paths: &AppPaths) -> PathBuf {
    extension_store_path(paths, "mcp.json")
}

pub fn is_reserved_mcp_id(id: &str) -> bool {
    matches!(
        id.to_ascii_lowercase().as_str(),
        "tako" | "tako-web-page" | "node_repl"
    )
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
        return Err("MCP 服务标识不能为空或包含控制字符，长度不能超过 512 字节".into());
    }
    if is_reserved_mcp_id(id) {
        return Err(format!(
            "`{id}` 是内置服务，不能在通用 MCP 中创建、修改或删除"
        ));
    }
    Ok(())
}

fn normalize_spec(mut spec: McpSpec) -> Result<McpSpec, String> {
    spec.command = spec
        .command
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    spec.url = spec
        .url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    match spec.transport.as_str() {
        "stdio" if spec.command.is_some() && spec.url.is_none() => {}
        "http" if spec.url.is_some() && spec.command.is_none() => {
            let url = reqwest::Url::parse(spec.url.as_deref().unwrap_or_default())
                .map_err(|err| format!("MCP 地址无效: {err}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err("MCP 地址必须使用 http 或 https".into());
            }
        }
        _ => return Err("stdio 服务需要命令，HTTP 服务需要地址，不能同时配置两种传输".into()),
    }
    if spec
        .extra
        .keys()
        .any(|key| KNOWN_KEYS.contains(&key.as_str()))
    {
        return Err("附加配置不能覆盖 MCP 传输或启用状态字段".into());
    }
    Ok(spec)
}

const KNOWN_KEYS: &[&str] = &[
    "command",
    "args",
    "env",
    "url",
    "httpUrl",
    "serverUrl",
    "headers",
    "httpHeaders",
    "http_headers",
    "transport",
    "enabled",
    "disabled",
    "type",
];

fn string_map(value: Option<&Value>, label: &str) -> Result<BTreeMap<String, String>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{label} 必须是键值对象"))?;
    object
        .iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                _ => return Err(format!("{label}.{key} 必须是文本、数字或布尔值")),
            };
            Ok((key.clone(), text))
        })
        .collect()
}

fn spec_from_value(value: &Value) -> Result<McpSpec, String> {
    let object = value.as_object().ok_or("MCP 配置必须是 JSON 对象")?;
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = object
        .get("url")
        .or_else(|| object.get("httpUrl"))
        .or_else(|| object.get("serverUrl"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let args = match object.get("args") {
        None => Vec::new(),
        Some(Value::Array(args)) => args
            .iter()
            .map(|arg| {
                arg.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "args 中的每一项都必须是文本".to_string())
            })
            .collect::<Result<_, _>>()?,
        _ => return Err("args 必须是文本数组".into()),
    };
    let extra = object
        .iter()
        .filter(|(key, _)| !KNOWN_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    normalize_spec(McpSpec {
        transport: if url.is_some() { "http" } else { "stdio" }.into(),
        command,
        url,
        args,
        env: string_map(object.get("env"), "env")?,
        headers: string_map(
            object
                .get("headers")
                .or_else(|| object.get("http_headers"))
                .or_else(|| object.get("httpHeaders")),
            "headers",
        )?,
        extra,
    })
}

fn toml_value_to_json(value: &TomlValue) -> Value {
    match value {
        TomlValue::String(value) => Value::String(value.value().clone()),
        TomlValue::Integer(value) => Value::from(*value.value()),
        TomlValue::Float(value) => Value::from(*value.value()),
        TomlValue::Boolean(value) => Value::Bool(*value.value()),
        TomlValue::Datetime(value) => Value::String(value.value().to_string()),
        TomlValue::Array(values) => Value::Array(values.iter().map(toml_value_to_json).collect()),
        TomlValue::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_string(), toml_value_to_json(value)))
                .collect(),
        ),
    }
}

fn toml_item_to_json(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(value) => toml_value_to_json(value),
        Item::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.to_string(), toml_item_to_json(item)))
                .collect(),
        ),
        Item::ArrayOfTables(tables) => Value::Array(
            tables
                .iter()
                .map(|table| {
                    Value::Object(
                        table
                            .iter()
                            .map(|(key, item)| (key.to_string(), toml_item_to_json(item)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn json_to_toml(value: &Value) -> Result<TomlValue, String> {
    match value {
        Value::String(value) => Ok(TomlValue::from(value.clone())),
        Value::Bool(value) => Ok(TomlValue::from(*value)),
        Value::Number(value) => value
            .as_i64()
            .map(TomlValue::from)
            .or_else(|| value.as_f64().map(TomlValue::from))
            .ok_or_else(|| "TOML 数字超出范围".into()),
        Value::Array(values) => {
            let mut array = toml_edit::Array::new();
            for value in values {
                array.push(json_to_toml(value)?);
            }
            Ok(TomlValue::Array(array))
        }
        Value::Object(values) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, value) in values {
                table.insert(key, json_to_toml(value)?);
            }
            Ok(TomlValue::InlineTable(table))
        }
        Value::Null => Err("Codex TOML 不支持 null 配置值".into()),
    }
}

fn set_toml_field(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    value: &Value,
) -> Result<(), String> {
    if let Some(existing) = table.get_mut(key) {
        if toml_item_to_json(existing) == *value {
            return Ok(());
        }
        if let Some(values) = value.as_object() {
            if let Some(nested) = existing.as_table_like_mut() {
                let removed: Vec<_> = nested
                    .iter()
                    .filter(|(key, _)| !values.contains_key(*key))
                    .map(|(key, _)| key.to_string())
                    .collect();
                for key in removed {
                    nested.remove(&key);
                }
                for (key, value) in values {
                    set_toml_field(nested, key, value)?;
                }
                return Ok(());
            }
        }
        let mut next = json_to_toml(value)?;
        if let Some(previous) = existing.as_value() {
            *next.decor_mut() = previous.decor().clone();
        }
        *existing = Item::Value(next);
    } else {
        table.insert(key, Item::Value(json_to_toml(value)?));
    }
    Ok(())
}

fn native_spec(spec: &McpSpec, platform: ExtPlatform) -> Map<String, Value> {
    let mut object: Map<_, _> = spec.extra.clone().into_iter().collect();
    if spec.transport == "http" {
        object.insert(
            "url".into(),
            Value::String(spec.url.clone().unwrap_or_default()),
        );
        if platform == ExtPlatform::Claude {
            object.insert("type".into(), Value::String("http".into()));
        }
        if !spec.headers.is_empty() {
            object.insert(
                if platform == ExtPlatform::Codex {
                    "http_headers"
                } else {
                    "headers"
                }
                .into(),
                serde_json::json!(spec.headers),
            );
        }
    } else {
        object.insert(
            "command".into(),
            Value::String(spec.command.clone().unwrap_or_default()),
        );
        object.insert("args".into(), serde_json::json!(spec.args));
        if !spec.env.is_empty() {
            object.insert("env".into(), serde_json::json!(spec.env));
        }
    }
    object
}

fn read_native_entries(path: &Path, platform: ExtPlatform) -> Result<Map<String, Value>, String> {
    let bytes = read_optional_file(path)?.unwrap_or_default();
    if bytes.is_empty() {
        return Ok(Map::new());
    }
    if platform == ExtPlatform::Codex {
        let raw = std::str::from_utf8(&bytes)
            .map_err(|err| format!("{} 不是 UTF-8: {err}", path.display()))?;
        let document = raw
            .trim_start_matches('\u{feff}')
            .parse::<DocumentMut>()
            .map_err(|err| format!("解析 {} 失败: {err}", path.display()))?;
        match document.get("mcp_servers") {
            None => Ok(Map::new()),
            Some(item) => toml_item_to_json(item)
                .as_object()
                .cloned()
                .ok_or_else(|| format!("{} 的 mcp_servers 必须是表", path.display())),
        }
    } else {
        let object = read_json_root(&bytes, path)?;
        match object.get("mcpServers") {
            None => Ok(Map::new()),
            Some(value) => value
                .as_object()
                .cloned()
                .ok_or_else(|| format!("{} 的 mcpServers 必须是对象", path.display())),
        }
    }
}

fn read_json_root(bytes: &[u8], path: &Path) -> Result<Map<String, Value>, String> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| format!("解析 {} 失败: {err}", path.display()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{} 必须是 JSON 对象", path.display()))
}

fn snapshot_unlocked(paths: &AppPaths) -> Result<McpHubSnapshot, String> {
    let store: McpStore = read_json_store(&mcp_store_path(paths));
    let mut entries: BTreeMap<String, McpEntry> = store
        .entries
        .into_iter()
        .filter(|entry| validate_id(&entry.id).is_ok())
        .map(|mut entry| {
            entry.platforms.clear();
            entry.variants.clear();
            entry.has_diff = false;
            if entry.name.is_empty() {
                entry.name = entry.id.clone();
            }
            (entry.id.clone(), entry)
        })
        .collect();
    let mut snapshot = McpHubSnapshot::default();
    for &platform in ExtPlatform::all() {
        let targets = mcp_config_paths(paths, platform);
        snapshot.platform_paths.insert(
            platform,
            targets
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        );
        for path in targets {
            let servers = match read_native_entries(&path, platform) {
                Ok(servers) => servers,
                Err(err) => {
                    snapshot.warnings.push(err);
                    continue;
                }
            };
            for (id, value) in servers {
                if is_reserved_mcp_id(&id) {
                    continue;
                }
                if let Err(err) = validate_id(&id) {
                    snapshot
                        .warnings
                        .push(format!("跳过 {} 中的 `{id}`: {err}", path.display()));
                    continue;
                }
                let spec = match spec_from_value(&value) {
                    Ok(spec) => spec,
                    Err(err) => {
                        snapshot
                            .warnings
                            .push(format!("{} 中的 `{id}`: {err}", path.display()));
                        continue;
                    }
                };
                let entry = entries.entry(id.clone()).or_insert_with(|| McpEntry {
                    id: id.clone(),
                    name: id,
                    spec: spec.clone(),
                    ..McpEntry::default()
                });
                if normalize_spec(entry.spec.clone()).is_err() {
                    entry.spec = spec.clone();
                }
                entry.has_diff |= entry.spec != spec
                    || entry
                        .variants
                        .get(&platform)
                        .is_some_and(|variant| variant != &spec);
                entry.variants.entry(platform).or_insert(spec);
                let enabled = value
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
                    && !value
                        .get("disabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                if enabled && !entry.platforms.contains(&platform) {
                    entry.platforms.push(platform);
                }
            }
        }
    }
    snapshot.entries = entries.into_values().collect();
    apply_file_updates(
        paths,
        vec![FileUpdate::store(
            mcp_store_path(paths),
            &McpStore {
                entries: snapshot.entries.clone(),
            },
        )?],
    )?;
    Ok(snapshot)
}

pub fn get_mcp_hub_snapshot_for_paths(paths: &AppPaths) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    snapshot_unlocked(paths)
}

fn stage_native_changes(
    paths: &AppPaths,
    entries: &[McpEntry],
    removals: &[String],
    only_platform: Option<ExtPlatform>,
) -> Result<Vec<FileUpdate>, String> {
    let mut updates = Vec::new();
    for &platform in ExtPlatform::all() {
        if only_platform.is_some_and(|target| target != platform) {
            continue;
        }
        for path in mcp_config_paths(paths, platform) {
            let before = read_optional_file(&path)?;
            let enabled: Vec<_> = entries
                .iter()
                .filter(|entry| entry.platforms.contains(&platform))
                .collect();
            if before.is_none() && enabled.is_empty() {
                continue;
            }
            let raw = before.as_deref().unwrap_or_default();
            let bytes = if platform == ExtPlatform::Codex {
                let text = std::str::from_utf8(raw)
                    .map_err(|err| format!("{} 不是 UTF-8: {err}", path.display()))?;
                let mut document = text
                    .trim_start_matches('\u{feff}')
                    .parse::<DocumentMut>()
                    .map_err(|err| format!("解析 {} 失败: {err}", path.display()))?;
                let original = document.to_string();
                if document.get("mcp_servers").is_none() && !enabled.is_empty() {
                    let mut table = Table::new();
                    table.set_implicit(true);
                    document["mcp_servers"] = Item::Table(table);
                }
                if let Some(item) = document.get_mut("mcp_servers") {
                    let servers = item
                        .as_table_like_mut()
                        .ok_or("mcp_servers 必须是 TOML 表")?;
                    for id in removals {
                        servers.remove(id);
                    }
                    for entry in entries {
                        if !entry.platforms.contains(&platform) {
                            servers.remove(&entry.id);
                            continue;
                        }
                        if servers.get(&entry.id).is_none() {
                            servers.insert(&entry.id, Item::Table(Table::new()));
                        }
                        let server = servers
                            .get_mut(&entry.id)
                            .and_then(Item::as_table_like_mut)
                            .ok_or_else(|| format!("MCP `{}` 必须是 TOML 表", entry.id))?;
                        let fields = native_spec(&entry.spec, platform);
                        for key in KNOWN_KEYS {
                            if !fields.contains_key(*key) {
                                server.remove(key);
                            }
                        }
                        for (key, value) in fields {
                            set_toml_field(server, &key, &value)?;
                        }
                    }
                }
                let next = document.to_string();
                if next == original {
                    continue;
                }
                next.into_bytes()
            } else {
                let mut root = read_json_root(raw, &path)?;
                let original = root.clone();
                if root.get("mcpServers").is_none() && !enabled.is_empty() {
                    root.insert("mcpServers".into(), Value::Object(Map::new()));
                }
                if let Some(value) = root.get_mut("mcpServers") {
                    let servers = value.as_object_mut().ok_or("mcpServers 必须是 JSON 对象")?;
                    for id in removals {
                        servers.remove(id);
                    }
                    for entry in entries {
                        if !entry.platforms.contains(&platform) {
                            servers.remove(&entry.id);
                            continue;
                        }
                        let value = servers
                            .entry(entry.id.clone())
                            .or_insert_with(|| Value::Object(Map::new()));
                        let server = value
                            .as_object_mut()
                            .ok_or_else(|| format!("MCP `{}` 必须是 JSON 对象", entry.id))?;
                        for key in KNOWN_KEYS {
                            server.remove(*key);
                        }
                        server.extend(native_spec(&entry.spec, platform));
                    }
                }
                if root == original {
                    continue;
                }
                super::json_bytes(&root)?
            };
            let key = match platform {
                ExtPlatform::Claude if path == paths.home.join(".claude.json") => {
                    "claude_code_mcp.json"
                }
                ExtPlatform::Claude if path.starts_with(&paths.claude_3p_dir) => {
                    "3p_claude_desktop_config.json"
                }
                ExtPlatform::Claude => "1p_claude_desktop_config.json",
                ExtPlatform::Codex => "config.toml",
                ExtPlatform::Antigravity => "mcp_config.json",
            };
            updates.push(FileUpdate::client(path, bytes, platform, key));
        }
    }
    Ok(updates)
}

fn save_entries(
    paths: &AppPaths,
    snapshot: &McpHubSnapshot,
    changed: &[McpEntry],
    removals: &[String],
) -> Result<(), String> {
    let mut updates = stage_native_changes(paths, changed, removals, None)?;
    updates.push(FileUpdate::store(
        mcp_store_path(paths),
        &McpStore {
            entries: snapshot.entries.clone(),
        },
    )?);
    apply_file_updates(paths, updates)
}

pub fn upsert_mcp_entry_for_paths(
    paths: &AppPaths,
    mut entry: McpEntry,
) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    validate_id(&entry.id)?;
    entry.spec = normalize_spec(entry.spec)?;
    entry.name = if entry.name.trim().is_empty() {
        entry.id.clone()
    } else {
        entry.name.trim().into()
    };
    entry.platforms.sort();
    entry.platforms.dedup();
    entry.variants.clear();
    entry.has_diff = false;
    let mut snapshot = snapshot_unlocked(paths)?;
    entry = aligned_entry(paths, &entry, None)?;
    snapshot.entries.retain(|existing| existing.id != entry.id);
    snapshot.entries.push(entry.clone());
    save_entries(paths, &snapshot, &[entry], &[])?;
    snapshot_unlocked(paths)
}

pub fn toggle_mcp_platform_for_paths(
    paths: &AppPaths,
    id: &str,
    platform: ExtPlatform,
    enabled: bool,
) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    validate_id(id)?;
    let mut snapshot = snapshot_unlocked(paths)?;
    let entry = snapshot
        .entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or("未找到该 MCP")?;
    if enabled && !entry.platforms.contains(&platform) {
        entry.platforms.push(platform);
    }
    if !enabled {
        entry.platforms.retain(|value| *value != platform);
    }
    let changed = entry.clone();
    // A single-column switch must not overwrite the other platforms' divergent parameters.
    let mut updates = stage_native_changes(paths, &[changed], &[], Some(platform))?;
    updates.push(FileUpdate::store(
        mcp_store_path(paths),
        &McpStore {
            entries: snapshot.entries.clone(),
        },
    )?);
    apply_file_updates(paths, updates)?;
    snapshot_unlocked(paths)
}

pub fn delete_mcp_entry_for_paths(paths: &AppPaths, id: &str) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    validate_id(id)?;
    let mut snapshot = snapshot_unlocked(paths)?;
    snapshot.entries.retain(|entry| entry.id != id);
    save_entries(paths, &snapshot, &[], &[id.into()])?;
    snapshot_unlocked(paths)
}

fn aligned_entry(
    paths: &AppPaths,
    entry: &McpEntry,
    source: Option<ExtPlatform>,
) -> Result<McpEntry, String> {
    let mut aligned = entry.clone();
    if let Some(source) = source {
        aligned.spec = entry
            .variants
            .get(&source)
            .cloned()
            .ok_or("所选来源平台没有该 MCP 配置")?;
    }
    for &platform in ExtPlatform::all() {
        for path in mcp_config_paths(paths, platform) {
            if let Some(value) = read_native_entries(&path, platform)?.get(&entry.id) {
                for (key, value) in spec_from_value(value)?.extra {
                    aligned.spec.extra.entry(key).or_insert(value);
                }
            }
        }
    }
    aligned.spec = normalize_spec(aligned.spec)?;
    aligned.has_diff = false;
    Ok(aligned)
}

pub fn align_mcp_entry_for_paths(
    paths: &AppPaths,
    id: &str,
    source: Option<ExtPlatform>,
) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    validate_id(id)?;
    let mut snapshot = snapshot_unlocked(paths)?;
    let entry = snapshot
        .entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or("未找到该 MCP")?;
    *entry = aligned_entry(paths, entry, source)?;
    let changed = entry.clone();
    save_entries(paths, &snapshot, &[changed], &[])?;
    snapshot_unlocked(paths)
}

pub fn align_all_mcp_entries_for_paths(paths: &AppPaths) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    let mut snapshot = snapshot_unlocked(paths)?;
    let changed = snapshot
        .entries
        .iter()
        .filter(|entry| entry.has_diff)
        .map(|entry| aligned_entry(paths, entry, None))
        .collect::<Result<Vec<_>, _>>()?;
    for entry in &mut snapshot.entries {
        if let Some(aligned) = changed.iter().find(|aligned| aligned.id == entry.id) {
            *entry = aligned.clone();
        }
    }
    for entry in &mut snapshot.entries {
        entry.has_diff = false;
    }
    save_entries(paths, &snapshot, &changed, &[])?;
    snapshot_unlocked(paths)
}

fn parsed_entry(id: &str, name: Option<&str>, value: &Value) -> Result<McpEntry, String> {
    validate_id(id)?;
    Ok(McpEntry {
        id: id.into(),
        name: name.unwrap_or(id).into(),
        spec: spec_from_value(value)?,
        ..McpEntry::default()
    })
}

pub fn parse_mcp_snippet(snippet: &str) -> Result<Vec<McpEntry>, String> {
    let text = snippet.trim().trim_start_matches('\u{feff}');
    let mut entries = Vec::new();
    if text.starts_with('{') {
        let root: Value =
            serde_json::from_str(text).map_err(|err| format!("JSON 格式错误: {err}"))?;
        if let Some(servers) = root.get("mcpServers") {
            for (id, value) in servers.as_object().ok_or("mcpServers 必须是对象")? {
                entries.push(parsed_entry(id, None, value)?);
            }
        } else if root.get("command").is_some()
            || root.get("url").is_some()
            || root.get("httpUrl").is_some()
            || root.get("serverUrl").is_some()
            || root.get("spec").is_some()
        {
            let id = root
                .get("id")
                .or_else(|| root.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("custom-mcp");
            let mut value = root.get("spec").unwrap_or(&root).clone();
            if let Some(object) = value.as_object_mut() {
                object.remove("id");
                object.remove("name");
            }
            if root.get("spec").is_some() {
                let spec: McpSpec =
                    serde_json::from_value(value).map_err(|err| format!("MCP 配置错误: {err}"))?;
                validate_id(id)?;
                entries.push(McpEntry {
                    id: id.into(),
                    name: id.into(),
                    spec: normalize_spec(spec)?,
                    ..McpEntry::default()
                });
            } else {
                entries.push(parsed_entry(
                    id,
                    root.get("name").and_then(Value::as_str),
                    &value,
                )?);
            }
        } else {
            for (id, value) in root.as_object().ok_or("MCP 片段必须是对象")? {
                entries.push(parsed_entry(id, None, value)?);
            }
        }
    } else {
        let document = text
            .parse::<DocumentMut>()
            .map_err(|err| format!("TOML 格式错误: {err}"))?;
        let servers = document
            .get("mcp_servers")
            .and_then(Item::as_table_like)
            .ok_or("未找到 [mcp_servers.<id>] 配置")?;
        for (id, value) in servers.iter() {
            entries.push(parsed_entry(id, None, &toml_item_to_json(value))?);
        }
    }
    if entries.is_empty() {
        return Err("片段中没有可导入的 MCP 服务".into());
    }
    Ok(entries)
}

pub fn export_mcp_backup_for_paths(paths: &AppPaths) -> Result<String, String> {
    let _guard = store_lock()?;
    let snapshot = snapshot_unlocked(paths)?;
    serde_json::to_string_pretty(&McpStore {
        entries: snapshot.entries,
    })
    .map_err(|err| format!("导出 MCP 失败: {err}"))
}

pub fn import_mcp_backup_for_paths(
    paths: &AppPaths,
    backup: &str,
) -> Result<McpHubSnapshot, String> {
    let _guard = store_lock()?;
    let value: Value =
        serde_json::from_str(backup).map_err(|err| format!("备份 JSON 格式错误: {err}"))?;
    if !value.get("entries").is_some_and(Value::is_array) {
        return Err("这不是 MCP 中心备份文件".into());
    }
    let mut imported: McpStore =
        serde_json::from_value(value).map_err(|err| format!("MCP 备份格式错误: {err}"))?;
    let mut ids = BTreeSet::new();
    for entry in &mut imported.entries {
        validate_id(&entry.id)?;
        if !ids.insert(entry.id.clone()) {
            return Err(format!("备份包含重复 MCP: {}", entry.id));
        }
        entry.spec = normalize_spec(entry.spec.clone())?;
        entry.platforms.sort();
        entry.platforms.dedup();
    }
    let mut snapshot = snapshot_unlocked(paths)?;
    snapshot.entries.retain(|entry| !ids.contains(&entry.id));
    snapshot.entries.extend(imported.entries.clone());
    let mut updates = Vec::new();
    for &platform in ExtPlatform::all() {
        let mut entries = imported.entries.clone();
        for entry in &mut entries {
            if let Some(variant) = entry.variants.get(&platform) {
                entry.spec = normalize_spec(variant.clone())?;
            }
        }
        updates.extend(stage_native_changes(paths, &entries, &[], Some(platform))?);
    }
    updates.push(FileUpdate::store(
        mcp_store_path(paths),
        &McpStore {
            entries: snapshot.entries,
        },
    )?);
    apply_file_updates(paths, updates)?;
    snapshot_unlocked(paths)
}
