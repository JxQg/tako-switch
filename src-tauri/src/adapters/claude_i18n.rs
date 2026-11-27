use crate::adapters::antigravity_i18n::{
    quick_check_clean_backup_asar, sha256_hex, AsarArchive, AsarHeaderIndex, UiPatchInfo,
    UiPatchStatus, PATCH_MARKER_FILE,
};
use crate::adapters::i18n_packs::{effective_ui_patch_version, load_i18n_overlay_dict};
#[cfg(target_os = "macos")]
use crate::adapters::antigravity_i18n::resign_macos_outer_bundle_for;
#[cfg(target_os = "macos")]
use crate::adapters::asar_patch::{
    compute_electron_asar_integrity_digest_from_plist_file,
    patch_electron_integrity_digest_slots_in_file, scan_active_electron_integrity_digest_slots,
};
use crate::adapters::asar_patch::{
    format_ui_patch_block_error, inspect_electron_asar_integrity_fuse, make_msix_readonly_prompt,
    patch_windows_pe_asar_integrity, probe_bundle_write_access, read_bundle_file_checked,
    update_info_plist_asar_key_hash, write_bundle_file_checked,
};
use crate::adapters::{AppId, DesktopAppAdapter};
use crate::config::{write_atomic, AppPaths};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const CLAUDE_PATCH_GUARD_COMMENT: &str = "/* __TAKO_CLAUDE_ZH_PATCH_V1__ */";
const CLAUDE_UI_POLISH_STYLE_ID: &str = "tako-claude-code-landing-polish";
const CLAUDE_UI_POLISH_MARKER: &str = "/* __TAKO_CLAUDE_UI_POLISH_V1__ */";
const CLAUDE_UI_POLISH_STYLE_TAG: &str = r#"<style id="tako-claude-code-landing-polish">/* __TAKO_CLAUDE_UI_POLISH_V1__ */
[data-testid="epitaxy-start-card"]{padding-inline:1.5rem;gap:.875rem!important}
[data-testid="epitaxy-start-card"]::before{content:"今天想一起完成什么？";display:block;width:min(100%,40rem);text-align:center;font-size:1.45rem;font-weight:500;letter-spacing:-.02em;line-height:1.35;color:var(--cds-text-primary,inherit);margin-bottom:.25rem;opacity:.94}
[data-testid="epitaxy-start-card"] .w-80,[data-testid="epitaxy-start-card"]>ul,[data-testid="epitaxy-start-card"] [data-start-card-environments],[data-testid="epitaxy-start-card"] [data-start-card-setup]{width:min(100%,40rem)!important;max-width:100%!important}
[data-testid="epitaxy-start-card"] [data-start-card-environments],[data-testid="epitaxy-start-card"] [data-start-card-setup]>ul,[data-testid="epitaxy-start-card"] [data-start-card-setup]>.rounded-xl{border-radius:1rem!important}
@media (min-height:680px){.epitaxy-chat-panel:has([data-testid="epitaxy-start-card"]) .epitaxy-prompt{margin-bottom:clamp(1.5rem,11vh,6.25rem);transition:margin-bottom 180ms ease}}
</style>"#;

const CLAUDE_ZH_DICT_RAW: &str = include_str!("claude_zh_dict.json");

const KNOWN_DYNAMIC_EN: &[(&str, &str)] = &[
    ("0gtp5o1nEE", "Complex, detailed work"),
    (
        "29OLd9bgWw",
        "Opus consumes usage limits faster than other models",
    ),
    ("2EcGZn58SU", "Auto thinking"),
    ("2qTZeZCPue", "Analysis tool"),
    ("4EGZ/Kv9qH", "Most efficient for everyday tasks"),
    ("4N8a6wReLf", "Best for math and coding challenges"),
    ("4RgMF5iUDm", "Extended"),
    ("6EhWLLZwj4", "Enable fast mode"),
    ("6eNkKSj+Er", "Web search"),
    ("8iuHf335Ey", "Always uses deep reasoning"),
    ("B4Nw7coGbc", "Adaptive"),
    ("BDqDM65MnU", "Adaptive thinking"),
    ("DvRKnnUSOG", "Thinking"),
    ("EGP6HJPOg+", "Most efficient for simpler tasks"),
    ("EpijVevLzK", "Fast mode"),
    (
        "GoDyM43G4X",
        "Claude uses its access to the web to improve answers when appropriate.",
    ),
    ("HoZQmASaw3", "Extra"),
    ("LCyVJcnXTZ", "Most capable for ambitious work"),
    ("MnpOJUobxY", "Thinks for more complex tasks"),
    (
        "N+VJI9Z0SU",
        "Upload CSVs for Claude to analyze quantitative data with high accuracy and create interactive data visualizations.",
    ),
    ("NKydG2KArY", "Extended"),
    ("PLBs+fqKwX", "Off"),
    ("QQ2UzmdmNN", "Light, casual tasks"),
    ("QimMWHgE5g", "Respond right away"),
    (
        "RaZ/cp/eey",
        "May use excessive tokens resulting in long response times and may hit token limits. Use sparingly for the hardest tasks.",
    ),
    ("U0mtsR78v2", "Medium"),
    ("XBURx2a572", "For complex work and everyday tasks"),
    ("XPiJIxyAVt", "Recommended"),
    ("Xads8RoR+/", "Default"),
    ("Xft0R8xbIB", "Low"),
    ("Yrtr74Rv+h", "For your toughest challenges"),
    ("Zk4EtKw26K", "Max"),
    ("Ztf14WwmNM", "Balanced for everyday work"),
    ("Zyu/cxL2/B", "Uses usage credits"),
    ("aWuhH8qI9/", "Think longer for complex tasks"),
    ("c42Iq1UBqb", "Extended"),
    (
        "dapWFrcWv4",
        "Higher effort means more thorough responses, but takes longer and uses your limits faster.",
    ),
    ("eAZhflTduv", "Match thinking to complexity"),
    ("g8BKYKpvDv", "Can think for more complex tasks"),
    (
        "gRa9I5BFc2",
        "Opus draws down usage faster than other models",
    ),
    (
        "hahKOhzAYF",
        "Claude can search the internet to provide more up-to-date and relevant responses. Claude will automatically determine when to use web search if the topic requires current information. Web search is only available when using Claude 3.7 Sonnet.",
    ),
    ("mQRd+O9jEI", "Org default"),
    ("n6XZvuhMVT", "Fastest for quick answers"),
    ("oAk189A0OS", "Fast"),
    ("oLD1YKsL8P", "The hardest problems. Takes longest."),
    (
        "sA9kR2u6VC",
        "Claude can write and run code to process data, run analysis, and produce data visualizations in real time.",
    ),
    ("u8fFNy1Mu4", "Instant"),
    ("u8n5HzNZ2R", "Extended thinking"),
    ("vv9L/MN3/H", "High"),
    ("w+qTzeepxX", "Best for most use cases"),
    ("zdSx1K4VL2", "Quick replies to simple questions"),
];

const STANDARD_ION_LOCALES: &[&str] = &[
    "en-US", "de-DE", "fr-FR", "ko-KR", "ja-JP", "es-419", "es-ES", "it-IT", "hi-IN", "pt-BR",
    "id-ID",
];

fn claude_zh_dict() -> &'static HashMap<String, String> {
    static DICT: OnceLock<HashMap<String, String>> = OnceLock::new();
    DICT.get_or_init(|| serde_json::from_str(CLAUDE_ZH_DICT_RAW).unwrap_or_default())
}

fn load_effective_claude_zh_dict(paths: &AppPaths) -> Cow<'static, HashMap<String, String>> {
    let base = claude_zh_dict();
    if let Some(overlay) = load_i18n_overlay_dict(paths, AppId::Claude) {
        let mut merged = base.clone();
        merged.extend(overlay);
        Cow::Owned(merged)
    } else {
        Cow::Borrowed(base)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudePatchMarker {
    schema: u32,
    patch_version: String,
    original_asar_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudePatchMeta {
    schema: u32,
    patch_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    app_version: Option<String>,
    original_app_asar_sha256: String,
    patched_app_asar_sha256: String,
    original_ion_asar_sha256: String,
    patched_ion_asar_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original_win_exe_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    patched_win_exe_sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedClaudeTarget {
    app_asar_path: PathBuf,
    ion_asar_path: PathBuf,
    bundle_root: Option<PathBuf>,
    info_plist_path: Option<PathBuf>,
    macos_bin_path: Option<PathBuf>,
    code_resources_path: Option<PathBuf>,
    win_exe_path: Option<PathBuf>,
    app_version_hint: Option<String>,
}

enum ClaudeTargetResolution {
    NotInstalled,
    MsixReadOnly { root: PathBuf },
    MissingAsar { root: PathBuf },
    Found(Box<ResolvedClaudeTarget>),
}

fn ui_patch_backup_dir(paths: &AppPaths) -> PathBuf {
    paths.backups_dir("claude").join("ui_patch")
}

fn read_patch_meta(paths: &AppPaths) -> Option<ClaudePatchMeta> {
    let raw = fs::read(ui_patch_backup_dir(paths).join("patch-meta.json")).ok()?;
    serde_json::from_slice::<ClaudePatchMeta>(&raw).ok()
}

fn is_windowsapps_path(path: &Path) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .contains("windowsapps")
}

fn find_windows_claude_exe(dir: &Path) -> Option<PathBuf> {
    for name in ["Claude.exe", "claude.exe"] {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn parse_numeric_version_components(ver: &str) -> Vec<u64> {
    ver.split(['.', '-', '_'])
        .filter_map(|seg| {
            let digits: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                None
            } else {
                digits.parse::<u64>().ok()
            }
        })
        .collect()
}

fn resolve_target_from_dir(root: &Path, version_hint: Option<String>) -> ClaudeTargetResolution {
    if is_windowsapps_path(root) {
        return ClaudeTargetResolution::MsixReadOnly {
            root: root.to_path_buf(),
        };
    }

    if !root.exists() {
        return ClaudeTargetResolution::NotInstalled;
    }

    let base_dir = if root.is_file() {
        root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf())
    } else {
        root.to_path_buf()
    };

    // 1. macOS .app bundle
    let is_macos_bundle = base_dir
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        || base_dir.join("Contents").join("Resources").join("app.asar").is_file()
        || base_dir.join("Contents").is_dir();

    if is_macos_bundle {
        let res_dir = base_dir.join("Contents").join("Resources");
        let app_asar = res_dir.join("app.asar");
        let ion_asar = res_dir.join("ion-dist.asar");
        if !app_asar.is_file() || !ion_asar.is_file() {
            return ClaudeTargetResolution::MissingAsar { root: base_dir };
        }
        return ClaudeTargetResolution::Found(Box::new(ResolvedClaudeTarget {
            app_asar_path: app_asar,
            ion_asar_path: ion_asar,
            bundle_root: Some(base_dir.clone()),
            info_plist_path: Some(base_dir.join("Contents").join("Info.plist")),
            macos_bin_path: Some(base_dir.join("Contents").join("MacOS").join("Claude")),
            code_resources_path: Some(
                base_dir
                    .join("Contents")
                    .join("_CodeSignature")
                    .join("CodeResources"),
            ),
            win_exe_path: None,
            app_version_hint: version_hint,
        }));
    }

    // 2. Windows Squirrel app-* layout (numeric semver descending)
    if let Ok(entries) = fs::read_dir(&base_dir) {
        let mut squirrel_dirs: Vec<(Vec<u64>, String, PathBuf, String)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            if let Some(ver_suffix) = lower.strip_prefix("app-") {
                let raw_suffix = name[name.len() - ver_suffix.len()..].to_string();
                let res = path.join("resources");
                if res.join("app.asar").is_file() && res.join("ion-dist.asar").is_file() {
                    let nums = parse_numeric_version_components(&raw_suffix);
                    squirrel_dirs.push((nums, name, path, raw_suffix));
                }
            }
        }
        squirrel_dirs.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        if let Some((_, _, app_dir, ver_suffix)) = squirrel_dirs.into_iter().next() {
            let res = app_dir.join("resources");
            let win_exe_path = find_windows_claude_exe(&app_dir);
            return ClaudeTargetResolution::Found(Box::new(ResolvedClaudeTarget {
                app_asar_path: res.join("app.asar"),
                ion_asar_path: res.join("ion-dist.asar"),
                bundle_root: None,
                info_plist_path: None,
                macos_bin_path: None,
                code_resources_path: None,
                win_exe_path,
                app_version_hint: version_hint.or(Some(ver_suffix)),
            }));
        }
    }

    // 3. Direct resources/ layout (<dir>/resources/{app.asar, ion-dist.asar})
    let res_dir = base_dir.join("resources");
    if res_dir.join("app.asar").is_file() && res_dir.join("ion-dist.asar").is_file() {
        return ClaudeTargetResolution::Found(Box::new(ResolvedClaudeTarget {
            app_asar_path: res_dir.join("app.asar"),
            ion_asar_path: res_dir.join("ion-dist.asar"),
            bundle_root: None,
            info_plist_path: None,
            macos_bin_path: None,
            code_resources_path: None,
            win_exe_path: find_windows_claude_exe(&base_dir),
            app_version_hint: version_hint,
        }));
    }

    // 4. Flat layout (<dir>/{app.asar, ion-dist.asar})
    if base_dir.join("app.asar").is_file() && base_dir.join("ion-dist.asar").is_file() {
        let win_exe_path = find_windows_claude_exe(&base_dir).or_else(|| {
            base_dir
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| n.eq_ignore_ascii_case("resources"))
                .and_then(|_| base_dir.parent())
                .and_then(find_windows_claude_exe)
        });
        return ClaudeTargetResolution::Found(Box::new(ResolvedClaudeTarget {
            app_asar_path: base_dir.join("app.asar"),
            ion_asar_path: base_dir.join("ion-dist.asar"),
            bundle_root: None,
            info_plist_path: None,
            macos_bin_path: None,
            code_resources_path: None,
            win_exe_path,
            app_version_hint: version_hint,
        }));
    }

    ClaudeTargetResolution::MissingAsar { root: base_dir }
}

fn resolve_claude_target(paths: &AppPaths) -> ClaudeTargetResolution {
    if let Some(ref override_dir) = paths.claude_app_override_dir {
        return resolve_target_from_dir(override_dir, None);
    }

    // Never touch host /Applications/Claude.app in test sandboxes without explicit override.
    if paths.is_test {
        return ClaudeTargetResolution::NotInstalled;
    }

    let install = super::claude::ClaudeAdapter.detect_install(paths);
    if !install.installed {
        return ClaudeTargetResolution::NotInstalled;
    }

    let Some(raw_path) = install.install_path else {
        return ClaudeTargetResolution::NotInstalled;
    };
    let path_buf = PathBuf::from(&raw_path);

    if install
        .install_kind
        .as_deref()
        .is_some_and(|k| k.eq_ignore_ascii_case("msix"))
        || is_windowsapps_path(&path_buf)
    {
        return ClaudeTargetResolution::MsixReadOnly { root: path_buf };
    }

    resolve_target_from_dir(&path_buf, install.version)
}

