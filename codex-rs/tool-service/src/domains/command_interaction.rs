use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use codex_command_service_api::CommandServiceApi;
use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_runtime::CommandNotificationKind;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::CommandWaitStatus;
use codex_command_runtime::WriteStdinRequest;
use codex_thread_api::SessionCommandInteractionCaller;
use codex_protocol::models::CommandWaitNotificationKind as ResponseCommandWaitNotificationKind;
use codex_protocol::models::CommandWaitStatus as ResponseCommandWaitStatus;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_command_wait_tool;
use codex_tool_planning::create_write_stdin_tool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;

use crate::context::TypedToolSpecRequest;

const COMMAND_WAIT_TOOL_NAME: &str = "command_wait";
const COMMAND_WRITE_STDIN_TOOL_NAME: &str = "command_write_stdin";
const WRITE_STDIN_EMPTY_INPUT_ERROR: &str = "command_write_stdin requires non-empty `chars`; use command_wait for command completion or output notifications instead of polling for output.";

// This domain owns command-session interaction tools. They operate on command
// sessions created by `exec_command` and should eventually depend on a dedicated
// command service rather than thread-runtime internals.
pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    if request.config.shell_type != codex_protocol::openai_models::ConfigShellToolType::UnifiedExec
    {
        return Vec::new();
    }

    vec![create_command_wait_tool(), create_write_stdin_tool()]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            COMMAND_WAIT_TOOL_NAME | COMMAND_WRITE_STDIN_TOOL_NAME
        )
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
    command_service_api: Arc<dyn CommandServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        COMMAND_WAIT_TOOL_NAME => {
            dispatch_command_wait(
                command_service_api,
                session.as_ref(),
                turn.as_ref(),
                &call,
            )
            .await
        }
        COMMAND_WRITE_STDIN_TOOL_NAME => {
            dispatch_command_write_stdin(command_service_api, session.as_ref(), turn.as_ref(), &call)
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

async fn dispatch_command_wait(
    command_service_api: Arc<dyn CommandServiceApi>,
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let args: CommandWaitArgs = parse_function_arguments(call)?;
    let item_id = format!("response-item-{}", uuid::Uuid::new_v4());
    let created_at_ms = now_unix_timestamp_ms();
    let command_wait = command_service_api
        .begin_command_wait(
            turn.session_arc() as Arc<dyn CommandServiceSessionCapability>,
            CommandWaitRequest {
                process_id: args.command_id,
            },
        )
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("command_wait failed: {err}")))?;
    let wait_timeout = command_wait.wait_timeout();
    let started_item = command_wait_item(CommandWaitItemInput {
        id: item_id.clone(),
        command_id: command_wait.process_id(),
        status: CommandWaitStatus::Running,
        notification: None,
        exit_code: None,
        wall_time: Duration::ZERO,
        wait_timeout,
        created_at_ms,
    });
    session
        .emit_model_item_started_display_event(turn, &started_item)
        .await;

    let output = command_wait
        .finish()
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("command_wait failed: {err}")))?;

    let response_item = command_wait_item(CommandWaitItemInput {
        id: item_id,
        command_id: output.process_id,
        status: output.status.clone(),
        notification: output.notification,
        exit_code: output.exit_code,
        wall_time: output.wall_time,
        wait_timeout: output.wait_timeout,
        created_at_ms,
    });
    SessionCommandInteractionCaller::record_model_items_and_emit_display_events(
        session,
        turn,
        std::slice::from_ref(&response_item),
    )
    .await;

    let text = serde_json::to_string(&CommandWaitResponse {
        command_id: output.process_id,
        status: match &output.status {
            CommandWaitStatus::Running => "running",
            CommandWaitStatus::Completed => "completed",
        },
        notification: output.notification.map(|kind| match kind {
            CommandNotificationKind::Output => "output",
            CommandNotificationKind::Exit => "exit",
        }),
        exit_code: output.exit_code,
        wall_time_seconds: output.wall_time.as_secs_f64(),
        wait_timeout_ms: output.wait_timeout.as_millis() as i64,
    })
    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;

    Ok(FunctionToolOutput {
        body: vec![FunctionCallOutputContentItem::InputText { text }],
        success: Some(true),
        post_tool_use_response: None,
    })
}

async fn dispatch_command_write_stdin(
    command_service_api: Arc<dyn CommandServiceApi>,
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
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

    let response = command_service_api
        .write_command_stdin(
            turn.session_arc() as Arc<dyn CommandServiceSessionCapability>,
            WriteStdinRequest {
                process_id: args.command_id,
                input: &chars,
            },
        )
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
    SessionCommandInteractionCaller::record_model_items_and_emit_display_events(
        session,
        turn,
        std::slice::from_ref(&response_item),
    )
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
struct CommandWaitArgs {
    command_id: i32,
}

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    command_id: i32,
    #[serde(default)]
    chars: Option<String>,
}

#[derive(Serialize)]
struct CommandWaitResponse<'a> {
    command_id: i32,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    wall_time_seconds: f64,
    wait_timeout_ms: i64,
}

#[derive(Serialize)]
struct CommandWriteStdinResponse {
    command_id: i32,
    bytes_written: usize,
}

struct CommandWaitItemInput {
    id: String,
    command_id: i32,
    status: CommandWaitStatus,
    notification: Option<CommandNotificationKind>,
    exit_code: Option<i32>,
    wall_time: Duration,
    wait_timeout: Duration,
    created_at_ms: i64,
}

fn command_wait_item(input: CommandWaitItemInput) -> ResponseItem {
    ResponseItem::CommandWait {
        id: Some(input.id),
        command_id: input.command_id.to_string(),
        status: match input.status {
            CommandWaitStatus::Running => ResponseCommandWaitStatus::Running,
            CommandWaitStatus::Completed => ResponseCommandWaitStatus::Completed,
        },
        notification: input.notification.map(|kind| match kind {
            CommandNotificationKind::Output => ResponseCommandWaitNotificationKind::Output,
            CommandNotificationKind::Exit => ResponseCommandWaitNotificationKind::Exit,
        }),
        exit_code: input.exit_code,
        wall_time_seconds: input.wall_time.as_secs_f64(),
        wait_timeout_ms: input.wait_timeout.as_millis() as i64,
        created_at_ms: input.created_at_ms,
    }
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
