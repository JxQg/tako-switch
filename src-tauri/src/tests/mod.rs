use crate::providers::{
    loader::parse_provider_catalog_file, types::DEFAULT_PROVIDER_CONFIG, PlatformWriter,
};
use chrono::Local;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

static INSTALL_DIR_TEST_LOCK: Mutex<()> = Mutex::new(());

pub fn install_dir_test_lock() -> MutexGuard<'static, ()> {
    INSTALL_DIR_TEST_LOCK.lock().unwrap()
}

pub fn provider_config_dir() -> &'static str {
    crate::config_paths::test_provider_config_dir()
}

pub fn provider_config_file() -> &'static str {
    crate::config_paths::test_provider_config_file()
}

pub fn default_platform_writer(platform_id: &str) -> PlatformWriter {
    parse_provider_catalog_file(DEFAULT_PROVIDER_CONFIG)
        .unwrap()
        .providers
        .into_iter()
        .find(|provider| provider.id == "tako")
        .unwrap()
        .platforms
        .get(platform_id)
        .unwrap()
        .writer
        .clone()
}

pub fn unique_temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "tako-switch-{name}-{}",
        Local::now().format("%Y%m%d%H%M%S%3f")
    ))
}
