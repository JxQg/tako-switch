#[cfg(not(windows))]
use crate::{backups::write_file_atomic, config_paths::home_dir, utils::display_path};
#[cfg(not(windows))]
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub fn write_user_env_var(name: &str, value: &str) -> Result<String, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (environment, _) = hkcu
        .create_subkey("Environment")
        .map_err(|err| format!("打开 Windows 用户环境变量注册表失败：{err}"))?;
    environment
        .set_value(name, &value)
        .map_err(|err| format!("写入 Windows 用户环境变量失败：{err}"))?;
    Ok(format!(
        "{name} 已写入 Windows 用户环境变量；请新开终端后再运行 Codex。"
    ))
}

#[cfg(not(windows))]
pub fn write_user_env_var(name: &str, value: &str) -> Result<String, String> {
    let mut touched = Vec::new();
    for path in profile_paths()? {
        upsert_env_block(&path, name, value)?;
        touched.push(display_path(&path));
    }

    Ok(format!(
        "{name} 已写入 {}；请新开终端后再运行 Codex。",
        touched.join(", ")
    ))
}

#[cfg(not(windows))]
fn profile_paths() -> Result<Vec<PathBuf>, String> {
    let home = home_dir()?;
    let mut paths = vec![home.join(".profile")];
    if cfg!(target_os = "macos") {
        paths.push(home.join(".zshrc"));
    }
    Ok(paths)
}

#[cfg(not(windows))]
fn upsert_env_block(path: &Path, name: &str, value: &str) -> Result<(), String> {
    let start = format!("# >>> Tako Switch: {name}");
    let end = format!("# <<< Tako Switch: {name}");
    let block = format!("{start}\nexport {name}={}\n{end}\n", shell_quote(value));
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let updated = replace_marked_block(&existing, &start, &end, &block);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建 shell 配置目录失败：{err}"))?;
    }
    write_file_atomic(path, &updated).map_err(|err| format!("更新 shell 配置文件失败：{err}"))
}

#[cfg(not(windows))]
fn replace_marked_block(existing: &str, start: &str, end: &str, block: &str) -> String {
    if let Some(start_index) = existing.find(start) {
        if let Some(relative_end) = existing[start_index..].find(end) {
            let end_index = start_index + relative_end + end.len();
            let mut updated = String::new();
            updated.push_str(existing[..start_index].trim_end());
            updated.push_str("\n\n");
            updated.push_str(block.trim_end());
            updated.push_str("\n");
            updated.push_str(existing[end_index..].trim_start_matches(&['\r', '\n'][..]));
            return crate::utils::ensure_trailing_newline(updated);
        }
    }

    let mut updated = existing.trim_end().to_string();
    if !updated.is_empty() {
        updated.push_str("\n\n");
    }
    updated.push_str(block.trim_end());
    updated.push('\n');
    updated
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn codex_env_note(name: &str) -> String {
    if cfg!(windows) {
        "已写入 Windows 用户环境变量；请新开终端后再运行 Codex。".to_string()
    } else if cfg!(target_os = "macos") {
        "已写入 ~/.profile 和 ~/.zshrc；请新开终端后再运行 Codex。".to_string()
    } else {
        format!("已将 {name} 写入 ~/.profile；新的登录 shell 会读取它。")
    }
}

pub fn profile_warnings(name: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if !cfg!(windows) {
        warnings.push(format!(
            "Codex 会从 shell 环境读取 {name}；应用配置后请新开终端。"
        ));
    }
    warnings
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn marked_block_replacement_is_idempotent() {
        let start = "# >>> Tako Switch: TAKO_CODEX_API_KEY";
        let end = "# <<< Tako Switch: TAKO_CODEX_API_KEY";
        let block = format!("{start}\nexport TAKO_CODEX_API_KEY='one'\n{end}\n");
        let first = replace_marked_block("export PATH=/bin\n", start, end, &block);
        let second = replace_marked_block(&first, start, end, &block);

        assert_eq!(first, second);
        assert!(first.contains("export PATH=/bin"));
        assert!(first.contains("TAKO_CODEX_API_KEY"));
    }
}
