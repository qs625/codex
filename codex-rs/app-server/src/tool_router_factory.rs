use codex_core::CoreToolDomainHost;
use codex_core::CoreToolRuntimeRouter;
use codex_core::ToolRouterBuildParams;
use codex_core::ToolRouterFactory;
use codex_tool_handlers::ToolRuntimeBuildParams;

#[derive(Default)]
pub(crate) struct AppServerToolRouterFactory;

impl ToolRouterFactory for AppServerToolRouterFactory {
    fn build_tool_router(
        &self,
        config: &codex_tool_config::ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> CoreToolRuntimeRouter {
        codex_tool_handlers::build_tool_router(
            config,
            &CoreToolDomainHost,
            ToolRuntimeBuildParams {
                mcp_tools: params.mcp_tools,
                deferred_mcp_tools: params.deferred_mcp_tools,
                discoverable_tools: params.discoverable_tools,
                extension_tool_executors: params.extension_tool_executors,
                dynamic_tools: params.dynamic_tools,
                default_agent_type_description: params.default_agent_type_description,
            },
        )
    }
}
