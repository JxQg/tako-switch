use super::{
    build_http_client, extension_store_path, extensions_dir, normalize_platforms,
    platform_skills_dirs, read_json_store, read_json_store_checked, skills_ssot_dir,
    store_lock as lock_store, validate_extension_id as validate_identifier, with_extension_paths,
    write_json_store, ExtPlatform,
};
use crate::config::{backup_file_once, AppPaths};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Cursor, ErrorKind, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::fs::FileTypeExt;

pub const MAX_LOCAL_SKILL_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ARCHIVE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const MAX_SKILL_MD_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 16;
const STREAM_BUF_SIZE: usize = 64 * 1024;
const HASH_CACHE_MIN_BYTES: u64 = 256 * 1024;
const HASH_CACHE_MAX_ENTRIES: usize = 512;
const STALE_TEMP_TTL: Duration = Duration::from_secs(3600);

type HashCacheMap = BTreeMap<PathBuf, ([u8; 32], String)>;
static TREE_HASH_CACHE: Mutex<Option<HashCacheMap>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillSyncMethod {
    #[default]
    Auto,
    Symlink,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillLinkMode {
    Symlink,
    Copy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    #[default]
    Local,
    Adopted,
    Zip,
    Github,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default)]
    pub repo_owner: Option<String>,
    #[serde(default)]
    pub repo_name: Option<String>,
    #[serde(default)]
    pub repo_branch: Option<String>,
    #[serde(default)]
    pub repo_subdir: Option<String>,
    #[serde(default)]
    pub installed_hash: String,
    #[serde(default)]
    pub remote_hash: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    #[serde(default)]
    pub enabled_platforms: Vec<ExtPlatform>,
}

fn default_targets() -> Vec<ExtPlatform> {
    ExtPlatform::all().to_vec()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsStore {
    #[serde(default)]
    pub sync_method: SkillSyncMethod,
    #[serde(default = "default_targets")]
    pub default_targets: Vec<ExtPlatform>,
    #[serde(default)]
    pub skills: Vec<SkillRecord>,
}

impl Default for SkillsStore {
    fn default() -> Self {
        Self {
            sync_method: SkillSyncMethod::Auto,
            default_targets: default_targets(),
            skills: Vec::new(),
        }
    }
}

fn main_branch() -> String {
    "main".to_string()
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepo {
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "main_branch")]
    pub branch: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub archive_url_override: Option<String>,
}

impl Default for SkillRepo {
    fn default() -> Self {
        Self {
            owner: String::new(),
            name: String::new(),
            branch: main_branch(),
            enabled: true,
            archive_url_override: None,
        }
    }
}

fn default_repos() -> Vec<SkillRepo> {
    vec![SkillRepo {
        owner: "anthropics".to_string(),
        name: "skills".to_string(),
        branch: main_branch(),
        enabled: true,
        archive_url_override: None,
    }]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillReposStore {
    #[serde(default = "default_repos")]
    pub repos: Vec<SkillRepo>,
}

impl Default for SkillReposStore {
    fn default() -> Self {
        Self {
            repos: default_repos(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTargetStatus {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub actual_method: Option<SkillLinkMode>,
    #[serde(default)]
    pub has_diff: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPlatformStatus {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub actual_method: Option<SkillLinkMode>,
    #[serde(default)]
    pub has_diff: bool,
    #[serde(default)]
    pub targets: Vec<SkillTargetStatus>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedSkill {
    #[serde(default)]
    pub skill: SkillRecord,
    #[serde(default)]
    pub master_path: String,
    #[serde(default)]
    pub platforms: BTreeMap<ExtPlatform, SkillPlatformStatus>,
    #[serde(default)]
    pub locally_modified: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsHubSnapshot {
    #[serde(default)]
    pub sync_method: SkillSyncMethod,
    #[serde(default)]
    pub default_targets: Vec<ExtPlatform>,
    #[serde(default)]
    pub master_dir: String,
    #[serde(default)]
    pub skills: Vec<ManagedSkill>,
    #[serde(default)]
    pub repos: Vec<SkillRepo>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverableSkill {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repo_owner: String,
    #[serde(default)]
    pub repo_name: String,
    #[serde(default)]
    pub repo_branch: String,
    #[serde(default)]
    pub repo_subdir: String,
    #[serde(default)]
    pub remote_hash: String,
    #[serde(default)]
    pub installed_id: Option<String>,
    #[serde(default)]
    pub update_available: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLocalSkillRequest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub zip_base64: Option<String>,
    #[serde(default)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub targets: Option<Vec<ExtPlatform>>,
}

#[derive(Debug, Clone)]
struct LocalFileEntry {
    source: PathBuf,
    size: u64,
    executable: bool,
    meta_stamp: [u8; 32],
}

#[derive(Debug, Clone)]
struct LocalSkillTree {
    canonical_root: PathBuf,
    files: BTreeMap<String, LocalFileEntry>,
    total_bytes: u64,
    meta_digest: [u8; 32],
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Frontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

pub fn master_skills_dir(paths: &AppPaths) -> PathBuf {
    skills_ssot_dir(paths)
}

fn skills_store_path(paths: &AppPaths) -> PathBuf {
    extension_store_path(paths, "skills.json")
}

fn repos_store_path(paths: &AppPaths) -> PathBuf {
    extension_store_path(paths, "skill_repos.json")
}

fn temp_uploads_dir(paths: &AppPaths) -> PathBuf {
    extensions_dir(paths).join(".tmp-uploads")
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => Ok(false),
        Err(e) => Err(format!("检查技能路径 {} 失败: {e}", path.display())),
    }
}

fn anchor_for_path<'a>(paths: &'a AppPaths, path: &Path) -> Result<&'a Path, String> {
    if path.starts_with(&paths.home) {
        Ok(&paths.home)
    } else if path.starts_with(&paths.codex_dir) {
        Ok(&paths.codex_dir)
    } else if path.starts_with(&paths.tako_dir) {
        Ok(&paths.tako_dir)
    } else {
        Err(format!("技能写入路径不在应用目录内: {}", path.display()))
    }
}

fn ensure_plain_ancestors(paths: &AppPaths, path: &Path) -> Result<(), String> {
    let anchor = anchor_for_path(paths, path)?;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if is_link(&meta) => {
                return Err(format!("拒绝通过链接目录写入技能: {}", ancestor.display()));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(format!("技能父路径不是目录: {}", ancestor.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) if e.kind() == ErrorKind::NotADirectory => {
                if ancestor == anchor {
                    return Err(format!("技能父路径不是目录: {}", ancestor.display()));
                }
            }
            Err(e) => return Err(format!("检查目录 {} 失败: {e}", ancestor.display())),
        }
        // Trust system aliases above the supplied home, including macOS temporary directories.
        if ancestor == anchor {
            break;
        }
    }
    Ok(())
}

fn validate_single_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty() || matches!(name, "." | "..") || name.contains(['/', '\\', '\0']) {
        return Err(format!("不安全的临时目录项名称: {name}"));
    }
    Ok(())
}

fn is_stale_meta(meta: &fs::Metadata, now: SystemTime) -> bool {
    meta.modified()
        .ok()
        .and_then(|mtime| now.duration_since(mtime).ok())
        .is_some_and(|age| age > STALE_TEMP_TTL)
}

#[cfg(unix)]
mod verified_fs {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    const O_RDONLY: std::ffi::c_int = 0;
    const O_WRONLY: std::ffi::c_int = 1;
    const F_GETFL: std::ffi::c_int = 3;
    const F_SETFL: std::ffi::c_int = 4;

    #[cfg(target_os = "macos")]
    type ModeT = u16;
    #[cfg(not(target_os = "macos"))]
    type ModeT = u32;

    #[cfg(target_os = "macos")]
    const O_NONBLOCK: std::ffi::c_int = 0x0004;
    #[cfg(target_os = "macos")]
    const O_APPEND: std::ffi::c_int = 0x0008;
    #[cfg(target_os = "macos")]
    const O_NOFOLLOW: std::ffi::c_int = 0x0100;
    #[cfg(target_os = "macos")]
    const O_CREAT: std::ffi::c_int = 0x0200;
    #[cfg(target_os = "macos")]
    const O_DIRECTORY: std::ffi::c_int = 0x0010_0000;
    #[cfg(target_os = "macos")]
    const O_CLOEXEC: std::ffi::c_int = 0x0100_0000;
    #[cfg(target_os = "macos")]
    const AT_REMOVEDIR: std::ffi::c_int = 0x0080;
    #[cfg(target_os = "macos")]
    const ELOOP_ERRNO: i32 = 62;

    #[cfg(not(target_os = "macos"))]
    const O_CREAT: std::ffi::c_int = 0o100;
    #[cfg(not(target_os = "macos"))]
    const O_APPEND: std::ffi::c_int = 0o2000;
    #[cfg(not(target_os = "macos"))]
    const O_NONBLOCK: std::ffi::c_int = 0o4000;
    #[cfg(not(target_os = "macos"))]
    const O_CLOEXEC: std::ffi::c_int = 0o2000000;
    #[cfg(not(target_os = "macos"))]
    const AT_REMOVEDIR: std::ffi::c_int = 0x200;
    #[cfg(not(target_os = "macos"))]
    const ELOOP_ERRNO: i32 = 40;
    #[cfg(all(
        not(target_os = "macos"),
        any(target_arch = "aarch64", target_arch = "arm")
    ))]
    const O_DIRECTORY: std::ffi::c_int = 0o40000;
    #[cfg(all(
        not(target_os = "macos"),
        any(target_arch = "aarch64", target_arch = "arm")
    ))]
    const O_NOFOLLOW: std::ffi::c_int = 0o100000;
    #[cfg(all(
        not(target_os = "macos"),
        not(any(target_arch = "aarch64", target_arch = "arm"))
    ))]
    const O_DIRECTORY: std::ffi::c_int = 0o200000;
    #[cfg(all(
        not(target_os = "macos"),
        not(any(target_arch = "aarch64", target_arch = "arm"))
    ))]
    const O_NOFOLLOW: std::ffi::c_int = 0o400000;

    extern "C" {
        fn openat(
            dirfd: std::ffi::c_int,
            pathname: *const std::ffi::c_char,
            flags: std::ffi::c_int,
            ...
        ) -> std::ffi::c_int;
        fn mkdirat(
            dirfd: std::ffi::c_int,
            pathname: *const std::ffi::c_char,
            mode: ModeT,
        ) -> std::ffi::c_int;
        fn unlinkat(
            dirfd: std::ffi::c_int,
            pathname: *const std::ffi::c_char,
            flags: std::ffi::c_int,
        ) -> std::ffi::c_int;
        fn fcntl(fd: std::ffi::c_int, cmd: std::ffi::c_int, ...) -> std::ffi::c_int;
    }

    fn c_segment(name: &str) -> Result<CString, String> {
        validate_single_entry_name(name)?;
        CString::new(name.as_bytes()).map_err(|_| format!("不安全的临时目录项名称: {name}"))
    }

    fn clear_nonblock(file: &fs::File) {
        let fd = file.as_raw_fd();
        let flags = unsafe { fcntl(fd, F_GETFL) };
        if flags >= 0 && (flags & O_NONBLOCK) != 0 {
            unsafe {
                let _ = fcntl(fd, F_SETFL, flags & !O_NONBLOCK);
            }
        }
    }

    pub(super) struct RawDir {
        pub(super) file: fs::File,
        dev: u64,
        ino: u64,
    }

    impl RawDir {
        fn from_file(file: fs::File, display_path: &Path) -> Result<Self, String> {
            let meta = file
                .metadata()
                .map_err(|e| format!("检查目录 {} 失败: {e}", display_path.display()))?;
            if is_link(&meta) {
                return Err(format!("拒绝通过链接目录写入技能: {}", display_path.display()));
            }
            if !meta.is_dir() {
                return Err(format!("技能父路径不是目录: {}", display_path.display()));
            }
            Ok(Self {
                dev: meta.dev(),
                ino: meta.ino(),
                file,
            })
        }

        pub(super) fn open_anchor(anchor: &Path, create: bool) -> Result<Option<Self>, String> {
            if create && !path_exists(anchor)? {
                fs::create_dir_all(anchor)
                    .map_err(|e| format!("创建目录 {} 失败: {e}", anchor.display()))?;
            }
            match OpenOptions::new()
                .read(true)
                .custom_flags(O_DIRECTORY | O_NOFOLLOW)
                .open(anchor)
            {
                Ok(file) => Self::from_file(file, anchor).map(Some),
                Err(e) if !create && matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
                    Ok(None)
                }
                Err(e) => {
                    if e.raw_os_error() == Some(ELOOP_ERRNO)
                        || fs::symlink_metadata(anchor).is_ok_and(|m| is_link(&m))
                    {
                        Err(format!("拒绝通过链接目录写入技能: {}", anchor.display()))
                    } else if e.kind() == ErrorKind::NotADirectory {
                        Err(format!("技能父路径不是目录: {}", anchor.display()))
                    } else {
                        Err(format!("打开目录 {} 失败: {e}", anchor.display()))
                    }
                }
            }
        }

        pub(super) fn open_or_create_subdir(
            &self,
            name: &str,
            step_path: &Path,
            create: bool,
        ) -> Result<Option<Self>, String> {
            let cname = c_segment(name)?;
            if create {
                let rc = unsafe { mkdirat(self.file.as_raw_fd(), cname.as_ptr(), 0o755) };
                if rc < 0 {
                    let err = std::io::Error::last_os_error();
                    if err.kind() != ErrorKind::AlreadyExists {
                        return Err(format!("创建目录 {} 失败: {err}", step_path.display()));
                    }
                }
            }
            let raw = unsafe {
                openat(
                    self.file.as_raw_fd(),
                    cname.as_ptr(),
                    O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
                )
            };
            if raw < 0 {
                let err = std::io::Error::last_os_error();
                if !create && err.kind() == ErrorKind::NotFound {
                    return Ok(None);
                }
                if err.raw_os_error() == Some(ELOOP_ERRNO)
                    || fs::symlink_metadata(step_path).is_ok_and(|m| is_link(&m))
                {
                    return Err(format!("拒绝通过链接目录写入技能: {}", step_path.display()));
                }
                if err.kind() == ErrorKind::NotADirectory {
                    return Err(format!("技能父路径不是目录: {}", step_path.display()));
                }
                return Err(format!("打开目录 {} 失败: {err}", step_path.display()));
            }
            let file = unsafe { fs::File::from_raw_fd(raw) };
            Self::from_file(file, step_path).map(Some)
        }

        pub(super) fn open_regular_read(&self, name: &str) -> Result<Option<fs::File>, String> {
            let cname = c_segment(name)?;
            let raw = unsafe {
                openat(
                    self.file.as_raw_fd(),
                    cname.as_ptr(),
                    O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK,
                )
            };
            if raw < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == ErrorKind::NotFound {
                    return Ok(None);
                }
                if err.raw_os_error() == Some(ELOOP_ERRNO) || err.kind() == ErrorKind::IsADirectory {
                    return Err("分片上传临时文件不是普通文件".to_string());
                }
                return Err(format!("检查分片临时文件失败: {err}"));
            }
            let file = unsafe { fs::File::from_raw_fd(raw) };
            let meta = file
                .metadata()
                .map_err(|e| format!("检查分片临时文件失败: {e}"))?;
            if is_link(&meta) || !meta.is_file() {
                return Err("分片上传临时文件不是普通文件".to_string());
            }
            clear_nonblock(&file);
            Ok(Some(file))
        }

        pub(super) fn open_regular_append(
            &self,
            name: &str,
            create: bool,
        ) -> Result<fs::File, String> {
            let cname = c_segment(name)?;
            let base_flags = O_WRONLY | O_APPEND | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK;
            let create_mode: std::ffi::c_uint = 0o600;
            let raw = if create {
                unsafe {
                    openat(
                        self.file.as_raw_fd(),
                        cname.as_ptr(),
                        base_flags | O_CREAT,
                        create_mode,
                    )
                }
            } else {
                unsafe { openat(self.file.as_raw_fd(), cname.as_ptr(), base_flags) }
            };
            if raw < 0 {
                let err = std::io::Error::last_os_error();
                if !create && err.kind() == ErrorKind::NotFound {
                    return Err("分片上传会话不存在或已过期".to_string());
                }
                if err.raw_os_error() == Some(ELOOP_ERRNO) || err.kind() == ErrorKind::IsADirectory {
                    return Err("分片上传临时文件不是普通文件".to_string());
                }
                return Err(format!("写入分片临时文件失败: {err}"));
            }
            let file = unsafe { fs::File::from_raw_fd(raw) };
            let meta = file
                .metadata()
                .map_err(|e| format!("检查分片临时文件失败: {e}"))?;
            if is_link(&meta) || !meta.is_file() {
                return Err("分片上传临时文件不是普通文件".to_string());
            }
            clear_nonblock(&file);
            Ok(file)
        }

        pub(super) fn unlink_regular_file(&self, name: &str) -> Result<bool, String> {
            let Some(verified) = self.open_regular_read(name)? else {
                return Ok(false);
            };
            drop(verified);
            let cname = c_segment(name)?;
            let rc = unsafe { unlinkat(self.file.as_raw_fd(), cname.as_ptr(), 0) };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == ErrorKind::NotFound {
                    return Ok(false);
                }
                return Err(format!("清理分片临时文件失败: {err}"));
            }
            Ok(true)
        }

        pub(super) fn unlink_stale_regular_file(&self, name: &str, now: SystemTime) -> bool {
            let Ok(Some(file)) = self.open_regular_read(name) else {
                return false;
            };
            let is_stale = file
                .metadata()
                .is_ok_and(|meta| !is_link(&meta) && meta.is_file() && is_stale_meta(&meta, now));
            drop(file);
            if !is_stale {
                return false;
            }
            let Ok(cname) = c_segment(name) else {
                return false;
            };
            unsafe { unlinkat(self.file.as_raw_fd(), cname.as_ptr(), 0) == 0 }
        }

        pub(super) fn unlink_non_dir_entry(&self, name: &str) -> Result<(), String> {
            let cname = c_segment(name)?;
            let rc = unsafe { unlinkat(self.file.as_raw_fd(), cname.as_ptr(), 0) };
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == ErrorKind::NotFound {
                    return Ok(());
                }
                return Err(format!("清理临时文件失败: {err}"));
            }
            Ok(())
        }

        pub(super) fn list_entry_names(&self, fallback_path: &Path) -> Vec<String> {
            let live_dir: Option<PathBuf> = {
                #[cfg(target_os = "macos")]
                {
                    use std::ffi::OsStr;
                    use std::os::unix::ffi::OsStrExt;
                    const F_GETPATH: std::ffi::c_int = 50;
                    let mut buf = vec![0u8; 1024];
                    let rc = unsafe { fcntl(self.file.as_raw_fd(), F_GETPATH, buf.as_mut_ptr()) };
                    if rc == 0 {
                        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                        Some(PathBuf::from(OsStr::from_bytes(&buf[..len])))
                    } else {
                        None
                    }
                }
                #[cfg(not(target_os = "macos"))]
                {
                    Some(PathBuf::from(format!(
                        "/proc/self/fd/{}",
                        self.file.as_raw_fd()
                    )))
                }
            };
            let read_target = live_dir.as_deref().unwrap_or(fallback_path);
            fs::read_dir(read_target)
                .or_else(|_| fs::read_dir(fallback_path))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default()
        }

        pub(super) fn try_remove_empty_subdir(&self, name: &str, child: &mut Option<RawDir>) {
            let Some(ref child_ref) = child else {
                return;
            };
            let Ok(cname) = c_segment(name) else {
                return;
            };
            let reopened = unsafe {
                openat(
                    self.file.as_raw_fd(),
                    cname.as_ptr(),
                    O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC,
                )
            };
            if reopened < 0 {
                return;
            }
            let reopened_file = unsafe { fs::File::from_raw_fd(reopened) };
            let same = reopened_file.metadata().is_ok_and(|meta| {
                !is_link(&meta)
                    && meta.is_dir()
                    && meta.dev() == child_ref.dev
                    && meta.ino() == child_ref.ino
            });
            drop(reopened_file);
            if same {
                let _ = child.take();
                unsafe {
                    let _ = unlinkat(self.file.as_raw_fd(), cname.as_ptr(), AT_REMOVEDIR);
                }
            }
        }
    }
}

