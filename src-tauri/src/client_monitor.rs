use crate::adapters::{get_adapter, snapshot_app, AppId, AppMode, UiPatchStatus};
use crate::config::AppPaths;
use crate::telemetry::{self, ErrorReport};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const MAX_LOG_FILES_PER_APP: usize = 6;
const MAX_DELTA_READ_BYTES: usize = 64 * 1024;
const MONITOR_COOLDOWN: Duration = Duration::from_secs(90);
const LIVENESS_POLL_INTERVAL: Duration = Duration::from_millis(1500);
const LIVENESS_POLL_ROUNDS: usize = 12; // 18 seconds
const NEVER_STARTED_THRESHOLD_MS: u64 = 12_000;
const EARLY_CRASH_THRESHOLD_MS: u64 = 15_000;
const DELAYED_LOG_SCAN_EXTRA_WAIT: Duration = Duration::from_secs(27); // ~45s after launch

static LAST_MONITOR_START: OnceLock<Mutex<HashMap<AppId, Instant>>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct LaunchBaseline {
    pub started_at: SystemTime,
    pub was_running: bool,
    pub file_offsets: HashMap<PathBuf, u64>,
    pub known_crash_files: HashSet<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogErrorFinding {
    pub error_type: &'static str,
    pub summary: String,
    pub excerpt: String,
    pub log_file: String,
    pub error_line_count: usize,
}

fn should_start_monitor(app_id: AppId) -> bool {
    let map = LAST_MONITOR_START.get_or_init(|| Mutex::new(HashMap::new()));
    let Ok(mut guard) = map.lock() else {
        return true;
    };
    let now = Instant::now();
    if let Some(prev) = guard.get(&app_id) {
        if now.duration_since(*prev) < MONITOR_COOLDOWN {
            return false;
        }
    }
    guard.insert(app_id, now);
    true
}

/// Launches a desktop client via its adapter and automatically attaches background
/// startup liveness + sanitized log/crash monitoring (honoring `telemetry_enabled`).
pub fn launch_and_monitor(paths: &AppPaths, app_id: AppId, trigger: &'static str) -> Result<(), String> {
    let baseline = capture_launch_baseline(paths, app_id);
    let result = get_adapter(app_id).launch_app(paths);
    if let Err(ref err) = result {
        report_launch_command_failure(paths, app_id, trigger, err);
        return result;
    }
    spawn_post_launch_monitor(paths.clone(), app_id, trigger, baseline);
    Ok(())
}

fn mode_str(mode: AppMode) -> &'static str {
    match mode {
        AppMode::Tako => "tako",
        AppMode::Official => "official",
    }
}

fn ui_patch_str(status: UiPatchStatus) -> &'static str {
    match status {
        UiPatchStatus::NotApplicable => "not_applicable",
        UiPatchStatus::NotInstalled => "not_installed",
        UiPatchStatus::Unpatched => "unpatched",
        UiPatchStatus::Patched => "patched",
        UiPatchStatus::OutdatedAfterUpgrade => "outdated_after_upgrade",
        UiPatchStatus::Unsupported => "unsupported",
    }
}

fn report_launch_command_failure(paths: &AppPaths, app_id: AppId, trigger: &str, err: &str) {
    let snap = snapshot_app(paths, app_id);
    let client_ver = snap.install.version.clone().unwrap_or_default();
    let mode = mode_str(snap.mode);
    let patch_st = ui_patch_str(snap.ui_patch.status);
    telemetry::track_event(
        paths,
        "client_launch",
        app_id.as_str(),
        false,
        0,
        serde_json::json!({
            "trigger": trigger,
            "client_version": client_ver,
            "mode": mode,
            "ui_patch_status": patch_st,
            "error_type": "LaunchCommandFailed",
        }),
    );
    telemetry::report_error(
        paths,
        ErrorReport {
            source: "client_launch",
            category: &format!("client_{}", app_id.as_str()),
            error_type: "LaunchCommandFailed",
            message: err,
            location: &format!("{}:{}:{}", app_id.as_str(), client_ver, trigger),
            stack: "",
            context: serde_json::json!({
                "app_id": app_id.as_str(),
                "trigger": trigger,
                "client_version": client_ver,
                "mode": mode,
                "ui_patch_status": patch_st,
            }),
        },
    );
}

