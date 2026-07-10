use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

const PROVIDER_CONFIG_DIR: &str = "config";
const PROVIDER_CONFIG_FILE: &str = "providers.json";
const APP_DATA_DIR: &str = "Tako Switch";

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
    Ok(user_provider_config_base_dir()?
        .join(PROVIDER_CONFIG_DIR)
        .join(PROVIDER_CONFIG_FILE))
}

pub fn bundled_provider_config_relative_path() -> PathBuf {
    PathBuf::from(PROVIDER_CONFIG_DIR).join(PROVIDER_CONFIG_FILE)
}

fn user_provider_config_base_dir() -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        return app_data_dir();
    }

    install_dir()
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

pub fn app_data_dir() -> Result<PathBuf, String> {
    if cfg!(test) {
        if let Some(path) = env::var_os("TAKO_SWITCH_DATA_DIR") {
            return Ok(PathBuf::from(path));
        }
    }

    if cfg!(target_os = "macos") {
        return Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join(APP_DATA_DIR));
    }

    install_dir()
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
pub fn test_provider_config_dir() -> &'static str {
    PROVIDER_CONFIG_DIR
}

#[cfg(test)]
pub fn test_provider_config_file() -> &'static str {
    PROVIDER_CONFIG_FILE
}
