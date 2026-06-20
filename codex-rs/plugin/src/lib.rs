//! Shared plugin identifiers and telemetry-facing summaries.

pub use codex_utils_plugins::mention_syntax;
pub use codex_utils_plugins::plugin_namespace_for_skill_path;

mod load_outcome;

pub use codex_plugin_types::AppConnectorId;
pub use codex_plugin_types::PluginCapabilitySummary;
pub use codex_plugin_types::PluginHookSource;
pub use codex_plugin_types::PluginId;
pub use codex_plugin_types::PluginIdError;
pub use codex_plugin_types::PluginTelemetryMetadata;
pub use codex_plugin_types::validate_plugin_segment;
pub use load_outcome::EffectiveSkillRoots;
pub use load_outcome::LoadedPlugin;
pub use load_outcome::PluginLoadOutcome;
pub use load_outcome::prompt_safe_plugin_description;
