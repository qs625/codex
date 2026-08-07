use std::pin::Pin;
use std::sync::Arc;

use command_service_api::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tokio::sync::Mutex;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio::time::Sleep;

use super::CommandNotificationFilter;
use super::CommandNotificationKind;
use super::CommandNotificationState;
use super::HeadTailBuffer;
use super::UnifiedExecContext;
use super::UnifiedExecProcess;
use super::command_notification_filter_to_protocol;
use super::events::command_exit_notification_message;
use super::events::command_output_notification_message;
use super::events::emit_unified_exec_end;
use super::events::emit_unified_exec_end_with_output;
use super::resolve_aggregated_output;
use super::split_valid_utf8_prefix;
use crate::time_utils::now_unix_timestamp_ms;
use codex_shell_utils::parse_command::parse_command;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::exec_output::ExecToolCallOutput;
use protocol::exec_output::StreamOutput;
use protocol::models::CommandExecutionNotificationKind;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandNotifyOn;
use protocol::protocol::ExecCommandOutputDeltaEvent;
use protocol::protocol::ExecCommandSource;
use protocol::protocol::ExecCommandStatus;
use protocol::protocol::ExecOutputStream;

pub(crate) const TRAILING_OUTPUT_GRACE: Duration = Duration::from_millis(100);

/// Spawn a background task that continuously reads from the PTY, appends to the
/// shared transcript, and emits ExecCommandOutputDelta events on UTF‑8
/// boundaries.
pub(crate) fn start_streaming_output(
    process: &UnifiedExecProcess,
    context: &UnifiedExecContext,
    transcript: Arc<Mutex<HeadTailBuffer>>,
    exit_notification_output: Arc<Mutex<HeadTailBuffer>>,
    notify_on: CommandNotificationFilter,
    notification_state: Arc<CommandNotificationState>,
) {
    let mut receiver = process.output_receiver();
    let output_drained = process.output_drained_notify();
    let exit_token = process.cancellation_token();

    let session_ref = Arc::clone(&context.session);
    let turn_ref = Arc::clone(&context.turn);
    let call_id = context.call_id.clone();

    tokio::spawn(async move {
        use tokio::sync::broadcast::error::RecvError;

        let mut pending = Vec::<u8>::new();
        let mut emitted_deltas: usize = 0;

        let mut grace_sleep: Option<Pin<Box<Sleep>>> = None;

        loop {
            tokio::select! {
                _ = exit_token.cancelled(), if grace_sleep.is_none() => {
                    let deadline = Instant::now() + TRAILING_OUTPUT_GRACE;
                    grace_sleep.replace(Box::pin(tokio::time::sleep_until(deadline)));
                }

                _ = async {
                    if let Some(sleep) = grace_sleep.as_mut() {
                        sleep.as_mut().await;
                    }
                }, if grace_sleep.is_some() => {
                    output_drained.notify_one();
                    break;
                }

                received = receiver.recv() => {
                    let chunk = match received {
                        Ok(chunk) => chunk,
                        Err(RecvError::Lagged(_)) => {
                            continue;
                        },
                        Err(RecvError::Closed) => {
                            output_drained.notify_one();
                            break;
                        }
                    };

                    process_chunk(
                        &mut pending,
                        &transcript,
                        &exit_notification_output,
                        &call_id,
                        &session_ref,
                        &turn_ref,
                        &mut emitted_deltas,
                        notify_on,
                        &notification_state,
                        chunk,
                    ).await;
                }
            }
        }
    });
}

