#[cfg(any(test, feature = "test-support"))]
use crate::SharedTurnDiffTracker;
#[cfg(any(test, feature = "test-support"))]
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
#[cfg(any(test, feature = "test-support"))]
use crate::session::turn_context::TurnContext;
#[cfg(any(test, feature = "test-support"))]
use crate::tools::context::ToolInvocation;
#[cfg(test)]
use crate::tools::handlers::core_apply_patch_handler_host;
#[cfg(any(test, feature = "test-support"))]
use crate::tools::registry::AnyToolResult;
#[cfg(test)]
use crate::tools::registry::CoreRegisteredTool;
#[cfg(any(test, feature = "test-support"))]
use crate::tools::registry::CoreToolDispatchHost;
#[cfg(test)]
use crate::tools::registry::ToolRegistryBuilder;
#[cfg(any(test, feature = "test-support"))]
use codex_extension_api::ExtensionToolExecutor;
#[cfg(any(test, feature = "test-support"))]
use codex_mcp_tool_types::ToolInfo;
#[cfg(any(test, feature = "test-support"))]
use codex_protocol::dynamic_tools::DynamicToolSpec;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_config::ToolsConfig;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_handlers::ToolRuntimeBuildParams;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_planning::DiscoverableTool;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_planning::ToolName;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_planning::ToolSpec;
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use tokio_util::sync::CancellationToken;
#[cfg(any(test, feature = "test-support"))]
use tracing::instrument;

#[cfg(any(test, feature = "test-support"))]
pub use codex_tool_planning::ToolCall;
#[cfg(any(test, feature = "test-support"))]
pub use codex_tool_planning::ToolCallSource;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime_api::ToolArgumentDiffConsumer;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime_api::ToolRouterBuildParams;

#[cfg(any(test, feature = "test-support"))]
pub type CoreToolInvocation = ToolInvocation;
#[cfg(any(test, feature = "test-support"))]
pub type CoreToolRegistry = codex_tool_runtime::ToolRegistry<CoreToolInvocation, TurnContext>;
#[cfg(any(test, feature = "test-support"))]
pub type CoreToolRuntimeRouterImpl = codex_tool_runtime::ToolRouter<CoreToolRegistry, TurnContext>;
#[cfg(any(test, feature = "test-support"))]
type CoreMcpToolCallHost = codex_session_api::SessionMcpToolCallHost<
    Session,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
>;
#[cfg(any(test, feature = "test-support"))]
type CoreMcpResourceHost = codex_session_api::SessionMcpResourceHost<
    Session,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
>;
#[cfg(any(test, feature = "test-support"))]
type CoreGoalHost = codex_session_api::SessionGoalHost<
    Session,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
>;
#[cfg(any(test, feature = "test-support"))]
type CoreWorkflowHost = codex_session_api::SessionWorkflowHost<
    Session,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
>;
#[cfg(any(test, feature = "test-support"))]
type CoreAgentJobHost = codex_session_api::SessionAgentJobHost<
    Session,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
    crate::config::Config,
>;