fn collect_recent_files_recursive(dir: &Path, max_depth: usize, out: &mut Vec<(SystemTime, PathBuf)>) {
    if max_depth == 0 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_recent_files_recursive(&path, max_depth - 1, out);
        } else if meta.is_file() {
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("log") || e.eq_ignore_ascii_case("txt"));
            if ext_ok {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((mtime, path));
            }
        }
    }
}

fn collect_matching_log_files(paths: &AppPaths, app_id: AppId) -> Vec<PathBuf> {
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(unused_mut))]
    let mut fixed: Vec<PathBuf> = Vec::new();
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(unused_mut))]
    let mut scan_dirs: Vec<(PathBuf, usize)> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let logs_root = paths.home.join("Library").join("Logs");
        match app_id {
            AppId::Claude => {
                fixed.push(logs_root.join("Claude-3p").join("main.log"));
                fixed.push(logs_root.join("Claude-3p").join("mcp.log"));
                fixed.push(logs_root.join("Claude").join("main.log"));
                fixed.push(logs_root.join("Claude").join("mcp.log"));
            }
            AppId::Codex => {
                scan_dirs.push((logs_root.join("com.openai.codex"), 4));
                fixed.push(
                    paths
                        .codex_dir
                        .join("app-server-daemon")
                        .join("daemon.stderr.log"),
                );
            }
            AppId::Antigravity => {
                fixed.push(logs_root.join("Antigravity").join("main.log"));
                fixed.push(logs_root.join("Antigravity").join("language_server.log"));
            }
            AppId::DshTako => {
                scan_dirs.push((logs_root.join("DSH Tako"), 2));
            }
            AppId::WorkBuddy => {
                scan_dirs.push((paths.workbuddy_dir.join("logs"), 3));
                scan_dirs.push((logs_root.join("WorkBuddy"), 2));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.home.join("AppData").join("Roaming"));
        let local_appdata = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.home.join("AppData").join("Local"));
        match app_id {
            AppId::Claude => {
                fixed.push(appdata.join("Claude-3p").join("logs").join("main.log"));
                fixed.push(appdata.join("Claude-3p").join("logs").join("mcp.log"));
                fixed.push(appdata.join("Claude").join("logs").join("main.log"));
                fixed.push(appdata.join("Claude").join("logs").join("mcp.log"));
            }
            AppId::Codex => {
                scan_dirs.push((local_appdata.join("OpenAI").join("Codex").join("logs"), 4));
                scan_dirs.push((appdata.join("Codex").join("logs"), 4));
                scan_dirs.push((
                    local_appdata
                        .join("Packages")
                        .join("OpenAI.Codex_2p2nqsd0c76g0")
                        .join("LocalCache")
                        .join("Local")
                        .join("OpenAI")
                        .join("Codex")
                        .join("logs"),
                    4,
                ));
                fixed.push(
                    paths
                        .codex_dir
                        .join("app-server-daemon")
                        .join("daemon.stderr.log"),
                );
            }
            AppId::Antigravity => {
                fixed.push(appdata.join("Antigravity").join("logs").join("main.log"));
                fixed.push(
                    appdata
                        .join("Antigravity")
                        .join("logs")
                        .join("language_server.log"),
                );
                fixed.push(local_appdata.join("Antigravity").join("logs").join("main.log"));
            }
            AppId::DshTako => {
                scan_dirs.push((appdata.join("DSH Tako").join("logs"), 2));
            }
            AppId::WorkBuddy => {
                scan_dirs.push((paths.workbuddy_dir.join("logs"), 3));
                scan_dirs.push((appdata.join("WorkBuddy").join("logs"), 2));
            }
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (paths, app_id);
    }

    let mut result: Vec<PathBuf> = fixed.into_iter().filter(|p| p.is_file()).collect();
    let mut dynamic: Vec<(SystemTime, PathBuf)> = Vec::new();
    for (dir, depth) in scan_dirs {
        collect_recent_files_recursive(&dir, depth, &mut dynamic);
    }
    dynamic.sort_by_key(|item| std::cmp::Reverse(item.0));
    for (_, p) in dynamic.into_iter().take(MAX_LOG_FILES_PER_APP) {
        if !result.contains(&p) {
            result.push(p);
        }
    }
    result.truncate(MAX_LOG_FILES_PER_APP);
    result
}

