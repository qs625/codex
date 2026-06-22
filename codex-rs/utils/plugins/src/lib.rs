//! Plugin path resolution, plaintext mention sigils, and MCP connector helpers shared across Codex
//! crates.

pub mod mcp_connector;
pub mod mention_syntax;
pub mod plugin_namespace;

pub use codex_plugin_manifest::find_plugin_manifest_path;
pub use codex_plugin_manifest::plugin_namespace_for_skill_path;
pub use codex_plugin_types::PLUGIN_TEXT_MENTION_SIGIL;
pub use codex_plugin_types::PluginSkillRoot;
pub use codex_plugin_types::TOOL_MENTION_SIGIL;
