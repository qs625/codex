use super::*;

    #[test]
    fn reconstructs_tool_items_from_persisted_completion_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run tools".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::WebSearchEnd(WebSearchEndEvent {
                call_id: "search-1".into(),
                query: "codex".into(),
                action: CoreWebSearchAction::Search {
                    query: Some("codex".into()),
                    queries: None,
                },
            }),
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-1".into(),
                process_id: Some("pid-1".into()),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "hello world".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo hello world".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: String::new(),
                stderr: String::new(),
                aggregated_output: "hello world\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(12),
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Completed,
            }),
            EventMsg::McpToolCallEnd(McpToolCallEndEvent {
                call_id: "mcp-1".into(),
                invocation: McpInvocation {
                    server: "docs".into(),
                    tool: "lookup".into(),
                    arguments: Some(serde_json::json!({"id":"123"})),
                },
                mcp_app_resource_uri: None,
                duration: Duration::from_millis(8),
                result: Err("boom".into()),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 4);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::WebSearch {
                id: "search-1".into(),
                query: "codex".into(),
                action: Some(WebSearchAction::Search {
                    query: Some("codex".into()),
                    queries: None,
                }),
            }
        );
        assert_eq!(
            turns[0].items[2],
            ThreadItem::CommandExecution {
                id: "exec-1".into(),
                command: "echo 'hello world'".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-1".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "echo hello world".into(),
                }],
                aggregated_output: Some("hello world\n".into()),
                exit_code: Some(0),
                duration_ms: Some(12),
            }
        );
        assert_eq!(
            turns[0].items[3],
            ThreadItem::McpToolCall {
                id: "mcp-1".into(),
                server: "docs".into(),
                tool: "lookup".into(),
                status: McpToolCallStatus::Failed,
                arguments: serde_json::json!({"id":"123"}),
                mcp_app_resource_uri: None,
                result: None,
                error: Some(McpToolCallError {
                    message: "boom".into(),
                }),
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn inline_completed_exec_replay_does_not_create_exit_notification_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(completed_exec_event("exec-inline", None)),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CommandExecution {
                id: "exec-inline".into(),
                command: "echo done".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::UnifiedExecStartup,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: Some(1000),
                notify_on: Some(CommandExecutionNotifyOn::Exit),
                command_actions: vec![CommandAction::Unknown {
                    command: "echo done".into(),
                }],
                aggregated_output: Some("done\n".into()),
                exit_code: Some(0),
                duration_ms: Some(5),
            }]
        );
    }

    #[test]
    fn replays_command_execution_notification_completed_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::CommandExecutionNotificationCompleted(
                CommandExecutionNotificationDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "exec-background:notification:exit".into(),
                    command_item_id: "exec-background".into(),
                    kind: protocol::models::CommandExecutionNotificationKind::Exit,
                    message: "Command exit notification received.".into(),
                    output: Some("done".into()),
                    exit_code: Some(0),
                    created_at_ms: 123,
                    completed_at_ms: 124,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::CommandExecutionNotification {
                id: "exec-background:notification:exit".into(),
                command_item_id: "exec-background".into(),
                kind: CommandExecutionNotificationKind::Exit,
                message: "Command exit notification received.".into(),
                output: Some("done".into()),
                exit_code: Some(0),
                created_at_ms: 123,
            }
        );
    }

    #[test]
    fn reconstructs_mcp_tool_result_meta_from_persisted_completion_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::McpToolCallEnd(McpToolCallEndEvent {
                call_id: "mcp-1".into(),
                invocation: McpInvocation {
                    server: "docs".into(),
                    tool: "lookup".into(),
                    arguments: Some(serde_json::json!({"id":"123"})),
                },
                mcp_app_resource_uri: Some("ui://widget/lookup.html".into()),
                duration: Duration::from_millis(8),
                result: Ok(CallToolResult {
                    content: vec![serde_json::json!({
                        "type": "text",
                        "text": "result"
                    })],
                    structured_content: Some(serde_json::json!({"id":"123"})),
                    is_error: Some(false),
                    meta: Some(serde_json::json!({
                        "ui/resourceUri": "ui://widget/lookup.html"
                    })),
                }),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::McpToolCall {
                id: "mcp-1".into(),
                server: "docs".into(),
                tool: "lookup".into(),
                status: McpToolCallStatus::Completed,
                arguments: serde_json::json!({"id":"123"}),
                mcp_app_resource_uri: Some("ui://widget/lookup.html".into()),
                result: Some(Box::new(McpToolCallResult {
                    content: vec![serde_json::json!({
                        "type": "text",
                        "text": "result"
                    })],
                    structured_content: Some(serde_json::json!({"id":"123"})),
                    meta: Some(serde_json::json!({
                        "ui/resourceUri": "ui://widget/lookup.html"
                    })),
                })),
                error: None,
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn reconstructs_dynamic_tool_items_from_request_and_response_events() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run dynamic tool".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::DynamicToolCallRequest(protocol::dynamic_tools::DynamicToolCallRequest {
                call_id: "dyn-1".into(),
                turn_id: "turn-1".into(),
                started_at_ms: 0,
                namespace: Some("codex_app".into()),
                tool: "lookup_ticket".into(),
                arguments: serde_json::json!({"id":"ABC-123"}),
            }),
            EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
                call_id: "dyn-1".into(),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                namespace: Some("codex_app".into()),
                tool: "lookup_ticket".into(),
                arguments: serde_json::json!({"id":"ABC-123"}),
                content_items: vec![CoreDynamicToolCallOutputContentItem::InputText {
                    text: "Ticket is open".into(),
                }],
                success: true,
                error: None,
                duration: Duration::from_millis(42),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::DynamicToolCall {
                id: "dyn-1".into(),
                namespace: Some("codex_app".into()),
                tool: "lookup_ticket".into(),
                arguments: serde_json::json!({"id":"ABC-123"}),
                status: DynamicToolCallStatus::Completed,
                content_items: Some(vec![DynamicToolCallOutputContentItem::InputText {
                    text: "Ticket is open".into(),
                }]),
                success: Some(true),
                duration_ms: Some(42),
            }
        );
    }

    #[test]
    fn typed_event_command_event_history_uses_typed_item_id() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".into(),
            kind: protocol::event_command::EventCommandEventKind::Exited,
            label: Some("tests".into()),
            command: "cargo test".into(),
            cwd: None,
            line: None,
            sequence: None,
            exit_code: Some(0),
            signal: None,
            message: Some("done".into()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::EventCommandEventCompleted(
                protocol::protocol::EventCommandDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "typed-event-command".into(),
                    event,
                    completed_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventCommandEvent {
                id: "typed-event-command".into(),
                subscription_id: "sub-1".into(),
                kind: app_server_protocol::EventCommandEventKind::Exited,
                label: Some("tests".into()),
                command: "cargo test".into(),
                cwd: None,
                line: None,
                sequence: None,
                exit_code: Some(0),
                signal: None,
                message: Some("done".into()),
                truncated: false,
                created_at: 1_700_000_000,
            }]
        );
    }

    #[test]
    fn legacy_event_command_completed_history_rebuilds_thread_item() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".into(),
            kind: protocol::event_command::EventCommandEventKind::Exited,
            label: Some("tests".into()),
            command: "cargo test".into(),
            cwd: None,
            line: None,
            sequence: Some(3),
            exit_code: Some(0),
            signal: None,
            message: Some("done".into()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventCommandEvent(CoreEventCommandEventItem {
                    id: "event-command-1".into(),
                    event,
                }),
                completed_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventCommandEvent {
                id: "event-command-1".into(),
                subscription_id: "sub-1".into(),
                kind: app_server_protocol::EventCommandEventKind::Exited,
                label: Some("tests".into()),
                command: "cargo test".into(),
                cwd: None,
                line: None,
                sequence: Some(3),
                exit_code: Some(0),
                signal: None,
                message: Some("done".into()),
                truncated: false,
                created_at: 1_700_000_000,
            }]
        );
    }

    #[test]
    fn legacy_event_command_started_history_does_not_create_completed_item() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: CoreTurnItem::EventCommandEvent(CoreEventCommandEventItem {
                    id: "event-command-1".into(),
                    event: EventCommandEvent {
                        subscription_id: "sub-1".into(),
                        kind: protocol::event_command::EventCommandEventKind::Output,
                        label: Some("tests".into()),
                        command: "cargo test".into(),
                        cwd: None,
                        line: Some("running".into()),
                        sequence: Some(3),
                        exit_code: None,
                        signal: None,
                        message: None,
                        truncated: false,
                        created_at: 1_700_000_000,
                    },
                }),
                started_at_ms: 123,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn typed_event_driven_tool_history_rebuilds_trigger_item() {
        let trigger = EventDrivenToolTrigger {
            tool: "fs_subscribe".into(),
            title: "File watch triggered".into(),
            text: "build.log changed".into(),
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::EventDrivenToolCompleted(
                protocol::protocol::EventDrivenToolDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "typed-trigger".into(),
                    trigger,
                    completed_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenTool {
                id: "typed-trigger".into(),
                tool: "fs_subscribe".into(),
                title: "File watch triggered".into(),
                text: "build.log changed".into(),
            }]
        );
    }

