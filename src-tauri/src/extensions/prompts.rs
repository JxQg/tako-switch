use super::{
    apply_file_updates, extension_store_path, platform_key, prompt_path, read_json_store,
    read_optional_file, store_lock, validate_extension_id, ExtPlatform, FileUpdate,
};
use crate::adapters::{TAKO_CHINESE_BLOCK_END, TAKO_CHINESE_BLOCK_START};
use crate::config::{backup_file_once, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptPreset {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformPrompts {
    #[serde(default)]
    pub presets: Vec<PromptPreset>,
    #[serde(default)]
    pub active_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsStore {
    #[serde(default)]
    pub platforms: BTreeMap<ExtPlatform, PlatformPrompts>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptPlatformSnapshot {
    #[serde(default)]
    pub presets: Vec<PromptPreset>,
    #[serde(default)]
    pub active_id: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub live_bytes: usize,
    #[serde(default)]
    pub has_chinese_block: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsHubSnapshot {
    #[serde(default)]
    pub platforms: BTreeMap<ExtPlatform, PromptPlatformSnapshot>,
}

pub fn prompts_store_path(paths: &AppPaths) -> PathBuf {
    extension_store_path(paths, "prompts.json")
}

struct PromptDocument {
    body: String,
    managed: String,
    live_bytes: usize,
}

fn read_document(path: &Path) -> Result<PromptDocument, String> {
    let bytes = read_optional_file(path)?.unwrap_or_default();
    let live_bytes = bytes.len();
    let raw = String::from_utf8(bytes)
        .map_err(|err| format!("{} 不是 UTF-8 文本: {err}", path.display()))?;
    let Some(start) = raw.find(TAKO_CHINESE_BLOCK_START) else {
        if raw.contains(TAKO_CHINESE_BLOCK_END) {
            return Err("汉化托管块不完整，请先修复结束标记".into());
        }
        return Ok(PromptDocument {
            body: raw,
            managed: String::new(),
            live_bytes,
        });
    };
    let end = raw[start..]
        .find(TAKO_CHINESE_BLOCK_END)
        .map(|end| start + end + TAKO_CHINESE_BLOCK_END.len())
        .ok_or("汉化托管块不完整，已阻止修改提示词文件")?;
    if raw[end..].contains(TAKO_CHINESE_BLOCK_START) || raw[end..].contains(TAKO_CHINESE_BLOCK_END)
    {
        return Err("检测到多个汉化托管块，请先合并后再修改预设".into());
    }
    let (managed, after) = if raw[end..].trim().is_empty() {
        (raw[start..].to_string(), "")
    } else {
        (raw[start..end].to_string(), &raw[end..])
    };
    let before = raw[..start].trim_end_matches(['\r', '\n']);
    let body = if after.trim().is_empty() {
        before.to_string()
    } else if before.is_empty() {
        after.trim_start_matches(['\r', '\n']).to_string()
    } else {
        format!("{before}\n\n{}", after.trim_start_matches(['\r', '\n']))
    };
    Ok(PromptDocument {
        body,
        managed,
        live_bytes,
    })
}

fn same_body(left: &str, right: &str) -> bool {
    left.trim_end_matches(['\r', '\n']) == right.trim_end_matches(['\r', '\n'])
}

fn validate_preset(preset: &PromptPreset) -> Result<(), String> {
    validate_extension_id(&preset.id)?;
    if preset.name.trim().is_empty() {
        return Err("预设名称不能为空".into());
    }
    if preset.content.contains(TAKO_CHINESE_BLOCK_START)
        || preset.content.contains(TAKO_CHINESE_BLOCK_END)
    {
        return Err("用户提示词不能包含汉化托管标记；汉化规则由客户端独立管理".into());
    }
    Ok(())
}

fn archive_id(platform: ExtPlatform) -> String {
    format!("imported-{}", platform_key(platform))
}

fn read_store_and_snapshot(paths: &AppPaths) -> Result<(PromptsStore, PromptsHubSnapshot), String> {
    let mut store: PromptsStore = read_json_store(&prompts_store_path(paths));
    let mut snapshot = PromptsHubSnapshot::default();
    for &platform in ExtPlatform::all() {
        let path = prompt_path(paths, platform);
        let document = read_document(&path)?;
        let previous = store.platforms.get(&platform);
        if previous.is_some_and(|state| state.active_id.is_some())
            && !document.body.trim().is_empty()
        {
            let active = previous.and_then(|state| {
                state
                    .active_id
                    .as_ref()
                    .and_then(|id| state.presets.iter().find(|preset| &preset.id == id))
            });
            if active.is_some_and(|preset| !same_body(&preset.content, &document.body)) {
                backup_file_once(
                    paths,
                    platform_key(platform),
                    "extensions-original-prompt.md",
                    &path,
                )?;
            }
        }
        let state = store.platforms.entry(platform).or_default();
        if state.presets.is_empty() && !document.body.trim().is_empty() {
            backup_file_once(
                paths,
                platform_key(platform),
                "extensions-original-prompt.md",
                &path,
            )?;
            let id = archive_id(platform);
            state.presets.push(PromptPreset {
                id: id.clone(),
                name: "导入的原始提示词备份".into(),
                content: document.body.clone(),
            });
            state.active_id = Some(id);
        }
        if let Some(active) = state
            .active_id
            .as_ref()
            .and_then(|id| state.presets.iter_mut().find(|preset| &preset.id == id))
        {
            if !same_body(&active.content, &document.body) {
                active.content = document.body.clone();
            }
        } else {
            state.active_id = None;
        }
        snapshot.platforms.insert(
            platform,
            PromptPlatformSnapshot {
                presets: state.presets.clone(),
                active_id: state.active_id.clone(),
                path: path.to_string_lossy().into_owned(),
                live_bytes: document.live_bytes,
                has_chinese_block: !document.managed.is_empty(),
            },
        );
    }
    apply_file_updates(
        paths,
        vec![FileUpdate::store(prompts_store_path(paths), &store)?],
    )?;
    Ok((store, snapshot))
}

pub fn get_prompts_hub_snapshot_for_paths(paths: &AppPaths) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    read_store_and_snapshot(paths).map(|(_, snapshot)| snapshot)
}

fn stage_prompt(
    paths: &AppPaths,
    platform: ExtPlatform,
    content: &str,
) -> Result<FileUpdate, String> {
    let path = prompt_path(paths, platform);
    let document = read_document(&path)?;
    let next = if document.managed.is_empty() {
        content.to_string()
    } else if content.is_empty() {
        document.managed
    } else {
        let separator = if content.ends_with("\n\n") {
            ""
        } else if content.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        format!("{content}{separator}{}", document.managed)
    };
    Ok(FileUpdate::client(
        path,
        next.into_bytes(),
        platform,
        "extensions-original-prompt.md",
    ))
}

fn commit_store(
    paths: &AppPaths,
    store: &PromptsStore,
    mut updates: Vec<FileUpdate>,
) -> Result<PromptsHubSnapshot, String> {
    updates.push(FileUpdate::store(prompts_store_path(paths), store)?);
    apply_file_updates(paths, updates)?;
    read_store_and_snapshot(paths).map(|(_, snapshot)| snapshot)
}

pub fn upsert_prompt_preset_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
    mut preset: PromptPreset,
) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    validate_preset(&preset)?;
    preset.name = preset.name.trim().into();
    let (mut store, _) = read_store_and_snapshot(paths)?;
    let state = store.platforms.entry(platform).or_default();
    if state
        .presets
        .iter()
        .any(|existing| existing.name == preset.name && existing.id != preset.id)
    {
        return Err("该平台已有同名预设，请使用其他名称".into());
    }
    let mut updates = Vec::new();
    if state.active_id.as_deref() == Some(&preset.id) {
        updates.push(stage_prompt(paths, platform, &preset.content)?);
    }
    state.presets.retain(|existing| existing.id != preset.id);
    state.presets.push(preset);
    commit_store(paths, &store, updates)
}

pub fn activate_prompt_preset_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
    id: Option<&str>,
) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    let (mut store, _) = read_store_and_snapshot(paths)?;
    let state = store.platforms.entry(platform).or_default();
    let content = if let Some(id) = id {
        &state
            .presets
            .iter()
            .find(|preset| preset.id == id)
            .ok_or("未找到该提示词预设")?
            .content
    } else {
        ""
    };
    let update = stage_prompt(paths, platform, content)?;
    state.active_id = id.map(str::to_string);
    commit_store(paths, &store, vec![update])
}

