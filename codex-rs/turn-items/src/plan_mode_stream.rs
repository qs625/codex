use std::collections::HashMap;
use std::collections::HashSet;

use protocol::items::AgentMessageItem;
use protocol::items::PlanItem;
use protocol::items::TurnItem;
use protocol::models::ResponseItem;

use crate::assistant_stream::ProposedPlanSegment;
use crate::assistant_stream::agent_message_text;
use crate::assistant_stream::proposed_plan_text_from_assistant_response_item;

/// Display actions emitted by the plan-mode stream state machine.
pub enum PlanModeStreamAction {
    TurnItemStarted(TurnItem),
    TurnItemCompleted(TurnItem),
    AgentMessageDelta { item_id: String, delta: String },
    PlanDelta { item_id: String, delta: String },
}

/// Aggregated state used only while streaming a plan-mode response.
///
/// The state owns plan-mode display decisions. Callers remain responsible for
/// sending the returned actions through their runtime/event channel.
pub struct PlanModeStreamState {
    pending_agent_message_items: HashMap<String, TurnItem>,
    started_agent_message_items: HashSet<String>,
    leading_whitespace_by_item: HashMap<String, String>,
    plan_item_state: ProposedPlanItemState,
}

struct ProposedPlanItemState {
    item_id: String,
    started: bool,
    completed: bool,
}

impl PlanModeStreamState {
    pub fn new(turn_id: &str) -> Self {
        Self {
            pending_agent_message_items: HashMap::new(),
            started_agent_message_items: HashSet::new(),
            leading_whitespace_by_item: HashMap::new(),
            plan_item_state: ProposedPlanItemState::new(turn_id),
        }
    }

    pub fn stage_agent_message_item(&mut self, item_id: String, item: TurnItem) {
        self.pending_agent_message_items.insert(item_id, item);
    }

    pub fn handle_segments(
        &mut self,
        item_id: &str,
        segments: Vec<ProposedPlanSegment>,
    ) -> Vec<PlanModeStreamAction> {
        let mut actions = Vec::new();
        for segment in segments {
            match segment {
                ProposedPlanSegment::Normal(delta) => {
                    self.handle_normal_text_segment(item_id, delta, &mut actions);
                }
                ProposedPlanSegment::ProposedPlanStart => {
                    self.plan_item_state.start(&mut actions);
                }
                ProposedPlanSegment::ProposedPlanDelta(delta) => {
                    self.plan_item_state.push_delta(delta, &mut actions);
                }
                ProposedPlanSegment::ProposedPlanEnd => {}
            }
        }
        actions
    }

    pub fn complete_plan_from_message(&mut self, item: &ResponseItem) -> Vec<PlanModeStreamAction> {
        let Some(plan_text) = proposed_plan_text_from_assistant_response_item(item) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        self.plan_item_state
            .complete_with_text(plan_text, &mut actions);
        actions
    }

    pub fn complete_turn_item(
        &mut self,
        turn_item: TurnItem,
        previously_active_item: Option<&TurnItem>,
    ) -> Vec<PlanModeStreamAction> {
        match turn_item {
            TurnItem::AgentMessage(agent_message) => self.complete_agent_message(agent_message),
            _ => {
                let mut actions = Vec::new();
                if previously_active_item.is_none() {
                    actions.push(PlanModeStreamAction::TurnItemStarted(turn_item.clone()));
                }
                actions.push(PlanModeStreamAction::TurnItemCompleted(turn_item));
                actions
            }
        }
    }

    fn handle_normal_text_segment(
        &mut self,
        item_id: &str,
        delta: String,
        actions: &mut Vec<PlanModeStreamAction>,
    ) {
        if delta.is_empty() {
            return;
        }
        let has_non_whitespace = delta.chars().any(|ch| !ch.is_whitespace());
        if !has_non_whitespace && !self.started_agent_message_items.contains(item_id) {
            let entry = self
                .leading_whitespace_by_item
                .entry(item_id.to_string())
                .or_default();
            entry.push_str(&delta);
            return;
        }

        let delta = if !self.started_agent_message_items.contains(item_id) {
            if let Some(prefix) = self.leading_whitespace_by_item.remove(item_id) {
                format!("{prefix}{delta}")
            } else {
                delta
            }
        } else {
            delta
        };
        self.maybe_emit_pending_agent_message_start(item_id, actions);
        actions.push(PlanModeStreamAction::AgentMessageDelta {
            item_id: item_id.to_string(),
            delta,
        });
    }

    fn maybe_emit_pending_agent_message_start(
        &mut self,
        item_id: &str,
        actions: &mut Vec<PlanModeStreamAction>,
    ) {
        if self.started_agent_message_items.contains(item_id) {
            return;
        }
        if let Some(item) = self.pending_agent_message_items.remove(item_id) {
            actions.push(PlanModeStreamAction::TurnItemStarted(item));
            self.started_agent_message_items.insert(item_id.to_string());
        }
    }

    fn complete_agent_message(
        &mut self,
        agent_message: AgentMessageItem,
    ) -> Vec<PlanModeStreamAction> {
        let mut actions = Vec::new();
        let agent_message_id = agent_message.id.clone();
        let text = agent_message_text(&agent_message);
        if text.trim().is_empty() {
            self.pending_agent_message_items.remove(&agent_message_id);
            self.started_agent_message_items.remove(&agent_message_id);
            return actions;
        }

        self.maybe_emit_pending_agent_message_start(&agent_message_id, &mut actions);

        if !self.started_agent_message_items.contains(&agent_message_id) {
            let start_item = self
                .pending_agent_message_items
                .remove(&agent_message_id)
                .unwrap_or_else(|| {
                    TurnItem::AgentMessage(AgentMessageItem {
                        id: agent_message_id.clone(),
                        content: Vec::new(),
                        phase: None,
                        memory_citation: None,
                    })
                });
            actions.push(PlanModeStreamAction::TurnItemStarted(start_item));
            self.started_agent_message_items
                .insert(agent_message_id.clone());
        }

        actions.push(PlanModeStreamAction::TurnItemCompleted(
            TurnItem::AgentMessage(agent_message),
        ));
        self.started_agent_message_items.remove(&agent_message_id);
        actions
    }
}

impl ProposedPlanItemState {
    fn new(turn_id: &str) -> Self {
        Self {
            item_id: format!("{turn_id}-plan"),
            started: false,
            completed: false,
        }
    }

    fn start(&mut self, actions: &mut Vec<PlanModeStreamAction>) {
        if self.started || self.completed {
            return;
        }
        self.started = true;
        actions.push(PlanModeStreamAction::TurnItemStarted(TurnItem::Plan(
            PlanItem {
                id: self.item_id.clone(),
                text: String::new(),
            },
        )));
    }

    fn push_delta(&mut self, delta: String, actions: &mut Vec<PlanModeStreamAction>) {
        if self.completed || delta.is_empty() {
            return;
        }
        if !self.started {
            self.start(actions);
        }
        actions.push(PlanModeStreamAction::PlanDelta {
            item_id: self.item_id.clone(),
            delta,
        });
    }

    fn complete_with_text(&mut self, text: String, actions: &mut Vec<PlanModeStreamAction>) {
        if self.completed {
            return;
        }
        if !self.started {
            self.start(actions);
        }
        self.completed = true;
        actions.push(PlanModeStreamAction::TurnItemCompleted(TurnItem::Plan(
            PlanItem {
                id: self.item_id.clone(),
                text,
            },
        )));
    }
}
