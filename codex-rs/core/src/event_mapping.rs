use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::CollabAgentMessageItem;
use codex_protocol::items::EventDrivenToolItem;
use codex_protocol::items::InjectedContextItem;
use codex_protocol::items::InjectedContextSection;
use codex_protocol::items::ReasoningItem;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::items::WebSearchItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WebSearchAction;
use codex_protocol::models::is_image_close_tag_text;
use codex_protocol::models::is_image_open_tag_text;
use codex_protocol::models::is_local_image_close_tag_text;
use codex_protocol::models::is_local_image_open_tag_text;
use codex_protocol::protocol::CommandExecutionNotificationDisplayEvent;
use codex_protocol::protocol::CommandWaitDisplayEvent;
use codex_protocol::protocol::CommandWriteStdinDisplayEvent;
use codex_protocol::protocol::EventCommandDisplayEvent;
use codex_protocol::protocol::EventDrivenToolDisplayEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentCommunicationDisplayEvent;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::protocol::ThreadGoalUpdateDisplayEvent;
use codex_protocol::protocol::WorkflowRunProgressDisplayEvent;
use codex_protocol::user_input::UserInput;
use tracing::warn;
use uuid::Uuid;

use crate::context::parse_visible_hook_prompt_message;
use crate::web_search::web_search_action_detail;
pub(crate) use codex_context_manager::is_contextual_user_message_content;

fn parse_user_message(message: &[ContentItem]) -> Option<UserMessageItem> {
    if is_contextual_user_message_content(message) {
        return None;
    }

    let mut content: Vec<UserInput> = Vec::new();

    for (idx, content_item) in message.iter().enumerate() {
        match content_item {
            ContentItem::InputText { text } => {
                if (is_local_image_open_tag_text(text) || is_image_open_tag_text(text))
                    && (matches!(message.get(idx + 1), Some(ContentItem::InputImage { .. })))
                    || (idx > 0
                        && (is_local_image_close_tag_text(text) || is_image_close_tag_text(text))
                        && matches!(message.get(idx - 1), Some(ContentItem::InputImage { .. })))
                {
                    continue;
                }
                content.push(UserInput::Text {
                    text: text.clone(),
                    // Model input content does not carry UI element ranges.
                    text_elements: Vec::new(),
                });
            }
            ContentItem::InputImage { image_url, .. } => {
                content.push(UserInput::Image {
                    image_url: image_url.clone(),
                });
            }
            ContentItem::OutputText { text } => {
                warn!("Output text in user message: {}", text);
            }
        }
    }

    Some(UserMessageItem::new(&content))
}

fn parse_agent_message(
    id: Option<&String>,
    message: &[ContentItem],
    phase: Option<MessagePhase>,
) -> AgentMessageItem {
    let mut content: Vec<AgentMessageContent> = Vec::new();
    for content_item in message.iter() {
        match content_item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                content.push(AgentMessageContent::Text { text: text.clone() });
            }
            _ => {
                warn!(
                    "Unexpected content item in agent message: {:?}",
                    content_item
                );
            }
        }
    }
    let id = id.cloned().unwrap_or_else(|| Uuid::new_v4().to_string());
    AgentMessageItem {
        id,
        content,
        phase,
        memory_citation: None,
    }
}

