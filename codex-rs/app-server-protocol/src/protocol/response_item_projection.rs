use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::CommandExecutionNotificationKind;
use crate::protocol::v2::CommandWaitNotificationKind;
use crate::protocol::v2::CommandWaitStatus;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::EventCommandEventKind;
use crate::protocol::v2::ThreadItem;
use crate::protocol::v2::ThreadGoalUpdateAction;
use crate::protocol::v2::ThreadGoalUpdateSource;
use codex_protocol::models::CommandExecutionNotificationKind as CoreCommandExecutionNotificationKind;
use codex_protocol::models::CommandWaitNotificationKind as CoreCommandWaitNotificationKind;
use codex_protocol::models::CommandWaitStatus as CoreCommandWaitStatus;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::ThreadGoalUpdateGoal;
use codex_protocol::models::ThreadGoalUpdateGoalStatus;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation as CoreInterAgentOperation;

#[doc(hidden)]
pub fn project_structured_response_item<F>(
    item: &ResponseItem,
    fallback_id: F,
) -> Option<ThreadItem>
where
    F: Fn() -> String,
{
    match item {
        ResponseItem::CommandWait {
            id,
            command_id,
            status,
            notification,
            exit_code,
            wall_time_seconds,
            created_at_ms,
        } => Some(ThreadItem::CommandWait {
            id: id.clone().unwrap_or_else(fallback_id),
            command_id: command_id.clone(),
            status: CommandWaitStatus::from(*status),
            notification: notification.map(CommandWaitNotificationKind::from),
            exit_code: *exit_code,
            wall_time_seconds: *wall_time_seconds,
            created_at_ms: *created_at_ms,
        }),
        ResponseItem::CommandWriteStdin {
            id,
            command_id,
            bytes_written,
            contains_newline,
            created_at_ms,
        } => Some(ThreadItem::CommandWriteStdin {
            id: id.clone().unwrap_or_else(fallback_id),
            command_id: command_id.clone(),
            bytes_written: *bytes_written,
            contains_newline: *contains_newline,
            created_at_ms: *created_at_ms,
        }),
        ResponseItem::CommandExecutionNotification {
            id,
            command_item_id,
            kind,
            message,
            output,
            exit_code,
            created_at_ms,
        } => Some(ThreadItem::CommandExecutionNotification {
            id: id.clone().unwrap_or_else(fallback_id),
            command_item_id: command_item_id.clone(),
            kind: CommandExecutionNotificationKind::from(*kind),
            message: message.clone(),
            output: output.clone(),
            exit_code: *exit_code,
            created_at_ms: *created_at_ms,
        }),
        ResponseItem::WorkflowRunProgress { id, event } => Some(ThreadItem::WorkflowRunProgress {
            id: id.clone().unwrap_or_else(fallback_id),
            event: event.clone().into(),
        }),
        ResponseItem::EventCommandEvent { id, event } => Some(ThreadItem::EventCommandEvent {
            id: id.clone().unwrap_or_else(|| event.stable_item_id()),
            subscription_id: event.subscription_id.clone(),
            kind: EventCommandEventKind::from(event.kind.clone()),
            label: event.label.clone(),
            command: event.command.clone(),
            cwd: event.cwd.clone(),
            line: event.line.clone(),
            sequence: event.sequence,
            exit_code: event.exit_code,
            signal: event.signal.clone(),
            message: event.message.clone(),
            truncated: event.truncated,
            created_at: event.created_at,
        }),
        ResponseItem::EventDrivenTool { id, trigger } => Some(ThreadItem::EventDrivenTool {
            id: id.clone().unwrap_or_else(fallback_id),
            tool: trigger.tool.clone(),
            title: trigger.title.clone(),
            text: trigger.text.clone(),
        }),
        ResponseItem::ThreadGoalUpdate {
            id,
            goal,
            action,
            source,
            previous_status,
        } => Some(ThreadItem::ThreadGoalUpdate {
            id: id.clone().unwrap_or_else(fallback_id),
            goal: thread_goal_from_update_goal(goal),
            action: ThreadGoalUpdateAction::from(*action),
            source: ThreadGoalUpdateSource::from(*source),
            previous_status: previous_status.map(thread_goal_status_from_update_status),
        }),
        ResponseItem::InterAgentCommunication { id, communication }
            if !matches!(communication.operation, CoreInterAgentOperation::Unknown) =>
        {
            Some(thread_item_from_inter_agent_communication(
                id.clone().unwrap_or_else(fallback_id),
                communication.clone(),
            ))
        }
        _ => None,
    }
}

