use std::sync::Arc;
use std::time::Duration;

use protocol::exec_output::ExecToolCallOutput;
use protocol::exec_output::StreamOutput;
use protocol::models::CommandExecutionNotificationKind;
use protocol::models::ResponseItem;
use protocol::protocol::ExecCommandBeginEvent;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandNotifyOn;
use protocol::protocol::ExecCommandSource;
use protocol::protocol::ExecCommandStatus;
use protocol::protocol::EventMsg;
use codex_shell_utils::parse_command::parse_command;
use codex_utils_absolute_path::AbsolutePathBuf;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadRuntimeCapability;
use tokio::sync::Mutex;

use super::HeadTailBuffer;
use super::resolve_aggregated_output;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn emit_unified_exec_begin(
    session_ref: Arc<dyn ThreadSessionCapability>,
    turn_ref: Arc<dyn ThreadRuntimeCapability>,
    call_id: &str,
    command: &[String],
    cwd: &AbsolutePathBuf,
    source: ExecCommandSource,
    process_id: Option<String>,
    initial_wait_ms: u64,
    notify_on: ExecCommandNotifyOn,
) {
    session_ref.emit_event(turn_ref.as_ref(), EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
        call_id: call_id.to_string(),
        process_id,
        turn_id: turn_ref.runtime_turn_id_str().to_string(),
        started_at_ms: crate::time_utils::now_unix_timestamp_ms(),
        command: command.to_vec(),
        cwd: cwd.clone(),
        parsed_cmd: parse_command(command),
        source,
        interaction_input: None,
        initial_wait_ms: Some(initial_wait_ms),
        notify_on: Some(notify_on),
    }))
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn emit_unified_exec_end(
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
    status: ExecCommandStatus,
) {
    let aggregated_output = resolve_aggregated_output(&transcript, fallback_output).await;
    let output = ExecToolCallOutput {
        exit_code,
        stdout: StreamOutput::new(aggregated_output.clone()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new(aggregated_output.clone()),
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
        status,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn emit_unified_exec_end_with_output(
    session_ref: Arc<dyn ThreadSessionCapability>,
    turn_ref: Arc<dyn ThreadRuntimeCapability>,
    call_id: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    process_id: Option<String>,
    output: ExecToolCallOutput,
    initial_wait_ms: u64,
    notify_on: ExecCommandNotifyOn,
    status: ExecCommandStatus,
) {
    let completed_at_ms = crate::time_utils::now_unix_timestamp_ms();
    session_ref.emit_event(turn_ref.as_ref(), EventMsg::ExecCommandEnd(ExecCommandEndEvent {
        call_id: call_id.clone(),
        process_id: process_id.clone(),
        turn_id: turn_ref.runtime_turn_id_str().to_string(),
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
        duration: output.duration,
        formatted_output: output.aggregated_output.text.clone(),
        status,
    }))
    .await;

    if process_id.is_some() {
        session_ref.record_model_items_and_emit_display_events(turn_ref.as_ref(), vec![
            ResponseItem::CommandExecutionNotification {
                id: Some(format!("{call_id}:notification:exit")),
                command_item_id: call_id,
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".to_string(),
                output: (!output.aggregated_output.text.is_empty())
                    .then_some(output.aggregated_output.text),
                exit_code: Some(output.exit_code),
                created_at_ms: completed_at_ms,
            },
        ])
        .await;
    }
}
