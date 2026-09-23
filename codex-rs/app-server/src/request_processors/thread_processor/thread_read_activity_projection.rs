use super::*;
use app_server_protocol::CommandExecutionNotificationKind;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::ContextCompactionReplacementItem;
use app_server_protocol::DynamicToolCallStatus;
use protocol::subscriptions::PersistedSubscription;
use rollout::EventPersistenceMode;
use rollout::is_persisted_rollout_item;
use std::collections::HashMap;
use std::collections::HashSet;

pub(super) fn restore_persisted_display_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    restore_persisted_compaction_replacement_turns(thread, persisted_turns);
    restore_persisted_injected_context_turns(thread, persisted_turns);
    reconcile_command_execution_exit_notifications(thread);
}

pub(crate) fn restore_persisted_display_turns_from_rollout_items(
    thread: &mut Thread,
    rollout_items: &[RolloutItem],
) {
    apply_thread_stats_from_rollout_items(thread, rollout_items);
    let persisted_turns = thread_history::build_turns_from_rollout_items(rollout_items);
    restore_persisted_display_turns(thread, &persisted_turns);
    apply_runtime_activity_items_from_turns(thread, &persisted_turns);
}

pub(crate) fn build_display_turns_from_rollout_items(rollout_items: &[RolloutItem]) -> Vec<Turn> {
    let limited_items = rollout_items
        .iter()
        .filter(|item| is_persisted_rollout_item(item, EventPersistenceMode::Limited))
        .cloned()
        .collect::<Vec<_>>();
    let mut turns = thread_history::build_turns_from_rollout_items(&limited_items);
    expand_persisted_compaction_replacement_turns(&mut turns);
    turns
}

pub(super) fn expand_persisted_compaction_replacement_turns(turns: &mut Vec<Turn>) {
    let persisted_turns = turns.clone();
    restore_persisted_compaction_replacement_turns_into(turns, &persisted_turns);
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
            restore_persisted_injected_context_items(live_turn, &persisted_injected_items);
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

fn restore_persisted_compaction_replacement_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    restore_persisted_compaction_replacement_turns_into(&mut thread.turns, persisted_turns);
}

fn restore_persisted_compaction_replacement_turns_into(
    turns: &mut Vec<Turn>,
    persisted_turns: &[Turn],
) {
    for (persisted_index, persisted_turn) in persisted_turns.iter().enumerate() {
        if !persisted_turn
            .items
            .iter()
            .any(|item| matches!(item, ThreadItem::ContextCompaction { .. }))
        {
            continue;
        }

        if let Some(live_turn) = turns.iter_mut().find(|turn| turn.id == persisted_turn.id) {
            restore_persisted_compaction_replacement_items(live_turn, persisted_turn);
            continue;
        }

        let restored_items = compact_replacement_display_items_for_turn(persisted_turn);
        if restored_items.is_empty() {
            continue;
        }
        let mut compact_turn = persisted_turn.clone();
        compact_turn.items = restored_items;
        compact_turn.items_view = TurnItemsView::Full;
        turns.insert(persisted_index.min(turns.len()), compact_turn);
    }
}

fn restore_persisted_compaction_replacement_items(live_turn: &mut Turn, persisted_turn: &Turn) {
    let mut insert_index = live_turn.items.len();
    for persisted_item in &persisted_turn.items {
        let ThreadItem::ContextCompaction {
            id,
            summary,
            replacement_history,
        } = persisted_item
        else {
            continue;
        };

        let compact_item = ThreadItem::ContextCompaction {
            id: id.clone(),
            summary: summary.clone(),
            replacement_history: None,
        };
        let compact_index = if let Some(existing_index) = live_turn
            .items
            .iter()
            .position(|item| matches!(item, ThreadItem::ContextCompaction { id: existing_id, .. } if existing_id == id))
        {
            live_turn.items[existing_index] = compact_item;
            existing_index
        } else {
            live_turn.items.insert(insert_index, compact_item);
            let inserted_index = insert_index;
            inserted_index
        };

        insert_index = compact_index + 1;
        if let Some(replacement_history) = replacement_history.as_ref() {
            let replacement_items = replacement_history
                .iter()
                .map(thread_item_from_compaction_replacement_item)
                .collect::<Vec<_>>();
            insert_persisted_items(live_turn, &replacement_items, &mut insert_index);
        }
    }

    if live_turn
        .items
        .iter()
        .any(|item| matches!(item, ThreadItem::ContextCompaction { .. }))
    {
        live_turn.items_view = TurnItemsView::Full;
    }
}

