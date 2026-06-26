#[cfg(test)]
use std::collections::HashMap;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

#[cfg(test)]
use crate::session::session::Session;
#[cfg(test)]
use crate::session::turn_context::TurnContext;
#[cfg(test)]
use crate::util::error_or_panic;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_api::SessionCapabilityFuture;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_api::ToolSessionCapability;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_api::ToolSessionDispatchTrace;
#[cfg(test)]
use codex_tool_planning::DuplicateToolName;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_planning::ToolName;
#[cfg(test)]
use codex_tool_planning::ToolSpec;
#[cfg(test)]
use codex_tool_types::FunctionCallError;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_types::ToolCallSource;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_types::ToolPayload;

#[cfg(test)]
pub use codex_tool_planning::ToolExecutor;
#[cfg(test)]
pub use codex_tool_planning::ToolExecutorFuture;
#[cfg(test)]
pub use codex_tool_planning::ToolExposure;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use codex_tool_runtime_api::AnyToolResult;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use codex_tool_runtime_api::PostToolUseHookOutcome;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use codex_tool_runtime_api::PostToolUsePayload;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use codex_tool_runtime_api::PreToolUseHookOutcome;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use codex_tool_runtime_api::PreToolUsePayload;
#[cfg(test)]
pub use codex_tool_runtime_api::RegisteredTool;
#[cfg(test)]
use codex_tool_runtime_api::ToolArgumentDiffConsumer;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::ToolDispatchTraceHandle;
#[cfg(test)]
pub use codex_tool_runtime_api::ToolHandler;
#[cfg(any(test, feature = "test-support"))]
pub use codex_tool_runtime_api::ToolTelemetryTags;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::override_tool_exposure;

#[cfg(test)]
type ToolInvocation = codex_tool_runtime::ToolInvocation<
    Arc<Session>,
    Arc<TurnContext>,
    crate::SharedTurnDiffTracker,
>;

#[cfg(test)]
pub(crate) type CoreRegisteredTool = dyn RegisteredTool<ToolInvocation, TurnContext>;

#[cfg(test)]
pub(crate) fn registered_tool<T>(handler: Arc<T>) -> Arc<CoreRegisteredTool>
where
    T: ToolHandler<ToolInvocation, TurnContext> + 'static,
{
    codex_tool_runtime_api::registered_tool(handler)
}

#[cfg(test)]
pub struct ToolRegistry {
    inner: codex_tool_runtime::ToolRegistry<ToolInvocation, TurnContext>,
    dispatch_host: codex_tool_handlers::SessionToolDispatchHost,
    _tool_session_capability: Option<Arc<dyn ToolSessionCapability>>,
}