fn thread_goal_from_update_goal(goal: &ThreadGoalUpdateGoal) -> crate::protocol::v2::ThreadGoal {
    crate::protocol::v2::ThreadGoal {
        thread_id: goal.thread_id.to_string(),
        objective: goal.objective.clone(),
        status: thread_goal_status_from_update_status(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at: goal.created_at,
        updated_at: goal.updated_at,
    }
}

fn thread_goal_status_from_update_status(
    status: ThreadGoalUpdateGoalStatus,
) -> crate::protocol::v2::ThreadGoalStatus {
    match status {
        ThreadGoalUpdateGoalStatus::Active => crate::protocol::v2::ThreadGoalStatus::Active,
        ThreadGoalUpdateGoalStatus::Paused => crate::protocol::v2::ThreadGoalStatus::Paused,
        ThreadGoalUpdateGoalStatus::BudgetLimited => {
            crate::protocol::v2::ThreadGoalStatus::BudgetLimited
        }
        ThreadGoalUpdateGoalStatus::Complete => crate::protocol::v2::ThreadGoalStatus::Complete,
    }
}

impl From<CoreCommandExecutionNotificationKind> for CommandExecutionNotificationKind {
    fn from(value: CoreCommandExecutionNotificationKind) -> Self {
        match value {
            CoreCommandExecutionNotificationKind::Output => Self::Output,
            CoreCommandExecutionNotificationKind::Exit => Self::Exit,
        }
    }
}

impl From<CoreCommandWaitStatus> for CommandWaitStatus {
    fn from(value: CoreCommandWaitStatus) -> Self {
        match value {
            CoreCommandWaitStatus::Running => Self::Running,
            CoreCommandWaitStatus::Completed => Self::Completed,
        }
    }
}

impl From<CoreCommandWaitNotificationKind> for CommandWaitNotificationKind {
    fn from(value: CoreCommandWaitNotificationKind) -> Self {
        match value {
            CoreCommandWaitNotificationKind::Output => Self::Output,
            CoreCommandWaitNotificationKind::Exit => Self::Exit,
        }
    }
}

#[doc(hidden)]
pub fn project_tool_call_start(
    name: &str,
    namespace: Option<&str>,
    arguments: &str,
    call_id: &str,
) -> Option<ThreadItem> {
    let arguments = parse_raw_function_call_arguments(arguments);
    subscription_tool_name(namespace, name).map(|tool| ThreadItem::EventDrivenToolCall {
        id: call_id.to_string(),
        tool,
        arguments,
        status: DynamicToolCallStatus::InProgress,
        output: None,
    })
}

#[doc(hidden)]
pub fn project_tool_call_completion(
    existing: &ThreadItem,
    call_id: &str,
    output: &FunctionCallOutputPayload,
) -> Option<ThreadItem> {
    match existing {
        ThreadItem::EventDrivenToolCall {
            tool, arguments, ..
        } => Some(ThreadItem::EventDrivenToolCall {
            id: call_id.to_string(),
            tool: tool.clone(),
            arguments: arguments.clone(),
            status: DynamicToolCallStatus::Completed,
            output: Some(function_call_output_payload_to_json(output)),
        }),
        _ => None,
    }
}

#[doc(hidden)]
pub fn thread_item_from_inter_agent_communication(
    id: String,
    communication: InterAgentCommunication,
) -> ThreadItem {
    if matches!(
        communication.operation,
        CoreInterAgentOperation::ChildCompletion
    ) && let Some(mut status) = communication.status.map(CollabAgentState::from)
    {
        status.path = Some(communication.author.to_string());
        return ThreadItem::CollabAgentStatusUpdate {
            id,
            sender_thread_id: communication
                .sender_thread_id
                .map(|value| value.to_string()),
            sender_path: communication.author.to_string(),
            recipient_thread_id: communication
                .recipient_thread_id
                .map(|value| value.to_string()),
            recipient_path: communication.recipient.to_string(),
            status,
        };
    }

    ThreadItem::CollabAgentMessage {
        id,
        operation: communication.operation.into(),
        sender_thread_id: communication
            .sender_thread_id
            .map(|value| value.to_string()),
        sender_path: communication.author.to_string(),
        recipient_thread_id: communication
            .recipient_thread_id
            .map(|value| value.to_string()),
        recipient_path: communication.recipient.to_string(),
        other_recipient_paths: communication
            .other_recipients
            .into_iter()
            .map(|path| path.to_string())
            .collect(),
        content: communication.content,
        trigger_turn: communication.trigger_turn,
    }
}

fn subscription_tool_name(namespace: Option<&str>, name: &str) -> Option<String> {
    if namespace.is_some() {
        return None;
    }

    match name {
        "schedule_subscribe"
        | "schedule_unsubscribe"
        | "event_command_write_stdin"
        | "command_wait"
        | "command_write_stdin" => Some(name.to_string()),
        _ => None,
    }
}

fn parse_raw_function_call_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::Value::String(arguments.into()))
}

