use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use toml_edit::{value, DocumentMut, Item, Table};
use url::Url;

const CODEX_ENV_KEY: &str = "TAKO_CODEX_API_KEY";
const CODEX_PROVIDER_ID: &str = "tako_proxy";
const MISSING_SENTINEL: &str = "TAKO_BACKUP_ORIGINAL_FILE_MISSING\n";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInput {
    gateway_base_url: String,
    api_key: String,
    codex_model: Option<String>,
    claude_model: Option<String>,
    configure_codex: bool,
    configure_claude: bool,
}

#[derive(Debug, Clone)]
struct NormalizedInput {
    gateway_base_url: String,
    api_key: String,
    codex_model: Option<String>,
    claude_model: Option<String>,
    configure_codex: bool,
    configure_claude: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    name: String,
    installed: bool,
    version: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingConfig {
    target: String,
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedConfigs {
    codex: ExistingConfig,
    claude: ExistingConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    target: String,
    path: String,
    exists: bool,
    backup_path: String,
    before: String,
    after: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvPreview {
    name: String,
    masked_value: String,
    note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    files: Vec<FilePreview>,
    env_updates: Vec<EnvPreview>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFile {
    target: String,
    path: String,
    backup_path: String,
    created: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    files: Vec<AppliedFile>,
    env_updates: Vec<String>,
    tools: Vec<ToolStatus>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    target: String,
    path: String,
    restored_from: String,
    deleted_target: bool,
}

#[tauri::command]
fn detect_tools() -> Vec<ToolStatus> {
    vec![
        detect_tool("Codex", tool_command_candidates("codex")),
        detect_tool("Claude Code", tool_command_candidates("claude")),
    ]
}

#[tauri::command]
fn load_current_configs() -> Result<LoadedConfigs, String> {
    let codex_path = codex_config_path()?;
    let claude_path = claude_settings_path()?;

    Ok(LoadedConfigs {
        codex: read_existing_config("codex", &codex_path, redact_plain_text),
        claude: read_existing_config("claude", &claude_path, redact_json_text),
    })
}

#[tauri::command]
fn preview_changes(input: ConfigInput) -> Result<PreviewResult, String> {
    let normalized = validate_input(input)?;
    let mut files = Vec::new();
    let mut env_updates = Vec::new();
    let warnings = if normalized.configure_codex {
        profile_warnings()
    } else {
        Vec::new()
    };

    if normalized.configure_codex {
        let path = codex_config_path()?;
        let before = fs::read_to_string(&path).unwrap_or_default();
        let after = merge_codex_config(
            &before,
            normalized.codex_model.as_deref().unwrap_or_default(),
            &normalized.gateway_base_url,
        )?;

        files.push(FilePreview {
            target: "codex".to_string(),
            path: display_path(&path),
            exists: path.exists(),
            backup_path: display_path(&make_backup_path(&path)),
            before: redact_plain_text(before),
            after: redact_plain_text(after),
        });

        env_updates.push(EnvPreview {
            name: CODEX_ENV_KEY.to_string(),
            masked_value: mask_secret(&normalized.api_key),
            note: codex_env_note(),
        });
    }

    if normalized.configure_claude {
        let path = claude_settings_path()?;
        let before = fs::read_to_string(&path).unwrap_or_default();
        let after = merge_claude_settings(
            &before,
            &normalized.gateway_base_url,
            &normalized.api_key,
            normalized.claude_model.as_deref(),
        )?;

        files.push(FilePreview {
            target: "claude".to_string(),
            path: display_path(&path),
            exists: path.exists(),
            backup_path: display_path(&make_backup_path(&path)),
            before: redact_json_text(before),
            after: redact_json_text(after),
        });
    }

    Ok(PreviewResult {
        files,
        env_updates,
        warnings,
    })
}

#[tauri::command]
fn apply_configs(input: ConfigInput) -> Result<ApplyResult, String> {
    let normalized = validate_input(input)?;
    let mut files = Vec::new();
    let mut env_updates = Vec::new();
    let warnings = if normalized.configure_codex {
        profile_warnings()
    } else {
        Vec::new()
    };

    if normalized.configure_codex {
        env_updates.push(write_user_env_var(CODEX_ENV_KEY, &normalized.api_key)?);

        let path = codex_config_path()?;
        let before = fs::read_to_string(&path).unwrap_or_default();
        let after = merge_codex_config(
            &before,
            normalized.codex_model.as_deref().unwrap_or_default(),
            &normalized.gateway_base_url,
        )?;
        files.push(write_config_file("codex", &path, &after)?);
    }

    if normalized.configure_claude {
        let path = claude_settings_path()?;
        let before = fs::read_to_string(&path).unwrap_or_default();
        let after = merge_claude_settings(
            &before,
            &normalized.gateway_base_url,
            &normalized.api_key,
            normalized.claude_model.as_deref(),
        )?;
        files.push(write_config_file("claude", &path, &after)?);
    }

    Ok(ApplyResult {
        files,
        env_updates,
        tools: detect_tools(),
        warnings,
    })
}

#[tauri::command]
fn restore_backup(target: String, backup_path: String) -> Result<RestoreResult, String> {
    let target_path = resolve_restore_target(&target)?;
    let backup_path = PathBuf::from(backup_path);
    if !backup_path.exists() {
        return Err(format!(
            "Backup file does not exist: {}",
            display_path(&backup_path)
        ));
    }

    let backup_content =
        fs::read_to_string(&backup_path).map_err(|err| format!("Failed to read backup: {err}"))?;

    if backup_content == MISSING_SENTINEL {
        if target_path.exists() {
            fs::remove_file(&target_path)
                .map_err(|err| format!("Failed to remove restored target: {err}"))?;
        }

        return Ok(RestoreResult {
            target,
            path: display_path(&target_path),
            restored_from: display_path(&backup_path),
            deleted_target: true,
        });
    }

    write_file_atomic(&target_path, &backup_content)
        .map_err(|err| format!("Failed to restore backup: {err}"))?;

    Ok(RestoreResult {
        target,
        path: display_path(&target_path),
        restored_from: display_path(&backup_path),
        deleted_target: false,
    })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            detect_tools,
            load_current_configs,
            preview_changes,
            apply_configs,
            restore_backup
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tako Switch");
}

fn validate_input(input: ConfigInput) -> Result<NormalizedInput, String> {
    if !input.configure_codex && !input.configure_claude {
        return Err("Please select at least Codex or Claude Code.".to_string());
    }

    let gateway_base_url = input
        .gateway_base_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    let parsed = Url::parse(&gateway_base_url)
        .map_err(|_| "Gateway URL must be a valid http:// or https:// address.".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Gateway URL must start with http:// or https://.".to_string());
    }

    let api_key = input.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API key cannot be empty.".to_string());
    }

    let codex_model = trim_optional(input.codex_model);
    if input.configure_codex && codex_model.is_none() {
        return Err("Codex model cannot be empty when Codex is selected.".to_string());
    }

    Ok(NormalizedInput {
        gateway_base_url,
        api_key,
        codex_model,
        claude_model: trim_optional(input.claude_model),
        configure_codex: input.configure_codex,
        configure_claude: input.configure_claude,
    })
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn codex_config_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(path).join("config.toml"));
    }
    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn claude_settings_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude").join("settings.json"))
}

fn home_dir() -> Result<PathBuf, String> {
    let candidates = [
        env::var_os("USERPROFILE"),
        env::var_os("HOME"),
        windows_home_from_parts(),
    ];

    candidates
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| "Could not determine the user home directory.".to_string())
}

