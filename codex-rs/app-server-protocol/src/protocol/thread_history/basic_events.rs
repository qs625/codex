use super::PendingAgentMessageResponse;
use super::ThreadHistoryBuilder;
use crate::protocol::event_item_projection::ProjectedEventItem;
use crate::protocol::event_item_projection::project_event_msg_item;
use crate::protocol::response_item_projection::is_legacy_structured_assistant_message_text;
use crate::protocol::ThreadItem;
use crate::protocol::assistant_message_thread_item;
use protocol::models::MessagePhase;
use protocol::protocol::AgentReasoningEvent;
use protocol::protocol::AgentReasoningRawContentEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::ItemCompletedEvent;
use protocol::protocol::ItemStartedEvent;
use protocol::protocol::UserMessageEvent;

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
        memory_citation: Option<crate::protocol::MemoryCitation>,
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
        memory_citation: Option<crate::protocol::MemoryCitation>,
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
