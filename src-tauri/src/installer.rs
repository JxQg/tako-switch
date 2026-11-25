use crate::adapters::{get_adapter, AppId, AppMode, AppSnapshot, snapshot_app};
use crate::config::AppPaths;
use crate::manifest::PlatformArtifact;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::fs;
use std::io::Write;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallProgressEvent {
    pub app_id: AppId,
    pub stage: String,
    pub percent: u8,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub app_id: AppId,
    pub mode_reapplied: AppMode,
    pub snapshot: AppSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_speed_kbps: Option<u64>,
}

#[derive(Debug, Clone)]
struct DownloadMetrics {
    route: String,
    bytes: u64,
    duration_ms: u64,
    speed_kbps: u64,
}

fn emit_progress(
    app: Option<&AppHandle>,
    app_id: AppId,
    stage: &str,
    percent: u8,
    message: impl Into<String>,
) {
    if let Some(handle) = app {
        let _ = handle.emit(
            "install-progress",
            InstallProgressEvent {
                app_id,
                stage: stage.to_string(),
                percent,
                message: message.into(),
            },
        );
    }
}

fn verify_digests(file_path: &Path, artifact: &PlatformArtifact) -> Result<(), String> {
    if artifact.sha256.is_none() && artifact.sha512.is_none() {
        return Ok(());
    }
    let bytes = fs::read(file_path)
        .map_err(|e| format!("读取下载包以校验哈希失败: {e}"))?;

    if let Some(expected) = artifact.sha256.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let actual = hex::encode(Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!("安装包 SHA-256 校验失败: 期望 {expected}，实际 {actual}"));
        }
    }

    if let Some(expected) = artifact.sha512.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let digest = Sha512::digest(&bytes);
        let actual_hex = hex::encode(digest);
        let actual_b64 = STANDARD.encode(digest);
        if !actual_hex.eq_ignore_ascii_case(expected) && actual_b64 != expected {
            return Err("安装包 SHA-512 校验失败".to_string());
        }
    }

    Ok(())
}

fn format_eta(secs: u64) -> String {
    if secs < 60 {
        format!("约 {secs}s")
    } else {
        format!("约 {}m{}s", secs / 60, secs % 60)
    }
}

fn route_display_label(url: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.contains("dl.lven.asia") || lower.contains("dogedns") {
        "国内加速"
    } else if lower.contains("oss-cn-") || lower.contains("aliyuncs.com") {
        "国内 OSS"
    } else if lower.contains("github.com") || lower.contains("githubusercontent.com") {
        "GitHub"
    } else {
        "官方源"
    }
}

/// Probe a single candidate URL by fetching up to 256 KiB (via `Range: bytes=0-262143`)
/// so we measure real connection + TLS + TTFB + actual download throughput instead of just HEAD.
async fn probe_download_speed(client: &Client, url: &str) -> Option<f64> {
    const PROBE_CAP_BYTES: usize = 256 * 1024;
    let started = Instant::now();
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-262143")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let mut stream = resp.bytes_stream();
    let mut received: usize = 0;
    while received < PROBE_CAP_BYTES {
        match tokio::time::timeout(Duration::from_millis(1800), stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                received = received.saturating_add(chunk.len());
            }
            Ok(None) => break,
            _ => break,
        }
    }
    if received == 0 {
        return None;
    }
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    Some(received as f64 / elapsed)
}

/// Concurrently probe multiple candidate download URLs and return them ordered fastest-first.
/// Healthy routes are ranked by measured throughput (bytes/sec); unreachable or timed-out routes
/// are placed at the end in their original order as last-resort fallbacks.
pub(crate) async fn rank_fastest_download_urls(urls: Vec<String>) -> Vec<(String, Option<f64>)> {
    if urls.len() <= 1 {
        return urls.into_iter().map(|u| (u, None)).collect();
    }

    let Ok(probe_client) = Client::builder()
        .connect_timeout(Duration::from_millis(2500))
        .timeout(Duration::from_millis(3800))
        .build()
    else {
        return urls.into_iter().map(|u| (u, None)).collect();
    };

    let mut set = tokio::task::JoinSet::new();
    for (idx, url) in urls.iter().enumerate() {
        let client = probe_client.clone();
        let url_owned = url.clone();
        set.spawn(async move {
            let speed = probe_download_speed(&client, &url_owned).await;
            (idx, url_owned, speed)
        });
    }

    let mut probed: Vec<(usize, String, Option<f64>)> = Vec::with_capacity(urls.len());
    while let Some(res) = set.join_next().await {
        if let Ok(item) = res {
            probed.push(item);
        }
    }

    probed.sort_by(|a, b| match (a.2, b.2) {
        (Some(sa), Some(sb)) => sb
            .partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });

    probed.into_iter().map(|(_, u, s)| (u, s)).collect()
}

