//! Read process image names from one Toolhelp snapshot without launching a shell.

use super::AppId;
use std::{collections::BTreeMap, ffi::c_void, io, mem::size_of};

const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const ERROR_NO_MORE_FILES: i32 = 18;
const MAX_PATH: usize = 260;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = -1_isize as Handle;

// Mirrors PROCESSENTRY32W, including the pointer-sized ULONG_PTR field.
#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    process_id: u32,
    default_heap_id: usize,
    module_id: u32,
    threads: u32,
    parent_process_id: u32,
    priority_base: i32,
    flags: u32,
    exe_file: [u16; MAX_PATH],
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct Snapshot(Handle);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: Snapshot owns a valid handle and closes it once.
        unsafe { CloseHandle(self.0) };
    }
}

fn find_process(mut matches: impl FnMut(&[u16]) -> bool) -> io::Result<bool> {
    // SAFETY: TH32CS_SNAPPROCESS requires no specific PID and returns an owned handle.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = Snapshot(handle);
    // SAFETY: Every field is an integer or integer array, valid when zeroed.
    let mut entry: ProcessEntry = unsafe { std::mem::zeroed() };
    entry.size = size_of::<ProcessEntry>() as u32;

    // SAFETY: The handle is live and entry is writable with the required size set.
    let mut found = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while found != 0 {
        let len = entry
            .exe_file
            .iter()
            .position(|&ch| ch == 0)
            .unwrap_or(MAX_PATH);
        if matches(&entry.exe_file[..len]) {
            return Ok(true);
        }
        // SAFETY: Same live snapshot and writable entry as above.
        found = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_NO_MORE_FILES) {
        Ok(false)
    } else {
        Err(error)
    }
}

fn exe_name(id: AppId) -> &'static str {
    match id {
        AppId::Codex => "Codex.exe",
        AppId::Claude => "Claude.exe",
        AppId::Antigravity => "Antigravity.exe",
    }
}

fn matches_ascii_name(image: &[u16], expected: &str) -> bool {
    image.len() == expected.len()
        && image
            .iter()
            .zip(expected.bytes())
            .all(|(&ch, byte)| u8::try_from(ch).is_ok_and(|ch| ch.eq_ignore_ascii_case(&byte)))
}

pub(super) fn is_running(id: AppId) -> bool {
    find_process(|image| matches_ascii_name(image, exe_name(id))).unwrap_or(false)
}

pub(crate) fn running_states() -> Result<BTreeMap<String, bool>, String> {
    let ids = AppId::all();
    let mut states: BTreeMap<_, _> = ids
        .iter()
        .map(|id| (id.as_str().to_string(), false))
        .collect();
    let mut remaining = ids.len();
    find_process(|image| {
        for &id in ids {
            if matches_ascii_name(image, exe_name(id)) {
                if let Some(running) = states.get_mut(id.as_str()) {
                    if !*running {
                        *running = true;
                        remaining -= 1;
                    }
                }
                break;
            }
        }
        remaining == 0
    })
    .map_err(|error| format!("枚举 Windows 进程失败: {error}"))?;
    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn matches_exact_names_case_insensitively() {
        for (id, name) in [
            (AppId::Codex, "CODEX.EXE"),
            (AppId::Claude, "claude.exe"),
            (AppId::Antigravity, "AntiGravity.Exe"),
        ] {
            assert!(matches_ascii_name(
                &name.encode_utf16().collect::<Vec<_>>(),
                exe_name(id)
            ));
        }
        for name in ["Codex", "CodexHelper.exe", "Codex.exe.old", "Çodex.exe"] {
            assert!(!matches_ascii_name(
                &name.encode_utf16().collect::<Vec<_>>(),
                exe_name(AppId::Codex)
            ));
        }
    }

    #[test]
    fn native_snapshot_contains_this_process() {
        let exe = std::env::current_exe().unwrap();
        let name: Vec<u16> = exe.file_name().unwrap().encode_wide().collect();
        assert!(find_process(|image| image == name.as_slice()).unwrap());
    }

    #[test]
    fn batch_returns_all_client_keys() {
        let keys: Vec<_> = running_states().unwrap().into_keys().collect();
        assert_eq!(keys, ["antigravity", "claude", "codex"]);
    }
}