#[doc(hidden)]
pub fn is_legacy_structured_assistant_message_text(text: &str) -> bool {
    let trimmed = text.trim();
    if is_wrapped_marker(trimmed, "<event_driven_tool>", "</event_driven_tool>")
        || is_wrapped_marker(trimmed, "<event_command>", "</event_command>")
        || is_wrapped_marker(
            trimmed,
            "<subagent_notification>",
            "</subagent_notification>",
        )
    {
        return true;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    if !object.contains_key("author") || !object.contains_key("recipient") {
        return false;
    }
    matches!(
        object.get("operation").and_then(serde_json::Value::as_str),
        Some("spawnAgent" | "sendMessage" | "send_message" | "followupTask" | "childCompletion")
    )
}

fn is_wrapped_marker(trimmed: &str, start_marker: &str, end_marker: &str) -> bool {
    trimmed.starts_with(start_marker) && trimmed.ends_with(end_marker)
}

fn function_call_output_payload_to_json(output: &FunctionCallOutputPayload) -> serde_json::Value {
    serde_json::to_value(output).unwrap_or_else(|_| serde_json::Value::String(output.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::event_command::EventCommandEvent;
    use codex_protocol::event_command::EventCommandEventKind;
    use codex_protocol::models::ThreadGoalUpdateEventAction as CoreThreadGoalUpdateEventAction;
    use codex_protocol::models::ThreadGoalUpdateEventSource as CoreThreadGoalUpdateEventSource;
    use codex_protocol::models::ThreadGoalUpdateGoal;
    use codex_protocol::models::ThreadGoalUpdateGoalStatus;
    use codex_protocol::ThreadId;

    #[test]
    fn event_command_event_projects_to_thread_item() {
        let event = EventCommandEvent {
            subscription_id: "sub-command".to_string(),
            kind: EventCommandEventKind::Output,
            label: Some("build log".to_string()),
            command: "tail -f /tmp/build.log".to_string(),
            cwd: Some("/repo".to_string()),
            line: Some("done".to_string()),
            sequence: Some(1),
            exit_code: None,
            signal: None,
            message: None,
            truncated: false,
            created_at: 1,
        };

        assert_eq!(
            project_structured_response_item(
                &ResponseItem::EventCommandEvent {
                    id: Some("typed-event-command".to_string()),
                    event,
                },
                || "fallback".to_string(),
            ),
            Some(ThreadItem::EventCommandEvent {
                id: "typed-event-command".to_string(),
                subscription_id: "sub-command".to_string(),
                kind: crate::protocol::v2::EventCommandEventKind::Output,
                label: Some("build log".to_string()),
                command: "tail -f /tmp/build.log".to_string(),
                cwd: Some("/repo".to_string()),
                line: Some("done".to_string()),
                sequence: Some(1),
                exit_code: None,
                signal: None,
                message: None,
                truncated: false,
                created_at: 1,
            }),
        );
    }

    #[test]
    fn thread_goal_update_projects_to_thread_item() {
        let thread_id = ThreadId::new();
        let goal = ThreadGoalUpdateGoal {
            thread_id: thread_id.clone(),
            objective: "Ship goal display".to_string(),
            status: ThreadGoalUpdateGoalStatus::Active,
            token_budget: Some(50_000),
            tokens_used: 1_250,
            time_used_seconds: 75,
            created_at: 1,
            updated_at: 2,
        };
        let expected_goal = crate::protocol::v2::ThreadGoal {
            thread_id: thread_id.to_string(),
            objective: "Ship goal display".to_string(),
            status: crate::protocol::v2::ThreadGoalStatus::Active,
            token_budget: Some(50_000),
            tokens_used: 1_250,
            time_used_seconds: 75,
            created_at: 1,
            updated_at: 2,
        };

        assert_eq!(
            project_structured_response_item(
                &ResponseItem::ThreadGoalUpdate {
                    id: Some("goal-item".to_string()),
                    goal: goal.clone(),
                    action: CoreThreadGoalUpdateEventAction::Created,
                    source: CoreThreadGoalUpdateEventSource::ModelTool,
                    previous_status: None,
                },
                || "fallback".to_string(),
            ),
            Some(ThreadItem::ThreadGoalUpdate {
                id: "goal-item".to_string(),
                goal: expected_goal,
                action: crate::protocol::v2::ThreadGoalUpdateAction::Created,
                source: crate::protocol::v2::ThreadGoalUpdateSource::ModelTool,
                previous_status: None,
            })
        );
    }
}
