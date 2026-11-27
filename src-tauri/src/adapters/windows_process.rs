//! Read process image names from one Toolhelp snapshot without launching a shell.

use super::AppId;
use std::{collections::BTreeMap, ffi::c_void, io, mem::size_of};

const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
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
    fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
    fn QueryFullProcessImageNameW(
        process: Handle,
        flags: u32,
        exe_name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct Snapshot(Handle);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: Snapshot owns a valid handle and closes it once.
        unsafe { CloseHandle(self.0) };
    }
}

fn find_process(mut matches: impl FnMut(&[u16], u32) -> bool) -> io::Result<bool> {
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
        if matches(&entry.exe_file[..len], entry.process_id) {
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
        AppId::DshTako => "DSH Tako.exe",
        AppId::Codex => "Codex.exe",
        AppId::Claude => "Claude.exe",
        AppId::Antigravity => "Antigravity.exe",
        AppId::WorkBuddy => "WorkBuddy.exe",
    }
}

fn matches_ascii_name(image: &[u16], expected: &str) -> bool {
    image.len() == expected.len()
        && image
            .iter()
            .zip(expected.bytes())
            .all(|(&ch, byte)| u8::try_from(ch).is_ok_and(|ch| ch.eq_ignore_ascii_case(&byte)))
}

fn is_codex_package_image_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("openai.codex")
}

fn query_process_image_path(process_id: u32) -> Option<String> {
    // SAFETY: Querying process image path with limited access does not mutate target process state.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let proc_handle = Snapshot(handle);
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` is writable for `len` u16 elements and `proc_handle` is a valid process handle.
    let ok = unsafe { QueryFullProcessImageNameW(proc_handle.0, 0, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return None;
    }
    let used = (len as usize).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..used]))
}

fn is_codex_packaged_chatgpt_process(process_id: u32) -> bool {
    query_process_image_path(process_id)
        .as_deref()
        .is_some_and(is_codex_package_image_path)
}

fn matches_app_process(id: AppId, image: &[u16], process_id: u32) -> bool {
    if matches_ascii_name(image, exe_name(id)) {
        return true;
    }
    // Newer OpenAI.Codex MSIX packages ship `app/ChatGPT.exe` as the main UI executable;
    // verify the full image path so standalone ChatGPT Desktop is never misclassified.
    id == AppId::Codex
        && matches_ascii_name(image, "ChatGPT.exe")
        && is_codex_packaged_chatgpt_process(process_id)
}

pub(super) fn is_running(id: AppId) -> bool {
    find_process(|image, pid| matches_app_process(id, image, pid)).unwrap_or(false)
}

pub(super) fn running_exe_path(id: AppId) -> Option<std::path::PathBuf> {
    let mut resolved = None;
    let _ = find_process(|image, pid| {
        if !matches_app_process(id, image, pid) {
            return false;
        }
        if let Some(raw) = query_process_image_path(pid) {
            let candidate = std::path::PathBuf::from(raw);
            // A PID from the snapshot may have been reused before OpenProcess.
            let matches_snapshot = candidate
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| matches_ascii_name(image, name));
            if matches_snapshot && candidate.is_file() {
                resolved = Some(candidate);
                return true;
            }
        }
        false
    });
    resolved
}

pub(crate) fn running_states() -> Result<BTreeMap<String, bool>, String> {
    let ids = AppId::all();
    let mut states: BTreeMap<_, _> = ids
        .iter()
        .map(|id| (id.as_str().to_string(), false))
        .collect();
    let mut remaining = ids.len();
    find_process(|image, pid| {
        for &id in ids {
            if matches_app_process(id, image, pid) {
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
            (AppId::DshTako, "dsh tako.exe"),
            (AppId::Codex, "CODEX.EXE"),
            (AppId::Claude, "claude.exe"),
            (AppId::Antigravity, "AntiGravity.Exe"),
            (AppId::WorkBuddy, "workbuddy.exe"),
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
    fn codex_packaged_chatgpt_path_filter_excludes_standalone_chatgpt() {
        assert!(is_codex_package_image_path(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1120.6357.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        ));
        assert!(!is_codex_package_image_path(
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2026.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        ));
    }

    #[test]
    fn native_snapshot_contains_this_process() {
        let exe = std::env::current_exe().unwrap();
        let name: Vec<u16> = exe.file_name().unwrap().encode_wide().collect();
        assert!(find_process(|image, _pid| image == name.as_slice()).unwrap());
    }

    #[test]
    fn batch_returns_all_client_keys() {
        let keys: Vec<_> = running_states().unwrap().into_keys().collect();
        assert_eq!(
            keys,
            ["antigravity", "claude", "codex", "dsh-tako", "workbuddy"]
        );
    }

    #[test]
    fn queries_current_process_full_image_path() {
        let resolved = query_process_image_path(std::process::id())
            .expect("current test binary image path must be queryable");
        assert!(std::path::Path::new(&resolved).is_file());
    }
}
