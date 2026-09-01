use std::sync::Arc;

use protocol::protocol::BuiltinToolCallDisplayEvent;
use protocol::protocol::BuiltinToolCallStatus;
use protocol::protocol::EventMsg;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::JsonSchema;
use tool_service_api::ResponsesApiTool;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolSpec;

use crate::HostLifecycleToolRuntime;
use crate::HostRelaunchRequest;
use crate::HostRelaunchResult;
use crate::HostRelaunchStatus;
use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

pub(crate) const REQUEST_RUNTIME_RESTART_TOOL_NAME: &str = "request_runtime_restart";
const RESUME_STRATEGY: &str = "client_bootstrap_autoresume";

pub(crate) fn specs(_request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![create_request_runtime_restart_tool()]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == REQUEST_RUNTIME_RESTART_TOOL_NAME
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

pub(crate) async fn dispatch(
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    runtime: Option<Arc<dyn HostLifecycleToolRuntime>>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let args: RequestRuntimeRestartArgs = parse_arguments(&call)?;
    let reason = normalize_reason(args.reason);
    let display_arguments = json!({
        "reason": reason.clone(),
    });
    session
        .emit_event(
            turn.as_ref(),
            display_event_started(
                session.as_ref(),
                turn.as_ref(),
                &call.call_id,
                display_arguments.clone(),
            ),
        )
        .await;

    let result = match runtime {
        Some(runtime) => {
            runtime
                .request_client_relaunch(HostRelaunchRequest {
                    reason: reason.clone(),
                    requested_by_thread_id: Some(session.conversation_id().to_string()),
                })
                .await
        }
        None => unsupported_relaunch_result(reason.clone()),
    };
    let output = serde_json::to_value(&result).map_err(|err| {
        FunctionCallError::Fatal(format!(
            "failed to serialize {REQUEST_RUNTIME_RESTART_TOOL_NAME} display output: {err}"
        ))
    })?;
    let status = if result.accepted {
        BuiltinToolCallStatus::Completed
    } else {
        BuiltinToolCallStatus::Failed
    };
    session
        .emit_event(
            turn.as_ref(),
            display_event_completed(
                session.as_ref(),
                turn.as_ref(),
                &call.call_id,
                display_arguments,
                status,
                Some(output),
            ),
        )
        .await;
    let tool_output = function_tool_json_output(&result)?;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(tool_output),
        post_tool_use_payload: None,
    })
}

fn create_request_runtime_restart_tool() -> ToolSpec {
    let properties = std::collections::BTreeMap::from([(
        "reason".to_string(),
        JsonSchema::string(Some(
            "Optional concise reason for requesting a full Morpheus client/app-server relaunch."
                .to_string(),
        )),
    )]);

    ToolSpec::Function(ResponsesApiTool {
        name: REQUEST_RUNTIME_RESTART_TOOL_NAME.to_string(),
        description: "Request a full Morpheus client/app-server relaunch after runtime, client, or server code changes. This does not run shell commands, kill processes directly, or wait for the restarted app to finish work; after relaunch, client bootstrap autoresume will restore eligible interrupted sessions and let the model decide the next step.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(Vec::new()), Some(false.into())),
        output_schema: Some(request_runtime_restart_output_schema()),
    })
}

fn request_runtime_restart_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["accepted", "unsupported", "failed"],
                "description": "Whether the host accepted, does not support, or failed the relaunch request."
            },
            "accepted": {
                "type": "boolean",
                "description": "Whether the relaunch request was accepted for delivery to the host client."
            },
            "relaunching": {
                "type": "boolean",
                "description": "Whether a client relaunch should now be in progress."
            },
            "message": {
                "type": "string",
                "description": "Human-readable result summary for the model."
            },
            "reason": {
                "type": ["string", "null"],
                "description": "The normalized relaunch reason."
            },
            "resumeStrategy": {
                "type": "string",
                "enum": [RESUME_STRATEGY],
                "description": "How continuation is attempted after relaunch."
            }
        },
        "required": ["status", "accepted", "relaunching", "message", "reason", "resumeStrategy"],
        "additionalProperties": false
    })
}

fn display_event_started(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call_id: &str,
    arguments: Value,
) -> EventMsg {
    EventMsg::BuiltinToolCallStarted(BuiltinToolCallDisplayEvent {
        thread_id: session.conversation_id(),
        turn_id: turn.runtime_turn_id_str().to_string(),
        id: call_id.to_string(),
        tool: REQUEST_RUNTIME_RESTART_TOOL_NAME.to_string(),
        arguments,
        status: BuiltinToolCallStatus::InProgress,
        output: None,
        lifecycle_at_ms: now_unix_timestamp_ms(),
    })
}

fn display_event_completed(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call_id: &str,
    arguments: Value,
    status: BuiltinToolCallStatus,
    output: Option<Value>,
) -> EventMsg {
    EventMsg::BuiltinToolCallCompleted(BuiltinToolCallDisplayEvent {
        thread_id: session.conversation_id(),
        turn_id: turn.runtime_turn_id_str().to_string(),
        id: call_id.to_string(),
        tool: REQUEST_RUNTIME_RESTART_TOOL_NAME.to_string(),
        arguments,
        status,
        output,
        lifecycle_at_ms: now_unix_timestamp_ms(),
    })
}

fn normalize_reason(reason: Option<String>) -> Option<String> {
    reason.and_then(|reason| {
        let trimmed = reason.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn unsupported_relaunch_result(reason: Option<String>) -> HostRelaunchResult {
    HostRelaunchResult {
        status: HostRelaunchStatus::Unsupported,
        accepted: false,
        relaunching: false,
        message: "The current host does not expose a client relaunch runtime.".to_string(),
        reason,
        resume_strategy: RESUME_STRATEGY.to_string(),
    }
}

fn function_tool_json_output(
    result: &HostRelaunchResult,
) -> Result<FunctionToolOutput, FunctionCallError> {
    serde_json::to_string(result)
        .map(|text| FunctionToolOutput::from_text(text, Some(result.accepted)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!(
                "failed to serialize {REQUEST_RUNTIME_RESTART_TOOL_NAME} result: {err}"
            ))
        })
}

fn parse_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn now_unix_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestRuntimeRestartArgs {
    reason: Option<String>,
}

#[cfg(test)]
#[path = "host_lifecycle_tests.rs"]
mod tests;
