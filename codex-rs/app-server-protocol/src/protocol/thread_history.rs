use crate::protocol::item_builders::build_command_execution_begin_item;
use crate::protocol::item_builders::build_command_execution_end_item;
use crate::protocol::item_builders::build_command_execution_exit_notification_item;
use crate::protocol::item_builders::build_command_execution_output_notification_item;
use crate::protocol::item_builders::build_file_change_approval_request_item;
use crate::protocol::item_builders::build_file_change_begin_item;
use crate::protocol::item_builders::build_file_change_end_item;
use crate::protocol::item_builders::build_item_from_guardian_event;
use crate::protocol::response_item_projection::is_legacy_structured_assistant_message_text;
use crate::protocol::response_item_projection::project_structured_response_item;
use crate::protocol::response_item_projection::project_tool_call_completion;
use crate::protocol::response_item_projection::project_tool_call_start;
use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::CollabAgentTool;
use crate::protocol::v2::CollabAgentToolCallStatus;
use crate::protocol::v2::CommandExecutionStatus;
use crate::protocol::v2::DynamicToolCallOutputContentItem;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::InjectedContextSection;
use crate::protocol::v2::McpToolCallError;
use crate::protocol::v2::McpToolCallResult;
use crate::protocol::v2::McpToolCallStatus;
use crate::protocol::v2::ThreadItem;
use crate::protocol::v2::Turn;
use crate::protocol::v2::TurnError as V2TurnError;
use crate::protocol::v2::TurnError;
use crate::protocol::v2::TurnItemsView;
use crate::protocol::v2::TurnStatus;
use crate::protocol::v2::UserInput;
use crate::protocol::v2::WebSearchAction;
use crate::protocol::v2::assistant_message_thread_item;
use codex_protocol::items::parse_hook_prompt_message;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::APPS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::APPS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::AgentReasoningEvent;
use codex_protocol::protocol::AgentReasoningRawContentEvent;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::ApplyPatchApprovalRequestEvent;
use codex_protocol::protocol::COLLABORATION_MODE_CLOSE_TAG;
use codex_protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use codex_protocol::protocol::CompactedItem;
use codex_protocol::protocol::ContextCompactedEvent;
use codex_protocol::protocol::DynamicToolCallResponseEvent;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_CLOSE_TAG;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_OPEN_TAG;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
use codex_protocol::protocol::GuardianAssessmentEvent;
use codex_protocol::protocol::GuardianAssessmentStatus;
use codex_protocol::protocol::ImageGenerationBeginEvent;
use codex_protocol::protocol::ImageGenerationEndEvent;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::McpToolCallBeginEvent;
use codex_protocol::protocol::McpToolCallEndEvent;
use codex_protocol::protocol::PLUGINS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::PLUGINS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::PatchApplyBeginEvent;
use codex_protocol::protocol::PatchApplyEndEvent;
use codex_protocol::protocol::REALTIME_CONVERSATION_CLOSE_TAG;
use codex_protocol::protocol::REALTIME_CONVERSATION_OPEN_TAG;
use codex_protocol::protocol::ReviewOutputEvent;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;
use codex_protocol::protocol::ThreadRolledBackEvent;
use codex_protocol::protocol::TurnAbortedEvent;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::protocol::TurnContextItem;
use codex_protocol::protocol::TurnStartedEvent;
use codex_protocol::protocol::UserMessageEvent;
use codex_protocol::protocol::ViewImageToolCallEvent;
use codex_protocol::protocol::WebSearchBeginEvent;
use codex_protocol::protocol::WebSearchEndEvent;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

#[cfg(test)]
use crate::protocol::v2::CommandAction;
#[cfg(test)]
use crate::protocol::v2::FileUpdateChange;
#[cfg(test)]
use crate::protocol::v2::PatchApplyStatus;
#[cfg(test)]
use crate::protocol::v2::PatchChangeKind;
#[cfg(test)]
use codex_protocol::config_types::ModeKind;
#[cfg(test)]
use codex_protocol::protocol::ExecCommandStatus as CoreExecCommandStatus;
#[cfg(test)]
use codex_protocol::protocol::PatchApplyStatus as CorePatchApplyStatus;

const INJECTED_CONTEXT_TITLE: &str = "Initial context injected";
const MAX_INJECTED_CONTEXT_PREVIEW_SECTIONS: usize = 3;

/// Convert persisted [`RolloutItem`] entries into a sequence of [`Turn`] values.
///
/// When available, this uses `TurnContext.turn_id` as the canonical turn id so
/// resumed/rebuilt thread history preserves the original turn identifiers.
pub fn build_turns_from_rollout_items(items: &[RolloutItem]) -> Vec<Turn> {
    let mut builder = ThreadHistoryBuilder::new();
    for item in items {
        builder.handle_rollout_item(item);
    }
    builder.finish()
}

pub struct ThreadHistoryBuilder {
    turns: Vec<Turn>,
    current_turn: Option<PendingTurn>,
    next_item_index: i64,
    current_rollout_index: usize,
    next_rollout_index: usize,
    pending_agent_message_responses: Vec<PendingAgentMessageResponse>,
    pending_legacy_agent_messages: Vec<PendingLegacyAgentMessage>,
}

