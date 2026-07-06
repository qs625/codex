use crate::protocol::Turn;
use protocol::protocol::EventMsg;
use protocol::protocol::RolloutItem;

mod pending_turn;
mod basic_events;
mod turn_helpers;
mod collab;
mod lifecycle;
mod tool_events;
mod support;
use pending_turn::PendingTurn;
use pending_turn::upsert_turn_item;
use support::PendingAgentMessageResponse;

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
            EventMsg::CollabListAgentsBegin(payload) => {
                self.handle_collab_list_agents_begin(payload)
            }
            EventMsg::CollabListAgentsEnd(payload) => self.handle_collab_list_agents_end(payload),
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
            EventMsg::ResponseItemStarted(_) | EventMsg::ResponseItemCompleted(_) => {}
            EventMsg::CommandWaitStarted(_)
            | EventMsg::CommandWaitCompleted(_)
            | EventMsg::CommandWriteStdinCompleted(_)
            | EventMsg::CommandExecutionNotificationCompleted(_)
            | EventMsg::WorkflowRunProgressCompleted(_)
            | EventMsg::EventCommandEventCompleted(_)
            | EventMsg::EventDrivenToolCompleted(_)
            | EventMsg::InterAgentCommunicationCompleted(_)
            | EventMsg::ThreadGoalUpdateCompleted(_) => {
                self.handle_projected_event_item(event);
            }
            EventMsg::RawResponseItem(_) => {}
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
            RolloutItem::ResponseItem(_) => {}
            RolloutItem::TurnContext(payload) => self.handle_turn_context(payload),
            RolloutItem::SessionMeta(_) => {}
        }
    }

}