#[cfg(target_os = "macos")]
fn macos_diagnostic_report_prefixes(app_id: AppId) -> &'static [&'static str] {
    match app_id {
        AppId::Claude => &["Claude-", "Claude Helper"],
        AppId::Codex => &["ChatGPT-", "Codex-"],
        AppId::Antigravity => &["Antigravity-"],
        AppId::DshTako => &["DSH Tako-"],
        AppId::WorkBuddy => &["WorkBuddy-", "WorkBuddy Helper"],
    }
}

fn collect_macos_crash_files(paths: &AppPaths, app_id: AppId) -> HashSet<PathBuf> {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut set = HashSet::new();
    #[cfg(target_os = "macos")]
    {
        let dir = paths.home.join("Library").join("Logs").join("DiagnosticReports");
        let prefixes = macos_diagnostic_report_prefixes(app_id);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if (name.ends_with(".ips") || name.ends_with(".crash"))
                        && prefixes.iter().any(|pre| name.starts_with(pre))
                    {
                        set.insert(p);
                    }
                }
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (paths, app_id);
    }
    set
}

pub fn capture_launch_baseline(paths: &AppPaths, app_id: AppId) -> LaunchBaseline {
    let was_running = get_adapter(app_id).is_running(paths);
    let mut file_offsets = HashMap::new();
    for file_path in collect_matching_log_files(paths, app_id) {
        if let Ok(meta) = fs::metadata(&file_path) {
            file_offsets.insert(file_path, meta.len());
        }
    }
    let known_crash_files = collect_macos_crash_files(paths, app_id);
    LaunchBaseline {
        started_at: SystemTime::now(),
        was_running,
        file_offsets,
        known_crash_files,
    }
}

fn read_file_delta(path: &Path, prev_offset: u64) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let len = meta.len();
    if len == 0 {
        return None;
    }
    // If file rotated/truncated since baseline, read from start.
    let start = if len < prev_offset { 0 } else { prev_offset };
    if start >= len {
        return None;
    }
    let to_read = ((len - start) as usize).min(MAX_DELTA_READ_BYTES);
    let read_from = len.saturating_sub(to_read as u64);
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(read_from)).ok()?;
    let mut buf = vec![0u8; to_read];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn is_benign_log_noise(lower: &str) -> bool {
    lower.contains("maxlistenersexceededwarning")
        || lower.contains("[antalytics]")
        || lower.contains("/api/v2/rum")
        || (lower.contains("[eventlogging]") && lower.contains("region_unavailable"))
        || (lower.contains("auto-update") && lower.contains("403"))
}

fn is_high_signal_error_line(lower: &str, is_crash_log_file: bool) -> bool {
    if is_benign_log_noise(lower) {
        return false;
    }
    if is_crash_log_file && !lower.trim().is_empty() {
        return true;
    }
    lower.contains("[error]")
        || lower.contains(" error ")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("uncaught exception")
        || lower.contains("unhandledpromiserejection")
        || lower.contains("all download attempts failed")
        || lower.contains("no path to claude code executable")
        || lower.contains("err_connection_timed_out")
        || lower.contains("err_connection_refused")
        || lower.contains("err_ssl_")
        || lower.contains("cannot find module")
        || lower.contains("invalid package")
        || lower.contains("asar integrity")
        || lower.contains("electronasarintegrity")
        || lower.contains("code signature invalid")
        || lower.contains("segmentation fault")
        || lower.contains("access violation")
        || lower.contains("invalid toml")
        || lower.contains("unrecognized configuration setting")
        || lower.contains("failed to parse config")
        || lower.contains("failed to load config")
        || lower.contains("failed to get commands")
        || lower.contains("eacces:")
        || lower.contains("eperm:")
}

