use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::planning::ToolSpec;
use crate::planning::create_write_stdin_tool;
use command_service_api::SessionCommandInteractionCaller;
use command_service_api::WriteStdinRequest;
use protocol::models::FunctionCallOutputContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::TerminalInteractionEvent;
use serde::Deserialize;
use serde::Serialize;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolPayload;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

const COMMAND_WRITE_STDIN_TOOL_NAME: &str = "command_write_stdin";
const WRITE_STDIN_EMPTY_INPUT_ERROR: &str = "command_write_stdin requires non-empty `chars`; use poll_event for command completion or output notifications instead of polling for output.";

// This domain owns command-session interaction tools. They operate on command
// sessions created by `exec_command` and should eventually depend on a dedicated
// command service rather than thread service internals.
pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    if request.config.shell_type != protocol::openai_models::ConfigShellToolType::UnifiedExec {
        return Vec::new();
    }

    vec![create_write_stdin_tool()]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name.as_str() == COMMAND_WRITE_STDIN_TOOL_NAME
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
    session_interaction: Arc<dyn SessionCommandInteractionCaller>,
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        COMMAND_WRITE_STDIN_TOOL_NAME => {
            dispatch_command_write_stdin(
                session_interaction.as_ref(),
                session.as_ref(),
                turn.as_ref(),
                &call,
            )
            .await
        }
        _ => Err(FunctionCallError::Fatal(format!(
            "unsupported command tool {}",
            call.tool_name
        ))),
    }?;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

async fn dispatch_command_write_stdin(
    session_interaction: &dyn SessionCommandInteractionCaller,
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let args: WriteStdinArgs = parse_function_arguments(call)?;
    let Some(chars) = args.chars else {
        return Err(FunctionCallError::RespondToModel(
            WRITE_STDIN_EMPTY_INPUT_ERROR.to_string(),
        ));
    };
    if chars.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            WRITE_STDIN_EMPTY_INPUT_ERROR.to_string(),
        ));
    }

    let response = session_interaction
        .write_command_stdin(WriteStdinRequest {
            process_id: args.command_id,
            input: &chars,
        })
        .await
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("command_write_stdin failed: {err}"))
        })?;

    session
        .send_terminal_interaction(
            turn,
            TerminalInteractionEvent {
                call_id: response.call_id.clone(),
                process_id: response.process_id.to_string(),
                stdin: chars.clone(),
            },
        )
        .await;

    let response_item = ResponseItem::CommandWriteStdin {
        id: None,
        command_id: response.process_id.to_string(),
        bytes_written: response.bytes_written,
        contains_newline: chars.contains('\n'),
        created_at_ms: now_unix_timestamp_ms(),
    };
    session
        .record_model_items_and_emit_display_events(turn, vec![response_item])
        .await;

    let text = serde_json::to_string(&CommandWriteStdinResponse {
        command_id: response.process_id,
        bytes_written: response.bytes_written,
    })
    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;

    Ok(FunctionToolOutput {
        body: vec![FunctionCallOutputContentItem::InputText { text }],
        success: Some(true),
        post_tool_use_response: None,
    })
}

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    command_id: i32,
    #[serde(default)]
    chars: Option<String>,
}

#[derive(Serialize)]
struct CommandWriteStdinResponse {
    command_id: i32,
    bytes_written: usize,
}

fn parse_function_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let arguments = match &call.payload {
        ToolPayload::Function { arguments } => arguments,
        _ => {
            return Err(FunctionCallError::RespondToModel(format!(
                "{} handler received unsupported payload",
                call.tool_name
            )));
        }
    };
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