#[cfg(test)]
impl ToolRegistry {
    fn new(handlers: HashMap<ToolName, Arc<CoreRegisteredTool>>) -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistry::new(handlers),
            dispatch_host: unavailable_dispatch_host_for_test(),
            _tool_session_capability: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistry::empty(),
            dispatch_host: unavailable_dispatch_host_for_test(),
            _tool_session_capability: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_with_session_capability_for_test(
        session_capability: Arc<dyn ToolSessionCapability>,
    ) -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistry::empty(),
            dispatch_host: codex_tool_handlers::SessionToolDispatchHost::new(Arc::downgrade(
                &session_capability,
            )),
            _tool_session_capability: Some(session_capability),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_handler_for_test<T>(handler: Arc<T>) -> Self
    where
        T: ToolHandler<ToolInvocation, TurnContext> + 'static,
    {
        Self {
            inner: codex_tool_runtime::ToolRegistry::with_handler(handler),
            dispatch_host: unavailable_dispatch_host_for_test(),
            _tool_session_capability: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_handler_and_session_capability_for_test<T>(
        handler: Arc<T>,
        session_capability: Arc<dyn ToolSessionCapability>,
    ) -> Self
    where
        T: ToolHandler<ToolInvocation, TurnContext> + 'static,
    {
        Self {
            inner: codex_tool_runtime::ToolRegistry::with_handler(handler),
            dispatch_host: codex_tool_handlers::SessionToolDispatchHost::new(Arc::downgrade(
                &session_capability,
            )),
            _tool_session_capability: Some(session_capability),
        }
    }

    fn handler(&self, name: &ToolName) -> Option<Arc<CoreRegisteredTool>> {
        self.inner.handler(name)
    }

    pub(crate) fn tool_exposure(&self, name: &ToolName) -> Option<ToolExposure> {
        self.inner.tool_exposure(name)
    }

    #[cfg(test)]
    pub(crate) fn has_handler(&self, name: &ToolName) -> bool {
        self.inner.has_handler(name)
    }

    pub(crate) fn create_diff_consumer(
        &self,
        name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<TurnContext>>> {
        self.inner.create_diff_consumer(name)
    }

    pub(crate) fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool> {
        self.inner.supports_parallel_tool_calls(name)
    }

    pub(crate) fn into_runtime(
        self,
    ) -> codex_tool_runtime::ToolRegistry<ToolInvocation, TurnContext> {
        self.inner
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "tool dispatch must keep active-turn accounting atomic"
    )]
    pub(crate) async fn dispatch_any(
        &self,
        invocation: ToolInvocation,
    ) -> Result<AnyToolResult, FunctionCallError> {
        self.inner
            .dispatch_any_with_host(&self.dispatch_host, invocation)
            .await
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn unavailable_dispatch_host_for_test() -> codex_tool_handlers::SessionToolDispatchHost {
    struct UnavailableToolSessionCapability;

    impl ToolSessionCapability for UnavailableToolSessionCapability {
        fn tool_dispatch_telemetry(
            &self,
            _turn: &dyn codex_thread_api::ToolTurnCapability,
        ) -> codex_session_telemetry_api::SharedSessionTelemetry {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn base_tool_result_tags(
            &self,
            _turn: &dyn codex_thread_api::ToolTurnCapability,
        ) -> ToolTelemetryTags {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn record_tool_call_started<'a>(
            &'a self,
            _turn: &'a dyn codex_thread_api::ToolTurnCapability,
        ) -> SessionCapabilityFuture<'a, ()> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn start_tool_dispatch_trace(
            &self,
            _turn: &dyn codex_thread_api::ToolTurnCapability,
            _call_id: &str,
            _tool_name: &ToolName,
            _source: &ToolCallSource,
            _payload: &ToolPayload,
        ) -> Box<dyn ToolSessionDispatchTrace> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn run_pre_tool_use_hooks_for_tool<'a>(
            &'a self,
            _turn: &'a dyn codex_thread_api::ToolTurnCapability,
            _call_id: String,
            _payload: PreToolUsePayload,
        ) -> SessionCapabilityFuture<'a, PreToolUseHookOutcome> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn run_post_tool_use_hooks_for_tool<'a>(
            &'a self,
            _turn: &'a dyn codex_thread_api::ToolTurnCapability,
            _payload: PostToolUsePayload,
        ) -> SessionCapabilityFuture<'a, PostToolUseHookOutcome> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn emit_tool_read_metric<'a>(
            &'a self,
            _turn: &'a dyn codex_thread_api::ToolTurnCapability,
            _tool_name: &'a ToolName,
            _payload: &'a ToolPayload,
            _success: bool,
        ) -> SessionCapabilityFuture<'a, ()> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }

        fn account_goal_tool_completed<'a>(
            &'a self,
            _turn: &'a dyn codex_thread_api::ToolTurnCapability,
            _tool_name: &'a ToolName,
        ) -> SessionCapabilityFuture<'a, Result<(), String>> {
            unreachable!("unavailable test dispatch host must not be used for dispatch")
        }
    }

    let session: Arc<dyn ToolSessionCapability> = Arc::new(UnavailableToolSessionCapability);
    codex_tool_handlers::SessionToolDispatchHost::new(Arc::downgrade(&session))
}

#[cfg(test)]
impl codex_tool_runtime_api::ToolRegistryView<TurnContext> for ToolRegistry {
    fn tool_exposure(&self, name: &ToolName) -> Option<ToolExposure> {
        self.tool_exposure(name)
    }

    fn create_diff_consumer(
        &self,
        name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<TurnContext>>> {
        self.create_diff_consumer(name)
    }

    fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool> {
        self.supports_parallel_tool_calls(name)
    }
}

#[cfg(test)]
pub struct ToolRegistryBuilder {
    inner: codex_tool_runtime::ToolRegistryBuilder<ToolInvocation, TurnContext>,
}

#[cfg(test)]
impl ToolRegistryBuilder {
    pub fn new() -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistryBuilder::new(),
        }
    }

    pub(crate) fn from_runtime(
        inner: codex_tool_runtime::ToolRegistryBuilder<ToolInvocation, TurnContext>,
    ) -> Self {
        Self { inner }
    }

    pub(crate) fn push_spec(&mut self, spec: ToolSpec) {
        self.inner.push_spec(spec);
    }

    pub(crate) fn register_tool(&mut self, handler: Arc<CoreRegisteredTool>) {
        self.inner
            .register_tool(handler)
            .unwrap_or_else(handle_duplicate_tool_name);
    }

    pub(crate) fn register_tool_without_spec(&mut self, handler: Arc<CoreRegisteredTool>) {
        self.inner
            .register_tool_without_spec(handler)
            .unwrap_or_else(handle_duplicate_tool_name);
    }

    pub fn build(self) -> (Vec<ToolSpec>, ToolRegistry) {
        let (specs, inner) = self.inner.build();
        (
            specs,
            ToolRegistry {
                inner,
                dispatch_host: unavailable_dispatch_host_for_test(),
                _tool_session_capability: None,
            },
        )
    }
}

#[cfg(test)]
fn handle_duplicate_tool_name(err: DuplicateToolName) {
    error_or_panic(format!("handler for tool {} already registered", err.name));
}

#[cfg(test)]
#[path = "tool_test_registry_tests.rs"]
mod tests;