#[cfg(windows)]
mod verified_fs {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};

    type NtStatus = i32;

    const DELETE_ACCESS: u32 = 0x0001_0000;
    const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
    const FILE_LIST_DIRECTORY_ACCESS: u32 = 0x0000_0001;
    const FILE_APPEND_DATA_ACCESS: u32 = 0x0000_0004;
    const FILE_READ_ATTRIBUTES_ACCESS: u32 = 0x0000_0080;
    const FILE_GENERIC_READ_ACCESS: u32 = 0x0012_0089;

    const FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
    const FILE_ATTRIBUTE_NORMAL_VAL: u32 = 0x0000_0080;
    const FILE_FLAG_OPEN_REPARSE_POINT_VAL: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS_VAL: u32 = 0x0200_0000;

    const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
    const FILE_OPEN: u32 = 1;
    const FILE_OPEN_IF: u32 = 3;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x0000_4000;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    const STATUS_OBJECT_NAME_INVALID: NtStatus = 0xC000_0033_u32 as i32;
    const STATUS_OBJECT_NAME_NOT_FOUND: NtStatus = 0xC000_0034_u32 as i32;
    const STATUS_OBJECT_PATH_NOT_FOUND: NtStatus = 0xC000_003A_u32 as i32;
    const STATUS_FILE_IS_A_DIRECTORY: NtStatus = 0xC000_00BA_u32 as i32;
    const STATUS_NOT_A_DIRECTORY: NtStatus = 0xC000_0103_u32 as i32;

    const FILE_DISPOSITION_INFO_CLASS: i32 = 4;
    const FILE_DISPOSITION_INFO_EX_CLASS: i32 = 21;
    const FILE_DISPOSITION_DELETE_POSIX: u32 = 0x0000_0001 | 0x0000_0002;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        file_attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }

    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: RawHandle,
        object_name: *const UnicodeString,
        attributes: u32,
        security_descriptor: *const std::ffi::c_void,
        security_quality_of_service: *const std::ffi::c_void,
    }

    #[repr(C)]
    struct IoStatusBlock {
        status_or_pointer: *mut std::ffi::c_void,
        information: usize,
    }

    #[repr(C)]
    struct FileDispositionInfo {
        delete_file: u8,
    }

    #[repr(C)]
    struct FileDispositionInfoEx {
        flags: u32,
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(
            file_handle: *mut RawHandle,
            desired_access: u32,
            object_attributes: *const ObjectAttributes,
            io_status_block: *mut IoStatusBlock,
            allocation_size: *const i64,
            file_attributes: u32,
            share_access: u32,
            create_disposition: u32,
            create_options: u32,
            ea_buffer: *const std::ffi::c_void,
            ea_length: u32,
        ) -> NtStatus;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            file: RawHandle,
            file_information: *mut ByHandleFileInformation,
        ) -> i32;
        fn GetFinalPathNameByHandleW(
            file: RawHandle,
            file_path: *mut u16,
            cch_file_path: u32,
            flags: u32,
        ) -> u32;
        fn SetFileInformationByHandle(
            file: RawHandle,
            file_information_class: i32,
            file_information: *const std::ffi::c_void,
            buffer_size: u32,
        ) -> i32;
    }

    fn live_path_by_handle(file: &fs::File) -> Option<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        let needed =
            unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
        if needed == 0 {
            return None;
        }
        let mut buf = vec![0u16; needed as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), buf.as_mut_ptr(), buf.len() as u32, 0)
        };
        if written == 0 || written as usize >= buf.len() {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from_wide(
            &buf[..written as usize],
        )))
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct WinFileId {
        vol: u32,
        idx_high: u32,
        idx_low: u32,
    }

    fn file_id_by_handle(file: &fs::File) -> Result<WinFileId, String> {
        let mut info: ByHandleFileInformation = unsafe { std::mem::zeroed() };
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
        if ok == 0 {
            return Err(format!(
                "读取目录句柄标识失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(WinFileId {
            vol: info.volume_serial_number,
            idx_high: info.file_index_high,
            idx_low: info.file_index_low,
        })
    }

    fn mark_handle_deleted(file: &fs::File) -> bool {
        let info_ex = FileDispositionInfoEx {
            flags: FILE_DISPOSITION_DELETE_POSIX,
        };
        let ok_ex = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FILE_DISPOSITION_INFO_EX_CLASS,
                (&raw const info_ex).cast(),
                std::mem::size_of::<FileDispositionInfoEx>() as u32,
            )
        };
        if ok_ex != 0 {
            return true;
        }
        let info = FileDispositionInfo { delete_file: 1 };
        let ok = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FILE_DISPOSITION_INFO_CLASS,
                (&raw const info).cast(),
                std::mem::size_of::<FileDispositionInfo>() as u32,
            )
        };
        ok != 0
    }

    fn nt_open_relative(
        root: RawHandle,
        name: &str,
        desired_access: u32,
        disposition: u32,
        create_options: u32,
    ) -> Result<fs::File, NtStatus> {
        let mut wide: Vec<u16> = OsStr::new(name).encode_wide().collect();
        let byte_len = wide
            .len()
            .checked_mul(2)
            .and_then(|n| u16::try_from(n).ok())
            .ok_or(STATUS_OBJECT_NAME_INVALID)?;
        let ustr = UnicodeString {
            length: byte_len,
            maximum_length: byte_len,
            buffer: wide.as_mut_ptr(),
        };
        let attrs = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root_directory: root,
            object_name: &ustr,
            attributes: OBJ_CASE_INSENSITIVE,
            security_descriptor: std::ptr::null(),
            security_quality_of_service: std::ptr::null(),
        };
        let mut iosb = IoStatusBlock {
            status_or_pointer: std::ptr::null_mut(),
            information: 0,
        };
        let mut out_handle: RawHandle = std::ptr::null_mut();
        let status = unsafe {
            NtCreateFile(
                &mut out_handle,
                desired_access,
                &attrs,
                &mut iosb,
                std::ptr::null(),
                FILE_ATTRIBUTE_NORMAL_VAL,
                FILE_SHARE_ALL,
                disposition,
                create_options,
                std::ptr::null(),
                0,
            )
        };
        if status < 0 {
            return Err(status);
        }
        Ok(unsafe { fs::File::from_raw_handle(out_handle) })
    }

    pub(super) struct RawDir {
        pub(super) file: fs::File,
        id: WinFileId,
    }

    impl RawDir {
        fn from_file(file: fs::File, display_path: &Path) -> Result<Self, String> {
            let meta = file
                .metadata()
                .map_err(|e| format!("检查目录 {} 失败: {e}", display_path.display()))?;
            if is_link(&meta) {
                return Err(format!("拒绝通过链接目录写入技能: {}", display_path.display()));
            }
            if !meta.is_dir() {
                return Err(format!("技能父路径不是目录: {}", display_path.display()));
            }
            let id = file_id_by_handle(&file)?;
            Ok(Self { file, id })
        }

        pub(super) fn open_anchor(anchor: &Path, create: bool) -> Result<Option<Self>, String> {
            if create && !path_exists(anchor)? {
                fs::create_dir_all(anchor)
                    .map_err(|e| format!("创建目录 {} 失败: {e}", anchor.display()))?;
            }
            match OpenOptions::new()
                .read(true)
                .access_mode(
                    FILE_LIST_DIRECTORY_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                )
                .share_mode(FILE_SHARE_ALL)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS_VAL | FILE_FLAG_OPEN_REPARSE_POINT_VAL)
                .open(anchor)
            {
                Ok(file) => Self::from_file(file, anchor).map(Some),
                Err(e) if !create && matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
                    Ok(None)
                }
                Err(e) => {
                    if fs::symlink_metadata(anchor).is_ok_and(|m| is_link(&m)) {
                        Err(format!("拒绝通过链接目录写入技能: {}", anchor.display()))
                    } else if e.kind() == ErrorKind::NotADirectory {
                        Err(format!("技能父路径不是目录: {}", anchor.display()))
                    } else {
                        Err(format!("打开目录 {} 失败: {e}", anchor.display()))
                    }
                }
            }
        }

        pub(super) fn open_or_create_subdir(
            &self,
            name: &str,
            step_path: &Path,
            create: bool,
        ) -> Result<Option<Self>, String> {
            validate_single_entry_name(name)?;
            let disposition = if create { FILE_OPEN_IF } else { FILE_OPEN };
            match nt_open_relative(
                self.file.as_raw_handle(),
                name,
                FILE_LIST_DIRECTORY_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                disposition,
                FILE_DIRECTORY_FILE
                    | FILE_OPEN_REPARSE_POINT
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | FILE_OPEN_FOR_BACKUP_INTENT,
            ) {
                Ok(file) => Self::from_file(file, step_path).map(Some),
                Err(s)
                    if !create
                        && matches!(
                            s,
                            STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND
                        ) =>
                {
                    Ok(None)
                }
                Err(s) => {
                    if fs::symlink_metadata(step_path).is_ok_and(|m| is_link(&m)) {
                        Err(format!("拒绝通过链接目录写入技能: {}", step_path.display()))
                    } else if s == STATUS_NOT_A_DIRECTORY {
                        Err(format!("技能父路径不是目录: {}", step_path.display()))
                    } else {
                        Err(format!(
                            "打开目录 {} 失败 (NTSTATUS 0x{:08X})",
                            step_path.display(),
                            s as u32
                        ))
                    }
                }
            }
        }

        pub(super) fn open_regular_read(&self, name: &str) -> Result<Option<fs::File>, String> {
            validate_single_entry_name(name)?;
            match nt_open_relative(
                self.file.as_raw_handle(),
                name,
                FILE_GENERIC_READ_ACCESS,
                FILE_OPEN,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ) {
                Ok(file) => {
                    let meta = file
                        .metadata()
                        .map_err(|e| format!("检查分片临时文件失败: {e}"))?;
                    if is_link(&meta) || !meta.is_file() {
                        return Err("分片上传临时文件不是普通文件".to_string());
                    }
                    Ok(Some(file))
                }
                Err(STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND) => {
                    Ok(None)
                }
                Err(STATUS_FILE_IS_A_DIRECTORY) => {
                    Err("分片上传临时文件不是普通文件".to_string())
                }
                Err(s) => Err(format!("检查分片临时文件失败 (NTSTATUS 0x{:08X})", s as u32)),
            }
        }

        pub(super) fn open_regular_append(
            &self,
            name: &str,
            create: bool,
        ) -> Result<fs::File, String> {
            validate_single_entry_name(name)?;
            let disposition = if create { FILE_OPEN_IF } else { FILE_OPEN };
            match nt_open_relative(
                self.file.as_raw_handle(),
                name,
                FILE_APPEND_DATA_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                disposition,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ) {
                Ok(file) => {
                    let meta = file
                        .metadata()
                        .map_err(|e| format!("检查分片临时文件失败: {e}"))?;
                    if is_link(&meta) || !meta.is_file() {
                        return Err("分片上传临时文件不是普通文件".to_string());
                    }
                    Ok(file)
                }
                Err(s)
                    if !create
                        && matches!(
                            s,
                            STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND
                        ) =>
                {
                    Err("分片上传会话不存在或已过期".to_string())
                }
                Err(STATUS_FILE_IS_A_DIRECTORY) => {
                    Err("分片上传临时文件不是普通文件".to_string())
                }
                Err(s) => Err(format!("写入分片临时文件失败 (NTSTATUS 0x{:08X})", s as u32)),
            }
        }

        pub(super) fn unlink_regular_file(&self, name: &str) -> Result<bool, String> {
            validate_single_entry_name(name)?;
            let file = match nt_open_relative(
                self.file.as_raw_handle(),
                name,
                DELETE_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                FILE_OPEN,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ) {
                Ok(file) => file,
                Err(STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND) => {
                    return Ok(false);
                }
                Err(STATUS_FILE_IS_A_DIRECTORY) => {
                    return Err("分片上传临时文件不是普通文件".to_string());
                }
                Err(s) => {
                    return Err(format!("清理分片临时文件失败 (NTSTATUS 0x{:08X})", s as u32));
                }
            };
            let meta = file
                .metadata()
                .map_err(|e| format!("检查分片临时文件失败: {e}"))?;
            if is_link(&meta) || !meta.is_file() {
                return Err("分片上传临时文件不是普通文件".to_string());
            }
            if !mark_handle_deleted(&file) {
                return Err("标记删除分片临时文件失败".to_string());
            }
            Ok(true)
        }

        pub(super) fn unlink_stale_regular_file(&self, name: &str, now: SystemTime) -> bool {
            if validate_single_entry_name(name).is_err() {
                return false;
            }
            let Ok(file) = nt_open_relative(
                self.file.as_raw_handle(),
                name,
                DELETE_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                FILE_OPEN,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ) else {
                return false;
            };
            let is_stale = file
                .metadata()
                .is_ok_and(|meta| !is_link(&meta) && meta.is_file() && is_stale_meta(&meta, now));
            if !is_stale {
                return false;
            }
            mark_handle_deleted(&file)
        }

        pub(super) fn unlink_non_dir_entry(&self, name: &str) -> Result<(), String> {
            validate_single_entry_name(name)?;
            match nt_open_relative(
                self.file.as_raw_handle(),
                name,
                DELETE_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                FILE_OPEN,
                FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ) {
                Ok(file) => {
                    let _ = mark_handle_deleted(&file);
                    Ok(())
                }
                Err(STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND) => {
                    Ok(())
                }
                Err(STATUS_FILE_IS_A_DIRECTORY) => {
                    if let Ok(link_dir) = nt_open_relative(
                        self.file.as_raw_handle(),
                        name,
                        DELETE_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                        FILE_OPEN,
                        FILE_DIRECTORY_FILE
                            | FILE_OPEN_REPARSE_POINT
                            | FILE_SYNCHRONOUS_IO_NONALERT
                            | FILE_OPEN_FOR_BACKUP_INTENT,
                    ) {
                        if link_dir.metadata().is_ok_and(|m| is_link(&m)) {
                            let _ = mark_handle_deleted(&link_dir);
                        }
                    }
                    Ok(())
                }
                Err(s) => Err(format!("清理临时文件失败 (NTSTATUS 0x{:08X})", s as u32)),
            }
        }

        pub(super) fn list_entry_names(&self, fallback_path: &Path) -> Vec<String> {
            let live_dir = live_path_by_handle(&self.file);
            let read_target = live_dir.as_deref().unwrap_or(fallback_path);
            fs::read_dir(read_target)
                .or_else(|_| fs::read_dir(fallback_path))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default()
        }

        pub(super) fn try_remove_empty_subdir(&self, name: &str, child: &mut Option<RawDir>) {
            let Some(ref child_ref) = child else {
                return;
            };
            if validate_single_entry_name(name).is_err() {
                return;
            }
            let Ok(del_dir) = nt_open_relative(
                self.file.as_raw_handle(),
                name,
                DELETE_ACCESS | FILE_READ_ATTRIBUTES_ACCESS | SYNCHRONIZE_ACCESS,
                FILE_OPEN,
                FILE_DIRECTORY_FILE
                    | FILE_OPEN_REPARSE_POINT
                    | FILE_SYNCHRONOUS_IO_NONALERT
                    | FILE_OPEN_FOR_BACKUP_INTENT,
            ) else {
                return;
            };
            let same = del_dir
                .metadata()
                .is_ok_and(|meta| !is_link(&meta) && meta.is_dir())
                && file_id_by_handle(&del_dir).is_ok_and(|id| id == child_ref.id);
            if same {
                let _ = child.take();
                let _ = mark_handle_deleted(&del_dir);
            }
        }
    }
}