fn compact_replacement_display_items_for_turn(turn: &Turn) -> Vec<ThreadItem> {
    let mut items = Vec::new();
    for item in &turn.items {
        let ThreadItem::ContextCompaction {
            id,
            summary,
            replacement_history,
        } = item
        else {
            continue;
        };
        items.push(ThreadItem::ContextCompaction {
            id: id.clone(),
            summary: summary.clone(),
            replacement_history: None,
        });
        if let Some(replacement_history) = replacement_history.as_ref() {
            items.extend(
                replacement_history
                    .iter()
                    .map(thread_item_from_compaction_replacement_item),
            );
        }
    }
    items
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

fn thread_item_from_compaction_replacement_item(
    item: &ContextCompactionReplacementItem,
) -> ThreadItem {
    match item {
        ContextCompactionReplacementItem::InjectedContext {
            id,
            title,
            preview,
            sections,
        } => ThreadItem::InjectedContext {
            id: id.clone(),
            title: title.clone(),
            preview: preview.clone(),
            sections: sections.clone(),
        },
        ContextCompactionReplacementItem::UserMessage { id, content } => ThreadItem::UserMessage {
            id: id.clone(),
            content: content.clone(),
        },
        ContextCompactionReplacementItem::AgentMessage {
            id,
            text,
            phase,
            memory_citation,
        } => ThreadItem::AgentMessage {
            id: id.clone(),
            text: text.clone(),
            phase: phase.clone(),
            memory_citation: memory_citation.clone(),
        },
        ContextCompactionReplacementItem::ConversationArtifact {
            id,
            title,
            source,
            mime_type,
            content,
            language,
            truncated,
        } => ThreadItem::ConversationArtifact {
            id: id.clone(),
            title: title.clone(),
            source: source.clone(),
            mime_type: mime_type.clone(),
            content: content.clone(),
            language: language.clone(),
            truncated: *truncated,
        },
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

#[cfg(test)]
mod restore_persisted_injected_context_turns_tests {
    use super::*;
    use app_server_protocol::InjectedContextSection;
    use app_server_protocol::SessionSource;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use protocol::ThreadId;
    use protocol::models::ContentItem;
    use protocol::models::ResponseItem;
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

    fn context_compaction_item(
        id: &str,
        summary: &str,
        replacement_history: Option<Vec<ContextCompactionReplacementItem>>,
    ) -> ThreadItem {
        ThreadItem::ContextCompaction {
            id: id.to_string(),
            summary: Some(summary.to_string()),
            replacement_history,
        }
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
    fn restore_persisted_display_turns_inserts_compaction_replacement_display_items() {
        let mut thread = thread_with_turns(Vec::new());
        let persisted_turns = vec![turn(
            "compact-turn",
            vec![context_compaction_item(
                "compact-1",
                "compact marker summary",
                Some(vec![
                    ContextCompactionReplacementItem::AgentMessage {
                        id: "summary-1".to_string(),
                        text: "compact summary body".to_string(),
                        phase: None,
                        memory_citation: None,
                    },
                    ContextCompactionReplacementItem::InjectedContext {
                        id: "ctx-1".to_string(),
                        title: "Init Context".to_string(),
                        preview: "Init Context".to_string(),
                        sections: vec![InjectedContextSection {
                            label: "User Preferences".to_string(),
                            text: "# User Preferences\n\nProject body".to_string(),
                        }],
                    },
                ]),
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(thread.turns.len(), 1);
        assert_eq!(
            thread.turns[0].items,
            vec![
                context_compaction_item("compact-1", "compact marker summary", None),
                agent_message_item("summary-1", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body"
                ),
            ]
        );
    }

    #[test]
    fn restore_persisted_display_turns_restores_replacement_items_after_live_compaction_marker() {
        let mut thread = thread_with_turns(vec![turn(
            "compact-turn",
            vec![
                context_compaction_item("compact-1", "stale marker summary", None),
                agent_message_item("suffix-1", "post compact suffix"),
            ],
        )]);
        let persisted_turns = vec![turn(
            "compact-turn",
            vec![context_compaction_item(
                "compact-1",
                "compact marker summary",
                Some(vec![
                    ContextCompactionReplacementItem::AgentMessage {
                        id: "summary-1".to_string(),
                        text: "compact summary body".to_string(),
                        phase: None,
                        memory_citation: None,
                    },
                    ContextCompactionReplacementItem::InjectedContext {
                        id: "ctx-1".to_string(),
                        title: "Init Context".to_string(),
                        preview: "Init Context".to_string(),
                        sections: vec![InjectedContextSection {
                            label: "Project Understanding".to_string(),
                            text: "# Project Understanding\n\nProject body".to_string(),
                        }],
                    },
                ]),
            )],
        )];

        restore_persisted_display_turns(&mut thread, &persisted_turns);

        assert_eq!(
            thread.turns[0].items,
            vec![
                context_compaction_item("compact-1", "compact marker summary", None),
                agent_message_item("summary-1", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "Project Understanding",
                    "# Project Understanding\n\nProject body"
                ),
                agent_message_item("suffix-1", "post compact suffix"),
            ]
        );
    }

    #[test]
    fn restore_persisted_display_turns_from_rollout_items_restores_compaction_replacement_display()
    {
        let mut thread = thread_with_turns(Vec::new());
        let rollout_items = vec![
            RolloutItem::Compacted(CompactedItem {
                message: "compact marker summary".to_string(),
                replacement_history: Some(vec![ResponseItem::Message {
                    id: None,
                    role: "assistant".to_string(),
                    content: vec![ContentItem::OutputText {
                        text: "compact summary body".to_string(),
                    }],
                    phase: None,
                }]),
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
                context_compaction_item("item-1", "compact marker summary", None),
                agent_message_item("replacement-0", "compact summary body"),
                injected_context_item_with_label(
                    "ctx-1",
                    "User Preferences",
                    "# User Preferences\n\nProject body"
                ),
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