fn windows_home_from_parts() -> Option<OsString> {
    let drive = env::var_os("HOMEDRIVE")?;
    let path = env::var_os("HOMEPATH")?;
    let mut home = drive;
    home.push(path);
    Some(home)
}

fn read_existing_config<F>(target: &str, path: &Path, redact: F) -> ExistingConfig
where
    F: Fn(String) -> String,
{
    let content = fs::read_to_string(path).unwrap_or_default();
    ExistingConfig {
        target: target.to_string(),
        path: display_path(path),
        exists: path.exists(),
        content: redact(content),
    }
}

fn tool_command_candidates(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{command}.cmd"),
            format!("{command}.exe"),
            command.to_string(),
        ]
    } else {
        vec![command.to_string()]
    }
}

fn detect_tool(name: &str, commands: Vec<String>) -> ToolStatus {
    let mut errors = Vec::new();

    for command in &commands {
        match command_version(command) {
            Ok((version, None)) => {
                return ToolStatus {
                    name: name.to_string(),
                    installed: true,
                    version,
                    error: None,
                };
            }
            Ok((version, Some(error))) => {
                let details = version
                    .map(|value| format!("{error}; output: {value}"))
                    .unwrap_or(error);
                errors.push(format!("{command}: {details}"));
            }
            Err(error) => errors.push(format!("{command}: {error}")),
        }
    }

    ToolStatus {
        name: name.to_string(),
        installed: false,
        version: None,
        error: Some(format!(
            "Tried {}. Last error: {}",
            commands.join(", "),
            errors
                .last()
                .cloned()
                .unwrap_or_else(|| "no candidates were available".to_string())
        )),
    }
}

