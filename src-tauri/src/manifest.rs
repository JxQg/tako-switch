use crate::adapters::{get_adapter, AppId};
use crate::config::{read_switch_state, write_atomic, AppPaths};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformArtifact {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub install_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_bundle_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub silent_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_family_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppManifestEntry {
    pub display_name: String,
    pub version: String,
    #[serde(default)]
    pub platforms: HashMap<String, PlatformArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub generated_at: String,
    #[serde(default)]
    pub apps: HashMap<String, AppManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateStatus {
    pub app_id: AppId,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub remote_version: Option<String>,
    pub has_update: bool,
    pub platform_supported: bool,
    pub artifact: Option<PlatformArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCheckResult {
    pub source_url: String,
    pub manifest: Manifest,
    pub updates: Vec<AppUpdateStatus>,
}

const BUNDLED_MANIFEST_RAW: &str = include_str!("../../manifest/manifest.json");

pub fn bundled_manifest() -> Option<Manifest> {
    serde_json::from_str::<Manifest>(BUNDLED_MANIFEST_RAW).ok()
}

/// Fills in any apps or platform entries present in the binary's bundled manifest
/// that are missing from a remote/cached manifest (e.g., when a new client like
/// WorkBuddy is shipped before the remote CDN manifest is refreshed), without
/// ever downgrading or overwriting entries already provided by the remote feed.
pub fn merge_with_bundled_fallback(mut manifest: Manifest) -> Manifest {
    let Some(bundled) = bundled_manifest() else {
        return manifest;
    };
    for (app_id, bundled_entry) in bundled.apps {
        match manifest.apps.get_mut(&app_id) {
            None => {
                manifest.apps.insert(app_id, bundled_entry);
            }
            Some(existing) => {
                for (plat, art) in bundled_entry.platforms {
                    existing.platforms.entry(plat).or_insert(art);
                }
            }
        }
    }
    manifest
}

pub fn manifest_cache_path(paths: &AppPaths) -> PathBuf {
    paths.switch_dir().join("manifest-cache.json")
}

pub fn load_cached_manifest(paths: &AppPaths) -> Option<Manifest> {
    let cached = fs::read_to_string(manifest_cache_path(paths))
        .ok()
        .and_then(|raw| serde_json::from_str::<Manifest>(&raw).ok());
    match cached {
        Some(m) => Some(merge_with_bundled_fallback(m)),
        None => bundled_manifest(),
    }
}

pub fn host_platform_keys() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        &["macos-arm64", "macos-universal"]
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        &["macos-x64", "macos-universal"]
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        &["windows-x64"]
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        &["windows-arm64", "windows-x64"]
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64")
    )))]
    {
        &[]
    }
}

