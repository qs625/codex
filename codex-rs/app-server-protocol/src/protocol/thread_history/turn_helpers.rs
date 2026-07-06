use super::PendingTurn;
use super::ThreadHistoryBuilder;
use super::upsert_turn_item;
use crate::protocol::ThreadItem;
use crate::protocol::Turn;
use crate::protocol::TurnStatus;
use crate::protocol::UserInput;
use protocol::protocol::UserMessageEvent;
use tracing::warn;
use uuid::Uuid;

impl ThreadHistoryBuilder {
    pub(super) fn finish_current_turn(&mut self) {
        self.pending_agent_message_responses.clear();
        if let Some(turn) = self.current_turn.take() {
            if turn.items.is_empty() && !turn.opened_explicitly && !turn.saw_compaction {
                return;
            }
            self.turns.push(Turn::from(turn));
        }
    }

    pub(super) fn new_turn(&mut self, id: Option<String>) -> PendingTurn {
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

    pub(super) fn ensure_turn(&mut self) -> &mut PendingTurn {
        if self.current_turn.is_none() {
            let turn = self.new_turn(/*id*/ None);
            return self.current_turn.insert(turn);
        }

        if let Some(turn) = self.current_turn.as_mut() {
            return turn;
        }

        unreachable!("current turn must exist after initialization");
    }

    pub(super) fn upsert_item_in_turn_id(&mut self, turn_id: &str, item: ThreadItem) {
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

    pub(super) fn upsert_item_in_turn_id_or_create(&mut self, turn_id: &str, item: ThreadItem) {
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

    pub(super) fn upsert_item_in_current_turn(&mut self, item: ThreadItem) {
        let turn = self.ensure_turn();
        upsert_turn_item(&mut turn.items, item);
    }

    pub(super) fn next_item_id(&mut self) -> String {
        let id = format!("item-{}", self.next_item_index);
        self.next_item_index += 1;
        id
    }

    pub(super) fn build_user_inputs(&self, payload: &UserMessageEvent) -> Vec<UserInput> {
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
