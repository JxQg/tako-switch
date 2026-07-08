use super::*;
use crate::{
    models::ToolStatus,
    tests::{install_dir_test_lock, unique_temp_dir},
};
use std::env;

#[test]
fn backups_and_latest_apply_result_use_app_data_folder() {
    let _lock = install_dir_test_lock();
    let data_dir = unique_temp_dir("index-test");
    fs::create_dir_all(&data_dir).unwrap();
    env::set_var("TAKO_SWITCH_DATA_DIR", &data_dir);

    let codex_backup = make_backup_path("codex", Path::new("config.toml")).unwrap();
    let claude_backup = make_backup_path("claude", Path::new("settings.json")).unwrap();
    let index = backup_index_path().unwrap();

    assert!(codex_backup.starts_with(data_dir.join("backups").join("codex")));
    assert!(claude_backup.starts_with(data_dir.join("backups").join("claude-code")));
    assert_eq!(index, data_dir.join(BACKUP_INDEX_FILE));

    let result = ApplyResult {
        files: vec![AppliedFile {
            target: "codex".to_string(),
            path: "C:\\Users\\demo\\.codex\\config.toml".to_string(),
            backup_path: display_path(
                &data_dir
                    .join("backups")
                    .join("codex")
                    .join("config.toml.tako-backup-test"),
            ),
            created: false,
        }],
        env_updates: vec!["env updated".to_string()],
        tools: vec![ToolStatus {
            name: "Codex".to_string(),
            installed: true,
            version: Some("codex 1.0.0".to_string()),
            error: None,
        }],
        warnings: vec!["warning".to_string()],
    };

    save_latest_apply_result_to_disk(&result).unwrap();
    assert!(data_dir.join(BACKUP_INDEX_FILE).exists());
    let loaded = load_latest_apply_result_from_disk().unwrap().unwrap();
    assert_eq!(loaded.files[0].backup_path, result.files[0].backup_path);

    clear_latest_apply_result_from_disk().unwrap();
    assert!(load_latest_apply_result_from_disk().unwrap().is_none());

    env::remove_var("TAKO_SWITCH_DATA_DIR");
    let _ = fs::remove_dir_all(&data_dir);
}
