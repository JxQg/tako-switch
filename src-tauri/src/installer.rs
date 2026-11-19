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
use std::path::{Path, PathBuf};
use std::time::Duration;
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

async fn download_artifact(
    app: Option<&AppHandle>,
    app_id: AppId,
    artifact: &PlatformArtifact,
    dest_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建下载缓存目录失败: {e}"))?;
    }

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| format!("创建下载客户端失败: {e}"))?;

    let mut urls = vec![artifact.url.clone()];
    if let Some(fb) = artifact.fallback_url.as_ref().filter(|u| !u.is_empty() && *u != &artifact.url) {
        urls.push(fb.clone());
    }

    let mut last_err = String::new();
    for url in urls {
        emit_progress(app, app_id, "downloading", 1, format!("正在下载 {url}"));
        let resp = match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                last_err = format!("{url}: HTTP {}", r.status().as_u16());
                continue;
            }
            Err(e) => {
                last_err = format!("{url}: {e}");
                continue;
            }
        };

        let total = resp.content_length().or(artifact.size);
        let mut out = fs::File::create(dest_path)
            .map_err(|e| format!("创建临时安装包 {} 失败: {e}", dest_path.display()))?;
        let mut stream = resp.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut last_pct: u8 = 1;
        let mut stream_err: Option<String> = None;

        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(chunk) => {
                    if let Err(e) = out.write_all(&chunk) {
                        stream_err = Some(format!("写入磁盘失败: {e}"));
                        break;
                    }
                    downloaded += chunk.len() as u64;
                    if let Some(tot) = total.filter(|&t| t > 0) {
                        let pct = ((downloaded.saturating_mul(75)) / tot).min(75) as u8;
                        if pct >= last_pct.saturating_add(3) || pct == 75 {
                            last_pct = pct;
                            emit_progress(
                                app,
                                app_id,
                                "downloading",
                                pct,
                                format!(
                                    "已下载 {:.1} MB / {:.1} MB",
                                    downloaded as f64 / 1_048_576.0,
                                    tot as f64 / 1_048_576.0
                                ),
                            );
                        }
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
        return Ok(());
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
                return Err(format!(
                    "静默安装失败 (exit {:?}): {}",
                    out.code_or_unknown(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
            Ok(())
        }
        other => Err(format!("Windows 不支持的包格式: {other}")),
    }
}

#[cfg(target_os = "windows")]
trait ExitCodeHelper {
    fn code_or_unknown(&self) -> String;
}

#[cfg(target_os = "windows")]
impl ExitCodeHelper for std::process::ExitStatus {
    fn code_or_unknown(&self) -> String {
        self.code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unknown".to_string())
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
) -> Result<InstallOutcome, String> {
    let work_dir = paths.switch_dir().join("downloads").join(app_id.as_str());
    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }
    fs::create_dir_all(&work_dir).map_err(|e| format!("创建工作目录失败: {e}"))?;
    let pkg_path = work_dir.join(infer_filename(app_id, artifact));

    let download_res = download_artifact(app, app_id, artifact, &pkg_path).await;
    if let Err(err) = download_res {
        let _ = fs::remove_dir_all(&work_dir);
        return Err(err);
    }

    emit_progress(app, app_id, "verifying", 80, "正在校验安装包完整性");
    if let Err(err) = verify_digests(&pkg_path, artifact) {
        let _ = fs::remove_dir_all(&work_dir);
        return Err(err);
    }

    let adapter = get_adapter(app_id);
    let prev_mode = adapter.read_mode(paths);
    let was_running = adapter.is_running(paths);
    if was_running {
        emit_progress(app, app_id, "closing", 85, "正在退出运行中的旧版本");
        adapter.quit_app(paths)?;
    }

    emit_progress(app, app_id, "installing", 90, "正在执行静默安装/覆盖升级");
    let install_res = install_platform_package(paths, app_id, &pkg_path, artifact, &work_dir);
    let _ = fs::remove_dir_all(&work_dir);
    install_res?;

    if prev_mode != AppMode::Official {
        emit_progress(app, app_id, "configuring", 96, "正在无损重放 Tako 配置");
        adapter.apply_mode(paths, prev_mode)?;
    }

    if was_running {
        let _ = adapter.launch_app(paths);
    }

    let snapshot = snapshot_app(paths, app_id);
    emit_progress(app, app_id, "done", 100, "安装升级完成");

    Ok(InstallOutcome {
        app_id,
        mode_reapplied: prev_mode,
        snapshot,
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
}
