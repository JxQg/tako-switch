#[cfg(not(windows))]
use crate::{backups::write_file_atomic, config_paths::home_dir};
#[cfg(not(windows))]
use std::path::{Path, PathBuf};

pub const LEGACY_CODEX_API_KEY_ENV: &str = "TAKO_CODEX_API_KEY";

#[cfg(windows)]
pub fn read_legacy_codex_api_key() -> Result<Option<String>, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let environment = match hkcu.open_subkey("Environment") {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to open Windows user environment registry key: {err}"
            ))
        }
    };

    match environment.get_value::<String, _>(LEGACY_CODEX_API_KEY_ENV) {
        Ok(value) => Ok(non_empty(value)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!(
            "Failed to read Windows user environment variable {LEGACY_CODEX_API_KEY_ENV}: {err}"
        )),
    }
}

#[cfg(windows)]
pub fn cleanup_legacy_codex_api_key() -> Result<(), String> {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
        RegKey,
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let environment = match hkcu.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE) {
        Ok(key) => key,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(format!(
                "Failed to open Windows user environment registry key: {err}"
            ))
        }
    };

    match environment.delete_value(LEGACY_CODEX_API_KEY_ENV) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "Failed to remove Windows user environment variable {LEGACY_CODEX_API_KEY_ENV}: {err}"
        )),
    }
}

#[cfg(not(windows))]
pub fn read_legacy_codex_api_key() -> Result<Option<String>, String> {
    for path in profile_paths()? {
        if !path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("Failed to read shell profile: {err}"))?;
        if let Some(value) = read_marked_env_value(&content, LEGACY_CODEX_API_KEY_ENV) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

#[cfg(not(windows))]
pub fn cleanup_legacy_codex_api_key() -> Result<(), String> {
    for path in profile_paths()? {
        remove_env_block(&path, LEGACY_CODEX_API_KEY_ENV)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn profile_paths() -> Result<Vec<PathBuf>, String> {
    let home = home_dir()?;
    let mut paths = vec![home.join(".profile")];
    if cfg!(target_os = "macos") {
        paths.push(home.join(".zshrc"));
        paths.push(home.join(".zprofile"));
    }
    Ok(paths)
}

#[cfg(not(windows))]
fn remove_env_block(path: &Path, name: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let start = format!("# >>> Tako Switch: {name}");
    let end = format!("# <<< Tako Switch: {name}");
    let existing = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read shell profile: {err}"))?;
    let updated = remove_marked_block(&existing, &start, &end);
    if updated == existing {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create shell profile directory: {err}"))?;
    }
    write_file_atomic(path, &updated)
        .map_err(|err| format!("Failed to update shell profile: {err}"))
}

#[cfg(not(windows))]
fn remove_marked_block(existing: &str, start: &str, end: &str) -> String {
    if let Some(start_index) = existing.find(start) {
        if let Some(relative_end) = existing[start_index..].find(end) {
            let end_index = start_index + relative_end + end.len();
            let mut updated = String::new();
            updated.push_str(existing[..start_index].trim_end());
            let tail = existing[end_index..].trim_start_matches(&['\r', '\n'][..]);
            if !updated.is_empty() && !tail.is_empty() {
                updated.push_str("\n\n");
            }
            updated.push_str(tail);
            return crate::utils::ensure_trailing_newline(updated);
        }
    }

    existing.to_string()
}

#[cfg(not(windows))]
fn read_marked_env_value(existing: &str, name: &str) -> Option<String> {
    let start = format!("# >>> Tako Switch: {name}");
    let end = format!("# <<< Tako Switch: {name}");
    let start_index = existing.find(&start)?;
    let relative_end = existing[start_index..].find(&end)?;
    let block = &existing[start_index..start_index + relative_end];
    for line in block.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix(&format!("export {name}=")) else {
            continue;
        };
        return non_empty(unquote_shell_value(value));
    }
    None
}

#[cfg(not(windows))]
fn unquote_shell_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("'\\''", "'");
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].to_string();
    }
    value.to_string()
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn cleanup_missing_legacy_variable_is_ok() {
        cleanup_legacy_codex_api_key().unwrap();
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn marked_block_removal_only_removes_tako_block() {
        let start = "# >>> Tako Switch: TAKO_CODEX_API_KEY";
        let end = "# <<< Tako Switch: TAKO_CODEX_API_KEY";
        let existing = format!(
            "export TAKO_CODEX_API_KEY='manual'\n\n{start}\nexport TAKO_CODEX_API_KEY='one'\n{end}\n\nexport PATH=/bin\n"
        );
        let first = remove_marked_block(&existing, start, end);
        let second = remove_marked_block(&first, start, end);

        assert_eq!(first, second);
        assert!(first.contains("export TAKO_CODEX_API_KEY='manual'"));
        assert!(first.contains("export PATH=/bin"));
        assert!(!first.contains(start));
        assert!(!first.contains(end));
    }

    #[test]
    fn marked_block_reader_only_reads_tako_block() {
        let content = "\
export TAKO_CODEX_API_KEY='manual'

# >>> Tako Switch: TAKO_CODEX_API_KEY
export TAKO_CODEX_API_KEY='one'\\''two'
# <<< Tako Switch: TAKO_CODEX_API_KEY
";

        assert_eq!(
            read_marked_env_value(content, LEGACY_CODEX_API_KEY_ENV).as_deref(),
            Some("one'two")
        );
    }
}