fn command_version(command: &str) -> Result<(Option<String>, Option<String>), String> {
    let mut process = Command::new(command);
    process.arg("--version");
    configure_detection_command(&mut process);

    match process.output() {
        Ok(output) => {
            let text = if output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stderr).to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).to_string()
            };
            let version = text.lines().next().map(|line| line.trim().to_string());
            if output.status.success() {
                Ok((version, None))
            } else {
                Ok((
                    version,
                    Some(format!("Command exited with {}", output.status)),
                ))
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

#[cfg(windows)]
fn configure_detection_command(command: &mut Command) {
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn configure_detection_command(_command: &mut Command) {}

fn merge_codex_config(
    existing: &str,
    codex_model: &str,
    gateway_base_url: &str,
) -> Result<String, String> {
    let mut document = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .map_err(|err| format!("Existing Codex config is not valid TOML: {err}"))?
    };

    document["model"] = value(codex_model);
    document["model_provider"] = value(CODEX_PROVIDER_ID);

    let providers = document
        .as_table_mut()
        .entry("model_providers")
        .or_insert(Item::Table(Table::new()));
    if !providers.is_table() {
        *providers = Item::Table(Table::new());
    }

    let providers_table = providers.as_table_mut().expect("table checked");
    let provider = providers_table
        .entry(CODEX_PROVIDER_ID)
        .or_insert(Item::Table(Table::new()));
    if !provider.is_table() {
        *provider = Item::Table(Table::new());
    }

    let provider_table = provider.as_table_mut().expect("table checked");
    provider_table.insert("name", value("Tako LLM Gateway"));
    provider_table.insert("base_url", value(gateway_base_url));
    provider_table.insert("env_key", value(CODEX_ENV_KEY));
    provider_table.insert("wire_api", value("responses"));

    Ok(ensure_trailing_newline(document.to_string()))
}

fn merge_claude_settings(
    existing: &str,
    gateway_base_url: &str,
    api_key: &str,
    claude_model: Option<&str>,
) -> Result<String, String> {
    let mut root = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(existing)
            .map_err(|err| format!("Existing Claude settings are not valid JSON: {err}"))?
    };

    if !root.is_object() {
        root = json!({});
    }

    let object = root.as_object_mut().expect("object checked");
    let env_value = object
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !env_value.is_object() {
        *env_value = Value::Object(Map::new());
    }

    let env_object = env_value.as_object_mut().expect("object checked");
    env_object.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        Value::String(gateway_base_url.to_string()),
    );
    env_object.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        Value::String(api_key.to_string()),
    );
    if let Some(model) = claude_model {
        env_object.insert(
            "ANTHROPIC_MODEL".to_string(),
            Value::String(model.to_string()),
        );
    } else {
        env_object.remove("ANTHROPIC_MODEL");
    }

    serde_json::to_string_pretty(&root)
        .map(ensure_trailing_newline)
        .map_err(|err| format!("Failed to render Claude settings JSON: {err}"))
}

