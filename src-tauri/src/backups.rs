use crate::{
    config_paths::{app_data_dir, resolve_restore_target},
    models::{AppliedFile, ApplyResult, RestoreResult},
    utils::{display_path, ensure_trailing_newline},
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const MISSING_SENTINEL: &str = "TAKO_BACKUP_ORIGINAL_FILE_MISSING\n";
const BACKUP_INDEX_FILE: &str = "tako-config-backups.json";
const BACKUP_INDEX_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupIndex {
    version: u32,
    saved_at: String,
    result: ApplyResult,
}

pub fn restore_backup_file(target: String, backup_path: String) -> Result<RestoreResult, String> {
    let target_path = resolve_restore_target(&target)?;
    let backup_path = PathBuf::from(backup_path);
    if !backup_path.exists() {
        return Err(format!("备份文件不存在：{}", display_path(&backup_path)));
    }

    let backup_content =
        fs::read_to_string(&backup_path).map_err(|err| format!("读取备份文件失败：{err}"))?;

    if backup_content == MISSING_SENTINEL {
        if target_path.exists() {
            fs::remove_file(&target_path).map_err(|err| format!("删除恢复目标文件失败：{err}"))?;
        }

        clear_latest_apply_result_from_disk()?;

        return Ok(RestoreResult {
            target,
            path: display_path(&target_path),
            restored_from: display_path(&backup_path),
            deleted_target: true,
        });
    }

    write_file_atomic(&target_path, &backup_content)
        .map_err(|err| format!("恢复备份失败：{err}"))?;
    clear_latest_apply_result_from_disk()?;

    Ok(RestoreResult {
        target,
        path: display_path(&target_path),
        restored_from: display_path(&backup_path),
        deleted_target: false,
    })
}

pub fn write_config_file(target: &str, path: &Path, content: &str) -> Result<AppliedFile, String> {
    let existed = path.exists();
    let backup_path = make_backup_path(target, path)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }

    if let Some(parent) = backup_path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建备份目录失败：{err}"))?;
    }

    if existed {
        fs::copy(path, &backup_path)
            .map_err(|err| format!("创建备份文件 {} 失败：{err}", display_path(&backup_path)))?;
    } else {
        fs::write(&backup_path, MISSING_SENTINEL)
            .map_err(|err| format!("创建缺失文件备份标记失败：{err}"))?;
    }

    write_file_atomic(path, content).map_err(|err| {
        let _ = restore_from_backup(path, &backup_path);
        format!("写入 {target} 配置失败，已尝试回滚：{err}")
    })?;

    Ok(AppliedFile {
        target: target.to_string(),
        path: display_path(path),
        backup_path: display_path(&backup_path),
        created: !existed,
    })
}

pub fn write_file_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temp_path = path.with_file_name(format!(
        ".{file_name}.tako-tmp-{}",
        Local::now().format("%Y%m%d%H%M%S%3f")
    ));

    fs::write(&temp_path, content)?;

    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            if path.exists() {
                fs::remove_file(path)?;
                fs::rename(&temp_path, path)
            } else {
                Err(first_error)
            }
        }
    }
}

fn restore_from_backup(path: &Path, backup_path: &Path) -> io::Result<()> {
    let content = fs::read_to_string(backup_path)?;
    if content == MISSING_SENTINEL {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    write_file_atomic(path, &content)
}

pub fn make_backup_path(target: &str, path: &Path) -> Result<PathBuf, String> {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    Ok(backup_root()?
        .join(backup_target_dir(target))
        .join(format!("{file_name}.tako-backup-{timestamp}")))
}

fn backup_target_dir(target: &str) -> &'static str {
    match target {
        "codex" => "codex",
        "claude" => "claude-code",
        _ => "other",
    }
}

fn backup_root() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("backups"))
}

fn backup_index_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(BACKUP_INDEX_FILE))
}

pub fn save_latest_apply_result_to_disk(result: &ApplyResult) -> Result<(), String> {
    let index_path = backup_index_path()?;
    let index = BackupIndex {
        version: BACKUP_INDEX_VERSION,
        saved_at: Local::now().to_rfc3339(),
        result: result.clone(),
    };
    let content = serde_json::to_string_pretty(&index)
        .map(ensure_trailing_newline)
        .map_err(|err| format!("生成备份索引失败：{err}"))?;
    write_file_atomic(&index_path, &content)
        .map_err(|err| format!("保存备份索引 {} 失败：{err}", display_path(&index_path)))
}

pub fn load_latest_apply_result_from_disk() -> Result<Option<ApplyResult>, String> {
    let index_path = backup_index_path()?;
    if !index_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&index_path)
        .map_err(|err| format!("读取备份索引 {} 失败：{err}", display_path(&index_path)))?;
    let index: BackupIndex = serde_json::from_str(&content).map_err(|err| {
        format!(
            "备份索引 {} 不是有效的 JSON：{err}",
            display_path(&index_path)
        )
    })?;

    if index.version != BACKUP_INDEX_VERSION {
        return Ok(None);
    }

    Ok(Some(index.result))
}

pub fn clear_latest_apply_result_from_disk() -> Result<(), String> {
    let index_path = backup_index_path()?;
    if index_path.exists() {
        fs::remove_file(&index_path)
            .map_err(|err| format!("清理备份索引 {} 失败：{err}", display_path(&index_path)))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/backups.rs"]
mod tests;
