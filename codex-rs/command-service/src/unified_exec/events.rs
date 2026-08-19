use std::sync::Arc;
use std::time::Duration;

use codex_shell_utils::parse_command::parse_command;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::exec_output::ExecToolCallOutput;
use protocol::exec_output::StreamOutput;
use protocol::models::CommandExecutionNotificationKind;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;
use protocol::protocol::ExecCommandBeginEvent;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandNotifyOn;
use protocol::protocol::ExecCommandSource;
use protocol::protocol::ExecCommandStatus;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tokio::sync::Mutex;

use super::HeadTailBuffer;
use super::bound_command_notification_output;
use super::resolve_aggregated_output;

pub(super) fn command_output_notification_message(command_item_id: &str) -> String {
    format!("Command {command_item_id} produced new output.")
}

pub(super) fn command_exit_notification_message(
    command_item_id: &str,
    output: Option<&str>,
    exit_code: Option<i32>,
) -> String {
    let has_output = output.is_some_and(|output| !output.is_empty());
    match (exit_code, has_output) {
        (Some(code), true) => format!("Command {command_item_id} has exited with code {code}."),
        (Some(code), false) => {
            format!("Command {command_item_id} has exited with code {code} and produced no output.")
        }
        (None, true) => {
            format!("Command {command_item_id} has exited with unknown exit code.")
        }
        (None, false) => {
            format!(
                "Command {command_item_id} has exited with unknown exit code and produced no output."
            )
        }
    }
}

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
    session_ref
        .emit_event(
            turn_ref.as_ref(),
            EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
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
            }),
        )
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
    session_ref
        .emit_event(
            turn_ref.as_ref(),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
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
            }),
        )
        .await;

    if process_id.is_some() {
        let notification_output = (!output.aggregated_output.text.is_empty())
            .then(|| bound_command_notification_output(output.aggregated_output.text));
        let message = command_exit_notification_message(
            &call_id,
            notification_output.as_deref(),
            Some(output.exit_code),
        );
        session_ref
            .record_model_items_and_emit_display_events(
                turn_ref.as_ref(),
                vec![ResponseItem::CommandExecutionNotification {
                    id: Some(format!("{call_id}:notification:exit")),
                    command_item_id: call_id,
                    kind: CommandExecutionNotificationKind::Exit,
                    message,
                    output: notification_output,
                    exit_code: Some(output.exit_code),
                    created_at_ms: completed_at_ms,
                }],
            )
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_notification_message_names_command() {
        assert_eq!(
            command_output_notification_message("cmd-1"),
            "Command cmd-1 produced new output."
        );
    }

    #[test]
    fn command_exit_notification_message_describes_exit_code_and_output() {
        assert_eq!(
            command_exit_notification_message("cmd-1", Some("hello"), Some(0)),
            "Command cmd-1 has exited with code 0."
        );
        assert_eq!(
            command_exit_notification_message("cmd-1", None, Some(0)),
            "Command cmd-1 has exited with code 0 and produced no output."
        );
        assert_eq!(
            command_exit_notification_message("cmd-1", Some("hello"), None),
            "Command cmd-1 has exited with unknown exit code."
        );
        assert_eq!(
            command_exit_notification_message("cmd-1", None, None),
            "Command cmd-1 has exited with unknown exit code and produced no output."
        );
    }
}
