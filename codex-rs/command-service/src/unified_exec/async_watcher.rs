use std::pin::Pin;
use std::sync::Arc;

use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_service_api::CommandServiceTurnCapability;
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
use super::events::emit_unified_exec_end;
use super::events::emit_unified_exec_end_with_output;
use crate::time_utils::now_unix_timestamp_ms;
use codex_command_runtime::resolve_aggregated_output;
use codex_command_runtime::split_valid_utf8_prefix;
use codex_command_runtime::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::models::CommandExecutionNotificationKind;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandNotifyOn;
use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
use codex_protocol::protocol::ExecCommandStatus;
use codex_protocol::protocol::ExecOutputStream;
use codex_utils_absolute_path::AbsolutePathBuf;

pub(crate) const TRAILING_OUTPUT_GRACE: Duration = Duration::from_millis(100);

/// Spawn a background task that continuously reads from the PTY, appends to the
/// shared transcript, and emits ExecCommandOutputDelta events on UTF‑8
/// boundaries.
pub(crate) fn start_streaming_output(
    process: &UnifiedExecProcess,
    context: &UnifiedExecContext,
    transcript: Arc<Mutex<HeadTailBuffer>>,
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
    session_ref: Arc<dyn CommandServiceSessionCapability>,
    turn_ref: Arc<dyn CommandServiceTurnCapability>,
    call_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: i32,
    transcript: Arc<Mutex<HeadTailBuffer>>,
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
        let process_id = notification_state
            .is_background_session_active()
            .then(|| process_id.to_string());
        if let Some(message) = process.failure_message() {
            emit_failed_exec_end_for_unified_exec(
                session_ref,
                turn_ref,
                call_id,
                command,
                cwd,
                process_id,
                transcript,
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
                session_ref,
                turn_ref,
                call_id,
                command,
                cwd,
                process_id,
                transcript,
                String::new(),
                exit_code,
                duration,
                initial_wait_ms,
                command_notification_filter_to_protocol(notify_on),
            )
            .await;
        }
        if notification_state.is_background_session_active() {
            notification_state
                .notify(CommandNotificationKind::Exit)
                .await;
        }
    });
}

async fn process_chunk(
    pending: &mut Vec<u8>,
    transcript: &Arc<Mutex<HeadTailBuffer>>,
    call_id: &str,
    session_ref: &Arc<dyn CommandServiceSessionCapability>,
    turn_ref: &Arc<dyn CommandServiceTurnCapability>,
    emitted_deltas: &mut usize,
    notify_on: CommandNotificationFilter,
    notification_state: &Arc<CommandNotificationState>,
    chunk: Vec<u8>,
) {
    pending.extend_from_slice(&chunk);
    while let Some(prefix) = split_valid_utf8_prefix(pending) {
        {
            let mut guard = transcript.lock().await;
            guard.push_chunk(prefix.to_vec());
        }

        if *emitted_deltas >= MAX_EXEC_OUTPUT_DELTAS_PER_CALL {
            continue;
        }

        let generates_notification = matches!(notify_on, CommandNotificationFilter::Output)
            && notification_state.is_background_session_active();
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
            .send_event(turn_ref.as_ref(), EventMsg::ExecCommandOutputDelta(event))
            .await;
        *emitted_deltas += 1;
        if generates_notification {
            let output = String::from_utf8_lossy(&prefix).to_string();
            let item = ResponseItem::CommandExecutionNotification {
                id: Some(format!("{call_id}:notification:output:{sequence}")),
                command_item_id: call_id.to_string(),
                kind: CommandExecutionNotificationKind::Output,
                message: "Command output notification received.".to_string(),
                output: Some(output),
                exit_code: None,
                created_at_ms: now_unix_timestamp_ms(),
            };
            session_ref
                .record_model_items_and_emit_display_events(turn_ref.as_ref(), &[item])
                .await;
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
    session_ref: Arc<dyn CommandServiceSessionCapability>,
    turn_ref: Arc<dyn CommandServiceTurnCapability>,
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
    session_ref: Arc<dyn CommandServiceSessionCapability>,
    turn_ref: Arc<dyn CommandServiceTurnCapability>,
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