async fn download_artifact(
    app: Option<&AppHandle>,
    app_id: AppId,
    artifact: &PlatformArtifact,
    dest_path: &Path,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> Result<DownloadMetrics, String> {
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建下载缓存目录失败: {e}"))?;
    }

    let is_cancelled = || {
        cancel_flag
            .map(|f| f.load(Ordering::Relaxed))
            .unwrap_or(false)
    };

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建下载客户端失败: {e}"))?;

    let mut raw_urls = vec![artifact.url.clone()];
    if let Some(fb) = artifact
        .fallback_url
        .as_ref()
        .filter(|u| !u.is_empty() && *u != &artifact.url)
    {
        raw_urls.push(fb.clone());
    }

    let ranked_urls = if raw_urls.len() > 1 {
        emit_progress(app, app_id, "downloading", 1, "正在自动测速，选择最快下载线路...");
        rank_fastest_download_urls(raw_urls).await
    } else {
        raw_urls.into_iter().map(|u| (u, None)).collect()
    };

    let mut last_err = String::new();
    for (route_idx, (url, probed_bps)) in ranked_urls.into_iter().enumerate() {
        if is_cancelled() {
            let _ = fs::remove_file(dest_path);
            emit_progress(app, app_id, "cancelled", 0, "已取消下载");
            return Err("已取消下载".to_string());
        }

        let route_label = route_display_label(&url);
        let connect_msg = if route_idx > 0 {
            format!("正在切换备用线路（{route_label}）...")
        } else if probed_bps.is_some() {
            format!("已自动选定最快线路（{route_label}），正在开始下载...")
        } else {
            format!("正在连接下载源（{route_label}）...")
        };
        emit_progress(app, app_id, "downloading", 1, connect_msg);
        let resp = match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                last_err = format!("{route_label} ({url}): HTTP {}", r.status().as_u16());
                continue;
            }
            Err(e) => {
                last_err = format!("{route_label} ({url}): {e}");
                continue;
            }
        };

        let total = resp.content_length().or(artifact.size);
        let mut out = fs::File::create(dest_path)
            .map_err(|e| format!("创建临时安装包 {} 失败: {e}", dest_path.display()))?;
        let mut stream = resp.bytes_stream();
        let started_at = Instant::now();
        let mut last_emit = started_at;
        let mut downloaded: u64 = 0;
        let mut last_pct: u8 = 1;
        let mut stream_err: Option<String> = None;

        loop {
            let chunk_res = match tokio::time::timeout(Duration::from_secs(20), stream.next()).await
            {
                Ok(Some(item)) => item,
                Ok(None) => break,
                Err(_) => {
                    stream_err = Some(format!("{route_label} 下载停滞超时，正在切换备用线路"));
                    break;
                }
            };
            if is_cancelled() {
                drop(out);
                let _ = fs::remove_file(dest_path);
                emit_progress(app, app_id, "cancelled", 0, "已取消下载");
                return Err("已取消下载".to_string());
            }
            match chunk_res {
                Ok(chunk) => {
                    if let Err(e) = out.write_all(&chunk) {
                        stream_err = Some(format!("写入磁盘失败: {e}"));
                        break;
                    }
                    downloaded += chunk.len() as u64;
                    let elapsed = started_at.elapsed().as_secs_f64().max(0.1);
                    let speed_mb = (downloaded as f64 / 1_048_576.0) / elapsed;
                    let pct = total
                        .filter(|&t| t > 0)
                        .map(|t| ((downloaded.saturating_mul(75)) / t).min(75) as u8)
                        .unwrap_or_else(|| {
                            // Incremental visual movement when server omits Content-Length
                            (1 + ((downloaded / (8 * 1_048_576)).min(70) as u8)).min(72)
                        });
                    let due_time = last_emit.elapsed() >= Duration::from_millis(250);
                    let due_pct = pct >= last_pct.saturating_add(2);
                    let finished = total.is_some_and(|t| t > 0 && downloaded >= t);
                    if due_time || due_pct || finished {
                        last_pct = pct;
                        last_emit = Instant::now();
                        let msg = if let Some(tot) = total.filter(|&t| t > 0) {
                            let rem_bytes = tot.saturating_sub(downloaded) as f64;
                            let bytes_per_sec = (downloaded as f64 / elapsed).max(1.0);
                            let eta_secs = (rem_bytes / bytes_per_sec).round() as u64;
                            format!(
                                "{route_label} · 已下载 {:.1} / {:.1} MB · {:.1} MB/s · {}",
                                downloaded as f64 / 1_048_576.0,
                                tot as f64 / 1_048_576.0,
                                speed_mb,
                                format_eta(eta_secs)
                            )
                        } else {
                            format!(
                                "{route_label} · 已下载 {:.1} MB · {:.1} MB/s",
                                downloaded as f64 / 1_048_576.0,
                                speed_mb
                            )
                        };
                        emit_progress(app, app_id, "downloading", pct, msg);
                    }
                }
                Err(e) => {
                    stream_err = Some(format!("下载中断: {e}"));
                    break;
                }
            }
        }
        let _ = out.flush();
        if let Some(err) = stream_err {
            last_err = format!("{url}: {err}");
            let _ = fs::remove_file(dest_path);
            continue;
        }
        let elapsed_ms = started_at.elapsed().as_millis().max(1) as u64;
        let speed_kbps = (downloaded as f64 / 1024.0 / (elapsed_ms as f64 / 1000.0)).round() as u64;
        return Ok(DownloadMetrics {
            route: route_label.to_string(),
            bytes: downloaded,
            duration_ms: elapsed_ms,
            speed_kbps,
        });
    }

    Err(format!("下载安装包失败: {last_err}"))
}

