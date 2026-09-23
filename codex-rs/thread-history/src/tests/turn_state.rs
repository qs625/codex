use super::*;

#[test]
fn patch_apply_begin_updates_active_turn_snapshot_with_file_change() {
    let turn_id = "turn-1";
    let mut builder = ThreadHistoryBuilder::new();
    let events = vec![
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: turn_id.to_string(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "apply patch".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
            call_id: "patch-call".into(),
            turn_id: turn_id.to_string(),
            auto_approved: false,
            changes: [(
                PathBuf::from("README.md"),
                protocol::protocol::FileChange::Add {
                    content: "hello\n".into(),
                },
            )]
            .into_iter()
            .collect(),
        }),
    ];

    for event in &events {
        builder.handle_event(event);
    }

    let snapshot = builder
        .active_turn_snapshot()
        .expect("active turn snapshot");
    assert_eq!(snapshot.id, turn_id);
    assert_eq!(snapshot.status, TurnStatus::InProgress);
    assert_eq!(
        snapshot.items,
        vec![
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "apply patch".into(),
                    text_elements: Vec::new(),
                }],
            },
            ThreadItem::FileChange {
                id: "patch-call".into(),
                changes: vec![FileUpdateChange {
                    path: "README.md".into(),
                    kind: PatchChangeKind::Add,
                    diff: "hello\n".into(),
                }],
                status: PatchApplyStatus::InProgress,
            },
        ]
    );
}

#[test]
fn apply_patch_approval_request_updates_active_turn_snapshot_with_file_change() {
    let turn_id = "turn-1";
    let mut builder = ThreadHistoryBuilder::new();
    let events = vec![
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: turn_id.to_string(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "apply patch".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
            call_id: "patch-call".into(),
            turn_id: turn_id.to_string(),
            started_at_ms: 0,
            changes: [(
                PathBuf::from("README.md"),
                protocol::protocol::FileChange::Add {
                    content: "hello\n".into(),
                },
            )]
            .into_iter()
            .collect(),
            reason: None,
            grant_root: None,
        }),
    ];

    for event in &events {
        builder.handle_event(event);
    }

    let snapshot = builder
        .active_turn_snapshot()
        .expect("active turn snapshot");
    assert_eq!(snapshot.id, turn_id);
    assert_eq!(snapshot.status, TurnStatus::InProgress);
    assert_eq!(
        snapshot.items,
        vec![
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "apply patch".into(),
                    text_elements: Vec::new(),
                }],
            },
            ThreadItem::FileChange {
                id: "patch-call".into(),
                changes: vec![FileUpdateChange {
                    path: "README.md".into(),
                    kind: PatchChangeKind::Add,
                    diff: "hello\n".into(),
                }],
                status: PatchApplyStatus::InProgress,
            },
        ]
    );
}

#[test]
fn late_turn_complete_does_not_close_active_turn() {
    let events = vec![
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-a".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "first".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-a".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        }),
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-b".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "second".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-a".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        }),
        EventMsg::AgentMessage(AgentMessageEvent {
            message: "still in b".into(),
            phase: None,
            memory_citation: None,
        }),
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-b".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        }),
    ];

    let items = events
        .into_iter()
        .map(RolloutItem::EventMsg)
        .collect::<Vec<_>>();
    let turns = build_turns_from_rollout_items(&items);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].id, "turn-a");
    assert_eq!(turns[1].id, "turn-b");
    assert_eq!(turns[1].items.len(), 2);
}

#[test]
fn late_turn_aborted_does_not_interrupt_active_turn() {
    let events = vec![
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-a".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "first".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-a".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        }),
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-b".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        }),
        EventMsg::UserMessage(UserMessageEvent {
            message: "second".into(),
            images: None,
            text_elements: Vec::new(),
            local_images: Vec::new(),
            skills: Vec::new(),
        }),
        EventMsg::TurnAborted(TurnAbortedEvent {
            turn_id: Some("turn-a".into()),
            reason: TurnAbortReason::Replaced,
            completed_at: None,
            duration_ms: None,
        }),
        EventMsg::AgentMessage(AgentMessageEvent {
            message: "still in b".into(),
            phase: None,
            memory_citation: None,
        }),
    ];

    let items = events
        .into_iter()
        .map(RolloutItem::EventMsg)
        .collect::<Vec<_>>();
    let turns = build_turns_from_rollout_items(&items);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].id, "turn-a");
    assert_eq!(turns[1].id, "turn-b");
    assert_eq!(turns[1].status, TurnStatus::InProgress);
    assert_eq!(turns[1].items.len(), 2);
}

