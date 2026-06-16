use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::CommandExecutionNotificationKind;
use crate::protocol::v2::CommandWaitNotificationKind;
use crate::protocol::v2::CommandWaitStatus;
use crate::protocol::v2::ThreadItem;
use codex_protocol::models::CommandExecutionNotificationKind as CoreCommandExecutionNotificationKind;
use codex_protocol::models::CommandWaitNotificationKind as CoreCommandWaitNotificationKind;
use codex_protocol::models::CommandWaitStatus as CoreCommandWaitStatus;
use codex_protocol::models::ThreadGoalUpdateGoal;
use codex_protocol::models::ThreadGoalUpdateGoalStatus;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation as CoreInterAgentOperation;

pub(crate) fn thread_goal_from_update_goal(
    goal: &ThreadGoalUpdateGoal,
) -> crate::protocol::v2::ThreadGoal {
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

pub(crate) fn thread_goal_status_from_update_status(
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
