use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::context::ToolInvocation;
#[cfg(test)]
use crate::tools::handlers::core_tool_domain_host;
use crate::tools::registry::AnyToolResult;
#[cfg(test)]
use crate::tools::registry::CoreRegisteredTool;
use crate::tools::registry::CoreToolDispatchHost;
use crate::tools::registry::ToolArgumentDiffConsumer;
#[cfg(test)]
use crate::tools::registry::ToolRegistryBuilder;
use codex_extension_api::ExtensionToolExecutor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tool_config::ToolsConfig;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_handlers::ToolRuntimeBuildParams;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

pub use codex_tool_planning::ToolCall;
pub use codex_tool_planning::ToolCallSource;

pub type CoreToolInvocation = ToolInvocation;
pub type CoreToolRegistry = codex_tool_runtime::ToolRegistry<CoreToolInvocation, TurnContext>;
pub type CoreToolRuntimeRouter = codex_tool_runtime::ToolRouter<CoreToolRegistry, TurnContext>;

pub struct ToolRouter {
    inner: CoreToolRuntimeRouter,
}

pub struct ToolRouterBuildParams<'a> {
    pub mcp_tools: Option<&'a [ToolInfo]>,
    pub deferred_mcp_tools: Option<&'a [ToolInfo]>,
    pub discoverable_tools: Option<&'a [DiscoverableTool]>,
    pub extension_tool_executors: &'a [Arc<dyn ExtensionToolExecutor>],
    pub dynamic_tools: &'a [DynamicToolSpec],
    pub default_agent_type_description: &'a str,
}

pub trait ToolRouterFactory: Send + Sync {
    fn build_tool_router(
        &self,
        config: &ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> CoreToolRuntimeRouter;
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct DefaultToolRouterFactory;

#[cfg(any(test, feature = "test-support"))]
impl ToolRouterFactory for DefaultToolRouterFactory {
    fn build_tool_router(
        &self,
        config: &ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> CoreToolRuntimeRouter {
        codex_tool_handlers::build_tool_router(
            config,
            &crate::tools::handlers::core_tool_domain_host(),
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

pub(crate) struct ToolRouterParams<'a> {
    pub(crate) mcp_tools: Option<Vec<ToolInfo>>,
    pub(crate) deferred_mcp_tools: Option<Vec<ToolInfo>>,
    pub(crate) discoverable_tools: Option<Vec<DiscoverableTool>>,
    pub(crate) extension_tool_executors: Vec<Arc<dyn ExtensionToolExecutor>>,
    pub(crate) dynamic_tools: &'a [DynamicToolSpec],
}

impl ToolRouter {
    pub fn from_runtime(inner: CoreToolRuntimeRouter) -> Self {
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
        Self::from_runtime(DefaultToolRouterFactory.build_tool_router(
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
                &core_tool_domain_host(),
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

pub(crate) type ToolCallRuntime = codex_tool_runtime::ToolCallRuntime<
    ToolRouter,
    Arc<Session>,
    Arc<TurnContext>,
    SharedTurnDiffTracker,
    TurnContext,
>;

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

pub(crate) fn extension_tool_executors(session: &Session) -> Vec<Arc<dyn ExtensionToolExecutor>> {
    session
        .services
        .extensions
        .tool_contributors()
        .iter()
        .flat_map(|contributor| {
            contributor.tools(
                &session.services.session_extension_data,
                &session.services.thread_extension_data,
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
