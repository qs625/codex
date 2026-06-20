use crate::AppToolApproval;
use crate::McpServerToolConfig;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;

const fn default_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PluginConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Per-MCP-server policy overlays for MCP servers contributed by this plugin.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub mcp_servers: HashMap<String, PluginMcpServerConfig>,
}

/// Policy settings for a plugin-provided MCP server.
///
/// This intentionally excludes transport settings: plugin manifests own how the
/// MCP server is launched, while user config owns enablement and tool policy.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PluginMcpServerConfig {
    /// When `false`, Codex skips initializing this plugin MCP server.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Approval mode for tools in this server unless a tool override exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tools_approval_mode: Option<AppToolApproval>,

    /// Explicit allow-list of tools exposed from this server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,

    /// Explicit deny-list of tools. These tools are removed after applying `enabled_tools`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<Vec<String>>,

    /// Per-tool approval settings keyed by tool name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tools: HashMap<String, McpServerToolConfig>,
}

impl Default for PluginMcpServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            tools: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct MarketplaceConfig {
    /// Last time Codex successfully added or refreshed this marketplace.
    #[serde(default)]
    pub last_updated: Option<String>,
    /// Git revision Codex last successfully activated for this marketplace.
    #[serde(default)]
    pub last_revision: Option<String>,
    /// Source kind used to install this marketplace.
    #[serde(default)]
    pub source_type: Option<MarketplaceSourceType>,
    /// Source location used when the marketplace was added.
    #[serde(default)]
    pub source: Option<String>,
    /// Git ref to check out when `source_type` is `git`.
    #[serde(default, rename = "ref")]
    pub ref_name: Option<String>,
    /// Sparse checkout paths used when `source_type` is `git`.
    #[serde(default)]
    pub sparse_paths: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarketplaceSourceType {
    Git,
    Local,
}
