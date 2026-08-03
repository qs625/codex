use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use ts_rs::TS;

mod agents;
mod apps;
mod config_lock;
mod constraint;
mod hooks;
mod mcp;
mod memories;
mod otel;
mod plugin;
mod realtime;
mod requirements;
mod sandbox;
mod skills;
mod thread_store;
mod tui_keymap;
mod ui;

pub const CONFIG_TOML_FILE: &str = "config.toml";

pub use agents::AgentRoleToml;
pub use agents::AgentsToml;
pub use apps::AppConfig;
pub use apps::AppToolConfig;
pub use apps::AppToolsConfig;
pub use apps::AppsConfigToml;
pub use apps::AppsDefaultConfig;
pub use config_lock::ConfigLockfileToml;
pub use constraint::Constrained;
pub use constraint::ConstraintError;
pub use constraint::ConstraintResult;
pub use hooks::HookEventsToml;
pub use hooks::HookHandlerConfig;
pub use hooks::HookStateToml;
pub use hooks::HooksFile;
pub use hooks::HooksToml;
pub use hooks::ManagedHooksRequirementsToml;
pub use hooks::MatcherGroup;
pub use mcp::AppToolApproval;
pub use mcp::McpServerConfig;
pub use mcp::McpServerDisabledReason;
pub use mcp::McpServerEnvVar;
pub use mcp::McpServerOAuthConfig;
pub use mcp::McpServerToolConfig;
pub use mcp::McpServerTransportConfig;
pub use mcp::RawMcpServerConfig;
pub use memories::CompactReplacementFileConfig;
pub use memories::CompactReplacementFileRole;
pub use memories::CompactReplacementFileToml;
pub use memories::DEFAULT_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT;
pub use memories::DEFAULT_MEMORIES_MAX_RAW_MEMORIES_FOR_CONSOLIDATION;
pub use memories::DEFAULT_MEMORIES_MAX_ROLLOUT_AGE_DAYS;
pub use memories::DEFAULT_MEMORIES_MAX_ROLLOUTS_PER_STARTUP;
pub use memories::DEFAULT_MEMORIES_MAX_UNUSED_DAYS;
pub use memories::DEFAULT_MEMORIES_MIN_RATE_LIMIT_REMAINING_PERCENT;
pub use memories::DEFAULT_MEMORIES_MIN_ROLLOUT_IDLE_HOURS;
pub use memories::MemoriesConfig;
pub use memories::MemoriesToml;
pub use otel::DEFAULT_OTEL_ENVIRONMENT;
pub use otel::OtelConfig;
pub use otel::OtelConfigToml;
pub use otel::OtelExporterKind;
pub use otel::OtelHttpProtocol;
pub use otel::OtelTlsConfig;
pub use otel::validate_otel_span_attributes;
pub use otel::validate_otel_tracestate_entries;
pub use otel::validate_otel_tracestate_member;
pub use plugin::MarketplaceConfig;
pub use plugin::MarketplaceSourceType;
pub use plugin::PluginConfig;
pub use plugin::PluginMcpServerConfig;
use protocol::config_types::TrustLevel;
pub use realtime::RealtimeAudioConfig;
pub use realtime::RealtimeTransport;
pub use realtime::RealtimeWsMode;
pub use requirements::RequirementSource;
pub use sandbox::SandboxWorkspaceWrite;
pub use skills::BundledSkillsConfig;
pub use skills::SkillConfig;
pub use skills::SkillsConfig;
pub use thread_store::ThreadStoreToml;
pub use tui_keymap::KeybindingSpec;
pub use tui_keymap::KeybindingsSpec;
pub use tui_keymap::TuiApprovalKeymap;
pub use tui_keymap::TuiChatKeymap;
pub use tui_keymap::TuiComposerKeymap;
pub use tui_keymap::TuiEditorKeymap;
pub use tui_keymap::TuiGlobalKeymap;
pub use tui_keymap::TuiKeymap;
pub use tui_keymap::TuiListKeymap;
pub use tui_keymap::TuiPagerKeymap;
pub use tui_keymap::TuiVimNormalKeymap;
pub use tui_keymap::TuiVimOperatorKeymap;
pub use ui::DEFAULT_TERMINAL_RESIZE_REFLOW_FALLBACK_MAX_ROWS;
pub use ui::ExternalConfigMigrationPrompts;
pub use ui::ModelAvailabilityNuxConfig;
pub use ui::Notice;
pub use ui::NotificationCondition;
pub use ui::NotificationMethod;
pub use ui::Notifications;
pub use ui::SessionPickerViewMode;
pub use ui::ToolSuggestConfig;
pub use ui::ToolSuggestDisabledTool;
pub use ui::ToolSuggestDiscoverable;
pub use ui::ToolSuggestDiscoverableType;
pub use ui::TuiNotificationSettings;
pub use ui::TuiPetAnchor;
pub use ui::UriBasedFileOpener;
pub use ui::WindowsSandboxModeToml;
pub use ui::WindowsToml;

