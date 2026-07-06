use super::*;

    #[test]
    fn builds_multiple_turns_with_reasoning_items() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "First turn".into(),
                images: Some(vec!["https://example.com/one.png".into()]),
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Hi there".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::AgentReasoning(AgentReasoningEvent {
                text: "thinking".into(),
            }),
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent {
                text: "full reasoning".into(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "Second turn".into(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "Reply two".into(),
                phase: None,
                memory_citation: None,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();
        assert_eq!(turns.len(), 2);

        let first = &turns[0];
        assert!(Uuid::parse_str(&first.id).is_ok());
        assert_eq!(first.status, TurnStatus::Completed);
        assert_eq!(first.items.len(), 3);
        assert_eq!(
            first.items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![
                    UserInput::Text {
                        text: "First turn".into(),
                        text_elements: Vec::new(),
                    },
                    UserInput::Image {
                        url: "https://example.com/one.png".into(),
                    }
                ],
            }
        );
        assert_eq!(
            first.items[1],
            ThreadItem::AgentMessage {
                id: "item-2".into(),
                text: "Hi there".into(),
                phase: None,
                memory_citation: None,
            }
        );
        assert_eq!(
            first.items[2],
            ThreadItem::Reasoning {
                id: "item-3".into(),
                summary: vec!["thinking".into()],
                content: vec!["full reasoning".into()],
            }
        );

        let second = &turns[1];
        assert!(Uuid::parse_str(&second.id).is_ok());
        assert_ne!(first.id, second.id);
        assert_eq!(second.items.len(), 2);
        assert_eq!(
            second.items[0],
            ThreadItem::UserMessage {
                id: "item-4".into(),
                content: vec![UserInput::Text {
                    text: "Second turn".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
        assert_eq!(
            second.items[1],
            ThreadItem::AgentMessage {
                id: "item-5".into(),
                text: "Reply two".into(),
                phase: None,
                memory_citation: None,
            }
        );
    }

    #[test]
    fn live_inter_agent_json_message_is_not_displayed_as_agent_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "done".into(),
            InterAgentOperation::SendMessage,
        )
        .with_trigger_turn(false);
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: text,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn live_child_completion_json_message_is_not_displayed_as_agent_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let text = serde_json::to_string(&communication).expect("serialize communication");
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: text,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn live_event_driven_tool_marker_is_not_displayed_as_agent_message() {
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: "<event_driven_tool>{\"tool\":\"process_exit_subscribe\",\"title\":\"Process exited\",\"text\":\"[Process exit subscription] Session 42 exited with code 0\"}</event_driven_tool>".into(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns, Vec::new());
    }

    #[test]
    fn maps_typed_event_driven_tool_completed_to_event_item() {
        let thread_id = ThreadId::new();
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id,
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventDrivenTool(CoreEventDrivenToolItem {
                    id: "event-1".into(),
                    tool: "process_exit_subscribe".into(),
                    title: "Process exited".into(),
                    text: "[Process exit subscription] Session 42 exited with code 0".into(),
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenTool {
                id: "event-1".into(),
                tool: "process_exit_subscribe".into(),
                title: "Process exited".into(),
                text: "[Process exit subscription] Session 42 exited with code 0".into(),
            }]
        );
    }

    #[test]
    fn maps_typed_agent_message_completed_to_agent_message() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::AgentMessage(CoreAgentMessageItem {
                    id: "agent-1".into(),
                    content: vec![CoreAgentMessageContent::Text {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "agent-1".into(),
                text: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn dedupes_typed_agent_message_completed_before_raw_response_item() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::AgentMessage(CoreAgentMessageItem {
                    id: "msg-1".into(),
                    content: vec![CoreAgentMessageContent::Text {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                    memory_citation: None,
                }),
                completed_at_ms: 123,
            }),
            EventMsg::RawResponseItem(protocol::protocol::RawResponseItemEvent {
                item: ResponseItem::Message {
                    id: Some("msg-1".into()),
                    role: "assistant".into(),
                    content: vec![ContentItem::OutputText {
                        text: "final answer".into(),
                    }],
                    phase: Some(CoreMessagePhase::FinalAnswer),
                },
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "msg-1".into(),
                text: "final answer".into(),
                phase: Some(CoreMessagePhase::FinalAnswer),
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn maps_typed_child_completion_completed_to_collab_status_update() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_typed_child_completion_without_turn_lifecycle() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(false)
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![RolloutItem::EventMsg(EventMsg::ItemCompleted(
            ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            },
        ))];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-1");
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_typed_child_completion_before_turn_started_without_duplicate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-1".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-1");
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_child_completion_inside_active_parent_turn_without_stealing_followup_items() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "parent continues".into(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "parent-turn".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::CollabAgentStatusUpdate {
                    id: "collab-1".into(),
                    sender_thread_id: None,
                    sender_path: "/root/worker".into(),
                    recipient_thread_id: None,
                    recipient_path: "/root".into(),
                    status: CollabAgentState {
                        path: Some("/root/worker".into()),
                        status: CollabAgentStatus::Completed,
                        message: Some("completed".into()),
                    },
                },
                ThreadItem::AgentMessage {
                    id: "item-1".into(),
                    text: "parent continues".into(),
                    phase: None,
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn restores_child_completion_after_aborted_parent_turn_as_separate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id: Some("parent-turn".into()),
                reason: TurnAbortReason::Interrupted,
                completed_at: None,
                duration_ms: None,
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(turns[0].status, TurnStatus::Interrupted);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
        assert_eq!(turns[1].id, "child-completion-turn");
        assert_eq!(turns[1].status, TurnStatus::Completed);
        assert_eq!(
            turns[1].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn restores_child_completion_after_failed_parent_turn_as_separate_turn() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".into(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "completed".into(),
        )));
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "parent-turn".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::Error(ErrorEvent {
                message: "stream failure".into(),
                codex_error_info: Some(CodexErrorInfo::ResponseStreamDisconnected {
                    http_status_code: Some(502),
                }),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "child-completion-turn".into(),
                item: CoreTurnItem::CollabAgentMessage(CoreCollabAgentMessageItem {
                    id: "collab-1".into(),
                    communication,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "parent-turn");
        assert_eq!(turns[0].status, TurnStatus::Failed);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
        assert_eq!(turns[1].id, "child-completion-turn");
        assert_eq!(turns[1].status, TurnStatus::Completed);
        assert_eq!(
            turns[1].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "collab-1".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    status: CollabAgentStatus::Completed,
                    message: Some("completed".into()),
                },
            }]
        );
    }

