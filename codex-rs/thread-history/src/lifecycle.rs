use super::PendingTurn;
use super::ThreadHistoryBuilder;
use super::support::REVIEW_FALLBACK_MESSAGE;
use super::support::render_review_output_text;
use app_server_protocol::ThreadItem;
use app_server_protocol::TurnError as V2TurnError;
use app_server_protocol::TurnStatus;
use app_server_protocol::context_compaction_replacement_item_from_core;
use protocol::items::context_compaction_replacement_items_from_response_items;
use protocol::protocol::CompactedItem;
use protocol::protocol::ContextCompactedEvent;
use protocol::protocol::ErrorEvent;
use protocol::protocol::ExternalTerminalStatus;
use protocol::protocol::ExternalTerminalStatusEvent;
use protocol::protocol::ThreadRolledBackEvent;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnContextItem;
use protocol::protocol::TurnStartedEvent;

impl ThreadHistoryBuilder {
    pub(super) fn handle_context_compacted(&mut self, _payload: &ContextCompactedEvent) {
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
                replacement_history: Vec::new(),
            });
    }

    pub(super) fn handle_entered_review_mode(
        &mut self,
        payload: &protocol::protocol::ReviewRequest,
    ) {
        let review = payload
            .user_facing_hint
            .clone()
            .unwrap_or_else(|| "Review requested.".to_string());
        let id = self.next_item_id();
        self.ensure_turn()
            .items
            .push(ThreadItem::EnteredReviewMode { id, review });
    }

    pub(super) fn handle_exited_review_mode(
        &mut self,
        payload: &protocol::protocol::ExitedReviewModeEvent,
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

    pub(super) fn handle_error(&mut self, payload: &ErrorEvent) {
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

    pub(super) fn handle_external_terminal_status(
        &mut self,
        payload: &ExternalTerminalStatusEvent,
    ) {
        let Some(turn) = self
            .current_turn
            .as_mut()
            .filter(|turn| turn.id == payload.turn_id)
        else {
            return;
        };
        turn.completed_at = Some(payload.terminal_at_ms / 1000);
        match payload.status {
            ExternalTerminalStatus::Errored => {
                turn.status = TurnStatus::Failed;
                turn.error = Some(V2TurnError {
                    message: payload.message.clone().unwrap_or_default(),
                    codex_error_info: None,
                    additional_details: None,
                });
            }
            ExternalTerminalStatus::Shutdown => {
                if matches!(turn.status, TurnStatus::Completed | TurnStatus::InProgress) {
                    turn.status = TurnStatus::Completed;
                }
            }
        }
        self.finish_current_turn();
    }

    pub(super) fn handle_turn_aborted(&mut self, payload: &TurnAbortedEvent) {
        let apply_abort = |turn: &mut PendingTurn| {
            turn.status = TurnStatus::Interrupted;
            turn.completed_at = payload.completed_at;
            turn.duration_ms = payload.duration_ms;
        };
        if let Some(turn_id) = payload.turn_id.as_deref() {
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

        if let Some(turn) = self.current_turn.as_mut() {
            apply_abort(turn);
        }
    }

    pub(super) fn handle_turn_started(&mut self, payload: &TurnStartedEvent) {
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

    pub(super) fn handle_turn_context(&mut self, payload: &TurnContextItem) {
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

    pub(super) fn handle_turn_complete(&mut self, payload: &TurnCompleteEvent) {
        let mark_completed = |turn: &mut PendingTurn| {
            if matches!(turn.status, TurnStatus::Completed | TurnStatus::InProgress) {
                turn.status = TurnStatus::Completed;
            }
            turn.completed_at = payload.completed_at;
            turn.duration_ms = payload.duration_ms;
        };

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

        if let Some(current_turn) = self.current_turn.as_mut() {
            mark_completed(current_turn);
            self.finish_current_turn();
        }
    }

    pub(super) fn handle_compacted(&mut self, payload: &CompactedItem) {
        self.latest_subscription_snapshot = None;
        self.schedule_subscription_rollout_indexes.clear();
        self.schedule_unsubscription_rollout_indexes.clear();
        let replacement_history = payload
            .replacement_history
            .as_ref()
            .map(|history| {
                context_compaction_replacement_items_from_response_items(history.clone())
                    .into_iter()
                    .map(context_compaction_replacement_item_from_core)
                    .collect()
            })
            .unwrap_or_default();
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

    pub(super) fn handle_thread_rollback(&mut self, payload: &ThreadRolledBackEvent) {
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
}
