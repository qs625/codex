use crate::protocol::DynamicToolCallStatus;
use crate::protocol::ThreadItem;
use crate::protocol::Turn;
use crate::protocol::event_item_projection::project_event_msg_item;
use protocol::protocol::EventMsg;
use protocol::protocol::RolloutItem;
use protocol::protocol::SessionMetaLine;
use protocol::subscriptions::PersistedSubscription;
use std::collections::{HashMap, HashSet};

mod basic_events;
mod collab;
mod lifecycle;
mod pending_turn;
mod support;
mod tool_events;
mod turn_helpers;
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
    latest_subscription_snapshot: Option<(usize, Vec<PersistedSubscription>)>,
    schedule_subscription_rollout_indexes: HashMap<String, usize>,
    schedule_unsubscription_rollout_indexes: HashMap<String, usize>,
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
            latest_subscription_snapshot: None,
            schedule_subscription_rollout_indexes: HashMap::new(),
            schedule_unsubscription_rollout_indexes: HashMap::new(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn finish(mut self) -> Vec<Turn> {
        self.finish_current_turn();
        self.append_subscription_snapshot_items();
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
            EventMsg::ResponseItemCompleted(_) if project_event_msg_item(event).is_some() => {
                self.handle_projected_event_item(event);
            }
            EventMsg::ResponseItemStarted(_) | EventMsg::ResponseItemCompleted(_) => {}
            EventMsg::CommandWaitStarted(_)
            | EventMsg::CommandWaitCompleted(_)
            | EventMsg::CommandWriteStdinCompleted(_)
            | EventMsg::CommandExecutionNotificationCompleted(_)
            | EventMsg::BuiltinToolCallStarted(_)
            | EventMsg::BuiltinToolCallCompleted(_)
            | EventMsg::ExternalToolCallStarted(_)
            | EventMsg::ExternalToolCallCompleted(_)
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
            EventMsg::ExternalTerminalStatus(payload) => {
                self.handle_external_terminal_status(payload);
            }
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
            RolloutItem::EventMsg(event) => {
                self.handle_event(event);
                self.record_schedule_subscription_event(event);
            }
            RolloutItem::Compacted(payload) => self.handle_compacted(payload),
            RolloutItem::ResponseItem(_) => {}
            RolloutItem::TurnContext(payload) => self.handle_turn_context(payload),
            RolloutItem::SessionMeta(payload) => self.handle_session_meta(payload),
        }
    }

    fn handle_session_meta(&mut self, payload: &SessionMetaLine) {
        let Some(subscriptions) = payload.meta.subscriptions.clone() else {
            return;
        };
        self.latest_subscription_snapshot = Some((self.current_rollout_index, subscriptions));
    }

    fn record_schedule_subscription_event(&mut self, event: &EventMsg) {
        let Some((tool, status, output)) = builtin_tool_event_parts(event) else {
            return;
        };
        if status != protocol::protocol::BuiltinToolCallStatus::Completed {
            return;
        }
        let Some(subscription_id) = output.as_ref().and_then(subscription_id_from_json) else {
            return;
        };

        match tool {
            "schedule_subscribe" => {
                self.schedule_subscription_rollout_indexes
                    .insert(subscription_id, self.current_rollout_index);
            }
            "schedule_unsubscribe"
                if output
                    .as_ref()
                    .and_then(|output| output.get("unsubscribed"))
                    .and_then(|value| value.as_bool())
                    == Some(true) =>
            {
                self.schedule_unsubscription_rollout_indexes
                    .insert(subscription_id, self.current_rollout_index);
            }
            _ => {}
        }
    }

    fn append_subscription_snapshot_items(&mut self) {
        let Some((snapshot_index, subscriptions)) = self.latest_subscription_snapshot.take() else {
            return;
        };
        let active_schedule_ids = active_schedule_subscription_ids(&subscriptions);
        let inactive_items = self.inactive_schedule_items(snapshot_index, &active_schedule_ids);
        let active_items = subscriptions
            .iter()
            .filter_map(|subscription| self.active_schedule_item_if_missing(subscription))
            .collect::<Vec<_>>();
        let items = inactive_items
            .into_iter()
            .chain(active_items)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }

        let mut turn = self.new_turn(Some("active-subscriptions".to_string()));
        turn.items = items;
        self.turns.push(Turn::from(turn));
    }

    fn inactive_schedule_items(
        &self,
        snapshot_index: usize,
        active_schedule_ids: &HashSet<String>,
    ) -> Vec<ThreadItem> {
        self.schedule_subscription_rollout_indexes
            .iter()
            .filter(|(subscription_id, subscribe_index)| {
                **subscribe_index < snapshot_index
                    && !active_schedule_ids.contains(*subscription_id)
                    && self
                        .schedule_unsubscription_rollout_indexes
                        .get(*subscription_id)
                        .is_none_or(|unsubscribe_index| unsubscribe_index < subscribe_index)
            })
            .map(|(subscription_id, _)| ThreadItem::BuiltinToolCall {
                id: format!("active-subscription:{subscription_id}:inactive"),
                tool: "schedule_unsubscribe".to_string(),
                arguments: serde_json::json!({
                    "subscription_id": subscription_id,
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "subscription_id": subscription_id,
                    "unsubscribed": true,
                })),
            })
            .collect()
    }

    fn active_schedule_item_if_missing(
        &self,
        subscription: &PersistedSubscription,
    ) -> Option<ThreadItem> {
        let PersistedSubscription::Schedule {
            subscription_id,
            schedule,
            label,
            message,
        } = subscription
        else {
            return None;
        };
        if self.has_schedule_monitor_item(subscription_id) {
            return None;
        }
        let mut arguments = serde_json::json!({
            "schedule": schedule,
            "label": label,
        });
        if let Some(message) = message {
            arguments["message"] = serde_json::Value::String(message.clone());
        }

        Some(ThreadItem::BuiltinToolCall {
            id: format!("active-subscription:{subscription_id}"),
            tool: "schedule_subscribe".to_string(),
            arguments,
            status: DynamicToolCallStatus::Completed,
            output: Some(serde_json::json!({
                "subscription_id": subscription_id,
            })),
        })
    }

    fn has_schedule_monitor_item(&self, subscription_id: &str) -> bool {
        self.turns
            .iter()
            .flat_map(|turn| turn.items.iter())
            .chain(self.current_turn.iter().flat_map(|turn| turn.items.iter()))
            .any(|item| schedule_subscription_id(item).as_deref() == Some(subscription_id))
    }
}

