use std::sync::Arc;

use codex_extension_api::ExtensionToolExecutor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tool_config::ToolsConfig;
use codex_tool_planning::DiscoverableTool;

/// Borrowed inputs required to build the per-turn tool router.
///
/// This DTO intentionally contains only protocol-neutral tool discovery data
/// and extension executors. The concrete router type is supplied by the owner
/// crate through [`ToolRouterFactory`]'s generic parameter.
pub struct ToolRouterBuildParams<'a> {
    pub mcp_tools: Option<&'a [ToolInfo]>,
    pub deferred_mcp_tools: Option<&'a [ToolInfo]>,
    pub discoverable_tools: Option<&'a [DiscoverableTool]>,
    pub extension_tool_executors: &'a [Arc<dyn ExtensionToolExecutor>],
    pub dynamic_tools: &'a [DynamicToolSpec],
    pub default_agent_type_description: &'a str,
}

/// Factory contract for constructing a concrete per-turn tool router.
///
/// Implementations own the concrete registry/handler wiring. Session and thread
/// runtime code should depend on this contract instead of defining the factory
/// trait in `codex-core`.
pub trait ToolRouterFactory<Router>: Send + Sync {
    fn build_tool_router(&self, config: &ToolsConfig, params: ToolRouterBuildParams<'_>) -> Router;
}
