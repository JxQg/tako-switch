#[cfg(test)]
use std::sync::{Mutex, MutexGuard};
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

const PROVIDER_CONFIG_DIR: &str = "config";
const PROVIDER_CONFIG_FILE: &str = "providers.json";

pub fn codex_config_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(path).join("config.toml"));
    }
    Ok(home_dir()?.join(".codex").join("config.toml"))
}

pub fn claude_settings_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude").join("settings.json"))
}

pub fn provider_config_path() -> Result<PathBuf, String> {
    Ok(install_dir()?
        .join(PROVIDER_CONFIG_DIR)
        .join(PROVIDER_CONFIG_FILE))
}

pub fn home_dir() -> Result<PathBuf, String> {
    let candidates = [
        env::var_os("USERPROFILE"),
        env::var_os("HOME"),
        windows_home_from_parts(),
    ];

    candidates
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "无法确定用户主目录，请检查 USERPROFILE 或 HOME 环境变量。".to_string())
}

fn windows_home_from_parts() -> Option<OsString> {
    let drive = env::var_os("HOMEDRIVE")?;
    let path = env::var_os("HOMEPATH")?;
    let mut home = drive;
    home.push(path);
    Some(home)
}

pub fn install_dir() -> Result<PathBuf, String> {
    if cfg!(test) {
        if let Some(path) = env::var_os("TAKO_SWITCH_INSTALL_DIR") {
            return Ok(PathBuf::from(path));
        }
    }

    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            return Ok(parent.to_path_buf());
        }
    }

    env::current_dir().map_err(|err| format!("无法确定程序目录：{err}"))
}

pub fn resolve_restore_target(target: &str) -> Result<PathBuf, String> {
    match target {
        "codex" => codex_config_path(),
        "claude" => claude_settings_path(),
        value if !value.trim().is_empty() => Ok(Path::new(value).to_path_buf()),
        _ => Err("恢复目标不能为空。".to_string()),
    }
}

#[cfg(test)]
pub const TEST_PROVIDER_CONFIG_DIR: &str = PROVIDER_CONFIG_DIR;

#[cfg(test)]
pub const TEST_PROVIDER_CONFIG_FILE: &str = PROVIDER_CONFIG_FILE;

#[cfg(test)]
static INSTALL_DIR_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub fn install_dir_test_lock() -> MutexGuard<'static, ()> {
    INSTALL_DIR_TEST_LOCK.lock().unwrap()
}
