//! Rollout replay and model-visible history reconstruction.

use codex_context_manager::ContextManager;
use codex_context_manager::is_user_turn_boundary;
use codex_utils_output_truncation::TruncationPolicy;
use protocol::models::ResponseItem;
use protocol::protocol::CompactedItem;
use protocol::protocol::EventMsg;
use protocol::protocol::RolloutItem;
use protocol::protocol::TurnContextItem;

/// Notes from the previous real user turn.
///
/// Conceptually this is the same role that `previous_model` used to fill, but
/// it can carry other prior-turn settings that matter when constructing
/// sensible state-change diffs or full-context reinjection, such as model
/// switches or detecting a prior `realtime_active -> false` transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviousTurnSettings {
    pub model: String,
    pub realtime_active: Option<bool>,
}

/// Options for replaying persisted rollout items into model-visible history.
#[derive(Clone, Copy, Debug)]
pub struct RolloutReconstructionOptions<'a> {
    pub truncation_policy: TruncationPolicy,
    pub summary_prefix: Option<&'a str>,
}

/// Rebuilt model-visible history plus resume/fork hydration metadata derived
/// from the same rollout replay.
#[derive(Debug)]
pub struct RolloutReconstruction {
    pub history: Vec<ResponseItem>,
    pub previous_turn_settings: Option<PreviousTurnSettings>,
    pub reference_context_item: Option<TurnContextItem>,
}

#[derive(Debug, Default)]
enum TurnReferenceContextItem {
    /// No `TurnContextItem` has been seen for this replay span yet.
    ///
    /// This differs from `Cleared`: `NeverSet` means there is no evidence this
    /// turn ever established a baseline, while `Cleared` means a baseline
    /// existed and a later compaction invalidated it. Only the latter must emit
    /// an explicit clearing segment for resume/fork hydration.
    #[default]
    NeverSet,
    /// A previously established baseline was invalidated by later compaction.
    Cleared,
    /// The latest baseline established by this replay span.
    Latest(Box<TurnContextItem>),
}

#[derive(Debug, Default)]
struct ActiveReplaySegment {
    turn_id: Option<String>,
    counts_as_user_turn: bool,
    previous_turn_settings: Option<PreviousTurnSettings>,
    reference_context_item: TurnReferenceContextItem,
    base_compaction_summary: Option<Option<ResponseItem>>,
}

fn turn_ids_are_compatible(active_turn_id: Option<&str>, item_turn_id: Option<&str>) -> bool {
    active_turn_id
        .is_none_or(|turn_id| item_turn_id.is_none_or(|item_turn_id| item_turn_id == turn_id))
}

fn finalize_active_segment(
    active_segment: ActiveReplaySegment,
    base_compaction_summary: &mut Option<Option<ResponseItem>>,
    previous_turn_settings: &mut Option<PreviousTurnSettings>,
    reference_context_item: &mut TurnReferenceContextItem,
    pending_rollback_turns: &mut usize,
) {
    // Thread rollback drops the newest surviving real user-message boundaries.
    // In reverse replay, that means skipping the next finalized segments that
    // contain a non-contextual `EventMsg::UserMessage`.
    if *pending_rollback_turns > 0 {
        if active_segment.counts_as_user_turn {
            *pending_rollback_turns -= 1;
        }
        return;
    }

    // A surviving compaction checkpoint starts a new chained context segment.
    // Once we know the newest surviving checkpoint, older rollout items do not
    // affect rebuilt model-visible history.
    if base_compaction_summary.is_none()
        && let Some(segment_base_compaction_summary) = active_segment.base_compaction_summary
    {
        *base_compaction_summary = Some(segment_base_compaction_summary);
    }

    // `previous_turn_settings` come from the newest surviving user turn that
    // established them.
    if previous_turn_settings.is_none() && active_segment.counts_as_user_turn {
        *previous_turn_settings = active_segment.previous_turn_settings;
    }

    // `reference_context_item` comes from the newest surviving user turn
    // baseline, or from a surviving compaction that explicitly cleared that
    // baseline.
    if matches!(reference_context_item, TurnReferenceContextItem::NeverSet)
        && (active_segment.counts_as_user_turn
            || matches!(
                active_segment.reference_context_item,
                TurnReferenceContextItem::Cleared
            ))
    {
        *reference_context_item = active_segment.reference_context_item;
    }
}