impl Default for ThreadHistoryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadHistoryBuilder {
    pub fn new() -> Self {
        Self {
            turns: Vec::new(),
            current_turn: None,
            next_item_index: 1,
            current_rollout_index: 0,
            next_rollout_index: 0,
            pending_agent_message_responses: Vec::new(),
            pending_legacy_agent_messages: Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn finish(mut self) -> Vec<Turn> {
        self.finish_current_turn();
        self.turns
    }

    pub fn active_turn_snapshot(&self) -> Option<Turn> {
        self.current_turn
            .as_ref()
            .map(Turn::from)
            .or_else(|| self.turns.last().cloned())
    }

    /// Returns the index of the active turn snapshot within the finished turn list.
    ///
    /// When a turn is still open, this is the index it will occupy after
    /// `finish`. When no turn is open, it is the index of the last finished turn.
    pub fn active_turn_position(&self) -> Option<usize> {
        if self.current_turn.is_some() {
            Some(self.turns.len())
        } else if self.turns.is_empty() {
            None
        } else {
            Some(self.turns.len() - 1)
        }
    }

    pub fn has_active_turn(&self) -> bool {
        self.current_turn.is_some()
    }

    pub fn active_turn_id_if_explicit(&self) -> Option<String> {
        self.current_turn
            .as_ref()
            .filter(|turn| turn.opened_explicitly)
            .map(|turn| turn.id.clone())
    }

    pub fn active_turn_start_index(&self) -> Option<usize> {
        self.current_turn
            .as_ref()
            .map(|turn| turn.rollout_start_index)
    }

    /// Shared reducer for persisted rollout replay and in-memory current-turn
    /// tracking used by running thread resume/rejoin.
    ///
    /// This function should handle all EventMsg variants that can be persisted in a rollout file.
    /// See `should_persist_event_msg` in `codex-rs/core/rollout/policy.rs`.
    pub fn handle_event(&mut self, event: &EventMsg) {
        match event {
            EventMsg::UserMessage(payload) => self.handle_user_message(payload),
            EventMsg::AgentMessage(payload) => self.handle_agent_message(
                payload.message.clone(),
                payload.phase.clone(),
                payload.memory_citation.clone().map(Into::into),
            ),
            EventMsg::AgentReasoning(payload) => self.handle_agent_reasoning(payload),
            EventMsg::AgentReasoningRawContent(payload) => {
                self.handle_agent_reasoning_raw_content(payload)
            }
            EventMsg::WebSearchBegin(payload) => self.handle_web_search_begin(payload),
            EventMsg::WebSearchEnd(payload) => self.handle_web_search_end(payload),
            EventMsg::ExecCommandBegin(payload) => self.handle_exec_command_begin(payload),
            EventMsg::ExecCommandOutputDelta(payload) => {
                self.handle_exec_command_output_delta(payload)
            }
            EventMsg::ExecCommandEnd(payload) => self.handle_exec_command_end(payload),
            EventMsg::GuardianAssessment(payload) => self.handle_guardian_assessment(payload),
            EventMsg::ApplyPatchApprovalRequest(payload) => {
                self.handle_apply_patch_approval_request(payload)
            }
            EventMsg::PatchApplyBegin(payload) => self.handle_patch_apply_begin(payload),
            EventMsg::PatchApplyEnd(payload) => self.handle_patch_apply_end(payload),
            EventMsg::DynamicToolCallRequest(payload) => {
                self.handle_dynamic_tool_call_request(payload)
            }
            EventMsg::DynamicToolCallResponse(payload) => {
                self.handle_dynamic_tool_call_response(payload)
            }
            EventMsg::McpToolCallBegin(payload) => self.handle_mcp_tool_call_begin(payload),
            EventMsg::McpToolCallEnd(payload) => self.handle_mcp_tool_call_end(payload),
            EventMsg::ViewImageToolCall(payload) => self.handle_view_image_tool_call(payload),
            EventMsg::ImageGenerationBegin(payload) => self.handle_image_generation_begin(payload),
            EventMsg::ImageGenerationEnd(payload) => self.handle_image_generation_end(payload),
            EventMsg::CollabAgentSpawnBegin(payload) => {
                self.handle_collab_agent_spawn_begin(payload)
            }
            EventMsg::CollabAgentSpawnEnd(payload) => self.handle_collab_agent_spawn_end(payload),
            EventMsg::CollabAgentInteractionBegin(payload) => {
                self.handle_collab_agent_interaction_begin(payload)
            }
            EventMsg::CollabAgentInteractionEnd(payload) => {
                self.handle_collab_agent_interaction_end(payload)
            }
            EventMsg::CollabWaitingBegin(payload) => self.handle_collab_waiting_begin(payload),
            EventMsg::CollabWaitingEnd(payload) => self.handle_collab_waiting_end(payload),
            EventMsg::CollabCloseBegin(payload) => self.handle_collab_close_begin(payload),
            EventMsg::CollabCloseEnd(payload) => self.handle_collab_close_end(payload),
            EventMsg::CollabResumeBegin(payload) => self.handle_collab_resume_begin(payload),
            EventMsg::CollabResumeEnd(payload) => self.handle_collab_resume_end(payload),
            EventMsg::ContextCompacted(payload) => self.handle_context_compacted(payload),
            EventMsg::EnteredReviewMode(payload) => self.handle_entered_review_mode(payload),
            EventMsg::ExitedReviewMode(payload) => self.handle_exited_review_mode(payload),
            EventMsg::ItemStarted(payload) => self.handle_item_started(payload),
            EventMsg::ItemCompleted(payload) => self.handle_item_completed(payload),
            EventMsg::ResponseItemCompleted(_) => {}
            EventMsg::RawResponseItem(payload) => self.handle_response_item(&payload.item),
            EventMsg::HookStarted(_) | EventMsg::HookCompleted(_) => {}
            EventMsg::Error(payload) => self.handle_error(payload),
            EventMsg::TokenCount(_) => {}
            EventMsg::ThreadRolledBack(payload) => self.handle_thread_rollback(payload),
            EventMsg::TurnAborted(payload) => self.handle_turn_aborted(payload),
            EventMsg::TurnStarted(payload) => self.handle_turn_started(payload),
            EventMsg::TurnComplete(payload) => self.handle_turn_complete(payload),
            _ => {}
        }
    }

    pub fn handle_rollout_item(&mut self, item: &RolloutItem) {
        self.current_rollout_index = self.next_rollout_index;
        self.next_rollout_index += 1;
        match item {
            RolloutItem::EventMsg(event) => self.handle_event(event),
            RolloutItem::Compacted(payload) => self.handle_compacted(payload),
            RolloutItem::ResponseItem(item) => self.handle_response_item(item),
            RolloutItem::TurnContext(payload) => self.handle_turn_context(payload),
            RolloutItem::SessionMeta(_) => {}
        }
    }

    fn handle_response_item(&mut self, item: &codex_protocol::models::ResponseItem) {
        match item {
            ResponseItem::Message {
                role,
                content,
                id,
                phase,
                ..
            } => {
                if self.try_handle_injected_context_message(role, content) {
                    return;
                }

                if role == "assistant" {
                    let Some(text) = single_text_message_content(content) else {
                        return;
                    };
                    if is_legacy_structured_assistant_message_text(text) {
                        return;
                    }
                    let id = id.clone().unwrap_or_else(|| self.next_item_id());
                    if self.consume_duplicate_agent_message_response(text, phase.clone(), None) {
                        return;
                    }
                    if self.consume_duplicate_legacy_agent_message_response(
                        &id,
                        text,
                        phase.clone(),
                    ) {
                        return;
                    }
                    self.ensure_turn().items.push(assistant_message_thread_item(
                        id.clone(),
                        text.to_string(),
                        phase.clone(),
                        None,
                    ));
                    self.pending_agent_message_responses
                        .push(PendingAgentMessageResponse {
                            id,
                            text: text.to_string(),
                            phase: phase.clone(),
                        });
                    return;
                }

                if role != "user" {
                    return;
                }

                let Some(hook_prompt) = parse_hook_prompt_message(id.as_ref(), content) else {
                    return;
                };

                self.ensure_turn().items.push(ThreadItem::HookPrompt {
                    id: hook_prompt.id,
                    fragments: hook_prompt
                        .fragments
                        .into_iter()
                        .map(crate::protocol::v2::HookPromptFragment::from)
                        .collect(),
                });
            }
            ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::InterAgentCommunication { .. } => {
                let fallback_id = self.next_item_id();
                if let Some(projected) =
                    project_structured_response_item(item, || fallback_id.clone())
                {
                    self.ensure_turn().items.push(projected);
                    return;
                }

                if let ResponseItem::InterAgentCommunication { id, communication } = item
                    && matches!(communication.operation, InterAgentOperation::Unknown)
                {
                    let text = serde_json::to_string(communication)
                        .unwrap_or_else(|_| communication.content.clone());
                    self.ensure_turn().items.push(assistant_message_thread_item(
                        id.clone().unwrap_or(fallback_id),
                        text,
                        None,
                        None,
                    ));
                }
            }
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                if let Some(item) =
                    project_tool_call_start(name, namespace.as_deref(), arguments, call_id)
                {
                    self.upsert_event_driven_tool_call_in_current_turn(item);
                }
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                let existing = self.find_event_driven_tool_call_in_current_turn(call_id);
                if existing.is_none() {
                    return;
                }
                if let Some(item) =
                    existing.and_then(|item| project_tool_call_completion(item, call_id, output))
                {
                    self.upsert_event_driven_tool_call_in_current_turn(item);
                }
            }
            _ => {}
        }
    }

    fn try_handle_injected_context_message(&mut self, role: &str, content: &[ContentItem]) -> bool {
        if !self.is_initial_injected_context_window() {
            return false;
        }

        let sections = parse_injected_context_sections(role, content);
        if sections.is_empty() {
            return false;
        }

        self.append_injected_context_sections(sections);
        true
    }

    fn is_initial_injected_context_window(&self) -> bool {
        self.turns.is_empty()
            && self.current_turn.as_ref().is_none_or(|turn| {
                (turn.items.is_empty() || turn.has_only_injected_context()) && !turn.saw_compaction
            })
    }

    fn append_injected_context_sections(&mut self, mut sections: Vec<InjectedContextSection>) {
        {
            let turn = self.ensure_turn();
            if let Some(ThreadItem::InjectedContext {
                preview,
                sections: existing_sections,
                ..
            }) = turn.items.last_mut()
            {
                existing_sections.append(&mut sections);
                *preview = build_injected_context_preview(existing_sections);
                return;
            }
        }

        let id = self.next_item_id();
        let preview = build_injected_context_preview(&sections);
        self.ensure_turn().items.push(ThreadItem::InjectedContext {
            id,
            title: INJECTED_CONTEXT_TITLE.to_string(),
            preview,
            sections,
        });
    }

    fn handle_user_message(&mut self, payload: &UserMessageEvent) {
        // User messages should stay in explicitly opened turns. For backward
        // compatibility with older streams that did not open turns explicitly,
        // close any implicit/inactive turn and start a fresh one for this input.
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

    fn handle_agent_message(
        &mut self,
        text: String,
        phase: Option<MessagePhase>,
        memory_citation: Option<crate::protocol::v2::MemoryCitation>,
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
        self.pending_legacy_agent_messages
            .push(PendingLegacyAgentMessage {
                id: id.clone(),
                text: text.clone(),
                phase: phase.clone(),
            });
        self.ensure_turn().items.push(assistant_message_thread_item(
            id,
            text,
            phase,
            memory_citation,
        ));
    }

    fn consume_duplicate_agent_message_response(
        &mut self,
        text: &str,
        phase: Option<MessagePhase>,
        memory_citation: Option<crate::protocol::v2::MemoryCitation>,
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

    fn consume_duplicate_legacy_agent_message_response(
        &mut self,
        response_id: &str,
        text: &str,
        phase: Option<MessagePhase>,
    ) -> bool {
        let Some(pending_index) = self
            .pending_legacy_agent_messages
            .iter()
            .position(|pending| pending.matches(text, phase.as_ref()))
        else {
            return false;
        };
        let pending = self.pending_legacy_agent_messages.remove(pending_index);

        let Some(existing_item) = self
            .ensure_turn()
            .items
            .iter_mut()
            .find(|item| item.id() == pending.id)
        else {
            return false;
        };

        match existing_item {
            ThreadItem::AgentMessage {
                id,
                text: item_text,
                phase: item_phase,
                ..
            } if item_text == text => {
                *id = response_id.to_string();
                if phase.is_some() {
                    *item_phase = phase;
                }
                true
            }
            _ => false,
        }
    }

    fn handle_agent_reasoning(&mut self, payload: &AgentReasoningEvent) {
        if payload.text.is_empty() {
            return;
        }

        // If the last item is a reasoning item, add the new text to the summary.
        if let Some(ThreadItem::Reasoning { summary, .. }) = self.ensure_turn().items.last_mut() {
            summary.push(payload.text.clone());
            return;
        }

        // Otherwise, create a new reasoning item.
        let id = self.next_item_id();
        self.ensure_turn().items.push(ThreadItem::Reasoning {
            id,
            summary: vec![payload.text.clone()],
            content: Vec::new(),
        });
    }

    fn handle_agent_reasoning_raw_content(&mut self, payload: &AgentReasoningRawContentEvent) {
        if payload.text.is_empty() {
            return;
        }

        // If the last item is a reasoning item, add the new text to the content.
        if let Some(ThreadItem::Reasoning { content, .. }) = self.ensure_turn().items.last_mut() {
            content.push(payload.text.clone());
            return;
        }

        // Otherwise, create a new reasoning item.
        let id = self.next_item_id();
        self.ensure_turn().items.push(ThreadItem::Reasoning {
            id,
            summary: Vec::new(),
            content: vec![payload.text.clone()],
        });
    }

    fn handle_item_started(&mut self, payload: &ItemStartedEvent) {
        match &payload.item {
            codex_protocol::items::TurnItem::Plan(plan) => {
                if plan.text.is_empty() {
                    return;
                }
                self.upsert_item_in_turn_id(
                    &payload.turn_id,
                    ThreadItem::from(payload.item.clone()),
                );
            }
            codex_protocol::items::TurnItem::UserMessage(_)
            | codex_protocol::items::TurnItem::HookPrompt(_)
            | codex_protocol::items::TurnItem::AgentMessage(_)
            | codex_protocol::items::TurnItem::EventDrivenTool(_)
            | codex_protocol::items::TurnItem::EventCommandEvent(_)
            | codex_protocol::items::TurnItem::CollabAgentMessage(_)
            | codex_protocol::items::TurnItem::Reasoning(_)
            | codex_protocol::items::TurnItem::WebSearch(_)
            | codex_protocol::items::TurnItem::ImageView(_)
            | codex_protocol::items::TurnItem::ImageGeneration(_)
            | codex_protocol::items::TurnItem::FileChange(_)
            | codex_protocol::items::TurnItem::McpToolCall(_)
            | codex_protocol::items::TurnItem::ContextCompaction(_) => {}
        }
    }

    fn handle_item_completed(&mut self, payload: &ItemCompletedEvent) {
        match &payload.item {
            codex_protocol::items::TurnItem::Plan(plan) => {
                if plan.text.is_empty() {
                    return;
                }
                self.upsert_item_in_turn_id(
                    &payload.turn_id,
                    ThreadItem::from(payload.item.clone()),
                );
            }
            codex_protocol::items::TurnItem::EventDrivenTool(_)
            | codex_protocol::items::TurnItem::EventCommandEvent(_) => {
                self.upsert_item_in_turn_id(
                    &payload.turn_id,
                    ThreadItem::from(payload.item.clone()),
                );
            }
            codex_protocol::items::TurnItem::AgentMessage(_) => {
                let item = ThreadItem::from(payload.item.clone());
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
            codex_protocol::items::TurnItem::CollabAgentMessage(_) => {
                self.upsert_item_in_turn_id_or_create(
                    &payload.turn_id,
                    ThreadItem::from(payload.item.clone()),
                );
            }
            codex_protocol::items::TurnItem::UserMessage(_)
            | codex_protocol::items::TurnItem::HookPrompt(_)
            | codex_protocol::items::TurnItem::Reasoning(_)
            | codex_protocol::items::TurnItem::WebSearch(_)
            | codex_protocol::items::TurnItem::ImageView(_)
            | codex_protocol::items::TurnItem::ImageGeneration(_)
            | codex_protocol::items::TurnItem::FileChange(_)
            | codex_protocol::items::TurnItem::McpToolCall(_)
            | codex_protocol::items::TurnItem::ContextCompaction(_) => {}
        }
    }

    fn handle_web_search_begin(&mut self, payload: &WebSearchBeginEvent) {
        let item = ThreadItem::WebSearch {
            id: payload.call_id.clone(),
            query: String::new(),
            action: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_web_search_end(&mut self, payload: &WebSearchEndEvent) {
        let item = ThreadItem::WebSearch {
            id: payload.call_id.clone(),
            query: payload.query.clone(),
            action: Some(WebSearchAction::from(payload.action.clone())),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_exec_command_begin(&mut self, payload: &ExecCommandBeginEvent) {
        let item = build_command_execution_begin_item(payload);
        self.upsert_item_in_turn_id(&payload.turn_id, item);
    }

    fn handle_exec_command_output_delta(&mut self, payload: &ExecCommandOutputDeltaEvent) {
        let Some(item) = build_command_execution_output_notification_item(payload) else {
            return;
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_exec_command_end(&mut self, payload: &ExecCommandEndEvent) {
        let item = build_command_execution_end_item(payload);
        // Command completions can arrive out of order. Unified exec may return
        // while a PTY is still running, then emit ExecCommandEnd later from a
        // background exit watcher when that process finally exits. By then, a
        // newer user turn may already have started. Route by event turn_id so
        // replay preserves the original turn association.
        self.upsert_item_in_turn_id(&payload.turn_id, item);
        if let Some(item) = build_command_execution_exit_notification_item(payload) {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_guardian_assessment(&mut self, payload: &GuardianAssessmentEvent) {
        let status = match payload.status {
            GuardianAssessmentStatus::InProgress => CommandExecutionStatus::InProgress,
            GuardianAssessmentStatus::Denied | GuardianAssessmentStatus::Aborted => {
                CommandExecutionStatus::Declined
            }
            GuardianAssessmentStatus::TimedOut => CommandExecutionStatus::Failed,
            GuardianAssessmentStatus::Approved => return,
        };
        let Some(item) = build_item_from_guardian_event(payload, status) else {
            return;
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_apply_patch_approval_request(&mut self, payload: &ApplyPatchApprovalRequestEvent) {
        let item = build_file_change_approval_request_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_patch_apply_begin(&mut self, payload: &PatchApplyBeginEvent) {
        let item = build_file_change_begin_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_patch_apply_end(&mut self, payload: &PatchApplyEndEvent) {
        let item = build_file_change_end_item(payload);
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_dynamic_tool_call_request(
        &mut self,
        payload: &codex_protocol::dynamic_tools::DynamicToolCallRequest,
    ) {
        let item = ThreadItem::DynamicToolCall {
            id: payload.call_id.clone(),
            namespace: payload.namespace.clone(),
            tool: payload.tool.clone(),
            arguments: payload.arguments.clone(),
            status: DynamicToolCallStatus::InProgress,
            content_items: None,
            success: None,
            duration_ms: None,
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_dynamic_tool_call_response(&mut self, payload: &DynamicToolCallResponseEvent) {
        let status = if payload.success {
            DynamicToolCallStatus::Completed
        } else {
            DynamicToolCallStatus::Failed
        };
        let duration_ms = i64::try_from(payload.duration.as_millis()).ok();
        let item = ThreadItem::DynamicToolCall {
            id: payload.call_id.clone(),
            namespace: payload.namespace.clone(),
            tool: payload.tool.clone(),
            arguments: payload.arguments.clone(),
            status,
            content_items: Some(convert_dynamic_tool_content_items(&payload.content_items)),
            success: Some(payload.success),
            duration_ms,
        };
        if payload.turn_id.is_empty() {
            self.upsert_item_in_current_turn(item);
        } else {
            self.upsert_item_in_turn_id(&payload.turn_id, item);
        }
    }

    fn handle_mcp_tool_call_begin(&mut self, payload: &McpToolCallBeginEvent) {
        let item = ThreadItem::McpToolCall {
            id: payload.call_id.clone(),
            server: payload.invocation.server.clone(),
            tool: payload.invocation.tool.clone(),
            status: McpToolCallStatus::InProgress,
            arguments: payload
                .invocation
                .arguments
                .clone()
                .unwrap_or(serde_json::Value::Null),
            mcp_app_resource_uri: payload.mcp_app_resource_uri.clone(),
            result: None,
            error: None,
            duration_ms: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_mcp_tool_call_end(&mut self, payload: &McpToolCallEndEvent) {
        let status = if payload.is_success() {
            McpToolCallStatus::Completed
        } else {
            McpToolCallStatus::Failed
        };
        let duration_ms = i64::try_from(payload.duration.as_millis()).ok();
        let (result, error) = match &payload.result {
            Ok(value) => (
                Some(Box::new(McpToolCallResult {
                    content: value.content.clone(),
                    structured_content: value.structured_content.clone(),
                    meta: value.meta.clone(),
                })),
                None,
            ),
            Err(message) => (
                None,
                Some(McpToolCallError {
                    message: message.clone(),
                }),
            ),
        };
        let item = ThreadItem::McpToolCall {
            id: payload.call_id.clone(),
            server: payload.invocation.server.clone(),
            tool: payload.invocation.tool.clone(),
            status,
            arguments: payload
                .invocation
                .arguments
                .clone()
                .unwrap_or(serde_json::Value::Null),
            mcp_app_resource_uri: payload.mcp_app_resource_uri.clone(),
            result,
            error,
            duration_ms,
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_view_image_tool_call(&mut self, payload: &ViewImageToolCallEvent) {
        let item = ThreadItem::ImageView {
            id: payload.call_id.clone(),
            path: payload.path.clone(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_image_generation_begin(&mut self, payload: &ImageGenerationBeginEvent) {
        let item = ThreadItem::ImageGeneration {
            id: payload.call_id.clone(),
            status: String::new(),
            revised_prompt: None,
            result: String::new(),
            saved_path: None,
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_image_generation_end(&mut self, payload: &ImageGenerationEndEvent) {
        let item = ThreadItem::ImageGeneration {
            id: payload.call_id.clone(),
            status: payload.status.clone(),
            revised_prompt: payload.revised_prompt.clone(),
            result: payload.result.clone(),
            saved_path: payload.saved_path.clone(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_agent_spawn_begin(
        &mut self,
        payload: &codex_protocol::protocol::CollabAgentSpawnBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: Vec::new(),
            receiver_paths: Vec::new(),
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: Some(payload.model.clone()),
            reasoning_effort: Some(payload.reasoning_effort),
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_agent_spawn_end(
        &mut self,
        payload: &codex_protocol::protocol::CollabAgentSpawnEndEvent,
    ) {
        let has_receiver = payload.new_thread_id.is_some();
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ if has_receiver => CollabAgentToolCallStatus::Completed,
            _ => CollabAgentToolCallStatus::Failed,
        };
        let (receiver_thread_ids, agents_states) = match &payload.new_thread_id {
            Some(id) => {
                let receiver_id = id.to_string();
                let mut received_status = CollabAgentState::from(payload.status.clone());
                received_status.path = payload.new_agent_path.clone();
                (
                    vec![receiver_id.clone()],
                    [(receiver_id, received_status)].into_iter().collect(),
                )
            }
            None => (Vec::new(), HashMap::new()),
        };
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SpawnAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids,
            receiver_paths: payload.new_agent_path.clone().into_iter().collect(),
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: Some(payload.model.clone()),
            reasoning_effort: Some(payload.reasoning_effort),
            agents_states,
        });
    }

    fn handle_collab_agent_interaction_begin(
        &mut self,
        payload: &codex_protocol::protocol::CollabAgentInteractionBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SendInput,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_agent_interaction_end(
        &mut self,
        payload: &codex_protocol::protocol::CollabAgentInteractionEndEvent,
    ) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut received_status = CollabAgentState::from(payload.status.clone());
        received_status.path = Some(payload.receiver_agent_path.clone());
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::SendInput,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id.clone()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: Some(payload.prompt.clone()),
            model: None,
            reasoning_effort: None,
            agents_states: [(receiver_id, received_status)].into_iter().collect(),
        });
    }

    fn handle_collab_waiting_begin(
        &mut self,
        payload: &codex_protocol::protocol::CollabWaitingBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::Wait,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: payload
                .receiver_thread_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
            receiver_paths: payload
                .receiver_agents
                .iter()
                .filter_map(|agent| agent.agent_path.clone())
                .collect(),
            timeout_ms: Some(payload.timeout_ms),
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_waiting_end(
        &mut self,
        payload: &codex_protocol::protocol::CollabWaitingEndEvent,
    ) {
        let status = if payload
            .statuses
            .values()
            .any(|status| matches!(status, AgentStatus::Errored(_) | AgentStatus::NotFound))
        {
            CollabAgentToolCallStatus::Failed
        } else {
            CollabAgentToolCallStatus::Completed
        };
        let mut receiver_thread_ids: Vec<String> =
            payload.statuses.keys().map(ToString::to_string).collect();
        receiver_thread_ids.sort();
        let agents_states = payload
            .statuses
            .iter()
            .map(|(id, status)| {
                let mut state = CollabAgentState::from(status.clone());
                state.path = payload
                    .agent_statuses
                    .iter()
                    .find(|entry| entry.thread_id == *id)
                    .and_then(|entry| entry.agent_path.clone());
                (id.to_string(), state)
            })
            .collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::Wait,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids,
            receiver_paths: payload
                .agent_statuses
                .iter()
                .filter_map(|entry| entry.agent_path.clone())
                .collect(),
            timeout_ms: Some(payload.timeout_ms),
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    fn handle_collab_close_begin(
        &mut self,
        payload: &codex_protocol::protocol::CollabCloseBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::CloseAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_close_end(&mut self, payload: &codex_protocol::protocol::CollabCloseEndEvent) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut state = CollabAgentState::from(payload.status.clone());
        state.path = Some(payload.receiver_agent_path.clone());
        let agents_states = [(receiver_id.clone(), state)].into_iter().collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::CloseAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    fn handle_collab_resume_begin(
        &mut self,
        payload: &codex_protocol::protocol::CollabResumeBeginEvent,
    ) {
        let item = ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ResumeAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![payload.receiver_thread_id.to_string()],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states: HashMap::new(),
        };
        self.upsert_item_in_current_turn(item);
    }

    fn handle_collab_resume_end(
        &mut self,
        payload: &codex_protocol::protocol::CollabResumeEndEvent,
    ) {
        let status = match &payload.status {
            AgentStatus::Errored(_) | AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
            _ => CollabAgentToolCallStatus::Completed,
        };
        let receiver_id = payload.receiver_thread_id.to_string();
        let mut state = CollabAgentState::from(payload.status.clone());
        state.path = Some(payload.receiver_agent_path.clone());
        let agents_states = [(receiver_id.clone(), state)].into_iter().collect();
        self.upsert_item_in_current_turn(ThreadItem::CollabAgentToolCall {
            id: payload.call_id.clone(),
            tool: CollabAgentTool::ResumeAgent,
            status,
            sender_thread_id: payload.sender_thread_id.to_string(),
            sender_path: payload.sender_agent_path.clone(),
            receiver_thread_ids: vec![receiver_id],
            receiver_paths: vec![payload.receiver_agent_path.clone()],
            timeout_ms: None,
            prompt: None,
            model: None,
            reasoning_effort: None,
            agents_states,
        });
    }

    fn handle_context_compacted(&mut self, _payload: &ContextCompactedEvent) {
        if self.ensure_turn().items.iter().any(|item| {
            matches!(
                item,
                ThreadItem::ContextCompaction {
                    replacement_history: _,
                    ..
                }
            )
        }) {
            return;
        }

        let id = self.next_item_id();
        self.ensure_turn()
            .items
            .push(ThreadItem::ContextCompaction {
                id,
                replacement_history: None,
            });
    }

    fn handle_entered_review_mode(&mut self, payload: &codex_protocol::protocol::ReviewRequest) {
        let review = payload
            .user_facing_hint
            .clone()
            .unwrap_or_else(|| "Review requested.".to_string());
        let id = self.next_item_id();
        self.ensure_turn()
            .items
            .push(ThreadItem::EnteredReviewMode { id, review });
    }

    fn handle_exited_review_mode(
        &mut self,
        payload: &codex_protocol::protocol::ExitedReviewModeEvent,
    ) {
        let review = payload
            .review_output
            .as_ref()
            .map(render_review_output_text)
            .unwrap_or_else(|| REVIEW_FALLBACK_MESSAGE.to_string());
        let id = self.next_item_id();
        self.ensure_turn()
            .items
            .push(ThreadItem::ExitedReviewMode { id, review });
    }

    fn handle_error(&mut self, payload: &ErrorEvent) {
        if !payload.affects_turn_status() {
            return;
        }
        let Some(turn) = self.current_turn.as_mut() else {
            return;
        };
        turn.status = TurnStatus::Failed;
        turn.error = Some(V2TurnError {
            message: payload.message.clone(),
            codex_error_info: payload.codex_error_info.clone().map(Into::into),
            additional_details: None,
        });
    }

    fn handle_turn_aborted(&mut self, payload: &TurnAbortedEvent) {
        let apply_abort = |turn: &mut PendingTurn| {
            turn.status = TurnStatus::Interrupted;
            turn.completed_at = payload.completed_at;
            turn.duration_ms = payload.duration_ms;
        };
        if let Some(turn_id) = payload.turn_id.as_deref() {
            // Prefer an exact ID match so we interrupt the turn explicitly targeted by the event.
            if let Some(turn) = self.current_turn.as_mut().filter(|turn| turn.id == turn_id) {
                apply_abort(turn);
                return;
            }

            if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == turn_id) {
                turn.status = TurnStatus::Interrupted;
                turn.completed_at = payload.completed_at;
                turn.duration_ms = payload.duration_ms;
                return;
            }
        }

        // If the event has no ID (or refers to an unknown turn), fall back to the active turn.
        if let Some(turn) = self.current_turn.as_mut() {
            apply_abort(turn);
        }
    }

    fn handle_turn_started(&mut self, payload: &TurnStartedEvent) {
        if let Some(turn) = self
            .current_turn
            .as_mut()
            .filter(|turn| turn.id == payload.turn_id && !turn.opened_explicitly)
        {
            turn.status = TurnStatus::InProgress;
            turn.started_at = payload.started_at;
            turn.opened_explicitly = true;
            return;
        }

        self.finish_current_turn();
        self.current_turn = Some(
            self.new_turn(Some(payload.turn_id.clone()))
                .with_status(TurnStatus::InProgress)
                .with_started_at(payload.started_at)
                .opened_explicitly(),
        );
    }

    fn handle_turn_context(&mut self, payload: &TurnContextItem) {
        let Some(turn_id) = payload
            .turn_id
            .as_ref()
            .filter(|turn_id| !turn_id.is_empty())
        else {
            return;
        };

        if self
            .current_turn
            .as_ref()
            .is_some_and(|turn| turn.id == *turn_id)
        {
            return;
        }

        if let Some(turn) = self.current_turn.as_mut()
            && !turn.opened_explicitly
            && (turn.items.is_empty() || turn.has_only_injected_context())
        {
            turn.id = turn_id.clone();
            turn.rollout_start_index = self.current_rollout_index;
            return;
        }

        self.finish_current_turn();
        self.current_turn = Some(self.new_turn(Some(turn_id.clone())));
    }

    fn handle_turn_complete(&mut self, payload: &TurnCompleteEvent) {
        let mark_completed = |turn: &mut PendingTurn| {
            if matches!(turn.status, TurnStatus::Completed | TurnStatus::InProgress) {
                turn.status = TurnStatus::Completed;
            }
            turn.completed_at = payload.completed_at;
            turn.duration_ms = payload.duration_ms;
        };

        // Prefer an exact ID match from the active turn and then close it.
        if let Some(current_turn) = self
            .current_turn
            .as_mut()
            .filter(|turn| turn.id == payload.turn_id)
        {
            mark_completed(current_turn);
            self.finish_current_turn();
            return;
        }

        if let Some(turn) = self
            .turns
            .iter_mut()
            .find(|turn| turn.id == payload.turn_id)
        {
            if matches!(turn.status, TurnStatus::Completed | TurnStatus::InProgress) {
                turn.status = TurnStatus::Completed;
            }
            turn.completed_at = payload.completed_at;
            turn.duration_ms = payload.duration_ms;
            return;
        }

        // If the completion event cannot be matched, apply it to the active turn.
        if let Some(current_turn) = self.current_turn.as_mut() {
            mark_completed(current_turn);
            self.finish_current_turn();
        }
    }

    /// Marks the current turn as containing a persisted compaction marker.
    ///
    /// This keeps compaction-only legacy turns from being dropped by
    /// `finish_current_turn` when they have no renderable items and were not
    /// explicitly opened.
    fn handle_compacted(&mut self, payload: &CompactedItem) {
        let replacement_history = payload
            .replacement_history
            .as_ref()
            .and_then(|history| serde_json::to_value(history).ok());
        {
            let turn = self.ensure_turn();
            turn.saw_compaction = true;

            if let Some(ThreadItem::ContextCompaction {
                replacement_history: existing_replacement_history,
                ..
            }) = turn
                .items
                .iter_mut()
                .rev()
                .find(|item| matches!(item, ThreadItem::ContextCompaction { .. }))
            {
                *existing_replacement_history = replacement_history;
                return;
            }
        }

        let id = self.next_item_id();
        let turn = self.ensure_turn();
        turn.items.push(ThreadItem::ContextCompaction {
            id,
            replacement_history,
        });
    }

    fn handle_thread_rollback(&mut self, payload: &ThreadRolledBackEvent) {
        self.finish_current_turn();

        let n = usize::try_from(payload.num_turns).unwrap_or(usize::MAX);
        if n >= self.turns.len() {
            self.turns.clear();
        } else {
            self.turns.truncate(self.turns.len().saturating_sub(n));
        }

        let item_count: usize = self.turns.iter().map(|t| t.items.len()).sum();
        self.next_item_index = i64::try_from(item_count.saturating_add(1)).unwrap_or(i64::MAX);
    }

    fn finish_current_turn(&mut self) {
        self.pending_agent_message_responses.clear();
        self.pending_legacy_agent_messages.clear();
        if let Some(turn) = self.current_turn.take() {
            if turn.items.is_empty() && !turn.opened_explicitly && !turn.saw_compaction {
                return;
            }
            self.turns.push(Turn::from(turn));
        }
    }

    fn new_turn(&mut self, id: Option<String>) -> PendingTurn {
        let id = id.unwrap_or_else(|| {
            if self.next_rollout_index == 0 {
                Uuid::now_v7().to_string()
            } else {
                format!("rollout-{}", self.current_rollout_index)
            }
        });
        PendingTurn {
            id,
            items: Vec::new(),
            error: None,
            status: TurnStatus::Completed,
            started_at: None,
            completed_at: None,
            duration_ms: None,
            opened_explicitly: false,
            saw_compaction: false,
            rollout_start_index: self.current_rollout_index,
        }
    }

    fn ensure_turn(&mut self) -> &mut PendingTurn {
        if self.current_turn.is_none() {
            let turn = self.new_turn(/*id*/ None);
            return self.current_turn.insert(turn);
        }

        if let Some(turn) = self.current_turn.as_mut() {
            return turn;
        }

        unreachable!("current turn must exist after initialization");
    }

    fn upsert_item_in_turn_id(&mut self, turn_id: &str, item: ThreadItem) {
        if let Some(turn) = self.current_turn.as_mut()
            && turn.id == turn_id
        {
            upsert_turn_item(&mut turn.items, item);
            return;
        }

        if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == turn_id) {
            upsert_turn_item(&mut turn.items, item);
            return;
        }

        warn!(
            item_id = item.id(),
            "dropping turn-scoped item for unknown turn id `{turn_id}`"
        );
    }

    fn upsert_item_in_turn_id_or_create(&mut self, turn_id: &str, item: ThreadItem) {
        if let Some(turn) = self.current_turn.as_mut()
            && turn.id == turn_id
        {
            upsert_turn_item(&mut turn.items, item);
            return;
        }

        if let Some(turn) = self.turns.iter_mut().find(|turn| turn.id == turn_id) {
            upsert_turn_item(&mut turn.items, item);
            return;
        }

        if let Some(turn) = self.current_turn.as_mut()
            && turn.opened_explicitly
            && matches!(turn.status, TurnStatus::InProgress)
        {
            upsert_turn_item(&mut turn.items, item);
            return;
        }

        self.finish_current_turn();
        let mut turn = self.new_turn(Some(turn_id.to_string()));
        upsert_turn_item(&mut turn.items, item);
        self.current_turn = Some(turn);
    }

    fn upsert_item_in_current_turn(&mut self, item: ThreadItem) {
        let turn = self.ensure_turn();
        upsert_turn_item(&mut turn.items, item);
    }

    fn upsert_event_driven_tool_call_in_current_turn(&mut self, item: ThreadItem) {
        let turn = self.ensure_turn();
        upsert_event_driven_tool_call(&mut turn.items, item);
    }

    fn find_event_driven_tool_call_in_current_turn(&self, item_id: &str) -> Option<&ThreadItem> {
        self.current_turn
            .as_ref()
            .and_then(|turn| turn.items.iter().find(|item| item.id() == item_id))
    }

    fn next_item_id(&mut self) -> String {
        let id = format!("item-{}", self.next_item_index);
        self.next_item_index += 1;
        id
    }

    fn build_user_inputs(&self, payload: &UserMessageEvent) -> Vec<UserInput> {
        let mut content = Vec::new();
        for skill in &payload.skills {
            content.push(UserInput::Skill {
                name: skill.name.clone(),
                path: skill.path.clone(),
            });
        }
        if !payload.message.trim().is_empty() {
            content.push(UserInput::Text {
                text: payload.message.clone(),
                text_elements: payload
                    .text_elements
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect(),
            });
        }
        if let Some(images) = &payload.images {
            for image in images {
                content.push(UserInput::Image { url: image.clone() });
            }
        }
        for path in &payload.local_images {
            content.push(UserInput::LocalImage { path: path.clone() });
        }
        content
    }
}

struct PendingAgentMessageResponse {
    id: String,
    text: String,
    phase: Option<MessagePhase>,
}

impl PendingAgentMessageResponse {
    fn matches(&self, text: &str, phase: Option<&MessagePhase>) -> bool {
        self.text == text && phases_are_compatible(self.phase.as_ref(), phase)
    }
}

struct PendingLegacyAgentMessage {
    id: String,
    text: String,
    phase: Option<MessagePhase>,
}

impl PendingLegacyAgentMessage {
    fn matches(&self, text: &str, phase: Option<&MessagePhase>) -> bool {
        self.text == text && phases_are_compatible(self.phase.as_ref(), phase)
    }
}

fn phases_are_compatible(
    response_phase: Option<&MessagePhase>,
    event_phase: Option<&MessagePhase>,
) -> bool {
    response_phase.is_none() || event_phase.is_none() || response_phase == event_phase
}

const REVIEW_FALLBACK_MESSAGE: &str = "Reviewer failed to output a response.";

fn render_review_output_text(output: &ReviewOutputEvent) -> String {
    let explanation = output.overall_explanation.trim();
    if explanation.is_empty() {
        REVIEW_FALLBACK_MESSAGE.to_string()
    } else {
        explanation.to_string()
    }
}

fn convert_dynamic_tool_content_items(
    items: &[codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem],
) -> Vec<DynamicToolCallOutputContentItem> {
    items
        .iter()
        .cloned()
        .map(|item| match item {
            codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem::InputText { text } => {
                DynamicToolCallOutputContentItem::InputText { text }
            }
            codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem::InputImage {
                image_url,
            } => DynamicToolCallOutputContentItem::InputImage { image_url },
        })
        .collect()
}

fn single_text_message_content(content: &[ContentItem]) -> Option<&str> {
    match content {
        [ContentItem::InputText { text }] | [ContentItem::OutputText { text }] => Some(text),
        _ => None,
    }
}

fn parse_injected_context_sections(
    role: &str,
    content: &[ContentItem],
) -> Vec<InjectedContextSection> {
    content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } => parse_injected_context_section(role, text),
            ContentItem::InputImage { .. } | ContentItem::OutputText { .. } => None,
        })
        .collect()
}

fn parse_injected_context_section(role: &str, text: &str) -> Option<InjectedContextSection> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let tagged_sections = [
        (
            "Permissions",
            "<permissions instructions>",
            "</permissions instructions>",
        ),
        ("Model", "<model_switch>", "</model_switch>"),
        (
            "Collaboration mode",
            COLLABORATION_MODE_OPEN_TAG,
            COLLABORATION_MODE_CLOSE_TAG,
        ),
        ("Personality", "<personality_spec>", "</personality_spec>"),
        (
            "Apps",
            APPS_INSTRUCTIONS_OPEN_TAG,
            APPS_INSTRUCTIONS_CLOSE_TAG,
        ),
        (
            "Skills",
            SKILLS_INSTRUCTIONS_OPEN_TAG,
            SKILLS_INSTRUCTIONS_CLOSE_TAG,
        ),
        (
            "Plugins",
            PLUGINS_INSTRUCTIONS_OPEN_TAG,
            PLUGINS_INSTRUCTIONS_CLOSE_TAG,
        ),
        (
            "Environment",
            ENVIRONMENT_CONTEXT_OPEN_TAG,
            ENVIRONMENT_CONTEXT_CLOSE_TAG,
        ),
        (
            "Multiagent",
            "<multiagent_context>",
            "</multiagent_context>",
        ),
        (
            "Realtime",
            REALTIME_CONVERSATION_OPEN_TAG,
            REALTIME_CONVERSATION_CLOSE_TAG,
        ),
    ];

    for (label, start, end) in tagged_sections {
        if let Some(section) = build_injected_context_section(label, trimmed, start, end) {
            return Some(section);
        }
    }

    if let Some(section) = parse_skill_injected_context_section(trimmed) {
        return Some(section);
    }

    if trimmed.starts_with("# AGENTS.md instructions for ") && trimmed.ends_with("</INSTRUCTIONS>")
    {
        return Some(InjectedContextSection {
            label: "AGENTS.md instructions".to_string(),
            text: trimmed.to_string(),
        });
    }

    if role == "developer" {
        return Some(InjectedContextSection {
            label: "Developer instructions".to_string(),
            text: trimmed.to_string(),
        });
    }

    None
}

fn parse_skill_injected_context_section(text: &str) -> Option<InjectedContextSection> {
    const SKILL_OPEN_TAG: &str = "<skill>";
    const SKILL_CLOSE_TAG: &str = "</skill>";

    let body = text
        .strip_prefix(SKILL_OPEN_TAG)?
        .strip_suffix(SKILL_CLOSE_TAG)?
        .trim();
    let (name, name_end) = extract_tag_value(body, "name")?;
    let body_after_name = body.get(name_end..)?.trim_start();
    let (path, path_end) = extract_tag_value(body_after_name, "path")?;
    let concrete = body_after_name.get(path_end..)?.trim();

    Some(InjectedContextSection {
        label: format!("Skill: {name}"),
        text: if concrete.is_empty() {
            format!("Path: {path}")
        } else {
            format!("Path: {path}\n\n{concrete}")
        },
    })
}

fn extract_tag_value<'a>(body: &'a str, tag: &str) -> Option<(&'a str, usize)> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let after_open = body.strip_prefix(open.as_str())?;
    let value_end = after_open.find(close.as_str())?;
    let value = after_open.get(..value_end)?;
    let consumed = open
        .len()
        .saturating_add(value_end)
        .saturating_add(close.len());
    Some((value.trim(), consumed))
}

fn build_injected_context_section(
    label: &str,
    text: &str,
    start_marker: &str,
    end_marker: &str,
) -> Option<InjectedContextSection> {
    let body = text
        .strip_prefix(start_marker)?
        .strip_suffix(end_marker)?
        .trim();
    Some(InjectedContextSection {
        label: label.to_string(),
        text: body.to_string(),
    })
}

fn build_injected_context_preview(sections: &[InjectedContextSection]) -> String {
    let labels: Vec<&str> = sections
        .iter()
        .map(|section| section.label.as_str())
        .take(MAX_INJECTED_CONTEXT_PREVIEW_SECTIONS)
        .collect();
    let remaining = sections
        .len()
        .saturating_sub(MAX_INJECTED_CONTEXT_PREVIEW_SECTIONS);
    if remaining == 0 {
        labels.join(" • ")
    } else {
        format!("{} • +{remaining} more", labels.join(" • "))
    }
}

fn upsert_turn_item(items: &mut Vec<ThreadItem>, item: ThreadItem) {
    if let Some(existing_item) = items
        .iter_mut()
        .find(|existing_item| existing_item.id() == item.id())
    {
        *existing_item = item;
        return;
    }
    items.push(item);
}

fn upsert_event_driven_tool_call(items: &mut Vec<ThreadItem>, item: ThreadItem) {
    if let Some(existing_item) = items
        .iter_mut()
        .find(|existing_item| existing_item.id() == item.id())
    {
        if matches!(
            existing_item,
            ThreadItem::EventDrivenToolCall { .. } | ThreadItem::EventCommandCall { .. }
        ) {
            *existing_item = item;
        }
        return;
    }
    items.push(item);
}

struct PendingTurn {
    id: String,
    items: Vec<ThreadItem>,
    error: Option<TurnError>,
    status: TurnStatus,
    started_at: Option<i64>,
    completed_at: Option<i64>,
    duration_ms: Option<i64>,
    /// True when this turn originated from an explicit `turn_started`/`turn_complete`
    /// boundary, so we preserve it even if it has no renderable items.
    opened_explicitly: bool,
    /// True when this turn includes a persisted `RolloutItem::Compacted`, which
    /// should keep the turn from being dropped even without normal items.
    saw_compaction: bool,
    /// Index of the rollout item that opened this turn during replay.
    rollout_start_index: usize,
}

impl PendingTurn {
    fn has_only_injected_context(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| matches!(item, ThreadItem::InjectedContext { .. }))
    }

    fn opened_explicitly(mut self) -> Self {
        self.opened_explicitly = true;
        self
    }

    fn with_status(mut self, status: TurnStatus) -> Self {
        self.status = status;
        self
    }

    fn with_started_at(mut self, started_at: Option<i64>) -> Self {
        self.started_at = started_at;
        self
    }
}

impl From<PendingTurn> for Turn {
    fn from(value: PendingTurn) -> Self {
        Self {
            id: value.id,
            items: value.items,
            items_view: TurnItemsView::Full,
            error: value.error,
            status: value.status,
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
        }
    }
}

impl From<&PendingTurn> for Turn {
    fn from(value: &PendingTurn) -> Self {
        Self {
            id: value.id.clone(),
            items: value.items.clone(),
            items_view: TurnItemsView::Full,
            error: value.error.clone(),
            status: value.status.clone(),
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v2::CollabAgentStatus;
    use crate::protocol::v2::CommandExecutionNotificationKind;
    use crate::protocol::v2::CommandExecutionNotifyOn;
    use crate::protocol::v2::CommandExecutionSource;
    use codex_protocol::AgentPath;
    use codex_protocol::ThreadId;
    use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem as CoreDynamicToolCallOutputContentItem;
    use codex_protocol::event_command::EventCommandEvent;
    use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
    use codex_protocol::items::AgentMessageContent as CoreAgentMessageContent;
    use codex_protocol::items::AgentMessageItem as CoreAgentMessageItem;
    use codex_protocol::items::CollabAgentMessageItem as CoreCollabAgentMessageItem;
    use codex_protocol::items::EventCommandEventItem as CoreEventCommandEventItem;
    use codex_protocol::items::EventDrivenToolItem as CoreEventDrivenToolItem;
    use codex_protocol::items::HookPromptFragment as CoreHookPromptFragment;
    use codex_protocol::items::TurnItem as CoreTurnItem;
    use codex_protocol::items::UserMessageItem as CoreUserMessageItem;
    use codex_protocol::items::build_hook_prompt_message;
    use codex_protocol::mcp::CallToolResult;
    use codex_protocol::models::ContentItem;
    use codex_protocol::models::FunctionCallOutputPayload;
    use codex_protocol::models::MessagePhase as CoreMessagePhase;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::models::WebSearchAction as CoreWebSearchAction;
    use codex_protocol::parse_command::ParsedCommand;
    use codex_protocol::protocol::AgentMessageEvent;
    use codex_protocol::protocol::AgentReasoningEvent;
    use codex_protocol::protocol::AgentReasoningRawContentEvent;
    use codex_protocol::protocol::ApplyPatchApprovalRequestEvent;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::CodexErrorInfo;
    use codex_protocol::protocol::CompactedItem;
    use codex_protocol::protocol::DynamicToolCallResponseEvent;
    use codex_protocol::protocol::ExecCommandEndEvent;
    use codex_protocol::protocol::ExecCommandSource;
    use codex_protocol::protocol::InterAgentCommunication;
    use codex_protocol::protocol::InterAgentOperation;
    use codex_protocol::protocol::ItemStartedEvent;
    use codex_protocol::protocol::McpInvocation;
    use codex_protocol::protocol::McpToolCallEndEvent;
    use codex_protocol::protocol::PatchApplyBeginEvent;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::protocol::ThreadRolledBackEvent;
    use codex_protocol::protocol::TurnAbortReason;
    use codex_protocol::protocol::TurnAbortedEvent;
    use codex_protocol::protocol::TurnCompleteEvent;
    use codex_protocol::protocol::TurnStartedEvent;
    use codex_protocol::protocol::UserMessageEvent;
    use codex_protocol::protocol::UserMessageSkill;
    use codex_protocol::protocol::WebSearchEndEvent;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;
    use std::time::Duration;
    use uuid::Uuid;

    fn turn_context_item_with_id(turn_id: &str) -> TurnContextItem {
        TurnContextItem {
            turn_id: Some(turn_id.to_string()),
            trace_id: None,
            cwd: PathBuf::from("/tmp"),
            current_date: None,
            timezone: None,
            approval_policy: AskForApproval::Never,
            sandbox_policy: SandboxPolicy::DangerFullAccess,
            permission_profile: None,
            network: None,
            file_system_sandbox_policy: None,
            model: "test-model".into(),
            personality: None,
            collaboration_mode: None,
            realtime_active: None,
            effort: None,
            summary: codex_protocol::config_types::ReasoningSummary::Auto,
            user_instructions: None,
            developer_instructions: None,
            final_output_json_schema: None,
            truncation_policy: None,
        }
    }

    fn completed_exec_event(call_id: &str, process_id: Option<String>) -> EventMsg {
        EventMsg::ExecCommandEnd(ExecCommandEndEvent {
            call_id: call_id.into(),
            process_id,
            turn_id: "turn-1".into(),
            completed_at_ms: 123,
            command: vec!["echo".into(), "done".into()],
            cwd: test_path_buf("/tmp").abs(),
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: "echo done".into(),
            }],
            source: ExecCommandSource::UnifiedExecStartup,
            interaction_input: None,
            initial_wait_ms: Some(1000),
            notify_on: Some(codex_protocol::protocol::ExecCommandNotifyOn::Exit),
            stdout: "done\n".into(),
            stderr: String::new(),
            aggregated_output: "done\n".into(),
            exit_code: 0,
            duration: Duration::from_millis(5),
            formatted_output: "done\n".into(),
            status: CoreExecCommandStatus::Completed,
        })
    }

    #[test]
    fn builds_multiple_turns_with_reasoning_items() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "First turn".into(),
                images: Some(vec!["https://example.com/one.png".into()]),
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Hi there".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "thinking".into(),
            }),
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent {
                text: "full reasoning".into(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Second turn".into(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Reply two".into(),
                phase: None,
                memory_citation: None,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();
        assert_eq!(turns.len(), 2);

        let first = &turns[0];
        assert!(Uuid::parse_str(&first.id).is_ok());
        assert_eq!(first.status, TurnStatus::Completed);
        assert_eq!(first.items.len(), 3);
        assert_eq!(
            first.items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![
                    UserInput::Text {
                        text: "First turn".into(),
                        text_elements: Vec::new(),
                    },
                    UserInput::Image {
                        url: "https://example.com/one.png".into(),
                    }
                ],
            }
        );
        assert_eq!(
            first.items[1],
            ThreadItem::AgentMessage {
                id: "item-2".into(),
                text: "Hi there".into(),
                phase: None,
                memory_citation: None,
            }
        );
        assert_eq!(
            first.items[2],
            ThreadItem::Reasoning {
                id: "item-3".into(),
                summary: vec!["thinking".into()],
                content: vec!["full reasoning".into()],
            }
        );

        let second = &turns[1];
        assert!(Uuid::parse_str(&second.id).is_ok());
        assert_ne!(first.id, second.id);
        assert_eq!(second.items.len(), 2);
        assert_eq!(
            second.items[0],
            ThreadItem::UserMessage {
                id: "item-4".into(),
                content: vec![UserInput::Text {
                    text: "Second turn".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
        assert_eq!(
            second.items[1],
            ThreadItem::AgentMessage {
                id: "item-5".into(),
                text: "Reply two".into(),
                phase: None,
                memory_citation: None,
            }
        );
    }

    #[test]
    fn live_inter_agent_json_message_is_not_displayed_as_agent_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "done".into(),
            InterAgentOperation::SendMessage,
        )
        .with_trigger_turn(false);
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: text,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn live_child_completion_json_message_is_not_displayed_as_agent_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: text,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn live_event_driven_tool_marker_is_not_displayed_as_agent_message() {
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: "<event_driven_tool>{\"tool\":\"process_exit_subscribe\",\"title\":\"Process exited\",\"text\":\"[Process exit subscription] Session 42 exited with code 0\"}</event_driven_tool>".into(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn maps_typed_event_driven_tool_completed_to_event_item() {
        let thread_id = ThreadId::new();
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id,
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventDrivenTool(CoreEventDrivenToolItem {
                    id: "event-1".into(),
                    tool: "process_exit_subscribe".into(),
                    title: "Process exited".into(),
                    text: "[Process exit subscription] Session 42 exited with code 0".into(),
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenTool {
                id: "event-1".into(),
                tool: "process_exit_subscribe".into(),
                title: "Process exited".into(),
                text: "[Process exit subscription] Session 42 exited with code 0".into(),
            }]
        );
    }

    #[test]
    fn maps_typed_agent_message_completed_to_agent_message() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::AgentMessage(CoreAgentMessageItem {
                    id: "agent-1".into(),
                    content: vec![CoreAgentMessageContent::Text {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "agent-1".into(),
                text: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn dedupes_typed_agent_message_completed_before_raw_response_item() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::AgentMessage(CoreAgentMessageItem {
                    id: "msg-1".into(),
                    content: vec![CoreAgentMessageContent::Text {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                }),
                completed_at_ms: 123,
            }),
            EventMsg::RawResponseItem(codex_protocol::protocol::RawResponseItemEvent {
                item: ResponseItem::Message {
                    id: Some("msg-1".into()),
                    role: "assistant".into(),
                    content: vec![ContentItem::OutputText {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                },
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "msg-1".into(),
                text: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn maps_typed_child_completion_completed_to_collab_status_update() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_typed_child_completion_without_turn_lifecycle() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(false)
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![RolloutItem::EventMsg(EventMsg::ItemCompleted(
            ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            },
        ))];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-1");
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_typed_child_completion_before_turn_started_without_duplicate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-1".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-1");
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_child_completion_inside_active_parent_turn_without_stealing_followup_items() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "parent continues".into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "parent-turn".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::CollabAgentStatusUpdate {
                    id: "collab-1".into(),
                    sender_thread_id: None,
                    sender_path: "/root/worker".into(),
                    recipient_thread_id: None,
                    recipient_path: "/root".into(),
                    status: CollabAgentState {
                        path: Some("/root/worker".into()),
                        status: CollabAgentStatus::Completed,
                        message: Some("completed".into()),
                    },
                },
                ThreadItem::AgentMessage {
                    id: "msg-1".into(),
                    text: "parent continues".into(),
                    phase: None,
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn restores_child_completion_after_aborted_parent_turn_as_separate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id: Some("parent-turn".into()),
                reason: TurnAbortReason::Interrupted,
                completed_at: None,
                duration_ms: None,
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(turns[0].status, TurnStatus::Interrupted);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
        assert_eq!(turns[1].id, "child-completion-turn");
        assert_eq!(turns[1].status, TurnStatus::Completed);
        assert_eq!(
            turns[1].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_child_completion_after_failed_parent_turn_as_separate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::Error(ErrorEvent {
                message: "stream failure".into(),
                codex_error_info: Some(CodexErrorInfo::ResponseStreamDisconnected {
                    http_status_code: Some(502),
                }),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(turns[0].status, TurnStatus::Failed);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
        assert_eq!(turns[1].id, "child-completion-turn");
        assert_eq!(turns[1].status, TurnStatus::Completed);
        assert_eq!(
            turns[1].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn keeps_unmarked_event_driven_tool_json_as_agent_message() {
        let message = serde_json::json!({
            "tool": "process_exit_subscribe",
            "title": "Process exited",
            "text": "[Process exit subscription] Session 42 exited with code 0",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn filters_malformed_event_driven_tool_marker_as_agent_message() {
        let message = concat!(
            "<event_driven_tool>",
            r#"{"tool":"process_exit_subscribe","title":"Process exited""#,
            "</event_driven_tool>"
        )
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert!(turns.is_empty());
    }

    #[test]
    fn keeps_unknown_inter_agent_shaped_json_as_agent_message() {
        let message = serde_json::json!({
            "author": "/root/worker",
            "recipient": "/root",
            "content": "plain assistant json",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn filters_raw_send_message_envelope_as_agent_message() {
        for operation in ["sendMessage", "send_message"] {
            let message = serde_json::json!({
                "author": "/root/worker",
                "recipient": "/root",
                "content": "legacy message",
                "operation": operation,
            })
            .to_string();
            let events = [EventMsg::AgentMessage(AgentMessageEvent {
                message,
                phase: None,
                memory_citation: None,
            })];

            let mut builder = ThreadHistoryBuilder::new();
            for event in &events {
                builder.handle_event(event);
            }
            let turns = builder.finish();

            assert!(turns.is_empty());
        }
    }

    #[test]
    fn preserves_loaded_skills_in_user_message_history() {
        let skill_path = test_path_buf("/tmp/skills/demo/SKILL.md");
        let events = vec![EventMsg::UserMessage(UserMessageEvent {
            message: "Use the selected skill.".into(),
            images: None,
            local_images: Vec::new(),
            skills: vec![UserMessageSkill {
                name: "demo".into(),
                path: skill_path.clone(),
            }],
            text_elements: Vec::new(),
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }

        let turns = builder.finish();

        assert_eq!(
            turns[0].items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![
                    UserInput::Skill {
                        name: "demo".into(),
                        path: skill_path,
                    },
                    UserInput::Text {
                        text: "Use the selected skill.".into(),
                        text_elements: Vec::new(),
                    },
                ],
            }
        );
    }

    #[test]
    fn ignores_non_plan_item_lifecycle_events() {
        let turn_id = "turn-1";
        let thread_id = ThreadId::new();
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id,
                turn_id: turn_id.to_string(),
                item: CoreTurnItem::UserMessage(CoreUserMessageItem {
                    id: "user-item-id".to_string(),
                    content: Vec::new(),
                }),
                started_at_ms: 0,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: turn_id.to_string(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
    }

    #[test]
    fn preserves_agent_message_phase_in_history() {
        let events = vec![EventMsg::AgentMessage(AgentMessageEvent {
            message: "Final reply".into(),
            phase: Some(CoreMessagePhase::FinalAnswer),
            memory_citation: None,
        })];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: "Final reply".into(),
                phase: Some(MessagePhase::FinalAnswer),
                memory_citation: None,
            }
        );
    }

    #[test]
    fn replays_image_generation_end_events_into_turn_history() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-image".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "generate an image".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
            RolloutItem::EventMsg(EventMsg::ImageGenerationEnd(ImageGenerationEndEvent {
                call_id: "ig_123".into(),
                status: "completed".into(),
                revised_prompt: Some("final prompt".into()),
                result: "Zm9v".into(),
                saved_path: Some(test_path_buf("/tmp/ig_123.png").abs()),
            })),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-image".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0],
            Turn {
                id: "turn-image".into(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                items_view: TurnItemsView::Full,
                items: vec![
                    ThreadItem::UserMessage {
                        id: "item-1".into(),
                        content: vec![UserInput::Text {
                            text: "generate an image".into(),
                            text_elements: Vec::new(),
                        }],
                    },
                    ThreadItem::ImageGeneration {
                        id: "ig_123".into(),
                        status: "completed".into(),
                        revised_prompt: Some("final prompt".into()),
                        result: "Zm9v".into(),
                        saved_path: Some(test_path_buf("/tmp/ig_123.png").abs()),
                    },
                ],
            }
        );
    }

    #[test]
    fn splits_reasoning_when_interleaved() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "Turn start".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "first summary".into(),
            }),
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent {
                text: "first content".into(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "interlude".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "second summary".into(),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        let turn = &turns[0];
        assert_eq!(turn.items.len(), 4);

        assert_eq!(
            turn.items[1],
            ThreadItem::Reasoning {
                id: "item-2".into(),
                summary: vec!["first summary".into()],
                content: vec!["first content".into()],
            }
        );
        assert_eq!(
            turn.items[3],
            ThreadItem::Reasoning {
                id: "item-4".into(),
                summary: vec!["second summary".into()],
                content: Vec::new(),
            }
        );
    }

    #[test]
    fn marks_turn_as_interrupted_when_aborted() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "Please do the thing".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Working...".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id: Some("turn-1".into()),
                reason: TurnAbortReason::Replaced,
                completed_at: None,
                duration_ms: None,
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Let's try again".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Second attempt complete.".into(),
                phase: None,
                memory_citation: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 2);

        let first_turn = &turns[0];
        assert_eq!(first_turn.status, TurnStatus::Interrupted);
        assert_eq!(first_turn.items.len(), 2);
        assert_eq!(
            first_turn.items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "Please do the thing".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
        assert_eq!(
            first_turn.items[1],
            ThreadItem::AgentMessage {
                id: "item-2".into(),
                text: "Working...".into(),
                phase: None,
                memory_citation: None,
            }
        );

        let second_turn = &turns[1];
        assert_eq!(second_turn.status, TurnStatus::Completed);
        assert_eq!(second_turn.items.len(), 2);
        assert_eq!(
            second_turn.items[0],
            ThreadItem::UserMessage {
                id: "item-3".into(),
                content: vec![UserInput::Text {
                    text: "Let's try again".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
        assert_eq!(
            second_turn.items[1],
            ThreadItem::AgentMessage {
                id: "item-4".into(),
                text: "Second attempt complete.".into(),
                phase: None,
                memory_citation: None,
            }
        );
    }

    #[test]
    fn drops_last_turns_on_thread_rollback() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "First".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "A1".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Second".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "A2".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::ThreadRolledBack(ThreadRolledBackEvent { num_turns: 1 }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Third".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "A3".into(),
                phase: None,
                memory_citation: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "rollout-0");
        assert_eq!(turns[1].id, "rollout-5");
        assert_ne!(turns[0].id, turns[1].id);
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(turns[1].status, TurnStatus::Completed);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "First".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::AgentMessage {
                    id: "item-2".into(),
                    text: "A1".into(),
                    phase: None,
                    memory_citation: None,
                },
            ]
        );
        assert_eq!(
            turns[1].items,
            vec![
                ThreadItem::UserMessage {
                    id: "item-3".into(),
                    content: vec![UserInput::Text {
                        text: "Third".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::AgentMessage {
                    id: "item-4".into(),
                    text: "A3".into(),
                    phase: None,
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn thread_rollback_clears_all_turns_when_num_turns_exceeds_history() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "One".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "A1".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Two".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "A2".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::ThreadRolledBack(ThreadRolledBackEvent { num_turns: 99 }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns, Vec::<Turn>::new());
    }

    #[test]
    fn uses_explicit_turn_boundaries_for_mid_turn_steering() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Start".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Steer".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "Start".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::UserMessage {
                    id: "item-2".into(),
                    content: vec![UserInput::Text {
                        text: "Steer".into(),
                        text_elements: Vec::new(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn reconstructs_tool_items_from_persisted_completion_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run tools".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::WebSearchEnd(WebSearchEndEvent {
                call_id: "search-1".into(),
                query: "codex".into(),
                action: CoreWebSearchAction::Search {
                    query: Some("codex".into()),
                    queries: None,
                },
            }),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".into(),
                process_id: Some("pid-1".into()),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "hello world".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo hello world".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: String::new(),
                stderr: String::new(),
                aggregated_output: "hello world\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(12),
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Completed,
            }),
            EventMsg::McpToolCallEnd(McpToolCallEndEvent {
                call_id: "mcp-1".into(),
                invocation: McpInvocation {
                    server: "docs".into(),
                    tool: "lookup".into(),
                    arguments: Some(serde_json::json!({"id":"123"})),
                },
                mcp_app_resource_uri: None,
                duration: Duration::from_millis(8),
                result: Err("boom".into()),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 5);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::WebSearch {
                id: "search-1".into(),
                query: "codex".into(),
                action: Some(WebSearchAction::Search {
                    query: Some("codex".into()),
                    queries: None,
                }),
            }
        );
        assert_eq!(
            turns[0].items[2],
            ThreadItem::CommandExecution {
                id: "exec-1".into(),
                command: "echo 'hello world'".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-1".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "echo hello world".into(),
                }],
                aggregated_output: Some("hello world\n".into()),
                exit_code: Some(0),
                duration_ms: Some(12),
            }
        );
        assert_eq!(
            turns[0].items[3],
            ThreadItem::CommandExecutionNotification {
                id: "exec-1:notification:exit".into(),
                command_item_id: "exec-1".into(),
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".into(),
                output: None,
                exit_code: Some(0),
                created_at_ms: 0,
            }
        );
        assert_eq!(
            turns[0].items[4],
            ThreadItem::McpToolCall {
                id: "mcp-1".into(),
                server: "docs".into(),
                tool: "lookup".into(),
                status: McpToolCallStatus::Failed,
                arguments: serde_json::json!({"id":"123"}),
                mcp_app_resource_uri: None,
                result: None,
                error: Some(McpToolCallError {
                    message: "boom".into(),
                }),
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn inline_completed_exec_replay_does_not_create_exit_notification_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(completed_exec_event("exec-inline", None)),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CommandExecution {
                id: "exec-inline".into(),
                command: "echo done".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::UnifiedExecStartup,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: Some(1000),
                notify_on: Some(CommandExecutionNotifyOn::Exit),
                command_actions: vec![CommandAction::Unknown {
                    command: "echo done".into(),
                }],
                aggregated_output: Some("done\n".into()),
                exit_code: Some(0),
                duration_ms: Some(5),
            }]
        );
    }

    #[test]
    fn background_exec_replay_creates_exit_notification_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(completed_exec_event(
                "exec-background",
                Some("pid-42".into()),
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecutionNotification {
                id: "exec-background:notification:exit".into(),
                command_item_id: "exec-background".into(),
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".into(),
                output: None,
                exit_code: Some(0),
                created_at_ms: 123,
            }
        );
    }

    #[test]
    fn reconstructs_mcp_tool_result_meta_from_persisted_completion_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::McpToolCallEnd(McpToolCallEndEvent {
                call_id: "mcp-1".into(),
                invocation: McpInvocation {
                    server: "docs".into(),
                    tool: "lookup".into(),
                    arguments: Some(serde_json::json!({"id":"123"})),
                },
                mcp_app_resource_uri: Some("ui://widget/lookup.html".into()),
                duration: Duration::from_millis(8),
                result: Ok(CallToolResult {
                    content: vec![serde_json::json!({
                        "type": "text",
                        "text": "result"
                    })],
                    structured_content: Some(serde_json::json!({"id":"123"})),
                    is_error: Some(false),
                    meta: Some(serde_json::json!({
                        "ui/resourceUri": "ui://widget/lookup.html"
                    })),
                }),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::McpToolCall {
                id: "mcp-1".into(),
                server: "docs".into(),
                tool: "lookup".into(),
                status: McpToolCallStatus::Completed,
                arguments: serde_json::json!({"id":"123"}),
                mcp_app_resource_uri: Some("ui://widget/lookup.html".into()),
                result: Some(Box::new(McpToolCallResult {
                    content: vec![serde_json::json!({
                        "type": "text",
                        "text": "result"
                    })],
                    structured_content: Some(serde_json::json!({"id":"123"})),
                    meta: Some(serde_json::json!({
                        "ui/resourceUri": "ui://widget/lookup.html"
                    })),
                })),
                error: None,
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn reconstructs_dynamic_tool_items_from_request_and_response_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run dynamic tool".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::DynamicToolCallRequest(
                codex_protocol::dynamic_tools::DynamicToolCallRequest {
                    call_id: "dyn-1".into(),
                    turn_id: "turn-1".into(),
                    started_at_ms: 0,
                    namespace: Some("codex_app".into()),
                    tool: "lookup_ticket".into(),
                    arguments: serde_json::json!({"id":"ABC-123"}),
                },
            ),
            EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
                call_id: "dyn-1".into(),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                namespace: Some("codex_app".into()),
                tool: "lookup_ticket".into(),
                arguments: serde_json::json!({"id":"ABC-123"}),
                content_items: vec![CoreDynamicToolCallOutputContentItem::InputText {
                    text: "Ticket is open".into(),
                }],
                success: true,
                error: None,
                duration: Duration::from_millis(42),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::DynamicToolCall {
                id: "dyn-1".into(),
                namespace: Some("codex_app".into()),
                tool: "lookup_ticket".into(),
                arguments: serde_json::json!({"id":"ABC-123"}),
                status: DynamicToolCallStatus::Completed,
                content_items: Some(vec![DynamicToolCallOutputContentItem::InputText {
                    text: "Ticket is open".into(),
                }]),
                success: Some(true),
                duration_ms: Some(42),
            }
        );
    }

    #[test]
    fn reconstructs_event_driven_tool_items_from_raw_response_history() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "remind me".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "schedule_subscribe".into(),
                namespace: None,
                arguments: r#"{"schedule":{"once_after":{"seconds":60}},"label":"standup"}"#.into(),
                call_id: "builtin-1".into(),
            }),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                call_id: "builtin-1".into(),
                output: FunctionCallOutputPayload::from_text(
                    r#"{"schedule_summary":"once after 60s"}"#.into(),
                ),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::EventDrivenToolCall {
                id: "builtin-1".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({
                    "schedule": {"once_after": {"seconds": 60}},
                    "label": "standup",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::Value::String(
                    r#"{"schedule_summary":"once after 60s"}"#.into(),
                )),
            }
        );
    }

    #[test]
    fn event_command_write_stdin_history_replays_as_event_driven_tool() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "event_command_write_stdin".into(),
                namespace: None,
                arguments: r#"{"subscription_id":"sub-1","chars":"\n"}"#.into(),
                call_id: "builtin-1".into(),
            }),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                call_id: "builtin-1".into(),
                output: FunctionCallOutputPayload::from_text(
                    r#"{"subscription_id":"sub-1","bytes_written":1}"#.into(),
                ),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::EventDrivenToolCall {
                id: "builtin-1".into(),
                tool: "event_command_write_stdin".into(),
                arguments: serde_json::json!({
                    "subscription_id": "sub-1",
                    "chars": "\n",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::Value::String(
                    r#"{"subscription_id":"sub-1","bytes_written":1}"#.into(),
                )),
            }
        );
    }

    #[test]
    fn event_command_event_history_uses_stable_event_id() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".into(),
            kind: codex_protocol::event_command::EventCommandEventKind::Exited,
            label: Some("tests".into()),
            command: "cargo test".into(),
            cwd: None,
            line: None,
            sequence: None,
            exit_code: Some(0),
            signal: None,
            message: Some("done".into()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let expected_id = event.stable_item_id();
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::EventCommandEvent { id: None, event }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        let ThreadItem::EventCommandEvent { id, .. } = &turns[0].items[0] else {
            panic!("expected event command event item");
        };
        assert_eq!(id, &expected_id);
    }

    #[test]
    fn typed_event_command_event_history_uses_typed_item_id() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".into(),
            kind: codex_protocol::event_command::EventCommandEventKind::Exited,
            label: Some("tests".into()),
            command: "cargo test".into(),
            cwd: None,
            line: None,
            sequence: None,
            exit_code: Some(0),
            signal: None,
            message: Some("done".into()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::EventCommandEvent {
                id: Some("typed-event-command".into()),
                event,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventCommandEvent {
                id: "typed-event-command".into(),
                subscription_id: "sub-1".into(),
                kind: crate::protocol::v2::EventCommandEventKind::Exited,
                label: Some("tests".into()),
                command: "cargo test".into(),
                cwd: None,
                line: None,
                sequence: None,
                exit_code: Some(0),
                signal: None,
                message: Some("done".into()),
                truncated: false,
                created_at: 1_700_000_000,
            }]
        );
    }

    #[test]
    fn legacy_event_command_completed_history_rebuilds_thread_item() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".into(),
            kind: codex_protocol::event_command::EventCommandEventKind::Exited,
            label: Some("tests".into()),
            command: "cargo test".into(),
            cwd: None,
            line: None,
            sequence: Some(3),
            exit_code: Some(0),
            signal: None,
            message: Some("done".into()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventCommandEvent(CoreEventCommandEventItem {
                    id: "event-command-1".into(),
                    event,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventCommandEvent {
                id: "event-command-1".into(),
                subscription_id: "sub-1".into(),
                kind: crate::protocol::v2::EventCommandEventKind::Exited,
                label: Some("tests".into()),
                command: "cargo test".into(),
                cwd: None,
                line: None,
                sequence: Some(3),
                exit_code: Some(0),
                signal: None,
                message: Some("done".into()),
                truncated: false,
                created_at: 1_700_000_000,
            }]
        );
    }

    #[test]
    fn legacy_event_command_started_history_does_not_create_completed_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventCommandEvent(CoreEventCommandEventItem {
                    id: "event-command-1".into(),
                    event: EventCommandEvent {
                        subscription_id: "sub-1".into(),
                        kind: codex_protocol::event_command::EventCommandEventKind::Output,
                        label: Some("tests".into()),
                        command: "cargo test".into(),
                        cwd: None,
                        line: Some("running".into()),
                        sequence: Some(3),
                        exit_code: None,
                        signal: None,
                        message: None,
                        truncated: false,
                        created_at: 1_700_000_000,
                    },
                }),
                started_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn typed_event_driven_tool_history_rebuilds_trigger_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::EventDrivenTool {
                id: Some("typed-trigger".into()),
                trigger: EventDrivenToolTrigger {
                    tool: "fs_subscribe".into(),
                    title: "File watch triggered".into(),
                    text: "build.log changed".into(),
                },
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenTool {
                id: "typed-trigger".into(),
                tool: "fs_subscribe".into(),
                title: "File watch triggered".into(),
                text: "build.log changed".into(),
            }]
        );
    }

    #[test]
    fn typed_workflow_progress_history_rebuilds_thread_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::WorkflowRunProgress {
                id: None,
                event: codex_protocol::models::WorkflowRunProgressEvent {
                    run_id: "wf_1".into(),
                    workflow_id: "feature-dev".into(),
                    status: serde_json::json!("running"),
                    runner_status: "control_plane_started".into(),
                    kind: codex_protocol::models::WorkflowRunProgressKind::Started,
                    message: "workflow control run started".into(),
                    updated_at: 1_700_000_000,
                },
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::WorkflowRunProgress {
                id: "item-1".into(),
                event: crate::protocol::v2::ThreadWorkflowRunProgressEvent {
                    run_id: "wf_1".into(),
                    workflow_id: "feature-dev".into(),
                    status: serde_json::json!("running"),
                    runner_status: "control_plane_started".into(),
                    kind: crate::protocol::v2::ThreadWorkflowRunProgressKind::Started,
                    message: "workflow control run started".into(),
                    updated_at: 1_700_000_000,
                },
            }]
        );
    }

    #[test]
    fn typed_inter_agent_history_rebuilds_collab_item_without_agent_message_leak() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(codex_protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::InterAgentCommunication {
                id: Some("typed-collab".into()),
                communication,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "typed-collab".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn typed_unknown_inter_agent_history_falls_back_to_agent_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "raw json should not leak".into(),
            InterAgentOperation::Unknown,
        );
        let expected_text = serde_json::to_string(&communication).expect("serialize communication");
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::InterAgentCommunication {
                id: Some("typed-unknown-collab".into()),
                communication,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "typed-unknown-collab".into(),
                text: expected_text,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn event_driven_tool_replay_does_not_override_specialized_items() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".into(),
                namespace: None,
                arguments: r#"{"cmd":"ls"}"#.into(),
                call_id: "call-1".into(),
            }),
            RolloutItem::EventMsg(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "call-1".into(),
                process_id: Some("pid-1".into()),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                command: vec!["ls".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown { cmd: "ls".into() }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: String::new(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: Duration::ZERO,
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Completed,
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                call_id: "call-1".into(),
                output: FunctionCallOutputPayload::from_text("ok".into()),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::CommandExecution {
                id: "call-1".into(),
                command: "ls".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-1".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "ls".into(),
                }],
                aggregated_output: None,
                exit_code: Some(0),
                duration_ms: Some(0),
            }
        );
    }

    #[test]
    fn reconstructs_typed_event_driven_tool_items_from_response_items() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::EventDrivenTool {
                id: Some("schedule-trigger-1".into()),
                trigger: EventDrivenToolTrigger {
                    tool: "schedule_subscribe".into(),
                    title: "Schedule triggered".into(),
                    text: "[Schedule subscription] Trigger fired: every 5 minutes".into(),
                },
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::EventDrivenTool {
                id: "schedule-trigger-1".into(),
                tool: "schedule_subscribe".into(),
                title: "Schedule triggered".into(),
                text: "[Schedule subscription] Trigger fired: every 5 minutes".into(),
            }
        );
    }

    #[test]
    fn response_item_completed_event_does_not_duplicate_command_wait_history() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::CommandWait {
                id: None,
                command_id: "cmd-1".into(),
                status: codex_protocol::models::CommandWaitStatus::Completed,
                notification: Some(codex_protocol::models::CommandWaitNotificationKind::Exit),
                exit_code: Some(0),
                wall_time_seconds: 1.25,
                created_at_ms: 1234,
            }),
            RolloutItem::EventMsg(EventMsg::ResponseItemCompleted(
                codex_protocol::protocol::ResponseItemCompletedEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    item: ResponseItem::CommandWait {
                        id: None,
                        command_id: "cmd-1".into(),
                        status: codex_protocol::models::CommandWaitStatus::Completed,
                        notification: Some(
                            codex_protocol::models::CommandWaitNotificationKind::Exit,
                        ),
                        exit_code: Some(0),
                        wall_time_seconds: 1.25,
                        created_at_ms: 1234,
                    },
                    completed_at_ms: 5678,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        let ThreadItem::CommandWait {
            command_id,
            status,
            notification,
            exit_code,
            wall_time_seconds,
            created_at_ms,
            ..
        } = &turns[0].items[0]
        else {
            panic!("expected CommandWait item");
        };
        assert_eq!(command_id, "cmd-1");
        assert_eq!(*status, crate::protocol::v2::CommandWaitStatus::Completed);
        assert_eq!(
            *notification,
            Some(crate::protocol::v2::CommandWaitNotificationKind::Exit)
        );
        assert_eq!(*exit_code, Some(0));
        assert_eq!(*wall_time_seconds, 1.25);
        assert_eq!(*created_at_ms, 1234);
    }

    #[test]
    fn legacy_event_driven_tool_response_message_is_not_displayed_as_agent_message() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("process-exit-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "<event_driven_tool>{\"tool\":\"process_exit_subscribe\",\"title\":\"Process exited\",\"text\":\"Session 42 exited with code 0\"}</event_driven_tool>".into(),
                }],
                phase: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn raw_assistant_response_item_updates_current_turn_history() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::RawResponseItem(codex_protocol::protocol::RawResponseItemEvent {
                item: ResponseItem::Message {
                    id: Some("msg-1".into()),
                    role: "assistant".into(),
                    content: vec![ContentItem::OutputText {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                },
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "msg-1".into(),
                text: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn reconstructs_agent_message_envelopes_from_assistant_response_messages() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::try_from("/root").expect("agent path"),
            Vec::new(),
            "done".into(),
            InterAgentOperation::SendMessage,
        )
        .with_trigger_turn(false);
        let unknown_operation_json = serde_json::json!({
            "author": "/root/worker",
            "recipient": "/root",
            "content": "plain assistant json",
        })
        .to_string();
        let plain_response_text = "final answer".to_string();
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: serde_json::to_string(&communication).expect("serialize communication"),
                }],
                phase: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-2".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: unknown_operation_json.clone(),
                }],
                phase: Some(CoreMessagePhase::Commentary),
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-3".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: plain_response_text.clone(),
                }],
                phase: Some(CoreMessagePhase::FinalAnswer),
            }),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: plain_response_text.clone(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::AgentMessage {
                    id: "msg-2".into(),
                    text: unknown_operation_json,
                    phase: Some(CoreMessagePhase::Commentary),
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "msg-3".into(),
                    text: plain_response_text,
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn rollout_turn_context_restores_implicit_turn_id() {
        let items = vec![
            RolloutItem::TurnContext(turn_context_item_with_id("turn-from-context")),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "hello from replay".into(),
                }],
                phase: Some(CoreMessagePhase::FinalAnswer),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-from-context");
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "msg-1".into(),
                text: "hello from replay".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn rollout_turn_context_restores_following_implicit_user_turn_id() {
        let items = vec![
            RolloutItem::TurnContext(turn_context_item_with_id("turn-from-context")),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-from-context");
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
            }]
        );
    }

    #[test]
    fn rollout_turn_context_restores_id_after_initial_injected_context() {
        let items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![ContentItem::InputText {
                    text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                        .into(),
                }],
                phase: None,
            }),
            RolloutItem::TurnContext(turn_context_item_with_id("turn-from-context")),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-from-context");
        assert_eq!(turns[0].items.len(), 2);
        assert!(matches!(
            turns[0].items[0],
            ThreadItem::InjectedContext { .. }
        ));
        assert!(matches!(turns[0].items[1], ThreadItem::UserMessage { .. }));
    }

    #[test]
    fn dedupes_response_agent_message_across_intervening_events() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "final answer".into(),
                }],
                phase: Some(CoreMessagePhase::FinalAnswer),
            }),
            RolloutItem::EventMsg(EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "reasoning emitted later".into(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::AgentMessage {
                    id: "msg-1".into(),
                    text: "final answer".into(),
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                },
                ThreadItem::Reasoning {
                    id: "item-1".into(),
                    summary: vec!["reasoning emitted later".into()],
                    content: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn dedupes_legacy_agent_message_when_response_item_arrives_later() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "final answer".into(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::EventMsg(EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "reasoning emitted later".into(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "final answer".into(),
                }],
                phase: Some(CoreMessagePhase::FinalAnswer),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::AgentMessage {
                    id: "msg-1".into(),
                    text: "final answer".into(),
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                },
                ThreadItem::Reasoning {
                    id: "item-2".into(),
                    summary: vec!["reasoning emitted later".into()],
                    content: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn preserves_legitimate_repeated_legacy_agent_messages() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "repeat me".into(),
                phase: Some(CoreMessagePhase::Commentary),
                memory_citation: None,
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "repeat me".into(),
                phase: Some(CoreMessagePhase::Commentary),
                memory_citation: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::AgentMessage {
                    id: "item-1".into(),
                    text: "repeat me".into(),
                    phase: Some(CoreMessagePhase::Commentary),
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "item-2".into(),
                    text: "repeat me".into(),
                    phase: Some(CoreMessagePhase::Commentary),
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn duplicate_agent_message_response_updates_plain_agent_message() {
        let trigger = EventDrivenToolTrigger {
            tool: "schedule_subscribe".into(),
            title: "Schedule triggered".into(),
            text: "[Schedule subscription] Trigger fired: every 5 minutes".into(),
        };
        let text = trigger.render_message_text();
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: text.clone(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText { text }],
                phase: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn inter_agent_json_agent_message_does_not_leak_when_response_item_arrives_later() {
        let mut communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::try_from("/root").expect("agent path"),
            Vec::new(),
            "done".into(),
            InterAgentOperation::SendMessage,
        );
        communication.trigger_turn = false;
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: text.clone(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText { text }],
                phase: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn child_completion_json_agent_message_does_not_leak_when_response_item_arrives_later() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::try_from("/root").expect("agent path"),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(AgentStatus::Completed(Some("completed".into())));
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: text.clone(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText { text }],
                phase: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn reconstructs_declined_exec_and_patch_items() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run tools".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-declined".into(),
                process_id: Some("pid-2".into()),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                command: vec!["ls".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown { cmd: "ls".into() }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: String::new(),
                stderr: "exec command rejected by user".into(),
                aggregated_output: "exec command rejected by user".into(),
                exit_code: -1,
                duration: Duration::ZERO,
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Declined,
            }),
            EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                call_id: "patch-declined".into(),
                turn_id: "turn-1".into(),
                stdout: String::new(),
                stderr: "patch rejected by user".into(),
                success: false,
                changes: [(
                    PathBuf::from("README.md"),
                    codex_protocol::protocol::FileChange::Add {
                        content: "hello\n".into(),
                    },
                )]
                .into_iter()
                .collect(),
                status: CorePatchApplyStatus::Declined,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 4);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "exec-declined".into(),
                command: "ls".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-2".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Declined,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "ls".into(),
                }],
                aggregated_output: Some("exec command rejected by user".into()),
                exit_code: Some(-1),
                duration_ms: Some(0),
            }
        );
        assert_eq!(
            turns[0].items[2],
            ThreadItem::CommandExecutionNotification {
                id: "exec-declined:notification:exit".into(),
                command_item_id: "exec-declined".into(),
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".into(),
                output: None,
                exit_code: Some(-1),
                created_at_ms: 0,
            }
        );
        assert_eq!(
            turns[0].items[3],
            ThreadItem::FileChange {
                id: "patch-declined".into(),
                changes: vec![FileUpdateChange {
                    path: "README.md".into(),
                    kind: PatchChangeKind::Add,
                    diff: "hello\n".into(),
                }],
                status: PatchApplyStatus::Declined,
            }
        );
    }

    #[test]
    fn reconstructs_declined_guardian_command_item() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "review this command".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-exec".into(),
                target_item_id: Some("guardian-exec".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 1_000,
                completed_at_ms: None,
                status: GuardianAssessmentStatus::InProgress,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: None,
                action: serde_json::from_value(serde_json::json!({
                    "type": "command",
                    "source": "shell",
                    "command": "rm -rf /tmp/guardian",
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-exec".into(),
                target_item_id: Some("guardian-exec".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 1_000,
                completed_at_ms: Some(1_042),
                status: GuardianAssessmentStatus::Denied,
                risk_level: Some(codex_protocol::protocol::GuardianRiskLevel::High),
                user_authorization: Some(codex_protocol::protocol::GuardianUserAuthorization::Low),
                rationale: Some("Would delete user data.".into()),
                decision_source: Some(
                    codex_protocol::protocol::GuardianAssessmentDecisionSource::Agent,
                ),
                action: serde_json::from_value(serde_json::json!({
                    "type": "command",
                    "source": "shell",
                    "command": "rm -rf /tmp/guardian",
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "guardian-exec".into(),
                command: "rm -rf /tmp/guardian".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Declined,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "rm -rf /tmp/guardian".into(),
                }],
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            }
        );
    }

    #[test]
    fn reconstructs_in_progress_guardian_execve_item() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run a subcommand".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-execve".into(),
                target_item_id: Some("guardian-execve".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 2_000,
                completed_at_ms: None,
                status: GuardianAssessmentStatus::InProgress,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: None,
                action: serde_json::from_value(serde_json::json!({
                    "type": "execve",
                    "source": "shell",
                    "program": "/bin/rm",
                    "argv": ["/usr/bin/rm", "-f", "/tmp/file.sqlite"],
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "guardian-execve".into(),
                command: "/bin/rm -f /tmp/file.sqlite".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::InProgress,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "/bin/rm -f /tmp/file.sqlite".into(),
                }],
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            }
        );
    }

    #[test]
    fn assigns_late_exec_completion_to_original_turn() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "first".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-b".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "second".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-late".into(),
                process_id: Some("pid-42".into()),
                turn_id: "turn-a".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "done".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo done".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: "done\n".into(),
                stderr: String::new(),
                aggregated_output: "done\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(5),
                formatted_output: "done\n".into(),
                status: CoreExecCommandStatus::Completed,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-b".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[1].id, "turn-b");
        assert_eq!(turns[0].items.len(), 3);
        assert_eq!(turns[1].items.len(), 1);
        assert_eq!(
            turns[0].items[2],
            ThreadItem::CommandExecutionNotification {
                id: "exec-late:notification:exit".into(),
                command_item_id: "exec-late".into(),
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".into(),
                output: None,
                exit_code: Some(0),
                created_at_ms: 0,
            }
        );
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "exec-late".into(),
                command: "echo done".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-42".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "echo done".into(),
                }],
                aggregated_output: Some("done\n".into()),
                exit_code: Some(0),
                duration_ms: Some(5),
            }
        );
    }

    #[test]
    fn drops_late_turn_scoped_item_for_unknown_turn_id() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "first".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-b".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "second".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-unknown-turn".into(),
                process_id: Some("pid-42".into()),
                turn_id: "turn-missing".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "done".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo done".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: "done\n".into(),
                stderr: String::new(),
                aggregated_output: "done\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(5),
                formatted_output: "done\n".into(),
                status: CoreExecCommandStatus::Completed,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-b".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[1].id, "turn-b");
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(turns[1].items.len(), 1);
        assert_eq!(
            turns[1].items[0],
            ThreadItem::UserMessage {
                id: "item-2".into(),
                content: vec![UserInput::Text {
                    text: "second".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
    }

    #[test]
    fn patch_apply_begin_updates_active_turn_snapshot_with_file_change() {
        let turn_id = "turn-1";
        let mut builder = ThreadHistoryBuilder::new();
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "apply patch".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
                call_id: "patch-call".into(),
                turn_id: turn_id.to_string(),
                auto_approved: false,
                changes: [(
                    PathBuf::from("README.md"),
                    codex_protocol::protocol::FileChange::Add {
                        content: "hello\n".into(),
                    },
                )]
                .into_iter()
                .collect(),
            }),
        ];

        for event in &events {
            builder.handle_event(event);
        }

        let snapshot = builder
            .active_turn_snapshot()
            .expect("active turn snapshot");
        assert_eq!(snapshot.id, turn_id);
        assert_eq!(snapshot.status, TurnStatus::InProgress);
        assert_eq!(
            snapshot.items,
            vec![
                ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "apply patch".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::FileChange {
                    id: "patch-call".into(),
                    changes: vec![FileUpdateChange {
                        path: "README.md".into(),
                        kind: PatchChangeKind::Add,
                        diff: "hello\n".into(),
                    }],
                    status: PatchApplyStatus::InProgress,
                },
            ]
        );
    }

    #[test]
    fn apply_patch_approval_request_updates_active_turn_snapshot_with_file_change() {
        let turn_id = "turn-1";
        let mut builder = ThreadHistoryBuilder::new();
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "apply patch".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
                call_id: "patch-call".into(),
                turn_id: turn_id.to_string(),
                started_at_ms: 0,
                changes: [(
                    PathBuf::from("README.md"),
                    codex_protocol::protocol::FileChange::Add {
                        content: "hello\n".into(),
                    },
                )]
                .into_iter()
                .collect(),
                reason: None,
                grant_root: None,
            }),
        ];

        for event in &events {
            builder.handle_event(event);
        }

        let snapshot = builder
            .active_turn_snapshot()
            .expect("active turn snapshot");
        assert_eq!(snapshot.id, turn_id);
        assert_eq!(snapshot.status, TurnStatus::InProgress);
        assert_eq!(
            snapshot.items,
            vec![
                ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "apply patch".into(),
                        text_elements: Vec::new(),
                    }],
                },
                ThreadItem::FileChange {
                    id: "patch-call".into(),
                    changes: vec![FileUpdateChange {
                        path: "README.md".into(),
                        kind: PatchChangeKind::Add,
                        diff: "hello\n".into(),
                    }],
                    status: PatchApplyStatus::InProgress,
                },
            ]
        );
    }

    #[test]
    fn late_turn_complete_does_not_close_active_turn() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "first".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-b".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "second".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "still in b".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-b".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[1].id, "turn-b");
        assert_eq!(turns[1].items.len(), 2);
    }

    #[test]
    fn late_turn_aborted_does_not_interrupt_active_turn() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "first".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-b".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "second".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id: Some("turn-a".into()),
                reason: TurnAbortReason::Replaced,
                completed_at: None,
                duration_ms: None,
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "still in b".into(),
                phase: None,
                memory_citation: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[1].id, "turn-b");
        assert_eq!(turns[1].status, TurnStatus::InProgress);
        assert_eq!(turns[1].items.len(), 2);
    }

    #[test]
    fn preserves_compaction_only_turn() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-compact".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::Compacted(CompactedItem {
                message: String::new(),
                replacement_history: None,
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-compact".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(
            turns,
            vec![Turn {
                id: "turn-compact".into(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                items_view: TurnItemsView::Full,
                items: vec![ThreadItem::ContextCompaction {
                    id: "item-1".into(),
                    replacement_history: None,
                }],
            }]
        );
    }

    #[test]
    fn preserves_compaction_replacement_history() {
        let replacement_history = vec![
            ResponseItem::Message {
                id: None,
                role: "user".into(),
                content: vec![ContentItem::InputText {
                    text: "recent request".into(),
                }],
                phase: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".into(),
                namespace: Some("functions".into()),
                arguments: "{\"cmd\":\"pwd\"}".into(),
                call_id: "call-1".into(),
            },
        ];
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-compact".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::Compacted(CompactedItem {
                message: "summary".into(),
                replacement_history: Some(replacement_history.clone()),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(
            turns[0].items,
            vec![ThreadItem::ContextCompaction {
                id: "item-1".into(),
                replacement_history: Some(serde_json::to_value(replacement_history).unwrap()),
            }]
        );
    }

    #[test]
    fn deduplicates_legacy_context_compacted_after_replacement_history() {
        let replacement_history = vec![ResponseItem::Message {
            id: None,
            role: "user".into(),
            content: vec![ContentItem::InputText {
                text: "recent request".into(),
            }],
            phase: None,
        }];
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-compact".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
            RolloutItem::Compacted(CompactedItem {
                message: "summary".into(),
                replacement_history: Some(replacement_history.clone()),
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-compact".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(
            turns,
            vec![Turn {
                id: "turn-compact".into(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                items_view: TurnItemsView::Full,
                items: vec![ThreadItem::ContextCompaction {
                    id: "item-1".into(),
                    replacement_history: Some(serde_json::to_value(replacement_history).unwrap()),
                }],
            }]
        );
    }

    #[test]
    fn reconstructs_collab_resume_end_item() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "resume agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabResumeEnd(codex_protocol::protocol::CollabResumeEndEvent {
                call_id: "resume-1".into(),
                completed_at_ms: 0,
                sender_thread_id: ThreadId::try_from("00000000-0000-0000-0000-000000000001")
                    .expect("valid sender thread id"),
                sender_agent_path: "/root".into(),
                receiver_thread_id: ThreadId::try_from("00000000-0000-0000-0000-000000000002")
                    .expect("valid receiver thread id"),
                receiver_agent_path: "/root/scout".into(),
                receiver_agent_nickname: None,
                receiver_agent_role: None,
                status: AgentStatus::Completed(None),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "resume-1".into(),
                tool: CollabAgentTool::ResumeAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        status: crate::protocol::v2::CollabAgentStatus::Completed,
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_collab_spawn_end_item_with_model_metadata() {
        let sender_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let spawned_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "spawn agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentSpawnEnd(codex_protocol::protocol::CollabAgentSpawnEndEvent {
                call_id: "spawn-1".into(),
                completed_at_ms: 0,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                new_thread_id: Some(spawned_thread_id),
                new_agent_path: Some("/root/scout".into()),
                new_agent_nickname: Some("Scout".into()),
                new_agent_role: Some("explorer".into()),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: codex_protocol::openai_models::ReasoningEffort::Medium,
                status: AgentStatus::Running,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "spawn-1".into(),
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("inspect the repo".into()),
                model: Some("gpt-5.4-mini".into()),
                reasoning_effort: Some(codex_protocol::openai_models::ReasoningEffort::Medium),
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        status: crate::protocol::v2::CollabAgentStatus::Running,
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_collab_spawn_begin_and_end_as_one_completed_item() {
        let sender_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let spawned_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "spawn agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentSpawnBegin(codex_protocol::protocol::CollabAgentSpawnBeginEvent {
                call_id: "spawn-1".into(),
                started_at_ms: 0,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: codex_protocol::openai_models::ReasoningEffort::Medium,
            }),
            EventMsg::CollabAgentSpawnEnd(codex_protocol::protocol::CollabAgentSpawnEndEvent {
                call_id: "spawn-1".into(),
                completed_at_ms: 1,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                new_thread_id: Some(spawned_thread_id),
                new_agent_path: Some("/root/scout".into()),
                new_agent_nickname: Some("Scout".into()),
                new_agent_role: Some("explorer".into()),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: codex_protocol::openai_models::ReasoningEffort::Medium,
                status: AgentStatus::Running,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "spawn-1".into(),
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("inspect the repo".into()),
                model: Some("gpt-5.4-mini".into()),
                reasoning_effort: Some(codex_protocol::openai_models::ReasoningEffort::Medium),
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        status: crate::protocol::v2::CollabAgentStatus::Running,
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_interrupted_send_input_as_completed_collab_call() {
        // `send_input(interrupt=true)` first stops the child's active turn, then redirects it with
        // new input. The transient interrupted status should remain visible in agent state, but the
        // collab tool call itself is still a successful redirect rather than a failed operation.
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let receiver = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "redirect".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentInteractionBegin(
                codex_protocol::protocol::CollabAgentInteractionBeginEvent {
                    call_id: "send-1".into(),
                    started_at_ms: 0,
                    sender_thread_id: sender,
                    sender_agent_path: "/root".into(),
                    receiver_thread_id: receiver,
                    receiver_agent_path: "/root/scout".into(),
                    prompt: "new task".into(),
                },
            ),
            EventMsg::CollabAgentInteractionEnd(
                codex_protocol::protocol::CollabAgentInteractionEndEvent {
                    call_id: "send-1".into(),
                    completed_at_ms: 0,
                    sender_thread_id: sender,
                    sender_agent_path: "/root".into(),
                    receiver_thread_id: receiver,
                    receiver_agent_path: "/root/scout".into(),
                    receiver_agent_nickname: None,
                    receiver_agent_role: None,
                    prompt: "new task".into(),
                    status: AgentStatus::Interrupted,
                },
            ),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "send-1".into(),
                tool: CollabAgentTool::SendInput,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec![receiver.to_string()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("new task".into()),
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    receiver.to_string(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        status: crate::protocol::v2::CollabAgentStatus::Interrupted,
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_wait_call_with_timeout_and_receiver_path() {
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let receiver = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "wait".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabWaitingBegin(codex_protocol::protocol::CollabWaitingBeginEvent {
                started_at_ms: 0,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                receiver_thread_ids: vec![receiver],
                receiver_agents: vec![codex_protocol::protocol::CollabAgentRef {
                    thread_id: receiver,
                    agent_path: Some("/root/scout".into()),
                    agent_nickname: None,
                    agent_role: None,
                }],
                timeout_ms: 30_000,
                call_id: "wait-1".into(),
            }),
            EventMsg::CollabWaitingEnd(codex_protocol::protocol::CollabWaitingEndEvent {
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                call_id: "wait-1".into(),
                completed_at_ms: 1,
                timeout_ms: 30_000,
                agent_statuses: vec![codex_protocol::protocol::CollabAgentStatusEntry {
                    thread_id: receiver,
                    agent_path: Some("/root/scout".into()),
                    agent_nickname: None,
                    agent_role: None,
                    status: AgentStatus::Completed(None),
                }],
                statuses: [(receiver, AgentStatus::Completed(None))]
                    .into_iter()
                    .collect(),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "wait-1".into(),
                tool: CollabAgentTool::Wait,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec![receiver.to_string()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: Some(30_000),
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    receiver.to_string(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        status: crate::protocol::v2::CollabAgentStatus::Completed,
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn rollback_failed_error_does_not_mark_turn_failed() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "done".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::Error(ErrorEvent {
                message: "rollback failed".into(),
                codex_error_info: Some(CodexErrorInfo::ThreadRollbackFailed),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(turns[0].error, None);
    }

    #[test]
    fn out_of_turn_error_does_not_create_or_fail_a_turn() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
            EventMsg::Error(ErrorEvent {
                message: "request-level failure".into(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0],
            Turn {
                id: "turn-a".into(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                items_view: TurnItemsView::Full,
                items: vec![ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "hello".into(),
                        text_elements: Vec::new(),
                    }],
                }],
            }
        );
    }

    #[test]
    fn error_then_turn_complete_preserves_failed_status() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::Error(ErrorEvent {
                message: "stream failure".into(),
                codex_error_info: Some(CodexErrorInfo::ResponseStreamDisconnected {
                    http_status_code: Some(502),
                }),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[0].status, TurnStatus::Failed);
        assert_eq!(
            turns[0].error,
            Some(TurnError {
                message: "stream failure".into(),
                codex_error_info: Some(
                    crate::protocol::v2::CodexErrorInfo::ResponseStreamDisconnected {
                        http_status_code: Some(502),
                    }
                ),
                additional_details: None,
            })
        );
    }

    #[test]
    fn rebuilds_hook_prompt_items_from_rollout_response_items() {
        let hook_prompt = build_hook_prompt_message(&[
            CoreHookPromptFragment::from_single_hook("Retry with tests.", "hook-run-1"),
            CoreHookPromptFragment::from_single_hook("Then summarize cleanly.", "hook-run-2"),
        ])
        .expect("hook prompt message");
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
            RolloutItem::ResponseItem(hook_prompt),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::HookPrompt {
                id: turns[0].items[1].id().to_string(),
                fragments: vec![
                    crate::protocol::v2::HookPromptFragment {
                        text: "Retry with tests.".into(),
                        hook_run_id: "hook-run-1".into(),
                    },
                    crate::protocol::v2::HookPromptFragment {
                        text: "Then summarize cleanly.".into(),
                        hook_run_id: "hook-run-2".into(),
                    },
                ],
            }
        );
    }

    #[test]
    fn ignores_plain_user_response_items_in_rollout_replay() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(codex_protocol::models::ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "user".into(),
                content: vec![codex_protocol::models::ContentItem::InputText {
                    text: "plain text".into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].items.is_empty());
    }

    #[test]
    fn rebuilds_initial_injected_context_from_rollout_response_items() {
        let items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![
                    ContentItem::InputText {
                        text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                            .into(),
                    },
                    ContentItem::InputText {
                        text: format!(
                            "{SKILLS_INSTRUCTIONS_OPEN_TAG}\n## Skills\n- skill-a\n{SKILLS_INSTRUCTIONS_CLOSE_TAG}"
                        ),
                    },
                ],
                phase: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("user-context".into()),
                role: "user".into(),
                content: vec![ContentItem::InputText {
                    text: format!(
                        "{ENVIRONMENT_CONTEXT_OPEN_TAG}\n  <cwd>/workspace</cwd>\n{ENVIRONMENT_CONTEXT_CLOSE_TAG}"
                    ),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::InjectedContext {
                id: turns[0].items[0].id().to_string(),
                title: INJECTED_CONTEXT_TITLE.to_string(),
                preview: "Permissions • Skills • Environment".to_string(),
                sections: vec![
                    InjectedContextSection {
                        label: "Permissions".to_string(),
                        text: "Sandbox: workspace-write".to_string(),
                    },
                    InjectedContextSection {
                        label: "Skills".to_string(),
                        text: "## Skills\n- skill-a".to_string(),
                    },
                    InjectedContextSection {
                        label: "Environment".to_string(),
                        text: "<cwd>/workspace</cwd>".to_string(),
                    },
                ],
            }
        );
        assert!(matches!(turns[0].items[1], ThreadItem::UserMessage { .. }));
    }

    #[test]
    fn rebuilds_initial_injected_context_after_explicit_turn_start() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: ModeKind::Default,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![ContentItem::InputText {
                    text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                        .into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert!(matches!(
            turns[0].items[0],
            ThreadItem::InjectedContext { .. }
        ));
        assert!(matches!(turns[0].items[1], ThreadItem::UserMessage { .. }));
    }
}
