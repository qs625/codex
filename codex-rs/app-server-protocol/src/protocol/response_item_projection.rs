use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::EventCommandEventKind;
use crate::protocol::v2::ThreadItem;
use codex_protocol::event_command::EventCommandEvent;
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
        ResponseItem::EventCommandEvent { id, event } => Some(event_command_event_item(
            id.clone().unwrap_or_else(|| event.stable_item_id()),
            event.clone(),
        )),
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
    if is_event_command_subscribe_tool(namespace, name) {
        return Some(event_command_call_item(
            call_id.to_string(),
            arguments,
            DynamicToolCallStatus::InProgress,
            None,
        ));
    }

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
        ThreadItem::EventCommandCall {
            command,
            cwd,
            label,
            ..
        } => {
            let output_json = event_command_output_payload_to_json(output);
            Some(ThreadItem::EventCommandCall {
                id: call_id.to_string(),
                subscription_id: string_field(&output_json, "subscription_id")
                    .or_else(|| string_field(&output_json, "subscriptionId"))
                    .unwrap_or_default(),
                command: string_field(&output_json, "command").unwrap_or_else(|| command.clone()),
                cwd: string_field(&output_json, "cwd").or_else(|| cwd.clone()),
                label: string_field(&output_json, "label").or_else(|| label.clone()),
                status: DynamicToolCallStatus::Completed,
                output: Some(output_json),
            })
        }
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
pub fn is_structured_response_item_completion(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::EventCommandEvent { .. } | ResponseItem::EventDrivenTool { .. }
    )
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
        "event_command_unsubscribe"
        | "event_command_write_stdin"
        | "schedule_subscribe"
        | "schedule_unsubscribe" => Some(name.to_string()),
        _ => None,
    }
}

fn is_event_command_subscribe_tool(namespace: Option<&str>, name: &str) -> bool {
    namespace.is_none() && name == "event_command_subscribe"
}

fn event_command_call_item(
    id: String,
    arguments: serde_json::Value,
    status: DynamicToolCallStatus,
    output: Option<serde_json::Value>,
) -> ThreadItem {
    ThreadItem::EventCommandCall {
        id,
        subscription_id: output
            .as_ref()
            .and_then(|value| {
                string_field(value, "subscription_id")
                    .or_else(|| string_field(value, "subscriptionId"))
            })
            .unwrap_or_default(),
        command: string_field(&arguments, "command").unwrap_or_default(),
        cwd: string_field(&arguments, "cwd"),
        label: string_field(&arguments, "label"),
        status,
        output,
    }
}

fn event_command_event_item(id: String, event: EventCommandEvent) -> ThreadItem {
    ThreadItem::EventCommandEvent {
        id,
        subscription_id: event.subscription_id,
        kind: EventCommandEventKind::from(event.kind),
        label: event.label,
        command: event.command,
        cwd: event.cwd,
        line: event.line,
        sequence: event.sequence,
        exit_code: event.exit_code,
        signal: event.signal,
        message: event.message,
        truncated: event.truncated,
        created_at: event.created_at,
    }
}

fn parse_raw_function_call_arguments(arguments: &str) -> serde_json::Value {
    serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::Value::String(arguments.into()))
}

fn function_call_output_payload_to_json(output: &FunctionCallOutputPayload) -> serde_json::Value {
    serde_json::to_value(output).unwrap_or_else(|_| serde_json::Value::String(output.to_string()))
}

fn event_command_output_payload_to_json(output: &FunctionCallOutputPayload) -> serde_json::Value {
    output
        .text_content()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| function_call_output_payload_to_json(output))
}

fn string_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}