/// Project-local trust decision loaded from the `[projects]` config map.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ProjectConfig {
    pub trust_level: Option<TrustLevel>,
}

impl ProjectConfig {
    pub fn is_trusted(&self) -> bool {
        matches!(self.trust_level, Some(TrustLevel::Trusted))
    }

    pub fn is_untrusted(&self) -> bool {
        matches!(self.trust_level, Some(TrustLevel::Untrusted))
    }
}

/// Identifies a configuration layer and its precedence in the merged config stack.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type")]
#[ts(export_to = "v2/")]
pub enum ConfigLayerSource {
    /// Managed preferences layer delivered by MDM (macOS only).
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Mdm {
        domain: String,
        key: String,
    },

    /// Managed config layer from a file (usually `managed_config.toml`).
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    System {
        /// This is the path to the system config.toml file, though it is not
        /// guaranteed to exist.
        file: AbsolutePathBuf,
    },

    /// User config layer from $MORPHEUS_HOME/config.toml. This layer is special
    /// in that it is expected to be:
    /// - writable by the user
    /// - generally outside the workspace directory
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    User {
        /// This is the path to the user's config.toml file, though it is not
        /// guaranteed to exist.
        file: AbsolutePathBuf,

        /// Name of the selected profile-v2 config layered on top of the base
        /// user config, when this layer represents one.
        profile: Option<String>,
    },

    /// Path to a .codex/ folder within a project. There could be multiple of
    /// these between `cwd` and the project/repo root.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Project {
        dot_codex_folder: AbsolutePathBuf,
    },

    /// Session-layer overrides supplied via `-c`/`--config`.
    SessionFlags,

    /// `managed_config.toml` was designed to be a config that was loaded
    /// as the last layer on top of everything else. This scheme did not quite
    /// work out as intended, but we keep this variant as a "best effort" while
    /// we phase out `managed_config.toml` in favor of `requirements.toml`.
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    LegacyManagedConfigTomlFromFile {
        file: AbsolutePathBuf,
    },

    LegacyManagedConfigTomlFromMdm,
}

impl ConfigLayerSource {
    /// Settings from a layer with a higher precedence override settings from a
    /// layer with a lower precedence.
    pub fn precedence(&self) -> i16 {
        match self {
            Self::Mdm { .. } => 0,
            Self::System { .. } => 10,
            Self::User { profile, .. } => {
                if profile.is_some() {
                    21
                } else {
                    20
                }
            }
            Self::Project { .. } => 25,
            Self::SessionFlags => 30,
            Self::LegacyManagedConfigTomlFromFile { .. } => 40,
            Self::LegacyManagedConfigTomlFromMdm => 50,
        }
    }
}

/// Compares [ConfigLayerSource] by precedence, so `A < B` means settings from
/// layer `A` will be overridden by settings from layer `B`.
impl PartialOrd for ConfigLayerSource {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.precedence().cmp(&other.precedence()))
    }
}

/// Metadata for one configuration layer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ConfigLayerMetadata {
    pub name: ConfigLayerSource,
    pub version: String,
}

/// Serialized view of one configuration layer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ConfigLayer {
    pub name: ConfigLayerSource,
    pub version: String,
    pub config: JsonValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

/// Settings that govern if and what will be written to `~/.morpheus/history.jsonl`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[serde(default)]
#[schemars(deny_unknown_fields)]
pub struct History {
    /// If true, history entries will not be written to disk.
    pub persistence: HistoryPersistence,

    /// If set, the maximum size of the history file in bytes. The oldest entries
    /// are dropped once the file exceeds this limit.
    pub max_bytes: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Default, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryPersistence {
    /// Save all history entries to disk.
    #[default]
    SaveAll,
    /// Do not write history to disk.
    None,
}

/// Determine where Codex should store CLI auth credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthCredentialsStoreMode {
    #[default]
    /// Persist credentials in MORPHEUS_HOME/auth.json.
    File,
    /// Persist credentials in the keyring. Fail if unavailable.
    Keyring,
    /// Use keyring when available; otherwise, fall back to a file in MORPHEUS_HOME.
    Auto,
    /// Store credentials in memory only for the current process.
    Ephemeral,
}

/// Determine where Codex should store and read MCP credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OAuthCredentialsStoreMode {
    /// `Keyring` when available; otherwise, `File`.
    /// Credentials stored in the keyring will only be readable by Codex unless the user explicitly grants access via OS-level keyring access.
    #[default]
    Auto,
    /// MORPHEUS_HOME/.credentials.json
    /// This file will be readable to Codex and other applications running as the same user.
    File,
    /// Keyring when available, otherwise fail.
    Keyring,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResidencyRequirement {
    Us,
}
