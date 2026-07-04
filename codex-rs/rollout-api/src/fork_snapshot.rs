//! Fork snapshot transforms for persisted rollout histories.

use std::collections::HashSet;

use codex_context_manager::ContextualUserFragment;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;
use protocol::protocol::InitialHistory;
use protocol::protocol::RolloutItem;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;

use crate::truncation;

// TODO(ccunningham): Add an explicit non-interrupting live-turn snapshot once
// core can represent sampling boundaries directly instead of relying on
// whichever items happened to be persisted mid-turn.
//
// Two likely future variants:
// - `TruncateToLastSamplingBoundary` for callers that want a coherent fork from
//   the last stable model boundary without synthesizing an interrupt.
// - `WaitUntilNextSamplingBoundary` (or similar) for callers that prefer to
//   fork after the next sampling boundary rather than interrupting immediately.
/// Represents how a fork should sample the source rollout history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSnapshot {
    /// Fork a committed prefix ending strictly before the nth user message.
    ///
    /// When `n` is within range, this cuts before that 0-based user-message
    /// boundary. When `n` is out of range and the source thread is currently
    /// mid-turn, this instead cuts before the active turn's opening boundary
    /// so the fork drops the unfinished turn suffix. When `n` is out of range
    /// and the source thread is already at a turn boundary, this returns the
    /// full committed history unchanged.
    TruncateBeforeNthUserMessage(usize),

    /// Fork the current persisted history as if the source thread had been
    /// interrupted now.
    ///
    /// If the persisted snapshot ends mid-turn, this appends the same
    /// `<turn_aborted>` marker produced by a real interrupt. If the snapshot is
    /// already at a turn boundary, this returns the current persisted history
    /// unchanged.
    Interrupted,
}

/// Preserve legacy `fork_thread(usize, ...)` callsites by mapping them to the
/// existing truncate-before-nth-user-message snapshot mode.
impl From<usize> for ForkSnapshot {
    fn from(value: usize) -> Self {
        Self::TruncateBeforeNthUserMessage(value)
    }
}

/// Controls whether interrupted fork snapshots include a model-visible marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptedTurnHistoryMarker {
    Disabled,
    ContextualUser,
    Developer,
}

/// Build the model-visible marker used by both the real interrupt path and
/// interrupted fork snapshots.
pub fn interrupted_turn_history_marker(
    marker: InterruptedTurnHistoryMarker,
) -> Option<ResponseItem> {
    match marker {
        InterruptedTurnHistoryMarker::Disabled => None,
        InterruptedTurnHistoryMarker::ContextualUser => Some(ContextualUserFragment::into(
            TurnAborted::new(TurnAborted::INTERRUPTED_GUIDANCE),
        )),
        InterruptedTurnHistoryMarker::Developer => {
            let marker = TurnAborted::new(TurnAborted::INTERRUPTED_DEVELOPER_GUIDANCE);
            Some(ResponseItem::Message {
                id: None,
                role: "developer".to_string(),
                content: vec![ContentItem::InputText {
                    text: marker.render(),
                }],
                phase: None,
            })
        }
    }
}

/// Model-visible interrupted turn fragment.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnAborted {
    guidance: String,
}

impl TurnAborted {
    pub const INTERRUPTED_GUIDANCE: &'static str = "The user interrupted the previous turn on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed.";
    pub const INTERRUPTED_DEVELOPER_GUIDANCE: &'static str = "The previous turn was interrupted on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed.";

    pub fn new(guidance: impl Into<String>) -> Self {
        Self {
            guidance: guidance.into(),
        }
    }
}

impl ContextualUserFragment for TurnAborted {
    const ROLE: &'static str = "user";
    const START_MARKER: &'static str = "<turn_aborted>";
    const END_MARKER: &'static str = "</turn_aborted>";

    fn body(&self) -> String {
        format!("\n{}\n", self.guidance)
    }
}

/// Snapshot state derived from a rollout history before fork transformation.
#[derive(Debug, Eq, PartialEq)]
pub struct SnapshotTurnState {
    pub ends_mid_turn: bool,
    pub active_turn_id: Option<String>,
    pub active_turn_start_index: Option<usize>,
}

/// Return a fork snapshot cut strictly before the nth user message (0-based).
///
/// Out-of-range values keep the full committed history at a turn boundary, but
/// when the source thread is currently mid-turn they fall back to cutting
/// before the active turn's opening boundary so the fork omits the unfinished
/// suffix entirely.
pub fn truncate_before_nth_user_message(
    history: InitialHistory,
    n: usize,
    snapshot_state: &SnapshotTurnState,
) -> InitialHistory {
    let items: Vec<RolloutItem> = history.get_rollout_items();
    let user_positions = truncation::user_message_positions_in_rollout(&items);
    let rolled = if snapshot_state.ends_mid_turn && n >= user_positions.len() {
        if let Some(cut_idx) = snapshot_state
            .active_turn_start_index
            .or_else(|| user_positions.last().copied())
        {
            items[..cut_idx].to_vec()
        } else {
            items
        }
    } else {
        truncation::truncate_rollout_before_nth_user_message_from_start(&items, n)
    };

    if rolled.is_empty() {
        InitialHistory::New
    } else {
        InitialHistory::Forked(rolled)
    }
}