pub fn delete_prompt_preset_for_paths(
    paths: &AppPaths,
    platform: ExtPlatform,
    id: &str,
) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    let (mut store, _) = read_store_and_snapshot(paths)?;
    let state = store.platforms.entry(platform).or_default();
    let mut updates = Vec::new();
    if state.active_id.as_deref() == Some(id) {
        updates.push(stage_prompt(paths, platform, "")?);
        state.active_id = None;
    }
    state.presets.retain(|preset| preset.id != id);
    commit_store(paths, &store, updates)
}

pub fn sync_prompt_to_platforms_for_paths(
    paths: &AppPaths,
    source: ExtPlatform,
    id: &str,
    targets: &[ExtPlatform],
    activate: bool,
) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    let (mut store, _) = read_store_and_snapshot(paths)?;
    let preset = store
        .platforms
        .get(&source)
        .and_then(|state| state.presets.iter().find(|preset| preset.id == id))
        .cloned()
        .ok_or("未找到要同步的提示词预设")?;
    let mut updates = Vec::new();
    for platform in targets
        .iter()
        .copied()
        .filter(|platform| *platform != source)
        .collect::<BTreeSet<_>>()
    {
        let state = store.platforms.entry(platform).or_default();
        let mut copy = preset.clone();
        if let Some(existing) = state
            .presets
            .iter()
            .find(|existing| existing.name == preset.name)
        {
            copy.id = existing.id.clone();
        } else if state.presets.iter().any(|existing| existing.id == copy.id) {
            copy.id = format!("prompt-{:016x}", rand::random::<u64>());
        }
        let should_write = activate || state.active_id.as_deref() == Some(&copy.id);
        if should_write {
            updates.push(stage_prompt(paths, platform, &copy.content)?);
        }
        if activate {
            state.active_id = Some(copy.id.clone());
        }
        state.presets.retain(|existing| existing.id != copy.id);
        state.presets.push(copy);
    }
    commit_store(paths, &store, updates)
}

