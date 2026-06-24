use crate::FunctionToolOutput;
use crate::ToolInvocationView;
use crate::ToolRegistry;
use crate::flat_tool_name;
use codex_tool_types::FunctionCallError;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

pub use codex_tool_runtime_api::PostToolUseHookOutcome;
pub use codex_tool_runtime_api::PreToolUseHookOutcome;
pub use codex_tool_runtime_api::ToolDispatchHost;
pub use codex_tool_runtime_api::ToolDispatchTraceHandle;

impl<Invocation, DiffContext> ToolRegistry<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    pub async fn dispatch_any_with_host<Host>(
        &self,
        host: &Host,
        mut invocation: Invocation,
    ) -> Result<crate::AnyToolResult, FunctionCallError>
    where
        Host: ToolDispatchHost<Invocation>,
    {
        let tool_name = invocation.tool_name().clone();
        let tool_name_flat = flat_tool_name(&tool_name);
        let call_id_owned = invocation.call_id().to_string();
        let telemetry = host.telemetry(&invocation);
        let base_tool_result_tags = host.base_tool_result_tags(&invocation);

        host.record_tool_call_started(&invocation).await;

        let dispatch_trace = host.start_trace(&invocation);
        let handler = match self.handler(&tool_name) {
            Some(handler) => handler,
            None => {
                let message =
                    crate::unsupported_tool_call_message(invocation.payload(), &tool_name);
                let log_payload = invocation.payload().log_payload();
                let base_tool_result_tag_refs = telemetry_tag_refs(&base_tool_result_tags);
                telemetry.tool_result_with_tags(
                    tool_name_flat.as_ref(),
                    &call_id_owned,
                    log_payload.as_ref(),
                    Duration::ZERO,
                    /*success*/ false,
                    &message,
                    &base_tool_result_tag_refs,
                    /*extra_trace_fields*/ &[],
                );
                let err = FunctionCallError::RespondToModel(message);
                dispatch_trace.record_failed(&err);
                return Err(err);
            }
        };

        let telemetry_tags = handler.telemetry_tags(&invocation).await;
        let (tool_result_tags, extra_trace_fields) =
            build_tool_result_tags(&base_tool_result_tags, &telemetry_tags);
        if !handler.matches_kind(invocation.payload()) {
            let message = format!("tool {tool_name} invoked with incompatible payload");
            let log_payload = invocation.payload().log_payload();
            telemetry.tool_result_with_tags(
                tool_name_flat.as_ref(),
                &call_id_owned,
                log_payload.as_ref(),
                Duration::ZERO,
                /*success*/ false,
                &message,
                &tool_result_tags,
                &extra_trace_fields,
            );
            let err = FunctionCallError::Fatal(message);
            dispatch_trace.record_failed(&err);
            return Err(err);
        }

        if let Some(pre_tool_use_payload) = handler.pre_tool_use_payload(&invocation) {
            match host
                .run_pre_tool_use_hooks(&invocation, pre_tool_use_payload)
                .await
            {
                PreToolUseHookOutcome::Blocked(message) => {
                    let err = FunctionCallError::RespondToModel(message);
                    dispatch_trace.record_failed(&err);
                    return Err(err);
                }
                PreToolUseHookOutcome::Continue {
                    updated_input: Some(updated_input),
                } => {
                    invocation = handler.with_updated_hook_input(invocation, updated_input)?;
                }
                PreToolUseHookOutcome::Continue {
                    updated_input: None,
                } => {}
            }
        }

        let response_cell = tokio::sync::Mutex::new(None);
        let invocation_for_tool = invocation.clone();
        let log_payload = invocation.payload().log_payload();

        let result = codex_session_telemetry_api::log_tool_result_with_tags(
            telemetry.as_ref(),
            tool_name_flat.as_ref(),
            &call_id_owned,
            log_payload.as_ref(),
            &tool_result_tags,
            &extra_trace_fields,
            || {
                let handler = Arc::clone(&handler);
                let response_cell = &response_cell;
                async move {
                    match handler.handle_any(invocation_for_tool).await {
                        Ok(result) => {
                            let preview = result.result.log_preview();
                            let success = result.result.success_for_logging();
                            let mut guard = response_cell.lock().await;
                            *guard = Some(result);
                            Ok((preview, success))
                        }
                        Err(err) => Err(err),
                    }
                }
            },
        )
        .await;
        let success = match &result {
            Ok((_, success)) => *success,
            Err(_) => false,
        };
        host.emit_tool_read_metric(&invocation, success).await;

        let post_tool_use_payload = if success {
            let guard = response_cell.lock().await;
            guard
                .as_ref()
                .and_then(|result| result.post_tool_use_payload.clone())
        } else {
            None
        };
        if let Some(post_tool_use_payload) = post_tool_use_payload {
            let outcome = host
                .run_post_tool_use_hooks(&invocation, post_tool_use_payload)
                .await;
            if let Some(replacement_text) = outcome.replacement_text {
                let mut guard = response_cell.lock().await;
                if let Some(result) = guard.as_mut() {
                    result.result = Box::new(FunctionToolOutput::from_text(
                        replacement_text,
                        /*success*/ None,
                    ));
                }
            }
        }

        if let Err(err) = host
            .account_goal_tool_completed(&invocation, &tool_name)
            .await
        {
            warn!("failed to account thread goal progress after tool call: {err}");
        }

        match result {
            Ok(_) => {
                let mut guard = response_cell.lock().await;
                let result = guard.take().ok_or_else(|| {
                    FunctionCallError::Fatal("tool produced no output".to_string())
                })?;
                dispatch_trace.record_completed(
                    &invocation,
                    &result.call_id,
                    &result.payload,
                    result.result.as_ref(),
                );
                Ok(result)
            }
            Err(err) => {
                dispatch_trace.record_failed(&err);
                Err(err)
            }
        }
    }
}

fn build_tool_result_tags<'a>(
    base_tags: &'a [(&'static str, String)],
    telemetry_tags: &'a [(&'static str, String)],
) -> (Vec<(&'static str, &'a str)>, Vec<(&'static str, &'a str)>) {
    let mut tool_result_tags = telemetry_tag_refs(base_tags);
    tool_result_tags.extend(
        telemetry_tags
            .iter()
            .map(|(key, value)| (*key, value.as_str())),
    );
    let extra_trace_fields = telemetry_tags
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    (tool_result_tags, extra_trace_fields)
}

fn telemetry_tag_refs<'a>(tags: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    tags.iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect()
}
