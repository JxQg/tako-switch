use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const DEFAULT_PROVIDER_CONFIG: &str = include_str!("../../config/providers.json");
pub const PLATFORM_CODEX: &str = "codex";
pub const PLATFORM_CLAUDE: &str = "claude";
pub const PLATFORM_ORDER: [&str; 2] = [PLATFORM_CODEX, PLATFORM_CLAUDE];
pub const WRITER_CODEX_CONFIG_TOML: &str = "codexConfigToml";
pub const WRITER_CLAUDE_SETTINGS_JSON: &str = "claudeSettingsJson";

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub default_provider_id: String,
    pub providers: Vec<ProviderDefinition>,
    pub source: String,
    pub warning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalogFile {
    pub default_provider_id: String,
    pub providers: Vec<ProviderDefinition>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDefinition {
    pub id: String,
    pub name: String,
    pub account: ProviderAccount,
    pub platforms: HashMap<String, PlatformDefinition>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    pub label: String,
    pub login_status_label: String,
    pub login_description: String,
    pub auth_service_url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlatformDefinition {
    pub enabled: bool,
    pub defaults: PlatformDefaults,
    pub rules: PlatformRules,
    pub writer: PlatformWriter,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlatformDefaults {
    pub base_url: String,
    pub model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlatformRules {
    #[serde(default)]
    pub base_url: Option<BaseUrlRules>,
    #[serde(default)]
    pub model: Option<ModelRules>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BaseUrlRules {
    #[serde(default)]
    pub forbid_path_suffixes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRules {
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlatformWriter {
    pub kind: String,
    pub bindings: HashMap<String, WriterBinding>,
    pub constants: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WriterBinding {
    pub storage: String,
    pub name: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInput {
    pub provider_id: String,
    pub api_key: String,
    pub platforms: ConfigPlatforms,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPlatforms {
    #[serde(default)]
    pub codex: Option<PlatformConfigInput>,
    #[serde(default)]
    pub claude: Option<PlatformConfigInput>,
}

impl ConfigPlatforms {
    pub fn get(&self, platform_id: &str) -> Option<&PlatformConfigInput> {
        match platform_id {
            PLATFORM_CODEX => self.codex.as_ref(),
            PLATFORM_CLAUDE => self.claude.as_ref(),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlatformConfigInput {
    pub enabled: bool,
    pub base_url: String,
    pub model: Option<String>,
    #[serde(default)]
    pub options: PlatformOptionsInput,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlatformOptionsInput {
    #[serde(default)]
    pub sandbox_mode: Option<String>,
    #[serde(default)]
    pub approval_policy: Option<String>,
    #[serde(default)]
    pub windows_sandbox: Option<String>,
    #[serde(default)]
    pub default_permissions: Option<String>,
    #[serde(default)]
    pub features: CodexFeatureOptionsInput,
    #[serde(default)]
    pub permissions_default_mode: Option<String>,
    #[serde(default)]
    pub skip_dangerous_mode_permission_prompt: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexFeatureOptionsInput {
    #[serde(default)]
    pub js_repl: Option<bool>,
    #[serde(default)]
    pub unified_exec: Option<bool>,
    #[serde(default)]
    pub shell_snapshot: Option<bool>,
    #[serde(default)]
    pub memories: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct NormalizedInput {
    pub api_key: String,
    pub platforms: Vec<NormalizedPlatformInput>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NormalizedPlatformInput {
    pub id: String,
    pub base_url: String,
    pub model: Option<String>,
    pub options: PlatformOptionsInput,
    pub definition: PlatformDefinition,
}
