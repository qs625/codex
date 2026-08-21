use super::*;

    #[test]
    fn keeps_unmarked_event_driven_tool_json_as_agent_message() {
        let message = serde_json::json!({
            "tool": "process_exit_subscribe",
            "title": "Process exited",
            "text": "[Process exit subscription] Session 42 exited with code 0",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn filters_malformed_event_driven_tool_marker_as_agent_message() {
        let message = concat!(
            "<event_driven_tool>",
            r#"{"tool":"process_exit_subscribe","title":"Process exited""#,
            "</event_driven_tool>"
        )
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert!(turns.is_empty());
    }

    #[test]
    fn keeps_unknown_inter_agent_shaped_json_as_agent_message() {
        let message = serde_json::json!({
            "author": "/root/worker",
            "recipient": "/root",
            "content": "plain assistant json",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn keeps_ordinary_json_with_operation_as_agent_message() {
        let message = serde_json::json!({
            "author": "/root/worker",
            "recipient": "/root",
            "content": "plain assistant json",
            "operation": "sendMessage",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn filters_raw_inter_agent_envelope_with_unknown_or_null_operation() {
        for operation in [
            serde_json::Value::Null,
            serde_json::Value::String("mysteryOperation".to_string()),
            serde_json::Value::Number(1.into()),
        ] {
            let message = serde_json::json!({
                "author": "/root/worker",
                "recipient": "/root",
                "content": "legacy message",
                "operation": operation,
                "content_parts": [],
                "trigger_turn": false,
            })
            .to_string();
            let events = [EventMsg::AgentMessage(AgentMessageEvent {
                message,
                phase: None,
                memory_citation: None,
            })];

            let mut builder = ThreadHistoryBuilder::new();
            for event in &events {
                builder.handle_event(event);
            }
            let turns = builder.finish();

            assert!(turns.is_empty());
        }
    }

    #[test]
    fn filters_nullable_raw_inter_agent_envelope_without_displaying_json() {
        let message = serde_json::json!({
            "author": "/cp_http_api/frontend_taskstatus_fix_2",
            "recipient": "/cp_http_api",
            "other_recipients": [],
            "content": "typecheck is available ...",
            "content_parts": [],
            "operation": null,
            "trigger_turn": false,
            "sender_thread_id": null,
            "recipient_thread_id": null,
            "status": null,
            "lifecycle_status": null,
            "agent_nickname": null,
            "agent_role": null,
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert!(turns.is_empty());
    }

    #[test]
    fn filters_status_message_raw_inter_agent_envelope_without_displaying_json() {
        let message = serde_json::json!({
            "author": "/cp_http_api/owner_infra",
            "recipient": "/cp_http_api",
            "other_recipients": [],
            "content": "status update",
            "content_parts": [],
            "operation": "message",
            "trigger_turn": false,
            "sender_thread_id": null,
            "recipient_thread_id": null,
            "status": {
                "in_progress": true
            },
            "lifecycle_status": null,
            "agent_nickname": null,
            "agent_role": null,
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message,
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert!(turns.is_empty());
    }

    #[test]
    fn filters_raw_command_execution_notification_envelope_without_displaying_json() {
        let message = serde_json::json!({
            "type": "command_execution_notification",
            "command_item_id": "call_OGVf4iCZPdZ19BGEqkwvD7UM",
            "kind": "exit",
            "message": "Command call_OGVf4iCZPdZ19BGEqkwvD7UM has exited with code 0.",
            "output": "cargo test: 3 passed, 206 filtered out (2 suites, 0.00s)\n",
            "exit_code": 0,
            "created_at_ms": 1787215776940_i64,
        })
        .to_string();

        for event in [
            EventMsg::AgentMessage(AgentMessageEvent {
                message: message.clone(),
                phase: None,
                memory_citation: None,
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: message.clone(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            }),
        ] {
            let mut builder = ThreadHistoryBuilder::new();
            builder.handle_event(&event);
            let turns = builder.finish();

            assert!(turns.is_empty());
        }
    }

    #[test]
    fn preserves_command_execution_notification_like_json_as_agent_message() {
        let message = serde_json::json!({
            "type": "command_execution_notification",
            "note": "this is documentation",
        })
        .to_string();
        let events = [EventMsg::AgentMessage(AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: message,
                phase: None,
                memory_citation: None,
            }]
        );
    }

    #[test]
    fn filters_raw_send_message_envelope_as_agent_message() {
        for operation in ["sendMessage", "send_message"] {
            let message = serde_json::json!({
                "author": "/root/worker",
                "recipient": "/root",
                "content": "legacy message",
                "operation": operation,
                "content_parts": [],
                "trigger_turn": false,
            })
            .to_string();
            let events = [EventMsg::AgentMessage(AgentMessageEvent {
                message,
                phase: None,
                memory_citation: None,
            })];

            let mut builder = ThreadHistoryBuilder::new();
            for event in &events {
                builder.handle_event(event);
            }
            let turns = builder.finish();

            assert!(turns.is_empty());
        }
    }

    #[test]
    fn preserves_loaded_skills_in_user_message_history() {
        let skill_path = test_path_buf("/tmp/skills/demo/SKILL.md");
        let events = vec![EventMsg::UserMessage(UserMessageEvent {
            message: "Use the selected skill.".into(),
            images: None,
            local_images: Vec::new(),
            skills: vec![UserMessageSkill {
                name: "demo".into(),
                path: skill_path.clone(),
            }],
            text_elements: Vec::new(),
        })];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }

        let turns = builder.finish();

        assert_eq!(
            turns[0].items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![
                    UserInput::Skill {
                        name: "demo".into(),
                        path: skill_path,
                    },
                    UserInput::Text {
                        text: "Use the selected skill.".into(),
                        text_elements: Vec::new(),
                    },
                ],
            }
        );
    }

    #[test]
    fn ignores_non_plan_item_lifecycle_events() {
        let turn_id = "turn-1";
        let thread_id = ThreadId::new();
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
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
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id,
                turn_id: turn_id.to_string(),
                item: CoreTurnItem::UserMessage(CoreUserMessageItem {
                    id: "user-item-id".to_string(),
                    content: Vec::new(),
                }),
                started_at_ms: 0,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: turn_id.to_string(),
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
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
    }

    #[test]
    fn preserves_agent_message_phase_in_history() {
        let events = vec![EventMsg::AgentMessage(AgentMessageEvent {
            message: "Final reply".into(),
            phase: Some(CoreMessagePhase::FinalAnswer),
            memory_citation: None,
        })];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::AgentMessage {
                id: "item-1".into(),
                text: "Final reply".into(),
                phase: Some(MessagePhase::FinalAnswer),
                memory_citation: None,
            }
        );
    }