fn classify_error_bucket(combined_lower: &str, is_crash_file: bool) -> &'static str {
    if combined_lower.contains("no path to claude code executable")
        || (combined_lower.contains("[ccd]")
            && (combined_lower.contains("download attempt")
                || combined_lower.contains("all download attempts failed")
                || combined_lower.contains("err_connection_timed_out")))
    {
        return "ClaudeCodeDownloadFailed";
    }
    if combined_lower.contains("asar")
        || combined_lower.contains("invalid package")
        || combined_lower.contains("code signature invalid")
        || combined_lower.contains("codesign")
    {
        return "AsarOrIntegrityError";
    }
    if combined_lower.contains("config.toml")
        || combined_lower.contains("invalid toml")
        || combined_lower.contains("unrecognized configuration setting")
        || combined_lower.contains("failed to parse config")
        || combined_lower.contains("failed to load config")
    {
        return "ConfigParseError";
    }
    if is_crash_file
        || combined_lower.contains("fatal")
        || combined_lower.contains("panic")
        || combined_lower.contains("uncaught exception")
        || combined_lower.contains("segmentation fault")
        || combined_lower.contains("access violation")
    {
        return "ClientFatalLogError";
    }
    "ClientStartupError"
}

pub fn extract_log_error_finding(file_name: &str, delta_text: &str) -> Option<LogErrorFinding> {
    let is_crash_file = file_name.to_ascii_lowercase().starts_with("crash-");
    let mut matched: Vec<String> = Vec::new();
    for raw_line in delta_text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if is_high_signal_error_line(&lower, is_crash_file) {
            let clean = telemetry::sanitize_text(trimmed, 240);
            if !clean.is_empty() && !matched.contains(&clean) {
                matched.push(clean);
            }
        }
    }
    if matched.is_empty() {
        return None;
    }
    let error_line_count = matched.len();
    let combined_lower = matched.join("\n").to_ascii_lowercase();
    let error_type = classify_error_bucket(&combined_lower, is_crash_file);
    let summary = matched[0].clone();
    let excerpt = matched
        .into_iter()
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");
    Some(LogErrorFinding {
        error_type,
        summary,
        excerpt: telemetry::sanitize_text(&excerpt, 2200),
        log_file: telemetry::sanitize_text(file_name, 80),
        error_line_count,
    })
}

fn scan_client_log_deltas(
    paths: &AppPaths,
    app_id: AppId,
    baseline: &mut LaunchBaseline,
) -> Option<LogErrorFinding> {
    let files = collect_matching_log_files(paths, app_id);
    let mut best: Option<LogErrorFinding> = None;
    for path in files {
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let new_len = meta.len();
        let prev_offset = if let Some(&off) = baseline.file_offsets.get(&path) {
            off
        } else {
            // New file created after launch? Only read if mtime >= started_at - 2s
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let cutoff = baseline
                .started_at
                .checked_sub(Duration::from_secs(2))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if mtime < cutoff {
                baseline.file_offsets.insert(path, new_len);
                continue;
            }
            0
        };
        baseline.file_offsets.insert(path.clone(), new_len);
        let Some(delta) = read_file_delta(&path, prev_offset) else {
            continue;
        };
        let fname = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("client.log");
        if let Some(finding) = extract_log_error_finding(fname, &delta) {
            let should_replace = match &best {
                None => true,
                Some(prev) => finding_priority(finding.error_type) > finding_priority(prev.error_type),
            };
            if should_replace {
                best = Some(finding);
            }
        }
    }
    best
}

fn finding_priority(error_type: &str) -> u8 {
    match error_type {
        "ClientFatalLogError" => 5,
        "AsarOrIntegrityError" => 4,
        "ClaudeCodeDownloadFailed" => 4,
        "ConfigParseError" => 3,
        _ => 1,
    }
}