#[derive(Debug, PartialEq, Eq)]
struct IcuProfile {
    vars: BTreeSet<String>,
    complex_kinds: BTreeMap<String, String>,
    tags: BTreeSet<String>,
}

fn is_valid_icu_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.contains('{')
        && !s.contains('}')
        && !s.chars().any(|c| c.is_whitespace())
}

fn extract_icu_vars_and_kinds(
    input: &str,
    vars: &mut BTreeSet<String>,
    complex_kinds: &mut BTreeMap<String, String>,
    record_complex: bool,
) -> bool {
    let chars: Vec<(usize, char)> = input.char_indices().collect();
    let mut pos = 0usize;
    let mut depth = 0usize;
    let mut block_start = 0usize;

    while pos < chars.len() {
        let (idx, ch) = chars[pos];
        // Respect ICU MessageFormat single-quote escaping (`'{'`, `'}'`, `'<'`, `''`)
        // at top level so literal braces like `should start with '{'` validate cleanly.
        if ch == '\'' && depth == 0 {
            if let Some(&(_, next_ch)) = chars.get(pos + 1) {
                if next_ch == '\'' {
                    pos += 2;
                    continue;
                }
                if matches!(next_ch, '{' | '}' | '<') {
                    pos += 2;
                    while pos < chars.len() {
                        if chars[pos].1 == '\'' {
                            if chars.get(pos + 1).is_some_and(|&(_, c)| c == '\'') {
                                pos += 2;
                                continue;
                            }
                            pos += 1;
                            break;
                        }
                        pos += 1;
                    }
                    continue;
                }
            }
        }
        if ch == '{' {
            if depth == 0 {
                block_start = idx + ch.len_utf8();
            }
            depth += 1;
        } else if ch == '}' {
            if depth == 0 {
                return false;
            }
            depth -= 1;
            if depth == 0 {
                let inner = &input[block_start..idx];
                let mut parts = inner.splitn(3, ',');
                let var_name = parts.next().unwrap_or("").trim();
                if !is_valid_icu_ident(var_name) {
                    return false;
                }
                vars.insert(var_name.to_string());

                if let Some(fmt_raw) = parts.next() {
                    let fmt_kind = fmt_raw.trim().to_ascii_lowercase();
                    if matches!(fmt_kind.as_str(), "plural" | "select" | "selectordinal") {
                        if record_complex {
                            complex_kinds.insert(var_name.to_string(), fmt_kind.clone());
                        }
                        let Some(arms_str) = parts.next() else {
                            return false;
                        };
                        if !parse_icu_arms(arms_str, vars, complex_kinds) {
                            return false;
                        }
                    } else if !fmt_kind.is_empty()
                        && record_complex
                        && matches!(fmt_kind.as_str(), "number" | "date" | "time")
                    {
                        complex_kinds.insert(var_name.to_string(), fmt_kind);
                    }
                }
            }
        }
        pos += 1;
    }

    depth == 0
}

fn parse_icu_arms(
    arms: &str,
    vars: &mut BTreeSet<String>,
    complex_kinds: &mut BTreeMap<String, String>,
) -> bool {
    let mut depth = 0usize;
    let mut arm_start = 0usize;
    let mut saw_arm = false;

    for (idx, ch) in arms.char_indices() {
        if ch == '{' {
            if depth == 0 {
                arm_start = idx + ch.len_utf8();
            }
            depth += 1;
        } else if ch == '}' {
            if depth == 0 {
                return false;
            }
            depth -= 1;
            if depth == 0 {
                saw_arm = true;
                let arm_body = &arms[arm_start..idx];
                if !extract_icu_vars_and_kinds(arm_body, vars, complex_kinds, false) {
                    return false;
                }
            }
        }
    }

    depth == 0 && saw_arm
}

fn extract_xml_tags(input: &str) -> Option<BTreeSet<String>> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut tags = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();

    while i < bytes.len() {
        // Skip ICU single-quoted literals like `'<device>'` or `'{'` so CLI placeholders
        // inside quotes are not misinterpreted as unclosed FormatJS XML tags.
        if bytes[i] == b'\'' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                i += 2;
                continue;
            }
            if i + 1 < bytes.len() && matches!(bytes[i + 1], b'<' | b'{' | b'}') {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let start = i + 1;
        if start >= bytes.len() {
            break;
        }
        let is_close = bytes[start] == b'/';
        let name_start = if is_close { start + 1 } else { start };
        if name_start >= bytes.len() || !bytes[name_start].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let mut j = name_start + 1;
        while j < bytes.len()
            && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
        {
            j += 1;
        }
        let tag_name = std::str::from_utf8(&bytes[name_start..j]).ok()?;
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        let is_self_closing = !is_close && k < bytes.len() && bytes[k] == b'/';
        if is_self_closing {
            k += 1;
        }
        if k < bytes.len() && bytes[k] == b'>' {
            tags.insert(tag_name.to_string());
            if !is_self_closing {
                if is_close {
                    let open = stack.pop()?;
                    if open != tag_name {
                        return None;
                    }
                } else {
                    stack.push(tag_name.to_string());
                }
            }
            i = k + 1;
            continue;
        }
        i += 1;
    }

    if !stack.is_empty() {
        return None;
    }
    Some(tags)
}

fn build_icu_profile(s: &str) -> Option<IcuProfile> {
    let mut vars = BTreeSet::new();
    let mut complex_kinds = BTreeMap::new();
    if !extract_icu_vars_and_kinds(s, &mut vars, &mut complex_kinds, true) {
        return None;
    }
    let tags = extract_xml_tags(s)?;
    Some(IcuProfile {
        vars,
        complex_kinds,
        tags,
    })
}

/// Validates that a candidate Simplified Chinese FormatJS / ICU translation preserves
/// balanced `{}` braces, identical ICU placeholder names & plural/select/number kinds,
/// and identical balanced XML/HTML rich-text tags (`<a>`, `<link>`, `<kbd>`, etc.).
pub fn validate_icu_compatibility(en: &str, zh: &str) -> bool {
    let Some(en_profile) = build_icu_profile(en) else {
        return false;
    };
    let Some(zh_profile) = build_icu_profile(zh) else {
        return false;
    };
    en_profile == zh_profile
}

fn extract_asar_header_sha256(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() < 16 {
        return Err("ASAR 文件过短，不足 16 字节头部".to_string());
    }
    let strlen = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    let json_end = 16usize
        .checked_add(strlen)
        .ok_or_else(|| "ASAR 头部 JSON 长度溢出".to_string())?;
    if json_end > bytes.len() {
        return Err("ASAR 头部 JSON 越界".to_string());
    }
    Ok(sha256_hex(&bytes[16..json_end]))
}

fn read_archive_app_version(archive: &AsarArchive) -> Option<String> {
    let pkg = archive.get_file("package.json")?;
    let val: Value = serde_json::from_slice(pkg).ok()?;
    val.get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn verify_clean_backup_file(
    paths: &AppPaths,
    backup_filename: &str,
    expected_sha256: &str,
) -> Result<Vec<u8>, String> {
    if expected_sha256.trim().is_empty() {
        return Err(format!("备份元数据缺少 {backup_filename} 对应的 SHA-256"));
    }
    let orig_path = ui_patch_backup_dir(paths).join(backup_filename);
    if !orig_path.is_file() {
        return Err(format!("缺少 {backup_filename} 原始备份文件"));
    }
    let bytes = fs::read(&orig_path)
        .map_err(|e| format!("读取原始备份 {} 失败: {e}", orig_path.display()))?;
    let actual_sha = sha256_hex(&bytes);
    if !actual_sha.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(format!(
            "原始备份 {backup_filename} SHA-256 校验不一致（期望 {}，实际 {}）",
            expected_sha256, actual_sha
        ));
    }
    let parsed = AsarArchive::parse(&bytes)?;
    if parsed.get_file(PATCH_MARKER_FILE).is_some() {
        return Err(format!(
            "备份文件 {backup_filename} 异常包含补丁标识，拒绝使用已污染备份"
        ));
    }
    Ok(bytes)
}

fn find_matching_delim(bytes: &[u8], open_idx: usize, open_ch: u8, close_ch: u8) -> Option<usize> {
    if bytes.get(open_idx).copied() != Some(open_ch) {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open_idx;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(q) = quote {
            if ch == b'\\' {
                i += 2;
                continue;
            }
            if ch == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if ch == b'\'' || ch == b'"' || ch == b'`' {
            quote = Some(ch);
            i += 1;
            continue;
        }
        if ch == open_ch {
            depth += 1;
        } else if ch == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_enclosing_brace_open(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut depth = 0isize;
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn patch_app_asar_desktop_intl_js(raw: &str) -> (String, bool) {
    if raw.contains(CLAUDE_PATCH_GUARD_COMMENT) {
        return (raw.to_string(), false);
    }

    let mut out = raw.to_string();
    let mut changed = false;

    // 1. Add zh-Hans/zh mappings to DesktopIntl locale map containing en:"en-US" (or "en":"en-US"), "ja-JP", and "id-ID".
    let needles = [
        "en:\"en-US\"",
        "\"en\":\"en-US\"",
        "en: \"en-US\"",
        "\"en\": \"en-US\"",
        "en:'en-US'",
        "'en':'en-US'",
    ];
    let mut search_from = 0usize;
    while search_from < out.len() {
        let next_match = needles
            .iter()
            .filter_map(|needle| out[search_from..].find(needle).map(|rel| (search_from + rel, *needle)))
            .min_by_key(|(pos, _)| *pos);
        let Some((match_pos, needle)) = next_match else {
            break;
        };
        let bytes = out.as_bytes();
        if let Some(open_idx) = find_enclosing_brace_open(bytes, match_pos) {
            if let Some(close_idx) = find_matching_delim(bytes, open_idx, b'{', b'}') {
                let obj_slice = &out[open_idx..=close_idx];
                if obj_slice.contains("ja-JP")
                    && obj_slice.contains("id-ID")
                    && !obj_slice.contains("zh-Hans")
                {
                    let inner_trim = out[open_idx + 1..close_idx].trim_end();
                    let sep = if inner_trim.ends_with(',') { "" } else { "," };
                    let injection = format!(
                        r#"{sep}"zh-Hans":"zh-Hans",zh:"zh-Hans","zh-CN":"zh-Hans","zh-SG":"zh-Hans""#
                    );
                    out.insert_str(close_idx, &injection);
                    changed = true;
                    search_from = close_idx + injection.len() + 1;
                    continue;
                }
            }
        }
        search_from = match_pos + needle.len();
    }

    // 2. Relax sHe() locale filename regex /[a-z]{2}-[A-Z]{2}/ -> /[a-z]{2,3}-[A-Za-z0-9]{2,8}/
    // so zh-Hans.json and es-419.json pass alongside en-US.json.
    if out.contains("[a-z]{2}-[A-Z]{2}") {
        out = out.replace("[a-z]{2}-[A-Z]{2}", "[a-z]{2,3}-[A-Za-z0-9]{2,8}");
        changed = true;
    }

    if changed {
        (format!("{CLAUDE_PATCH_GUARD_COMMENT}\n{out}"), true)
    } else {
        (out, false)
    }
}

const CLAUDE_CODE_BASE_URL_JSON_PROP: &str =
    r#""baseUrl":"https://downloads.claude.ai/claude-code-releases""#;
const CLAUDE_CODE_BASE_URL_JSON_PROP_PATCHED: &str = r#""baseUrl":"'+((()=>{try{return require("electron").app.getPath("userData").replaceAll("\\","/").replace(/\/+$/,"").toLowerCase().endsWith("/claude-3p")?"http://127.0.0.1:18419/claude-code-releases":"https://downloads.claude.ai/claude-code-releases"}catch{return"https://downloads.claude.ai/claude-code-releases"}})())+'""#;

/// Rewrites Claude Desktop's embedded `claude-code-releases` `baseUrl` inside
/// `JSON.parse('...')` so Tako (3P / `Claude-3p`) mode downloads Claude Code CLI
/// through the local Tako gateway (`http://127.0.0.1:18419/claude-code-releases`,
/// China CDN mirror with official upstream fallback), while Official (1P) mode
/// continues using `https://downloads.claude.ai/claude-code-releases` directly.
fn patch_claude_code_mirror_base_url(raw: &str) -> (String, bool) {
    if raw.contains("127.0.0.1:18419/claude-code-releases") {
        return (raw.to_string(), false);
    }
    let Some(match_pos) = raw.find(CLAUDE_CODE_BASE_URL_JSON_PROP) else {
        return (raw.to_string(), false);
    };
    // Confirm the match is inside a single-quoted JSON.parse('...') literal so
    // breaking out of single quotes is guaranteed to stay valid JS.
    let lookback_start = match_pos.saturating_sub(16 * 1024);
    let prefix = &raw[lookback_start..match_pos];
    let Some(parse_rel) = prefix.rfind("JSON.parse('") else {
        return (raw.to_string(), false);
    };
    let between = &prefix[parse_rel + "JSON.parse('".len()..];
    let mut escaped = false;
    for ch in between.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' {
            return (raw.to_string(), false);
        }
    }

    let mut out = String::with_capacity(raw.len() + CLAUDE_CODE_BASE_URL_JSON_PROP_PATCHED.len());
    out.push_str(&raw[..match_pos]);
    out.push_str(CLAUDE_CODE_BASE_URL_JSON_PROP_PATCHED);
    out.push_str(&raw[match_pos + CLAUDE_CODE_BASE_URL_JSON_PROP.len()..]);
    (out, true)
}

fn parse_quoted_string_array_elements(slice: &str) -> Option<(Vec<String>, char)> {
    let trimmed = slice.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return None;
    }
    let mut elems = Vec::new();
    let mut quote_char = '"';
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let bytes = p.as_bytes();
        if bytes.len() < 2 {
            return None;
        }
        let q = bytes[0];
        if (q != b'"' && q != b'\'') || bytes[bytes.len() - 1] != q {
            return None;
        }
        quote_char = q as char;
        elems.push(p[1..p.len() - 1].to_string());
    }
    Some((elems, quote_char))
}

