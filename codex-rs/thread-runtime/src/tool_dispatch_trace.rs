//! Adapter between session tool capability calls and rollout-trace events.
//!
//! `codex-rollout-trace` owns the event schema and writer behavior. This module
//! keeps session-owned trace construction next to the session capability
//! implementation instead of making the tool service own rollout details.

use codex_protocol::ThreadId;
use codex_rollout_trace_api::ExecutionStatus;
use codex_rollout_trace_api::ThreadTraceContext;
use codex_rollout_trace_api::ToolDispatchInvocation;
use codex_rollout_trace_api::ToolDispatchPayload;
use codex_rollout_trace_api::ToolDispatchRequester;
use codex_rollout_trace_api::ToolDispatchResult;
use codex_rollout_trace_api::ToolDispatchTraceContext;
use codex_thread_api::ToolSessionDispatchTrace;
use codex_tool_types::ToolName;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;

/// Keeps tool dispatch trace lifecycle paired with the session-owned rollout trace.
pub struct ToolDispatchTrace {
    context: ToolDispatchTraceContext,
    source: ToolCallSource,
}

impl ToolDispatchTrace {
    pub(crate) fn start_parts(
        thread_id: ThreadId,
        codex_turn_id: String,
        call_id: &str,
        tool_name: &ToolName,
        source: &ToolCallSource,
        payload: &ToolPayload,
        rollout_thread_trace: ThreadTraceContext,
    ) -> Self {
        let context = rollout_thread_trace.start_tool_dispatch_trace(|| {
            tool_dispatch_invocation_parts(
                thread_id,
                codex_turn_id,
                call_id,
                tool_name,
                source,
                payload,
            )
        });
        Self {
            context,
            source: source.clone(),
        }
    }
}

impl ToolSessionDispatchTrace for ToolDispatchTrace {
    fn record_completed(&self, call_id: &str, payload: &ToolPayload, result: &dyn ToolOutput) {
        if !self.context.is_enabled() {
            return;
        }

        let Some(result_payload) =
            tool_dispatch_result_parts(&self.source, call_id, payload, result)
        else {
            return;
        };
        let status = if result.success_for_logging() {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Failed
        };
        self.context.record_completed(status, result_payload);
    }

    fn record_failed(&self, error: &FunctionCallError) {
        self.context.record_failed(error);
    }
}

fn tool_dispatch_invocation_parts(
    thread_id: ThreadId,
    codex_turn_id: String,
    call_id: &str,
    tool_name: &ToolName,
    source: &ToolCallSource,
    payload: &ToolPayload,
) -> Option<ToolDispatchInvocation> {
    let requester = match source {
        ToolCallSource::Direct => ToolDispatchRequester::Model {
            model_visible_call_id: call_id.to_string(),
        },
        ToolCallSource::CodeMode {
            cell_id,
            runtime_tool_call_id,
        } => ToolDispatchRequester::CodeCell {
            runtime_cell_id: cell_id.clone(),
            runtime_tool_call_id: runtime_tool_call_id.clone(),
        },
    };

    Some(ToolDispatchInvocation {
        thread_id: thread_id.to_string(),
        codex_turn_id,
        tool_call_id: call_id.to_string(),
        tool_name: tool_name.name.clone(),
        tool_namespace: tool_name.namespace.clone(),
        requester,
        payload: tool_dispatch_payload(payload),
    })
}

fn tool_dispatch_result_parts(
    source: &ToolCallSource,
    call_id: &str,
    payload: &ToolPayload,
    result: &dyn ToolOutput,
) -> Option<ToolDispatchResult> {
    match source {
        ToolCallSource::Direct => Some(ToolDispatchResult::DirectResponse {
            response_item: result.to_response_item(call_id, payload),
        }),
        ToolCallSource::CodeMode { .. } => Some(ToolDispatchResult::CodeModeResponse {
            value: result.code_mode_result(payload),
        }),
    }
}

fn tool_dispatch_payload(payload: &ToolPayload) -> ToolDispatchPayload {
    match payload {
        ToolPayload::Function { arguments } => ToolDispatchPayload::Function {
            arguments: arguments.clone(),
        },
        ToolPayload::ToolSearch { arguments } => ToolDispatchPayload::ToolSearch {
            arguments: arguments.clone(),
        },
        ToolPayload::Custom { input } => ToolDispatchPayload::Custom {
            input: input.clone(),
        },
    }
}

#[cfg(test)]
#[path = "tool_dispatch_trace_tests.rs"]
mod tests;
