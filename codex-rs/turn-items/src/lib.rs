use protocol::items::AgentMessageContent;
use protocol::items::AgentMessageItem;
use protocol::items::CollabAgentMessageItem;
use protocol::items::EventDrivenToolItem;
use protocol::items::InjectedContextItem;
use protocol::items::InjectedContextSection;
use protocol::items::ReasoningItem;
use protocol::items::TurnItem;
use protocol::items::UserMessageItem;
use protocol::items::WebSearchItem;
use protocol::models::ContentItem;
use protocol::models::MessagePhase;
use protocol::models::ReasoningItemContent;
use protocol::models::ReasoningItemReasoningSummary;
use protocol::models::ResponseItem;
use protocol::models::WebSearchAction;
use protocol::models::is_image_close_tag_text;
use protocol::models::is_image_open_tag_text;
use protocol::models::is_local_image_close_tag_text;
use protocol::models::is_local_image_open_tag_text;
use protocol::protocol::CommandExecutionNotificationDisplayEvent;
use protocol::protocol::CommandWaitDisplayEvent;
use protocol::protocol::CommandWriteStdinDisplayEvent;
use protocol::protocol::EventCommandDisplayEvent;
use protocol::protocol::EventDrivenToolDisplayEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentCommunicationDisplayEvent;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::ThreadGoalUpdateDisplayEvent;
use protocol::protocol::WorkflowRunProgressDisplayEvent;
use protocol::user_input::UserInput;
use tracing::warn;
use uuid::Uuid;

use codex_context_manager::is_contextual_user_message_content;
use codex_context_manager::parse_visible_hook_prompt_message;

mod assistant_stream;
mod plan_mode_stream;
mod remote_compaction;
mod response_item_policy;

pub use assistant_stream::AssistantMessageStreamParsers;
pub use assistant_stream::ParsedAssistantTextDelta;
pub use assistant_stream::ProposedPlanSegment;
pub use assistant_stream::agent_message_text;
pub use assistant_stream::last_assistant_message_from_item;
pub use assistant_stream::last_assistant_message_from_turn;
pub use assistant_stream::proposed_plan_text_from_assistant_response_item;
pub use assistant_stream::raw_assistant_output_text_from_item;
pub use assistant_stream::realtime_text_for_event;
pub use assistant_stream::strip_hidden_assistant_markup;
pub use plan_mode_stream::PlanModeStreamAction;
pub use plan_mode_stream::PlanModeStreamState;
pub use remote_compaction::CompactRequestLogData;
pub use remote_compaction::build_compact_request_log_data;
pub use remote_compaction::build_remote_v2_compacted_history;
pub use remote_compaction::process_remote_compacted_history;
pub use remote_compaction::should_keep_remote_compacted_history_item;
pub use response_item_policy::FinalizedTurnItemFacts;
pub use response_item_policy::completed_item_defers_mailbox_delivery_to_next_turn;
pub use response_item_policy::finalize_agent_message_content;
pub use response_item_policy::finalized_turn_item_facts;
pub use response_item_policy::response_input_to_response_item;
pub use response_item_policy::response_item_may_include_external_context;

fn search_action_detail(query: &Option<String>, queries: &Option<Vec<String>>) -> String {
    query.clone().filter(|q| !q.is_empty()).unwrap_or_else(|| {
        let items = queries.as_ref();
        let first = items
            .and_then(|queries| queries.first())
            .cloned()
            .unwrap_or_default();
        if items.is_some_and(|queries| queries.len() > 1) && !first.is_empty() {
            format!("{first} ...")
        } else {
            first
        }
    })
}

pub fn web_search_action_detail(action: &WebSearchAction) -> String {
    match action {
        WebSearchAction::Search { query, queries } => search_action_detail(query, queries),
        WebSearchAction::OpenPage { url } => url.clone().unwrap_or_default(),
        WebSearchAction::FindInPage { url, pattern } => match (pattern, url) {
            (Some(pattern), Some(url)) => format!("'{pattern}' in {url}"),
            (Some(pattern), None) => format!("'{pattern}'"),
            (None, Some(url)) => url.clone(),
            (None, None) => String::new(),
        },
        WebSearchAction::Other => String::new(),
    }
}

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
            protocol::items::EventCommandEventItem {
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
            protocol::items::ImageGenerationItem {
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

pub fn is_structured_display_response_item(item: &ResponseItem) -> bool {
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

pub fn injected_context_item_from_response_items(items: &[ResponseItem]) -> Option<TurnItem> {
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

pub fn started_display_event_from_model_item(
    thread_id: protocol::ThreadId,
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

pub fn completed_display_event_from_model_item(
    thread_id: protocol::ThreadId,
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
mod tests;