fn patch_ion_whitelist_arrays(src: &str) -> (String, usize) {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len() + 128);
    let mut cursor = 0usize;
    let mut count = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'[' {
            let max_end = (i + 320).min(bytes.len());
            if let Some(rel_close) = bytes[i..max_end].iter().position(|&b| b == b']') {
                let close_idx = i + rel_close;
                let candidate = &src[i..=close_idx];
                if !candidate.contains('{') && !candidate.contains(';') {
                    if let Some((elems, q)) = parse_quoted_string_array_elements(candidate) {
                        let matched_std = STANDARD_ION_LOCALES
                            .iter()
                            .filter(|loc| elems.iter().any(|e| e == *loc))
                            .count();
                        if matched_std >= 8
                            && elems.iter().any(|e| e == "en-US")
                            && elems.iter().any(|e| e == "id-ID")
                        {
                            if elems.iter().any(|e| e == "zh-Hans") {
                                count += 1;
                            } else {
                                out.push_str(&src[cursor..close_idx]);
                                let inner_trim = src[i + 1..close_idx].trim_end();
                                if !inner_trim.ends_with(',') {
                                    out.push(',');
                                }
                                out.push(q);
                                out.push_str("zh-Hans");
                                out.push(q);
                                out.push(']');
                                cursor = close_idx + 1;
                                count += 1;
                                i = close_idx + 1;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }

    out.push_str(&src[cursor..]);
    (out, count)
}

fn patch_ion_locale_display_map(src: &str) -> (String, bool) {
    let mut out = src.to_string();
    let mut changed = false;
    let needles = ["\"en-US\":{", "\"en-US\": {", "'en-US':{", "'en-US': {"];
    let mut search_from = 0usize;

    while search_from < out.len() {
        let next_match = needles
            .iter()
            .filter_map(|n| out[search_from..].find(n).map(|rel| (search_from + rel, *n)))
            .min_by_key(|(pos, _)| *pos);
        let Some((match_pos, needle)) = next_match else {
            break;
        };
        let bytes = out.as_bytes();
        if let Some(open_idx) = find_enclosing_brace_open(bytes, match_pos) {
            if let Some(close_idx) = find_matching_delim(bytes, open_idx, b'{', b'}') {
                let slice = &out[open_idx..=close_idx];
                if slice.contains("localName")
                    && (slice.contains("id-ID") || slice.contains("ja-JP"))
                    && !slice.contains("zh-Hans")
                {
                    let inner_trim = out[open_idx + 1..close_idx].trim_end();
                    let sep = if inner_trim.ends_with(',') { "" } else { "," };
                    let injection = format!(
                        r#"{sep}"zh-Hans":{{name:"Chinese (Simplified)",localName:"中文（简体）"}}"#
                    );
                    out.insert_str(close_idx, &injection);
                    changed = true;
                    search_from = close_idx + injection.len() + 1;
                    continue;
                }
            }
        }
        search_from = match_pos + needle.len();
    }

    (out, changed)
}

fn patch_ion_js_chunk(raw: &str) -> (String, usize, bool) {
    if raw.contains(CLAUDE_PATCH_GUARD_COMMENT) {
        return (raw.to_string(), 1, false);
    }
    let (after_arr, arr_hits) = patch_ion_whitelist_arrays(raw);
    let (after_map, map_changed) = patch_ion_locale_display_map(&after_arr);
    let mutated = after_arr != raw || map_changed;
    if mutated {
        (
            format!("{CLAUDE_PATCH_GUARD_COMMENT}\n{after_map}"),
            arr_hits,
            true,
        )
    } else {
        (raw.to_string(), arr_hits, false)
    }
}

fn parse_js_quoted_string(slice: &str) -> Option<(String, usize)> {
    let bytes = slice.as_bytes();
    let quote = *bytes.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let mut i = 1usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == b'\n' || b == b'\r' {
            return None;
        }
        if b == quote {
            let raw_inner = &slice[1..i];
            let decoded = if quote == b'"' {
                let raw_literal = &slice[..=i];
                serde_json::from_str::<String>(raw_literal).ok().or_else(|| {
                    let sanitized = raw_inner.replace("\\'", "'");
                    serde_json::from_str::<String>(&format!("\"{sanitized}\"")).ok()
                })?
            } else {
                let mut json_buf = String::with_capacity(raw_inner.len() + 2);
                json_buf.push('"');
                let mut chars = raw_inner.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        if let Some(&next) = chars.peek() {
                            if next == '\'' {
                                chars.next();
                                json_buf.push('\'');
                            } else {
                                json_buf.push('\\');
                                json_buf.push(chars.next().unwrap());
                            }
                        } else {
                            return None;
                        }
                    } else if c == '"' {
                        json_buf.push_str("\\\"");
                    } else {
                        json_buf.push(c);
                    }
                }
                json_buf.push('"');
                serde_json::from_str::<String>(&json_buf).ok()?
            };
            return Some((decoded, i + 1));
        }
        i += 1;
    }
    None
}

fn is_valid_format_message_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && !id.chars().any(|c| c.is_whitespace() || matches!(c, '{' | '}' | '"' | '\''))
}

fn skip_ws(s: &str) -> &str {
    s.trim_start_matches(|c: char| c.is_ascii_whitespace())
}

fn try_parse_named_prop<'a>(s: &'a str, prop: &str) -> Option<(String, &'a str)> {
    let rest = skip_ws(s);
    let after_name = rest
        .strip_prefix(prop)
        .or_else(|| {
            rest.strip_prefix('"')
                .and_then(|r| r.strip_prefix(prop))
                .and_then(|r| r.strip_prefix('"'))
        })
        .or_else(|| {
            rest.strip_prefix('\'')
                .and_then(|r| r.strip_prefix(prop))
                .and_then(|r| r.strip_prefix('\''))
        })?;
    let after_colon = skip_ws(after_name).strip_prefix(':')?;
    let val_slice = skip_ws(after_colon);
    let (val, consumed) = parse_js_quoted_string(val_slice)?;
    Some((val, &val_slice[consumed..]))
}

