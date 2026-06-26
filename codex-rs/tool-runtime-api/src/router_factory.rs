use std::sync::Arc;

use codex_extension_api::ExtensionData;
use codex_extension_api::ToolContributor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tool_planning::DiscoverableTool;

/// Borrowed extension state required by the tool owner to discover extension tools.
#[derive(Clone, Copy)]
pub struct ExtensionToolBuildParams<'a> {
    pub tool_contributors: &'a [Arc<dyn ToolContributor>],
    pub session_store: &'a ExtensionData,
    pub thread_store: &'a ExtensionData,
}

/// Borrowed inputs required to build the per-turn tool router.
///
/// This DTO intentionally contains only protocol-neutral tool discovery data
/// plus extension contributor state. Concrete tool-service implementations use
/// it to prepare one turn's tool set.
pub struct ToolServiceParams<'a> {
    pub mcp_tools: Option<&'a [ToolInfo]>,
    pub deferred_mcp_tools: Option<&'a [ToolInfo]>,
    pub discoverable_tools: Option<&'a [DiscoverableTool]>,
    pub extension_tools: Option<ExtensionToolBuildParams<'a>>,
    pub dynamic_tools: &'a [DynamicToolSpec],
    pub default_agent_type_description: &'a str,
}
