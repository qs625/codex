use codex_core::CoreApplyPatchHandlerHost;
use codex_core::CoreToolDispatchHost;
use codex_tool_handlers::SessionToolRouterAdapter;
use codex_tool_handlers::ToolRuntimeBuildParams;
use codex_tool_runtime_api::ToolRouterBuildParams;
use std::sync::Arc;

type AppServerMcpToolCallHost = codex_session_api::SessionMcpToolCallHost<
    codex_core::Session,
    Arc<codex_core::TurnContext>,
    codex_core::SharedTurnDiffTracker,
    codex_core::TurnContext,
>;
type AppServerMcpResourceHost = codex_session_api::SessionMcpResourceHost<
    codex_core::Session,
    Arc<codex_core::TurnContext>,
    codex_core::SharedTurnDiffTracker,
    codex_core::TurnContext,
>;
type AppServerGoalHost = codex_session_api::SessionGoalHost<
    codex_core::Session,
    Arc<codex_core::TurnContext>,
    codex_core::SharedTurnDiffTracker,
    codex_core::TurnContext,
>;
type AppServerWorkflowHost = codex_session_api::SessionWorkflowHost<
    codex_core::Session,
    Arc<codex_core::TurnContext>,
    codex_core::SharedTurnDiffTracker,
    codex_core::TurnContext,
>;
type AppServerAgentJobHost = codex_session_api::SessionAgentJobHost<
    codex_core::Session,
    Arc<codex_core::TurnContext>,
    codex_core::SharedTurnDiffTracker,
    codex_core::TurnContext,
    codex_core::config::Config,
>;

#[derive(Default)]
pub(crate) struct AppServerToolRouterFactory;

impl
    codex_session_api::SessionToolRouterFactory<
        Arc<codex_core::Session>,
        Arc<codex_core::TurnContext>,
        codex_core::SharedTurnDiffTracker,
        codex_core::TurnContext,
    > for AppServerToolRouterFactory
{
    fn build_tool_router(
        &self,
        config: &codex_tool_config::ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> Arc<codex_core::CoreToolRuntimeRouter> {
        Arc::new(SessionToolRouterAdapter::new(
            codex_tool_handlers::build_tool_router(
                config,
                &CoreApplyPatchHandlerHost,
                ToolRuntimeBuildParams {
                    mcp_tools: params.mcp_tools,
                    deferred_mcp_tools: params.deferred_mcp_tools,
                    discoverable_tools: params.discoverable_tools,
                    extension_tool_executors: params.extension_tool_executors,
                    dynamic_tools: params.dynamic_tools,
                    default_agent_type_description: params.default_agent_type_description,
                    mcp_tool_call_host: AppServerMcpToolCallHost::default(),
                    mcp_resource_host: AppServerMcpResourceHost::default(),
                    goal_host: AppServerGoalHost::default(),
                    workflow_host: AppServerWorkflowHost::default(),
                    agent_job_host: AppServerAgentJobHost::default(),
                },
            ),
            CoreToolDispatchHost,
        ))
    }
}
