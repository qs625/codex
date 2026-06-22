//! Rollout replay and model-visible history reconstruction.

use codex_context_manager::ContextManager;
use codex_context_manager::is_contextual_user_message_content;
use codex_context_manager::is_user_turn_boundary;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::TurnContextItem;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;

const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;

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
struct ActiveReplaySegment<'a> {
    turn_id: Option<String>,
    counts_as_user_turn: bool,
    previous_turn_settings: Option<PreviousTurnSettings>,
    reference_context_item: TurnReferenceContextItem,
    base_replacement_history: Option<&'a [ResponseItem]>,
}

fn turn_ids_are_compatible(active_turn_id: Option<&str>, item_turn_id: Option<&str>) -> bool {
    active_turn_id
        .is_none_or(|turn_id| item_turn_id.is_none_or(|item_turn_id| item_turn_id == turn_id))
}

fn finalize_active_segment<'a>(
    active_segment: ActiveReplaySegment<'a>,
    base_replacement_history: &mut Option<&'a [ResponseItem]>,
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

    // A surviving replacement-history checkpoint is a complete history base.
    // Once we know the newest surviving one, older rollout items do not affect
    // rebuilt history.
    if base_replacement_history.is_none()
        && let Some(segment_base_replacement_history) = active_segment.base_replacement_history
    {
        *base_replacement_history = Some(segment_base_replacement_history);
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
    // Scan newest-to-oldest, stopping once a surviving replacement-history
    // checkpoint and the required resume metadata are both known; then replay
    // only the buffered surviving tail forward to preserve exact history
    // semantics.
    let mut base_replacement_history: Option<&[ResponseItem]> = None;
    let mut previous_turn_settings = None;
    let mut reference_context_item = TurnReferenceContextItem::NeverSet;
    // Rollback is "drop the newest N user turns". While scanning in reverse,
    // that becomes "skip the next N user-turn segments we finalize".
    let mut pending_rollback_turns = 0usize;
    // Borrowed suffix of rollout items newer than the newest surviving
    // replacement-history checkpoint. If no such checkpoint exists, this
    // remains the full rollout.
    let mut rollout_suffix = rollout_items;
    // Reverse replay accumulates rollout items into the newest in-progress turn
    // segment until we hit its matching `TurnStarted`, at which point the
    // segment can be finalized.
    let mut active_segment: Option<ActiveReplaySegment<'_>> = None;

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
                if active_segment.base_replacement_history.is_none()
                    && let Some(replacement_history) = &compacted.replacement_history
                {
                    active_segment.base_replacement_history = Some(replacement_history);
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
                        &mut base_replacement_history,
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

        if base_replacement_history.is_some()
            && previous_turn_settings.is_some()
            && !matches!(reference_context_item, TurnReferenceContextItem::NeverSet)
        {
            // At this point we have both eager resume metadata values and the
            // replacement-history base for the surviving tail, so older rollout
            // items cannot affect this result.
            break;
        }
    }

    if let Some(active_segment) = active_segment.take() {
        finalize_active_segment(
            active_segment,
            &mut base_replacement_history,
            &mut previous_turn_settings,
            &mut reference_context_item,
            &mut pending_rollback_turns,
        );
    }

    let mut history = ContextManager::new();
    let mut saw_legacy_compaction_without_replacement_history = false;
    if let Some(base_replacement_history) = base_replacement_history {
        history.replace(base_replacement_history.to_vec());
    }
    // Materialize exact history semantics from the replay-derived suffix. The
    // eventual lazy design should keep this same replay shape, but drive it from
    // a resumable reverse source instead of an eagerly loaded `&[RolloutItem]`.
    for item in rollout_suffix {
        match item {
            RolloutItem::ResponseItem(response_item) => {
                history.record_items(std::iter::once(response_item), options.truncation_policy);
            }
            RolloutItem::Compacted(compacted) => {
                if let Some(replacement_history) = &compacted.replacement_history {
                    // This should never happen, because the reverse loop above
                    // should stop before any compaction with replacement history.
                    history.replace(replacement_history.clone());
                } else {
                    saw_legacy_compaction_without_replacement_history = true;
                    // Legacy rollouts without `replacement_history` should
                    // rebuild the historical TurnContext at the correct
                    // insertion point from persisted `TurnContextItem`s. These
                    // are rare enough that we currently clear
                    // `reference_context_item`, reinject canonical context at
                    // the end of the resumed conversation, and accept the
                    // temporary out-of-distribution prompt shape.
                    let user_messages =
                        collect_user_messages(history.raw_items(), options.summary_prefix);
                    let rebuilt =
                        build_compacted_history(Vec::new(), &user_messages, &compacted.message);
                    history.replace(rebuilt);
                }
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback)) => {
                history.drop_last_n_user_turns(rollback.num_turns);
            }
            RolloutItem::EventMsg(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::SessionMeta(_) => {}
        }
    }

    let reference_context_item = match reference_context_item {
        TurnReferenceContextItem::NeverSet | TurnReferenceContextItem::Cleared => None,
        TurnReferenceContextItem::Latest(turn_reference_context_item) => {
            Some(*turn_reference_context_item)
        }
    };
    let reference_context_item = if saw_legacy_compaction_without_replacement_history {
        None
    } else {
        reference_context_item
    };

    RolloutReconstruction {
        history: history.raw_items().to_vec(),
        previous_turn_settings,
        reference_context_item,
    }
}

fn collect_user_messages(items: &[ResponseItem], summary_prefix: Option<&str>) -> Vec<String> {
    items
        .iter()
        .filter_map(user_message_text)
        .filter(|message| !is_summary_message(message, summary_prefix))
        .filter(|message| !is_legacy_warning_message(message))
        .collect()
}

fn user_message_text(item: &ResponseItem) -> Option<String> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "user" || is_contextual_user_message_content(content) {
        return None;
    }
    content_items_to_text(content)
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

fn is_summary_message(message: &str, summary_prefix: Option<&str>) -> bool {
    summary_prefix.is_some_and(|prefix| message.starts_with(format!("{prefix}\n").as_str()))
}

fn is_legacy_warning_message(message: &str) -> bool {
    message.starts_with(
        "Warning: The maximum number of unified exec processes you can keep open is ",
    ) || message.starts_with(
        "Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.",
    ) || message.starts_with(
        "Warning: Your account was flagged for potentially high-risk cyber activity",
    )
}

fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    build_compacted_history_with_limit(
        initial_context,
        user_messages,
        summary_text,
        COMPACT_USER_MESSAGE_MAX_TOKENS,
    )
}

fn build_compacted_history_with_limit(
    mut history: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
    max_tokens: usize,
) -> Vec<ResponseItem> {
    let mut selected_messages: Vec<String> = Vec::new();
    if max_tokens > 0 {
        let mut remaining = max_tokens;
        for message in user_messages.iter().rev() {
            if remaining == 0 {
                break;
            }
            let tokens = approx_token_count(message);
            if tokens <= remaining {
                selected_messages.push(message.clone());
                remaining = remaining.saturating_sub(tokens);
            } else {
                let truncated = truncate_text(message, TruncationPolicy::Tokens(remaining));
                selected_messages.push(truncated);
                break;
            }
        }
        selected_messages.reverse();
    }

    for message in &selected_messages {
        history.push(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: message.clone(),
            }],
            phase: None,
        });
    }

    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };

    history.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: summary_text }],
        phase: None,
    });

    history
}

#[cfg(test)]
#[path = "reconstruction_tests.rs"]
mod tests;