pub fn export_prompts_backup_for_paths(paths: &AppPaths) -> Result<String, String> {
    let _guard = store_lock()?;
    let (store, _) = read_store_and_snapshot(paths)?;
    serde_json::to_string_pretty(&store).map_err(|err| format!("导出提示词失败: {err}"))
}

pub fn import_prompts_backup_for_paths(
    paths: &AppPaths,
    backup: &str,
) -> Result<PromptsHubSnapshot, String> {
    let _guard = store_lock()?;
    let value: Value =
        serde_json::from_str(backup).map_err(|err| format!("备份 JSON 格式错误: {err}"))?;
    if !value.get("platforms").is_some_and(Value::is_object) {
        return Err("这不是提示词预设备份文件".into());
    }
    let imported: PromptsStore =
        serde_json::from_value(value).map_err(|err| format!("提示词备份格式错误: {err}"))?;
    for state in imported.platforms.values() {
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for preset in &state.presets {
            validate_preset(preset)?;
            if !ids.insert(preset.id.clone()) || !names.insert(preset.name.clone()) {
                return Err("备份包含重复预设标识或名称".into());
            }
        }
        if state.active_id.as_ref().is_some_and(|id| !ids.contains(id)) {
            return Err("备份的启用预设不存在".into());
        }
    }
    let (mut store, _) = read_store_and_snapshot(paths)?;
    let mut updates = Vec::new();
    for (platform, state) in imported.platforms {
        let content = state
            .active_id
            .as_ref()
            .and_then(|id| state.presets.iter().find(|preset| &preset.id == id))
            .map(|preset| preset.content.as_str());
        if let Some(content) = content {
            updates.push(stage_prompt(paths, platform, content)?);
        } else if store
            .platforms
            .get(&platform)
            .is_some_and(|existing| existing.active_id.is_some())
        {
            updates.push(stage_prompt(paths, platform, "")?);
        }
        // Import merges saved presets so a restore cannot discard unrelated local work.
        let target = store.platforms.entry(platform).or_default();
        for preset in state.presets {
            target
                .presets
                .retain(|existing| existing.id != preset.id && existing.name != preset.name);
            target.presets.push(preset);
        }
        target.active_id = state.active_id;
    }
    commit_store(paths, &store, updates)
}