pub fn parse_turn_item(item: &ResponseItem) -> Option<TurnItem> {
    match item {
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::ThreadGoalUpdate { .. } => None,
        ResponseItem::EventCommandEvent { id, event } => Some(TurnItem::EventCommandEvent(
            codex_protocol::items::EventCommandEventItem {
                id: id.clone().unwrap_or_else(|| event.stable_item_id()),
                event: event.clone(),
            },
        )),
        ResponseItem::EventDrivenTool { id, trigger } => {
            Some(TurnItem::EventDrivenTool(EventDrivenToolItem {
                id: id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
                tool: trigger.tool.clone(),
                title: trigger.title.clone(),
                text: trigger.text.clone(),
            }))
        }
        ResponseItem::InterAgentCommunication { id, communication } => {
            Some(TurnItem::CollabAgentMessage(CollabAgentMessageItem {
                id: id.clone().unwrap_or_else(|| Uuid::new_v4().to_string()),
                communication: communication.clone(),
            }))
        }
        ResponseItem::Message {
            role,
            content,
            id,
            phase,
            ..
        } => match role.as_str() {
            "user" => parse_visible_hook_prompt_message(id.as_ref(), content)
                .map(TurnItem::HookPrompt)
                .or_else(|| parse_user_message(content).map(TurnItem::UserMessage)),
            "assistant" => Some(TurnItem::AgentMessage(parse_agent_message(
                id.as_ref(),
                content,
                phase.clone(),
            ))),
            "system" => None,
            _ => None,
        },
        ResponseItem::Reasoning {
            id,
            summary,
            content,
            ..
        } => {
            let summary_text = summary
                .iter()
                .map(|entry| match entry {
                    ReasoningItemReasoningSummary::SummaryText { text } => text.clone(),
                })
                .collect();
            let raw_content = content
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|entry| match entry {
                    ReasoningItemContent::ReasoningText { text }
                    | ReasoningItemContent::Text { text } => text,
                })
                .collect();
            Some(TurnItem::Reasoning(ReasoningItem {
                id: id.clone(),
                summary_text,
                raw_content,
            }))
        }
        ResponseItem::WebSearchCall { id, action, .. } => {
            let (action, query) = match action {
                Some(action) => (action.clone(), web_search_action_detail(action)),
                None => (WebSearchAction::Other, String::new()),
            };
            Some(TurnItem::WebSearch(WebSearchItem {
                id: id.clone().unwrap_or_default(),
                query,
                action,
            }))
        }
        ResponseItem::ImageGenerationCall {
            id,
            status,
            revised_prompt,
            result,
        } => Some(TurnItem::ImageGeneration(
            codex_protocol::items::ImageGenerationItem {
                id: id.clone(),
                status: status.clone(),
                revised_prompt: revised_prompt.clone(),
                result: result.clone(),
                saved_path: None,
            },
        )),
        _ => None,
    }
}

pub(crate) fn is_structured_display_response_item(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::ThreadGoalUpdate { .. }
            | ResponseItem::InterAgentCommunication {
                communication: InterAgentCommunication {
                    operation: InterAgentOperation::SpawnAgent
                        | InterAgentOperation::SendMessage
                        | InterAgentOperation::FollowupTask
                        | InterAgentOperation::ChildCompletion,
                    ..
                },
                ..
            }
    )
}

pub(crate) fn injected_context_item_from_response_items(
    items: &[ResponseItem],
) -> Option<TurnItem> {
    let sections: Vec<InjectedContextSection> = items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. }
                if role == "developer" || role == "user" =>
            {
                Some((role.as_str(), content))
            }
            _ => None,
        })
        .flat_map(|(role, content)| {
            content.iter().filter_map(move |item| match item {
                ContentItem::InputText { text } if !text.trim().is_empty() => {
                    Some(InjectedContextSection {
                        label: injected_context_section_label(role, text).to_string(),
                        text: text.clone(),
                    })
                }
                _ => None,
            })
        })
        .collect();

    if sections.is_empty() {
        return None;
    }

    let mut preview_labels = Vec::new();
    for section in &sections {
        if !preview_labels.contains(&section.label) {
            preview_labels.push(section.label.clone());
        }
        if preview_labels.len() == 3 {
            break;
        }
    }

    Some(TurnItem::InjectedContext(InjectedContextItem {
        id: Uuid::new_v4().to_string(),
        title: "Init Context".to_string(),
        preview: preview_labels.join(" • "),
        sections,
    }))
}

fn injected_context_section_label(role: &str, text: &str) -> &'static str {
    if text.contains("# AGENTS.md instructions") {
        return "AGENTS.md";
    }
    if text.contains("<environment_context>") {
        return "Environment";
    }
    if role == "developer" {
        return "Developer";
    }
    "Context"
}