fn write_config_file(target: &str, path: &Path, content: &str) -> Result<AppliedFile, String> {
    let existed = path.exists();
    let backup_path = make_backup_path(path);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create config folder: {err}"))?;
    }

    if existed {
        fs::copy(path, &backup_path).map_err(|err| {
            format!(
                "Failed to create backup {}: {err}",
                display_path(&backup_path)
            )
        })?;
    } else {
        fs::write(&backup_path, MISSING_SENTINEL)
            .map_err(|err| format!("Failed to create missing-file backup marker: {err}"))?;
    }

    write_file_atomic(path, content).map_err(|err| {
        let _ = restore_from_backup(path, &backup_path);
        format!("Failed to write {} config: {err}", target)
    })?;

    Ok(AppliedFile {
        target: target.to_string(),
        path: display_path(path),
        backup_path: display_path(&backup_path),
        created: !existed,
    })
}

fn write_file_atomic(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temp_path = path.with_file_name(format!(
        ".{file_name}.tako-tmp-{}",
        Local::now().format("%Y%m%d%H%M%S%3f")
    ));

    fs::write(&temp_path, content)?;

    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            if path.exists() {
                fs::remove_file(path)?;
                fs::rename(&temp_path, path)
            } else {
                Err(first_error)
            }
        }
    }
}

fn restore_from_backup(path: &Path, backup_path: &Path) -> io::Result<()> {
    let content = fs::read_to_string(backup_path)?;
    if content == MISSING_SENTINEL {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    write_file_atomic(path, &content)
}

fn make_backup_path(path: &Path) -> PathBuf {
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    path.with_file_name(format!("{file_name}.tako-backup-{timestamp}"))
}

fn resolve_restore_target(target: &str) -> Result<PathBuf, String> {
    match target {
        "codex" => codex_config_path(),
        "claude" => claude_settings_path(),
        value if !value.trim().is_empty() => Ok(PathBuf::from(value)),
        _ => Err("Restore target cannot be empty.".to_string()),
    }
}

#[cfg(windows)]
fn write_user_env_var(name: &str, value: &str) -> Result<String, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (environment, _) = hkcu
        .create_subkey("Environment")
        .map_err(|err| format!("Failed to open user environment registry key: {err}"))?;
    environment
        .set_value(name, &value)
        .map_err(|err| format!("Failed to write user environment variable: {err}"))?;
    Ok(format!(
        "{name} has been written to the Windows user environment. Open a new terminal before running Codex."
    ))
}

#[cfg(not(windows))]
fn write_user_env_var(name: &str, value: &str) -> Result<String, String> {
    let mut touched = Vec::new();
    for path in profile_paths()? {
        upsert_env_block(&path, name, value)?;
        touched.push(display_path(&path));
    }

    Ok(format!(
        "{name} has been written to {}. Open a new terminal before running Codex.",
        touched.join(", ")
    ))
}

#[cfg(not(windows))]
fn profile_paths() -> Result<Vec<PathBuf>, String> {
    let home = home_dir()?;
    let mut paths = vec![home.join(".profile")];
    if cfg!(target_os = "macos") {
        paths.push(home.join(".zshrc"));
    }
    Ok(paths)
}

