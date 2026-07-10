use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_installed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_installed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingConfig {
    pub target: String,
    pub path: String,
    pub exists: bool,
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedConfigs {
    pub codex: ExistingConfig,
    pub claude: ExistingConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub target: String,
    pub path: String,
    pub exists: bool,
    pub backup_path: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvPreview {
    pub name: String,
    pub masked_value: String,
    pub note: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    pub files: Vec<FilePreview>,
    pub env_updates: Vec<EnvPreview>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFile {
    pub target: String,
    pub path: String,
    pub backup_path: String,
    pub created: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub files: Vec<AppliedFile>,
    pub env_updates: Vec<String>,
    pub tools: Vec<ToolStatus>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub target: String,
    pub path: String,
    pub restored_from: String,
    pub deleted_target: bool,
}