fn schedule_subscription_id(item: &ThreadItem) -> Option<String> {
    let ThreadItem::BuiltinToolCall {
        tool,
        status,
        output,
        ..
    } = item
    else {
        return None;
    };
    if tool != "schedule_subscribe" || *status != DynamicToolCallStatus::Completed {
        return None;
    }
    output.as_ref().and_then(subscription_id_from_json)
}

fn builtin_tool_event_parts(
    event: &EventMsg,
) -> Option<(
    &str,
    protocol::protocol::BuiltinToolCallStatus,
    &Option<serde_json::Value>,
)> {
    match event {
        EventMsg::BuiltinToolCallStarted(event) => Some((&event.tool, event.status, &event.output)),
        EventMsg::BuiltinToolCallCompleted(event) => {
            Some((&event.tool, event.status, &event.output))
        }
        _ => None,
    }
}

fn subscription_id_from_json(output: &serde_json::Value) -> Option<String> {
    output
        .get("subscription_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
}

fn active_schedule_subscription_ids(subscriptions: &[PersistedSubscription]) -> HashSet<String> {
    subscriptions
        .iter()
        .filter_map(|subscription| match subscription {
            PersistedSubscription::Schedule {
                subscription_id, ..
            } => Some(subscription_id.clone()),
            _ => None,
        })
        .collect()
}
