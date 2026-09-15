use super::context_usage_replay;
use super::context_usage_replay::ThreadUsageSource;
use super::thread_processor::should_preserve_persisted_lifecycle_status_for_not_loaded_overlay;
use super::token_usage_replay;
use super::*;

pub(super) struct RunningThreadResumeProjection {
    pub(super) thread: Thread,
    pub(super) token_usage_thread: Option<Thread>,
}

pub(super) async fn project_running_thread_resume_content(
    mut thread: Thread,
    history_items: &[RolloutItem],
    active_turn: Option<&Turn>,
    include_turns: bool,
    usage_source: &(impl ThreadUsageSource + ?Sized),
) -> Thread {
    apply_thread_usage_overlay(&mut thread, history_items, usage_source).await;
    if include_turns {
        populate_thread_turns_from_history(&mut thread, history_items, active_turn);
    }
    thread
}

pub(super) fn finish_running_thread_resume_projection(
    mut thread: Thread,
    include_turns: bool,
    loaded_status: ThreadLifecycleStatus,
    has_live_in_progress_turn: bool,
) -> RunningThreadResumeProjection {
    set_thread_status_and_interrupt_stale_turns(
        &mut thread,
        loaded_status,
        has_live_in_progress_turn,
    );
    let token_usage_thread = include_turns.then(|| thread.clone());
    RunningThreadResumeProjection {
        thread,
        token_usage_thread,
    }
}

async fn apply_thread_usage_overlay(
    thread: &mut Thread,
    history_items: &[RolloutItem],
    usage_source: &(impl ThreadUsageSource + ?Sized),
) {
    thread.token_usage =
        token_usage_replay::latest_thread_token_usage_from_rollout_items(history_items);
    thread.context_usage =
        context_usage_replay::latest_nonzero_thread_context_usage_from_rollout_items(history_items)
            .map(Into::into);
    if let Some(token_usage) = usage_source.token_usage_info().await.map(Into::into) {
        thread.token_usage = Some(token_usage);
    }
    if thread.context_usage.is_none() {
        thread.context_usage = Some(
            context_usage_replay::thread_context_usage_from_rollout_or_conversation(
                usage_source,
                history_items,
            )
            .await
            .into(),
        );
    }
}

pub(crate) fn populate_thread_turns_from_history(
    thread: &mut Thread,
    items: &[RolloutItem],
    active_turn: Option<&Turn>,
) {
    apply_thread_stats_from_rollout_items(thread, items);
    let mut turns = build_api_turns_from_rollout_items(items);
    prune_turns_to_latest_compaction_boundary(&mut turns);
    if let Some(active_turn) = active_turn {
        merge_turn_history_with_active_turn(&mut turns, active_turn.clone());
    }
    thread.turns = turns;
}

pub(super) fn merge_turn_history_with_active_turn(turns: &mut Vec<Turn>, active_turn: Turn) {
    let Some(persisted_turn) = turns.iter_mut().find(|turn| turn.id == active_turn.id) else {
        turns.push(active_turn);
        return;
    };

    persisted_turn.status = active_turn.status;
    persisted_turn.error = active_turn.error;
    persisted_turn.started_at = active_turn.started_at.or(persisted_turn.started_at);
    persisted_turn.completed_at = active_turn.completed_at;
    persisted_turn.duration_ms = active_turn.duration_ms;
    persisted_turn.items_view = active_turn.items_view;

    for active_item in active_turn.items {
        if let Some(existing_item) = persisted_turn
            .items
            .iter_mut()
            .find(|existing_item| existing_item.id() == active_item.id())
        {
            if existing_item == &active_item {
                continue;
            }
            if same_thread_item_kind(existing_item, &active_item)
                && !is_generated_thread_item_id(active_item.id())
            {
                *existing_item = active_item;
            } else if let Some(renamed_item) =
                rename_thread_item_id(active_item, unique_live_item_id(&persisted_turn.items))
            {
                persisted_turn.items.push(renamed_item);
            }
            continue;
        }
        if let Some(existing_index) = persisted_turn.items.iter().position(|existing_item| {
            is_generated_agent_message_duplicate(existing_item, &active_item)
        }) {
            if should_prefer_active_agent_message(
                &persisted_turn.items[existing_index],
                &active_item,
            ) {
                persisted_turn.items[existing_index] = active_item;
            }
            continue;
        }
        if persisted_turn
            .items
            .iter()
            .any(|existing_item| existing_item == &active_item)
        {
            continue;
        }
        persisted_turn.items.push(active_item);
    }
}

fn same_thread_item_kind(left: &ThreadItem, right: &ThreadItem) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn unique_live_item_id(existing_items: &[ThreadItem]) -> String {
    let mut suffix = 1;
    loop {
        let candidate = format!("live-item-{suffix}");
        if existing_items
            .iter()
            .all(|existing_item| existing_item.id() != candidate)
        {
            return candidate;
        }
        suffix += 1;
    }
}

fn is_generated_thread_item_id(id: &str) -> bool {
    id.strip_prefix("item-")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
}

fn is_generated_agent_message_duplicate(left: &ThreadItem, right: &ThreadItem) -> bool {
    let (
        ThreadItem::AgentMessage {
            id: left_id,
            text: left_text,
            phase: left_phase,
            memory_citation: left_memory_citation,
        },
        ThreadItem::AgentMessage {
            id: right_id,
            text: right_text,
            phase: right_phase,
            memory_citation: right_memory_citation,
        },
    ) = (left, right)
    else {
        return false;
    };

    is_generated_thread_item_id(left_id) != is_generated_thread_item_id(right_id)
        && left_text == right_text
        && left_phase == right_phase
        && left_memory_citation == right_memory_citation
}

fn should_prefer_active_agent_message(
    existing_item: &ThreadItem,
    active_item: &ThreadItem,
) -> bool {
    is_generated_thread_item_id(existing_item.id())
        && !is_generated_thread_item_id(active_item.id())
}

fn rename_thread_item_id(item: ThreadItem, next_id: String) -> Option<ThreadItem> {
    let mut value = serde_json::to_value(item).ok()?;
    value.get_mut("id")?.as_str()?;
    value["id"] = serde_json::Value::String(next_id);
    serde_json::from_value(value).ok()
}

pub(super) fn set_thread_status_and_interrupt_stale_turns(
    thread: &mut Thread,
    loaded_status: ThreadLifecycleStatus,
    has_live_in_progress_turn: bool,
) {
    let status = resolve_thread_status(loaded_status, has_live_in_progress_turn);
    let preserve_persisted_status = matches!(status, ThreadLifecycleStatus::NotLoaded)
        && should_preserve_persisted_lifecycle_status_for_not_loaded_overlay(thread);
    let effective_status = if preserve_persisted_status {
        thread.lifecycle_status.clone()
    } else {
        status
    };

    if !matches!(effective_status, ThreadLifecycleStatus::Active { .. }) {
        for turn in &mut thread.turns {
            if matches!(turn.status, TurnStatus::InProgress) {
                turn.status = TurnStatus::Interrupted;
            }
        }
    }
    if !preserve_persisted_status {
        thread.lifecycle_status = effective_status;
    }
}