/// Extracts React-Intl `{ id, defaultMessage }` message descriptors from bundled JS
/// chunks so messages omitted from `i18n/en-US.json` (or living in `frame-shell`)
/// can still be translated into `zh-Hans.json`.
fn extract_js_format_messages(src: &str, out: &mut HashMap<String, String>) {
    let needle = "defaultMessage";
    let mut search_from = 0usize;
    while let Some(rel) = src[search_from..].find(needle) {
        let key_pos = search_from + rel;
        search_from = key_pos + needle.len();

        // Ensure `defaultMessage` is a property key (preceded by `{`, `,`, quote, or whitespace)
        if key_pos > 0 {
            let prev = src.as_bytes()[key_pos - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                continue;
            }
        }

        let after_key = &src[key_pos + needle.len()..];
        let after_key_trim = after_key
            .strip_prefix('"')
            .or_else(|| after_key.strip_prefix('\''))
            .unwrap_or(after_key);
        let Some(after_colon) = skip_ws(after_key_trim).strip_prefix(':') else {
            continue;
        };
        let dm_slice = skip_ws(after_colon);
        let Some((dm_val, dm_len)) = parse_js_quoted_string(dm_slice) else {
            continue;
        };
        let after_dm = &dm_slice[dm_len..];

        // Case 1: `defaultMessage: "...", [description: "...",] id: "..."`
        let mut found_id: Option<String> = None;
        if let Some(after_comma) = skip_ws(after_dm).strip_prefix(',') {
            if let Some((id_val, _)) = try_parse_named_prop(after_comma, "id") {
                found_id = Some(id_val);
            } else if let Some((_, after_desc)) = try_parse_named_prop(after_comma, "description") {
                if let Some(after_desc_comma) = skip_ws(after_desc).strip_prefix(',') {
                    if let Some((id_val, _)) = try_parse_named_prop(after_desc_comma, "id") {
                        found_id = Some(id_val);
                    }
                }
            }
        }

        // Case 2: `id: "...", [description: "...",] defaultMessage: "..."`
        if found_id.is_none() {
            let lookback_start = key_pos.saturating_sub(320);
            let mut safe_start = lookback_start;
            while safe_start < key_pos && !src.is_char_boundary(safe_start) {
                safe_start += 1;
            }
            let prefix = &src[safe_start..key_pos];
            // Trim to after the closest `{` so we stay inside the same flat descriptor object.
            let obj_prefix = match prefix.rfind('{') {
                Some(b) => &prefix[b + 1..],
                None => prefix,
            };
            if !obj_prefix.contains('}') {
                if let Some((id_val, rest)) = try_parse_named_prop(obj_prefix, "id") {
                    let after_id = skip_ws(rest);
                    if let Some(after_comma) = after_id.strip_prefix(',') {
                        let tail = skip_ws(after_comma);
                        if tail.is_empty() || tail == "\"" || tail == "'" {
                            found_id = Some(id_val);
                        } else if let Some((_, after_desc)) =
                            try_parse_named_prop(after_comma, "description")
                        {
                            if let Some(after_desc_comma) = skip_ws(after_desc).strip_prefix(',') {
                                let tail2 = skip_ws(after_desc_comma);
                                if tail2.is_empty() || tail2 == "\"" || tail2 == "'" {
                                    found_id = Some(id_val);
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(id) = found_id {
            if is_valid_format_message_id(&id) {
                out.entry(id).or_insert(dm_val);
            }
        }
    }
}

fn patch_frame_shell_html_meta(html: &str, hash12: &str) -> (String, bool) {
    let Some(meta_pos) = html.find("name=\"frame-shell-i18n\"") else {
        return (html.to_string(), false);
    };
    let Some(tag_start) = html[..meta_pos].rfind('<') else {
        return (html.to_string(), false);
    };
    let Some(tag_end_rel) = html[meta_pos..].find('>') else {
        return (html.to_string(), false);
    };
    let tag_end = meta_pos + tag_end_rel + 1;
    let orig_tag = &html[tag_start..tag_end];
    let mut tag = orig_tag.to_string();

    // 1. Ensure `zh-Hans` is listed in `data-shipped="..."`
    if let Some(shipped_pos) = tag.find("data-shipped=\"") {
        let val_start = shipped_pos + "data-shipped=\"".len();
        if let Some(val_end_rel) = tag[val_start..].find('"') {
            let val_end = val_start + val_end_rel;
            let shipped_val = &tag[val_start..val_end];
            if !shipped_val.split(',').any(|loc| loc == "zh-Hans") {
                let sep = if shipped_val.is_empty() { "" } else { "," };
                tag.insert_str(val_end, &format!("{sep}zh-Hans"));
            }
        }
    }

    // 2. Ensure `zh-Hans:<hash12>` is listed in `data-catalogs="..."`
    if let Some(cat_pos) = tag.find("data-catalogs=\"") {
        let val_start = cat_pos + "data-catalogs=\"".len();
        if let Some(val_end_rel) = tag[val_start..].find('"') {
            let val_end = val_start + val_end_rel;
            let cat_val = &tag[val_start..val_end];
            let mut entries: Vec<String> = cat_val
                .split(',')
                .filter(|e| !e.is_empty() && !e.starts_with("zh-Hans:"))
                .map(str::to_string)
                .collect();
            entries.push(format!("zh-Hans:{hash12}"));
            let next_cat = entries.join(",");
            if next_cat != cat_val {
                tag.replace_range(val_start..val_end, &next_cat);
            }
        }
    }

    if tag == orig_tag {
        return (html.to_string(), false);
    }

    let mut out = String::with_capacity(html.len() + 32);
    out.push_str(&html[..tag_start]);
    out.push_str(&tag);
    out.push_str(&html[tag_end..]);
    (out, true)
}

fn patch_app_frame_shell_i18n(
    archive: &mut AsarArchive,
    app_en_map: &HashMap<String, String>,
    ion_en_map: &HashMap<String, String>,
    shell_en_to_zh: &HashMap<String, String>,
    dict: &HashMap<String, String>,
) -> Result<(), String> {
    let html_path = "resources/frame/shell/frame-shell.html";
    let Some(html_bytes) = archive.get_file(html_path) else {
        return Ok(());
    };
    let Ok(html_str) = std::str::from_utf8(html_bytes).map(str::to_string) else {
        return Ok(());
    };

    let ref_catalog_path = archive
        .file_paths()
        .into_iter()
        .find(|p| {
            p.starts_with("resources/frame/shell/i18n/frame-shell/")
                && p.ends_with(".json")
                && !p.ends_with("/zh-Hans.json")
        });
    let Some(ref_catalog_path) = ref_catalog_path else {
        return Ok(());
    };
    let Some(ref_bytes) = archive.get_file(&ref_catalog_path) else {
        return Ok(());
    };
    let Ok(ref_map) = serde_json::from_slice::<Map<String, Value>>(ref_bytes) else {
        return Ok(());
    };

    let mut frame_js_msgs: HashMap<String, String> = HashMap::new();
    let frame_js_paths: Vec<String> = archive
        .file_paths()
        .into_iter()
        .filter(|p| p.starts_with("resources/frame/shell/") && p.ends_with(".js"))
        .collect();
    for rel in frame_js_paths {
        if let Some(raw) = archive.get_file(&rel) {
            if let Ok(txt) = std::str::from_utf8(raw) {
                extract_js_format_messages(txt, &mut frame_js_msgs);
            }
        }
    }

    let mut zh_map = Map::with_capacity(ref_map.len());
    for key in ref_map.keys() {
        let en_opt = ion_en_map
            .get(key)
            .or_else(|| frame_js_msgs.get(key))
            .or_else(|| app_en_map.get(key));
        if let Some(en_str) = en_opt {
            let zh = translate_en_to_zh(en_str, shell_en_to_zh, dict);
            zh_map.insert(key.clone(), Value::String(zh.to_string()));
        }
    }
    for (key, en_str) in &frame_js_msgs {
        if !zh_map.contains_key(key) {
            let zh = translate_en_to_zh(en_str, shell_en_to_zh, dict);
            if zh != en_str.as_str() {
                zh_map.insert(key.clone(), Value::String(zh.to_string()));
            }
        }
    }

    if zh_map.is_empty() {
        return Ok(());
    }

    let zh_bytes = serde_json::to_vec(&Value::Object(zh_map))
        .map_err(|e| format!("序列化 frame-shell zh-Hans.json 失败: {e}"))?;
    let hash12 = &sha256_hex(&zh_bytes)[..12];
    archive.set_file("resources/frame/shell/i18n/frame-shell/zh-Hans.json", zh_bytes)?;

    let (patched_html, changed) = patch_frame_shell_html_meta(&html_str, hash12);
    if changed {
        archive.set_file(html_path, patched_html.into_bytes())?;
    }
    Ok(())
}

fn translate_en_to_zh<'a>(
    en: &'a str,
    shell_en_to_zh: &'a HashMap<String, String>,
    dict: &'a HashMap<String, String>,
) -> &'a str {
    if let Some(cand) = shell_en_to_zh.get(en) {
        if validate_icu_compatibility(en, cand) {
            return cand.as_str();
        }
    }
    if let Some(cand) = dict.get(en) {
        if validate_icu_compatibility(en, cand) {
            return cand.as_str();
        }
    }
    en
}

fn patch_app_asar_in_memory(
    archive: &mut AsarArchive,
    orig_sha256: &str,
    app_version: Option<&str>,
    target_patch_version: &str,
    ion_en_map: &HashMap<String, String>,
    dict: &HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    let mut shell_en_to_zh: HashMap<String, String> = HashMap::new();
    let mut app_en_strings: HashMap<String, String> = HashMap::new();

    // 1. Merge/fill resources/i18n/zh-Hans.json while preserving genuinely translated entries
    //    and upgrading entries that upstream left untranslated (existing_zh == en_str).
    if let Some(en_bytes) = archive.get_file("resources/i18n/en-US.json") {
        let en_map: Map<String, Value> = serde_json::from_slice(en_bytes)
            .map_err(|e| format!("解析 app.asar resources/i18n/en-US.json 失败: {e}"))?;
        let mut zh_map: Map<String, Value> = archive
            .get_file("resources/i18n/zh-Hans.json")
            .and_then(|b| serde_json::from_slice::<Map<String, Value>>(b).ok())
            .unwrap_or_default();

        for (key, en_val) in &en_map {
            let Some(en_str) = en_val.as_str() else {
                continue;
            };
            app_en_strings.insert(key.clone(), en_str.to_string());
            if let Some(existing_zh) = zh_map.get(key).and_then(Value::as_str) {
                let existing_is_translated = !existing_zh.is_empty() && existing_zh != en_str;
                if existing_is_translated && validate_icu_compatibility(en_str, existing_zh) {
                    shell_en_to_zh
                        .entry(en_str.to_string())
                        .or_insert_with(|| existing_zh.to_string());
                } else if let Some(cand) = dict.get(en_str).filter(|cand| {
                    cand.as_str() != en_str && validate_icu_compatibility(en_str, cand)
                }) {
                    zh_map.insert(key.clone(), Value::String(cand.clone()));
                }
            } else {
                let translated = dict
                    .get(en_str)
                    .filter(|cand| validate_icu_compatibility(en_str, cand))
                    .map(String::as_str)
                    .unwrap_or(en_str);
                zh_map.insert(key.clone(), Value::String(translated.to_string()));
            }
        }

        let zh_bytes = serde_json::to_vec(&Value::Object(zh_map))
            .map_err(|e| format!("序列化 app.asar resources/i18n/zh-Hans.json 失败: {e}"))?;
        archive.set_file("resources/i18n/zh-Hans.json", zh_bytes)?;
    }

    // 1b. Generate resources/frame/shell/i18n/frame-shell/zh-Hans.json and register it in
    //     frame-shell.html so embedded Artifact / Cowork frames also render in Chinese.
    patch_app_frame_shell_i18n(archive, &app_en_strings, ion_en_map, &shell_en_to_zh, dict)?;

    // 2. Patch .vite/build/*.js
    let build_js_paths: Vec<String> = archive
        .file_paths()
        .into_iter()
        .filter(|p| p.starts_with(".vite/build/") && p.ends_with(".js"))
        .collect();

    let mut matched_or_guarded = 0usize;
    for rel in build_js_paths {
        let Some(raw_bytes) = archive.get_file(&rel) else {
            continue;
        };
        let Ok(raw_str) = std::str::from_utf8(raw_bytes) else {
            continue;
        };
        let (working, intl_changed) = if raw_str.contains(CLAUDE_PATCH_GUARD_COMMENT) {
            matched_or_guarded += 1;
            (raw_str.to_string(), false)
        } else {
            let (patched, changed) = patch_app_asar_desktop_intl_js(raw_str);
            if changed {
                matched_or_guarded += 1;
            }
            (patched, changed)
        };
        let (final_js, ccd_changed) = patch_claude_code_mirror_base_url(&working);
        if intl_changed || ccd_changed {
            archive.set_file(&rel, final_js.into_bytes())?;
        }
    }

    if matched_or_guarded == 0 {
        return Err(
            "未在 Claude Desktop app.asar (.vite/build/*.js) 中找到受支持的语言配置入口，已安全中止修改"
                .to_string(),
        );
    }

    // 3. Inject patch marker
    let marker = ClaudePatchMarker {
        schema: 1,
        patch_version: target_patch_version.to_string(),
        original_asar_sha256: orig_sha256.to_string(),
        app_version: app_version.map(str::to_string),
    };
    let marker_bytes = serde_json::to_vec_pretty(&marker)
        .map_err(|e| format!("序列化 app.asar 补丁标识失败: {e}"))?;
    archive.set_file(PATCH_MARKER_FILE, marker_bytes)?;

    Ok(shell_en_to_zh)
}

fn patch_ion_index_html_ui_polish(raw: &str) -> (String, bool) {
    if raw.contains(CLAUDE_UI_POLISH_MARKER) {
        return (raw.to_string(), false);
    }
    let cleaned = if let Some(start) = raw.find(&format!("<style id=\"{CLAUDE_UI_POLISH_STYLE_ID}\"")) {
        if let Some(end_rel) = raw[start..].find("</style>") {
            let end = start + end_rel + "</style>".len();
            format!("{}{}", &raw[..start], &raw[end..])
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };
    if let Some(pos) = cleaned.rfind("</body>") {
        let mut out = String::with_capacity(cleaned.len() + CLAUDE_UI_POLISH_STYLE_TAG.len());
        out.push_str(&cleaned[..pos]);
        out.push_str(CLAUDE_UI_POLISH_STYLE_TAG);
        out.push_str(&cleaned[pos..]);
        (out, true)
    } else if let Some(pos) = cleaned.rfind("</head>") {
        let mut out = String::with_capacity(cleaned.len() + CLAUDE_UI_POLISH_STYLE_TAG.len());
        out.push_str(&cleaned[..pos]);
        out.push_str(CLAUDE_UI_POLISH_STYLE_TAG);
        out.push_str(&cleaned[pos..]);
        (out, true)
    } else {
        (cleaned, false)
    }
}

fn patch_ion_asar_in_memory(
    archive: &mut AsarArchive,
    orig_sha256: &str,
    app_version: Option<&str>,
    target_patch_version: &str,
    shell_en_to_zh: &HashMap<String, String>,
    dict: &HashMap<String, String>,
) -> Result<(), String> {

    // 1. Scan assets/**/*.js for 11-locale whitelist arrays, display name map, and any
    //    inline FormatJS defaultMessage descriptors missing from i18n/en-US.json.
    let js_paths: Vec<String> = archive
        .file_paths()
        .into_iter()
        .filter(|p| p.starts_with("assets/") && p.ends_with(".js"))
        .collect();

    let mut total_whitelist_hits = 0usize;
    let mut js_default_messages: HashMap<String, String> = HashMap::new();
    for rel in js_paths {
        let Some(raw_bytes) = archive.get_file(&rel) else {
            continue;
        };
        let Ok(raw_str) = std::str::from_utf8(raw_bytes) else {
            continue;
        };
        extract_js_format_messages(raw_str, &mut js_default_messages);
        let (patched, hits, mutated) = patch_ion_js_chunk(raw_str);
        total_whitelist_hits += hits;
        if mutated {
            archive.set_file(&rel, patched.into_bytes())?;
        }
    }

    if total_whitelist_hits == 0 {
        return Err(
            "未在 Claude Desktop ion-dist.asar 中找到受支持的语言白名单数组，已安全中止修改"
                .to_string(),
        );
    }

    // 2. Build & inject i18n/dynamic/zh-Hans.json
    let mut dyn_zh = Map::new();
    if let Some(dyn_en_bytes) = archive.get_file("i18n/dynamic/en-US.json") {
        let dyn_en: Map<String, Value> = serde_json::from_slice(dyn_en_bytes)
            .map_err(|e| format!("解析 ion-dist.asar i18n/dynamic/en-US.json 失败: {e}"))?;
        for (k, v) in dyn_en {
            if let Some(en_str) = v.as_str() {
                let zh = translate_en_to_zh(en_str, shell_en_to_zh, dict);
                dyn_zh.insert(k, Value::String(zh.to_string()));
            } else {
                dyn_zh.insert(k, v);
            }
        }
    } else {
        for &(k, en_str) in KNOWN_DYNAMIC_EN {
            let zh = translate_en_to_zh(en_str, shell_en_to_zh, dict);
            dyn_zh.insert(k.to_string(), Value::String(zh.to_string()));
        }
    }
    let dyn_zh_bytes = serde_json::to_vec(&Value::Object(dyn_zh))
        .map_err(|e| format!("序列化 i18n/dynamic/zh-Hans.json 失败: {e}"))?;
    archive.set_file("i18n/dynamic/zh-Hans.json", dyn_zh_bytes)?;

    // 3. Build & inject i18n/zh-Hans.json with 100% key parity to i18n/en-US.json
    //    plus translations for any JS-only defaultMessage IDs omitted from en-US.json.
    let en_bytes = archive
        .get_file("i18n/en-US.json")
        .ok_or_else(|| "ion-dist.asar 缺少 i18n/en-US.json 主语言目录".to_string())?;
    let en_map: Map<String, Value> = serde_json::from_slice(en_bytes)
        .map_err(|e| format!("解析 ion-dist.asar i18n/en-US.json 失败: {e}"))?;
    let mut zh_map = Map::with_capacity(en_map.len() + js_default_messages.len().min(512));
    for (k, v) in en_map {
        if let Some(en_str) = v.as_str() {
            let zh = translate_en_to_zh(en_str, shell_en_to_zh, dict);
            zh_map.insert(k, Value::String(zh.to_string()));
        } else {
            zh_map.insert(k, v);
        }
    }
    for (id, en_str) in js_default_messages {
        if !zh_map.contains_key(&id) {
            let zh = translate_en_to_zh(&en_str, shell_en_to_zh, dict);
            if zh != en_str.as_str() {
                zh_map.insert(id, Value::String(zh.to_string()));
            }
        }
    }
    let zh_bytes = serde_json::to_vec(&Value::Object(zh_map))
        .map_err(|e| format!("序列化 i18n/zh-Hans.json 失败: {e}"))?;
    archive.set_file("i18n/zh-Hans.json", zh_bytes)?;

    // 3b. Inject clean pure-CSS empty-state layout polish into index.html (scoped strictly
    //     to `:has([data-testid="epitaxy-start-card"])` so active conversations are untouched).
    if let Some(html_bytes) = archive.get_file("index.html") {
        if let Ok(html_str) = std::str::from_utf8(html_bytes) {
            let (patched_html, changed) = patch_ion_index_html_ui_polish(html_str);
            if changed {
                archive.set_file("index.html", patched_html.into_bytes())?;
            }
        }
    }

    // 4. Inject patch marker
    let marker = ClaudePatchMarker {
        schema: 1,
        patch_version: target_patch_version.to_string(),
        original_asar_sha256: orig_sha256.to_string(),
        app_version: app_version.map(str::to_string),
    };
    let marker_bytes = serde_json::to_vec_pretty(&marker)
        .map_err(|e| format!("序列化 ion-dist.asar 补丁标识失败: {e}"))?;
    archive.set_file(PATCH_MARKER_FILE, marker_bytes)?;

    Ok(())
}

fn update_claude_info_plist_integrity(
    info_plist_path: &Path,
    new_app_header_sha256: &str,
    new_ion_header_sha256: &str,
    is_test: bool,
) -> Result<(), String> {
    if !info_plist_path.is_file() {
        return Ok(());
    }
    let raw_bytes = read_bundle_file_checked(AppId::Claude, "patch", info_plist_path)?;
    if let Ok(xml_text) = std::str::from_utf8(&raw_bytes) {
        let mut updated = xml_text.to_string();
        let mut changed = false;
        if let Some(next) =
            update_info_plist_asar_key_hash(&updated, "Resources/app.asar", new_app_header_sha256)
        {
            updated = next;
            changed = true;
        }
        if let Some(next) = update_info_plist_asar_key_hash(
            &updated,
            "Resources/ion-dist.asar",
            new_ion_header_sha256,
        ) {
            updated = next;
            changed = true;
        }
        if changed {
            return write_bundle_file_checked(
                AppId::Claude,
                "patch",
                info_plist_path,
                updated.as_bytes(),
                false,
            );
        }
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    if !is_test && raw_bytes.starts_with(b"bplist00") {
        for (key, hash) in [
            (r"ElectronAsarIntegrity.Resources/app\.asar.hash", new_app_header_sha256),
            (
                r"ElectronAsarIntegrity.Resources/ion-dist\.asar.hash",
                new_ion_header_sha256,
            ),
        ] {
            let _ = std::process::Command::new("/usr/bin/plutil")
                .args(["-replace", key, "-string", hash])
                .arg(info_plist_path)
                .output();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = is_test;

    Ok(())
}

#[cfg(target_os = "macos")]
fn electron_framework_paths(bundle_root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let fw_bundle = bundle_root
        .join("Contents")
        .join("Frameworks")
        .join("Electron Framework.framework");
    let fw_bin = fw_bundle.join("Versions").join("A").join("Electron Framework");
    let fw_cr = fw_bundle
        .join("Versions")
        .join("A")
        .join("_CodeSignature")
        .join("CodeResources");
    (fw_bundle, fw_bin, fw_cr)
}

#[cfg(target_os = "macos")]
fn list_claude_helper_apps(bundle_root: &Path) -> Vec<(String, PathBuf)> {
    let frameworks_dir = bundle_root.join("Contents").join("Frameworks");
    let Ok(entries) = fs::read_dir(&frameworks_dir) else {
        return Vec::new();
    };
    let mut helpers = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = name.strip_suffix(".app") {
            helpers.push((stem.to_string(), path));
        }
    }
    helpers.sort_by(|a, b| a.0.cmp(&b.0));
    helpers
}

#[cfg(target_os = "macos")]
fn copy_file_cow_or_regular(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录 {} 失败: {e}", parent.display()))?;
    }
    let tmp = dst.with_file_name(format!(
        ".{}.tako-tmp.{}",
        dst.file_name().and_then(|n| n.to_str()).unwrap_or("orig"),
        std::process::id()
    ));
    let _ = fs::remove_file(&tmp);
    let cow_ok = std::process::Command::new("/bin/cp")
        .args(["-c", "-p"])
        .arg(src)
        .arg(&tmp)
        .output()
        .is_ok_and(|o| o.status.success());
    if !cow_ok {
        fs::copy(src, &tmp)
            .map_err(|e| format!("复制 {} -> {} 失败: {e}", src.display(), dst.display()))?;
    }
    fs::rename(&tmp, dst).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("原子保存 {} 失败: {e}", dst.display())
    })
}

#[cfg(target_os = "macos")]
fn macos_electron_framework_digest_matches_plist(bundle_root: &Path, plist_path: &Path) -> bool {
    let (_, fw_bin, _) = electron_framework_paths(bundle_root);
    if !fw_bin.is_file() {
        return true;
    }
    let Ok(slots) = scan_active_electron_integrity_digest_slots(&fw_bin) else {
        return true;
    };
    if slots.is_empty() {
        return true;
    }
    let Ok(Some(expected)) = compute_electron_asar_integrity_digest_from_plist_file(plist_path) else {
        return false;
    };
    slots.iter().all(|(_, current)| current == &expected)
}

#[cfg(target_os = "macos")]
fn sync_macos_electron_framework_and_resign(
    bundle_root: &Path,
    plist_path: &Path,
    backup_dir: &Path,
    need_refresh_backups: bool,
) -> Result<(), String> {
    let (fw_bundle, fw_bin, fw_cr) = electron_framework_paths(bundle_root);
    let mut patched_framework = false;

    if fw_bin.is_file() {
        let slots = scan_active_electron_integrity_digest_slots(&fw_bin)?;
        if !slots.is_empty() {
            let expected_digest = compute_electron_asar_integrity_digest_from_plist_file(plist_path)?
                .ok_or_else(|| {
                    "无法从 Claude.app Info.plist 解析 ElectronAsarIntegrity 摘要".to_string()
                })?;

            let needs_digest_patch = slots.iter().any(|(_, cur)| cur != &expected_digest);
            if needs_digest_patch {
                probe_bundle_write_access(AppId::Claude, "patch", &fw_bin)?;

                let fw_backup = backup_dir.join("Electron_Framework.orig");
                let fw_cr_backup = backup_dir.join("Electron_Framework_CodeResources.orig");
                if need_refresh_backups || !fw_backup.is_file() {
                    copy_file_cow_or_regular(&fw_bin, &fw_backup)?;
                }
                if fw_cr.is_file() && (need_refresh_backups || !fw_cr_backup.is_file()) {
                    copy_file_cow_or_regular(&fw_cr, &fw_cr_backup)?;
                }

                let helpers = list_claude_helper_apps(bundle_root);
                for (stem, helper_app) in &helpers {
                    let h_bin = helper_app.join("Contents").join("MacOS").join(stem);
                    let h_cr = helper_app
                        .join("Contents")
                        .join("_CodeSignature")
                        .join("CodeResources");
                    let h_dir = backup_dir.join("helpers").join(stem);
                    let h_bin_bak = h_dir.join("binary.orig");
                    let h_cr_bak = h_dir.join("CodeResources.orig");
                    if h_bin.is_file() && (need_refresh_backups || !h_bin_bak.is_file()) {
                        copy_file_cow_or_regular(&h_bin, &h_bin_bak)?;
                    }
                    if h_cr.is_file() && (need_refresh_backups || !h_cr_bak.is_file()) {
                        copy_file_cow_or_regular(&h_cr, &h_cr_bak)?;
                    }
                }

                let offsets: Vec<u64> = slots.iter().map(|(off, _)| *off).collect();
                patch_electron_integrity_digest_slots_in_file(
                    AppId::Claude,
                    "patch",
                    &fw_bin,
                    &offsets,
                    &expected_digest,
                )?;
                patched_framework = true;
            }
        }
    }

    if patched_framework {
        resign_macos_outer_bundle_for(AppId::Claude, "patch", &fw_bundle, backup_dir)?;
        for (_, helper_app) in list_claude_helper_apps(bundle_root) {
            resign_macos_outer_bundle_for(AppId::Claude, "patch", &helper_app, backup_dir)?;
        }
    }

    resign_macos_outer_bundle_for(AppId::Claude, "patch", bundle_root, backup_dir)
}

#[cfg(target_os = "macos")]
fn restore_macos_electron_framework_and_helpers(
    bundle_root: &Path,
    backup_dir: &Path,
) -> Result<(), String> {
    let (_, fw_bin, fw_cr) = electron_framework_paths(bundle_root);
    let fw_backup = backup_dir.join("Electron_Framework.orig");
    if fw_backup.is_file() && fw_bin.is_file() {
        probe_bundle_write_access(AppId::Claude, "restore", &fw_bin)?;
        copy_file_cow_or_regular(&fw_backup, &fw_bin)?;
    }
    let fw_cr_backup = backup_dir.join("Electron_Framework_CodeResources.orig");
    if fw_cr_backup.is_file() && fw_cr.is_file() {
        let bytes = fs::read(&fw_cr_backup)
            .map_err(|e| format!("读取 {} 失败: {e}", fw_cr_backup.display()))?;
        write_bundle_file_checked(AppId::Claude, "restore", &fw_cr, &bytes, false)?;
    }

    for (stem, helper_app) in list_claude_helper_apps(bundle_root) {
        let h_dir = backup_dir.join("helpers").join(&stem);
        let h_bin_bak = h_dir.join("binary.orig");
        let h_cr_bak = h_dir.join("CodeResources.orig");
        let h_bin = helper_app.join("Contents").join("MacOS").join(&stem);
        let h_cr = helper_app
            .join("Contents")
            .join("_CodeSignature")
            .join("CodeResources");
        if h_bin_bak.is_file() && h_bin.is_file() {
            let bytes = fs::read(&h_bin_bak)
                .map_err(|e| format!("读取 {} 失败: {e}", h_bin_bak.display()))?;
            write_bundle_file_checked(AppId::Claude, "restore", &h_bin, &bytes, true)?;
        }
        if h_cr_bak.is_file() && h_cr.is_file() {
            let bytes = fs::read(&h_cr_bak)
                .map_err(|e| format!("读取 {} 失败: {e}", h_cr_bak.display()))?;
            write_bundle_file_checked(AppId::Claude, "restore", &h_cr, &bytes, false)?;
        }
    }
    Ok(())
}

pub fn inspect_claude_ui_patch(paths: &AppPaths) -> UiPatchInfo {
    let target_patch_version = effective_ui_patch_version(paths, AppId::Claude);
    let target = match resolve_claude_target(paths) {
        ClaudeTargetResolution::NotInstalled => {
            return UiPatchInfo {
                status: UiPatchStatus::NotInstalled,
                patch_version: None,
                target_patch_version: Some(target_patch_version),
                update_available: false,
                has_valid_backup: false,
                target_asar_path: None,
                note: None,
            };
        }
        ClaudeTargetResolution::MsixReadOnly { root } => {
            let prompt = make_msix_readonly_prompt(AppId::Claude, "patch", Some(&root));
            return UiPatchInfo {
                status: UiPatchStatus::Unsupported,
                patch_version: None,
                target_patch_version: Some(target_patch_version),
                update_available: false,
                has_valid_backup: false,
                target_asar_path: Some(root.to_string_lossy().to_string()),
                note: Some(prompt.message),
            };
        }
        ClaudeTargetResolution::MissingAsar { root } => {
            return UiPatchInfo {
                status: UiPatchStatus::Unsupported,
                patch_version: None,
                target_patch_version: Some(target_patch_version),
                update_available: false,
                has_valid_backup: false,
                target_asar_path: None,
                note: Some(format!(
                    "未在 {} 中找到完整的 app.asar 与 ion-dist.asar 资源包",
                    root.display()
                )),
            };
        }
        ClaudeTargetResolution::Found(t) => *t,
    };

    let target_asar_str = target.ion_asar_path.to_string_lossy().to_string();
    let (Ok(app_index), Ok(ion_index)) = (
        AsarHeaderIndex::open(&target.app_asar_path),
        AsarHeaderIndex::open(&target.ion_asar_path),
    ) else {
        return UiPatchInfo {
            status: UiPatchStatus::Unsupported,
            patch_version: None,
            target_patch_version: Some(target_patch_version),
            update_available: false,
            has_valid_backup: false,
            target_asar_path: Some(target_asar_str),
            note: Some("读取或解析 Claude Desktop ASAR 资源包头部失败".to_string()),
        };
    };

    let meta = read_patch_meta(paths);
    let app_marker_bytes = app_index.read_file(PATCH_MARKER_FILE).ok().flatten();
    let ion_marker_bytes = ion_index.read_file(PATCH_MARKER_FILE).ok().flatten();

    if let (Some(app_mb), Some(ion_mb)) = (app_marker_bytes.as_deref(), ion_marker_bytes.as_deref())
    {
        let app_marker = serde_json::from_slice::<ClaudePatchMarker>(app_mb).ok();
        let ion_marker = serde_json::from_slice::<ClaudePatchMarker>(ion_mb).ok();
        let patch_version = ion_marker
            .as_ref()
            .map(|m| m.patch_version.clone())
            .or_else(|| app_marker.as_ref().map(|m| m.patch_version.clone()))
            .or_else(|| meta.as_ref().map(|m| m.patch_version.clone()));
        let expected_app_sha = meta
            .as_ref()
            .map(|m| m.original_app_asar_sha256.as_str())
            .or_else(|| app_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
            .unwrap_or("");
        let expected_ion_sha = meta
            .as_ref()
            .map(|m| m.original_ion_asar_sha256.as_str())
            .or_else(|| ion_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
            .unwrap_or("");
        let backup_dir = ui_patch_backup_dir(paths);
        let has_valid_backup =
            quick_check_clean_backup_asar(&backup_dir.join("app.asar.orig"), expected_app_sha)
                && quick_check_clean_backup_asar(
                    &backup_dir.join("ion-dist.asar.orig"),
                    expected_ion_sha,
                );
        let update_available = patch_version.as_deref() != Some(target_patch_version.as_str());
        let note = if update_available {
            format!(
                "检测到新版简体中文词典（v{target_patch_version}），Claude 关闭时将自动同步，也可立即一键更新"
            )
        } else {
            "Claude Desktop 全量简体中文界面补丁已生效（Chat / Cowork / Claude Code）".to_string()
        };

        return UiPatchInfo {
            status: UiPatchStatus::Patched,
            patch_version,
            target_patch_version: Some(target_patch_version),
            update_available,
            has_valid_backup,
            target_asar_path: Some(target_asar_str),
            note: Some(note),
        };
    }

    if let Some(old_meta) = meta {
        return UiPatchInfo {
            status: UiPatchStatus::OutdatedAfterUpgrade,
            patch_version: Some(old_meta.patch_version),
            target_patch_version: Some(target_patch_version),
            update_available: true,
            has_valid_backup: false,
            target_asar_path: Some(target_asar_str),
            note: Some(
                "检测到 Claude Desktop 已升级至新版本，关闭客户端时将自动重打汉化，也可立即重新应用"
                    .to_string(),
            ),
        };
    }

    if !ion_index.has_packed_file("i18n/en-US.json") {
        return UiPatchInfo {
            status: UiPatchStatus::Unsupported,
            patch_version: None,
            target_patch_version: Some(target_patch_version),
            update_available: false,
            has_valid_backup: false,
            target_asar_path: Some(target_asar_str),
            note: Some("当前 Claude Desktop 安装包缺少 i18n/en-US.json 语言目录".to_string()),
        };
    }

    UiPatchInfo {
        status: UiPatchStatus::Unpatched,
        patch_version: None,
        target_patch_version: Some(target_patch_version),
        update_available: false,
        has_valid_backup: false,
        target_asar_path: Some(target_asar_str),
        note: None,
    }
}

pub fn patch_claude_ui_for_paths(paths: &AppPaths) -> Result<UiPatchInfo, String> {
    let target = match resolve_claude_target(paths) {
        ClaudeTargetResolution::NotInstalled => {
            return Err("未检测到 Claude Desktop 安装目录".to_string());
        }
        ClaudeTargetResolution::MsixReadOnly { root } => {
            return Err(format_ui_patch_block_error(&make_msix_readonly_prompt(
                AppId::Claude,
                "patch",
                Some(&root),
            )));
        }
        ClaudeTargetResolution::MissingAsar { root } => {
            return Err(format!(
                "未在 {} 中找到完整的 app.asar 与 ion-dist.asar 资源包",
                root.display()
            ));
        }
        ClaudeTargetResolution::Found(t) => *t,
    };

    let current_app_bytes = read_bundle_file_checked(AppId::Claude, "patch", &target.app_asar_path)?;
    let current_ion_bytes = read_bundle_file_checked(AppId::Claude, "patch", &target.ion_asar_path)?;
    let current_app_archive = AsarArchive::parse(&current_app_bytes)?;
    let current_ion_archive = AsarArchive::parse(&current_ion_bytes)?;

    let target_patch_version = effective_ui_patch_version(paths, AppId::Claude);
    let backup_dir = ui_patch_backup_dir(paths);
    let existing_meta = read_patch_meta(paths);

    let both_marked = current_app_archive.get_file(PATCH_MARKER_FILE).is_some()
        && current_ion_archive.get_file(PATCH_MARKER_FILE).is_some();

    if both_marked {
        let app_marker = current_app_archive
            .get_file(PATCH_MARKER_FILE)
            .and_then(|b| serde_json::from_slice::<ClaudePatchMarker>(b).ok());
        let ion_marker = current_ion_archive
            .get_file(PATCH_MARKER_FILE)
            .and_then(|b| serde_json::from_slice::<ClaudePatchMarker>(b).ok());
        let expected_app_sha = existing_meta
            .as_ref()
            .map(|m| m.original_app_asar_sha256.as_str())
            .or_else(|| app_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
            .unwrap_or("");
        let expected_ion_sha = existing_meta
            .as_ref()
            .map(|m| m.original_ion_asar_sha256.as_str())
            .or_else(|| ion_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
            .unwrap_or("");
        if app_marker
            .as_ref()
            .is_some_and(|m| m.patch_version == target_patch_version)
            && ion_marker
                .as_ref()
                .is_some_and(|m| m.patch_version == target_patch_version)
            && verify_clean_backup_file(paths, "app.asar.orig", expected_app_sha).is_ok()
            && verify_clean_backup_file(paths, "ion-dist.asar.orig", expected_ion_sha).is_ok()
        {
            #[cfg(target_os = "macos")]
            let framework_ok = if paths.is_test {
                true
            } else if let (Some(ref bundle_root), Some(ref plist_path)) =
                (&target.bundle_root, &target.info_plist_path)
            {
                macos_electron_framework_digest_matches_plist(bundle_root, plist_path)
            } else {
                true
            };
            #[cfg(not(target_os = "macos"))]
            let framework_ok = true;

            if framework_ok {
                return Ok(inspect_claude_ui_patch(paths));
            }
        }
    }

    // Pre-flight write access checks BEFORE creating/updating any backup or mutating bundle files.
    probe_bundle_write_access(AppId::Claude, "patch", &target.app_asar_path)?;
    probe_bundle_write_access(AppId::Claude, "patch", &target.ion_asar_path)?;
    if let Some(ref plist_path) = target.info_plist_path {
        if plist_path.is_file() {
            probe_bundle_write_access(AppId::Claude, "patch", plist_path)?;
        }
    }
    if let Some(ref exe_path) = target.win_exe_path {
        if exe_path.is_file() {
            let exe_bytes = read_bundle_file_checked(AppId::Claude, "patch", exe_path)?;
            if inspect_electron_asar_integrity_fuse(&exe_bytes) == Some(true) {
                probe_bundle_write_access(AppId::Claude, "patch", exe_path)?;
            }
        }
    }

    // Resolve clean base archives (NEVER overwrite .orig with patched ASAR bytes).
    let (
        mut base_app_archive,
        orig_app_sha,
        orig_app_header_sha,
        mut base_ion_archive,
        orig_ion_sha,
        orig_ion_header_sha,
        clean_win_exe_bytes_opt,
        need_refresh_backups,
    ) = if both_marked {
        let app_marker = serde_json::from_slice::<ClaudePatchMarker>(
            current_app_archive.get_file(PATCH_MARKER_FILE).unwrap(),
        )
        .map_err(|e| format!("解析 app.asar 补丁标识失败: {e}"))?;
        let ion_marker = serde_json::from_slice::<ClaudePatchMarker>(
            current_ion_archive.get_file(PATCH_MARKER_FILE).unwrap(),
        )
        .map_err(|e| format!("解析 ion-dist.asar 补丁标识失败: {e}"))?;

        let expected_app_sha = existing_meta
            .as_ref()
            .map(|m| m.original_app_asar_sha256.clone())
            .unwrap_or(app_marker.original_asar_sha256);
        let expected_ion_sha = existing_meta
            .as_ref()
            .map(|m| m.original_ion_asar_sha256.clone())
            .unwrap_or(ion_marker.original_asar_sha256);

        let clean_app_bytes =
            verify_clean_backup_file(paths, "app.asar.orig", &expected_app_sha)?;
        let clean_ion_bytes =
            verify_clean_backup_file(paths, "ion-dist.asar.orig", &expected_ion_sha)?;
        let app_hdr_sha = extract_asar_header_sha256(&clean_app_bytes)?;
        let ion_hdr_sha = extract_asar_header_sha256(&clean_ion_bytes)?;

        let clean_exe = if let Some(ref exe_path) = target.win_exe_path {
            let orig_exe_path = backup_dir.join("Claude.exe.orig");
            if orig_exe_path.is_file() {
                Some(
                    fs::read(&orig_exe_path)
                        .map_err(|e| format!("读取 {} 失败: {e}", orig_exe_path.display()))?,
                )
            } else if exe_path.is_file() {
                Some(read_bundle_file_checked(AppId::Claude, "patch", exe_path)?)
            } else {
                None
            }
        } else {
            None
        };

        (
            AsarArchive::parse(&clean_app_bytes)?,
            expected_app_sha,
            app_hdr_sha,
            AsarArchive::parse(&clean_ion_bytes)?,
            expected_ion_sha,
            ion_hdr_sha,
            clean_exe,
            false,
        )
    } else {
        // At least one ASAR is clean (fresh install or after official self-upgrade).
        // If one ASAR still has a marker (partial upgrade), recover its clean bytes from backup if available.
        let (clean_app_bytes, app_arc) =
            if let Some(mb) = current_app_archive.get_file(PATCH_MARKER_FILE) {
                let marker = serde_json::from_slice::<ClaudePatchMarker>(mb)
                    .map_err(|e| format!("解析 app.asar 补丁标识失败: {e}"))?;
                let sha = existing_meta
                    .as_ref()
                    .map(|m| m.original_app_asar_sha256.as_str())
                    .unwrap_or(&marker.original_asar_sha256);
                let orig = verify_clean_backup_file(paths, "app.asar.orig", sha)?;
                let parsed = AsarArchive::parse(&orig)?;
                (orig, parsed)
            } else {
                (current_app_bytes.clone(), current_app_archive)
            };

        let (clean_ion_bytes, ion_arc) =
            if let Some(mb) = current_ion_archive.get_file(PATCH_MARKER_FILE) {
                let marker = serde_json::from_slice::<ClaudePatchMarker>(mb)
                    .map_err(|e| format!("解析 ion-dist.asar 补丁标识失败: {e}"))?;
                let sha = existing_meta
                    .as_ref()
                    .map(|m| m.original_ion_asar_sha256.as_str())
                    .unwrap_or(&marker.original_asar_sha256);
                let orig = verify_clean_backup_file(paths, "ion-dist.asar.orig", sha)?;
                let parsed = AsarArchive::parse(&orig)?;
                (orig, parsed)
            } else {
                (current_ion_bytes.clone(), current_ion_archive)
            };

        let app_sha = sha256_hex(&clean_app_bytes);
        let ion_sha = sha256_hex(&clean_ion_bytes);
        let app_hdr_sha = extract_asar_header_sha256(&clean_app_bytes)?;
        let ion_hdr_sha = extract_asar_header_sha256(&clean_ion_bytes)?;

        let clean_exe = if let Some(ref exe_path) = target.win_exe_path {
            if exe_path.is_file() {
                Some(read_bundle_file_checked(AppId::Claude, "patch", exe_path)?)
            } else {
                None
            }
        } else {
            None
        };

        (
            app_arc,
            app_sha,
            app_hdr_sha,
            ion_arc,
            ion_sha,
            ion_hdr_sha,
            clean_exe,
            true,
        )
    };

    let app_version =
        read_archive_app_version(&base_app_archive).or(target.app_version_hint.clone());

    // Mutate BOTH archives fully in memory BEFORE writing anything to disk!
    let effective_dict = load_effective_claude_zh_dict(paths);
    let ion_en_strings: HashMap<String, String> = base_ion_archive
        .get_file("i18n/en-US.json")
        .and_then(|b| serde_json::from_slice::<Map<String, Value>>(b).ok())
        .map(|m| {
            m.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let shell_en_to_zh = patch_app_asar_in_memory(
        &mut base_app_archive,
        &orig_app_sha,
        app_version.as_deref(),
        &target_patch_version,
        &ion_en_strings,
        effective_dict.as_ref(),
    )?;
    patch_ion_asar_in_memory(
        &mut base_ion_archive,
        &orig_ion_sha,
        app_version.as_deref(),
        &target_patch_version,
        &shell_en_to_zh,
        effective_dict.as_ref(),
    )?;

    let (patched_app_bytes, patched_app_header_json) = base_app_archive.pack()?;
    let (patched_ion_bytes, patched_ion_header_json) = base_ion_archive.pack()?;

    let patched_app_sha = sha256_hex(&patched_app_bytes);
    let patched_ion_sha = sha256_hex(&patched_ion_bytes);
    let new_app_header_sha = sha256_hex(patched_app_header_json.as_bytes());
    let new_ion_header_sha = sha256_hex(patched_ion_header_json.as_bytes());

    // Prepare Windows PE patched bytes in memory (if applicable) before mutating disk
    let mut patched_win_exe_opt: Option<(Vec<u8>, String, String)> = None;
    if let Some(ref clean_exe_bytes) = clean_win_exe_bytes_opt {
        if let Some(patched_exe) = patch_windows_pe_asar_integrity(
            clean_exe_bytes,
            &[
                (&orig_app_header_sha, &new_app_header_sha),
                (&orig_ion_header_sha, &new_ion_header_sha),
            ],
        )? {
            let orig_exe_sha = sha256_hex(clean_exe_bytes);
            let patched_exe_sha = sha256_hex(&patched_exe);
            patched_win_exe_opt = Some((patched_exe, orig_exe_sha, patched_exe_sha));
        }
    }

    // Write/refresh clean .orig backups ONLY now that in-memory patching succeeded
    if need_refresh_backups {
        // Save clean original ASARs (note: if partial upgrade recovered from orig, re-writing clean bytes is identical)
        if current_app_archive_is_clean(&target.app_asar_path) {
            write_atomic(&backup_dir.join("app.asar.orig"), &current_app_bytes, false)?;
        }
        if current_app_archive_is_clean(&target.ion_asar_path) {
            write_atomic(&backup_dir.join("ion-dist.asar.orig"), &current_ion_bytes, false)?;
        }
        if let Some(ref plist_path) = target.info_plist_path {
            if plist_path.is_file() {
                let bytes = read_bundle_file_checked(AppId::Claude, "patch", plist_path)?;
                write_atomic(&backup_dir.join("Info.plist.orig"), &bytes, false)?;
            }
        }
        if let Some(ref bin_path) = target.macos_bin_path {
            if bin_path.is_file() {
                let bytes = read_bundle_file_checked(AppId::Claude, "patch", bin_path)?;
                write_atomic(&backup_dir.join("MacOS_Claude.orig"), &bytes, false)?;
            }
        }
        if let Some(ref cr_path) = target.code_resources_path {
            if cr_path.is_file() {
                let bytes = read_bundle_file_checked(AppId::Claude, "patch", cr_path)?;
                write_atomic(&backup_dir.join("CodeResources.orig"), &bytes, false)?;
            }
        }
        if patched_win_exe_opt.is_some() {
            if let Some(ref clean_exe_bytes) = clean_win_exe_bytes_opt {
                write_atomic(&backup_dir.join("Claude.exe.orig"), clean_exe_bytes, false)?;
            }
        }
    } else if patched_win_exe_opt.is_some() && !backup_dir.join("Claude.exe.orig").is_file() {
        if let Some(ref clean_exe_bytes) = clean_win_exe_bytes_opt {
            write_atomic(&backup_dir.join("Claude.exe.orig"), clean_exe_bytes, false)?;
        }
    }

    // Write patched ASARs, Info.plist, and Windows PE (all permission-checked!)
    write_bundle_file_checked(
        AppId::Claude,
        "patch",
        &target.app_asar_path,
        &patched_app_bytes,
        false,
    )?;
    write_bundle_file_checked(
        AppId::Claude,
        "patch",
        &target.ion_asar_path,
        &patched_ion_bytes,
        false,
    )?;

    if let Some(ref plist_path) = target.info_plist_path {
        update_claude_info_plist_integrity(
            plist_path,
            &new_app_header_sha,
            &new_ion_header_sha,
            paths.is_test,
        )?;
    }

    let (orig_win_exe_sha, patched_win_exe_sha) =
        if let (Some(ref exe_path), Some((patched_exe, orig_sha, patched_sha))) =
            (&target.win_exe_path, patched_win_exe_opt)
        {
            write_bundle_file_checked(AppId::Claude, "patch", exe_path, &patched_exe, false)?;
            (Some(orig_sha), Some(patched_sha))
        } else {
            (None, None)
        };

    let meta = ClaudePatchMeta {
        schema: 1,
        patch_version: target_patch_version,
        app_version,
        original_app_asar_sha256: orig_app_sha,
        patched_app_asar_sha256: patched_app_sha,
        original_ion_asar_sha256: orig_ion_sha,
        patched_ion_asar_sha256: patched_ion_sha,
        original_win_exe_sha256: orig_win_exe_sha,
        patched_win_exe_sha256: patched_win_exe_sha,
    };
    let meta_bytes = serde_json::to_vec_pretty(&meta)
        .map_err(|e| format!("序列化 Claude patch-meta.json 失败: {e}"))?;
    write_atomic(&backup_dir.join("patch-meta.json"), &meta_bytes, false)?;

    #[cfg(target_os = "macos")]
    if !paths.is_test {
        if let Some(ref bundle_root) = target.bundle_root {
            if let Some(ref plist_path) = target.info_plist_path {
                sync_macos_electron_framework_and_resign(
                    bundle_root,
                    plist_path,
                    &backup_dir,
                    need_refresh_backups,
                )?;
            } else {
                resign_macos_outer_bundle_for(AppId::Claude, "patch", bundle_root, &backup_dir)?;
            }
        }
    }

    Ok(inspect_claude_ui_patch(paths))
}

fn current_app_archive_is_clean(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .and_then(|b| AsarArchive::parse(&b).ok())
        .is_some_and(|a| a.get_file(PATCH_MARKER_FILE).is_none())
}

pub fn restore_claude_ui_for_paths(paths: &AppPaths) -> Result<UiPatchInfo, String> {
    let target = match resolve_claude_target(paths) {
        ClaudeTargetResolution::NotInstalled => {
            return Err("未检测到 Claude Desktop 安装目录".to_string());
        }
        ClaudeTargetResolution::MsixReadOnly { root } => {
            return Err(format_ui_patch_block_error(&make_msix_readonly_prompt(
                AppId::Claude,
                "restore",
                Some(&root),
            )));
        }
        ClaudeTargetResolution::MissingAsar { root } => {
            return Err(format!(
                "未在 {} 中找到完整的 app.asar 与 ion-dist.asar 资源包",
                root.display()
            ));
        }
        ClaudeTargetResolution::Found(t) => *t,
    };

    let current_app_bytes =
        read_bundle_file_checked(AppId::Claude, "restore", &target.app_asar_path)?;
    let current_ion_bytes =
        read_bundle_file_checked(AppId::Claude, "restore", &target.ion_asar_path)?;
    let current_app_archive = AsarArchive::parse(&current_app_bytes)?;
    let current_ion_archive = AsarArchive::parse(&current_ion_bytes)?;

    let backup_dir = ui_patch_backup_dir(paths);
    let meta_path = backup_dir.join("patch-meta.json");

    // If either ASAR lacks our patch marker (e.g. upgraded by official updater), NEVER downgrade!
    let (Some(app_mb), Some(ion_mb)) = (
        current_app_archive.get_file(PATCH_MARKER_FILE),
        current_ion_archive.get_file(PATCH_MARKER_FILE),
    ) else {
        if meta_path.exists() {
            fs::remove_file(&meta_path)
                .map_err(|e| format!("清理过期 {} 失败: {e}", meta_path.display()))?;
        }
        return Ok(inspect_claude_ui_patch(paths));
    };

    let meta = read_patch_meta(paths);
    let app_marker = serde_json::from_slice::<ClaudePatchMarker>(app_mb).ok();
    let ion_marker = serde_json::from_slice::<ClaudePatchMarker>(ion_mb).ok();

    let expected_app_sha = meta
        .as_ref()
        .map(|m| m.original_app_asar_sha256.as_str())
        .or_else(|| app_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
        .unwrap_or("");
    let expected_ion_sha = meta
        .as_ref()
        .map(|m| m.original_ion_asar_sha256.as_str())
        .or_else(|| ion_marker.as_ref().map(|m| m.original_asar_sha256.as_str()))
        .unwrap_or("");

    let clean_app_bytes = verify_clean_backup_file(paths, "app.asar.orig", expected_app_sha)?;
    let clean_ion_bytes = verify_clean_backup_file(paths, "ion-dist.asar.orig", expected_ion_sha)?;

    // Pre-flight permission probe before writing restored files
    probe_bundle_write_access(AppId::Claude, "restore", &target.app_asar_path)?;
    probe_bundle_write_access(AppId::Claude, "restore", &target.ion_asar_path)?;
    if let Some(ref plist_path) = target.info_plist_path {
        if backup_dir.join("Info.plist.orig").is_file() && plist_path.is_file() {
            probe_bundle_write_access(AppId::Claude, "restore", plist_path)?;
        }
    }
    if let Some(ref exe_path) = target.win_exe_path {
        if backup_dir.join("Claude.exe.orig").is_file() && exe_path.is_file() {
            probe_bundle_write_access(AppId::Claude, "restore", exe_path)?;
        }
    }

    write_bundle_file_checked(
        AppId::Claude,
        "restore",
        &target.app_asar_path,
        &clean_app_bytes,
        false,
    )?;
    write_bundle_file_checked(
        AppId::Claude,
        "restore",
        &target.ion_asar_path,
        &clean_ion_bytes,
        false,
    )?;

    let mut restored_full_signature = false;
    if target.bundle_root.is_some() {
        let orig_plist = backup_dir.join("Info.plist.orig");
        if orig_plist.is_file() {
            if let Some(ref dst_plist) = target.info_plist_path {
                let bytes = fs::read(&orig_plist)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_plist.display()))?;
                write_bundle_file_checked(AppId::Claude, "restore", dst_plist, &bytes, false)?;
            }
        }

        let orig_bin = backup_dir.join("MacOS_Claude.orig");
        let orig_cr = backup_dir.join("CodeResources.orig");
        let mut restored_bin = false;
        let mut restored_cr = false;

        if orig_bin.is_file() {
            if let Some(ref dst_bin) = target.macos_bin_path {
                let bytes = fs::read(&orig_bin)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_bin.display()))?;
                write_bundle_file_checked(AppId::Claude, "restore", dst_bin, &bytes, true)?;
                restored_bin = true;
            }
        }

        if orig_cr.is_file() {
            if let Some(ref dst_cr) = target.code_resources_path {
                let bytes = fs::read(&orig_cr)
                    .map_err(|e| format!("读取 {} 失败: {e}", orig_cr.display()))?;
                write_bundle_file_checked(AppId::Claude, "restore", dst_cr, &bytes, false)?;
                restored_cr = true;
            }
        }

        #[cfg(target_os = "macos")]
        if !paths.is_test {
            if let Some(ref bundle_root) = target.bundle_root {
                restore_macos_electron_framework_and_helpers(bundle_root, &backup_dir)?;
            }
        }

        restored_full_signature = orig_plist.is_file() && restored_bin && restored_cr;
    }

    if let Some(ref exe_path) = target.win_exe_path {
        let orig_exe = backup_dir.join("Claude.exe.orig");
        if orig_exe.is_file() {
            let bytes = fs::read(&orig_exe)
                .map_err(|e| format!("读取 {} 失败: {e}", orig_exe.display()))?;
            if let Some(expected_exe_sha) = meta
                .as_ref()
                .and_then(|m| m.original_win_exe_sha256.as_deref())
            {
                let actual_exe_sha = sha256_hex(&bytes);
                if !actual_exe_sha.eq_ignore_ascii_case(expected_exe_sha) {
                    return Err(format!(
                        "Claude.exe.orig SHA-256 校验不一致（期望 {expected_exe_sha}，实际 {actual_exe_sha}）"
                    ));
                }
            }
            write_bundle_file_checked(AppId::Claude, "restore", exe_path, &bytes, false)?;
        }
    }

    if meta_path.exists() {
        fs::remove_file(&meta_path)
            .map_err(|e| format!("清理 {} 失败: {e}", meta_path.display()))?;
    }

    #[cfg(target_os = "macos")]
    if !paths.is_test && !restored_full_signature {
        if let Some(ref bundle_root) = target.bundle_root {
            resign_macos_outer_bundle_for(AppId::Claude, "restore", bundle_root, &backup_dir)?;
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = restored_full_signature;

    Ok(inspect_claude_ui_patch(paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::antigravity_i18n::CURRENT_UI_PATCH_VERSION;
    use crate::adapters::asar_patch::{parse_ui_patch_block_error, UiPatchBlockKind};
    use tempfile::TempDir;

    fn build_synthetic_asar(files: &[(&str, &[u8])]) -> (Vec<u8>, String) {
        let mut archive = AsarArchive::new_empty();
        for &(rel, content) in files {
            archive.set_file(rel, content.to_vec()).unwrap();
        }
        archive.pack().unwrap()
    }

    fn setup_fake_macos_claude_bundle(root: &Path, version: &str) -> (AppPaths, PathBuf) {
        let mut paths = AppPaths::for_test(root);
        let app_dir = root.join("Claude.app");
        let res_dir = app_dir.join("Contents").join("Resources");
        let macos_dir = app_dir.join("Contents").join("MacOS");
        let sig_dir = app_dir.join("Contents").join("_CodeSignature");
        fs::create_dir_all(&res_dir).unwrap();
        fs::create_dir_all(&macos_dir).unwrap();
        fs::create_dir_all(&sig_dir).unwrap();

        let pkg_json = format!(r#"{{"name":"claude-desktop","version":"{version}"}}"#);
        let shell_en = r#"{
            "3KbALjoPbx":"Claude needs to fix file ownership so auto-updates can work correctly.",
            "DLwCsMd5iw":"list the sites you’re signed into in {browser}",
            "existingKey":"Settings"
        }"#;
        let shell_zh = r#"{
            "existingKey":"设置（原厂）"
        }"#;
        let index_js = r#"const QVe={en:"en-US",de:"de-DE",fr:"fr-FR",ko:"ko-KR",ja:"ja-JP","es-419":"es-419","es-ES":"es-ES",it:"it-IT",hi:"hi-IN","pt-BR":"pt-BR","id-ID":"id-ID"};function sHe(f){return /[a-z]{2}-[A-Z]{2}/.test(f);}"#;
        let (app_asar_bytes, app_hdr_json) = build_synthetic_asar(&[
            ("package.json", pkg_json.as_bytes()),
            ("resources/i18n/en-US.json", shell_en.as_bytes()),
            ("resources/i18n/zh-Hans.json", shell_zh.as_bytes()),
            (".vite/build/index.js", index_js.as_bytes()),
        ]);

        let ion_en = r#"{
            "k1":"Settings",
            "k2":"New chat",
            "k3":"Upload CSVs for Claude to analyze quantitative data with high accuracy and create interactive data visualizations.",
            "k4":"Untranslated futuristic string {foo}"
        }"#;
        let ion_dyn_en = r#"{
            "2EcGZn58SU":"Auto thinking",
            "u8n5HzNZ2R":"Extended thinking",
            "vv9L/MN3/H":"High"
        }"#;
        let ion_chunk = r#"const SUPPORTED=["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"];const NAMES={"en-US":{name:"English (United States)",localName:"English (United States)"},"id-ID":{name:"Indonesian (Indonesia)",localName:"Indonesia (Indonesia)"}};"#;
        let (ion_asar_bytes, ion_hdr_json) = build_synthetic_asar(&[
            ("i18n/en-US.json", ion_en.as_bytes()),
            ("i18n/dynamic/en-US.json", ion_dyn_en.as_bytes()),
            ("assets/v1/index-abc.js", ion_chunk.as_bytes()),
        ]);

        fs::write(res_dir.join("app.asar"), &app_asar_bytes).unwrap();
        fs::write(res_dir.join("ion-dist.asar"), &ion_asar_bytes).unwrap();

        let app_hdr_sha = sha256_hex(app_hdr_json.as_bytes());
        let ion_hdr_sha = sha256_hex(ion_hdr_json.as_bytes());
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>ElectronAsarIntegrity</key>
<dict>
  <key>Resources/app.asar</key>
  <dict><key>algorithm</key><string>SHA256</string><key>hash</key><string>{app_hdr_sha}</string></dict>
  <key>Resources/ion-dist.asar</key>
  <dict><key>algorithm</key><string>SHA256</string><key>hash</key><string>{ion_hdr_sha}</string></dict>
</dict>
</dict></plist>"#
        );
        fs::write(app_dir.join("Contents").join("Info.plist"), plist).unwrap();
        fs::write(macos_dir.join("Claude"), format!("BIN-{version}")).unwrap();
        fs::write(sig_dir.join("CodeResources"), format!("SIG-{version}")).unwrap();

        paths.claude_app_override_dir = Some(app_dir.clone());
        (paths, app_dir)
    }

    #[test]
    fn icu_compatibility_validator_checks_vars_plurals_and_xml_tags() {
        assert!(validate_icu_compatibility("Hello {name}", "你好 {name}"));
        assert!(validate_icu_compatibility(
            "{count, plural, one {# item} other {# items}}",
            "{count, plural, other {# 项}}"
        ));
        assert!(validate_icu_compatibility(
            "Click <link>here</link> or <kbd>Enter</kbd>",
            "点击<link>这里</link>或按 <kbd>Enter</kbd>"
        ));
        assert!(!validate_icu_compatibility("Hello {name}", "你好 {user}"));
        assert!(!validate_icu_compatibility("Hello {name}", "你好 {name"));
        assert!(!validate_icu_compatibility(
            "Click <link>here</link>",
            "点击这里"
        ));
        assert!(!validate_icu_compatibility(
            "Click <link>here</link>",
            "点击<link>这里</other>"
        ));
    }

    #[test]
    fn claude_macos_dual_asar_patch_idempotent_upgrade_and_lossless_restore() {
        let tmp = TempDir::new().unwrap();
        let (paths, app_dir) = setup_fake_macos_claude_bundle(tmp.path(), "6.28899.6");
        let app_asar_path = app_dir.join("Contents/Resources/app.asar");
        let ion_asar_path = app_dir.join("Contents/Resources/ion-dist.asar");
        let plist_path = app_dir.join("Contents/Info.plist");
        let bin_path = app_dir.join("Contents/MacOS/Claude");
        let cr_path = app_dir.join("Contents/_CodeSignature/CodeResources");

        let orig_app = fs::read(&app_asar_path).unwrap();
        let orig_ion = fs::read(&ion_asar_path).unwrap();
        let orig_plist = fs::read(&plist_path).unwrap();
        let orig_bin = fs::read(&bin_path).unwrap();
        let orig_cr = fs::read(&cr_path).unwrap();

        assert_eq!(inspect_claude_ui_patch(&paths).status, UiPatchStatus::Unpatched);

        let patched = patch_claude_ui_for_paths(&paths).unwrap();
        assert_eq!(patched.status, UiPatchStatus::Patched);
        assert!(patched.has_valid_backup);
        assert_eq!(patched.patch_version.as_deref(), Some(CURRENT_UI_PATCH_VERSION));

        // Verify app.asar mutations
        let patched_app_bytes = fs::read(&app_asar_path).unwrap();
        let app_arc = AsarArchive::parse(&patched_app_bytes).unwrap();
        let index_js = std::str::from_utf8(app_arc.get_file(".vite/build/index.js").unwrap()).unwrap();
        assert_eq!(index_js.matches(CLAUDE_PATCH_GUARD_COMMENT).count(), 1);
        assert!(index_js.contains(r#""zh-Hans":"zh-Hans""#));
        assert!(index_js.contains("[a-z]{2,3}-[A-Za-z0-9]{2,8}"));

        let shell_zh: Map<String, Value> =
            serde_json::from_slice(app_arc.get_file("resources/i18n/zh-Hans.json").unwrap()).unwrap();
        assert_eq!(shell_zh.get("existingKey").and_then(Value::as_str), Some("设置（原厂）"));
        assert!(shell_zh.contains_key("3KbALjoPbx"));
        assert!(shell_zh.contains_key("DLwCsMd5iw"));

        // Verify ion-dist.asar mutations
        let patched_ion_bytes = fs::read(&ion_asar_path).unwrap();
        let ion_arc = AsarArchive::parse(&patched_ion_bytes).unwrap();
        let chunk = std::str::from_utf8(ion_arc.get_file("assets/v1/index-abc.js").unwrap()).unwrap();
        assert_eq!(chunk.matches(CLAUDE_PATCH_GUARD_COMMENT).count(), 1);
        assert!(chunk.contains(r#""id-ID","zh-Hans"]"#));
        assert!(chunk.contains(r#""zh-Hans":{name:"Chinese (Simplified)",localName:"中文（简体）"}"#));

        let ion_zh: Map<String, Value> =
            serde_json::from_slice(ion_arc.get_file("i18n/zh-Hans.json").unwrap()).unwrap();
        assert_eq!(ion_zh.len(), 4);
        assert_eq!(ion_zh.get("k1").and_then(Value::as_str), Some("设置（原厂）"));
        assert_eq!(ion_zh.get("k4").and_then(Value::as_str), Some("Untranslated futuristic string {foo}"));

        let dyn_zh: Map<String, Value> =
            serde_json::from_slice(ion_arc.get_file("i18n/dynamic/zh-Hans.json").unwrap()).unwrap();
        assert_eq!(dyn_zh.get("2EcGZn58SU").and_then(Value::as_str), Some("自动思考"));

        // Verify dual Info.plist hashes updated
        let updated_plist = fs::read_to_string(&plist_path).unwrap();
        assert!(updated_plist.contains(&extract_asar_header_sha256(&patched_app_bytes).unwrap()));
        assert!(updated_plist.contains(&extract_asar_header_sha256(&patched_ion_bytes).unwrap()));

        // Idempotent second patch
        let second = patch_claude_ui_for_paths(&paths).unwrap();
        assert_eq!(second.status, UiPatchStatus::Patched);
        assert_eq!(fs::read(ui_patch_backup_dir(&paths).join("app.asar.orig")).unwrap(), orig_app);
        assert_eq!(fs::read(ui_patch_backup_dir(&paths).join("ion-dist.asar.orig")).unwrap(), orig_ion);

        // Lossless restore
        let restored = restore_claude_ui_for_paths(&paths).unwrap();
        assert_eq!(restored.status, UiPatchStatus::Unpatched);
        assert_eq!(fs::read(&app_asar_path).unwrap(), orig_app);
        assert_eq!(fs::read(&ion_asar_path).unwrap(), orig_ion);
        assert_eq!(fs::read(&plist_path).unwrap(), orig_plist);
        assert_eq!(fs::read(&bin_path).unwrap(), orig_bin);
        assert_eq!(fs::read(&cr_path).unwrap(), orig_cr);

        // Upgrade detection & safe restore (no downgrade)
        patch_claude_ui_for_paths(&paths).unwrap();
        let (_, _) = setup_fake_macos_claude_bundle(tmp.path(), "6.29000.0");
        let upgraded_ion = fs::read(&ion_asar_path).unwrap();
        assert_eq!(
            inspect_claude_ui_patch(&paths).status,
            UiPatchStatus::OutdatedAfterUpgrade
        );
        let safe_restore = restore_claude_ui_for_paths(&paths).unwrap();
        assert_eq!(safe_restore.status, UiPatchStatus::Unpatched);
        assert_eq!(fs::read(&ion_asar_path).unwrap(), upgraded_ion);
    }

    #[test]
    fn windows_squirrel_numeric_semver_and_msix_guard() {
        let tmp = TempDir::new().unwrap();
        let sq_root = tmp.path().join("AnthropicClaude");
        let old_res = sq_root.join("app-6.9999.0").join("resources");
        let new_res = sq_root.join("app-6.28899.6").join("resources");
        fs::create_dir_all(&old_res).unwrap();
        fs::create_dir_all(&new_res).unwrap();
        let (dummy, _) = build_synthetic_asar(&[("package.json", br#"{"version":"6.28899.6"}"#)]);
        fs::write(old_res.join("app.asar"), &dummy).unwrap();
        fs::write(old_res.join("ion-dist.asar"), &dummy).unwrap();
        fs::write(new_res.join("app.asar"), &dummy).unwrap();
        fs::write(new_res.join("ion-dist.asar"), &dummy).unwrap();

        match resolve_target_from_dir(&sq_root, None) {
            ClaudeTargetResolution::Found(t) => {
                assert!(t.app_asar_path.to_string_lossy().contains("app-6.28899.6"));
            }
            _ => panic!("expected numeric semver resolution to pick app-6.28899.6"),
        }

        // MSIX WindowsApps guard
        let mut msix_paths = AppPaths::for_test(tmp.path());
        let msix_dir = tmp.path().join("WindowsApps").join("Claude_1.0");
        fs::create_dir_all(&msix_dir).unwrap();
        msix_paths.claude_app_override_dir = Some(msix_dir);
        assert_eq!(
            inspect_claude_ui_patch(&msix_paths).status,
            UiPatchStatus::Unsupported
        );
        let err = patch_claude_ui_for_paths(&msix_paths).unwrap_err();
        let prompt = parse_ui_patch_block_error(&err).expect("must emit structured MSIX block");
        assert_eq!(prompt.kind, UiPatchBlockKind::WindowsMsixReadOnly);
    }

    #[test]
    fn cloud_overlay_pack_triggers_update_available_and_auto_sync() {
        let tmp = TempDir::new().unwrap();
        let (paths, app_dir) = setup_fake_macos_claude_bundle(tmp.path(), "1.1.0");
        let ion_asar_path = app_dir.join("Contents").join("Resources").join("ion-dist.asar");

        let mut st = crate::config::read_switch_state(&paths);
        st.prefer_chinese_ui = true;
        crate::config::save_switch_state(&paths, &st).unwrap();

        // 1. Initial patch uses built-in version and has no pending update
        let first = patch_claude_ui_for_paths(&paths).unwrap();
        assert_eq!(first.status, UiPatchStatus::Patched);
        assert!(!first.update_available);
        assert_eq!(first.patch_version.as_deref(), Some(CURRENT_UI_PATCH_VERSION));

        // 2. Installing a newer cloud dictionary overlay marks update_available = true
        let overlay = r#"{"New chat":"全新会话（云端词典）"}"#.as_bytes();
        let sha = sha256_hex(overlay);
        assert!(crate::adapters::install_i18n_pack_bytes(
            &paths,
            AppId::Claude,
            "20261123.9",
            overlay,
            Some(&sha),
            None,
        )
        .unwrap());

        let before_sync = inspect_claude_ui_patch(&paths);
        assert_eq!(before_sync.status, UiPatchStatus::Patched);
        assert!(before_sync.update_available);

        // 3. Background auto-sync upgrades the patch seamlessly with the overlay translation
        let report = crate::auto_sync_pending_ui_patches_for_paths(&paths, None);
        assert_eq!(report.updated, vec![AppId::Claude]);

        let after_sync = inspect_claude_ui_patch(&paths);
        assert_eq!(after_sync.status, UiPatchStatus::Patched);
        assert!(!after_sync.update_available);
        assert_eq!(
            after_sync.patch_version.as_deref(),
            Some(format!("{CURRENT_UI_PATCH_VERSION}+pack.20261123.9").as_str())
        );

        let ion_arc = AsarArchive::parse(&fs::read(&ion_asar_path).unwrap()).unwrap();
        let zh_map: Map<String, Value> =
            serde_json::from_slice(ion_arc.get_file("i18n/zh-Hans.json").unwrap()).unwrap();
        assert_eq!(
            zh_map.get("k2").and_then(Value::as_str),
            Some("全新会话（云端词典）")
        );
    }

    #[test]
    fn claude_code_mirror_base_url_patch_routes_only_3p_and_is_idempotent() {
        let sample = r#"function nc(){return JSON.parse('{"version":"2.1.331","baseUrl":"https://downloads.claude.ai/claude-code-releases","sdkWrapperVersion":"0.3.331"}')}"#;
        let (patched, changed) = patch_claude_code_mirror_base_url(sample);
        assert!(changed);
        assert!(patched.contains("127.0.0.1:18419/claude-code-releases"));
        assert!(patched.contains("https://downloads.claude.ai/claude-code-releases"));
        assert!(patched.contains("/claude-3p"));

        // Second pass must be a no-op.
        let (second, changed_again) = patch_claude_code_mirror_base_url(&patched);
        assert!(!changed_again);
        assert_eq!(second, patched);

        // Double-quoted JSON.parse("...") is skipped safely rather than producing invalid JS.
        let double_quoted = r#"JSON.parse("{\"baseUrl\":\"https://downloads.claude.ai/claude-code-releases\"}")"#;
        let (untouched, dq_changed) = patch_claude_code_mirror_base_url(double_quoted);
        assert!(!dq_changed);
        assert_eq!(untouched, double_quoted);
    }

    #[test]
    fn ion_index_html_ui_polish_injects_once_and_is_idempotent() {
        let html = "<!doctype html><html><head></head><body><div id=\"root\"></div></body></html>";
        let (patched, changed) = patch_ion_index_html_ui_polish(html);
        assert!(changed);
        assert!(patched.contains(CLAUDE_UI_POLISH_STYLE_ID));
        assert!(patched.contains(CLAUDE_UI_POLISH_MARKER));
        assert!(patched.ends_with("</body></html>"));

        let (second, changed_again) = patch_ion_index_html_ui_polish(&patched);
        assert!(!changed_again);
        assert_eq!(second, patched);
    }

    #[test]
    fn icu_validator_accepts_single_quoted_literal_braces_and_cli_tags() {
        assert!(validate_icu_compatibility(
            "The JSON must be an object of settings — it should start with '{'.",
            "JSON 必须是设置对象 — 应以 '{' 开头。"
        ));
        assert!(validate_icu_compatibility(
            "No booted simulator found. Boot one with `xcrun simctl boot '<device>'`.",
            "未找到已启动的模拟器。请运行 `xcrun simctl boot '<device>'` 启动一个。"
        ));
        // Still rejects truly unclosed ICU braces or mismatched XML tags
        assert!(!validate_icu_compatibility(
            "Hello {name}",
            "你好 {name"
        ));
        assert!(!validate_icu_compatibility(
            "Click <link>here</link>",
            "点击 <link>这里"
        ));
    }

    #[test]
    fn js_format_message_extractor_and_frame_shell_meta_patch_work_and_are_idempotent() {
        let js = r#"
            const a = f({defaultMessage:"Approve with edits",id:"UJPHMm0vsr"});
            const b = f({id:"prOcQ9FPvY",defaultMessage:"Don\u2019t enter passwords here."});
            const c = f({defaultMessage:"Voice call",description:"btn",id:"/3Yo0ONwYu"});
            const d = f({id:'singleId',defaultMessage:'Can\'t connect'});
        "#;
        let mut extracted = HashMap::new();
        extract_js_format_messages(js, &mut extracted);
        assert_eq!(
            extracted.get("UJPHMm0vsr").map(String::as_str),
            Some("Approve with edits")
        );
        assert_eq!(
            extracted.get("prOcQ9FPvY").map(String::as_str),
            Some("Don\u{2019}t enter passwords here.")
        );
        assert_eq!(
            extracted.get("/3Yo0ONwYu").map(String::as_str),
            Some("Voice call")
        );
        assert_eq!(
            extracted.get("singleId").map(String::as_str),
            Some("Can't connect")
        );

        let html = r#"<!doctype html><html><head><meta name="frame-shell-i18n" data-shipped="en-US,de-DE" data-catalogs="de-DE:7ff29108bb22"></head><body></body></html>"#;
        let (patched, changed) = patch_frame_shell_html_meta(html, "0123456789ab");
        assert!(changed);
        assert!(patched.contains(r#"data-shipped="en-US,de-DE,zh-Hans""#));
        assert!(patched.contains(r#"data-catalogs="de-DE:7ff29108bb22,zh-Hans:0123456789ab""#));

        let (second, changed_again) = patch_frame_shell_html_meta(&patched, "0123456789ab");
        assert!(!changed_again);
        assert_eq!(second, patched);
    }

    #[test]
    fn patch_upgrades_untranslated_app_shell_entries_and_injects_frame_shell_and_js_only_messages() {
        let mut app_arc = AsarArchive::new_empty();
        app_arc
            .set_file(
                "package.json",
                br#"{"name":"claude","version":"7.37095.0"}"#.to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                ".vite/build/index.js",
                br#"const locales={en:"en-US","ja-JP":"ja-JP","id-ID":"id-ID"};"#.to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                "resources/i18n/en-US.json",
                br#"{"jewac1Lnjr":"Max","NPqkiAP6m1":"Remote Control","k1":"Settings"}"#.to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                "resources/i18n/zh-Hans.json",
                r#"{"jewac1Lnjr":"Max","NPqkiAP6m1":"Remote Control","k1":"设置"}"#
                    .as_bytes()
                    .to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                "resources/frame/shell/frame-shell.html",
                br#"<!doctype html><head><meta name="frame-shell-i18n" data-shipped="en-US,de-DE" data-catalogs="de-DE:abc123456789"></head>"#.to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                "resources/frame/shell/i18n/frame-shell/de-DE.json",
                br#"{"fsOnly":"Das braucht deinen Chat.","sharedKey":"Schlaf"}"#.to_vec(),
            )
            .unwrap();
        app_arc
            .set_file(
                "resources/frame/shell/frame-shell-chrome.js",
                br#"const x={defaultMessage:"This needs your chat.",id:"fsOnly"};"#.to_vec(),
            )
            .unwrap();

        let mut ion_arc = AsarArchive::new_empty();
        ion_arc
            .set_file(
                "assets/main.js",
                br#"const LOC=["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"];const msg={defaultMessage:"Approve with edits",id:"UJPHMm0vsr"};"#.to_vec(),
            )
            .unwrap();
        ion_arc
            .set_file(
                "i18n/en-US.json",
                br#"{"sharedKey":"Sleep","mKey":"Max"}"#.to_vec(),
            )
            .unwrap();

        let ion_en: HashMap<String, String> = [
            ("sharedKey".to_string(), "Sleep".to_string()),
            ("mKey".to_string(), "Max".to_string()),
        ]
        .into_iter()
        .collect();
        let dict = claude_zh_dict();

        let shell_en_to_zh = patch_app_asar_in_memory(
            &mut app_arc,
            "orig_app_sha",
            Some("7.37095.0"),
            "2.3.7",
            &ion_en,
            dict,
        )
        .unwrap();
        // Untranslated English in orig zh-Hans must NOT shadow dict
        assert!(!shell_en_to_zh.contains_key("Max"));
        assert!(!shell_en_to_zh.contains_key("Remote Control"));
        assert_eq!(shell_en_to_zh.get("Settings").map(String::as_str), Some("设置"));

        let app_zh: Map<String, Value> =
            serde_json::from_slice(app_arc.get_file("resources/i18n/zh-Hans.json").unwrap())
                .unwrap();
        assert_eq!(app_zh.get("NPqkiAP6m1").and_then(Value::as_str), Some("远程控制"));
        assert_eq!(app_zh.get("jewac1Lnjr").and_then(Value::as_str), Some("极致"));

        let fs_zh: Map<String, Value> = serde_json::from_slice(
            app_arc
                .get_file("resources/frame/shell/i18n/frame-shell/zh-Hans.json")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs_zh.get("fsOnly").and_then(Value::as_str),
            Some("此操作需要打开您的对话。")
        );
        assert_eq!(fs_zh.get("sharedKey").and_then(Value::as_str), Some("休眠"));
        let fs_html =
            std::str::from_utf8(app_arc.get_file("resources/frame/shell/frame-shell.html").unwrap())
                .unwrap();
        assert!(fs_html.contains("data-shipped=\"en-US,de-DE,zh-Hans\""));
        assert!(fs_html.contains(",zh-Hans:"));

        patch_ion_asar_in_memory(
            &mut ion_arc,
            "orig_ion_sha",
            Some("7.37095.0"),
            "2.3.7",
            &shell_en_to_zh,
            dict,
        )
        .unwrap();
        let ion_zh: Map<String, Value> =
            serde_json::from_slice(ion_arc.get_file("i18n/zh-Hans.json").unwrap()).unwrap();
        assert_eq!(ion_zh.get("sharedKey").and_then(Value::as_str), Some("休眠"));
        assert_eq!(ion_zh.get("mKey").and_then(Value::as_str), Some("极致"));
        assert_eq!(
            ion_zh.get("UJPHMm0vsr").and_then(Value::as_str),
            Some("修改后批准")
        );
    }
}