struct VerifiedDir {
    parent: Option<verified_fs::RawDir>,
    leaf_name: Option<String>,
    dir: Option<verified_fs::RawDir>,
}

impl VerifiedDir {
    fn open_internal(
        paths: &AppPaths,
        dir_path: &Path,
        create: bool,
    ) -> Result<Option<Self>, String> {
        ensure_plain_ancestors(paths, dir_path)?;
        let anchor = anchor_for_path(paths, dir_path)?;
        let rel = dir_path
            .strip_prefix(anchor)
            .map_err(|_| format!("技能写入路径不在应用目录内: {}", dir_path.display()))?;
        let mut segments = Vec::new();
        for comp in rel.components() {
            match comp {
                Component::Normal(seg) => {
                    let s = seg
                        .to_str()
                        .ok_or_else(|| format!("技能目录路径包含非 UTF-8 字符: {}", dir_path.display()))?;
                    validate_single_entry_name(s)?;
                    segments.push(s);
                }
                Component::CurDir => {}
                _ => return Err(format!("不安全的技能目录路径: {}", dir_path.display())),
            }
        }
        let Some(mut cur) = verified_fs::RawDir::open_anchor(anchor, create)? else {
            return Ok(None);
        };
        if segments.is_empty() {
            return Ok(Some(Self {
                parent: None,
                leaf_name: None,
                dir: Some(cur),
            }));
        }
        let mut step_path = anchor.to_path_buf();
        let last_idx = segments.len() - 1;
        for (idx, seg) in segments.into_iter().enumerate() {
            step_path.push(seg);
            let Some(next) = cur.open_or_create_subdir(seg, &step_path, create)? else {
                return Ok(None);
            };
            if idx == last_idx {
                return Ok(Some(Self {
                    parent: Some(cur),
                    leaf_name: Some(seg.to_string()),
                    dir: Some(next),
                }));
            }
            cur = next;
        }
        Ok(None)
    }

    fn open_existing(paths: &AppPaths, dir_path: &Path) -> Result<Option<Self>, String> {
        Self::open_internal(paths, dir_path, false)
    }

    fn open_or_create(paths: &AppPaths, dir_path: &Path) -> Result<Self, String> {
        Self::open_internal(paths, dir_path, true)?
            .ok_or_else(|| format!("无法打开技能目录: {}", dir_path.display()))
    }

    fn raw(&self) -> Result<&verified_fs::RawDir, String> {
        self.dir
            .as_ref()
            .ok_or_else(|| "已释放的目录句柄".to_string())
    }

    fn metadata(&self) -> Result<fs::Metadata, String> {
        self.raw()?
            .file
            .metadata()
            .map_err(|e| format!("读取目录元数据失败: {e}"))
    }

    fn open_child_dir(&self, name: &str, child_path: &Path) -> Result<Option<Self>, String> {
        let Some(sub) = self.raw()?.open_or_create_subdir(name, child_path, false)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            parent: None,
            leaf_name: Some(name.to_string()),
            dir: Some(sub),
        }))
    }

    fn create_child_dir(&self, name: &str, child_path: &Path) -> Result<Self, String> {
        let sub = self
            .raw()?
            .open_or_create_subdir(name, child_path, true)?
            .ok_or_else(|| format!("创建临时目录 {} 失败", child_path.display()))?;
        Ok(Self {
            parent: None,
            leaf_name: Some(name.to_string()),
            dir: Some(sub),
        })
    }

    fn open_regular_read(&self, name: &str) -> Result<Option<fs::File>, String> {
        self.raw()?.open_regular_read(name)
    }

    fn open_regular_append(&self, name: &str, create: bool) -> Result<fs::File, String> {
        self.raw()?.open_regular_append(name, create)
    }

    fn unlink_regular_file(&self, name: &str) -> Result<bool, String> {
        self.raw()?.unlink_regular_file(name)
    }

    fn unlink_stale_regular_file(&self, name: &str, now: SystemTime) -> bool {
        self.dir
            .as_ref()
            .is_some_and(|raw| raw.unlink_stale_regular_file(name, now))
    }

    fn list_entry_names(&self, fallback_path: &Path) -> Vec<String> {
        self.dir
            .as_ref()
            .map(|raw| raw.list_entry_names(fallback_path))
            .unwrap_or_default()
    }

    fn try_remove_self_if_empty(&mut self) {
        if let (Some(ref parent), Some(ref name)) = (&self.parent, &self.leaf_name) {
            parent.try_remove_empty_subdir(name, &mut self.dir);
        }
    }

    fn remove_child_tree(&self, name: &str, hint_path: &Path) -> Result<(), String> {
        validate_single_entry_name(name)?;
        let parent_raw = self.raw()?;
        match self.open_child_dir(name, hint_path) {
            Ok(Some(mut sub)) => {
                for child_name in sub.list_entry_names(hint_path) {
                    if validate_single_entry_name(&child_name).is_err() {
                        continue;
                    }
                    let _ = sub.remove_child_tree(&child_name, &hint_path.join(&child_name));
                }
                parent_raw.try_remove_empty_subdir(name, &mut sub.dir);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(_) => parent_raw.unlink_non_dir_entry(name),
        }
    }
}

fn read_store(paths: &AppPaths) -> Result<SkillsStore, String> {
    ensure_plain_ancestors(paths, &extensions_dir(paths))?;
    ensure_plain_ancestors(paths, &master_skills_dir(paths))?;
    let store: SkillsStore = read_json_store_checked(&skills_store_path(paths))?;
    let mut ids = BTreeSet::new();
    for skill in &store.skills {
        validate_identifier(&skill.id)?;
        let master = master_skills_dir(paths).join(&skill.id);
        if fs::symlink_metadata(&master).is_ok_and(|meta| is_link(&meta)) {
            return Err(format!("主副本不能是链接: {}", master.display()));
        }
        if !ids.insert(skill.id.to_lowercase()) {
            return Err(format!("技能中心存在重复标识符: {}", skill.id));
        }
    }
    Ok(store)
}

fn save_store(paths: &AppPaths, store: &SkillsStore) -> Result<(), String> {
    ensure_plain_ancestors(paths, &extensions_dir(paths))?;
    write_json_store(&skills_store_path(paths), store)
}

fn read_repos(paths: &AppPaths) -> Result<SkillReposStore, String> {
    ensure_plain_ancestors(paths, &extensions_dir(paths))?;
    let store: SkillReposStore = read_json_store_checked(&repos_store_path(paths))?;
    if !repos_store_path(paths).exists() {
        write_json_store(&repos_store_path(paths), &store)?;
    }
    Ok(store)
}

fn skipped(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "__pycache__"
            | ".venv"
            | "venv"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".DS_Store"
            | "Thumbs.db"
    )
}

/// 首页轻量只读技能计数：仅读取 `skills.json` 并浅层检查各端 `SKILL.md` 目录存在性，零文件读取、零落盘拷贝。
/// 返回 `(已发现/纳管技能总数, 已标记可更新技能数)`。
pub fn skills_summary_counts_for_paths(paths: &AppPaths) -> (usize, usize) {
    let mut seen = BTreeSet::new();
    let mut updatable = BTreeSet::new();
    let store: SkillsStore = read_json_store(&skills_store_path(paths));
    for skill in store.skills {
        if validate_identifier(&skill.id).is_ok() {
            let key = skill.id.to_lowercase();
            if skill.update_available {
                updatable.insert(key.clone());
            }
            seen.insert(key);
        }
    }
    let roots = std::iter::once(master_skills_dir(paths)).chain(
        ExtPlatform::all()
            .iter()
            .flat_map(|&platform| platform_skills_dirs(paths, platform)),
    );
    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') || validate_identifier(&id).is_err() {
                continue;
            }
            if entry.path().join("SKILL.md").is_file() {
                seen.insert(id.to_lowercase());
            }
        }
    }
    (seen.len(), updatable.len())
}

pub fn count_skills_for_summary_for_paths(paths: &AppPaths) -> usize {
    skills_summary_counts_for_paths(paths).0
}

fn cleanup_stale_temp_artifacts(paths: &AppPaths) {
    cleanup_temp_artifacts_at(paths, SystemTime::now());
}