pub fn parse_macos_ips_crash_summary(raw: &str) -> Option<String> {
    // Modern macOS .ips files contain a 1-line JSON header followed by a JSON body.
    let mut lines = raw.lines();
    let first = lines.next()?.trim();
    let mut parts: Vec<String> = Vec::new();
    if let Ok(hdr) = serde_json::from_str::<serde_json::Value>(first) {
        if let Some(name) = hdr.get("app_name").and_then(|v| v.as_str()) {
            parts.push(format!("app={name}"));
        }
        if let Some(ver) = hdr.get("app_version").and_then(|v| v.as_str()) {
            parts.push(format!("ver={ver}"));
        }
    }
    let rest: String = lines.take(120).collect::<Vec<_>>().join("\n");
    if let Ok(body) = serde_json::from_str::<serde_json::Value>(&rest) {
        if let Some(exc) = body.get("exception") {
            let etype = exc.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let sig = exc.get("signal").and_then(|v| v.as_str()).unwrap_or("");
            let sub = exc.get("subtype").and_then(|v| v.as_str()).unwrap_or("");
            if !etype.is_empty() || !sig.is_empty() {
                parts.push(format!("exception={etype} {sig} {sub}").trim().to_string());
            }
        }
        if let Some(term) = body.get("termination") {
            let reason = term.get("indicator").and_then(|v| v.as_str()).unwrap_or("");
            let code = term.get("code").and_then(|v| v.as_i64());
            if !reason.is_empty() || code.is_some() {
                parts.push(format!("termination={reason} code={:?}", code));
            }
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(telemetry::sanitize_text(&parts.join(" | "), 500))
}

fn collect_os_crash_evidence(paths: &AppPaths, app_id: AppId, baseline: &LaunchBaseline) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let current = collect_macos_crash_files(paths, app_id);
        for new_file in current.difference(&baseline.known_crash_files) {
            if let Ok(raw) = fs::read_to_string(new_file) {
                if let Some(summary) = parse_macos_ips_crash_summary(&raw) {
                    return Some(summary);
                }
            }
        }
        None
    }

    #[cfg(target_os = "windows")]
    {
        let _ = (paths, baseline);
        let pattern = match app_id {
            AppId::Claude => "Claude",
            AppId::Codex => "ChatGPT|Codex",
            AppId::Antigravity => "Antigravity",
            AppId::DshTako => "DSH Tako",
            AppId::WorkBuddy => "WorkBuddy",
        };
        let script = format!(
            "$since = (Get-Date).AddSeconds(-90); \
             $ev = Get-WinEvent -FilterHashtable @{{LogName='Application'; Id=1000,1001; StartTime=$since}} -MaxEvents 15 -ErrorAction SilentlyContinue | \
             Where-Object {{ $_.Message -match '{pattern}' }} | Select-Object -First 1; \
             if ($ev) {{ Write-Output $ev.Message }}"
        );
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let out = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if text.is_empty() {
            return None;
        }
        Some(telemetry::sanitize_text(&text, 900))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (paths, app_id, baseline);
        None
    }
}

