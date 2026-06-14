use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::ThreadItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
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
        ResponseItem::WorkflowRunProgress { id, event } => Some(ThreadItem::WorkflowRunProgress {
            id: id.clone().unwrap_or_else(fallback_id),
            event: event.clone().into(),
        }),
        ResponseItem::EventDrivenTool { id, trigger } => Some(ThreadItem::EventDrivenTool {
            id: id.clone().unwrap_or_else(fallback_id),
            tool: trigger.tool.clone(),
            title: trigger.title.clone(),
            text: trigger.text.clone(),
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
        "schedule_subscribe" | "schedule_unsubscribe" => Some(name.to_string()),
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

    #[test]
    fn event_command_event_is_not_projected_to_thread_item() {
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
            None,
        );
    }
}