fn sweep_stale_archives_in_verified_dir<I>(
    ext_verified: &VerifiedDir,
    ext_dir: &Path,
    names: I,
    now: SystemTime,
) where
    I: IntoIterator<Item = String>,
{
    for name in names {
        if !name.starts_with(".tako-archive-") || validate_single_entry_name(&name).is_err() {
            continue;
        }
        let hint_path = ext_dir.join(&name);
        let stale_dir = ext_verified
            .open_child_dir(&name, &hint_path)
            .ok()
            .flatten()
            .and_then(|sub| sub.metadata().ok())
            .is_some_and(|meta| is_stale_meta(&meta, now));
        if stale_dir {
            let _ = ext_verified.remove_child_tree(&name, &hint_path);
        } else {
            let _ = ext_verified.unlink_stale_regular_file(&name, now);
        }
    }
}

fn sweep_stale_uploads_in_verified_dir<I>(uploads_verified: &VerifiedDir, names: I, now: SystemTime)
where
    I: IntoIterator<Item = String>,
{
    for name in names {
        if name.ends_with(".zip") && validate_single_entry_name(&name).is_ok() {
            let _ = uploads_verified.unlink_stale_regular_file(&name, now);
        }
    }
}

fn cleanup_temp_artifacts_at(paths: &AppPaths, now: SystemTime) {
    let ext_dir = extensions_dir(paths);
    if let Ok(Some(ext_verified)) = VerifiedDir::open_existing(paths, &ext_dir) {
        let names = ext_verified.list_entry_names(&ext_dir);
        sweep_stale_archives_in_verified_dir(&ext_verified, &ext_dir, names, now);
    }
    let uploads = temp_uploads_dir(paths);
    if let Ok(Some(uploads_verified)) = VerifiedDir::open_existing(paths, &uploads) {
        let names = uploads_verified.list_entry_names(&uploads);
        sweep_stale_uploads_in_verified_dir(&uploads_verified, names, now);
    }
}

fn parse_frontmatter(bytes: &[u8], fallback: &str) -> Result<Frontmatter, String> {
    let raw = std::str::from_utf8(bytes).map_err(|e| format!("SKILL.md 不是 UTF-8: {e}"))?;
    let mut lines = raw.trim_start_matches('\u{feff}').lines();
    let mut frontmatter = Frontmatter::default();
    if lines.next().is_some_and(|line| line.trim() == "---") {
        let mut yaml = String::new();
        let mut closed = false;
        for line in lines {
            if matches!(line.trim(), "---" | "...") {
                closed = true;
                break;
            }
            yaml.push_str(line);
            yaml.push('\n');
        }
        if !closed {
            return Err("SKILL.md 的 YAML frontmatter 缺少结束分隔符".to_string());
        }
        if !yaml.trim().is_empty() {
            frontmatter = serde_yaml_ng::from_str(&yaml)
                .map_err(|e| format!("解析 SKILL.md frontmatter 失败: {e}"))?;
        }
    }
    if frontmatter.name.trim().is_empty() {
        frontmatter.name = fallback.to_string();
    }
    frontmatter.name = frontmatter.name.trim().to_string();
    frontmatter.description = frontmatter.description.trim().to_string();
    Ok(frontmatter)
}

fn read_frontmatter_file(skill_md: &Path, fallback: &str) -> Result<Frontmatter, String> {
    let file =
        fs::File::open(skill_md).map_err(|e| format!("打开 {} 失败: {e}", skill_md.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_SKILL_MD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 {} 失败: {e}", skill_md.display()))?;
    if bytes.len() as u64 > MAX_SKILL_MD_BYTES {
        return Err("SKILL.md 超过 2 MiB 解析上限".to_string());
    }
    parse_frontmatter(&bytes, fallback)
}

fn compute_file_meta_stamp(relative: &str, metadata: &fs::Metadata) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update((relative.len() as u64).to_le_bytes());
    hasher.update(relative.as_bytes());
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(dur) = modified.duration_since(UNIX_EPOCH) {
            hasher.update(dur.as_nanos().to_le_bytes());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        hasher.update(metadata.creation_time().to_le_bytes());
        hasher.update(metadata.last_write_time().to_le_bytes());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        hasher.update(metadata.ino().to_le_bytes());
        hasher.update(metadata.ctime_nsec().to_le_bytes());
        hasher.update(metadata.mtime_nsec().to_le_bytes());
    }
    hasher.finalize().into()
}

fn build_tree_from_entries(
    canonical_root: PathBuf,
    files: BTreeMap<String, LocalFileEntry>,
) -> LocalSkillTree {
    let mut total_bytes = 0u64;
    let mut meta_hasher = Sha256::new();
    for entry in files.values() {
        total_bytes = total_bytes.saturating_add(entry.size);
        meta_hasher.update(entry.meta_stamp);
    }
    LocalSkillTree {
        canonical_root,
        files,
        total_bytes,
        meta_digest: meta_hasher.finalize().into(),
    }
}

fn collect_tree_entries(
    root: &Path,
    max_bytes: u64,
) -> Result<(PathBuf, BTreeMap<String, LocalFileEntry>), String> {
    let canonical =
        fs::canonicalize(root).map_err(|e| format!("读取技能目录 {} 失败: {e}", root.display()))?;
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    collect_local_tree(&canonical, &canonical, "", &mut files, &mut total, 0, max_bytes)?;
    Ok((canonical, files))
}

fn scan_local_tree(root: &Path, max_bytes: u64) -> Result<LocalSkillTree, String> {
    let (canonical, files) = collect_tree_entries(root, max_bytes)?;
    if !files.contains_key("SKILL.md") {
        return Err(format!("技能目录缺少 SKILL.md: {}", root.display()));
    }
    Ok(build_tree_from_entries(canonical, files))
}

fn collect_local_tree(
    root: &Path,
    directory: &Path,
    prefix: &str,
    files: &mut BTreeMap<String, LocalFileEntry>,
    total: &mut u64,
    depth: usize,
    max_bytes: u64,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("技能目录嵌套过深".to_string());
    }
    let entries =
        fs::read_dir(directory).map_err(|e| format!("读取目录 {} 失败: {e}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取技能目录项失败: {e}"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "技能文件名不是 UTF-8")?;
        if skipped(&name) {
            continue;
        }
        validate_path_segment(&name)?;
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let canonical = fs::canonicalize(entry.path())
            .map_err(|e| format!("解析技能文件 {} 失败: {e}", entry.path().display()))?;
        if !canonical.starts_with(root) {
            return Err(format!("技能包含指向目录外部的链接: {relative}"));
        }
        let metadata =
            fs::metadata(&canonical).map_err(|e| format!("读取技能文件属性失败: {e}"))?;
        if metadata.is_dir() {
            collect_local_tree(
                root,
                &canonical,
                &relative,
                files,
                total,
                depth + 1,
                max_bytes,
            )?;
        } else if metadata.is_file() {
            let size = metadata.len();
            if files.len() >= MAX_FILES || size > max_bytes.saturating_sub(*total) {
                return Err(format!(
                    "技能超过 {} MiB 或文件数量上限",
                    max_bytes / (1024 * 1024)
                ));
            }
            *total += size;
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            let meta_stamp = compute_file_meta_stamp(&relative, &metadata);
            files.insert(
                relative,
                LocalFileEntry {
                    source: canonical,
                    size,
                    executable,
                    meta_stamp,
                },
            );
        } else {
            return Err(format!("技能包含不支持的文件类型: {relative}"));
        }
    }
    Ok(())
}

fn validate_path_segment(segment: &str) -> Result<(), String> {
    let reserved = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if segment.is_empty()
        || matches!(segment, "." | "..")
        || segment.ends_with(['.', ' '])
        || segment
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*/\\".contains(c))
        || matches!(reserved.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (reserved.len() == 4
            && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
            && matches!(reserved.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(format!("不安全的技能路径段: {segment}"));
    }
    Ok(())
}

struct TempExtractDir {
    parent: VerifiedDir,
    dir_name: String,
    path: PathBuf,
}

impl TempExtractDir {
    fn new(paths: &AppPaths) -> Result<Self, String> {
        let base = extensions_dir(paths);
        let parent = VerifiedDir::open_or_create(paths, &base)?;
        let nonce = format!("{}-{:016x}", std::process::id(), rand::random::<u64>());
        let dir_name = format!(".tako-archive-{nonce}");
        let path = base.join(&dir_name);
        let _created = parent.create_child_dir(&dir_name, &path)?;
        Ok(Self {
            parent,
            dir_name,
            path,
        })
    }
}

impl Drop for TempExtractDir {
    fn drop(&mut self) {
        let _ = self.parent.remove_child_tree(&self.dir_name, &self.path);
    }
}

struct StagedUploadCleanup {
    dir: VerifiedDir,
    file_name: String,
}

impl StagedUploadCleanup {
    fn open(paths: &AppPaths, upload_id: &str) -> Result<Option<Self>, String> {
        validate_identifier(upload_id)?;
        let uploads_dir = temp_uploads_dir(paths);
        let Some(dir) = VerifiedDir::open_existing(paths, &uploads_dir)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            dir,
            file_name: format!("{upload_id}.zip"),
        }))
    }
}

impl Drop for StagedUploadCleanup {
    fn drop(&mut self) {
        let _ = self.dir.unlink_regular_file(&self.file_name);
        self.dir.try_remove_self_if_empty();
    }
}

fn extract_zip_reader_to_dir<R: Read + Seek>(
    reader: R,
    dest: &Path,
    max_bytes: u64,
) -> Result<Vec<String>, String> {
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|e| format!("读取 ZIP 失败: {e}"))?;
    if archive.len() > MAX_FILES {
        return Err("ZIP 文件数量超过上限".to_string());
    }
    let mut extracted = Vec::new();
    let mut seen = BTreeSet::new();
    let mut path_kinds = BTreeMap::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; STREAM_BUF_SIZE];
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("读取 ZIP 项失败: {e}"))?;
        let name = entry.name().to_string();
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| format!("Zip Slip: 不安全的路径 {name}"))?;
        if name.starts_with(['/', '\\']) || name.contains('\\') || name.contains(':') {
            return Err(format!("Zip Slip: 不安全的路径 {name}"));
        }
        let segments: Vec<_> = name.trim_end_matches('/').split('/').collect();
        for segment in &segments {
            validate_path_segment(segment).map_err(|e| format!("Zip Slip: {e}"))?;
        }
        if segments.len() > MAX_DEPTH
            || enclosed
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(format!("Zip Slip: 路径越界或嵌套过深 {name}"));
        }
        let normalized = segments.join("/");
        if !seen.insert(normalized.to_lowercase()) {
            return Err(format!("ZIP 存在重复路径: {normalized}"));
        }
        let lower = normalized.to_lowercase();
        let mut parent = lower.as_str();
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            if path_kinds.get(prefix) == Some(&false) {
                return Err(format!("ZIP 文件与目录路径冲突: {name}"));
            }
            parent = prefix;
        }
        if !entry.is_dir()
            && path_kinds
                .keys()
                .any(|key: &String| key.starts_with(&format!("{lower}/")))
        {
            return Err(format!("ZIP 文件与目录路径冲突: {name}"));
        }
        path_kinds.insert(lower, entry.is_dir());
        let mode = entry.unix_mode().unwrap_or(0);
        if mode & 0o170000 != 0 && !matches!(mode & 0o170000, 0o100000 | 0o040000) {
            return Err(format!("ZIP 不允许链接或特殊文件: {name}"));
        }
        if entry.size() > max_bytes.saturating_sub(total) {
            return Err(format!(
                "ZIP 超过 {} MiB 总解压体积上限",
                max_bytes / (1024 * 1024)
            ));
        }
        total += entry.size();
        if entry.is_dir() || segments.iter().any(|seg| skipped(seg)) {
            continue;
        }
        let expected = entry.size();
        let out_path = dest.join(&normalized);
        if let Some(parent_dir) = out_path.parent() {
            fs::create_dir_all(parent_dir)
                .map_err(|e| format!("创建解压目录 {} 失败: {e}", parent_dir.display()))?;
        }
        let out_file = fs::File::create(&out_path)
            .map_err(|e| format!("创建解压文件 {} 失败: {e}", out_path.display()))?;
        let mut writer = BufWriter::with_capacity(STREAM_BUF_SIZE, out_file);
        let mut limited = (&mut entry).take(expected + 1);
        let mut written = 0u64;
        loop {
            let read = limited
                .read(&mut buf)
                .map_err(|e| format!("解压 ZIP 项 {name} 失败: {e}"))?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > expected {
                return Err(format!("ZIP 解压体积与声明不一致: {name}"));
            }
            writer
                .write_all(&buf[..read])
                .map_err(|e| format!("写入解压项 {name} 失败: {e}"))?;
        }
        writer
            .flush()
            .map_err(|e| format!("刷新解压项 {name} 失败: {e}"))?;
        if written != expected {
            return Err(format!("ZIP 解压体积与声明不一致: {name}"));
        }
        #[cfg(unix)]
        if mode & 0o111 != 0 {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&out_path, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("保留 ZIP 脚本执行权限失败: {e}"))?;
        }
        extracted.push(normalized);
    }
    Ok(extracted)
}

fn hash_tree_streaming(tree: &LocalSkillTree) -> Result<String, String> {
    let mut outer = Sha256::new();
    let mut buf = vec![0u8; STREAM_BUF_SIZE];
    for (path, entry) in &tree.files {
        let mut file = fs::File::open(&entry.source)
            .map_err(|e| format!("读取技能文件 {} 失败: {e}", entry.source.display()))?;
        let mut inner = Sha256::new();
        loop {
            let read = file
                .read(&mut buf)
                .map_err(|e| format!("读取技能文件 {} 失败: {e}", entry.source.display()))?;
            if read == 0 {
                break;
            }
            inner.update(&buf[..read]);
        }
        outer.update((path.len() as u64).to_le_bytes());
        outer.update(path.as_bytes());
        outer.update(inner.finalize());
    }
    Ok(hex::encode(outer.finalize()))
}

fn hash_tree_cached(tree: &LocalSkillTree, force_rehash: bool) -> Result<String, String> {
    if !force_rehash && tree.total_bytes >= HASH_CACHE_MIN_BYTES {
        let guard = TREE_HASH_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cache) = guard.as_ref() {
            if let Some((digest, hash)) = cache.get(&tree.canonical_root) {
                if *digest == tree.meta_digest {
                    return Ok(hash.clone());
                }
            }
        }
    }
    let computed = hash_tree_streaming(tree)?;
    if tree.total_bytes >= HASH_CACHE_MIN_BYTES {
        let mut guard = TREE_HASH_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = guard.get_or_insert_with(BTreeMap::new);
        if cache.len() >= HASH_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(
            tree.canonical_root.clone(),
            (tree.meta_digest, computed.clone()),
        );
    }
    Ok(computed)
}