/// Derive whether a rollout history ends inside an unfinished turn.
pub fn snapshot_turn_state(history: &InitialHistory) -> SnapshotTurnState {
    let rollout_items = history.get_rollout_items();

    let mut finished_explicit_turn_ids = HashSet::new();
    let mut active_explicit_turn: Option<(String, usize)> = None;
    for (index, item) in rollout_items.iter().enumerate() {
        match item {
            RolloutItem::EventMsg(EventMsg::TurnStarted(event)) => {
                if let Some((turn_id, _)) = active_explicit_turn.take() {
                    finished_explicit_turn_ids.insert(turn_id);
                }
                active_explicit_turn = Some((event.turn_id.clone(), index));
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                if active_explicit_turn
                    .as_ref()
                    .is_some_and(|(turn_id, _)| turn_id == &event.turn_id)
                {
                    if let Some((turn_id, _)) = active_explicit_turn.take() {
                        finished_explicit_turn_ids.insert(turn_id);
                    }
                } else if !finished_explicit_turn_ids.contains(&event.turn_id)
                    && let Some((turn_id, _)) = active_explicit_turn.take()
                {
                    finished_explicit_turn_ids.insert(turn_id);
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnAborted(event)) => match event.turn_id.as_deref() {
                Some(aborted_turn_id) => {
                    if active_explicit_turn
                        .as_ref()
                        .is_some_and(|(turn_id, _)| turn_id == aborted_turn_id)
                    {
                        if let Some((turn_id, _)) = active_explicit_turn.take() {
                            finished_explicit_turn_ids.insert(turn_id);
                        }
                    } else if !finished_explicit_turn_ids.contains(aborted_turn_id)
                        && let Some((turn_id, _)) = active_explicit_turn.take()
                    {
                        finished_explicit_turn_ids.insert(turn_id);
                    }
                }
                None => {
                    if let Some((turn_id, _)) = active_explicit_turn.take() {
                        finished_explicit_turn_ids.insert(turn_id);
                    }
                }
            },
            _ => {}
        }
    }

    if let Some((turn_id, start_index)) = active_explicit_turn {
        return SnapshotTurnState {
            ends_mid_turn: true,
            active_turn_id: Some(turn_id),
            active_turn_start_index: Some(start_index),
        };
    }

    let Some(last_user_position) = truncation::user_message_positions_in_rollout(&rollout_items)
        .last()
        .copied()
    else {
        return SnapshotTurnState {
            ends_mid_turn: false,
            active_turn_id: None,
            active_turn_start_index: None,
        };
    };

    // Synthetic fork/resume histories can contain user/assistant response items
    // without explicit turn lifecycle events. If the persisted snapshot has no
    // terminating boundary after its last user message, treat it as mid-turn.
    SnapshotTurnState {
        ends_mid_turn: !rollout_items[last_user_position + 1..].iter().any(|item| {
            matches!(
                item,
                RolloutItem::EventMsg(EventMsg::TurnComplete(_) | EventMsg::TurnAborted(_))
            )
        }),
        active_turn_id: None,
        active_turn_start_index: None,
    }
}

/// Apply a fork snapshot mode to an initial history.
pub fn fork_history_from_snapshot(
    snapshot: ForkSnapshot,
    history: InitialHistory,
    interrupted_marker: InterruptedTurnHistoryMarker,
) -> InitialHistory {
    let snapshot_state = snapshot_turn_state(&history);
    match snapshot {
        ForkSnapshot::TruncateBeforeNthUserMessage(nth_user_message) => {
            truncate_before_nth_user_message(history, nth_user_message, &snapshot_state)
        }
        ForkSnapshot::Interrupted => {
            let history = match history {
                InitialHistory::New => InitialHistory::New,
                InitialHistory::Cleared => InitialHistory::Cleared,
                InitialHistory::Forked(history) => InitialHistory::Forked(history),
                InitialHistory::Resumed(resumed) => InitialHistory::Forked(resumed.history),
            };
            if snapshot_state.ends_mid_turn {
                append_interrupted_boundary(
                    history,
                    snapshot_state.active_turn_id,
                    interrupted_marker,
                )
            } else {
                history
            }
        }
    }
}

/// Append the same persisted interrupt boundary used by the live interrupt path
/// to an existing fork snapshot after the source thread has been confirmed to
/// be mid-turn.
pub fn append_interrupted_boundary(
    history: InitialHistory,
    turn_id: Option<String>,
    interrupted_marker: InterruptedTurnHistoryMarker,
) -> InitialHistory {
    let aborted_event = RolloutItem::EventMsg(EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id,
        reason: TurnAbortReason::Interrupted,
        completed_at: None,
        duration_ms: None,
    }));

    match history {
        InitialHistory::New | InitialHistory::Cleared => {
            let mut history = Vec::new();
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                history.push(RolloutItem::ResponseItem(marker));
            }
            history.push(aborted_event);
            InitialHistory::Forked(history)
        }
        InitialHistory::Forked(mut history) => {
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                history.push(RolloutItem::ResponseItem(marker));
            }
            history.push(aborted_event);
            InitialHistory::Forked(history)
        }
        InitialHistory::Resumed(mut resumed) => {
            if let Some(marker) = interrupted_turn_history_marker(interrupted_marker) {
                resumed.history.push(RolloutItem::ResponseItem(marker));
            }
            resumed.history.push(aborted_event);
            InitialHistory::Forked(resumed.history)
        }
    }
}

#[cfg(test)]
#[path = "fork_snapshot_tests.rs"]
mod tests;