#[cfg(not(windows))]
fn upsert_env_block(path: &Path, name: &str, value: &str) -> Result<(), String> {
    let start = format!("# >>> Tako Switch: {name}");
    let end = format!("# <<< Tako Switch: {name}");
    let block = format!("{start}\nexport {name}={}\n{end}\n", shell_quote(value));
    let existing = fs::read_to_string(path).unwrap_or_default();
    let updated = replace_marked_block(&existing, &start, &end, &block);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create profile folder: {err}"))?;
    }
    write_file_atomic(path, &updated)
        .map_err(|err| format!("Failed to update shell profile: {err}"))
}

#[cfg(not(windows))]
fn replace_marked_block(existing: &str, start: &str, end: &str, block: &str) -> String {
    if let Some(start_index) = existing.find(start) {
        if let Some(relative_end) = existing[start_index..].find(end) {
            let end_index = start_index + relative_end + end.len();
            let mut updated = String::new();
            updated.push_str(existing[..start_index].trim_end());
            updated.push_str("\n\n");
            updated.push_str(block.trim_end());
            updated.push_str("\n");
            updated.push_str(existing[end_index..].trim_start_matches(&['\r', '\n'][..]));
            return ensure_trailing_newline(updated);
        }
    }

    let mut updated = existing.trim_end().to_string();
    if !updated.is_empty() {
        updated.push_str("\n\n");
    }
    updated.push_str(block.trim_end());
    updated.push('\n');
    updated
}

#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn codex_env_note() -> String {
    if cfg!(windows) {
        "Written to the Windows user environment; new terminals pick it up.".to_string()
    } else if cfg!(target_os = "macos") {
        "Written to ~/.profile and ~/.zshrc; new terminals pick it up.".to_string()
    } else {
        "Written to ~/.profile; new login shells pick it up.".to_string()
    }
}

fn profile_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    if !cfg!(windows) {
        warnings.push(
            "Codex reads TAKO_CODEX_API_KEY from the shell environment; open a new terminal after applying."
                .to_string(),
        );
    }
    warnings
}

