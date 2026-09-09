use std::sync::Arc;

use protocol::AgentPath;
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
use tool_service_api::ToolCallOutcome;
use tool_service_api::ToolName;
use tool_service_api::ToolSpec;

use crate::HostLifecycleToolRuntime;
use crate::HostRelaunchMode;
use crate::HostRelaunchRequest;
use crate::HostRelaunchResult;
use crate::HostRelaunchStatus;
use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

pub(crate) const REQUEST_RUNTIME_RESTART_TOOL_NAME: &str = "request_runtime_restart";
const AUTHORIZED_AGENT_PATH: &str = "/self";
const RESUME_STRATEGY: &str = "expected_restart_intent";

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    specs_for_agent_path(request.current_agent_path.as_ref())
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
    current_agent_path: Option<AgentPath>,
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    runtime: Option<Arc<dyn HostLifecycleToolRuntime>>,
    call: ToolCall,
) -> Result<ToolCallOutcome, FunctionCallError> {
    ensure_runtime_restart_authorized(current_agent_path.as_ref())?;
    let args: RequestRuntimeRestartArgs = parse_arguments(&call)?;
    let reason = normalize_reason(args.reason);
    let mode = args.mode;
    let display_arguments = json!({
        "mode": mode,
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
                    request_id: call.call_id.clone(),
                    mode: mode.clone(),
                    reason: reason.clone(),
                    requested_by_thread_id: Some(session.conversation_id().to_string()),
                })
                .await
        }
        None => unsupported_relaunch_result(call.call_id.clone(), mode.clone(), reason.clone()),
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
    if result.accepted {
        return Ok(ToolCallOutcome::FinishTurn);
    }
    let tool_output = function_tool_json_output(&result)?;

    Ok(ToolCallOutcome::ReturnToModel(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(tool_output),
        post_tool_use_payload: None,
    }))
}

fn specs_for_agent_path(current_agent_path: Option<&AgentPath>) -> Vec<ToolSpec> {
    is_runtime_restart_authorized(current_agent_path)
        .then(create_request_runtime_restart_tool)
        .into_iter()
        .collect()
}

fn ensure_runtime_restart_authorized(
    current_agent_path: Option<&AgentPath>,
) -> Result<(), FunctionCallError> {
    if is_runtime_restart_authorized(current_agent_path) {
        return Ok(());
    }
    Err(FunctionCallError::RespondToModel(format!(
        "{REQUEST_RUNTIME_RESTART_TOOL_NAME} is only authorized for the exact canonical agent path {AUTHORIZED_AGENT_PATH}"
    )))
}

fn is_runtime_restart_authorized(current_agent_path: Option<&AgentPath>) -> bool {
    current_agent_path.is_some_and(|path| path.as_str() == AUTHORIZED_AGENT_PATH)
}

fn create_request_runtime_restart_tool() -> ToolSpec {
    let properties = std::collections::BTreeMap::from([
        (
            "mode".to_string(),
            JsonSchema::string_enum(
                vec![json!("hot"), json!("full")],
                Some(
                    "Required refresh mode. Use hot to restart the app-server and reload renderer windows after updating artifacts. Use full when Electron main or preload code must be loaded by a full app relaunch.".to_string(),
                ),
            ),
        ),
        (
            "reason".to_string(),
            JsonSchema::string(Some(
                "Optional concise reason for refreshing the running Morpheus host.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: REQUEST_RUNTIME_RESTART_TOOL_NAME.to_string(),
        description: concat!(
            "Use after completing a feature, fixing a bug, or changing Morpheus runtime, ",
            "client, server, frontend, or backend code when the running app needs to pick up ",
            "the latest compiled code. You must explicitly choose mode: \"hot\" for app-server ",
            "restart plus renderer reload, or mode: \"full\" when Electron main/preload changes ",
            "require a full app relaunch. Before calling, ensure the relevant frontend and ",
            "backend builds needed for the changes have already completed."
        )
        .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["mode".to_string()]),
            Some(false.into()),
        ),
        output_schema: Some(request_runtime_restart_output_schema()),
    })
}

fn request_runtime_restart_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "requestId": {
                "type": "string",
                "description": "Correlation identifier for this host lifecycle request."
            },
            "status": {
                "type": "string",
                "enum": ["accepted", "unsupported", "failed"],
                "description": "Whether the host accepted, does not support, or failed the refresh request."
            },
            "accepted": {
                "type": "boolean",
                "description": "Whether the request was delivered to the registered Host as a terminal control action."
            },
            "relaunching": {
                "type": "boolean",
                "description": "Whether the host has already confirmed that a relaunch-style fallback is in progress. A delivered request can still report false while the host update is pending."
            },
            "requestedMode": {
                "type": "string",
                "enum": ["hot", "full"],
                "description": "The explicit refresh mode requested by the tool caller."
            },
            "executedMode": {
                "type": ["string", "null"],
                "enum": ["hot", "full", null],
                "description": "The refresh mode the host accepted for execution, when available."
            },
            "message": {
                "type": "string",
                "description": "Human-readable result summary for the model."
            },
            "reason": {
                "type": ["string", "null"],
                "description": "The normalized refresh reason."
            },
            "resumeStrategy": {
                "type": "string",
                "enum": [RESUME_STRATEGY],
                "description": "How continuation is attempted after the host refreshes."
            }
        },
        "required": ["requestId", "status", "accepted", "relaunching", "requestedMode", "executedMode", "message", "reason", "resumeStrategy"],
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

fn unsupported_relaunch_result(
    request_id: String,
    requested_mode: HostRelaunchMode,
    reason: Option<String>,
) -> HostRelaunchResult {
    HostRelaunchResult {
        request_id,
        status: HostRelaunchStatus::Unsupported,
        accepted: false,
        relaunching: false,
        requested_mode,
        executed_mode: None,
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
    mode: HostRelaunchMode,
    reason: Option<String>,
}

#[cfg(test)]
#[path = "host_lifecycle_tests.rs"]
mod tests;
