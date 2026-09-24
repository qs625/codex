use super::*;
use app_server_protocol::CommandExecutionNotificationKind;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::DynamicToolCallStatus;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::CompactedItem;
use protocol::protocol::EventMsg;
use protocol::subscriptions::PersistedSubscription;
use rollout::EventPersistenceMode;
use rollout::is_persisted_rollout_item;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn restore_persisted_display_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    restore_persisted_flat_compaction_display_turns(thread, persisted_turns);
    restore_persisted_injected_context_turns(thread, persisted_turns);
    reconcile_command_execution_exit_notifications(thread);
}

pub(crate) fn restore_persisted_display_turns_from_rollout_items(
    thread: &mut Thread,
    rollout_items: &[RolloutItem],
) {
    let projection_items = compact_display_projection_rollout_items(rollout_items);
    apply_thread_stats_from_rollout_items(thread, &projection_items);
    let persisted_turns = thread_history::build_turns_from_rollout_items(&projection_items);
    restore_persisted_display_turns(thread, &persisted_turns);
    apply_runtime_activity_items_from_turns(thread, &persisted_turns);
}

pub(crate) fn build_display_turns_from_rollout_items(rollout_items: &[RolloutItem]) -> Vec<Turn> {
    let projection_items = compact_display_projection_rollout_items(rollout_items);
    thread_history::build_turns_from_rollout_items(&projection_items)
}

fn compact_display_projection_rollout_items(rollout_items: &[RolloutItem]) -> Vec<RolloutItem> {
    let mut limited_items = rollout_items
        .iter()
        .filter(|item| is_persisted_rollout_item(item, EventPersistenceMode::Limited))
        .cloned()
        .collect::<Vec<_>>();
    if limited_items
        .iter()
        .any(|item| matches!(item, RolloutItem::Compacted(_)))
    {
        return limited_items;
    }

    if let Some((insert_index, summary)) =
        compact_summary_from_response_item_prefix(&limited_items)
    {
        limited_items.insert(
            insert_index,
            RolloutItem::Compacted(CompactedItem {
                message: summary,
                replacement_history: None,
                visible_replacement_history_len: None,
            }),
        );
    }
    limited_items
}

fn compact_summary_from_response_item_prefix(items: &[RolloutItem]) -> Option<(usize, String)> {
    let context_compacted_index = items
        .iter()
        .position(|item| matches!(item, RolloutItem::EventMsg(EventMsg::ContextCompacted(_))))?;
    let prefix = &items[..context_compacted_index];

    let (summary_index, summary_item) = prefix
        .iter()
        .enumerate()
        .find(|(_, item)| !matches!(item, RolloutItem::SessionMeta(_)))?;
    let RolloutItem::ResponseItem(summary_response_item) = summary_item else {
        return None;
    };
    let summary = assistant_response_message_text(summary_response_item)?;

    let has_compact_context_suffix = prefix[summary_index + 1..].iter().any(|item| {
        matches!(item, RolloutItem::TurnContext(_))
            || matches!(
                item,
                RolloutItem::ResponseItem(ResponseItem::Message { role, .. })
                    if role != "assistant"
            )
    });
    has_compact_context_suffix.then_some((context_compacted_index, summary))
}

fn assistant_response_message_text(item: &ResponseItem) -> Option<String> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "assistant" {
        return None;
    }

    let text = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::OutputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn restore_persisted_flat_compaction_display_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    for (persisted_index, persisted_turn) in persisted_turns.iter().enumerate() {
        let compact_items = flat_compaction_display_items_for_turn(persisted_turn);
        if compact_items.is_empty() {
            continue;
        }

        if let Some(live_turn) = thread
            .turns
            .iter_mut()
            .find(|turn| turn.id == persisted_turn.id)
        {
            remove_unmatched_marker_only_compactions(live_turn, &compact_items);
            let mut insert_index = live_turn
                .items
                .iter()
                .position(|item| matches!(item, ThreadItem::ContextCompaction { .. }))
                .unwrap_or(0);
            insert_persisted_items(live_turn, &compact_items, &mut insert_index);
            live_turn.items_view = TurnItemsView::Full;
            continue;
        }

        let mut compact_turn = persisted_turn.clone();
        compact_turn.items = compact_items;
        compact_turn.items_view = TurnItemsView::Full;
        thread
            .turns
            .insert(persisted_index.min(thread.turns.len()), compact_turn);
    }
}