pub(crate) fn started_display_event_from_model_item(
    thread_id: codex_protocol::ThreadId,
    turn_id: String,
    item: &ResponseItem,
    started_at_ms: i64,
) -> Option<EventMsg> {
    match item {
        ResponseItem::CommandWait {
            id: Some(id),
            command_id,
            status,
            notification,
            exit_code,
            wall_time_seconds,
            wait_timeout_ms,
            created_at_ms,
        } => Some(EventMsg::CommandWaitStarted(CommandWaitDisplayEvent {
            thread_id,
            turn_id,
            id: id.clone(),
            command_id: command_id.clone(),
            status: *status,
            notification: *notification,
            exit_code: *exit_code,
            wall_time_seconds: *wall_time_seconds,
            wait_timeout_ms: *wait_timeout_ms,
            created_at_ms: *created_at_ms,
            lifecycle_at_ms: started_at_ms,
        })),
        _ => None,
    }
}

pub(crate) fn completed_display_event_from_model_item(
    thread_id: codex_protocol::ThreadId,
    turn_id: String,
    item: &ResponseItem,
    completed_at_ms: i64,
) -> Option<EventMsg> {
    match item {
        ResponseItem::CommandWait {
            id: Some(id),
            command_id,
            status,
            notification,
            exit_code,
            wall_time_seconds,
            wait_timeout_ms,
            created_at_ms,
        } => Some(EventMsg::CommandWaitCompleted(CommandWaitDisplayEvent {
            thread_id,
            turn_id,
            id: id.clone(),
            command_id: command_id.clone(),
            status: *status,
            notification: *notification,
            exit_code: *exit_code,
            wall_time_seconds: *wall_time_seconds,
            wait_timeout_ms: *wait_timeout_ms,
            created_at_ms: *created_at_ms,
            lifecycle_at_ms: completed_at_ms,
        })),
        ResponseItem::CommandWriteStdin {
            id: Some(id),
            command_id,
            bytes_written,
            contains_newline,
            created_at_ms,
        } => Some(EventMsg::CommandWriteStdinCompleted(
            CommandWriteStdinDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                command_id: command_id.clone(),
                bytes_written: *bytes_written,
                contains_newline: *contains_newline,
                created_at_ms: *created_at_ms,
                completed_at_ms,
            },
        )),
        ResponseItem::CommandExecutionNotification {
            id: Some(id),
            command_item_id,
            kind,
            message,
            output,
            exit_code,
            created_at_ms,
        } => Some(EventMsg::CommandExecutionNotificationCompleted(
            CommandExecutionNotificationDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                command_item_id: command_item_id.clone(),
                kind: *kind,
                message: message.clone(),
                output: output.clone(),
                exit_code: *exit_code,
                created_at_ms: *created_at_ms,
                completed_at_ms,
            },
        )),
        ResponseItem::WorkflowRunProgress {
            id: Some(id),
            event,
        } => Some(EventMsg::WorkflowRunProgressCompleted(
            WorkflowRunProgressDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                event: event.clone(),
                completed_at_ms,
            },
        )),
        ResponseItem::EventCommandEvent {
            id: Some(id),
            event,
        } => Some(EventMsg::EventCommandEventCompleted(
            EventCommandDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                event: event.clone(),
                completed_at_ms,
            },
        )),
        ResponseItem::EventDrivenTool {
            id: Some(id),
            trigger,
        } => Some(EventMsg::EventDrivenToolCompleted(
            EventDrivenToolDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                trigger: trigger.clone(),
                completed_at_ms,
            },
        )),
        ResponseItem::ThreadGoalUpdate {
            id: Some(id),
            goal,
            action,
            source,
            previous_status,
        } => Some(EventMsg::ThreadGoalUpdateCompleted(
            ThreadGoalUpdateDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                goal: goal.clone(),
                action: *action,
                source: *source,
                previous_status: *previous_status,
                completed_at_ms,
            },
        )),
        ResponseItem::InterAgentCommunication {
            id: Some(id),
            communication,
        } => Some(EventMsg::InterAgentCommunicationCompleted(
            InterAgentCommunicationDisplayEvent {
                thread_id,
                turn_id,
                id: id.clone(),
                communication: communication.clone(),
                completed_at_ms,
            },
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "event_mapping_tests.rs"]
mod tests;
