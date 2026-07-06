use super::ApprovalsReviewer;
use super::AskForApproval;
use super::SandboxMode;
use super::shared::default_enabled;
pub use codex_config_types::ConfigLayer;
pub use codex_config_types::ConfigLayerMetadata;
pub use codex_config_types::ConfigLayerSource;
use codex_experimental_api_macros::ExperimentalApi;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::ForcedLoginMethod;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::Verbosity;
use protocol::config_types::WebSearchMode;
use protocol::config_types::WebSearchToolConfig;
use protocol::openai_models::ReasoningEffort;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct SandboxWorkspaceWrite {
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub exclude_tmpdir_env_var: bool,
    #[serde(default)]
    pub exclude_slash_tmp: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolsV2 {
    pub web_search: Option<WebSearchToolConfig>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ProfileV2 {
    pub model: Option<String>,
    pub model_provider: Option<String>,
    #[experimental(nested)]
    pub approval_policy: Option<AskForApproval>,
    /// [UNSTABLE] Optional profile-level override for where approval requests
    /// are routed for review. If omitted, the enclosing config default is
    /// used.
    #[experimental("config/read.approvalsReviewer")]
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub service_tier: Option<String>,
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    pub model_verbosity: Option<Verbosity>,
    pub web_search: Option<WebSearchMode>,
    pub tools: Option<ToolsV2>,
    pub chatgpt_base_url: Option<String>,
    #[serde(default, flatten)]
    pub additional: HashMap<String, JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AnalyticsConfig {
    pub enabled: Option<bool>,
    #[serde(default, flatten)]
    pub additional: HashMap<String, JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum AppToolApproval {
    Auto,
    Prompt,
    Approve,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AppsDefaultConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_enabled")]
    pub destructive_enabled: bool,
    #[serde(default = "default_enabled")]
    pub open_world_enabled: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AppToolConfig {
    pub enabled: Option<bool>,
    pub approval_mode: Option<AppToolApproval>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AppToolsConfig {
    #[serde(default, flatten)]
    pub tools: HashMap<String, AppToolConfig>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AppConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub destructive_enabled: Option<bool>,
    pub open_world_enabled: Option<bool>,
    pub default_tools_approval_mode: Option<AppToolApproval>,
    pub default_tools_enabled: Option<bool>,
    pub tools: Option<AppToolsConfig>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct AppsConfig {
    #[serde(default, rename = "_default")]
    pub default: Option<AppsDefaultConfig>,
    #[serde(default, flatten)]
    pub apps: HashMap<String, AppConfig>,
}

/// Backward-compatible API shape for ChatGPT workspace login restrictions.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ForcedChatgptWorkspaceIds {
    Single(String),
    Multiple(Vec<String>),
}

impl ForcedChatgptWorkspaceIds {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(value) => vec![value],
            Self::Multiple(values) => values,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct Config {
    pub model: Option<String>,
    pub review_model: Option<String>,
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub model_provider: Option<String>,
    #[experimental(nested)]
    pub approval_policy: Option<AskForApproval>,
    /// [UNSTABLE] Optional default for where approval requests are routed for
    /// review.
    #[experimental("config/read.approvalsReviewer")]
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub sandbox_mode: Option<SandboxMode>,
    pub sandbox_workspace_write: Option<SandboxWorkspaceWrite>,
    pub forced_chatgpt_workspace_id: Option<ForcedChatgptWorkspaceIds>,
    pub forced_login_method: Option<ForcedLoginMethod>,
    pub web_search: Option<WebSearchMode>,
    pub tools: Option<ToolsV2>,
    pub profile: Option<String>,
    #[experimental(nested)]
    #[serde(default)]
    pub profiles: HashMap<String, ProfileV2>,
    pub instructions: Option<String>,
    pub developer_instructions: Option<String>,
    pub compact_prompt: Option<String>,
    pub model_reasoning_effort: Option<ReasoningEffort>,
    pub model_reasoning_summary: Option<ReasoningSummary>,
    pub model_verbosity: Option<Verbosity>,
    pub service_tier: Option<String>,
    pub analytics: Option<AnalyticsConfig>,
    #[experimental("config/read.apps")]
    #[serde(default)]
    pub apps: Option<AppsConfig>,
    pub desktop: Option<HashMap<String, JsonValue>>,
    #[serde(default, flatten)]
    pub additional: HashMap<String, JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum MergeStrategy {
    Replace,
    Upsert,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum WriteStatus {
    Ok,
    OkOverridden,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct OverriddenMetadata {
    pub message: String,
    pub overriding_layer: ConfigLayerMetadata,
    pub effective_value: JsonValue,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigWriteResponse {
    pub status: WriteStatus,
    pub version: String,
    /// Canonical path to the config file that was written.
    pub file_path: AbsolutePathBuf,
    pub overridden_metadata: Option<OverriddenMetadata>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ConfigWriteErrorCode {
    ConfigLayerReadonly,
    ConfigVersionConflict,
    ConfigValidationError,
    ConfigPathNotFound,
    ConfigSchemaUnknownKey,
    UserLayerNotFound,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigReadParams {
    #[serde(default)]
    pub include_layers: bool,
    /// Optional working directory to resolve project config layers. If specified,
    /// return the effective config as seen from that directory (i.e., including any
    /// project layers between `cwd` and the project/repo root).
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigReadResponse {
    #[experimental(nested)]
    pub config: Config,
    pub origins: HashMap<String, ConfigLayerMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<Vec<ConfigLayer>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigRequirements {
    #[experimental(nested)]
    pub allowed_approval_policies: Option<Vec<AskForApproval>>,
    #[experimental("configRequirements/read.allowedApprovalsReviewers")]
    pub allowed_approvals_reviewers: Option<Vec<ApprovalsReviewer>>,
    pub allowed_sandbox_modes: Option<Vec<SandboxMode>>,
    pub allowed_web_search_modes: Option<Vec<WebSearchMode>>,
    pub allow_managed_hooks_only: Option<bool>,
    pub feature_requirements: Option<BTreeMap<String, bool>>,
    #[experimental("configRequirements/read.hooks")]
    pub hooks: Option<ManagedHooksRequirements>,
    pub enforce_residency: Option<ResidencyRequirement>,
    #[experimental("configRequirements/read.network")]
    pub network: Option<NetworkRequirements>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ManagedHooksRequirements {
    pub managed_dir: Option<PathBuf>,
    pub windows_managed_dir: Option<PathBuf>,
    #[serde(rename = "PreToolUse")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PreToolUse"))]
    pub pre_tool_use: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "PermissionRequest")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PermissionRequest"))]
    pub permission_request: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "PostToolUse")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PostToolUse"))]
    pub post_tool_use: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "PreCompact")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PreCompact"))]
    pub pre_compact: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "PostCompact")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PostCompact"))]
    pub post_compact: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "SessionStart")]
    #[cfg_attr(feature = "schema-export", ts(rename = "SessionStart"))]
    pub session_start: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "UserPromptSubmit")]
    #[cfg_attr(feature = "schema-export", ts(rename = "UserPromptSubmit"))]
    pub user_prompt_submit: Vec<ConfiguredHookMatcherGroup>,
    #[serde(rename = "Stop")]
    #[cfg_attr(feature = "schema-export", ts(rename = "Stop"))]
    pub stop: Vec<ConfiguredHookMatcherGroup>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfiguredHookMatcherGroup {
    pub matcher: Option<String>,
    pub hooks: Vec<ConfiguredHookHandler>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
pub enum ConfiguredHookHandler {
    #[serde(rename = "command")]
    #[cfg_attr(feature = "schema-export", ts(rename = "command"))]
    Command {
        command: String,
        #[serde(rename = "commandWindows")]
        #[cfg_attr(feature = "schema-export", ts(rename = "commandWindows"))]
        command_windows: Option<String>,
        #[serde(rename = "timeoutSec")]
        #[cfg_attr(feature = "schema-export", ts(rename = "timeoutSec"))]
        timeout_sec: Option<u64>,
        r#async: bool,
        #[serde(rename = "statusMessage")]
        #[cfg_attr(feature = "schema-export", ts(rename = "statusMessage"))]
        status_message: Option<String>,
    },
    #[serde(rename = "prompt")]
    #[cfg_attr(feature = "schema-export", ts(rename = "prompt"))]
    Prompt {},
    #[serde(rename = "agent")]
    #[cfg_attr(feature = "schema-export", ts(rename = "agent"))]
    Agent {},
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct NetworkRequirements {
    pub enabled: Option<bool>,
    pub http_port: Option<u16>,
    pub socks_port: Option<u16>,
    pub allow_upstream_proxy: Option<bool>,
    pub dangerously_allow_non_loopback_proxy: Option<bool>,
    pub dangerously_allow_all_unix_sockets: Option<bool>,
    /// Canonical network permission map for `experimental_network`.
    pub domains: Option<BTreeMap<String, NetworkDomainPermission>>,
    /// When true, only managed allowlist entries are respected while managed
    /// network enforcement is active.
    pub managed_allowed_domains_only: Option<bool>,
    /// Legacy compatibility view derived from `domains`.
    pub allowed_domains: Option<Vec<String>>,
    /// Legacy compatibility view derived from `domains`.
    pub denied_domains: Option<Vec<String>>,
    /// Canonical unix socket permission map for `experimental_network`.
    pub unix_sockets: Option<BTreeMap<String, NetworkUnixSocketPermission>>,
    /// Legacy compatibility view derived from `unix_sockets`.
    pub allow_unix_sockets: Option<Vec<String>>,
    pub allow_local_binding: Option<bool>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum NetworkDomainPermission {
    Allow,
    Deny,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum NetworkUnixSocketPermission {
    Allow,
    None,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ResidencyRequirement {
    Us,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigRequirementsReadResponse {
    /// Null if no requirements are configured (e.g. no requirements.toml/MDM entries).
    #[experimental(nested)]
    pub requirements: Option<ConfigRequirements>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ExternalAgentConfigMigrationItemType {
    #[serde(rename = "AGENTS_MD")]
    #[cfg_attr(feature = "schema-export", ts(rename = "AGENTS_MD"))]
    AgentsMd,
    #[serde(rename = "CONFIG")]
    #[cfg_attr(feature = "schema-export", ts(rename = "CONFIG"))]
    Config,
    #[serde(rename = "SKILLS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "SKILLS"))]
    Skills,
    #[serde(rename = "PLUGINS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "PLUGINS"))]
    Plugins,
    #[serde(rename = "MCP_SERVER_CONFIG")]
    #[cfg_attr(feature = "schema-export", ts(rename = "MCP_SERVER_CONFIG"))]
    McpServerConfig,
    #[serde(rename = "SUBAGENTS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "SUBAGENTS"))]
    Subagents,
    #[serde(rename = "HOOKS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "HOOKS"))]
    Hooks,
    #[serde(rename = "COMMANDS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "COMMANDS"))]
    Commands,
    #[serde(rename = "SESSIONS")]
    #[cfg_attr(feature = "schema-export", ts(rename = "SESSIONS"))]
    Sessions,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct PluginsMigration {
    #[serde(rename = "marketplaceName")]
    #[cfg_attr(feature = "schema-export", ts(rename = "marketplaceName"))]
    pub marketplace_name: String,
    #[serde(rename = "pluginNames")]
    #[cfg_attr(feature = "schema-export", ts(rename = "pluginNames"))]
    pub plugin_names: Vec<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct SessionMigration {
    pub path: PathBuf,
    pub cwd: PathBuf,
    pub title: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerMigration {
    pub name: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct HookMigration {
    pub name: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct SubagentMigration {
    pub name: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandMigration {
    pub name: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MigrationDetails {
    #[serde(default)]
    pub plugins: Vec<PluginsMigration>,
    #[serde(default)]
    pub sessions: Vec<SessionMigration>,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerMigration>,
    #[serde(default)]
    pub hooks: Vec<HookMigration>,
    #[serde(default)]
    pub subagents: Vec<SubagentMigration>,
    #[serde(default)]
    pub commands: Vec<CommandMigration>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigMigrationItem {
    pub item_type: ExternalAgentConfigMigrationItemType,
    pub description: String,
    /// Null or empty means home-scoped migration; non-empty means repo-scoped migration.
    pub cwd: Option<PathBuf>,
    pub details: Option<MigrationDetails>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigDetectResponse {
    pub items: Vec<ExternalAgentConfigMigrationItem>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigDetectParams {
    /// If true, include detection under the user's home (~/.claude, ~/.codex, etc.).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_home: bool,
    /// Zero or more working directories to include for repo-scoped detection.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwds: Option<Vec<PathBuf>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigImportParams {
    pub migration_items: Vec<ExternalAgentConfigMigrationItem>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigImportResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ExternalAgentConfigImportCompletedNotification {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigValueWriteParams {
    pub key_path: String,
    pub value: JsonValue,
    pub merge_strategy: MergeStrategy,
    /// Path to the config file to write; defaults to the user's `config.toml` when omitted.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub file_path: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub expected_version: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigBatchWriteParams {
    pub edits: Vec<ConfigEdit>,
    /// Path to the config file to write; defaults to the user's `config.toml` when omitted.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub file_path: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub expected_version: Option<String>,
    /// When true, hot-reload the updated user config into all loaded threads after writing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reload_user_config: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigEdit {
    pub key_path: String,
    pub value: JsonValue,
    pub merge_strategy: MergeStrategy,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TextPosition {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (in Unicode scalar values).
    pub column: usize,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ConfigWarningNotification {
    /// Concise summary of the warning.
    pub summary: String,
    /// Optional extra guidance or error details.
    pub details: Option<String>,
    /// Optional path to the config file that triggered the warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub path: Option<String>,
    /// Optional range for the error location inside the config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub range: Option<TextRange>,
}