fn scan_archive_trees(
    archive_root: &Path,
    max_depth: usize,
    max_bytes: u64,
) -> Result<Vec<(String, LocalSkillTree)>, String> {
    if !archive_root.exists() {
        return Err("未找到包含 SKILL.md 的技能目录".to_string());
    }
    let (canonical, files) = collect_tree_entries(archive_root, max_bytes)?;
    let mut roots = Vec::new();
    for path in files.keys() {
        let root = if path == "SKILL.md" {
            ""
        } else if let Some(root) = path.strip_suffix("/SKILL.md") {
            root
        } else {
            continue;
        };
        if root.is_empty() || root.split('/').count() <= max_depth {
            roots.push(root.to_string());
        }
    }
    if roots.is_empty() {
        return Err("未找到包含 SKILL.md 的技能目录".to_string());
    }
    if roots.len() > 200 {
        return Err("单次最多导入 200 个技能".to_string());
    }
    let mut result = Vec::new();
    for root in &roots {
        let prefix = if root.is_empty() {
            String::new()
        } else {
            format!("{root}/")
        };
        let nested: Vec<_> = roots
            .iter()
            .filter(|other| *other != root && other.starts_with(&prefix))
            .map(|other| format!("{other}/"))
            .collect();
        let subset: BTreeMap<String, LocalFileEntry> = files
            .iter()
            .filter_map(|(path, file)| {
                if nested.iter().any(|nested_prefix| path.starts_with(nested_prefix)) {
                    return None;
                }
                path.strip_prefix(&prefix).map(|relative| {
                    let mut entry = file.clone();
                    if !prefix.is_empty() {
                        if let Ok(meta) = fs::metadata(&entry.source) {
                            entry.meta_stamp = compute_file_meta_stamp(relative, &meta);
                        }
                    }
                    (relative.to_string(), entry)
                })
            })
            .collect();
        let sub_root = if root.is_empty() {
            canonical.clone()
        } else {
            canonical.join(root)
        };
        result.push((root.clone(), build_tree_from_entries(sub_root, subset)));
    }
    Ok(result)
}

fn remove_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Ok(());
        }
        Err(e) => return Err(format!("检查待清理路径 {} 失败: {e}", path.display())),
    };
    if is_link(&metadata) {
        #[cfg(windows)]
        let result = if metadata.is_dir() || metadata.file_type().is_symlink_dir() {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        };
        #[cfg(not(windows))]
        let result = fs::remove_file(path);
        return result.map_err(|e| format!("移除技能链接 {} 失败: {e}", path.display()));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| format!("读取待清理目录失败: {e}"))? {
            remove_path(&entry.map_err(|e| format!("读取目录项失败: {e}"))?.path())?;
        }
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| format!("清理技能路径 {} 失败: {e}", path.display()))
}

fn directory_link(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(source, target)
    }
}

struct DirectoryChange {
    destination: PathBuf,
    staged: Option<PathBuf>,
    previous: PathBuf,
    moved_previous: bool,
    installed: bool,
}

struct DirectoryTransaction<'a> {
    paths: &'a AppPaths,
    changes: Vec<DirectoryChange>,
    committed: bool,
}

impl<'a> DirectoryTransaction<'a> {
    fn new(paths: &'a AppPaths) -> Self {
        Self {
            paths,
            changes: Vec::new(),
            committed: false,
        }
    }

    fn reserve(&mut self, destination: &Path, removal: bool) -> Result<Option<PathBuf>, String> {
        let parent = destination.parent().ok_or("技能目标缺少父目录")?;
        ensure_plain_ancestors(self.paths, parent)?;
        fs::create_dir_all(parent).map_err(|e| format!("创建技能目录失败: {e}"))?;
        let nonce = format!("{}-{:016x}", std::process::id(), rand::random::<u64>());
        let staged = (!removal).then(|| parent.join(format!(".tako-skill-stage-{nonce}")));
        self.changes.push(DirectoryChange {
            destination: destination.to_path_buf(),
            staged: staged.clone(),
            previous: parent.join(format!(".tako-skill-previous-{nonce}")),
            moved_previous: false,
            installed: false,
        });
        Ok(staged)
    }

    fn stage_tree(
        &mut self,
        destination: &Path,
        tree: &LocalSkillTree,
        max_bytes: u64,
    ) -> Result<(), String> {
        let staged = self
            .reserve(destination, false)?
            .ok_or("缺少技能暂存路径")?;
        write_tree_streaming(&staged, tree, max_bytes)
    }

    fn stage_delivery(
        &mut self,
        master: &Path,
        destination: &Path,
        tree: Option<&LocalSkillTree>,
        method: SkillSyncMethod,
    ) -> Result<(), String> {
        self.stage_delivery_with(master, destination, tree, method, directory_link)
    }

    fn stage_delivery_with<F>(
        &mut self,
        master: &Path,
        destination: &Path,
        tree: Option<&LocalSkillTree>,
        method: SkillSyncMethod,
        create_link: F,
    ) -> Result<(), String>
    where
        F: FnOnce(&Path, &Path) -> std::io::Result<()>,
    {
        let staged = self
            .reserve(destination, false)?
            .ok_or("缺少技能暂存路径")?;
        if method != SkillSyncMethod::Copy {
            match create_link(master, &staged) {
                Ok(()) => return Ok(()),
                Err(e) if method == SkillSyncMethod::Symlink => {
                    return Err(format!("创建技能软链失败，请改用自动或复制模式: {e}"));
                }
                Err(_) => remove_path(&staged)?,
            }
        }
        let owned_tree;
        let resolved_tree = match tree {
            Some(existing) => existing,
            None => {
                owned_tree = scan_local_tree(master, MAX_LOCAL_SKILL_BYTES)?;
                &owned_tree
            }
        };
        write_tree_streaming(&staged, resolved_tree, MAX_LOCAL_SKILL_BYTES)
    }

    fn stage_remove(&mut self, destination: &Path) -> Result<(), String> {
        if path_exists(destination)? {
            self.reserve(destination, true)?;
        }
        Ok(())
    }

    fn apply(&mut self) -> Result<(), String> {
        for change in &mut self.changes {
            if path_exists(&change.destination)? {
                fs::rename(&change.destination, &change.previous).map_err(|e| {
                    format!("暂存原技能 {} 失败: {e}", change.destination.display())
                })?;
                change.moved_previous = true;
            }
            if let Some(staged) = &change.staged {
                fs::rename(staged, &change.destination).map_err(|e| {
                    format!("原子替换技能 {} 失败: {e}", change.destination.display())
                })?;
                change.installed = true;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        self.committed = true;
        for change in &self.changes {
            remove_path(&change.previous)?;
        }
        Ok(())
    }
}

impl Drop for DirectoryTransaction<'_> {
    fn drop(&mut self) {
        for change in self.changes.iter().rev() {
            let cleanup = || -> Result<(), String> {
                if !self.committed {
                    if change.installed {
                        remove_path(&change.destination)?;
                    }
                    if change.moved_previous {
                        fs::rename(&change.previous, &change.destination).map_err(|e| {
                            format!("恢复原技能失败，备份在 {}: {e}", change.previous.display())
                        })?;
                    }
                }
                if let Some(staged) = &change.staged {
                    remove_path(staged)?;
                }
                Ok(())
            };
            if let Err(error) = cleanup() {
                eprintln!("{error}");
            }
        }
    }
}

fn write_tree_streaming(
    directory: &Path,
    tree: &LocalSkillTree,
    max_bytes: u64,
) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|e| format!("创建技能暂存目录失败: {e}"))?;
    let mut copied_total = 0u64;
    let mut buf = vec![0u8; STREAM_BUF_SIZE];
    for (relative, entry) in &tree.files {
        let target = directory.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("创建技能子目录 {} 失败: {e}", parent.display()))?;
        }
        let src = fs::File::open(&entry.source)
            .map_err(|e| format!("读取技能文件 {} 失败: {e}", entry.source.display()))?;
        let dst = fs::File::create(&target)
            .map_err(|e| format!("创建技能文件 {} 失败: {e}", target.display()))?;
        let mut writer = BufWriter::with_capacity(STREAM_BUF_SIZE, dst);
        let mut limited = src.take(max_bytes.saturating_sub(copied_total) + 1);
        loop {
            let read = limited
                .read(&mut buf)
                .map_err(|e| format!("读取技能文件 {} 失败: {e}", entry.source.display()))?;
            if read == 0 {
                break;
            }
            copied_total += read as u64;
            if copied_total > max_bytes {
                return Err(format!(
                    "技能超过 {} MiB 体积上限",
                    max_bytes / (1024 * 1024)
                ));
            }
            writer
                .write_all(&buf[..read])
                .map_err(|e| format!("写入技能文件 {} 失败: {e}", target.display()))?;
        }
        writer
            .flush()
            .map_err(|e| format!("刷新技能文件 {} 失败: {e}", target.display()))?;
        #[cfg(unix)]
        if entry.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("保留技能脚本执行权限失败: {e}"))?;
        }
        #[cfg(not(unix))]
        let _ = entry.executable;
    }
    Ok(())
}

fn backup_target(
    paths: &AppPaths,
    platform: ExtPlatform,
    id: &str,
    target: &Path,
    index: usize,
) -> Result<(), String> {
    if !path_exists(target)? {
        return Ok(());
    }
    let meta = fs::symlink_metadata(target).map_err(|e| format!("读取原技能属性失败: {e}"))?;
    if is_link(&meta) {
        return Ok(());
    }
    let tree = scan_local_tree(target, MAX_LOCAL_SKILL_BYTES)?;
    for relative in tree.files.keys() {
        let backup_key = format!(
            "{}/{id}/{index}/{relative}",
            platform.display_name().to_lowercase()
        );
        let backup_path = paths.backups_dir("extensions-skills").join(&backup_key);
        ensure_plain_ancestors(paths, backup_path.parent().ok_or("技能备份路径无效")?)?;
        backup_file_once(
            paths,
            "extensions-skills",
            &backup_key,
            &target.join(relative),
        )?;
    }
    Ok(())
}

fn stage_platforms(
    paths: &AppPaths,
    transaction: &mut DirectoryTransaction<'_>,
    skill: &SkillRecord,
    tree: Option<&LocalSkillTree>,
    targets: &[ExtPlatform],
    method: SkillSyncMethod,
) -> Result<(), String> {
    let master = master_skills_dir(paths).join(&skill.id);
    for &platform in ExtPlatform::all() {
        let enabled = targets.contains(&platform);
        if !enabled && !skill.enabled_platforms.contains(&platform) {
            continue;
        }
        for (index, root) in platform_skills_dirs(paths, platform).iter().enumerate() {
            let destination = root.join(&skill.id);
            if !enabled && !path_exists(&destination)? {
                continue;
            }
            ensure_plain_ancestors(paths, root)?;
            backup_target(paths, platform, &skill.id, &destination, index)?;
            if enabled {
                transaction.stage_delivery(&master, &destination, tree, method)?;
            } else {
                transaction.stage_remove(&destination)?;
            }
        }
    }
    Ok(())
}

fn scan_adopt(
    paths: &AppPaths,
    store: &mut SkillsStore,
    force_rehash: bool,
) -> Result<Vec<String>, String> {
    cleanup_stale_temp_artifacts(paths);
    let known: BTreeSet<_> = store
        .skills
        .iter()
        .map(|skill| skill.id.to_lowercase())
        .collect();
    let mut warnings = Vec::new();
    let mut transaction = DirectoryTransaction::new(paths);
    let roots = std::iter::once((None, master_skills_dir(paths))).chain(
        ExtPlatform::all().iter().flat_map(|&platform| {
            platform_skills_dirs(paths, platform)
                .into_iter()
                .map(move |root| (Some(platform), root))
        }),
    );
    for (platform, root) in roots {
        if !root.is_dir() {
            continue;
        }
        if let Err(err) = ensure_plain_ancestors(paths, &root) {
            warnings.push(err);
            continue;
        }
        let mut entries = match fs::read_dir(&root)
            .and_then(|rd| rd.collect::<Result<Vec<_>, _>>())
        {
            Ok(entries) => entries,
            Err(e) => {
                warnings.push(format!("扫描技能目录 {} 失败: {e}", root.display()));
                continue;
            }
        };
        entries.sort_by_key(|entry| entry.file_name());
        let source_label = platform.map_or("主副本", ExtPlatform::display_name);
        for entry in entries {
            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') {
                continue;
            }
            let entry_path = entry.path();
            if known.contains(&id.to_lowercase()) {
                continue;
            }
            if !entry_path.join("SKILL.md").is_file() {
                if fs::symlink_metadata(&entry_path).is_ok_and(|meta| is_link(&meta))
                    && fs::metadata(&entry_path).is_err()
                {
                    warnings.push(format!(
                        "跳过 {source_label} 技能 {id}：目录软链目标不存在或已失效"
                    ));
                }
                continue;
            }
            if let Err(err) = validate_identifier(&id) {
                warnings.push(format!("跳过 {source_label} 技能 {id}：{err}"));
                continue;
            }
            let position = store
                .skills
                .iter()
                .position(|skill| skill.id.eq_ignore_ascii_case(&id));
            let position = if let Some(position) = position {
                position
            } else {
                let adopt_res = (|| -> Result<(SkillRecord, Option<LocalSkillTree>), String> {
                    if platform.is_none()
                        && fs::symlink_metadata(&entry_path)
                            .map_err(|e| e.to_string())?
                            .pipe_is_link()
                    {
                        return Err(format!("主副本不能是外部链接: {id}"));
                    }
                    let tree = scan_local_tree(&entry_path, MAX_LOCAL_SKILL_BYTES)?;
                    let skill_md = &tree.files["SKILL.md"].source;
                    let frontmatter = read_frontmatter_file(skill_md, &id)?;
                    let installed_hash = hash_tree_cached(&tree, force_rehash)?;
                    let record = SkillRecord {
                        id: id.clone(),
                        name: frontmatter.name,
                        description: frontmatter.description,
                        source: SkillSource::Adopted,
                        installed_hash,
                        ..Default::default()
                    };
                    Ok((record, platform.map(|_| tree)))
                })();
                match adopt_res {
                    Ok((record, maybe_tree)) => {
                        if let Some(tree) = maybe_tree {
                            let master = master_skills_dir(paths).join(&id);
                            if let Err(err) =
                                transaction.stage_tree(&master, &tree, MAX_LOCAL_SKILL_BYTES)
                            {
                                warnings.push(format!("跳过 {source_label} 技能 {id}：{err}"));
                                continue;
                            }
                        }
                        store.skills.push(record);
                        store.skills.len() - 1
                    }
                    Err(err) => {
                        warnings.push(format!("跳过 {source_label} 技能 {id}：{err}"));
                        continue;
                    }
                }
            };
            if let Some(platform) = platform {
                let skill = &mut store.skills[position];
                if !skill.enabled_platforms.contains(&platform) {
                    skill.enabled_platforms.push(platform);
                }
            }
        }
    }
    store.skills.sort_by(|a, b| a.id.cmp(&b.id));
    for skill in &mut store.skills {
        skill.enabled_platforms = ExtPlatform::all()
            .iter()
            .copied()
            .filter(|&platform| {
                platform_skills_dirs(paths, platform)
                    .iter()
                    .any(|root| root.join(&skill.id).join("SKILL.md").is_file())
            })
            .collect();
    }
    transaction.apply()?;
    save_store(paths, store)?;
    transaction.finish()?;
    Ok(warnings)
}

