use super::*;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::DynamicToolCallStatus;
use protocol::subscriptions::PersistedSubscription;

pub(super) fn restore_persisted_display_turns(thread: &mut Thread, persisted_turns: &[Turn]) {
    restore_persisted_injected_context_turns(thread, persisted_turns);
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

pub(super) fn apply_runtime_activity_items_from_persisted_turns(thread: &mut Thread) {
    let persisted_turns = thread.turns.clone();
    apply_runtime_activity_items_from_turns(thread, &persisted_turns);
}

pub(super) fn apply_live_active_command_items_from_active_turn(
    thread: &mut Thread,
    active_turn: Option<&Turn>,
) {
    let active_command_items = active_turn
        .map(active_command_items_from_live_turn)
        .unwrap_or_default();
    thread.active_command_items = Some(active_command_items);
}

fn active_command_items_from_live_turn(turn: &Turn) -> Vec<ThreadItem> {
    turn.items
        .iter()
        .filter_map(|item| match item {
            ThreadItem::CommandExecution {
                status: CommandExecutionStatus::InProgress,
                ..
            } => Some(item.clone()),
            _ => None,
        })
        .collect()
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
        ThreadItem::InjectedContext {
            id: id.to_string(),
            title: "Init Context".to_string(),
            preview: "Init Context".to_string(),
            sections: vec![InjectedContextSection {
                label: "Instructions".to_string(),
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
