use crate::adapters::antigravity_i18n::{sha256_hex, CURRENT_UI_PATCH_VERSION};
use crate::adapters::AppId;
use crate::config::{write_atomic, AppPaths};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_I18N_MANIFEST_URL: &str =
    "https://dsh-tako-dl.oss-cn-shanghai.aliyuncs.com/tako-switch/i18n/manifest.json";
pub const DEFAULT_FALLBACK_I18N_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/tako-dev/tako-switch/main/manifest/i18n-manifest.json";

const MAX_REMOTE_DICT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct I18nPackManifestEntry {
    pub revision: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct I18nPackManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub apps: HashMap<String, I18nPackManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalI18nPackMeta {
    pub schema: u32,
    pub revision: String,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
}

pub fn i18n_pack_dir(paths: &AppPaths, app_id: AppId) -> PathBuf {
    paths.switch_dir().join("i18n-packs").join(app_id.as_str())
}

fn parse_semver3(v: &str) -> Option<(u64, u64, u64)> {
    let clean = v.trim().trim_start_matches(['v', 'V']);
    let core = clean.split(['-', '+']).next().unwrap_or(clean);
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn app_meets_min_version(min_app_version: Option<&str>) -> bool {
    let Some(min_ver) = min_app_version.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    match (parse_semver3(CURRENT_UI_PATCH_VERSION), parse_semver3(min_ver)) {
        (Some(cur), Some(req)) => cur >= req,
        _ => true,
    }
}

fn read_verified_local_pack(
    paths: &AppPaths,
    app_id: AppId,
) -> Option<(LocalI18nPackMeta, Vec<u8>)> {
    if !matches!(app_id, AppId::Claude | AppId::Antigravity) {
        return None;
    }
    let dir = i18n_pack_dir(paths, app_id);
    let meta_bytes = fs::read(dir.join("meta.json")).ok()?;
    let meta: LocalI18nPackMeta = serde_json::from_slice(&meta_bytes).ok()?;
    if meta.revision.trim().is_empty() || !app_meets_min_version(meta.min_app_version.as_deref()) {
        return None;
    }
    let dict_bytes = fs::read(dir.join("dict.json")).ok()?;
    if dict_bytes.len() > MAX_REMOTE_DICT_BYTES {
        return None;
    }
    if sha256_hex(&dict_bytes) != meta.sha256 {
        return None;
    }
    Some((meta, dict_bytes))
}

/// Returns the target UI patch version string for `app_id`, incorporating any verified
/// local cloud-synced i18n overlay pack revision.
pub fn effective_ui_patch_version(paths: &AppPaths, app_id: AppId) -> String {
    if let Some((meta, _)) = read_verified_local_pack(paths, app_id) {
        format!("{CURRENT_UI_PATCH_VERSION}+pack.{}", meta.revision.trim())
    } else {
        CURRENT_UI_PATCH_VERSION.to_string()
    }
}

/// Loads a verified local cloud-synced dictionary overlay for `app_id`, if present.
pub fn load_i18n_overlay_dict(paths: &AppPaths, app_id: AppId) -> Option<HashMap<String, String>> {
    let (_, dict_bytes) = read_verified_local_pack(paths, app_id)?;
    parse_dict_json_bytes(&dict_bytes)
}

pub fn parse_dict_json_bytes(bytes: &[u8]) -> Option<HashMap<String, String>> {
    let raw: HashMap<String, String> = serde_json::from_slice(bytes).ok()?;
    let mut cleaned = HashMap::with_capacity(raw.len());
    for (k, v) in raw {
        if !k.is_empty() && !v.trim().is_empty() {
            cleaned.insert(k, v);
        }
    }
    if cleaned.is_empty() {
        return None;
    }
    Some(cleaned)
}

/// Merge built-in JSON dictionary with verified cloud overlay (if any) and serialize as JSON.
pub fn merge_json_dict_with_overlay(
    builtin_raw: &str,
    overlay: Option<HashMap<String, String>>,
) -> String {
    let Some(extra) = overlay else {
        return builtin_raw.to_string();
    };
    let mut merged: BTreeMap<String, String> =
        serde_json::from_str(builtin_raw).unwrap_or_default();
    for (k, v) in extra {
        merged.insert(k, v);
    }
    serde_json::to_string(&merged).unwrap_or_else(|_| builtin_raw.to_string())
}

/// Persist a downloaded dictionary pack atomically after validating JSON and optional SHA-256.
pub fn install_i18n_pack_bytes(
    paths: &AppPaths,
    app_id: AppId,
    revision: &str,
    dict_bytes: &[u8],
    expected_sha256: Option<&str>,
    min_app_version: Option<&str>,
) -> Result<bool, String> {
    let rev = revision.trim();
    if rev.is_empty() {
        return Err("词典版本号为空".to_string());
    }
    if dict_bytes.len() > MAX_REMOTE_DICT_BYTES {
        return Err("远端词典包超过大小上限".to_string());
    }
    if !app_meets_min_version(min_app_version) {
        return Ok(false);
    }
    let actual_sha = sha256_hex(dict_bytes);
    if let Some(exp) = expected_sha256.map(str::trim).filter(|s| !s.is_empty()) {
        if !actual_sha.eq_ignore_ascii_case(exp) {
            return Err(format!("词典包 SHA-256 校验失败: expected {exp}, got {actual_sha}"));
        }
    }
    if parse_dict_json_bytes(dict_bytes).is_none() {
        return Err("词典包不是有效的键值对 JSON 词典".to_string());
    }

    if let Some((existing_meta, _)) = read_verified_local_pack(paths, app_id) {
        if existing_meta.revision == rev && existing_meta.sha256 == actual_sha {
            return Ok(false);
        }
    }

    let dir = i18n_pack_dir(paths, app_id);
    let meta = LocalI18nPackMeta {
        schema: 1,
        revision: rev.to_string(),
        sha256: actual_sha,
        min_app_version: min_app_version
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };
    let meta_bytes =
        serde_json::to_vec_pretty(&meta).map_err(|e| format!("序列化词典元信息失败: {e}"))?;
    write_atomic(&dir.join("dict.json"), dict_bytes, false)?;
    write_atomic(&dir.join("meta.json"), &meta_bytes, false)?;
    Ok(true)
}

async fn fetch_bytes_with_fallback(client: &Client, primary: &str, fallback: Option<&str>) -> Option<Vec<u8>> {
    let mut candidates = vec![primary];
    if let Some(fb) = fallback.filter(|s| !s.trim().is_empty() && *s != primary) {
        candidates.push(fb);
    }
    for url in candidates {
        let Ok(resp) = client.get(url).send().await else {
            continue;
        };
        if !resp.status().is_success() {
            continue;
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_REMOTE_DICT_BYTES {
                continue;
            }
        }
        if let Ok(bytes) = resp.bytes().await {
            if !bytes.is_empty() && bytes.len() <= MAX_REMOTE_DICT_BYTES {
                return Some(bytes.to_vec());
            }
        }
    }
    None
}

/// Silently checks remote OSS/GitHub i18n manifest and downloads any newer dictionary packs.
/// Returns the list of apps whose local overlay dictionary was updated.
/// Never returns an error to callers so offline/404 setups seamlessly fall back to built-in dicts.
pub async fn refresh_remote_i18n_packs(paths: &AppPaths) -> Vec<AppId> {
    if paths.is_test {
        return Vec::new();
    }
    let Ok(client) = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .user_agent(format!("TakoSwitch/{}", CURRENT_UI_PATCH_VERSION))
        .build()
    else {
        return Vec::new();
    };

    let Some(manifest_bytes) = fetch_bytes_with_fallback(
        &client,
        DEFAULT_I18N_MANIFEST_URL,
        Some(DEFAULT_FALLBACK_I18N_MANIFEST_URL),
    )
    .await
    else {
        return Vec::new();
    };

    let Ok(manifest) = serde_json::from_slice::<I18nPackManifest>(&manifest_bytes) else {
        return Vec::new();
    };

    let mut updated = Vec::new();
    for &app_id in &[AppId::Claude, AppId::Antigravity] {
        let Some(entry) = manifest.apps.get(app_id.as_str()) else {
            continue;
        };
        let rev = entry.revision.trim();
        if rev.is_empty() || !app_meets_min_version(entry.min_app_version.as_deref()) {
            continue;
        }
        if let Some((local_meta, _)) = read_verified_local_pack(paths, app_id) {
            if local_meta.revision == rev {
                if let Some(exp_sha) = entry.sha256.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    if local_meta.sha256.eq_ignore_ascii_case(exp_sha) {
                        continue;
                    }
                } else {
                    continue;
                }
            }
        }

        let Some(dict_bytes) =
            fetch_bytes_with_fallback(&client, &entry.url, entry.fallback_url.as_deref()).await
        else {
            continue;
        };

        if let Ok(true) = install_i18n_pack_bytes(
            paths,
            app_id,
            rev,
            &dict_bytes,
            entry.sha256.as_deref(),
            entry.min_app_version.as_deref(),
        ) {
            updated.push(app_id);
        }
    }

    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_and_load_verified_overlay_updates_effective_patch_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = AppPaths::for_test(tmp.path());

        assert_eq!(
            effective_ui_patch_version(&paths, AppId::Claude),
            CURRENT_UI_PATCH_VERSION
        );
        assert!(load_i18n_overlay_dict(&paths, AppId::Claude).is_none());

        let dict_bytes = r#"{"Hello World":"你好世界","Scheduled tasks":"定时任务"}"#.as_bytes();
        let sha = sha256_hex(dict_bytes);

        // Wrong checksum must be rejected
        assert!(install_i18n_pack_bytes(
            &paths,
            AppId::Claude,
            "20261123.1",
            dict_bytes,
            Some("0000deadbeef"),
            None,
        )
        .is_err());
        assert!(load_i18n_overlay_dict(&paths, AppId::Claude).is_none());

        // Valid checksum installs cleanly and changes effective patch version
        assert!(install_i18n_pack_bytes(
            &paths,
            AppId::Claude,
            "20261123.1",
            dict_bytes,
            Some(&sha),
            None,
        )
        .unwrap());
        assert_eq!(
            effective_ui_patch_version(&paths, AppId::Claude),
            format!("{CURRENT_UI_PATCH_VERSION}+pack.20261123.1")
        );
        let loaded = load_i18n_overlay_dict(&paths, AppId::Claude).unwrap();
        assert_eq!(loaded.get("Hello World").map(String::as_str), Some("你好世界"));

        // Re-installing identical revision+sha is a no-op (returns false)
        assert!(!install_i18n_pack_bytes(
            &paths,
            AppId::Claude,
            "20261123.1",
            dict_bytes,
            Some(&sha),
            None,
        )
        .unwrap());

        // Corrupting dict.json causes automatic fallback to built-in version
        fs::write(i18n_pack_dir(&paths, AppId::Claude).join("dict.json"), b"{}").unwrap();
        assert_eq!(
            effective_ui_patch_version(&paths, AppId::Claude),
            CURRENT_UI_PATCH_VERSION
        );
        assert!(load_i18n_overlay_dict(&paths, AppId::Claude).is_none());
    }

    #[test]
    fn merge_json_dict_overrides_and_preserves_builtin_entries() {
        let builtin = r#"{"A":"甲","B":"乙"}"#;
        let mut overlay = HashMap::new();
        overlay.insert("B".to_string(), "新乙".to_string());
        overlay.insert("C".to_string(), "丙".to_string());
        let merged_str = merge_json_dict_with_overlay(builtin, Some(overlay));
        let parsed: HashMap<String, String> = serde_json::from_str(&merged_str).unwrap();
        assert_eq!(parsed.get("A").map(String::as_str), Some("甲"));
        assert_eq!(parsed.get("B").map(String::as_str), Some("新乙"));
        assert_eq!(parsed.get("C").map(String::as_str), Some("丙"));
    }
}
