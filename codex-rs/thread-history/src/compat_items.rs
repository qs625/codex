use app_server_protocol::CommandAction;
use app_server_protocol::CommandExecutionNotifyOn;
use app_server_protocol::CommandExecutionSource;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::FileUpdateChange;
use app_server_protocol::PatchApplyStatus;
use app_server_protocol::PatchChangeKind;
use app_server_protocol::ThreadItem;
use protocol::protocol::ApplyPatchApprovalRequestEvent;
use protocol::protocol::ExecCommandBeginEvent;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandNotifyOn as CoreExecCommandNotifyOn;
use protocol::protocol::FileChange;
use protocol::protocol::GuardianAssessmentAction;
use protocol::protocol::GuardianAssessmentEvent;
use protocol::protocol::PatchApplyBeginEvent;
use protocol::protocol::PatchApplyEndEvent;
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn build_file_change_approval_request_item(
    payload: &ApplyPatchApprovalRequestEvent,
) -> ThreadItem {
    ThreadItem::FileChange {
        id: payload.call_id.clone(),
        changes: convert_patch_changes(&payload.changes),
        status: PatchApplyStatus::InProgress,
    }
}

pub(crate) fn build_file_change_begin_item(payload: &PatchApplyBeginEvent) -> ThreadItem {
    ThreadItem::FileChange {
        id: payload.call_id.clone(),
        changes: convert_patch_changes(&payload.changes),
        status: PatchApplyStatus::InProgress,
    }
}

pub(crate) fn build_file_change_end_item(payload: &PatchApplyEndEvent) -> ThreadItem {
    ThreadItem::FileChange {
        id: payload.call_id.clone(),
        changes: convert_patch_changes(&payload.changes),
        status: (&payload.status).into(),
    }
}

pub(crate) fn build_command_execution_begin_item(payload: &ExecCommandBeginEvent) -> ThreadItem {
    ThreadItem::CommandExecution {
        id: payload.call_id.clone(),
        command: format_shell_command(&payload.command),
        cwd: payload.cwd.clone(),
        process_id: payload.process_id.clone(),
        source: payload.source.into(),
        status: CommandExecutionStatus::InProgress,
        initial_wait_ms: payload
            .initial_wait_ms
            .and_then(|value| i64::try_from(value).ok()),
        notify_on: payload.notify_on.map(map_notify_on),
        command_actions: payload
            .parsed_cmd
            .iter()
            .cloned()
            .map(|parsed| CommandAction::from_core_with_cwd(parsed, &payload.cwd))
            .collect(),
        aggregated_output: None,
        exit_code: None,
        duration_ms: None,
    }
}

pub(crate) fn build_command_execution_end_item(payload: &ExecCommandEndEvent) -> ThreadItem {
    let aggregated_output = if payload.aggregated_output.is_empty() {
        None
    } else {
        Some(payload.aggregated_output.clone())
    };
    let duration_ms = i64::try_from(payload.duration.as_millis()).unwrap_or(i64::MAX);

    ThreadItem::CommandExecution {
        id: payload.call_id.clone(),
        command: format_shell_command(&payload.command),
        cwd: payload.cwd.clone(),
        process_id: payload.process_id.clone(),
        source: payload.source.into(),
        status: (&payload.status).into(),
        initial_wait_ms: payload
            .initial_wait_ms
            .and_then(|value| i64::try_from(value).ok()),
        notify_on: payload.notify_on.map(map_notify_on),
        command_actions: payload
            .parsed_cmd
            .iter()
            .cloned()
            .map(|parsed| CommandAction::from_core_with_cwd(parsed, &payload.cwd))
            .collect(),
        aggregated_output,
        exit_code: Some(payload.exit_code),
        duration_ms: Some(duration_ms),
    }
}

pub fn build_item_from_guardian_event(
    assessment: &GuardianAssessmentEvent,
    status: CommandExecutionStatus,
) -> Option<ThreadItem> {
    match &assessment.action {
        GuardianAssessmentAction::Command { command, cwd, .. } => {
            let id = assessment.target_item_id.as_ref()?;
            let command = command.clone();
            let command_actions = vec![CommandAction::Unknown {
                command: command.clone(),
            }];
            Some(ThreadItem::CommandExecution {
                id: id.clone(),
                command,
                cwd: cwd.clone(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status,
                initial_wait_ms: None,
                notify_on: None,
                command_actions,
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            })
        }
        GuardianAssessmentAction::Execve {
            program, argv, cwd, ..
        } => {
            let id = assessment.target_item_id.as_ref()?;
            let argv = if argv.is_empty() {
                vec![program.clone()]
            } else {
                std::iter::once(program.clone())
                    .chain(argv.iter().skip(1).cloned())
                    .collect::<Vec<_>>()
            };
            let command = format_shell_command(&argv);
            let command_actions = vec![CommandAction::Unknown {
                command: command.clone(),
            }];
            Some(ThreadItem::CommandExecution {
                id: id.clone(),
                command,
                cwd: cwd.clone(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status,
                initial_wait_ms: None,
                notify_on: None,
                command_actions,
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            })
        }
        GuardianAssessmentAction::ApplyPatch { .. }
        | GuardianAssessmentAction::NetworkAccess { .. }
        | GuardianAssessmentAction::McpToolCall { .. }
        | GuardianAssessmentAction::RequestPermissions { .. } => None,
    }
}

fn convert_patch_changes(changes: &HashMap<PathBuf, FileChange>) -> Vec<FileUpdateChange> {
    let mut converted: Vec<FileUpdateChange> = changes
        .iter()
        .map(|(path, change)| FileUpdateChange {
            path: path.to_string_lossy().into_owned(),
            kind: map_patch_change_kind(change),
            diff: format_file_change_diff(change),
        })
        .collect();
    converted.sort_by(|a, b| a.path.cmp(&b.path));
    converted
}

fn map_patch_change_kind(change: &FileChange) -> PatchChangeKind {
    match change {
        FileChange::Add { .. } => PatchChangeKind::Add,
        FileChange::Delete { .. } => PatchChangeKind::Delete,
        FileChange::Update { move_path, .. } => PatchChangeKind::Update {
            move_path: move_path.clone(),
        },
    }
}

fn format_file_change_diff(change: &FileChange) -> String {
    match change {
        FileChange::Add { content } => content.clone(),
        FileChange::Delete { content } => content.clone(),
        FileChange::Update {
            unified_diff,
            move_path,
        } => {
            if let Some(path) = move_path {
                format!("{unified_diff}\n\nMoved to: {}", path.display())
            } else {
                unified_diff.clone()
            }
        }
    }
}

fn format_shell_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn map_notify_on(value: CoreExecCommandNotifyOn) -> CommandExecutionNotifyOn {
    match value {
        CoreExecCommandNotifyOn::Output => CommandExecutionNotifyOn::Output,
        CoreExecCommandNotifyOn::Exit => CommandExecutionNotifyOn::Exit,
    }
}

fn shell_escape(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'%' | b'+' | b',' | b'=')) {
        return arg.to_string();
    }

    let escaped = arg.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