#[test]
fn preserves_compaction_only_turn() {
    let items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::Compacted(CompactedItem {
            message: String::new(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-compact".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);
    assert_eq!(
        turns,
        vec![Turn {
            id: "turn-compact".into(),
            status: TurnStatus::Completed,
            error: None,
            started_at: None,
            completed_at: None,
            duration_ms: None,
            items_view: TurnItemsView::Full,
            items: vec![ThreadItem::ContextCompaction {
                id: "item-1".into(),
                summary: None,
                replacement_history: None,
            }],
        }]
    );
}

#[test]
fn compact_head_projects_summary_and_init_context_as_flat_items() {
    let items = vec![
        RolloutItem::Compacted(CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
                .expect("valid thread id"),
            turn_id: "turn-after-compact".into(),
            item: CoreTurnItem::InjectedContext(CoreInjectedContextItem {
                id: "ctx-1".into(),
                title: "Init Context".into(),
                preview: "Permissions, Environment".into(),
                sections: vec![
                    CoreInjectedContextSection {
                        label: "Permissions".into(),
                        text: "danger-full-access".into(),
                    },
                    CoreInjectedContextSection {
                        label: "Environment".into(),
                        text: "cwd=/tmp/project".into(),
                    },
                ],
            }),
            completed_at_ms: 1,
        })),
        RolloutItem::TurnContext(turn_context_item_with_id("turn-after-compact")),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].items.len(), 3);
    assert!(matches!(
        &turns[0].items[0],
        ThreadItem::ContextCompaction {
            summary,
            replacement_history,
            ..
        } if summary.is_none() && replacement_history.is_none()
    ));
    assert!(matches!(
        &turns[0].items[1],
        ThreadItem::AgentMessage { id, text, .. }
            if id == "item-1:summary" && text == "summary"
    ));
    assert!(matches!(
        &turns[0].items[2],
        ThreadItem::InjectedContext {
            title, sections, ..
        } if title == "Init Context"
            && sections.iter().map(|section| section.label.as_str()).collect::<Vec<_>>()
                == vec!["Permissions", "Environment"]
    ));
}

#[test]
fn compact_head_deduplicates_repeated_flat_init_context_completion() {
    let items = vec![
        RolloutItem::Compacted(CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
                .expect("valid thread id"),
            turn_id: "turn-after-compact".into(),
            item: CoreTurnItem::InjectedContext(CoreInjectedContextItem {
                id: "ctx-1".into(),
                title: "Init Context".into(),
                preview: "Permissions".into(),
                sections: vec![CoreInjectedContextSection {
                    label: "Permissions".into(),
                    text: "danger-full-access".into(),
                }],
            }),
            completed_at_ms: 1,
        })),
        RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
                .expect("valid thread id"),
            turn_id: "turn-after-compact".into(),
            item: CoreTurnItem::InjectedContext(CoreInjectedContextItem {
                id: "ctx-1".into(),
                title: "Init Context".into(),
                preview: "Permissions".into(),
                sections: vec![CoreInjectedContextSection {
                    label: "Permissions".into(),
                    text: "danger-full-access".into(),
                }],
            }),
            completed_at_ms: 2,
        })),
        RolloutItem::TurnContext(turn_context_item_with_id("turn-after-compact")),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].items.len(), 3);
    assert_eq!(
        turns[0]
            .items
            .iter()
            .filter(|item| matches!(item, ThreadItem::InjectedContext { .. }))
            .count(),
        1
    );
}