fn infer_filename(app_id: AppId, artifact: &PlatformArtifact) -> String {
    let raw_name = artifact
        .url
        .split('?')
        .next()
        .and_then(|u| u.rsplit('/').next())
        .filter(|s| !s.is_empty() && s.contains('.'))
        .map(str::to_string);
    raw_name.unwrap_or_else(|| format!("{}-installer.{}", app_id.as_str(), artifact.install_type))
}

#[cfg(target_os = "macos")]
fn default_app_bundle_name(app_id: AppId, artifact: &PlatformArtifact) -> String {
    if let Some(name) = artifact.app_bundle_name.as_deref().filter(|s| s.ends_with(".app")) {
        return name.to_string();
    }
    match app_id {
        AppId::DshTako => "DSH Tako.app".to_string(),
        AppId::Codex => "ChatGPT.app".to_string(),
        AppId::Claude => "Claude.app".to_string(),
        AppId::Antigravity => "Antigravity.app".to_string(),
    }
}

#[cfg(target_os = "macos")]
fn find_extracted_app(dir: &Path, expected_name: &str) -> Option<PathBuf> {
    for candidate in [expected_name, "Codex.app"] {
        let direct = dir.join(candidate);
        if direct.exists() {
            return Some(direct);
        }
    }
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("app") {
            return Some(p);
        }
        if p.is_dir() {
            let nested = p.join(expected_name);
            if nested.exists() {
                return Some(nested);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn replace_macos_app_bundle(src_app: &Path, target_app: &Path, work_dir: &Path) -> Result<(), String> {
    if let Some(parent) = target_app.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目标目录 {} 失败: {e}", parent.display()))?;
    }
    let backup_old = work_dir.join("previous.app");
    if backup_old.exists() {
        let _ = fs::remove_dir_all(&backup_old);
    }

    let moved_old = if target_app.exists() {
        fs::rename(target_app, &backup_old).map_err(|e| {
            format!(
                "无法备份旧应用 {} (请确认有写入权限): {e}",
                target_app.display()
            )
        })?;
        true
    } else {
        false
    };

    let stage_out = std::process::Command::new("/usr/bin/ditto")
        .arg(src_app)
        .arg(target_app)
        .output();

    match stage_out {
        Ok(out) if out.status.success() => {
            if moved_old {
                let _ = fs::remove_dir_all(&backup_old);
            }
            Ok(())
        }
        Ok(out) => {
            if moved_old && !target_app.exists() {
                let _ = fs::rename(&backup_old, target_app);
            }
            Err(format!(
                "安装 .app 失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
        Err(e) => {
            if moved_old && !target_app.exists() {
                let _ = fs::rename(&backup_old, target_app);
            }
            Err(format!("调用 ditto 失败: {e}"))
        }
    }
}

#[cfg(target_os = "macos")]
fn install_platform_package(
    paths: &AppPaths,
    app_id: AppId,
    package_path: &Path,
    artifact: &PlatformArtifact,
    work_dir: &Path,
) -> Result<(), String> {
    let adapter = get_adapter(app_id);
    let existing = adapter.detect_install(paths);
    let bundle_name = default_app_bundle_name(app_id, artifact);
    let target_app = existing.install_path.map(PathBuf::from).unwrap_or_else(|| {
        let preferred = PathBuf::from("/Applications").join(&bundle_name);
        if app_id == AppId::Codex
            && preferred.exists()
            && crate::adapters::read_macos_bundle_id(&preferred).as_deref()
                != Some("com.openai.codex")
        {
            return PathBuf::from("/Applications/Codex.app");
        }
        preferred
    });

    let extract_dir = work_dir.join("extracted");
    if extract_dir.exists() {
        let _ = fs::remove_dir_all(&extract_dir);
    }
    fs::create_dir_all(&extract_dir).map_err(|e| format!("创建解压目录失败: {e}"))?;

    match artifact.install_type.as_str() {
        "zip" => {
            let out = std::process::Command::new("/usr/bin/ditto")
                .args(["-xk"])
                .arg(package_path)
                .arg(&extract_dir)
                .output()
                .map_err(|e| format!("解压 zip 失败: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "解压 zip 失败: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            let src_app = find_extracted_app(&extract_dir, &bundle_name)
                .ok_or_else(|| format!("压缩包中未找到 {bundle_name}"))?;
            replace_macos_app_bundle(&src_app, &target_app, work_dir)
        }
        "dmg" => {
            let mount_point = work_dir.join("mnt");
            fs::create_dir_all(&mount_point).map_err(|e| format!("创建挂载目录失败: {e}"))?;
            let attach = std::process::Command::new("/usr/bin/hdiutil")
                .arg("attach")
                .arg(package_path)
                .args(["-nobrowse", "-readonly", "-mountpoint"])
                .arg(&mount_point)
                .output()
                .map_err(|e| format!("挂载 DMG 失败: {e}"))?;
            if !attach.status.success() {
                return Err(format!(
                    "挂载 DMG 失败: {}",
                    String::from_utf8_lossy(&attach.stderr).trim()
                ));
            }

            let install_res = (|| {
                let src_app = find_extracted_app(&mount_point, &bundle_name)
                    .ok_or_else(|| format!("DMG 镜像中未找到 {bundle_name}"))?;
                let staged_app = extract_dir.join(&bundle_name);
                let cp = std::process::Command::new("/usr/bin/ditto")
                    .arg(&src_app)
                    .arg(&staged_app)
                    .output()
                    .map_err(|e| format!("拷贝镜像内 .app 失败: {e}"))?;
                if !cp.status.success() {
                    return Err(format!(
                        "拷贝镜像内 .app 失败: {}",
                        String::from_utf8_lossy(&cp.stderr).trim()
                    ));
                }
                replace_macos_app_bundle(&staged_app, &target_app, work_dir)
            })();

            let _ = std::process::Command::new("/usr/bin/hdiutil")
                .args(["detach"])
                .arg(&mount_point)
                .arg("-force")
                .output();
            install_res
        }
        other => Err(format!("macOS 不支持的包格式: {other}")),
    }
}

#[cfg(target_os = "windows")]
fn install_platform_package(
    _paths: &AppPaths,
    app_id: AppId,
    package_path: &Path,
    artifact: &PlatformArtifact,
    _work_dir: &Path,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    match artifact.install_type.as_str() {
        "msix" | "appxbundle" | "msixbundle" => {
            let escaped = package_path.to_string_lossy().replace('\'', "''");
            let script = format!("Add-AppxPackage -Path '{escaped}' -ForceApplicationShutdown");
            let out = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("安装 MSIX 失败: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "安装 MSIX 失败: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(())
        }
        "exe" => {
            let default_args: Vec<String> = match app_id {
                AppId::DshTako => vec!["/S".to_string()],
                AppId::Claude => vec!["--silent".to_string()],
                AppId::Antigravity => vec![
                    "/VERYSILENT".to_string(),
                    "/NORESTART".to_string(),
                    "/CLOSEAPPLICATIONS".to_string(),
                ],
                AppId::Codex => vec!["/S".to_string()],
            };
            let args = artifact.silent_args.as_ref().unwrap_or(&default_args);
            let out = std::process::Command::new(package_path)
                .args(args)
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("运行安装程序失败: {e}"))?;
            if !out.status.success() {
                let code = out
                    .status
                    .code()
                    .map_or_else(|| "unknown".to_string(), |c| c.to_string());
                return Err(format!(
                    "静默安装失败 (exit {code}): {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(())
        }
        other => Err(format!("Windows 不支持的包格式: {other}")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn install_platform_package(
    _paths: &AppPaths,
    _app_id: AppId,
    _package_path: &Path,
    artifact: &PlatformArtifact,
    _work_dir: &Path,
) -> Result<(), String> {
    Err(format!(
        "当前操作系统暂不支持自动安装 ({})",
        artifact.install_type
    ))
}

pub async fn install_or_upgrade_app(
    app: Option<&AppHandle>,
    paths: &AppPaths,
    app_id: AppId,
    artifact: &PlatformArtifact,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<InstallOutcome, String> {
    let work_dir = paths.switch_dir().join("downloads").join(app_id.as_str());
    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }
    fs::create_dir_all(&work_dir).map_err(|e| format!("创建工作目录失败: {e}"))?;
    let pkg_path = work_dir.join(infer_filename(app_id, artifact));

    let download_metrics = match download_artifact(app, app_id, artifact, &pkg_path, cancel_flag.as_ref()).await {
        Ok(m) => m,
        Err(err) => {
            let _ = fs::remove_dir_all(&work_dir);
            if err != "已取消下载" {
                emit_progress(app, app_id, "error", 0, &err);
            }
            return Err(err);
        }
    };

    emit_progress(app, app_id, "verifying", 80, "正在校验安装包完整性");
    if let Err(err) = verify_digests(&pkg_path, artifact) {
        let _ = fs::remove_dir_all(&work_dir);
        emit_progress(app, app_id, "error", 0, &err);
        return Err(err);
    }

    let (prev_mode, was_running) = {
        let adapter = get_adapter(app_id);
        let prev_mode = adapter.read_mode(paths);
        let was_running = adapter.is_running(paths);
        if was_running {
            emit_progress(app, app_id, "closing", 85, "正在退出运行中的旧版本");
            if let Err(err) = adapter.quit_app(paths) {
                let _ = fs::remove_dir_all(&work_dir);
                emit_progress(app, app_id, "error", 0, &err);
                return Err(err);
            }
        }
        (prev_mode, was_running)
    };
    if was_running {
        for _ in 0..24 {
            if !get_adapter(app_id).is_running(paths) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    emit_progress(app, app_id, "installing", 90, "正在执行静默安装/覆盖升级");
    let install_res = install_platform_package(paths, app_id, &pkg_path, artifact, &work_dir);
    let _ = fs::remove_dir_all(&work_dir);
    if let Err(err) = install_res {
        emit_progress(app, app_id, "error", 0, &err);
        return Err(err);
    }

    let adapter = get_adapter(app_id);
    let target_mode = if prev_mode != AppMode::Official {
        prev_mode
    } else if app_id == AppId::DshTako
        && crate::config::read_main_api_key(paths).is_some()
        && !crate::adapters::dsh_tako::has_any_dsh_tako_key(paths)
    {
        AppMode::Tako
    } else {
        AppMode::Official
    };
    if target_mode != AppMode::Official {
        let msg = if app_id == AppId::DshTako {
            "正在同步 Tako 账号密钥（开箱即用）"
        } else {
            "正在无损重放 Tako 配置"
        };
        emit_progress(app, app_id, "configuring", 95, msg);
        if let Err(err) = adapter.apply_mode(paths, target_mode) {
            emit_progress(app, app_id, "error", 0, &err);
            return Err(err);
        }
    }
    drop(adapter);

    // If user enabled Chinese UI preference, automatically refresh standalone Chinese locale
    // and re-apply UI Chinese patch for the freshly upgraded Claude / Antigravity bundle before relaunch.
    let switch_state = crate::config::read_switch_state(paths);
    if switch_state.prefer_chinese_ui
        && matches!(app_id, AppId::Codex | AppId::Claude | AppId::Antigravity)
    {
        match app_id {
            AppId::Codex => {
                let _ = crate::adapters::codex::CodexAdapter::sync_standalone_chinese_locale(
                    paths, true,
                );
            }
            AppId::Claude => {
                let _ = crate::adapters::claude::ClaudeAdapter::sync_standalone_chinese_locale(
                    paths, true,
                );
            }
            AppId::Antigravity => {
                let _ =
                    crate::adapters::antigravity::AntigravityAdapter::sync_standalone_chinese_locale(
                        paths, true,
                    );
            }
            AppId::DshTako => {}
        }

        if matches!(app_id, AppId::Claude | AppId::Antigravity) {
            emit_progress(
                app,
                app_id,
                "configuring",
                98,
                "正在为新版本自动应用简体中文界面",
            );
            let paths_clone = paths.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || match app_id {
                AppId::Claude => {
                    crate::adapters::claude_i18n::patch_claude_ui_for_paths(&paths_clone)
                        .map(|_| ())
                }
                AppId::Antigravity => {
                    crate::adapters::antigravity_i18n::patch_antigravity_ui_for_paths(&paths_clone)
                        .map(|_| ())
                }
                _ => Ok(()),
            })
            .await;
        }
    }

    if was_running {
        let _ = get_adapter(app_id).launch_app(paths);
    }

    let snapshot = snapshot_app(paths, app_id);
    emit_progress(app, app_id, "done", 100, "安装升级完成");

    Ok(InstallOutcome {
        app_id,
        mode_reapplied: target_mode,
        snapshot,
        download_route: Some(download_metrics.route),
        download_bytes: Some(download_metrics.bytes),
        download_duration_ms: Some(download_metrics.duration_ms),
        download_speed_kbps: Some(download_metrics.speed_kbps),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn verifies_sha256_and_sha512_hex_and_base64() {
        let tmp = TempDir::new().unwrap();
        let pkg = tmp.path().join("pkg.bin");
        fs::write(&pkg, b"tako-switch-payload").unwrap();

        let sum256 = hex::encode(Sha256::digest(b"tako-switch-payload"));
        let sum512_b64 = STANDARD.encode(Sha512::digest(b"tako-switch-payload"));

        let good = PlatformArtifact {
            url: "https://example.com/a.zip".into(),
            fallback_url: None,
            version: Some("1.0.0".into()),
            sha256: Some(sum256),
            sha512: Some(sum512_b64),
            size: Some(19),
            install_type: "zip".into(),
            app_bundle_name: None,
            silent_args: None,
            package_family_name: None,
        };
        assert!(verify_digests(&pkg, &good).is_ok());

        let mut bad = good.clone();
        bad.sha256 = Some("0".repeat(64));
        assert!(verify_digests(&pkg, &bad).is_err());
    }

    #[tokio::test]
    async fn rank_fastest_download_urls_prefers_fast_healthy_route_over_slow_or_broken() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // Fast server: responds immediately with 64 KiB
        let fast_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fast_addr = fast_listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = fast_listener.accept().await {
                let mut req = [0u8; 512];
                let _ = sock.read(&mut req).await;
                let payload = vec![b'A'; 64 * 1024];
                let hdr = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = sock.write_all(hdr.as_bytes()).await;
                let _ = sock.write_all(&payload).await;
            }
        });

        // Slow server: sleeps 350ms before sending 8 KiB
        let slow_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let slow_addr = slow_listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = slow_listener.accept().await {
                let mut req = [0u8; 512];
                let _ = sock.read(&mut req).await;
                tokio::time::sleep(Duration::from_millis(350)).await;
                let payload = vec![b'B'; 8 * 1024];
                let hdr = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = sock.write_all(hdr.as_bytes()).await;
                let _ = sock.write_all(&payload).await;
            }
        });

        // Broken server: returns 503
        let broken_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broken_addr = broken_listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = broken_listener.accept().await {
                let mut req = [0u8; 512];
                let _ = sock.read(&mut req).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let broken_url = format!("http://{broken_addr}/pkg.zip");
        let slow_url = format!("http://{slow_addr}/pkg.zip");
        let fast_url = format!("http://{fast_addr}/pkg.zip");

        let ranked = rank_fastest_download_urls(vec![
            broken_url.clone(),
            slow_url.clone(),
            fast_url.clone(),
        ])
        .await;

        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].0, fast_url);
        assert!(ranked[0].1.is_some());
        assert_eq!(ranked[1].0, slow_url);
        assert!(ranked[1].1.is_some());
        assert_eq!(ranked[2].0, broken_url);
        assert!(ranked[2].1.is_none());
    }
}