/// Reconstruct model-visible history and resume metadata from persisted rollout
/// items.
pub fn reconstruct_history_from_rollout(
    rollout_items: &[RolloutItem],
    options: RolloutReconstructionOptions<'_>,
) -> RolloutReconstruction {
    // Replay metadata should already match the shape of the future lazy reverse
    // loader, even while history materialization still uses an eager bridge.
    // Scan newest-to-oldest, stopping once a surviving compaction checkpoint
    // and the required resume metadata are both known; then replay only the
    // buffered surviving tail forward to preserve exact history semantics.
    let mut base_compaction_summary: Option<Option<ResponseItem>> = None;
    let mut previous_turn_settings = None;
    let mut reference_context_item = TurnReferenceContextItem::NeverSet;
    // Rollback is "drop the newest N user turns". While scanning in reverse,
    // that becomes "skip the next N user-turn segments we finalize".
    let mut pending_rollback_turns = 0usize;
    // Borrowed suffix of rollout items newer than the newest surviving
    // compaction checkpoint. If no such checkpoint exists, this remains the
    // full rollout.
    let mut rollout_suffix = rollout_items;
    // Reverse replay accumulates rollout items into the newest in-progress turn
    // segment until we hit its matching `TurnStarted`, at which point the
    // segment can be finalized.
    let mut active_segment: Option<ActiveReplaySegment> = None;

    for (index, item) in rollout_items.iter().enumerate().rev() {
        match item {
            RolloutItem::Compacted(compacted) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                // Looking backward, compaction clears any older baseline unless
                // a newer `TurnContextItem` in this same segment has already
                // re-established it.
                if matches!(
                    active_segment.reference_context_item,
                    TurnReferenceContextItem::NeverSet
                ) {
                    active_segment.reference_context_item = TurnReferenceContextItem::Cleared;
                }
                if active_segment.base_compaction_summary.is_none() {
                    active_segment.base_compaction_summary =
                        Some(compact_summary_response_item(compacted));
                    rollout_suffix = &rollout_items[index + 1..];
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                pending_rollback_turns = pending_rollback_turns
                    .saturating_add(usize::try_from(rollback.num_turns).unwrap_or(usize::MAX));
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                // Reverse replay often sees `TurnComplete` before any
                // turn-scoped metadata. Capture the turn id early so later
                // `TurnContext` / abort items can match it.
                if active_segment.turn_id.is_none() {
                    active_segment.turn_id = Some(event.turn_id.clone());
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnAborted(event)) => {
                if let Some(active_segment) = active_segment.as_mut() {
                    if active_segment.turn_id.is_none()
                        && let Some(turn_id) = &event.turn_id
                    {
                        active_segment.turn_id = Some(turn_id.clone());
                    }
                } else if let Some(turn_id) = &event.turn_id {
                    active_segment = Some(ActiveReplaySegment {
                        turn_id: Some(turn_id.clone()),
                        ..Default::default()
                    });
                }
            }
            RolloutItem::EventMsg(EventMsg::UserMessage(_)) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                active_segment.counts_as_user_turn = true;
            }
            RolloutItem::TurnContext(ctx) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                // `TurnContextItem` can attach metadata to an existing segment,
                // but only a real `UserMessage` event should make the segment
                // count as a user turn.
                if active_segment.turn_id.is_none() {
                    active_segment.turn_id = ctx.turn_id.clone();
                }
                if turn_ids_are_compatible(
                    active_segment.turn_id.as_deref(),
                    ctx.turn_id.as_deref(),
                ) {
                    active_segment.previous_turn_settings = Some(PreviousTurnSettings {
                        model: ctx.model.clone(),
                        realtime_active: ctx.realtime_active,
                    });
                    if matches!(
                        active_segment.reference_context_item,
                        TurnReferenceContextItem::NeverSet
                    ) {
                        active_segment.reference_context_item =
                            TurnReferenceContextItem::Latest(Box::new(ctx.clone()));
                    }
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnStarted(event)) => {
                // `TurnStarted` is the oldest boundary of the active reverse
                // segment.
                if active_segment.as_ref().is_some_and(|active_segment| {
                    turn_ids_are_compatible(
                        active_segment.turn_id.as_deref(),
                        Some(event.turn_id.as_str()),
                    )
                }) && let Some(active_segment) = active_segment.take()
                {
                    finalize_active_segment(
                        active_segment,
                        &mut base_compaction_summary,
                        &mut previous_turn_settings,
                        &mut reference_context_item,
                        &mut pending_rollback_turns,
                    );
                }
            }
            RolloutItem::ResponseItem(response_item) => {
                let active_segment =
                    active_segment.get_or_insert_with(ActiveReplaySegment::default);
                active_segment.counts_as_user_turn |= is_user_turn_boundary(response_item);
            }
            RolloutItem::EventMsg(_) | RolloutItem::SessionMeta(_) => {}
        }

        if base_compaction_summary.is_some()
            && previous_turn_settings.is_some()
            && !matches!(reference_context_item, TurnReferenceContextItem::NeverSet)
        {
            // At this point we have both eager resume metadata values and the
            // compaction checkpoint for the surviving tail, so older rollout
            // items cannot affect this result.
            break;
        }
    }

    if let Some(active_segment) = active_segment.take() {
        finalize_active_segment(
            active_segment,
            &mut base_compaction_summary,
            &mut previous_turn_settings,
            &mut reference_context_item,
            &mut pending_rollback_turns,
        );
    }

    let mut history = ContextManager::new();
    if let Some(base_compaction_summary) = &base_compaction_summary {
        history.replace(base_compaction_summary.clone().into_iter().collect());
    }
    // Materialize exact history semantics from the replay-derived suffix. The
    // eventual lazy design should keep this same replay shape, but drive it from
    // a resumable reverse source instead of an eagerly loaded `&[RolloutItem]`.
    let mut skip_compact_summary_echo = base_compaction_summary.clone().flatten();
    for item in rollout_suffix {
        match item {
            RolloutItem::ResponseItem(response_item) => {
                if skip_compact_summary_echo
                    .as_ref()
                    .is_some_and(|summary| summary == response_item)
                {
                    skip_compact_summary_echo = None;
                    continue;
                }
                skip_compact_summary_echo = None;
                history.record_items(std::iter::once(response_item), options.truncation_policy);
            }
            RolloutItem::Compacted(compacted) => {
                let summary = compact_summary_response_item(compacted);
                history.replace(summary.clone().into_iter().collect());
                skip_compact_summary_echo = summary;
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                skip_compact_summary_echo = None;
                history.drop_last_n_user_turns(rollback.num_turns);
            }
            RolloutItem::EventMsg(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::SessionMeta(_) => {
                skip_compact_summary_echo = None;
            }
        }
    }

    let reference_context_item = match reference_context_item {
        TurnReferenceContextItem::NeverSet | TurnReferenceContextItem::Cleared => None,
        TurnReferenceContextItem::Latest(turn_reference_context_item) => {
            Some(*turn_reference_context_item)
        }
    };
    RolloutReconstruction {
        history: history.raw_items().to_vec(),
        previous_turn_settings,
        reference_context_item,
    }
}

fn compact_summary_response_item(compacted: &CompactedItem) -> Option<ResponseItem> {
    if compacted.message.trim().is_empty() {
        return None;
    }
    Some(compacted.clone().into())
}

#[cfg(test)]
#[path = "reconstruction_tests.rs"]
mod tests;
