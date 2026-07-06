use super::PendingAgentMessageResponse;
use super::ThreadHistoryBuilder;
use app_server_protocol::ProjectedEventItem;
use app_server_protocol::project_event_msg_item;
use app_server_protocol::MemoryCitation;
use app_server_protocol::ThreadItem;
use protocol::models::MessagePhase;
use protocol::protocol::AgentReasoningEvent;
use protocol::protocol::AgentReasoningRawContentEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::ItemCompletedEvent;
use protocol::protocol::ItemStartedEvent;
use protocol::protocol::UserMessageEvent;

fn assistant_message_thread_item(
    id: String,
    text: String,
    phase: Option<MessagePhase>,
    memory_citation: Option<MemoryCitation>,
) -> ThreadItem {
    ThreadItem::AgentMessage {
        id,
        text,
        phase,
        memory_citation,
    }
}

fn is_wrapped_marker(trimmed: &str, start_marker: &str, end_marker: &str) -> bool {
    trimmed.starts_with(start_marker) && trimmed.ends_with(end_marker)
}

fn is_legacy_structured_assistant_message_text(text: &str) -> bool {
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

impl ThreadHistoryBuilder {
    pub(super) fn handle_user_message(&mut self, payload: &UserMessageEvent) {
        if let Some(turn) = self.current_turn.as_ref()
            && !turn.opened_explicitly
            && !turn.items.is_empty()
            && !turn.has_only_injected_context()
            && !(turn.saw_compaction && turn.items.is_empty())
        {
            self.finish_current_turn();
        }
        let mut turn = self
            .current_turn
            .take()
            .unwrap_or_else(|| self.new_turn(/*id*/ None));
        let id = self.next_item_id();
        let content = self.build_user_inputs(payload);
        turn.items.push(ThreadItem::UserMessage { id, content });
        self.current_turn = Some(turn);
    }

    pub(super) fn handle_agent_message(
        &mut self,
        text: String,
        phase: Option<MessagePhase>,
        memory_citation: Option<MemoryCitation>,
    ) {
        if text.is_empty() {
            return;
        }
        if is_legacy_structured_assistant_message_text(&text) {
            return;
        }

        if self.consume_duplicate_agent_message_response(
            &text,
            phase.clone(),
            memory_citation.clone(),
        ) {
            return;
        }

        let id = self.next_item_id();
        self.ensure_turn().items.push(assistant_message_thread_item(
            id,
            text,
            phase,
            memory_citation,
        ));
    }

    pub(super) fn consume_duplicate_agent_message_response(
        &mut self,
        text: &str,
        phase: Option<MessagePhase>,
        memory_citation: Option<MemoryCitation>,
    ) -> bool {
        let Some(pending_index) = self
            .pending_agent_message_responses
            .iter()
            .position(|pending| pending.matches(text, phase.as_ref()))
        else {
            return false;
        };
        let pending = self.pending_agent_message_responses.remove(pending_index);

        if let Some(ThreadItem::AgentMessage {
            text: item_text,
            phase: item_phase,
            memory_citation: item_memory_citation,
            ..
        }) = self
            .ensure_turn()
            .items
            .iter_mut()
            .find(|item| item.id() == pending.id)
            && item_text == text
        {
            if phase.is_some() {
                *item_phase = phase;
            }
            if memory_citation.is_some() {
                *item_memory_citation = memory_citation;
            }
        }

        true
    }

    pub(super) fn handle_agent_reasoning(&mut self, payload: &AgentReasoningEvent) {
        if payload.text.is_empty() {
            return;
        }

        if let Some(ThreadItem::Reasoning { summary, .. }) = self.ensure_turn().items.last_mut() {
            summary.push(payload.text.clone());
            return;
        }

        let id = self.next_item_id();
        self.ensure_turn().items.push(ThreadItem::Reasoning {
            id,
            summary: vec![payload.text.clone()],
            content: Vec::new(),
        });
    }

    pub(super) fn handle_agent_reasoning_raw_content(
        &mut self,
        payload: &AgentReasoningRawContentEvent,
    ) {
        if payload.text.is_empty() {
            return;
        }

        if let Some(ThreadItem::Reasoning { content, .. }) = self.ensure_turn().items.last_mut() {
            content.push(payload.text.clone());
            return;
        }

        let id = self.next_item_id();
        self.ensure_turn().items.push(ThreadItem::Reasoning {
            id,
            summary: Vec::new(),
            content: vec![payload.text.clone()],
        });
    }

    pub(super) fn handle_item_started(&mut self, payload: &ItemStartedEvent) {
        match &payload.item {
            protocol::items::TurnItem::Plan(plan) => {
                if plan.text.is_empty() {
                    return;
                }
                self.handle_projected_event_item(&EventMsg::ItemStarted(payload.clone()));
            }
            protocol::items::TurnItem::UserMessage(_)
            | protocol::items::TurnItem::HookPrompt(_)
            | protocol::items::TurnItem::InjectedContext(_)
            | protocol::items::TurnItem::AgentMessage(_)
            | protocol::items::TurnItem::EventDrivenTool(_)
            | protocol::items::TurnItem::EventCommandEvent(_)
            | protocol::items::TurnItem::CollabAgentMessage(_)
            | protocol::items::TurnItem::Reasoning(_)
            | protocol::items::TurnItem::WebSearch(_)
            | protocol::items::TurnItem::ImageView(_)
            | protocol::items::TurnItem::ImageGeneration(_)
            | protocol::items::TurnItem::FileChange(_)
            | protocol::items::TurnItem::McpToolCall(_)
            | protocol::items::TurnItem::ContextCompaction(_) => {}
        }
    }

    pub(super) fn handle_item_completed(&mut self, payload: &ItemCompletedEvent) {
        match &payload.item {
            protocol::items::TurnItem::Plan(plan) => {
                if plan.text.is_empty() {
                    return;
                }
                self.handle_projected_event_item(&EventMsg::ItemCompleted(payload.clone()));
            }
            protocol::items::TurnItem::EventDrivenTool(_)
            | protocol::items::TurnItem::EventCommandEvent(_)
            | protocol::items::TurnItem::InjectedContext(_) => {
                self.handle_projected_event_item(&EventMsg::ItemCompleted(payload.clone()));
            }
            protocol::items::TurnItem::AgentMessage(_) => {
                if let Some(ProjectedEventItem::Completed { item, .. }) =
                    project_event_msg_item(&EventMsg::ItemCompleted(payload.clone()))
                {
                    if let ThreadItem::AgentMessage {
                        id, text, phase, ..
                    } = &item
                    {
                        self.pending_agent_message_responses
                            .push(PendingAgentMessageResponse {
                                id: id.clone(),
                                text: text.clone(),
                                phase: phase.clone(),
                            });
                    }
                    self.upsert_item_in_turn_id(&payload.turn_id, item);
                }
            }
            protocol::items::TurnItem::CollabAgentMessage(_) => {
                self.handle_projected_event_item(&EventMsg::ItemCompleted(payload.clone()));
            }
            protocol::items::TurnItem::UserMessage(_)
            | protocol::items::TurnItem::HookPrompt(_)
            | protocol::items::TurnItem::Reasoning(_)
            | protocol::items::TurnItem::WebSearch(_)
            | protocol::items::TurnItem::ImageView(_)
            | protocol::items::TurnItem::ImageGeneration(_)
            | protocol::items::TurnItem::FileChange(_)
            | protocol::items::TurnItem::McpToolCall(_)
            | protocol::items::TurnItem::ContextCompaction(_) => {}
        }
    }
}
