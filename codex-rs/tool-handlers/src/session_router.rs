use codex_session_api::SessionToolDispatchFuture;
use codex_session_api::SessionToolRouter;
use codex_tool_runtime::ToolArgumentDiffConsumer;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime::ToolRegistry;
use codex_tool_runtime::ToolRouter;
use codex_tool_runtime_api::ToolDispatchHost;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolSpec;
use tokio_util::sync::CancellationToken;

/// Adapter from the concrete tool handler registry to the session-facing router API.
///
/// Tool owner crates construct the handler registry, while session runtimes
/// inject the dispatch host that owns hooks, telemetry, tracing, and goal
/// accounting side effects for one live session.
pub struct SessionToolRouterAdapter<Session, Turn, Tracker, DiffContext, DispatchHost> {
    inner:
        ToolRouter<ToolRegistry<ToolInvocation<Session, Turn, Tracker>, DiffContext>, DiffContext>,
    dispatch_host: DispatchHost,
}

impl<Session, Turn, Tracker, DiffContext, DispatchHost>
    SessionToolRouterAdapter<Session, Turn, Tracker, DiffContext, DispatchHost>
{
    pub fn new(
        inner: ToolRouter<
            ToolRegistry<ToolInvocation<Session, Turn, Tracker>, DiffContext>,
            DiffContext,
        >,
        dispatch_host: DispatchHost,
    ) -> Self {
        Self {
            inner,
            dispatch_host,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext, DispatchHost>
    SessionToolRouter<Session, Turn, Tracker, DiffContext>
    for SessionToolRouterAdapter<Session, Turn, Tracker, DiffContext, DispatchHost>
where
    Session: Clone + Send + Sync + 'static,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
    DispatchHost: ToolDispatchHost<ToolInvocation<Session, Turn, Tracker>> + Send + Sync + 'static,
{
    fn model_visible_specs(&self) -> Vec<ToolSpec> {
        self.inner.model_visible_specs()
    }

    fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.inner.create_diff_consumer(tool_name)
    }

    fn tool_supports_parallel(&self, call: &ToolCall) -> bool {
        self.inner.tool_supports_parallel(call)
    }

    fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Session,
        turn: Turn,
        cancellation_token: CancellationToken,
        tracker: Tracker,
        call: ToolCall,
        source: ToolCallSource,
    ) -> SessionToolDispatchFuture<'_> {
        Box::pin(async move {
            let invocation = ToolInvocation {
                session,
                turn,
                cancellation_token,
                tracker,
                metadata: call.into_invocation_metadata(source),
            };

            self.inner
                .registry()
                .dispatch_any_with_host(&self.dispatch_host, invocation)
                .await
        })
    }
}