#[test]
fn init_context_after_compact_turn_context_stays_top_level() {
    let items = vec![
        RolloutItem::Compacted(CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        RolloutItem::TurnContext(turn_context_item_with_id("turn-after-compact")),
        RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
                .expect("valid thread id"),
            turn_id: "turn-after-compact".into(),
            item: CoreTurnItem::InjectedContext(CoreInjectedContextItem {
                id: "ctx-1".into(),
                title: "Init Context".into(),
                preview: "Permissions".into(),
                sections: vec![CoreInjectedContextSection {
                    label: "Permissions".into(),
                    text: "danger-full-access".into(),
                }],
            }),
            completed_at_ms: 1,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 2);
    assert!(matches!(
        &turns[0].items[0],
        ThreadItem::ContextCompaction {
            summary,
            replacement_history,
            ..
        } if summary.is_none() && replacement_history.is_none()
    ));
    assert!(matches!(
        &turns[1].items[0],
        ThreadItem::InjectedContext { title, .. } if title == "Init Context"
    ));
}

#[test]
fn checkpoint_compaction_prompt_marks_boundary_without_hiding_summary() {
    let items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-before".into(),
            started_at: Some(1),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
            message: "previous visible answer".into(),
            phase: None,
            memory_citation: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-before".into(),
            last_agent_message: Some("previous visible answer".into()),
            completed_at: Some(2),
            duration_ms: Some(1000),
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: Some(3),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "developer".into(),
            content: vec![ContentItem::InputText {
                text: "你正在为 my-codex 项目执行 CONTEXT CHECKPOINT COMPACTION。".into(),
            }],
            phase: None,
        }),
        RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
            message: "## Current Goal\n\n- compact summary".into(),
            phase: None,
            memory_citation: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-compact".into(),
            last_agent_message: Some("## Current Goal\n\n- compact summary".into()),
            completed_at: Some(4),
            duration_ms: Some(1000),
            time_to_first_token_ms: None,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[1].items,
        vec![
            ThreadItem::ContextCompaction {
                id: "item-2".into(),
                summary: None,
                replacement_history: None,
            },
            ThreadItem::AgentMessage {
                id: "item-3".into(),
                text: "## Current Goal\n\n- compact summary".into(),
                phase: None,
                memory_citation: None,
            },
        ]
    );
}

#[test]
fn checkpoint_compaction_prompt_before_turn_start_uses_summary_turn_boundary() {
    let items = vec![
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "developer".into(),
            content: vec![ContentItem::InputText {
                text: "CONTEXT CHECKPOINT COMPACTION".into(),
            }],
            phase: None,
        }),
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: Some(3),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
            message: "compact summary".into(),
            phase: None,
            memory_citation: None,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].id, "turn-compact");
    assert_eq!(
        turns[0].items,
        vec![
            ThreadItem::ContextCompaction {
                id: "item-1".into(),
                summary: None,
                replacement_history: None,
            },
            ThreadItem::AgentMessage {
                id: "item-2".into(),
                text: "compact summary".into(),
                phase: None,
                memory_citation: None,
            },
        ]
    );
}

#[test]
fn checkpoint_compaction_prompt_without_summary_does_not_mark_boundary() {
    let items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: Some(3),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "developer".into(),
            content: vec![ContentItem::InputText {
                text: "CONTEXT CHECKPOINT COMPACTION".into(),
            }],
            phase: None,
        }),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-compact".into(),
            last_agent_message: None,
            completed_at: Some(4),
            duration_ms: Some(1000),
            time_to_first_token_ms: None,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].id, "turn-compact");
    assert!(
        turns[0]
            .items
            .iter()
            .all(|item| !matches!(item, ThreadItem::ContextCompaction { .. }))
    );
}

#[test]
fn checkpoint_compaction_summary_boundary_preserves_flat_summary() {
    let items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: Some(3),
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: "developer".into(),
            content: vec![ContentItem::InputText {
                text: "CONTEXT CHECKPOINT COMPACTION".into(),
            }],
            phase: None,
        }),
        RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
            message: "compact summary".into(),
            phase: None,
            memory_citation: None,
        })),
        RolloutItem::Compacted(CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items,
        vec![
            ThreadItem::ContextCompaction {
                id: "item-1".into(),
                summary: None,
                replacement_history: None,
            },
            ThreadItem::AgentMessage {
                id: "item-1:summary".into(),
                text: "summary".into(),
                phase: None,
                memory_citation: None,
            },
            ThreadItem::AgentMessage {
                id: "item-2".into(),
                text: "compact summary".into(),
                phase: None,
                memory_citation: None,
            },
        ]
    );
}

#[test]
fn compact_marker_remains_single_marker_when_context_compacted_repeats() {
    let items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
            turn_id: "turn-compact".into(),
            started_at: None,
            model_context_window: None,
            collaboration_mode_kind: Default::default(),
        })),
        RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        RolloutItem::Compacted(CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::ContextCompacted(ContextCompactedEvent {})),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-compact".into(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ];

    let turns = build_turns_from_rollout_items(&items);

    assert_eq!(
        turns,
        vec![Turn {
            id: "turn-compact".into(),
            status: TurnStatus::Completed,
            error: None,
            started_at: None,
            completed_at: None,
            duration_ms: None,
            items_view: TurnItemsView::Full,
            items: vec![
                ThreadItem::ContextCompaction {
                    id: "item-1".into(),
                    summary: None,
                    replacement_history: None,
                },
                ThreadItem::AgentMessage {
                    id: "item-1:summary".into(),
                    text: "summary".into(),
                    phase: None,
                    memory_citation: None,
                },
            ],
        }]
    );
}