fn remove_unmatched_marker_only_compactions(live_turn: &mut Turn, compact_items: &[ThreadItem]) {
    if matches!(live_turn.status, TurnStatus::InProgress) {
        return;
    }

    let persisted_compaction_ids = compact_items
        .iter()
        .filter_map(|item| match item {
            ThreadItem::ContextCompaction { id } => Some(id.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();

    let mut index = 0;
    while index < live_turn.items.len() {
        let should_remove = match &live_turn.items[index] {
            ThreadItem::ContextCompaction { id }
                if !persisted_compaction_ids.contains(id.as_str()) =>
            {
                let summary_id = format!("{id}:summary");
                !live_turn.items.get(index + 1).is_some_and(|item| {
                    matches!(
                        item,
                        ThreadItem::AgentMessage { id, .. } if id == &summary_id
                    )
                })
            }
            _ => false,
        };
        if should_remove {
            live_turn.items.remove(index);
        } else {
            index += 1;
        }
    }
}

fn flat_compaction_display_items_for_turn(turn: &Turn) -> Vec<ThreadItem> {
    let mut items = Vec::new();
    let mut iter = turn.items.iter().peekable();
    while let Some(item) = iter.next() {
        let ThreadItem::ContextCompaction { id, .. } = item else {
            continue;
        };
        items.push(item.clone());
        if iter.peek().is_some_and(|next_item| {
            matches!(
                *next_item,
                ThreadItem::AgentMessage { id: summary_id, .. }
                    if summary_id == &format!("{id}:summary")
            )
        }) && let Some(summary_item) = iter.next()
        {
            items.push(summary_item.clone());
        }
    }
    items
}

fn restore_persisted_injected_context_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    for (persisted_index, persisted_turn) in persisted_turns.iter().enumerate() {
        let persisted_injected_items: Vec<_> = persisted_turn
            .items
            .iter()
            .filter(|item| matches!(item, ThreadItem::InjectedContext { .. }))
            .cloned()
            .collect();
        if persisted_injected_items.is_empty() {
            continue;
        }

        if let Some(live_turn) = thread
            .turns
            .iter_mut()
            .find(|turn| turn.id == persisted_turn.id)
        {
            restore_persisted_injected_context_items(
                live_turn,
                persisted_turn,
                &persisted_injected_items,
            );
            continue;
        }

        let mut injected_context_turn = persisted_turn.clone();
        injected_context_turn.items = persisted_injected_items;
        injected_context_turn.items_view = TurnItemsView::Full;
        thread.turns.insert(
            persisted_index.min(thread.turns.len()),
            injected_context_turn,
        );
    }
}

fn insert_persisted_items(
    live_turn: &mut Turn,
    persisted_items: &[ThreadItem],
    insert_index: &mut usize,
) {
    for persisted_item in persisted_items {
        let persisted_id = persisted_item.id().to_string();
        if let Some(existing_index) = live_turn
            .items
            .iter()
            .position(|item| item.id() == persisted_id)
        {
            live_turn.items[existing_index] = persisted_item.clone();
            *insert_index = (*insert_index).max(existing_index + 1);
        } else {
            live_turn
                .items
                .insert(*insert_index, persisted_item.clone());
            *insert_index += 1;
        }
    }
}

pub(super) fn apply_runtime_activity_items_from_persisted_turns(thread: &mut Thread) {
    let persisted_turns = thread.turns.clone();
    apply_runtime_activity_items_from_turns(thread, &persisted_turns);
}

pub(super) fn apply_live_active_command_items_from_active_turn(
    thread: &mut Thread,
    active_turn: Option<&Turn>,
) {
    let mut exited_command_item_ids = reconcile_command_execution_exit_notifications(thread);
    if let Some(active_turn) = active_turn {
        collect_command_execution_exit_notification_ids(active_turn, &mut exited_command_item_ids);
    }
    let active_command_items = active_turn
        .map(|turn| active_command_items_from_live_turn(turn, &exited_command_item_ids))
        .unwrap_or_default();
    thread.active_command_items = Some(active_command_items);
}

fn active_command_items_from_live_turn(
    turn: &Turn,
    exited_command_item_ids: &HashSet<String>,
) -> Vec<ThreadItem> {
    let mut reconciled_turn = turn.clone();
    let exit_notifications = command_execution_exit_notifications_from_turn(&reconciled_turn);
    reconcile_turn_command_execution_exit_notifications(&mut reconciled_turn, &exit_notifications);
    reconciled_turn
        .items
        .iter()
        .filter_map(|item| match item {
            ThreadItem::CommandExecution {
                id,
                status: CommandExecutionStatus::InProgress,
                ..
            } if !exited_command_item_ids.contains(id) => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn reconcile_command_execution_exit_notifications(thread: &mut Thread) -> HashSet<String> {
    let exit_notifications = command_execution_exit_notifications_from_turns(&thread.turns);
    let exited_command_item_ids = exit_notifications.keys().cloned().collect::<HashSet<_>>();
    for turn in &mut thread.turns {
        reconcile_turn_command_execution_exit_notifications(turn, &exit_notifications);
    }
    prune_completed_active_command_items(thread, &exited_command_item_ids);
    exited_command_item_ids
}

fn collect_command_execution_exit_notification_ids(
    turn: &Turn,
    exited_command_item_ids: &mut HashSet<String>,
) {
    for item in &turn.items {
        if let ThreadItem::CommandExecutionNotification {
            command_item_id,
            kind: CommandExecutionNotificationKind::Exit,
            ..
        } = item
        {
            exited_command_item_ids.insert(command_item_id.clone());
        }
    }
}

fn command_execution_exit_notifications_from_turns(
    turns: &[Turn],
) -> HashMap<String, (Option<String>, Option<i32>)> {
    let mut exit_notifications = HashMap::new();
    for turn in turns {
        exit_notifications.extend(command_execution_exit_notifications_from_turn(turn));
    }
    exit_notifications
}

fn command_execution_exit_notifications_from_turn(
    turn: &Turn,
) -> HashMap<String, (Option<String>, Option<i32>)> {
    turn.items
        .iter()
        .filter_map(|item| match item {
            ThreadItem::CommandExecutionNotification {
                command_item_id,
                kind: CommandExecutionNotificationKind::Exit,
                output,
                exit_code,
                ..
            } => Some((command_item_id.clone(), (output.clone(), *exit_code))),
            _ => None,
        })
        .collect()
}

fn reconcile_turn_command_execution_exit_notifications(
    turn: &mut Turn,
    exit_notifications: &HashMap<String, (Option<String>, Option<i32>)>,
) {
    if exit_notifications.is_empty() {
        return;
    }

    for item in &mut turn.items {
        let ThreadItem::CommandExecution {
            id,
            status,
            aggregated_output,
            exit_code: command_exit_code,
            ..
        } = item
        else {
            continue;
        };
        if let Some((output, exit_code)) = exit_notifications.get(id) {
            *status = command_status_from_exit_code(*exit_code);
            *command_exit_code = *exit_code;
            if aggregated_output.is_none() {
                *aggregated_output = output.clone();
            }
        }
    }
}

fn command_status_from_exit_code(exit_code: Option<i32>) -> CommandExecutionStatus {
    match exit_code {
        Some(0) | None => CommandExecutionStatus::Completed,
        Some(_) => CommandExecutionStatus::Failed,
    }
}

fn prune_completed_active_command_items(
    thread: &mut Thread,
    exited_command_item_ids: &HashSet<String>,
) {
    let Some(active_command_items) = thread.active_command_items.as_mut() else {
        return;
    };
    active_command_items.retain(|item| {
        matches!(
            item,
            ThreadItem::CommandExecution {
                id,
                status: CommandExecutionStatus::InProgress,
                ..
            } if !exited_command_item_ids.contains(id)
        )
    });
}

fn apply_runtime_activity_items_from_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    let has_subscription_activity_turn = persisted_turns.iter().any(is_active_subscriptions_turn);
    let has_command_activity_turn = persisted_turns.iter().any(is_active_commands_turn);
    let subscription_items = persisted_turns
        .iter()
        .filter(|turn| is_active_subscriptions_turn(turn))
        .flat_map(|turn| turn.items.iter().cloned())
        .collect::<Vec<_>>();
    let command_items = persisted_turns
        .iter()
        .filter(|turn| is_active_commands_turn(turn))
        .flat_map(|turn| turn.items.iter().cloned())
        .collect::<Vec<_>>();
    thread
        .turns
        .retain(|turn| !is_active_subscriptions_turn(turn) && !is_active_commands_turn(turn));
    if has_subscription_activity_turn {
        thread.active_subscription_items = Some(subscription_items);
    }
    if has_command_activity_turn {
        thread.active_command_items = Some(command_items);
    }
    reconcile_command_execution_exit_notifications(thread);
}

pub(super) fn is_active_subscriptions_turn(turn: &Turn) -> bool {
    turn.id == "active-subscriptions"
}

pub(super) fn active_subscription_items_from_snapshot(
    subscriptions: &[PersistedSubscription],
) -> Vec<ThreadItem> {
    subscriptions
        .iter()
        .filter_map(active_subscription_item_from_snapshot)
        .collect()
}

fn active_subscription_item_from_snapshot(
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

    let mut arguments = serde_json::json!({
        "schedule": schedule,
    });
    if let Some(object) = arguments.as_object_mut() {
        if let Some(label) = label {
            object.insert(
                "label".to_string(),
                serde_json::Value::String(label.clone()),
            );
        }
        if let Some(message) = message {
            object.insert(
                "message".to_string(),
                serde_json::Value::String(message.clone()),
            );
        }
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

pub(super) fn is_active_commands_turn(turn: &Turn) -> bool {
    turn.id == "active-commands"
}

fn restore_persisted_injected_context_items(
    live_turn: &mut Turn,
    persisted_turn: &Turn,
    persisted_injected_items: &[ThreadItem],
) {
    let mut injected_insert_index = live_turn
        .items
        .iter()
        .position(|item| !matches!(item, ThreadItem::InjectedContext { .. }))
        .unwrap_or(live_turn.items.len());

    for persisted_item in persisted_injected_items {
        let persisted_id = persisted_item.id().to_string();
        if let Some(existing_index) = live_turn
            .items
            .iter()
            .position(|item| item.id() == persisted_id)
        {
            live_turn.items[existing_index] = persisted_item.clone();
            injected_insert_index = injected_insert_index.max(existing_index + 1);
        } else {
            if let Some(relative_index) =
                persisted_relative_insert_index(live_turn, persisted_turn, &persisted_id)
            {
                injected_insert_index = injected_insert_index.max(relative_index);
            }
            live_turn
                .items
                .insert(injected_insert_index, persisted_item.clone());
            injected_insert_index += 1;
        }
    }

    if live_turn
        .items
        .iter()
        .any(|item| matches!(item, ThreadItem::InjectedContext { .. }))
    {
        live_turn.items_view = TurnItemsView::Full;
    }
}

fn persisted_relative_insert_index(
    live_turn: &Turn,
    persisted_turn: &Turn,
    persisted_id: &str,
) -> Option<usize> {
    let persisted_index = persisted_turn
        .items
        .iter()
        .position(|item| item.id() == persisted_id)?;

    persisted_turn.items[..persisted_index]
        .iter()
        .rev()
        .find_map(|prior_persisted_item| {
            live_turn
                .items
                .iter()
                .position(|live_item| live_item.id() == prior_persisted_item.id())
                .map(|prior_live_index| prior_live_index + 1)
        })
}

#[cfg(test)]
mod restore_persisted_injected_context_turns_tests {
    use super::*;
    use app_server_protocol::InjectedContextSection;
    use app_server_protocol::SessionSource;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use protocol::ThreadId;
    use protocol::protocol::CompactedItem;
    use protocol::protocol::ContextCompactedEvent;
    use protocol::protocol::ErrorEvent;
    use protocol::protocol::EventMsg;
    use protocol::protocol::ItemCompletedEvent;
    use protocol::protocol::RolloutItem;
    use protocol::protocol::TurnCompleteEvent;
    use protocol::protocol::TurnStartedEvent;

    fn thread_with_turns(turns: Vec<Turn>) -> Thread {
        Thread {
            id: "thread-1".to_string(),
            session_id: "session-1".to_string(),
            forked_from_id: None,
            preview: String::new(),
            ephemeral: false,
            model_provider: "mock_provider".to_string(),
            created_at: 1,
            updated_at: 1,
            lifecycle_status: ThreadLifecycleStatus::completed(None),
            path: None,
            cwd: codex_utils_absolute_path::test_support::test_path_buf("/tmp").abs(),
            cli_version: "0.0.0".to_string(),
            source: SessionSource::Cli,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            git_info: None,
            name: None,
            skills: Vec::new(),
            token_usage: None,
            context_usage: None,
            stats: None,
            turns,
            active_subscription_items: None,
            active_command_items: None,
        }
    }

    fn injected_context_item(id: &str, text: &str) -> ThreadItem {
        injected_context_item_with_label(id, "Instructions", text)
    }

    fn injected_context_item_with_label(id: &str, label: &str, text: &str) -> ThreadItem {
        ThreadItem::InjectedContext {
            id: id.to_string(),
            title: "Init Context".to_string(),
            preview: "Init Context".to_string(),
            sections: vec![InjectedContextSection {
                label: label.to_string(),
                text: text.to_string(),
            }],
        }
    }

    fn agent_message_item(id: &str, text: &str) -> ThreadItem {
        ThreadItem::AgentMessage {
            id: id.to_string(),
            text: text.to_string(),
            phase: None,
            memory_citation: None,
        }
    }

    fn context_compaction_item(id: &str) -> ThreadItem {
        ThreadItem::ContextCompaction { id: id.to_string() }
    }

    fn schedule_subscribe_item(id: &str, subscription_id: &str, label: &str) -> ThreadItem {
        ThreadItem::BuiltinToolCall {
            id: id.to_string(),
            tool: "schedule_subscribe".to_string(),
            arguments: serde_json::json!({
                "schedule": {
                    "kind": "every_interval",
                    "interval_ms": 60000,
                },
                "label": label,
            }),
            status: DynamicToolCallStatus::Completed,
            output: Some(serde_json::json!({
                "subscription_id": subscription_id,
            })),
        }
    }

    fn schedule_unsubscribe_item(id: &str, subscription_id: &str) -> ThreadItem {
        ThreadItem::BuiltinToolCall {
            id: id.to_string(),
            tool: "schedule_unsubscribe".to_string(),
            arguments: serde_json::json!({
                "subscription_id": subscription_id,
            }),
            status: DynamicToolCallStatus::Completed,
            output: Some(serde_json::json!({
                "subscription_id": subscription_id,
                "unsubscribed": true,
            })),
        }
    }

    fn command_execution_item(id: &str, status: CommandExecutionStatus) -> ThreadItem {
        ThreadItem::CommandExecution {
            id: id.to_string(),
            command: "cargo test".to_string(),
            cwd: codex_utils_absolute_path::test_support::test_path_buf("/tmp").abs(),
            process_id: Some("process-1".to_string()),
            source: app_server_protocol::CommandExecutionSource::Agent,
            status,
            initial_wait_ms: Some(1_000),
            notify_on: Some(app_server_protocol::CommandExecutionNotifyOn::Exit),
            command_actions: vec![app_server_protocol::CommandAction::Unknown {
                command: "cargo test".to_string(),
            }],
            aggregated_output: None,
            exit_code: None,
            duration_ms: None,
        }
    }

    fn command_exit_notification_item(
        id: &str,
        command_item_id: &str,
        exit_code: i32,
        output: &str,
    ) -> ThreadItem {
        ThreadItem::CommandExecutionNotification {
            id: id.to_string(),
            command_item_id: command_item_id.to_string(),
            kind: CommandExecutionNotificationKind::Exit,
            message: format!("Command {command_item_id} has exited with code {exit_code}."),
            output: Some(output.to_string()),
            exit_code: Some(exit_code),
            created_at_ms: 3_000,
        }
    }

    fn turn(id: &str, items: Vec<ThreadItem>) -> Turn {
        Turn {
            id: id.to_string(),
            items,
            items_view: TurnItemsView::Full,
            status: TurnStatus::Completed,
            error: None,
            started_at: Some(1),
            completed_at: Some(2),
            duration_ms: Some(100),
        }
    }

    #[test]
    fn restore_persisted_injected_context_turns_replaces_live_sections_with_persisted_item() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![injected_context_item("ctx-1", "live init context")],
        )]);
        let persisted_turns = vec![turn(
            "turn-1",
            vec![injected_context_item(
                "ctx-1",
                "persisted instruction_files text",
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(
            thread.turns[0].items,
            vec![injected_context_item(
                "ctx-1",
                "persisted instruction_files text"
            )]
        );
    }

    #[test]
    fn restore_persisted_injected_context_turns_inserts_missing_turn_without_dup_agent_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-2",
            vec![agent_message_item("msg-1", "final assistant output")],
        )]);
        let persisted_turns = vec![
            turn(
                "turn-1",
                vec![injected_context_item(
                    "ctx-1",
                    "persisted instruction_files text",
                )],
            ),
            turn(
                "turn-2",
                vec![
                    injected_context_item("ctx-2", "persisted compact init context"),
                    agent_message_item("msg-1", "older assistant output"),
                ],
            ),
        ];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(
            thread.turns[0].items,
            vec![injected_context_item(
                "ctx-1",
                "persisted instruction_files text"
            )]
        );
        assert_eq!(
            thread.turns[1].items,
            vec![
                injected_context_item("ctx-2", "persisted compact init context"),
                agent_message_item("msg-1", "final assistant output"),
            ]
        );
    }

    #[test]
    fn restore_persisted_display_turns_from_rollout_items_synthesizes_marker_summary() {
        let mut thread = thread_with_turns(Vec::new());
        let rollout_items = vec![
            RolloutItem::Compacted(CompactedItem {
                message: "compact summary body".to_string(),
                replacement_history: None,
                visible_replacement_history_len: None,
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "compact-turn".to_string(),
                item: protocol::items::TurnItem::InjectedContext(
                    protocol::items::InjectedContextItem {
                        id: "ctx-1".to_string(),
                        title: "Init Context".to_string(),
                        preview: "Init Context".to_string(),
                        sections: vec![protocol::items::InjectedContextSection {
                            label: "User Preferences".to_string(),
                            text: "# User Preferences\n\nProject body".to_string(),
                        }],
                    },
                ),
                completed_at_ms: 1,
            })),
        ];

        restore_persisted_display_turns_from_rollout_items(&mut thread, &rollout_items);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(
            thread.turns[0].items,
            vec![
                context_compaction_item("item-1"),
                agent_message_item("item-1:summary", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body"
                ),
            ]
        );
    }

    #[test]
    fn restore_persisted_display_turns_replaces_live_marker_context_with_durable_summary() {
        let mut thread = thread_with_turns(vec![turn(
            "compact-turn",
            vec![
                context_compaction_item("item-2"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body",
                ),
            ],
        )]);
        let persisted_turns = vec![turn(
            "compact-turn",
            vec![
                context_compaction_item("item-1"),
                agent_message_item("item-1:summary", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body",
                ),
            ],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);
        prune_turns_to_latest_compaction_boundary(&mut thread.turns);

        assert_eq!(
            thread.turns[0].items,
            vec![
                context_compaction_item("item-1"),
                agent_message_item("item-1:summary", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body"
                ),
            ]
        );
    }

    #[test]
    fn restore_persisted_display_turns_recovers_head_compact_summary_before_live_marker() {
        let turn_id = "01a0d249-650b-73f3-89ab-7074cb750ed9";
        let init_context_id = "28a0f05b-3b5b-4f72-9da1-5f3dff71a395";
        let mut thread = thread_with_turns(vec![turn(
            turn_id,
            vec![
                context_compaction_item("item-2"),
                injected_context_item_with_label(
                    init_context_id,
                    "Permissions",
                    "danger-full-access",
                ),
                agent_message_item("live-message-1", "post-compact live suffix"),
            ],
        )]);
        let rollout_items = vec![
            RolloutItem::Compacted(CompactedItem {
                message: "compact summary body".to_string(),
                replacement_history: None,
                visible_replacement_history_len: None,
            }),
            RolloutItem::TurnContext(protocol::protocol::TurnContextItem {
                turn_id: Some(turn_id.to_string()),
                trace_id: None,
                cwd: std::path::PathBuf::from("/tmp"),
                current_date: None,
                timezone: None,
                approval_policy: protocol::protocol::AskForApproval::Never,
                sandbox_policy: protocol::protocol::SandboxPolicy::DangerFullAccess,
                permission_profile: None,
                network: None,
                file_system_sandbox_policy: None,
                model: "test-model".to_string(),
                personality: None,
                collaboration_mode: None,
                realtime_active: None,
                effort: None,
                summary: protocol::config_types::ReasoningSummary::Auto,
                user_instructions: None,
                developer_instructions: None,
                init_context_snapshot: None,
                final_output_json_schema: None,
                truncation_policy: None,
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: turn_id.to_string(),
                item: protocol::items::TurnItem::InjectedContext(
                    protocol::items::InjectedContextItem {
                        id: init_context_id.to_string(),
                        title: "Init Context".to_string(),
                        preview: "Permissions • Apps • Skills".to_string(),
                        sections: vec![protocol::items::InjectedContextSection {
                            label: "Permissions".to_string(),
                            text: "danger-full-access".to_string(),
                        }],
                    },
                ),
                completed_at_ms: 1,
            })),
        ];

        restore_persisted_display_turns_from_rollout_items(&mut thread, &rollout_items);
        prune_turns_to_latest_compaction_boundary(&mut thread.turns);

        assert_eq!(
            thread.turns[0]
                .items
                .iter()
                .map(ThreadItem::id)
                .collect::<Vec<_>>(),
            vec![
                "item-1",
                "item-1:summary",
                init_context_id,
                "live-message-1"
            ]
        );
        assert!(matches!(
            &thread.turns[0].items[1],
            ThreadItem::AgentMessage { text, .. } if text == "compact summary body"
        ));
    }

    #[test]
    fn build_display_turns_from_rollout_items_recovers_compact_summary_response_prefix() {
        let rollout_items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "recovered compact summary".to_string(),
                }],
                phase: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "developer".to_string(),
                content: vec![ContentItem::InputText {
                    text: "developer context restored after compact".to_string(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "compact-turn".to_string(),
                item: protocol::items::TurnItem::InjectedContext(
                    protocol::items::InjectedContextItem {
                        id: "ctx-1".to_string(),
                        title: "Init Context".to_string(),
                        preview: "Init Context".to_string(),
                        sections: vec![protocol::items::InjectedContextSection {
                            label: "User Preferences".to_string(),
                            text: "# User Preferences\n\nProject body".to_string(),
                        }],
                    },
                ),
                completed_at_ms: 1,
            })),
        ];

        let turns = build_display_turns_from_rollout_items(&rollout_items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                context_compaction_item("item-1"),
                agent_message_item("item-1:summary", "recovered compact summary"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body"
                ),
            ]
        );
    }

    #[test]
    fn build_display_turns_from_rollout_items_does_not_display_generic_response_messages() {
        let rollout_items = vec![RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "model context only".to_string(),
            }],
            phase: None,
        })];

        let turns = build_display_turns_from_rollout_items(&rollout_items);

        assert!(turns.is_empty());
    }

    #[test]
    fn build_display_turns_from_rollout_items_does_not_treat_plain_response_before_compact_as_summary()
    {
        let rollout_items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "ordinary model context assistant output".to_string(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        ];

        let turns = build_display_turns_from_rollout_items(&rollout_items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, vec![context_compaction_item("item-1")]);
    }

    #[test]
    fn build_display_turns_from_rollout_items_prefers_persisted_compacted_item() {
        let rollout_items = vec![
            RolloutItem::Compacted(CompactedItem {
                message: "persisted compacted summary".to_string(),
                replacement_history: None,
                visible_replacement_history_len: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "fallback summary must not duplicate".to_string(),
                }],
                phase: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: None,
                role: "developer".to_string(),
                content: vec![ContentItem::InputText {
                    text: "developer context restored after compact".to_string(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        ];

        let turns = build_display_turns_from_rollout_items(&rollout_items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                context_compaction_item("item-1"),
                agent_message_item("item-1:summary", "persisted compacted summary"),
            ]
        );
    }

    #[test]
    fn build_display_turns_from_rollout_items_filters_extended_only_events() {
        let rollout_items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".to_string(),
                started_at: Some(1),
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(
                protocol::protocol::AgentMessageEvent {
                    message: "visible assistant output".to_string(),
                    phase: None,
                    memory_citation: None,
                },
            )),
            RolloutItem::EventMsg(EventMsg::Error(ErrorEvent {
                message: "extended-only stream failure".to_string(),
                codex_error_info: None,
            })),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-1".to_string(),
                last_agent_message: None,
                completed_at: Some(2),
                duration_ms: Some(1),
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_display_turns_from_rollout_items(&rollout_items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(turns[0].error, None);
        assert_eq!(
            turns[0].items,
            vec![agent_message_item("item-1", "visible assistant output")]
        );
    }

    #[test]
    fn restore_persisted_display_turns_keeps_active_subscription_out_of_turns() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        let persisted_turns = vec![
            turn(
                "turn-1",
                vec![agent_message_item("msg-1", "thread restored live")],
            ),
            turn(
                "active-subscriptions",
                vec![schedule_subscribe_item(
                    "active-subscription:sub-schedule",
                    "sub-schedule",
                    "standup",
                )],
            ),
        ];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.active_subscription_items.is_none());
    }

    #[test]
    fn apply_runtime_activity_items_from_turns_sets_subscription_current_state_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        let persisted_turns = vec![
            turn(
                "turn-1",
                vec![agent_message_item("msg-1", "thread restored live")],
            ),
            turn(
                "active-subscriptions",
                vec![schedule_subscribe_item(
                    "active-subscription:sub-schedule",
                    "sub-schedule",
                    "standup",
                )],
            ),
        ];

        apply_runtime_activity_items_from_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(
            thread.active_subscription_items,
            Some(vec![schedule_subscribe_item(
                "active-subscription:sub-schedule",
                "sub-schedule",
                "standup"
            )])
        );
    }

    #[test]
    fn active_subscription_items_from_snapshot_projects_schedule_subscribe_item() {
        let subscriptions = vec![protocol::subscriptions::PersistedSubscription::Schedule {
            subscription_id: "sub-schedule".to_string(),
            schedule: protocol::subscriptions::ScheduleSpec::EveryInterval {
                interval_ms: 60_000,
            },
            label: Some("standup".to_string()),
            message: Some("Run standup checks".to_string()),
        }];

        let items = active_subscription_items_from_snapshot(subscriptions.as_slice());

        assert_eq!(
            items,
            vec![ThreadItem::BuiltinToolCall {
                id: "active-subscription:sub-schedule".to_string(),
                tool: "schedule_subscribe".to_string(),
                arguments: serde_json::json!({
                    "schedule": {
                        "kind": "every_interval",
                        "interval_ms": 60_000,
                    },
                    "label": "standup",
                    "message": "Run standup checks",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "subscription_id": "sub-schedule",
                })),
            }]
        );
    }

    #[test]
    fn apply_runtime_activity_items_from_turns_sets_command_current_state_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        let persisted_turns = vec![
            turn(
                "turn-1",
                vec![agent_message_item("msg-1", "thread restored live")],
            ),
            turn(
                "active-commands",
                vec![command_execution_item(
                    "exec-1",
                    CommandExecutionStatus::InProgress,
                )],
            ),
        ];

        apply_runtime_activity_items_from_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(
            thread.active_command_items,
            Some(vec![command_execution_item(
                "exec-1",
                CommandExecutionStatus::InProgress
            )])
        );
    }

    #[test]
    fn restore_persisted_display_turns_from_rollout_items_ignores_session_meta_display_snapshot() {
        use protocol::protocol::RolloutItem;
        use protocol::protocol::SessionMeta;
        use protocol::protocol::SessionMetaLine;
        use protocol::subscriptions::PersistedSubscription;
        use protocol::subscriptions::ScheduleSpec;

        let mut thread = thread_with_turns(Vec::new());
        let rollout_items = vec![RolloutItem::SessionMeta(SessionMetaLine {
            meta: SessionMeta {
                subscriptions: Some(vec![PersistedSubscription::Schedule {
                    subscription_id: "sub-schedule".to_string(),
                    schedule: ScheduleSpec::EveryInterval {
                        interval_ms: 60_000,
                    },
                    label: Some("standup".to_string()),
                    message: None,
                }]),
                ..SessionMeta::default()
            },
            git: None,
        })];

        restore_persisted_display_turns_from_rollout_items(&mut thread, &rollout_items);

        assert!(thread.turns.is_empty());
        assert!(thread.active_subscription_items.is_none());
    }

    #[test]
    fn restore_persisted_display_turns_skips_duplicate_schedule_subscription() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![schedule_subscribe_item(
                "call-schedule",
                "sub-schedule",
                "standup",
            )],
        )]);
        let persisted_turns = vec![turn(
            "active-subscriptions",
            vec![schedule_subscribe_item(
                "active-subscription:sub-schedule",
                "sub-schedule",
                "standup",
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.active_subscription_items.is_none());
        assert_eq!(
            thread.turns[0].items,
            vec![schedule_subscribe_item(
                "call-schedule",
                "sub-schedule",
                "standup"
            )]
        );
    }

    #[test]
    fn restore_persisted_display_turns_inserts_empty_snapshot_cleanup_turn() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        let persisted_turns = vec![turn(
            "active-subscriptions",
            vec![schedule_unsubscribe_item(
                "active-subscription:sub-schedule:inactive",
                "sub-schedule",
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.active_subscription_items.is_none());
        apply_runtime_activity_items_from_turns(&mut thread, &persisted_turns);
        assert_eq!(
            thread.active_subscription_items,
            Some(vec![schedule_unsubscribe_item(
                "active-subscription:sub-schedule:inactive",
                "sub-schedule"
            )])
        );
    }

    #[test]
    fn apply_runtime_activity_items_from_turns_clears_current_state_with_empty_turns() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        thread.active_subscription_items = Some(vec![schedule_subscribe_item(
            "active-subscription:sub-schedule",
            "sub-schedule",
            "standup",
        )]);
        thread.active_command_items = Some(vec![command_execution_item(
            "exec-1",
            CommandExecutionStatus::InProgress,
        )]);
        let persisted_turns = vec![
            turn("active-subscriptions", Vec::new()),
            turn("active-commands", Vec::new()),
        ];

        apply_runtime_activity_items_from_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.active_subscription_items, Some(Vec::new()));
        assert_eq!(thread.active_command_items, Some(Vec::new()));
        assert_eq!(thread.turns.len(), 1);
    }

    #[test]
    fn live_active_command_items_override_persisted_command_current_state_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        let persisted_turns = vec![turn(
            "active-commands",
            vec![command_execution_item(
                "persisted-exec",
                CommandExecutionStatus::InProgress,
            )],
        )];

        apply_runtime_activity_items_from_turns(&mut thread, &persisted_turns);
        let live_turn = turn(
            "turn-live",
            vec![command_execution_item(
                "live-exec",
                CommandExecutionStatus::InProgress,
            )],
        );
        apply_live_active_command_items_from_active_turn(&mut thread, Some(&live_turn));

        assert_eq!(
            thread.active_command_items,
            Some(vec![command_execution_item(
                "live-exec",
                CommandExecutionStatus::InProgress
            )])
        );
    }

    #[test]
    fn restore_persisted_display_turns_preserves_live_command_items() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-live",
            vec![
                agent_message_item("msg-1", "thinking"),
                command_execution_item("exec-running", CommandExecutionStatus::InProgress),
            ],
        )]);
        let persisted_turns = vec![turn(
            "turn-live",
            vec![injected_context_item("ctx-1", "restored instructions")],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(
            thread.turns[0].items,
            vec![
                injected_context_item("ctx-1", "restored instructions"),
                agent_message_item("msg-1", "thinking"),
                command_execution_item("exec-running", CommandExecutionStatus::InProgress),
            ]
        );
    }

    #[test]
    fn apply_live_active_command_items_uses_only_live_in_progress_commands() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![agent_message_item("msg-1", "thread restored live")],
        )]);
        thread.active_command_items = Some(vec![command_execution_item(
            "stale-exec",
            CommandExecutionStatus::InProgress,
        )]);
        let live_turn = turn(
            "turn-live",
            vec![
                command_execution_item("exec-running", CommandExecutionStatus::InProgress),
                command_execution_item("exec-done", CommandExecutionStatus::Completed),
                schedule_subscribe_item("call-schedule", "sub-schedule", "standup"),
            ],
        );

        apply_live_active_command_items_from_active_turn(&mut thread, Some(&live_turn));

        assert_eq!(
            thread.active_command_items,
            Some(vec![command_execution_item(
                "exec-running",
                CommandExecutionStatus::InProgress
            )])
        );
    }

    #[test]
    fn command_exit_notification_completes_cross_turn_command_and_clears_stale_active_item() {
        let mut thread = thread_with_turns(vec![
            turn(
                "turn-command",
                vec![command_execution_item(
                    "exec-running",
                    CommandExecutionStatus::InProgress,
                )],
            ),
            turn(
                "turn-exit",
                vec![
                    command_exit_notification_item(
                        "exec-running:notification:exit",
                        "exec-running",
                        0,
                        "done\n",
                    ),
                    agent_message_item("msg-after", "next output"),
                ],
            ),
        ]);
        thread.active_command_items = Some(vec![command_execution_item(
            "exec-running",
            CommandExecutionStatus::InProgress,
        )]);
        let live_turn = turn(
            "turn-live",
            vec![command_execution_item(
                "exec-running",
                CommandExecutionStatus::InProgress,
            )],
        );

        apply_live_active_command_items_from_active_turn(&mut thread, Some(&live_turn));

        assert_eq!(thread.active_command_items, Some(Vec::new()));
        let command_item = thread.turns[0]
            .items
            .iter()
            .find(|item| item.id() == "exec-running")
            .expect("command item should remain in its original turn");
        assert!(matches!(
            command_item,
            ThreadItem::CommandExecution {
                status: CommandExecutionStatus::Completed,
                aggregated_output: Some(output),
                exit_code: Some(0),
                ..
            } if output == "done\n"
        ));
        assert_eq!(
            thread.turns[1]
                .items
                .iter()
                .map(|item| item.id())
                .collect::<Vec<_>>(),
            vec!["exec-running:notification:exit", "msg-after"]
        );
    }

    #[test]
    fn apply_live_active_command_items_clears_stale_commands_when_live_has_none() {
        let mut thread = thread_with_turns(Vec::new());
        thread.active_command_items = Some(vec![command_execution_item(
            "stale-exec",
            CommandExecutionStatus::InProgress,
        )]);

        apply_live_active_command_items_from_active_turn(&mut thread, None);

        assert_eq!(thread.active_command_items, Some(Vec::new()));
    }

    #[test]
    fn restore_persisted_display_turns_skips_duplicate_schedule_unsubscription() {
        let mut thread = thread_with_turns(vec![turn(
            "turn-1",
            vec![schedule_unsubscribe_item(
                "call-unsubscribe",
                "sub-schedule",
            )],
        )]);
        let persisted_turns = vec![turn(
            "active-subscriptions",
            vec![schedule_unsubscribe_item(
                "active-subscription:sub-schedule:inactive",
                "sub-schedule",
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.active_subscription_items.is_none());
        assert_eq!(
            thread.turns[0].items,
            vec![schedule_unsubscribe_item(
                "call-unsubscribe",
                "sub-schedule"
            )]
        );
    }
}