pub fn spawn_post_launch_monitor(
    paths: AppPaths,
    app_id: AppId,
    trigger: &'static str,
    mut baseline: LaunchBaseline,
) {
    if !telemetry::telemetry_allowed(&paths) {
        return;
    }
    if !should_start_monitor(app_id) {
        return;
    }

    std::thread::spawn(move || {
        let start = Instant::now();
        let adapter = get_adapter(app_id);

        let mut seen_running = baseline.was_running;
        let mut first_seen_ms: Option<u64> = if baseline.was_running { Some(0) } else { None };
        let mut exited_early = false;
        let mut exited_at_ms: Option<u64> = None;

        for _ in 0..LIVENESS_POLL_ROUNDS {
            std::thread::sleep(LIVENESS_POLL_INTERVAL);
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let running = adapter.is_running(&paths);
            if running {
                if !seen_running {
                    seen_running = true;
                    first_seen_ms = Some(elapsed_ms);
                }
            } else if seen_running && !baseline.was_running && elapsed_ms <= EARLY_CRASH_THRESHOLD_MS {
                exited_early = true;
                exited_at_ms = Some(elapsed_ms);
                break;
            }
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let never_started = !seen_running && elapsed_ms >= NEVER_STARTED_THRESHOLD_MS;

        let snap = snapshot_app(&paths, app_id);
        let client_ver = snap.install.version.clone().unwrap_or_default();
        let mode = mode_str(snap.mode);
        let patch_st = ui_patch_str(snap.ui_patch.status);

        let log_finding = scan_client_log_deltas(&paths, app_id, &mut baseline);
        let os_crash = if exited_early || never_started {
            collect_os_crash_evidence(&paths, app_id, &baseline)
        } else {
            None
        };

        let crash_error_type = if exited_early {
            Some("ClientEarlyCrash")
        } else if never_started {
            Some("ClientNeverStarted")
        } else {
            None
        };

        let ok = crash_error_type.is_none() && log_finding.is_none();
        let event_error_type = crash_error_type
            .or_else(|| log_finding.as_ref().map(|f| f.error_type))
            .unwrap_or("");

        telemetry::track_event(
            &paths,
            "client_launch",
            app_id.as_str(),
            ok,
            first_seen_ms.unwrap_or(elapsed_ms),
            serde_json::json!({
                "trigger": trigger,
                "client_version": client_ver,
                "mode": mode,
                "ui_patch_status": patch_st,
                "seen_running": seen_running,
                "exited_early": exited_early,
                "exited_at_ms": exited_at_ms,
                "has_os_crash": os_crash.is_some(),
                "log_error_count": log_finding.as_ref().map(|f| f.error_line_count).unwrap_or(0),
                "error_type": event_error_type,
            }),
        );

        if let Some(etype) = crash_error_type {
            let msg = if let Some(ref finding) = log_finding {
                format!("{}: {}", etype, finding.summary)
            } else if let Some(ref crash_summary) = os_crash {
                format!("{}: {}", etype, crash_summary)
            } else if exited_early {
                format!(
                    "{} v{} 启动后 {}ms 内闪退退出",
                    app_id.display_name(),
                    client_ver,
                    exited_at_ms.unwrap_or(elapsed_ms)
                )
            } else {
                format!(
                    "{} v{} 启动命令返回成功但 {}ms 内未检测到运行进程",
                    app_id.display_name(),
                    client_ver,
                    elapsed_ms
                )
            };

            let mut stack_parts = Vec::new();
            if let Some(ref crash_summary) = os_crash {
                stack_parts.push(format!("[os_crash]\n{crash_summary}"));
            }
            if let Some(ref finding) = log_finding {
                stack_parts.push(format!("[log:{}]\n{}", finding.log_file, finding.excerpt));
            }
            let stack_combined = stack_parts.join("\n\n");

            telemetry::report_error(
                &paths,
                ErrorReport {
                    source: "client_crash",
                    category: &format!("client_{}", app_id.as_str()),
                    error_type: etype,
                    message: &msg,
                    location: &format!("{}:{}", app_id.as_str(), client_ver),
                    stack: &stack_combined,
                    context: serde_json::json!({
                        "app_id": app_id.as_str(),
                        "trigger": trigger,
                        "client_version": client_ver,
                        "mode": mode,
                        "ui_patch_status": patch_st,
                        "seen_running": seen_running,
                        "exited_early": exited_early,
                        "exited_at_ms": exited_at_ms,
                    }),
                },
            );
            // Already crashed — no need to wait for Stage 2 delayed log scan.
            return;
        }

        let mut reported_stage1_error_type: Option<&'static str> = None;

        if let Some(ref finding) = log_finding {
            telemetry::report_error(
                &paths,
                ErrorReport {
                    source: "client_log",
                    category: &format!("client_{}", app_id.as_str()),
                    error_type: finding.error_type,
                    message: &finding.summary,
                    location: &format!("{}:{}:{}", app_id.as_str(), client_ver, finding.log_file),
                    stack: &finding.excerpt,
                    context: serde_json::json!({
                        "app_id": app_id.as_str(),
                        "trigger": trigger,
                        "client_version": client_ver,
                        "mode": mode,
                        "ui_patch_status": patch_st,
                        "log_file": finding.log_file,
                        "error_line_count": finding.error_line_count,
                        "stage": "startup_18s",
                    }),
                },
            );
            reported_stage1_error_type = Some(finding.error_type);
        }

        // Stage 2: Catch slower post-launch async failures (e.g. Claude Code download
        // exhausting 3 retries ~35s after opening Claude Desktop).
        std::thread::sleep(DELAYED_LOG_SCAN_EXTRA_WAIT);
        if let Some(delayed) = scan_client_log_deltas(&paths, app_id, &mut baseline) {
            if reported_stage1_error_type != Some(delayed.error_type) {
                telemetry::report_error(
                    &paths,
                    ErrorReport {
                        source: "client_log",
                        category: &format!("client_{}", app_id.as_str()),
                        error_type: delayed.error_type,
                        message: &delayed.summary,
                        location: &format!("{}:{}:{}", app_id.as_str(), client_ver, delayed.log_file),
                        stack: &delayed.excerpt,
                        context: serde_json::json!({
                            "app_id": app_id.as_str(),
                            "trigger": trigger,
                            "client_version": client_ver,
                            "mode": mode,
                            "ui_patch_status": patch_st,
                            "log_file": delayed.log_file,
                            "error_line_count": delayed.error_line_count,
                            "stage": "startup_45s",
                        }),
                    },
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn classifies_claude_code_download_timeout_and_filters_benign_noise() {
        let log_sample = "\
2026-11-25 19:42:13 [error] [EventLogging] Batch flush FAILED: 5 events | status: 403 | response: {\"error_code\":\"region_unavailable\"}\n\
2026-11-25 19:42:14 [warn] MaxListenersExceededWarning: Possible EventEmitter memory leak\n\
2026-11-25 19:42:14 [warn] [antalytics] failed: flush Error: batch dropped (4xx)\n\
2026-11-25 19:42:15 [error] [CCD] Download attempt 3/3 failed at fetch (ERR_CONNECTION_TIMED_OUT) C:\\Users\\alice\\AppData\\Roaming\\Claude-3p\n\
2026-11-25 19:42:15 [error] [CCD] All download attempts failed\n\
2026-11-25 19:42:15 [error] Failed to get commands from temporary query: Error: No path to Claude code executable (download failed at fetch: ERR_CONNECTION_TIMED_OUT)\n";

        let finding = extract_log_error_finding("main.log", log_sample).expect("must detect CCD error");
        assert_eq!(finding.error_type, "ClaudeCodeDownloadFailed");
        assert_eq!(finding.error_line_count, 3);
        assert!(!finding.excerpt.contains("alice"));
        assert!(finding.excerpt.contains("C:\\Users\\<redacted>"));
        assert!(!finding.excerpt.contains("region_unavailable"));
        assert!(!finding.excerpt.contains("antalytics"));
    }

    #[test]
    fn classifies_config_asar_and_fatal_errors() {
        let cfg = extract_log_error_finding(
            "daemon.stderr.log",
            "2026-11-24T17:19:22Z ERROR codex_app_server: Codex is ignoring 1 unrecognized configuration setting. user (/Users/bob/.codex/config.toml): `foo` is ignored.\n",
        )
        .unwrap();
        assert_eq!(cfg.error_type, "ConfigParseError");
        assert!(cfg.summary.contains("/Users/<redacted>"));

        let asar = extract_log_error_finding(
            "main.log",
            "FATAL: ElectronAsarIntegrity mismatch for Resources/app.asar (sk-secret1234567890)\n",
        )
        .unwrap();
        assert_eq!(asar.error_type, "AsarOrIntegrityError");
        assert!(!asar.summary.contains("sk-secret1234567890"));
        assert!(asar.summary.contains("<redacted-key>"));

        let crash = extract_log_error_finding(
            "crash-2026-11-21-main.log",
            "Uncaught Exception: TypeError: cannot read properties of undefined\n",
        )
        .unwrap();
        assert_eq!(crash.error_type, "ClientFatalLogError");
    }

    #[test]
    fn read_file_delta_only_reads_new_bytes_after_baseline_and_handles_rotation() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("main.log");
        fs::write(&log_path, "[error] old error from yesterday\n").unwrap();
        let baseline_offset = fs::metadata(&log_path).unwrap().len();

        // No new bytes -> None
        assert_eq!(read_file_delta(&log_path, baseline_offset), None);

        // Appended new error -> only new bytes returned
        let mut f = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        std::io::Write::write_all(&mut f, b"[error] new crash after launch\n").unwrap();
        let delta = read_file_delta(&log_path, baseline_offset).unwrap();
        assert!(!delta.contains("yesterday"));
        assert!(delta.contains("new crash after launch"));

        // Truncated/rotated file (smaller than previous offset) -> reads from 0
        fs::write(&log_path, "[error] rotated\n").unwrap();
        let rotated = read_file_delta(&log_path, 9999).unwrap();
        assert_eq!(rotated, "[error] rotated\n");
    }

    #[test]
    fn parses_macos_ips_crash_header() {
        let sample = "{\"app_name\":\"ChatGPT\",\"app_version\":\"26.1120.50957\"}\n{\"exception\":{\"type\":\"EXC_BAD_ACCESS\",\"signal\":\"SIGSEGV\",\"subtype\":\"KERN_INVALID_ADDRESS\"},\"termination\":{\"indicator\":\"Segmentation fault: 11\",\"code\":11}}\n";
        let summary = parse_macos_ips_crash_summary(sample).unwrap();
        assert!(summary.contains("app=ChatGPT"));
        assert!(summary.contains("EXC_BAD_ACCESS"));
        assert!(summary.contains("SIGSEGV"));
    }
}
