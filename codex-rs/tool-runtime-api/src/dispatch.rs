use crate::PostToolUsePayload;
use crate::PreToolUsePayload;
use crate::ToolTelemetryTags;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_tool_planning::ToolName;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use serde_json::Value;
use std::future::Future;

pub enum PreToolUseHookOutcome {
    Continue { updated_input: Option<Value> },
    Blocked(String),
}

#[derive(Default)]
pub struct PostToolUseHookOutcome {
    pub replacement_text: Option<String>,
}

/// Host-side dispatch hooks needed by the tools domain.
///
/// Implementations bridge a concrete session runtime to the host-neutral tool
/// registry. The registry owns dispatch ordering and result shaping; the host
/// owns integration with telemetry, hooks, trace sinks, and goal accounting.
pub trait ToolDispatchHost<Invocation>: Send + Sync {
    type Trace: ToolDispatchTraceHandle<Invocation> + Send;

    fn telemetry(&self, invocation: &Invocation) -> SharedSessionTelemetry;

    fn base_tool_result_tags(&self, invocation: &Invocation) -> ToolTelemetryTags;

    fn record_tool_call_started<'a>(
        &'a self,
        invocation: &'a Invocation,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn start_trace(&self, invocation: &Invocation) -> Self::Trace;

    fn run_pre_tool_use_hooks<'a>(
        &'a self,
        invocation: &'a Invocation,
        payload: PreToolUsePayload,
    ) -> impl Future<Output = PreToolUseHookOutcome> + Send + 'a;

    fn run_post_tool_use_hooks<'a>(
        &'a self,
        invocation: &'a Invocation,
        payload: PostToolUsePayload,
    ) -> impl Future<Output = PostToolUseHookOutcome> + Send + 'a;

    fn emit_tool_read_metric<'a>(
        &'a self,
        invocation: &'a Invocation,
        success: bool,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn account_goal_tool_completed<'a>(
        &'a self,
        invocation: &'a Invocation,
        tool_name: &'a ToolName,
    ) -> impl Future<Output = Result<(), String>> + Send + 'a;
}

pub trait ToolDispatchTraceHandle<Invocation> {
    fn record_completed(
        &self,
        invocation: &Invocation,
        call_id: &str,
        payload: &ToolPayload,
        result: &dyn ToolOutput,
    );

    fn record_failed(&self, error: &FunctionCallError);
}