/// Spawn a background watcher that waits for the PTY to exit and then emits a
/// single ExecCommandEnd event with the aggregated transcript.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_exit_watcher(
    process: Arc<UnifiedExecProcess>,
    session_ref: Arc<dyn ThreadSessionCapability>,
    turn_ref: Arc<dyn ThreadRuntimeCapability>,
    call_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: i32,
    transcript: Arc<Mutex<HeadTailBuffer>>,
    exit_notification_output: Arc<Mutex<HeadTailBuffer>>,
    started_at: Instant,
    notification_state: Arc<CommandNotificationState>,
    initial_wait_ms: u64,
    notify_on: CommandNotificationFilter,
) {
    let exit_token = process.cancellation_token();
    let output_drained = process.output_drained_notify();

    tokio::spawn(async move {
        exit_token.cancelled().await;
        output_drained.notified().await;

        let duration = Instant::now().saturating_duration_since(started_at);
        let background_session_active = notification_state.is_background_session_active();
        let process_id = background_session_active.then(|| process_id.to_string());
        let failure_message = process.failure_message();
        if background_session_active {
            let status = if failure_message.is_some() {
                ExecCommandStatus::Failed
            } else {
                ExecCommandStatus::Completed
            };
            let output = exit_output_for_unified_exec(
                Arc::clone(&transcript),
                String::new(),
                process.exit_code().unwrap_or(-1),
                failure_message.clone(),
                duration,
            )
            .await;
            let completed_at_ms = now_unix_timestamp_ms();
            let event = exec_end_event_for_unified_exec(
                call_id.clone(),
                turn_ref.runtime_turn_id_str().to_string(),
                command.clone(),
                cwd.clone(),
                process_id,
                output.clone(),
                duration,
                initial_wait_ms,
                command_notification_filter_to_protocol(notify_on),
                status,
                completed_at_ms,
            );
            let notification_output = resolve_exit_notification_output(
                &transcript,
                &exit_notification_output,
                notify_on,
                failure_message.as_deref(),
            )
            .await;
            let item = ResponseItem::CommandExecutionNotification {
                id: Some(format!("{call_id}:notification:exit")),
                command_item_id: call_id.clone(),
                kind: CommandExecutionNotificationKind::Exit,
                message: command_exit_notification_message(
                    &call_id,
                    notification_output.as_deref(),
                    process.exit_code(),
                ),
                output: notification_output,
                exit_code: process.exit_code(),
                created_at_ms: completed_at_ms,
            };
            let _ = session_ref
                .append_conversation_item_with_observed_event(item, event)
                .await;
            notification_state
                .notify(CommandNotificationKind::Exit)
                .await;
            return;
        }
        if let Some(message) = failure_message.clone() {
            emit_failed_exec_end_for_unified_exec(
                Arc::clone(&session_ref),
                Arc::clone(&turn_ref),
                call_id.clone(),
                command.clone(),
                cwd.clone(),
                process_id,
                Arc::clone(&transcript),
                String::new(),
                message,
                duration,
                initial_wait_ms,
                command_notification_filter_to_protocol(notify_on),
            )
            .await;
        } else {
            let exit_code = process.exit_code().unwrap_or(-1);
            emit_exec_end_for_unified_exec(
                Arc::clone(&session_ref),
                Arc::clone(&turn_ref),
                call_id.clone(),
                command.clone(),
                cwd.clone(),
                process_id,
                Arc::clone(&transcript),
                String::new(),
                exit_code,
                duration,
                initial_wait_ms,
                command_notification_filter_to_protocol(notify_on),
            )
            .await;
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn exec_end_event_for_unified_exec(
    call_id: String,
    turn_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: Option<String>,
    output: ExecToolCallOutput,
    duration: Duration,
    initial_wait_ms: u64,
    notify_on: ExecCommandNotifyOn,
    status: ExecCommandStatus,
    completed_at_ms: i64,
) -> EventMsg {
    EventMsg::ExecCommandEnd(ExecCommandEndEvent {
        call_id,
        process_id,
        turn_id,
        completed_at_ms,
        command: command.clone(),
        cwd,
        parsed_cmd: parse_command(&command),
        source: ExecCommandSource::UnifiedExecStartup,
        interaction_input: None,
        initial_wait_ms: Some(initial_wait_ms),
        notify_on: Some(notify_on),
        stdout: output.stdout.text.clone(),
        stderr: output.stderr.text.clone(),
        aggregated_output: output.aggregated_output.text.clone(),
        exit_code: output.exit_code,
        duration,
        formatted_output: output.aggregated_output.text,
        status,
    })
}

async fn exit_output_for_unified_exec(
    transcript: Arc<Mutex<HeadTailBuffer>>,
    fallback_output: String,
    exit_code: i32,
    failure_message: Option<String>,
    duration: Duration,
) -> ExecToolCallOutput {
    let stdout = resolve_aggregated_output(&transcript, fallback_output).await;
    let (stderr, aggregated_output) = match failure_message {
        Some(message) if stdout.is_empty() => (message.clone(), message),
        Some(message) => (message.clone(), format!("{stdout}\n{message}")),
        None => (String::new(), stdout.clone()),
    };
    ExecToolCallOutput {
        exit_code: if stderr.is_empty() { exit_code } else { -1 },
        stdout: StreamOutput::new(stdout),
        stderr: StreamOutput::new(stderr),
        aggregated_output: StreamOutput::new(aggregated_output),
        duration,
        timed_out: false,
    }
}

async fn resolve_exit_notification_output(
    transcript: &Arc<Mutex<HeadTailBuffer>>,
    exit_notification_output: &Arc<Mutex<HeadTailBuffer>>,
    notify_on: CommandNotificationFilter,
    failure_message: Option<&str>,
) -> Option<String> {
    let output_buffer = match notify_on {
        CommandNotificationFilter::Exit => transcript,
        CommandNotificationFilter::Output => exit_notification_output,
    };
    let stdout = resolve_aggregated_output(output_buffer, String::new()).await;
    let output = match failure_message {
        Some(message) if stdout.is_empty() => message.to_string(),
        Some(message) => format!("{stdout}\n{message}"),
        None => stdout,
    };
    (!output.is_empty()).then_some(output)
}

#[allow(clippy::too_many_arguments)]
async fn process_chunk(
    pending: &mut Vec<u8>,
    transcript: &Arc<Mutex<HeadTailBuffer>>,
    exit_notification_output: &Arc<Mutex<HeadTailBuffer>>,
    call_id: &str,
    session_ref: &Arc<dyn ThreadSessionCapability>,
    turn_ref: &Arc<dyn ThreadRuntimeCapability>,
    emitted_deltas: &mut usize,
    notify_on: CommandNotificationFilter,
    notification_state: &Arc<CommandNotificationState>,
    chunk: Vec<u8>,
) {
    pending.extend_from_slice(&chunk);
    while let Some(prefix) = split_valid_utf8_prefix(pending) {
        let background_session_active = notification_state.is_background_session_active();
        {
            let mut guard = transcript.lock().await;
            guard.push_chunk(prefix.to_vec());
        }
        if matches!(notify_on, CommandNotificationFilter::Output) && background_session_active {
            let mut guard = exit_notification_output.lock().await;
            guard.push_chunk(prefix.to_vec());
        }

        if *emitted_deltas >= MAX_EXEC_OUTPUT_DELTAS_PER_CALL {
            continue;
        }

        let generates_notification =
            matches!(notify_on, CommandNotificationFilter::Output) && background_session_active;
        let sequence = *emitted_deltas as u64 + 1;
        let event = ExecCommandOutputDeltaEvent {
            call_id: call_id.to_string(),
            sequence: Some(sequence),
            generates_notification,
            created_at_ms: now_unix_timestamp_ms(),
            stream: ExecOutputStream::Stdout,
            chunk: prefix.clone(),
        };
        session_ref
            .emit_event(turn_ref.as_ref(), EventMsg::ExecCommandOutputDelta(event))
            .await;
        *emitted_deltas += 1;
        if generates_notification {
            let output = String::from_utf8_lossy(&prefix).to_string();
            let item = ResponseItem::CommandExecutionNotification {
                id: Some(format!("{call_id}:notification:output:{sequence}")),
                command_item_id: call_id.to_string(),
                kind: CommandExecutionNotificationKind::Output,
                message: command_output_notification_message(&call_id),
                output: Some(output),
                exit_code: None,
                created_at_ms: now_unix_timestamp_ms(),
            };
            let _ = session_ref.append_conversation_item(item).await;
            {
                let mut guard = exit_notification_output.lock().await;
                let _ = guard.drain_chunks();
            }
            notification_state
                .notify(CommandNotificationKind::Output)
                .await;
        }
    }
}

/// Emit an ExecCommandEnd event for a unified exec session, using the transcript
/// as the primary source of aggregated_output and falling back to the provided
/// text when the transcript is empty.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn emit_exec_end_for_unified_exec(
    session_ref: Arc<dyn ThreadSessionCapability>,
    turn_ref: Arc<dyn ThreadRuntimeCapability>,
    call_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: Option<String>,
    transcript: Arc<Mutex<HeadTailBuffer>>,
    fallback_output: String,
    exit_code: i32,
    duration: Duration,
    initial_wait_ms: u64,
    notify_on: ExecCommandNotifyOn,
) {
    emit_unified_exec_end(
        session_ref,
        turn_ref,
        call_id,
        command,
        cwd,
        process_id,
        transcript,
        fallback_output,
        exit_code,
        duration,
        initial_wait_ms,
        notify_on,
        ExecCommandStatus::Completed,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn emit_failed_exec_end_for_unified_exec(
    session_ref: Arc<dyn ThreadSessionCapability>,
    turn_ref: Arc<dyn ThreadRuntimeCapability>,
    call_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: Option<String>,
    transcript: Arc<Mutex<HeadTailBuffer>>,
    fallback_output: String,
    message: String,
    duration: Duration,
    initial_wait_ms: u64,
    notify_on: ExecCommandNotifyOn,
) {
    let stdout = if fallback_output.is_empty() {
        resolve_aggregated_output(&transcript, fallback_output).await
    } else {
        fallback_output
    };
    let aggregated_output = if stdout.is_empty() {
        message.clone()
    } else {
        format!("{stdout}\n{message}")
    };
    let output = ExecToolCallOutput {
        exit_code: -1,
        stdout: StreamOutput::new(stdout),
        stderr: StreamOutput::new(message),
        aggregated_output: StreamOutput::new(aggregated_output),
        duration,
        timed_out: false,
    };
    emit_unified_exec_end_with_output(
        session_ref,
        turn_ref,
        call_id,
        command,
        cwd,
        process_id,
        output,
        initial_wait_ms,
        notify_on,
        ExecCommandStatus::Failed,
    )
    .await;
}

#[cfg(test)]
#[path = "async_watcher_tests.rs"]
mod tests;