trait MetadataLinkExt {
    fn pipe_is_link(&self) -> bool;
}

impl MetadataLinkExt for fs::Metadata {
    fn pipe_is_link(&self) -> bool {
        is_link(self)
    }
}

fn snapshot(
    paths: &AppPaths,
    store: &SkillsStore,
    mut warnings: Vec<String>,
    force_rehash: bool,
) -> Result<SkillsHubSnapshot, String> {
    let mut skills = Vec::new();
    for skill in &store.skills {
        let master = master_skills_dir(paths).join(&skill.id);
        let master_hash = if master.join("SKILL.md").is_file() {
            match scan_local_tree(&master, MAX_LOCAL_SKILL_BYTES)
                .and_then(|tree| hash_tree_cached(&tree, force_rehash))
            {
                Ok(hash) => Some(hash),
                Err(err) => {
                    warnings.push(format!("主副本 {} 校验异常：{err}", skill.id));
                    None
                }
            }
        } else {
            None
        };
        let mut platforms = BTreeMap::new();
        for &platform in ExtPlatform::all() {
            let mut targets = Vec::new();
            for root in platform_skills_dirs(paths, platform) {
                let target = root.join(&skill.id);
                let exists = path_exists(&target)?;
                let metadata = fs::symlink_metadata(&target).ok();
                let actual_method = if exists {
                    Some(if metadata.as_ref().is_some_and(is_link) {
                        SkillLinkMode::Symlink
                    } else {
                        SkillLinkMode::Copy
                    })
                } else {
                    None
                };
                let matches_master_link = actual_method == Some(SkillLinkMode::Symlink)
                    && fs::canonicalize(&target).ok() == fs::canonicalize(&master).ok();
                let has_diff = if matches_master_link {
                    false
                } else if exists {
                    match scan_local_tree(&target, MAX_LOCAL_SKILL_BYTES)
                        .and_then(|tree| hash_tree_cached(&tree, force_rehash))
                    {
                        Ok(target_hash) => Some(target_hash) != master_hash,
                        Err(err) => {
                            warnings.push(format!(
                                "{} 技能 {} 校验异常：{err}",
                                platform.display_name(),
                                skill.id
                            ));
                            true
                        }
                    }
                } else {
                    skill.enabled_platforms.contains(&platform)
                };
                targets.push(SkillTargetStatus {
                    path: target.to_string_lossy().into_owned(),
                    actual_method,
                    has_diff,
                });
            }
            let enabled = targets.iter().any(|target| target.actual_method.is_some());
            let actual_method = if enabled {
                Some(
                    if targets
                        .iter()
                        .any(|target| target.actual_method == Some(SkillLinkMode::Copy))
                    {
                        SkillLinkMode::Copy
                    } else {
                        SkillLinkMode::Symlink
                    },
                )
            } else {
                None
            };
            platforms.insert(
                platform,
                SkillPlatformStatus {
                    enabled,
                    actual_method,
                    has_diff: targets.iter().any(|target| target.has_diff),
                    targets,
                },
            );
        }
        skills.push(ManagedSkill {
            skill: skill.clone(),
            master_path: master.to_string_lossy().into_owned(),
            platforms,
            locally_modified: master_hash.as_deref() != Some(skill.installed_hash.as_str()),
        });
    }
    Ok(SkillsHubSnapshot {
        sync_method: store.sync_method,
        default_targets: store.default_targets.clone(),
        master_dir: master_skills_dir(paths).to_string_lossy().into_owned(),
        skills,
        repos: read_repos(paths)?.repos,
        warnings,
    })
}

pub fn get_skills_hub_snapshot_for_paths(paths: &AppPaths) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let warnings = scan_adopt(paths, &mut store, false)?;
    snapshot(paths, &store, warnings, false)
}

pub fn scan_adopt_local_skills_for_paths(paths: &AppPaths) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let warnings = scan_adopt(paths, &mut store, true)?;
    snapshot(paths, &store, warnings, true)
}

fn align_platforms(
    paths: &AppPaths,
    store: &mut SkillsStore,
    id: &str,
    targets: &[ExtPlatform],
) -> Result<(), String> {
    validate_identifier(id)?;
    let position = store
        .skills
        .iter()
        .position(|skill| skill.id == id)
        .ok_or_else(|| format!("未找到技能: {id}"))?;
    let master = master_skills_dir(paths).join(id);
    let tree = (store.sync_method == SkillSyncMethod::Copy)
        .then(|| scan_local_tree(&master, MAX_LOCAL_SKILL_BYTES))
        .transpose()?;
    let targets = normalize_platforms(targets);
    let mut transaction = DirectoryTransaction::new(paths);
    stage_platforms(
        paths,
        &mut transaction,
        &store.skills[position],
        tree.as_ref(),
        &targets,
        store.sync_method,
    )?;
    transaction.apply()?;
    store.skills[position].enabled_platforms = targets;
    save_store(paths, store)?;
    transaction.finish()
}

pub fn toggle_skill_platform_for_paths(
    paths: &AppPaths,
    id: &str,
    platform: ExtPlatform,
    enabled: bool,
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    validate_identifier(id)?;
    let mut store = read_store(paths)?;
    let position = store
        .skills
        .iter()
        .position(|skill| skill.id == id)
        .ok_or_else(|| format!("未找到技能: {id}"))?;
    let master = master_skills_dir(paths).join(id);
    if enabled && !master.join("SKILL.md").is_file() {
        return Err(format!("主副本缺少 SKILL.md: {}", master.display()));
    }
    let tree = (enabled && store.sync_method == SkillSyncMethod::Copy)
        .then(|| scan_local_tree(&master, MAX_LOCAL_SKILL_BYTES))
        .transpose()?;
    let mut transaction = DirectoryTransaction::new(paths);
    for (index, root) in platform_skills_dirs(paths, platform).iter().enumerate() {
        let target = root.join(id);
        if !enabled && !path_exists(&target)? {
            continue;
        }
        ensure_plain_ancestors(paths, root)?;
        backup_target(paths, platform, id, &target, index)?;
        if enabled {
            transaction.stage_delivery(&master, &target, tree.as_ref(), store.sync_method)?;
        } else {
            transaction.stage_remove(&target)?;
        }
    }
    transaction.apply()?;
    store.skills[position]
        .enabled_platforms
        .retain(|target| *target != platform);
    if enabled {
        store.skills[position].enabled_platforms.push(platform);
    }
    save_store(paths, &store)?;
    transaction.finish()?;
    snapshot(paths, &store, Vec::new(), false)
}

pub fn align_skill_platforms_for_paths(
    paths: &AppPaths,
    id: &str,
    targets: &[ExtPlatform],
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    align_platforms(paths, &mut store, id, targets)?;
    snapshot(paths, &store, Vec::new(), true)
}

pub fn uninstall_skill_for_paths(paths: &AppPaths, id: &str) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    validate_identifier(id)?;
    let mut store = read_store(paths)?;
    if !store.skills.iter().any(|skill| skill.id == id) {
        return Err(format!("未找到技能: {id}"));
    }
    let mut transaction = DirectoryTransaction::new(paths);
    for &platform in ExtPlatform::all() {
        for (index, root) in platform_skills_dirs(paths, platform).iter().enumerate() {
            let target = root.join(id);
            if !path_exists(&target)? {
                continue;
            }
            ensure_plain_ancestors(paths, root)?;
            backup_target(paths, platform, id, &target, index)?;
            transaction.stage_remove(&target)?;
        }
    }
    transaction.stage_remove(&master_skills_dir(paths).join(id))?;
    transaction.apply()?;
    store.skills.retain(|skill| skill.id != id);
    save_store(paths, &store)?;
    transaction.finish()?;
    snapshot(paths, &store, Vec::new(), false)
}

pub fn set_skill_sync_method_for_paths(
    paths: &AppPaths,
    method: SkillSyncMethod,
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let mut transaction = DirectoryTransaction::new(paths);
    for skill in &store.skills {
        let master = master_skills_dir(paths).join(&skill.id);
        let tree = (method == SkillSyncMethod::Copy)
            .then(|| scan_local_tree(&master, MAX_LOCAL_SKILL_BYTES))
            .transpose()?;
        stage_platforms(
            paths,
            &mut transaction,
            skill,
            tree.as_ref(),
            &skill.enabled_platforms,
            method,
        )?;
    }
    transaction.apply()?;
    store.sync_method = method;
    save_store(paths, &store)?;
    transaction.finish()?;
    snapshot(paths, &store, Vec::new(), false)
}

pub fn set_skill_default_targets_for_paths(
    paths: &AppPaths,
    targets: &[ExtPlatform],
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    store.default_targets = normalize_platforms(targets);
    save_store(paths, &store)?;
    snapshot(paths, &store, Vec::new(), false)
}

fn new_record_from_tree(
    id: &str,
    tree: &LocalSkillTree,
    source: SkillSource,
) -> Result<SkillRecord, String> {
    validate_identifier(id)?;
    let skill_md = tree.files.get("SKILL.md").ok_or("缺少 SKILL.md")?;
    let frontmatter = read_frontmatter_file(&skill_md.source, id)?;
    Ok(SkillRecord {
        id: id.to_string(),
        name: frontmatter.name,
        description: frontmatter.description,
        source,
        installed_hash: hash_tree_cached(tree, true)?,
        ..Default::default()
    })
}

fn install_batch(
    paths: &AppPaths,
    store: &mut SkillsStore,
    incoming: Vec<(SkillRecord, LocalSkillTree)>,
    targets: &[ExtPlatform],
    max_bytes: u64,
) -> Result<(), String> {
    let mut ids: BTreeSet<_> = store
        .skills
        .iter()
        .map(|skill| skill.id.to_lowercase())
        .collect();
    for (skill, _) in &incoming {
        if !ids.insert(skill.id.to_lowercase())
            || path_exists(&master_skills_dir(paths).join(&skill.id))?
        {
            return Err(format!("技能 {} 已存在，未覆盖任何主副本", skill.id));
        }
        for &platform in ExtPlatform::all() {
            for root in platform_skills_dirs(paths, platform) {
                if path_exists(&root.join(&skill.id))? {
                    return Err(format!(
                        "{} 中已存在技能 {}，请先扫描收编，未覆盖原目录",
                        platform.display_name(),
                        skill.id
                    ));
                }
            }
        }
    }
    let mut transaction = DirectoryTransaction::new(paths);
    let targets = normalize_platforms(targets);
    for (mut skill, tree) in incoming {
        transaction.stage_tree(&master_skills_dir(paths).join(&skill.id), &tree, max_bytes)?;
        stage_platforms(
            paths,
            &mut transaction,
            &skill,
            Some(&tree),
            &targets,
            store.sync_method,
        )?;
        skill.enabled_platforms = targets.clone();
        store.skills.push(skill);
    }
    store.skills.sort_by(|a, b| a.id.cmp(&b.id));
    transaction.apply()?;
    save_store(paths, store)?;
    transaction.finish()
}

pub fn stage_skill_zip_chunk_for_paths(
    paths: &AppPaths,
    upload_id: Option<&str>,
    chunk_base64: &str,
) -> Result<String, String> {
    let _guard = lock_store()?;
    let uploads_dir = temp_uploads_dir(paths);
    ensure_plain_ancestors(paths, &uploads_dir)?;
    cleanup_stale_temp_artifacts(paths);
    let max_encoded = (MAX_UPLOAD_CHUNK_BYTES.div_ceil(3)) * 4 + 4;
    if chunk_base64.len() > max_encoded {
        return Err("单次分片超过 8 MiB 上限".to_string());
    }
    let chunk = base64::engine::general_purpose::STANDARD
        .decode(chunk_base64)
        .map_err(|e| format!("ZIP 分片 Base64 无效: {e}"))?;
    let mut uploads_verified = VerifiedDir::open_or_create(paths, &uploads_dir)?;
    let (id, create_new) = match upload_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => {
            validate_identifier(existing)?;
            (existing.to_string(), false)
        }
        None => (
            format!("zip-{}-{:016x}", std::process::id(), rand::random::<u64>()),
            true,
        ),
    };
    let file_name = format!("{id}.zip");
    let mut file = uploads_verified.open_regular_append(&file_name, create_new)?;
    let existing_len = file.metadata().map_or(0, |meta| meta.len());
    if chunk.len() as u64 > MAX_ARCHIVE_BYTES.saturating_sub(existing_len) {
        drop(file);
        let _ = uploads_verified.unlink_regular_file(&file_name);
        uploads_verified.try_remove_self_if_empty();
        return Err("ZIP 文件超过 200 MiB 上限".to_string());
    }
    file.write_all(&chunk)
        .and_then(|()| file.flush())
        .map_err(|e| format!("保存分片临时文件失败: {e}"))?;
    Ok(id)
}

pub fn discard_skill_zip_upload_for_paths(paths: &AppPaths, upload_id: &str) -> Result<(), String> {
    let _guard = lock_store()?;
    let trimmed = upload_id.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let _cleanup = StagedUploadCleanup::open(paths, trimmed)?;
    Ok(())
}

