pub mod config_layers;
mod load_outcome;
mod manifest;
pub mod mcp_connector;
pub mod mention_syntax;
mod runtime;
mod types;

pub const OPENAI_CURATED_MARKETPLACE_NAME: &str = "openai-curated";
pub const OPENAI_BUNDLED_MARKETPLACE_NAME: &str = "openai-bundled";

pub const TOOL_SUGGEST_DISCOVERABLE_PLUGIN_ALLOWLIST: &[&str] = &[
    "github@openai-curated",
    "notion@openai-curated",
    "slack@openai-curated",
    "gmail@openai-curated",
    "google-calendar@openai-curated",
    "google-drive@openai-curated",
    "openai-developers@openai-curated",
    "canva@openai-curated",
    "teams@openai-curated",
    "sharepoint@openai-curated",
    "outlook-email@openai-curated",
    "outlook-calendar@openai-curated",
    "linear@openai-curated",
    "figma@openai-curated",
    "chrome@openai-bundled",
    "computer-use@openai-bundled",
];

pub use config_layers::PluginConfigLayerEntry;
pub use config_layers::PluginConfigLayerStack;
pub use load_outcome::EffectiveSkillRoots;
pub use load_outcome::LoadedPlugin;
pub use load_outcome::PluginAgentDir;
pub use load_outcome::prompt_safe_plugin_description;
pub use manifest::find_plugin_manifest_path;
pub use manifest::plugin_namespace_for_skill_path;
pub use runtime::DisabledPluginRuntime;
pub use runtime::PluginLoadOutcome;
pub use runtime::PluginRuntime;
pub use runtime::PluginRuntimeFuture;
pub use runtime::SharedPluginRuntime;
pub use runtime::ToolSuggestDiscoverablePlugin;
pub use types::AppConnectorId;
pub use types::PLUGIN_TEXT_MENTION_SIGIL;
pub use types::PluginAuthPolicy;
pub use types::PluginAvailability;
pub use types::PluginCapabilitySummary;
pub use types::PluginHookSource;
pub use types::PluginId;
pub use types::PluginIdError;
pub use types::PluginInstallPolicy;
pub use types::PluginInterface;
pub use types::PluginSkillRoot;
pub use types::PluginTelemetryMetadata;
pub use types::SkillInterface;
pub use types::TOOL_MENTION_SIGIL;
pub use types::validate_plugin_segment;

#[derive(Debug, Clone)]
pub struct PluginsConfigInput {
    pub config_layer_stack: PluginConfigLayerStack,
    pub plugins_enabled: bool,
    pub remote_plugin_enabled: bool,
    pub plugin_hooks_enabled: bool,
    pub chatgpt_base_url: String,
}

impl PluginsConfigInput {
    pub fn new(
        config_layer_stack: PluginConfigLayerStack,
        plugins_enabled: bool,
        remote_plugin_enabled: bool,
        plugin_hooks_enabled: bool,
        chatgpt_base_url: String,
    ) -> Self {
        Self {
            config_layer_stack,
            plugins_enabled,
            remote_plugin_enabled,
            plugin_hooks_enabled,
            chatgpt_base_url,
        }
    }
}
