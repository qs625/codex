use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::planning::ToolSpec;
use crate::planning::create_command_wait_tool;
use crate::planning::create_write_stdin_tool;
use command_service_api::CommandNotificationKind;
use command_service_api::CommandServiceApi;
use command_service_api::CommandWaitOutput;
use command_service_api::CommandWaitRequest;
use command_service_api::CommandWaitStatus;
use command_service_api::SessionCommandInteractionCaller;
use command_service_api::WriteStdinRequest;
use protocol::models::CommandWaitNotificationKind as ResponseCommandWaitNotificationKind;
use protocol::models::CommandWaitStatus as ResponseCommandWaitStatus;
use protocol::models::FunctionCallOutputContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::TerminalInteractionEvent;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc;
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

const COMMAND_WAIT_TOOL_NAME: &str = "command_wait";
const COMMAND_WRITE_STDIN_TOOL_NAME: &str = "command_write_stdin";
const WRITE_STDIN_EMPTY_INPUT_ERROR: &str = "command_write_stdin requires non-empty `chars`; use command_wait for command completion or output notifications instead of polling for output.";

// This domain owns command-session interaction tools. They operate on command
// sessions created by `exec_command` and should eventually depend on a dedicated
// command service rather than thread service internals.
pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    if request.config.shell_type != protocol::openai_models::ConfigShellToolType::UnifiedExec {
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
    _command_service_api: Arc<dyn CommandServiceApi>,
    session_interaction: Arc<dyn SessionCommandInteractionCaller>,
    session: Arc<dyn ThreadSessionCapability>,
    thread_service_api: Arc<dyn thread_service_api::ThreadServiceApi>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        COMMAND_WAIT_TOOL_NAME => {
            dispatch_command_wait(
                session_interaction.as_ref(),
                session.as_ref(),
                thread_service_api.as_ref(),
                Arc::clone(&turn),
                &call,
            )
            .await
        }
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

async fn dispatch_command_wait(
    session_interaction: &dyn SessionCommandInteractionCaller,
    session: &dyn ThreadSessionCapability,
    thread_service_api: &dyn thread_service_api::ThreadServiceApi,
    turn: Arc<dyn ThreadRuntimeCapability>,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let item_id = format!("response-item-{}", uuid::Uuid::new_v4());
    let created_at_ms = now_unix_timestamp_ms();
    let args: CommandWaitArgs = parse_function_arguments(call)?;
    let mut command_wait = session_interaction
        .begin_command_wait(CommandWaitRequest {
            process_id: args.command_id,
        })
        .await
        .map_err(|err| FunctionCallError::RespondToModel(format!("command_wait failed: {err}")))?;
    let initial_timeout_ms = command_wait.initial_wait_timeout().as_millis() as i64;
    let hard_cap_timeout_ms = command_wait.hard_cap_wait_timeout().as_millis() as i64;
    let wait_timeout = command_wait.initial_wait_timeout();
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
        .emit_model_item_started_display_event(turn.as_ref(), &started_item)
        .await;

    let output = tokio::select! {
        output = command_wait.finish() => {
            let output = output
                .map_err(|err| FunctionCallError::RespondToModel(format!("command_wait failed: {err}")))?;
            thread_service_api
                .reset_thread_wait_backoff(Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>)
                .await;
            output
        }
        poll_result = thread_service_api.poll_event(
            Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
            thread_service_api::ThreadPollEventRequest {
                initial_timeout_ms: Some(initial_timeout_ms),
                hard_cap_timeout_ms: Some(hard_cap_timeout_ms),
            },
        ) => {
            let poll_result = poll_result?;
            if let Some(output) = command_wait
                .try_finish_now()
                .await
                .map_err(|err| FunctionCallError::RespondToModel(format!("command_wait failed: {err}")))? {
                output
            } else {
                CommandWaitOutput {
                    process_id: args.command_id,
                    status: CommandWaitStatus::Running,
                    notification: None,
                    exit_code: None,
                    wall_time: Duration::from_millis(poll_result.waited_ms as u64),
                    wait_timeout: Duration::from_millis(poll_result.current_timeout_ms as u64),
                }
            }
        }
    };

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
    session
        .record_model_items_and_emit_display_events(turn.as_ref(), vec![response_item])
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
