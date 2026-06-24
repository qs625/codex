#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;

use crate::function_tool::FunctionCallError;
use crate::goal::GoalRuntimeEvent;
use crate::hook_runtime::PreToolUseHookResult;
use crate::hook_runtime::record_additional_contexts;
use crate::hook_runtime::run_post_tool_use_hooks;
use crate::hook_runtime::run_pre_tool_use_hooks;
use crate::memory_usage::emit_metric_for_tool_read;
#[cfg(test)]
use crate::session::turn_context::TurnContext;
use crate::tools::context::ToolInvocation;
use crate::tools::tool_dispatch_trace::ToolDispatchTrace;
#[cfg(test)]
use crate::util::error_or_panic;
use codex_sandboxing_api::permission_profile_policy_tag;
use codex_sandboxing_api::permission_profile_sandbox_tag;
use codex_session_telemetry_api::SharedSessionTelemetry;
#[cfg(test)]
use codex_tool_planning::DuplicateToolName;
use codex_tool_planning::ToolName;
#[cfg(test)]
use codex_tool_planning::ToolSpec;

#[cfg(test)]
pub use codex_tool_planning::ToolExecutor;
#[cfg(test)]
pub use codex_tool_planning::ToolExecutorFuture;
#[cfg(test)]
pub use codex_tool_planning::ToolExposure;
pub(crate) use codex_tool_runtime_api::AnyToolResult;
pub(crate) use codex_tool_runtime_api::PostToolUseHookOutcome;
pub(crate) use codex_tool_runtime_api::PostToolUsePayload;
pub(crate) use codex_tool_runtime_api::PreToolUseHookOutcome;
pub(crate) use codex_tool_runtime_api::PreToolUsePayload;
pub use codex_tool_runtime_api::RegisteredTool;
pub use codex_tool_runtime_api::ToolArgumentDiffConsumer;
pub(crate) use codex_tool_runtime_api::ToolDispatchHost;
pub(crate) use codex_tool_runtime_api::ToolDispatchTraceHandle;
#[cfg(test)]
pub use codex_tool_runtime_api::ToolHandler;
pub use codex_tool_runtime_api::ToolTelemetryTags;
#[cfg(test)]
pub(crate) use codex_tool_runtime_api::override_tool_exposure;

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
}

#[cfg(test)]
impl ToolRegistry {
    fn new(handlers: HashMap<ToolName, Arc<CoreRegisteredTool>>) -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistry::new(handlers),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            inner: codex_tool_runtime::ToolRegistry::empty(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_handler_for_test<T>(handler: Arc<T>) -> Self
    where
        T: ToolHandler<ToolInvocation, TurnContext> + 'static,
    {
        Self {
            inner: codex_tool_runtime::ToolRegistry::with_handler(handler),
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
            .dispatch_any_with_host(&CoreToolDispatchHost, invocation)
            .await
    }
}

pub(crate) struct CoreToolDispatchHost;

impl ToolDispatchHost<ToolInvocation> for CoreToolDispatchHost {
    type Trace = ToolDispatchTrace;

    fn telemetry(&self, invocation: &ToolInvocation) -> SharedSessionTelemetry {
        invocation.turn.session_telemetry.clone()
    }

    fn base_tool_result_tags(&self, invocation: &ToolInvocation) -> ToolTelemetryTags {
        vec![
            (
                "sandbox",
                permission_profile_sandbox_tag(
                    &invocation.turn.permission_profile,
                    invocation.turn.windows_sandbox_level,
                    invocation.turn.network.is_some(),
                )
                .to_string(),
            ),
            (
                "sandbox_policy",
                permission_profile_policy_tag(
                    &invocation.turn.permission_profile,
                    #[allow(deprecated)]
                    invocation.turn.cwd.as_path(),
                )
                .to_string(),
            ),
        ]
    }

    async fn record_tool_call_started(&self, invocation: &ToolInvocation) {
        let mut active = invocation.session.active_turn.lock().await;
        if let Some(active_turn) = active.as_mut() {
            let mut turn_state = active_turn.turn_state.lock().await;
            turn_state.tool_calls = turn_state.tool_calls.saturating_add(1);
        }
    }

    fn start_trace(&self, invocation: &ToolInvocation) -> Self::Trace {
        ToolDispatchTrace::start(invocation)
    }

    async fn run_pre_tool_use_hooks(
        &self,
        invocation: &ToolInvocation,
        payload: PreToolUsePayload,
    ) -> PreToolUseHookOutcome {
        match run_pre_tool_use_hooks(
            &invocation.session,
            &invocation.turn,
            invocation.call_id.clone(),
            &payload.tool_name,
            &payload.tool_input,
        )
        .await
        {
            PreToolUseHookResult::Blocked(message) => PreToolUseHookOutcome::Blocked(message),
            PreToolUseHookResult::Continue { updated_input } => {
                PreToolUseHookOutcome::Continue { updated_input }
            }
        }
    }

    async fn run_post_tool_use_hooks(
        &self,
        invocation: &ToolInvocation,
        payload: PostToolUsePayload,
    ) -> PostToolUseHookOutcome {
        let outcome = run_post_tool_use_hooks(
            &invocation.session,
            &invocation.turn,
            payload.tool_use_id,
            payload.tool_name.name().to_string(),
            payload.tool_name.matcher_aliases().to_vec(),
            payload.tool_input,
            payload.tool_response,
        )
        .await;

        record_additional_contexts(
            &invocation.session,
            &invocation.turn,
            outcome.additional_contexts.clone(),
        )
        .await;
        let replacement_text = if outcome.should_stop {
            Some(
                outcome
                    .feedback_message
                    .or(outcome.stop_reason)
                    .unwrap_or_else(|| "PostToolUse hook stopped execution".to_string()),
            )
        } else {
            outcome.feedback_message
        };

        PostToolUseHookOutcome { replacement_text }
    }

    async fn emit_tool_read_metric(&self, invocation: &ToolInvocation, success: bool) {
        emit_metric_for_tool_read(invocation, success).await;
    }

    async fn account_goal_tool_completed(
        &self,
        invocation: &ToolInvocation,
        tool_name: &ToolName,
    ) -> Result<(), String> {
        invocation
            .session
            .goal_runtime_apply(GoalRuntimeEvent::ToolCompleted {
                turn_context: invocation.turn.as_ref(),
                tool_name: tool_name.name.as_str(),
            })
            .await
            .map_err(|err| err.to_string())
    }
}

impl ToolDispatchTraceHandle<ToolInvocation> for ToolDispatchTrace {
    fn record_completed(
        &self,
        invocation: &ToolInvocation,
        call_id: &str,
        payload: &codex_tool_planning::ToolPayload,
        result: &dyn codex_tool_planning::ToolOutput,
    ) {
        ToolDispatchTrace::record_completed(self, invocation, call_id, payload, result);
    }

    fn record_failed(&self, error: &FunctionCallError) {
        ToolDispatchTrace::record_failed(self, error);
    }
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
        (specs, ToolRegistry { inner })
    }
}

#[cfg(test)]
fn handle_duplicate_tool_name(err: DuplicateToolName) {
    error_or_panic(format!("handler for tool {} already registered", err.name));
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