pub fn import_local_skill_for_paths(
    paths: &AppPaths,
    input: ImportLocalSkillRequest,
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let targets = input
        .targets
        .unwrap_or_else(|| store.default_targets.clone());
    let path_opt = input.path.filter(|path| !path.trim().is_empty());
    let base64_opt = input.zip_base64.filter(|encoded| !encoded.trim().is_empty());
    let upload_opt = input.upload_id.filter(|id| !id.trim().is_empty());

    let mut temp_extract: Option<TempExtractDir> = None;
    let (incoming, max_bytes) = match (path_opt, base64_opt, upload_opt) {
        (Some(path), None, None) => {
            let path = PathBuf::from(path.trim());
            if path.is_dir() {
                let tree = scan_local_tree(&path, MAX_LOCAL_SKILL_BYTES)?;
                let id = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("技能目录名无效")?;
                (
                    vec![(new_record_from_tree(id, &tree, SkillSource::Local)?, tree)],
                    MAX_LOCAL_SKILL_BYTES,
                )
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
            {
                let meta = fs::metadata(&path).map_err(|e| format!("读取 ZIP 失败: {e}"))?;
                if meta.len() > MAX_ARCHIVE_BYTES {
                    return Err("ZIP 文件超过 200 MiB 上限".to_string());
                }
                let file = fs::File::open(&path).map_err(|e| format!("打开 ZIP 失败: {e}"))?;
                let temp = TempExtractDir::new(paths)?;
                let records = extract_local_archive_records(file, &temp.path)?;
                temp_extract = Some(temp);
                (records, MAX_ARCHIVE_BYTES)
            } else {
                return Err("请选择包含 SKILL.md 的目录或 .zip 文件".to_string());
            }
        }
        (None, Some(encoded), None) => {
            if encoded.len() as u64 > MAX_ARCHIVE_BYTES.div_ceil(3) * 4 + 4 {
                return Err("ZIP Base64 超过 200 MiB 体积上限".to_string());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|e| format!("ZIP Base64 无效: {e}"))?;
            if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
                return Err("ZIP 文件超过 200 MiB 上限".to_string());
            }
            let temp = TempExtractDir::new(paths)?;
            let records = extract_local_archive_records(Cursor::new(bytes), &temp.path)?;
            temp_extract = Some(temp);
            (records, MAX_ARCHIVE_BYTES)
        }
        (None, None, Some(upload_id)) => {
            let trimmed = upload_id.trim();
            let cleanup = StagedUploadCleanup::open(paths, trimmed)?
                .ok_or_else(|| "分片上传文件不存在或已过期".to_string())?;
            let file = cleanup
                .dir
                .open_regular_read(&cleanup.file_name)?
                .ok_or_else(|| "分片上传文件不存在或已过期".to_string())?;
            let meta = file
                .metadata()
                .map_err(|e| format!("读取分片 ZIP 失败: {e}"))?;
            if meta.len() > MAX_ARCHIVE_BYTES {
                return Err("ZIP 文件超过 200 MiB 上限".to_string());
            }
            let temp = TempExtractDir::new(paths)?;
            let records = extract_local_archive_records(file, &temp.path)?;
            temp_extract = Some(temp);
            (records, MAX_ARCHIVE_BYTES)
        }
        _ => return Err("请只提供一个本地路径或 ZIP 文件".to_string()),
    };

    install_batch(paths, &mut store, incoming, &targets, max_bytes)?;
    drop(temp_extract);
    snapshot(paths, &store, Vec::new(), false)
}

fn extract_local_archive_records<R: Read + Seek>(
    reader: R,
    extract_dir: &Path,
) -> Result<Vec<(SkillRecord, LocalSkillTree)>, String> {
    extract_zip_reader_to_dir(reader, extract_dir, MAX_ARCHIVE_BYTES)?;
    scan_archive_trees(extract_dir, 2, MAX_ARCHIVE_BYTES)?
        .into_iter()
        .map(|(root, tree)| {
            let id = if root.is_empty() {
                read_frontmatter_file(&tree.files["SKILL.md"].source, "imported-skill")?.name
            } else {
                root.rsplit('/').next().unwrap_or_default().to_string()
            };
            Ok((new_record_from_tree(&id, &tree, SkillSource::Zip)?, tree))
        })
        .collect()
}

fn normalize_repo(mut repo: SkillRepo) -> Result<SkillRepo, String> {
    repo.owner = repo.owner.trim().to_string();
    repo.name = repo.name.trim().to_string();
    for value in [&repo.owner, &repo.name] {
        validate_identifier(value)?;
        if !value.is_ascii() {
            return Err("GitHub owner/repo 只能使用 ASCII 标识符".to_string());
        }
    }
    repo.branch = repo.branch.trim().to_string();
    if repo.branch.is_empty() {
        repo.branch = main_branch();
    }
    if repo.branch.len() > 200
        || repo
            .branch
            .split('/')
            .any(|part| validate_identifier(part).is_err())
    {
        return Err("GitHub 分支名无效".to_string());
    }
    if let Some(url) = &repo.archive_url_override {
        let parsed = reqwest::Url::parse(&url.replace("{branch}", "main"))
            .map_err(|e| format!("仓库归档地址无效: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("仓库归档地址必须是无内嵌凭据的 HTTP(S) URL".to_string());
        }
    }
    Ok(repo)
}

pub fn upsert_skill_repo_for_paths(
    paths: &AppPaths,
    repo: SkillRepo,
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let repo = normalize_repo(repo)?;
    let mut repos = read_repos(paths)?;
    repos.repos.retain(|existing| {
        !(existing.owner.eq_ignore_ascii_case(&repo.owner)
            && existing.name.eq_ignore_ascii_case(&repo.name))
    });
    repos.repos.push(repo);
    write_json_store(&repos_store_path(paths), &repos)?;
    snapshot(paths, &read_store(paths)?, Vec::new(), false)
}

pub fn remove_skill_repo_for_paths(
    paths: &AppPaths,
    owner: &str,
    name: &str,
) -> Result<SkillsHubSnapshot, String> {
    let _guard = lock_store()?;
    let mut repos = read_repos(paths)?;
    repos.repos.retain(|repo| {
        !(repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name))
    });
    write_json_store(&repos_store_path(paths), &repos)?;
    snapshot(paths, &read_store(paths)?, Vec::new(), false)
}

fn find_repo(paths: &AppPaths, owner: &str, name: &str) -> Result<SkillRepo, String> {
    let _guard = lock_store()?;
    let repo = read_repos(paths)?
        .repos
        .into_iter()
        .find(|repo| repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name))
        .ok_or("未订阅该 GitHub 仓库，请先添加仓库源")?;
    if !repo.enabled {
        return Err("该仓库源已停用".to_string());
    }
    normalize_repo(repo)
}

fn archive_url(repo: &SkillRepo, branch: &str) -> Result<reqwest::Url, String> {
    if let Some(url) = &repo.archive_url_override {
        return reqwest::Url::parse(&url.replace("{branch}", branch))
            .map_err(|e| format!("仓库归档地址无效: {e}"));
    }
    let mut url = reqwest::Url::parse("https://codeload.github.com").map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "仓库归档路径无效")?
        .extend([
            repo.owner.as_str(),
            repo.name.as_str(),
            "zip",
            "refs",
            "heads",
        ])
        .extend(branch.split('/'));
    Ok(url)
}

struct RepoArchive {
    _temp: TempExtractDir,
    branch: String,
    skills: Vec<(String, LocalSkillTree)>,
}

async fn fetch_repo_archive(paths: &AppPaths, repo: &SkillRepo) -> Result<RepoArchive, String> {
    let client = build_http_client(paths, Duration::from_secs(45), true)?;
    let mut branch = repo.branch.clone();
    let mut response = client
        .get(archive_url(repo, &branch)?)
        .send()
        .await
        .map_err(|e| format!("下载仓库归档失败: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND && branch == "main" {
        branch = "master".to_string();
        response = client
            .get(archive_url(repo, &branch)?)
            .send()
            .await
            .map_err(|e| format!("下载 master 归档失败: {e}"))?;
    }
    if !response.status().is_success() {
        return Err(format!("下载仓库归档失败: HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE_BYTES)
    {
        return Err("仓库 ZIP 超过 200 MiB 上限".to_string());
    }

    let temp = TempExtractDir::new(paths)?;
    let zip_path = temp.path.join("repo.zip");
    let extracted_dir = temp.path.join("extracted");
    fs::create_dir_all(&extracted_dir)
        .map_err(|e| format!("创建仓库解压目录失败: {e}"))?;
    {
        let zip_file =
            fs::File::create(&zip_path).map_err(|e| format!("创建临时归档文件失败: {e}"))?;
        let mut writer = BufWriter::with_capacity(STREAM_BUF_SIZE, zip_file);
        let mut downloaded = 0u64;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("读取仓库归档失败: {e}"))?
        {
            downloaded += chunk.len() as u64;
            if downloaded > MAX_ARCHIVE_BYTES {
                return Err("仓库 ZIP 超过 200 MiB 上限".to_string());
            }
            writer
                .write_all(&chunk)
                .map_err(|e| format!("写入临时归档失败: {e}"))?;
        }
        writer
            .flush()
            .map_err(|e| format!("刷新临时归档失败: {e}"))?;
    }

    let zip_reader = fs::File::open(&zip_path).map_err(|e| format!("打开临时归档失败: {e}"))?;
    let extracted_files = extract_zip_reader_to_dir(zip_reader, &extracted_dir, MAX_ARCHIVE_BYTES)?;
    let _ = fs::remove_file(&zip_path);

    // GitHub wraps archives in one repository directory; strip that wrapper without moving files.
    let first = extracted_files
        .first()
        .and_then(|path| path.split_once('/'))
        .map(|(prefix, _)| prefix.to_string());
    let repo_root = if let Some(prefix) = first.filter(|prefix| {
        extracted_files
            .iter()
            .all(|path| path.starts_with(&format!("{prefix}/")))
    }) {
        extracted_dir.join(prefix)
    } else {
        extracted_dir
    };

    let skills = scan_archive_trees(&repo_root, MAX_DEPTH, MAX_ARCHIVE_BYTES)?;
    Ok(RepoArchive {
        _temp: temp,
        branch,
        skills,
    })
}

fn same_source(skill: &SkillRecord, repo: &SkillRepo, branch: &str, subdir: &str) -> bool {
    skill.source == SkillSource::Github
        && skill
            .repo_owner
            .as_deref()
            .is_some_and(|owner| owner.eq_ignore_ascii_case(&repo.owner))
        && skill
            .repo_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(&repo.name))
        && skill.repo_branch.as_deref() == Some(branch)
        && skill.repo_subdir.as_deref() == Some(subdir)
}

pub async fn discover_repo_skills_for_paths(
    paths: &AppPaths,
    owner: &str,
    name: &str,
) -> Result<Vec<DiscoverableSkill>, String> {
    let repo = find_repo(paths, owner, name)?;
    let archive = fetch_repo_archive(paths, &repo).await?;
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let mut discovered = Vec::new();
    for (subdir, tree) in archive.skills {
        let metadata = read_frontmatter_file(
            &tree.files["SKILL.md"].source,
            subdir.rsplit('/').next().unwrap_or(&repo.name),
        )?;
        let id = if subdir.is_empty() {
            metadata.name.clone()
        } else {
            subdir.rsplit('/').next().unwrap_or_default().to_string()
        };
        validate_identifier(&id)?;
        let remote_hash = hash_tree_streaming(&tree)?;
        let installed = store
            .skills
            .iter_mut()
            .find(|skill| same_source(skill, &repo, &archive.branch, &subdir));
        let (installed_id, update_available) = if let Some(installed) = installed {
            installed.remote_hash = Some(remote_hash.clone());
            installed.update_available = installed.installed_hash != remote_hash;
            (Some(installed.id.clone()), installed.update_available)
        } else {
            (None, false)
        };
        discovered.push(DiscoverableSkill {
            id,
            name: metadata.name,
            description: metadata.description,
            repo_owner: repo.owner.clone(),
            repo_name: repo.name.clone(),
            repo_branch: archive.branch.clone(),
            repo_subdir: subdir,
            remote_hash,
            installed_id,
            update_available,
        });
    }
    save_store(paths, &store)?;
    Ok(discovered)
}

pub async fn install_discovered_skill_for_paths(
    paths: &AppPaths,
    owner: &str,
    name: &str,
    subdir: &str,
    targets: Option<Vec<ExtPlatform>>,
) -> Result<SkillsHubSnapshot, String> {
    let repo = find_repo(paths, owner, name)?;
    let archive = fetch_repo_archive(paths, &repo).await?;
    let (_, tree) = archive
        .skills
        .into_iter()
        .find(|(root, _)| root == subdir)
        .ok_or("仓库中未找到该技能，请刷新发现列表")?;
    let metadata = read_frontmatter_file(&tree.files["SKILL.md"].source, &repo.name)?;
    let id = if subdir.is_empty() {
        metadata.name
    } else {
        subdir.rsplit('/').next().unwrap_or_default().to_string()
    };
    let mut skill = new_record_from_tree(&id, &tree, SkillSource::Github)?;
    skill.repo_owner = Some(repo.owner.clone());
    skill.repo_name = Some(repo.name.clone());
    skill.repo_branch = Some(archive.branch);
    skill.repo_subdir = Some(subdir.to_string());
    skill.remote_hash = Some(skill.installed_hash.clone());
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let targets = targets.unwrap_or_else(|| store.default_targets.clone());
    install_batch(paths, &mut store, vec![(skill, tree)], &targets, MAX_ARCHIVE_BYTES)?;
    snapshot(paths, &store, Vec::new(), false)
}

pub async fn update_installed_skill_for_paths(
    paths: &AppPaths,
    id: &str,
) -> Result<SkillsHubSnapshot, String> {
    validate_identifier(id)?;
    let original = {
        let _guard = lock_store()?;
        read_store(paths)?
            .skills
            .into_iter()
            .find(|skill| skill.id == id)
            .ok_or("未找到已安装技能")?
    };
    if original.source != SkillSource::Github || !original.update_available {
        return Err("该技能没有可用的仓库更新".to_string());
    }
    let mut repo = find_repo(
        paths,
        original.repo_owner.as_deref().ok_or("技能缺少仓库 owner")?,
        original.repo_name.as_deref().ok_or("技能缺少仓库名称")?,
    )?;
    repo.branch = original.repo_branch.clone().ok_or("技能缺少仓库分支")?;
    let archive = fetch_repo_archive(paths, &repo).await?;
    let subdir = original
        .repo_subdir
        .as_deref()
        .ok_or("技能缺少仓库子目录")?;
    let (_, tree) = archive
        .skills
        .into_iter()
        .find(|(root, _)| root == subdir)
        .ok_or("远端已移除该技能，未更改本地主副本")?;
    let _guard = lock_store()?;
    let mut store = read_store(paths)?;
    let position = store
        .skills
        .iter()
        .position(|skill| skill.id == id && same_source(skill, &repo, &repo.branch, subdir))
        .ok_or("技能来源已变化，请重新检查更新")?;
    let current = &store.skills[position];
    let master = master_skills_dir(paths).join(id);
    let existing = scan_local_tree(&master, MAX_LOCAL_SKILL_BYTES)?;
    if hash_tree_cached(&existing, true)? != current.installed_hash {
        return Err("主副本包含本地修改，请先备份修改后再更新".to_string());
    }
    for &platform in &current.enabled_platforms {
        for target in platform_skills_dirs(paths, platform)
            .into_iter()
            .map(|root| root.join(id))
        {
            if target.join("SKILL.md").is_file() {
                let target_tree = scan_local_tree(&target, MAX_LOCAL_SKILL_BYTES)?;
                if hash_tree_cached(&target_tree, true)? != current.installed_hash {
                    return Err(format!(
                        "{} 副本包含本地修改，请先三端对齐或备份",
                        platform.display_name()
                    ));
                }
            }
        }
    }
    let metadata = read_frontmatter_file(&tree.files["SKILL.md"].source, id)?;
    let new_hash = hash_tree_cached(&tree, true)?;
    let mut transaction = DirectoryTransaction::new(paths);
    transaction.stage_tree(&master, &tree, MAX_ARCHIVE_BYTES)?;
    stage_platforms(
        paths,
        &mut transaction,
        current,
        Some(&tree),
        &current.enabled_platforms,
        store.sync_method,
    )?;
    transaction.apply()?;
    let current = &mut store.skills[position];
    current.name = metadata.name;
    current.description = metadata.description;
    current.installed_hash = new_hash;
    current.remote_hash = Some(current.installed_hash.clone());
    current.repo_branch = Some(archive.branch);
    current.update_available = false;
    save_store(paths, &store)?;
    transaction.finish()?;
    snapshot(paths, &store, Vec::new(), false)
}

#[tauri::command]
pub async fn get_skills_hub_snapshot() -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(get_skills_hub_snapshot_for_paths).await
}

#[tauri::command]
pub async fn scan_adopt_local_skills() -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(scan_adopt_local_skills_for_paths).await
}