#[cfg(any(test, feature = "test-support"))]
pub struct ToolRouter {
    inner: CoreToolRuntimeRouterImpl,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct DefaultToolRouterFactory;

#[cfg(any(test, feature = "test-support"))]
impl
    codex_session_api::SessionToolRouterFactory<
        Arc<Session>,
        Arc<TurnContext>,
        SharedTurnDiffTracker,
        TurnContext,
    > for DefaultToolRouterFactory
{
    fn build_tool_router(
        &self,
        config: &ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> Arc<crate::CoreToolRuntimeRouter> {
        Arc::new(codex_tool_handlers::SessionToolRouterAdapter::new(
            build_runtime_router(config, params),
            CoreToolDispatchHost,
        ))
    }
}

#[cfg(any(test, feature = "test-support"))]
fn build_runtime_router(
    config: &ToolsConfig,
    params: ToolRouterBuildParams<'_>,
) -> CoreToolRuntimeRouterImpl {
    codex_tool_handlers::build_tool_router(
        config,
        &crate::tools::handlers::core_apply_patch_handler_host(),
        ToolRuntimeBuildParams {
            mcp_tools: params.mcp_tools,
            deferred_mcp_tools: params.deferred_mcp_tools,
            discoverable_tools: params.discoverable_tools,
            extension_tool_executors: params.extension_tool_executors,
            dynamic_tools: params.dynamic_tools,
            default_agent_type_description: params.default_agent_type_description,
            mcp_tool_call_host: CoreMcpToolCallHost::default(),
            mcp_resource_host: CoreMcpResourceHost::default(),
            goal_host: CoreGoalHost::default(),
            workflow_host: CoreWorkflowHost::default(),
            agent_job_host: CoreAgentJobHost::default(),
        },
    )
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) struct ToolRouterParams<'a> {
    pub(crate) mcp_tools: Option<Vec<ToolInfo>>,
    pub(crate) deferred_mcp_tools: Option<Vec<ToolInfo>>,
    pub(crate) discoverable_tools: Option<Vec<DiscoverableTool>>,
    pub(crate) extension_tool_executors: Vec<Arc<dyn ExtensionToolExecutor>>,
    pub(crate) dynamic_tools: &'a [DynamicToolSpec],
}

#[cfg(any(test, feature = "test-support"))]
impl ToolRouter {
    pub fn from_runtime(inner: CoreToolRuntimeRouterImpl) -> Self {
        Self { inner }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_config(config: &ToolsConfig, params: ToolRouterParams<'_>) -> Self {
        let ToolRouterParams {
            mcp_tools,
            deferred_mcp_tools,
            discoverable_tools,
            extension_tool_executors,
            dynamic_tools,
        } = params;
        let default_agent_type_description =
            codex_agent_roles::spawn_tool_spec::build(&std::collections::BTreeMap::new());
        Self::from_runtime(build_runtime_router(
            config,
            ToolRouterBuildParams {
                mcp_tools: mcp_tools.as_deref(),
                deferred_mcp_tools: deferred_mcp_tools.as_deref(),
                discoverable_tools: discoverable_tools.as_deref(),
                extension_tool_executors: &extension_tool_executors,
                dynamic_tools,
                default_agent_type_description: &default_agent_type_description,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn from_executors(
        config: &ToolsConfig,
        executors: Vec<Arc<CoreRegisteredTool>>,
        hosted_specs: Vec<ToolSpec>,
    ) -> Self {
        let builder = ToolRegistryBuilder::from_runtime(
            codex_tool_handlers::build_tool_registry_builder_from_executors(
                config,
                executors,
                hosted_specs,
                &core_apply_patch_handler_host(),
            ),
        );
        let (specs, registry) = builder.build();

        Self {
            inner: codex_tool_runtime::ToolRouter::new(
                config.code_mode_only_enabled,
                specs,
                registry.into_runtime(),
            ),
        }
    }

    pub fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.inner.model_visible_specs()
    }

    pub(crate) fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<crate::session::turn_context::TurnContext>>> {
        self.inner.create_diff_consumer(tool_name)
    }

    pub fn tool_supports_parallel(&self, call: &ToolCall) -> bool {
        self.inner.tool_supports_parallel(call)
    }

    #[instrument(level = "trace", skip_all, err)]
    pub async fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        cancellation_token: CancellationToken,
        tracker: SharedTurnDiffTracker,
        call: ToolCall,
        source: ToolCallSource,
    ) -> Result<AnyToolResult, FunctionCallError> {
        let invocation = ToolInvocation {
            session,
            turn,
            cancellation_token,
            tracker,
            metadata: call.into_invocation_metadata(source),
        };

        self.inner
            .registry()
            .dispatch_any_with_host(&CoreToolDispatchHost, invocation)
            .await
    }
}

#[cfg(any(test, feature = "test-support"))]
impl
    codex_tool_runtime::ToolCallRuntimeRouter<
        Arc<Session>,
        Arc<TurnContext>,
        SharedTurnDiffTracker,
        TurnContext,
    > for ToolRouter
{
    fn create_diff_consumer(
        &self,
        tool_name: &codex_tool_runtime::ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<TurnContext>>> {
        ToolRouter::create_diff_consumer(self, tool_name)
    }

    fn tool_supports_parallel(&self, call: &codex_tool_runtime::ToolCall) -> bool {
        ToolRouter::tool_supports_parallel(self, call)
    }

    async fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        cancellation_token: CancellationToken,
        tracker: SharedTurnDiffTracker,
        call: codex_tool_runtime::ToolCall,
        source: codex_tool_runtime::ToolCallSource,
    ) -> Result<AnyToolResult, FunctionCallError> {
        ToolRouter::dispatch_tool_call_with_code_mode_result(
            self,
            session,
            turn,
            cancellation_token,
            tracker,
            call,
            source,
        )
        .await
    }
}

#[cfg(any(test, feature = "test-support"))]
impl
    codex_session_api::SessionToolRouter<
        Arc<Session>,
        Arc<TurnContext>,
        SharedTurnDiffTracker,
        TurnContext,
    > for ToolRouter
{
    fn model_visible_specs(&self) -> Vec<ToolSpec> {
        ToolRouter::model_visible_specs(self)
    }

    fn create_diff_consumer(
        &self,
        tool_name: &codex_tool_runtime::ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<TurnContext>>> {
        ToolRouter::create_diff_consumer(self, tool_name)
    }

    fn tool_supports_parallel(&self, call: &codex_tool_runtime::ToolCall) -> bool {
        ToolRouter::tool_supports_parallel(self, call)
    }

    fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        cancellation_token: CancellationToken,
        tracker: SharedTurnDiffTracker,
        call: codex_tool_runtime::ToolCall,
        source: codex_tool_runtime::ToolCallSource,
    ) -> codex_session_api::SessionToolDispatchFuture<'_> {
        let result = ToolRouter::dispatch_tool_call_with_code_mode_result(
            self,
            session,
            turn,
            cancellation_token,
            tracker,
            call,
            source,
        );
        Box::pin(result)
    }
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