pub fn resolve_platform_artifact(
    manifest: &Manifest,
    app_id: AppId,
) -> Option<(String, &PlatformArtifact)> {
    let entry = manifest.apps.get(app_id.as_str())?;
    for key in host_platform_keys() {
        if let Some(art) = entry.platforms.get(*key) {
            let ver = art
                .version
                .as_deref()
                .filter(|v| !v.is_empty())
                .unwrap_or(&entry.version)
                .to_string();
            return Some((ver, art));
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerSeg {
    num: Option<u64>,
    text: String,
}

fn parse_version_segments(raw: &str) -> Vec<VerSeg> {
    let cleaned = raw.trim().trim_start_matches(['v', 'V']);
    if cleaned.is_empty() {
        return Vec::new();
    }
    cleaned
        .split(['.', '-', '+'])
        .filter(|p| !p.is_empty())
        .map(|part| {
            let digit_prefix: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            let num = (!digit_prefix.is_empty())
                .then(|| digit_prefix.parse::<u64>().ok())
                .flatten();
            let rest = &part[digit_prefix.len()..];
            VerSeg {
                num,
                text: rest.to_ascii_lowercase(),
            }
        })
        .collect()
}

pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let sa = parse_version_segments(a);
    let sb = parse_version_segments(b);
    let max_len = sa.len().max(sb.len());
    for idx in 0..max_len {
        let pa = sa.get(idx);
        let pb = sb.get(idx);
        let num_a = pa.and_then(|s| s.num).unwrap_or(0);
        let num_b = pb.and_then(|s| s.num).unwrap_or(0);
        match num_a.cmp(&num_b) {
            Ordering::Equal => {
                let ta = pa.map(|s| s.text.as_str()).unwrap_or("");
                let tb = pb.map(|s| s.text.as_str()).unwrap_or("");
                if ta != tb {
                    // Release beats prerelease suffix when same numeric segment
                    return match (ta.is_empty(), tb.is_empty()) {
                        (true, false) => Ordering::Greater,
                        (false, true) => Ordering::Less,
                        _ => ta.cmp(tb),
                    };
                }
            }
            other => return other,
        }
    }
    Ordering::Equal
}

pub fn compute_updates(paths: &AppPaths, manifest: &Manifest) -> Vec<AppUpdateStatus> {
    AppId::all()
        .iter()
        .map(|&app_id| {
            let adapter = get_adapter(app_id);
            let install = adapter.detect_install(paths);
            let resolved = resolve_platform_artifact(manifest, app_id);
            let (remote_version, platform_supported, artifact) = match resolved {
                Some((ver, art)) => (Some(ver), true, Some(art.clone())),
                None => {
                    let top_ver = manifest
                        .apps
                        .get(app_id.as_str())
                        .map(|e| e.version.clone());
                    (top_ver, false, None)
                }
            };
            let has_update = match (install.installed, &install.version, &remote_version, platform_supported) {
                (true, Some(local), Some(remote), true) => {
                    compare_versions(remote, local) == Ordering::Greater
                }
                _ => false,
            };
            AppUpdateStatus {
                app_id,
                installed: install.installed,
                installed_version: install.version,
                remote_version,
                has_update,
                platform_supported,
                artifact,
            }
        })
        .collect()
}

pub async fn fetch_manifest(paths: &AppPaths) -> Result<(String, Manifest), String> {
    let state = read_switch_state(paths);
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))?;

    let mut urls = vec![state.manifest_url.trim().to_string()];
    let fallback = state.fallback_manifest_url.trim().to_string();
    if !fallback.is_empty() && !urls.contains(&fallback) {
        urls.push(fallback);
    }

    let mut errors = Vec::new();
    for url in urls {
        if url.is_empty() {
            continue;
        }
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<Manifest>().await {
                Ok(manifest) => {
                    let merged = merge_with_bundled_fallback(manifest);
                    if let Ok(bytes) = serde_json::to_vec_pretty(&merged) {
                        let _ = write_atomic(&manifest_cache_path(paths), &bytes, false);
                    }
                    return Ok((url, merged));
                }
                Err(err) => errors.push(format!("{url}: JSON 解析失败 ({err})")),
            },
            Ok(resp) => errors.push(format!("{url}: HTTP {}", resp.status().as_u16())),
            Err(err) => errors.push(format!("{url}: {err}")),
        }
    }

    if let Some(bundled) = bundled_manifest() {
        return Ok(("bundled".to_string(), bundled));
    }

    Err(format!("拉取软件清单失败: {}", errors.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fallback_fills_missing_workbuddy_without_overwriting_remote_versions() {
        let mut remote = Manifest {
            schema_version: 1,
            generated_at: "2026-11-25T00:00:00Z".to_string(),
            apps: HashMap::new(),
        };
        remote.apps.insert(
            "claude".to_string(),
            AppManifestEntry {
                display_name: "Claude Desktop".to_string(),
                version: "9.9.9".to_string(),
                platforms: HashMap::from([(
                    "macos-universal".to_string(),
                    PlatformArtifact {
                        url: "https://example.com/claude-9.9.9.zip".to_string(),
                        fallback_url: None,
                        version: Some("9.9.9".to_string()),
                        sha256: None,
                        sha512: None,
                        size: Some(123),
                        install_type: "zip".to_string(),
                        app_bundle_name: Some("Claude.app".to_string()),
                        silent_args: None,
                        package_family_name: None,
                    },
                )]),
            },
        );

        let merged = merge_with_bundled_fallback(remote);
        assert!(merged.apps.contains_key("workbuddy"));
        let wb_platforms = &merged.apps["workbuddy"].platforms;
        assert!(wb_platforms.contains_key("macos-arm64"));
        assert!(wb_platforms.contains_key("macos-x64"));
        assert!(wb_platforms.contains_key("windows-x64"));
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(resolve_platform_artifact(&merged, AppId::WorkBuddy).is_some());
        assert_eq!(merged.apps["claude"].version, "9.9.9");
        assert_eq!(
            merged.apps["claude"].platforms["macos-universal"].url,
            "https://example.com/claude-9.9.9.zip"
        );
    }

    #[test]
    fn compares_semver_and_calver_formats() {
        assert_eq!(compare_versions("1.20.2", "1.20.1"), Ordering::Greater);
        assert_eq!(compare_versions("v1.20.2", "1.20.2"), Ordering::Equal);
        assert_eq!(
            compare_versions("26.1112.1", "26.1111.71704"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("26.1111.71704.0", "26.1111.71704"),
            Ordering::Equal
        );
        assert_eq!(compare_versions("1.20.0", "1.20.0-beta.1"), Ordering::Greater);
        assert_eq!(compare_versions("1.3.0", "1.12.0"), Ordering::Less);
    }
}
