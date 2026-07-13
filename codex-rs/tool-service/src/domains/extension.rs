use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_extension_api::ExtensionToolExecutor;
use codex_extension_api::ToolOutput;
use protocol::protocol::BuiltinToolCallDisplayEvent;
use protocol::protocol::BuiltinToolCallStatus;
use protocol::protocol::EventMsg;
use serde_json::Value;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::HookToolName;
use tool_service_api::PostToolUsePayload;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolPayload;
use tool_service_api::ToolSpec;

use crate::context::TypedToolSpecRequest;
use crate::output::flat_tool_name;

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    let Some(extension_tools) = request.params.extension_tools else {
        return Vec::new();
    };

    extension_tools
        .tool_contributors
        .iter()
        .flat_map(|contributor| {
            contributor.tools(extension_tools.session_store, extension_tools.thread_store)
        })
        .filter(|tool| tool.exposure().is_direct())
        .filter_map(|tool| tool.spec())
        .collect()
}

pub(crate) fn owns_tool_name(request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    let Some(extension_tools) = request.params.extension_tools else {
        return false;
    };

    extension_tools
        .tool_contributors
        .iter()
        .flat_map(|contributor| {
            contributor.tools(extension_tools.session_store, extension_tools.thread_store)
        })
        .any(|tool| tool.tool_name() == *tool_name)
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) fn resolve_executor(
    request: &TypedToolSpecRequest<'_>,
    tool_name: &ToolName,
) -> Result<Arc<dyn ExtensionToolExecutor>, FunctionCallError> {
    let Some(extension_tools) = request.params.extension_tools else {
        return Err(FunctionCallError::Fatal(format!(
            "tool domain extension is unavailable for {tool_name}"
        )));
    };

    extension_tools
        .tool_contributors
        .iter()
        .flat_map(|contributor| {
            contributor.tools(extension_tools.session_store, extension_tools.thread_store)
        })
        .find(|tool| tool.tool_name() == *tool_name)
        .ok_or_else(|| FunctionCallError::Fatal(format!("unsupported extension tool {tool_name}")))
}

pub(crate) async fn dispatch(
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    executor: Arc<dyn ExtensionToolExecutor>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let tool = flat_tool_name(&executor.tool_name()).into_owned();
    if !is_schedule_display_tool(&tool) {
        let result = executor.handle(call.clone()).await?;
        let post_tool_use_payload = post_tool_use_payload(executor.as_ref(), &call, &result);

        return Ok(AnyToolResult {
            call_id: call.call_id,
            payload: call.payload,
            result: Box::new(result),
            post_tool_use_payload,
        });
    }

    let arguments = arguments_from_payload(&call.payload)
        .map(extension_tool_hook_input)
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let display_event = |status, output| BuiltinToolCallDisplayEvent {
        thread_id: session.conversation_id(),
        turn_id: turn.runtime_turn_id_str().to_string(),
        id: call.call_id.clone(),
        tool: tool.clone(),
        arguments: arguments.clone(),
        status,
        output,
        lifecycle_at_ms: now_unix_timestamp_ms(),
    };

    session
        .emit_event(
            turn.as_ref(),
            EventMsg::BuiltinToolCallStarted(display_event(
                BuiltinToolCallStatus::InProgress,
                None,
            )),
        )
        .await;

    let result = match executor.handle(call.clone()).await {
        Ok(result) => result,
        Err(err) => {
            session
                .emit_event(
                    turn.as_ref(),
                    EventMsg::BuiltinToolCallCompleted(display_event(
                        BuiltinToolCallStatus::Failed,
                        Some(serde_json::json!({ "error": err.to_string() })),
                    )),
                )
                .await;
            return Err(err);
        }
    };
    let post_tool_use_payload = post_tool_use_payload(executor.as_ref(), &call, &result);
    session
        .emit_event(
            turn.as_ref(),
            EventMsg::BuiltinToolCallCompleted(display_event(
                BuiltinToolCallStatus::Completed,
                result.post_tool_use_response(&call.call_id, &call.payload),
            )),
        )
        .await;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload,
    })
}

fn arguments_from_payload(payload: &ToolPayload) -> Option<&str> {
    let ToolPayload::Function { arguments } = payload else {
        return None;
    };
    Some(arguments)
}

fn post_tool_use_payload(
    executor: &dyn ExtensionToolExecutor,
    call: &ToolCall,
    result: &codex_extension_api::ExtensionToolOutput,
) -> Option<PostToolUsePayload> {
    let arguments = arguments_from_payload(&call.payload)?;
    Some(PostToolUsePayload {
        tool_name: HookToolName::new(flat_tool_name(&executor.tool_name()).into_owned()),
        tool_use_id: call.call_id.clone(),
        tool_input: extension_tool_hook_input(arguments),
        tool_response: result.post_tool_use_response(&call.call_id, &call.payload)?,
    })
}

fn extension_tool_hook_input(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }

    serde_json::from_str(arguments).unwrap_or_else(|_| Value::String(arguments.to_string()))
}

fn is_schedule_display_tool(tool: &str) -> bool {
    matches!(tool, "schedule_subscribe" | "schedule_unsubscribe")
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::is_schedule_display_tool;

    #[test]
    fn schedule_display_is_limited_to_schedule_tools() {
        assert!(is_schedule_display_tool("schedule_subscribe"));
        assert!(is_schedule_display_tool("schedule_unsubscribe"));
        assert!(!is_schedule_display_tool("memories/read"));
        assert!(!is_schedule_display_tool("event_command_subscribe"));
    }
}