#[tauri::command]
pub async fn toggle_skill_platform(
    id: String,
    platform: ExtPlatform,
    enabled: bool,
) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| toggle_skill_platform_for_paths(paths, &id, platform, enabled))
        .await
}

#[tauri::command]
pub async fn align_skill_platforms(
    id: String,
    targets: Vec<ExtPlatform>,
) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| align_skill_platforms_for_paths(paths, &id, &targets)).await
}

#[tauri::command]
pub async fn uninstall_skill(id: String) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| uninstall_skill_for_paths(paths, &id)).await
}

#[tauri::command]
pub async fn set_skill_sync_method(method: SkillSyncMethod) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| set_skill_sync_method_for_paths(paths, method)).await
}

#[tauri::command]
pub async fn set_skill_default_targets(
    targets: Vec<ExtPlatform>,
) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| set_skill_default_targets_for_paths(paths, &targets)).await
}

#[tauri::command]
pub async fn stage_skill_zip_chunk(
    upload_id: Option<String>,
    chunk_base64: String,
) -> Result<String, String> {
    with_extension_paths(move |paths| {
        stage_skill_zip_chunk_for_paths(paths, upload_id.as_deref(), &chunk_base64)
    })
    .await
}

#[tauri::command]
pub async fn discard_skill_zip_upload(upload_id: String) -> Result<(), String> {
    with_extension_paths(move |paths| discard_skill_zip_upload_for_paths(paths, &upload_id)).await
}

#[tauri::command]
pub async fn import_local_skill(
    input: ImportLocalSkillRequest,
) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| import_local_skill_for_paths(paths, input)).await
}

#[tauri::command]
pub async fn upsert_skill_repo(repo: SkillRepo) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| upsert_skill_repo_for_paths(paths, repo)).await
}

#[tauri::command]
pub async fn remove_skill_repo(owner: String, name: String) -> Result<SkillsHubSnapshot, String> {
    with_extension_paths(move |paths| remove_skill_repo_for_paths(paths, &owner, &name)).await
}

#[tauri::command]
pub async fn discover_repo_skills(
    owner: String,
    name: String,
) -> Result<Vec<DiscoverableSkill>, String> {
    discover_repo_skills_for_paths(&AppPaths::detect()?, &owner, &name).await
}

#[tauri::command]
pub async fn install_discovered_skill(
    owner: String,
    name: String,
    subdir: String,
    targets: Option<Vec<ExtPlatform>>,
) -> Result<SkillsHubSnapshot, String> {
    install_discovered_skill_for_paths(&AppPaths::detect()?, &owner, &name, &subdir, targets).await
}

#[tauri::command]
pub async fn update_installed_skill(id: String) -> Result<SkillsHubSnapshot, String> {
    update_installed_skill_for_paths(&AppPaths::detect()?, &id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::write_atomic;

    fn seed_skill_dir(dir: &Path, body: &[u8]) -> LocalSkillTree {
        write_atomic(&dir.join("SKILL.md"), body, false).unwrap();
        scan_local_tree(dir, MAX_LOCAL_SKILL_BYTES).unwrap()
    }

    #[test]
    fn paths_beneath_a_file_are_absent_but_invalid_for_writes() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let file = paths.home.join("blocked");
        fs::write(&file, b"preserve").unwrap();
        assert!(path_exists(&file).unwrap());

        for child in [file.join("skills"), file.join("config/skills")] {
            assert!(!path_exists(&child).unwrap());
            remove_path(&child).unwrap();
            assert_eq!(
                ensure_plain_ancestors(&paths, &child).unwrap_err(),
                format!("技能父路径不是目录: {}", file.display())
            );
        }
        assert_eq!(fs::read(&file).unwrap(), b"preserve");
    }

    #[cfg(unix)]
    #[test]
    fn non_directory_above_anchor_is_reported_at_anchor() {
        let temp = tempfile::TempDir::new().unwrap();
        let file = temp.path().join("blocked");
        fs::write(&file, b"preserve").unwrap();
        let paths = AppPaths::for_test(&file.join("home"));
        let target = paths.agy_gemini_config_dir.join("skills");

        assert_eq!(
            ensure_plain_ancestors(&paths, &target).unwrap_err(),
            format!("技能父路径不是目录: {}", paths.home.display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn file_link_ancestor_is_rejected_for_nested_writes() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let file = paths.home.join("blocked");
        let link = paths.home.join(".gemini");
        fs::write(&file, b"preserve").unwrap();
        std::os::unix::fs::symlink(&file, &link).unwrap();

        assert_eq!(
            ensure_plain_ancestors(&paths, &link.join("config/skills")).unwrap_err(),
            format!("拒绝通过链接目录写入技能: {}", link.display())
        );
    }

    #[test]
    fn auto_falls_back_after_permission_denied_without_changing_master() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        assert!(paths.is_test);
        let master = master_skills_dir(&paths).join("fallback");
        let tree = seed_skill_dir(&master, b"# fallback");
        let destination = paths.codex_dir.join("skills/fallback");
        let mut transaction = DirectoryTransaction::new(&paths);
        transaction
            .stage_delivery_with(
                &master,
                &destination,
                Some(&tree),
                SkillSyncMethod::Auto,
                |_, _| Err(std::io::Error::from(ErrorKind::PermissionDenied)),
            )
            .unwrap();
        transaction.apply().unwrap();
        transaction.finish().unwrap();
        assert!(!is_link(&fs::symlink_metadata(&destination).unwrap()));
        assert_eq!(
            fs::read(destination.join("SKILL.md")).unwrap(),
            b"# fallback"
        );
        assert_eq!(fs::read(master.join("SKILL.md")).unwrap(), b"# fallback");
    }

    #[test]
    fn uncommitted_directory_replacements_restore_previous_contents() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        assert!(paths.is_test);
        let destination = paths.codex_dir.join("skills/rollback");
        write_atomic(&destination.join("SKILL.md"), b"original", false).unwrap();
        let source = paths.home.join("incoming/rollback");
        let replacement = seed_skill_dir(&source, b"replacement");
        {
            let mut transaction = DirectoryTransaction::new(&paths);
            transaction
                .stage_tree(&destination, &replacement, MAX_LOCAL_SKILL_BYTES)
                .unwrap();
            transaction.apply().unwrap();
            assert_eq!(
                fs::read(destination.join("SKILL.md")).unwrap(),
                b"replacement"
            );
        }
        assert_eq!(fs::read(destination.join("SKILL.md")).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(destination.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn large_tree_hash_cache_hits_and_invalidates_on_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("heavy-skill");
        write_atomic(&dir.join("SKILL.md"), b"# heavy", false).unwrap();
        let payload = vec![b'a'; 300 * 1024];
        write_atomic(&dir.join("assets/bundle.bin"), &payload, false).unwrap();

        let tree1 = scan_local_tree(&dir, MAX_LOCAL_SKILL_BYTES).unwrap();
        let h1 = hash_tree_cached(&tree1, false).unwrap();
        let h2 = hash_tree_cached(&tree1, false).unwrap();
        assert_eq!(h1, h2);

        let mut updated = payload;
        updated[0] = b'z';
        write_atomic(&dir.join("assets/bundle.bin"), &updated, false).unwrap();
        let tree2 = scan_local_tree(&dir, MAX_LOCAL_SKILL_BYTES).unwrap();
        let h3 = hash_tree_cached(&tree2, false).unwrap();
        assert_ne!(h1, h3);
    }

    fn create_test_dir_link(source: &Path, link: &Path) {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(source, link).unwrap();
        }
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args([
                    "/C",
                    "mklink",
                    "/J",
                    &link.to_string_lossy(),
                    &source.to_string_lossy(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "mklink /J failed");
        }
    }

    #[test]
    fn temp_upload_cleanup_and_staging_never_traverse_directory_links() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let external = tmp.path().join("external-target");
        write_atomic(&external.join("stale-upload.zip"), b"keep-zip", false).unwrap();
        write_atomic(&external.join("notes.txt"), b"keep-notes", false).unwrap();
        write_atomic(&external.join("nested/doc.md"), b"keep-nested", false).unwrap();

        let ext_dir = extensions_dir(&paths);
        fs::create_dir_all(&ext_dir).unwrap();
        let uploads_link = temp_uploads_dir(&paths);
        create_test_dir_link(&external, &uploads_link);
        assert!(is_link(&fs::symlink_metadata(&uploads_link).unwrap()));

        let future_now = SystemTime::now() + STALE_TEMP_TTL + Duration::from_secs(60);
        cleanup_temp_artifacts_at(&paths, future_now);
        let _snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();

        let err_stage = stage_skill_zip_chunk_for_paths(&paths, None, "UEsD").unwrap_err();
        assert!(err_stage.contains("拒绝通过链接目录写入技能"), "{err_stage}");
        let err_discard = discard_skill_zip_upload_for_paths(&paths, "stale-upload").unwrap_err();
        assert!(
            err_discard.contains("拒绝通过链接目录写入技能"),
            "{err_discard}"
        );
        let err_import = import_local_skill_for_paths(
            &paths,
            ImportLocalSkillRequest {
                upload_id: Some("stale-upload".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            err_import.contains("拒绝通过链接目录写入技能"),
            "{err_import}"
        );

        assert_eq!(fs::read(external.join("stale-upload.zip")).unwrap(), b"keep-zip");
        assert_eq!(fs::read(external.join("notes.txt")).unwrap(), b"keep-notes");
        assert_eq!(
            fs::read(external.join("nested/doc.md")).unwrap(),
            b"keep-nested"
        );

        remove_path(&uploads_link).unwrap();
        write_atomic(&uploads_link.join("expired.zip"), b"expired", false).unwrap();
        write_atomic(&uploads_link.join("keep.txt"), b"keep", false).unwrap();
        write_atomic(&uploads_link.join("dir.zip/inner.txt"), b"keep-dir", false).unwrap();

        cleanup_temp_artifacts_at(&paths, future_now);
        assert!(!uploads_link.join("expired.zip").exists());
        assert_eq!(fs::read(uploads_link.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(
            fs::read(uploads_link.join("dir.zip/inner.txt")).unwrap(),
            b"keep-dir"
        );
    }

    #[test]
    fn upload_cleanup_and_stale_sweep_survive_post_validation_directory_swap() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());
        let uploads = temp_uploads_dir(&paths);
        let external = tmp.path().join("external-swap-target");

        // 1. StagedUploadCleanup Drop after .tmp-uploads is displaced and replaced by a directory link
        let upload_id = stage_skill_zip_chunk_for_paths(&paths, None, "UEsD").unwrap();
        let zip_name = format!("{upload_id}.zip");
        write_atomic(&external.join(&zip_name), b"external-must-survive", false).unwrap();

        let cleanup = StagedUploadCleanup::open(&paths, &upload_id)
            .unwrap()
            .unwrap();
        let displaced = tmp.path().join("displaced-uploads-1");
        fs::rename(&uploads, &displaced).unwrap();
        create_test_dir_link(&external, &uploads);
        assert!(is_link(&fs::symlink_metadata(&uploads).unwrap()));

        drop(cleanup);

        assert_eq!(
            fs::read(external.join(&zip_name)).unwrap(),
            b"external-must-survive"
        );
        assert!(
            !displaced.join(&zip_name).exists(),
            "staged file in original verified directory handle should be unlinked"
        );
        assert!(is_link(&fs::symlink_metadata(&uploads).unwrap()));

        // 2. Stale sweep after VerifiedDir open and before unlink
        remove_path(&uploads).unwrap();
        write_atomic(&uploads.join("expired.zip"), b"stale-internal", false).unwrap();
        write_atomic(
            &external.join("expired.zip"),
            b"external-expired-must-survive",
            false,
        )
        .unwrap();

        let verified_uploads = VerifiedDir::open_existing(&paths, &uploads)
            .unwrap()
            .unwrap();
        let displaced_stale = tmp.path().join("displaced-uploads-2");
        fs::rename(&uploads, &displaced_stale).unwrap();
        create_test_dir_link(&external, &uploads);

        let future_now = SystemTime::now() + STALE_TEMP_TTL + Duration::from_secs(60);
        sweep_stale_uploads_in_verified_dir(
            &verified_uploads,
            vec!["expired.zip".to_string()],
            future_now,
        );
        drop(verified_uploads);

        assert_eq!(
            fs::read(external.join("expired.zip")).unwrap(),
            b"external-expired-must-survive"
        );
        assert!(!displaced_stale.join("expired.zip").exists());

        // 3. TempExtractDir Drop after extensions_dir is displaced and replaced by a directory link
        remove_path(&uploads).unwrap();
        let temp_extract = TempExtractDir::new(&paths).unwrap();
        let archive_dir_name = temp_extract.dir_name.clone();
        write_atomic(&temp_extract.path.join("SKILL.md"), b"internal-temp", false).unwrap();
        write_atomic(
            &temp_extract.path.join("nested/only_in_internal.txt"),
            b"internal-nested",
            false,
        )
        .unwrap();

        let external_ext = tmp.path().join("external-extensions");
        write_atomic(
            &external_ext.join(&archive_dir_name).join("SKILL.md"),
            b"external-archive-must-survive",
            false,
        )
        .unwrap();

        let ext_dir = extensions_dir(&paths);
        let displaced_ext = tmp.path().join("displaced-extensions");
        fs::rename(&ext_dir, &displaced_ext).unwrap();
        create_test_dir_link(&external_ext, &ext_dir);

        drop(temp_extract);

        assert_eq!(
            fs::read(external_ext.join(&archive_dir_name).join("SKILL.md")).unwrap(),
            b"external-archive-must-survive"
        );
        assert!(!displaced_ext.join(&archive_dir_name).exists());
    }
}
