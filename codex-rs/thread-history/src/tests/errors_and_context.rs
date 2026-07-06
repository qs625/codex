use super::*;

    #[test]
    fn rollback_failed_error_does_not_mark_turn_failed() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::AgentMessage(AgentMessageEvent {
                message: "done".into(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::Error(ErrorEvent {
                message: "rollback failed".into(),
                codex_error_info: Some(CodexErrorInfo::ThreadRollbackFailed),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, TurnStatus::Completed);
        assert_eq!(turns[0].error, None);
    }

    #[test]
    fn out_of_turn_error_does_not_create_or_fail_a_turn() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
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
            EventMsg::Error(ErrorEvent {
                message: "request-level failure".into(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0],
            Turn {
                id: "turn-a".into(),
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
                items_view: TurnItemsView::Full,
                items: vec![ThreadItem::UserMessage {
                    id: "item-1".into(),
                    content: vec![UserInput::Text {
                        text: "hello".into(),
                        text_elements: Vec::new(),
                    }],
                }],
            }
        );
    }

    #[test]
    fn error_then_turn_complete_preserves_failed_status() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::Error(ErrorEvent {
                message: "stream failure".into(),
                codex_error_info: Some(CodexErrorInfo::ResponseStreamDisconnected {
                    http_status_code: Some(502),
                }),
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
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
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[0].status, TurnStatus::Failed);
        assert_eq!(
            turns[0].error,
            Some(TurnError {
                message: "stream failure".into(),
                codex_error_info: Some(
                    app_server_protocol::CodexErrorInfo::ResponseStreamDisconnected {
                        http_status_code: Some(502),
                    }
                ),
                additional_details: None,
            })
        );
    }

    #[test]
    fn ignores_plain_user_response_items_in_rollout_replay() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(protocol::models::ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "user".into(),
                content: vec![protocol::models::ContentItem::InputText {
                    text: "plain text".into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-a".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].items.is_empty());
    }

    #[test]
    fn ignores_initial_injected_context_response_items() {
        let items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![
                    ContentItem::InputText {
                        text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                            .into(),
                    },
                    ContentItem::InputText {
                        text: "<skills instructions>\n## Skills\n- skill-a\n</skills instructions>"
                            .into(),
                    },
                ],
                phase: None,
            }),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("user-context".into()),
                role: "user".into(),
                content: vec![ContentItem::InputText {
                    text: "<environment_context>\n  <cwd>/workspace</cwd>\n</environment_context>"
                        .into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert!(matches!(turns[0].items[0], ThreadItem::UserMessage { .. }));
    }

    #[test]
    fn replays_typed_injected_context_with_agent_file_instructions() {
        let items = vec![RolloutItem::EventMsg(EventMsg::ItemCompleted(
            ItemCompletedEvent {
                thread_id: ThreadId::default(),
                turn_id: "turn-a".into(),
                item: CoreTurnItem::InjectedContext(CoreInjectedContextItem {
                    id: "ctx-1".into(),
                    title: "Init Context".into(),
                    preview: "Developer".into(),
                    sections: vec![CoreInjectedContextSection {
                        label: "Developer".into(),
                        text: "Agent type file body".into(),
                    }],
                }),
                completed_at_ms: 1_000,
            },
        ))];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::InjectedContext {
                id: "ctx-1".into(),
                title: "Init Context".into(),
                preview: "Developer".into(),
                sections: vec![InjectedContextSection {
                    label: "Developer".into(),
                    text: "Agent type file body".into(),
                }],
            }]
        );
    }

    #[test]
    fn ignores_initial_injected_context_response_items_after_explicit_turn_start() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: ModeKind::Default,
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![ContentItem::InputText {
                    text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                        .into(),
                }],
                phase: None,
            }),
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "hello".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert!(matches!(turns[0].items[0], ThreadItem::UserMessage { .. }));
    }