fn redact_json_text(content: String) -> String {
    if content.trim().is_empty() {
        return content;
    }

    match serde_json::from_str::<Value>(&content) {
        Ok(mut value) => {
            redact_json_value(&mut value);
            serde_json::to_string_pretty(&value)
                .map(ensure_trailing_newline)
                .unwrap_or_else(|_| redact_plain_text(content))
        }
        Err(_) => redact_plain_text(content),
    }
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map {
                let lowered = key.to_ascii_lowercase();
                if lowered.contains("token")
                    || lowered.contains("api_key")
                    || lowered.contains("apikey")
                    || lowered.contains("secret")
                {
                    if let Some(raw) = item.as_str() {
                        *item = Value::String(mask_secret(raw));
                    }
                } else {
                    redact_json_value(item);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        _ => {}
    }
}

fn redact_plain_text(content: String) -> String {
    content
        .lines()
        .map(|line| {
            let lowered = line.to_ascii_lowercase();
            if lowered.contains("token")
                || lowered.contains("api_key")
                || lowered.contains("apikey")
                || lowered.contains("secret")
            {
                mask_assignment_line(line)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mask_assignment_line(line: &str) -> String {
    if let Some((left, right)) = line.split_once('=') {
        format!(
            "{}= {}",
            left.trim_end(),
            mask_secret(right.trim().trim_matches('"'))
        )
    } else {
        line.to_string()
    }
}

fn mask_secret(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "****".to_string();
    }
    let start: String = chars.iter().take(4).collect();
    let end: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{start}****{end}")
}

fn ensure_trailing_newline(mut value: String) -> String {
    if !value.ends_with('\n') {
        value.push('\n');
    }
    value
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_tool_candidates_prefer_cmd_then_exe() {
        assert_eq!(
            tool_command_candidates("codex"),
            vec![
                "codex.cmd".to_string(),
                "codex.exe".to_string(),
                "codex".to_string(),
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_tool_candidates_use_plain_command() {
        assert_eq!(tool_command_candidates("codex"), vec!["codex".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn detect_tool_tries_later_windows_candidates() {
        let test_dir = env::temp_dir().join(format!(
            "tako-switch-detect-test-{}",
            Local::now().format("%Y%m%d%H%M%S%3f")
        ));
        fs::create_dir_all(&test_dir).unwrap();
        let command_path = test_dir.join("fake-tool.cmd");
        fs::write(&command_path, "@echo off\r\necho fake-tool 1.2.3\r\n").unwrap();

        let status = detect_tool(
            "Fake Tool",
            vec![
                test_dir
                    .join("missing-tool.cmd")
                    .to_string_lossy()
                    .to_string(),
                command_path.to_string_lossy().to_string(),
            ],
        );

        let _ = fs::remove_file(&command_path);
        let _ = fs::remove_dir(&test_dir);

        assert!(status.installed);
        assert_eq!(status.version, Some("fake-tool 1.2.3".to_string()));
        assert!(status.error.is_none());
    }

    #[test]
    fn codex_merge_preserves_existing_fields_and_is_idempotent() {
        let existing = r#"
approval_policy = "on-request"

[model_providers.other]
name = "Other"
"#;
        let first = merge_codex_config(existing, "gpt-5.4", "http://127.0.0.1:3000/v1").unwrap();
        let second = merge_codex_config(&first, "gpt-5.4", "http://127.0.0.1:3000/v1").unwrap();

        assert_eq!(first, second);
        assert!(first.contains("approval_policy = \"on-request\""));
        assert!(first.contains("model = \"gpt-5.4\""));
        assert!(first.contains("model_provider = \"tako_proxy\""));
        assert!(first.contains("env_key = \"TAKO_CODEX_API_KEY\""));
    }

    #[test]
    fn claude_merge_preserves_existing_settings_and_env() {
        let existing = r#"{
  "theme": "dark",
  "env": {
    "OTHER": "1"
  }
}"#;
        let merged = merge_claude_settings(
            existing,
            "https://gateway.example.com/v1",
            "sk-test-secret",
            Some("claude-custom"),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();

        assert_eq!(value["theme"], "dark");
        assert_eq!(value["env"]["OTHER"], "1");
        assert_eq!(
            value["env"]["ANTHROPIC_BASE_URL"],
            "https://gateway.example.com/v1"
        );
        assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-test-secret");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-custom");
    }

    #[test]
    fn claude_merge_removes_empty_model() {
        let existing = r#"{"env":{"ANTHROPIC_MODEL":"old"}}"#;
        let merged =
            merge_claude_settings(existing, "http://localhost:3000/v1", "sk-test", None).unwrap();
        let value: Value = serde_json::from_str(&merged).unwrap();

        assert!(value["env"].get("ANTHROPIC_MODEL").is_none());
    }

    #[test]
    fn validation_rejects_empty_secret_and_bad_url() {
        let input = ConfigInput {
            gateway_base_url: "ftp://localhost".to_string(),
            api_key: "".to_string(),
            codex_model: Some("gpt-5.4".to_string()),
            claude_model: None,
            configure_codex: true,
            configure_claude: false,
        };

        assert!(validate_input(input).is_err());
    }

    #[test]
    fn secret_masking_keeps_edges_only() {
        assert_eq!(mask_secret("sk-1234567890"), "sk-1****7890");
        assert_eq!(mask_secret("short"), "****");
    }

    #[cfg(not(windows))]
    #[test]
    fn marked_block_replacement_is_idempotent() {
        let start = "# >>> Tako Switch: TAKO_CODEX_API_KEY";
        let end = "# <<< Tako Switch: TAKO_CODEX_API_KEY";
        let block = format!("{start}\nexport TAKO_CODEX_API_KEY='one'\n{end}\n");
        let first = replace_marked_block("export PATH=/bin\n", start, end, &block);
        let second = replace_marked_block(&first, start, end, &block);

        assert_eq!(first, second);
        assert!(first.contains("export PATH=/bin"));
        assert!(first.contains("TAKO_CODEX_API_KEY"));
    }
}
