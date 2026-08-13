use super::*;

    #[test]
    fn typed_builtin_tool_history_rebuilds_thread_item() {
        let started_output = serde_json::json!({
            "initialTimeoutMs": 50,
            "currentTimeoutMs": 50,
            "hardCapTimeoutMs": 1000
        });
        let output = serde_json::json!({
            "timedOut": false,
            "sourceHint": "user_input",
            "waitedMs": 5,
            "initialTimeoutMs": 50,
            "currentTimeoutMs": 50,
            "hardCapTimeoutMs": 1000
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallStarted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-1".into(),
                    tool: "poll_event".into(),
                    arguments: serde_json::json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::InProgress,
                    output: Some(started_output),
                    lifecycle_at_ms: 100,
                },
            )),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-1".into(),
                    tool: "poll_event".into(),
                    arguments: serde_json::json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "builtin-1".into(),
                tool: "poll_event".into(),
                arguments: serde_json::json!({}),
                status: DynamicToolCallStatus::Completed,
                output: Some(output),
            }]
        );
    }

    #[test]
    fn typed_read_agent_builtin_tool_history_rebuilds_thread_item() {
        let output = serde_json::json!({
            "target": "/root/worker",
            "agentName": "/root/worker",
            "agentNickname": "Worker",
            "agentRole": "feature-owner",
            "lifecycleStatus": {
                "type": "final",
                "result": {
                    "type": "completed"
                }
            },
            "lastTaskMessage": "do work",
            "lastAgentMessage": "done"
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallStarted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "read-agent-1".into(),
                    tool: "read_agent".into(),
                    arguments: serde_json::json!({
                        "target": "/root/worker",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::InProgress,
                    output: None,
                    lifecycle_at_ms: 100,
                },
            )),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "read-agent-1".into(),
                    tool: "read_agent".into(),
                    arguments: serde_json::json!({
                        "target": "/root/worker",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "read-agent-1".into(),
                tool: "read_agent".into(),
                arguments: serde_json::json!({
                    "target": "/root/worker",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(output),
            }]
        );
    }

    #[test]
    fn typed_builtin_tool_started_history_keeps_poll_event_timeout_metadata() {
        let output = serde_json::json!({
            "initialTimeoutMs": 50,
            "currentTimeoutMs": 50,
            "hardCapTimeoutMs": 1000
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallStarted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-1".into(),
                    tool: "poll_event".into(),
                    arguments: serde_json::json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::InProgress,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 100,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "builtin-1".into(),
                tool: "poll_event".into(),
                arguments: serde_json::json!({}),
                status: DynamicToolCallStatus::InProgress,
                output: Some(output),
            }]
        );
    }

    #[test]
    fn typed_external_tool_history_rebuilds_event_driven_tool_item() {
        let arguments = serde_json::json!({ "pathPrefix": "/root/external" });
        let output = serde_json::json!({ "agents": [] });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::ExternalToolCallStarted(
                protocol::protocol::ExternalToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "external-1".into(),
                    tool: "list_external_agents".into(),
                    arguments: arguments.clone(),
                    status: protocol::protocol::ExternalToolCallStatus::InProgress,
                    output: None,
                    lifecycle_at_ms: 100,
                },
            )),
            RolloutItem::EventMsg(EventMsg::ExternalToolCallCompleted(
                protocol::protocol::ExternalToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "external-1".into(),
                    tool: "list_external_agents".into(),
                    arguments: arguments.clone(),
                    status: protocol::protocol::ExternalToolCallStatus::Completed,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenToolCall {
                id: "external-1".into(),
                tool: "list_external_agents".into(),
                arguments,
                status: DynamicToolCallStatus::Completed,
                output: Some(output),
            }]
        );
    }

    #[test]
    fn typed_workflow_progress_history_rebuilds_thread_item() {
        let event = protocol::models::WorkflowRunProgressEvent {
            run_id: "wf_1".into(),
            workflow_id: "feature-dev".into(),
            status: serde_json::json!("running"),
            runner_status: "control_plane_started".into(),
            kind: protocol::models::WorkflowRunProgressKind::Started,
            message: "workflow control run started".into(),
            updated_at: 1_700_000_000,
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::WorkflowRunProgressCompleted(
                protocol::protocol::WorkflowRunProgressDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "workflow-progress-1".into(),
                    event,
                    completed_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::WorkflowRunProgress {
                id: "workflow-progress-1".into(),
                event: app_server_protocol::ThreadWorkflowRunProgressEvent {
                    run_id: "wf_1".into(),
                    workflow_id: "feature-dev".into(),
                    status: serde_json::json!("running"),
                    runner_status: "control_plane_started".into(),
                    kind: app_server_protocol::ThreadWorkflowRunProgressKind::Started,
                    message: "workflow control run started".into(),
                    updated_at: 1_700_000_000,
                },
            }]
        );
    }

    #[test]
    fn typed_inter_agent_history_rebuilds_collab_item_without_agent_message_leak() {
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
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::InterAgentCommunicationCompleted(
                protocol::protocol::InterAgentCommunicationDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "typed-collab".into(),
                    communication,
                    completed_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CollabAgentStatusUpdate {
                id: "typed-collab".into(),
                sender_thread_id: None,
                sender_path: "/root/worker".into(),
                recipient_thread_id: None,
                recipient_path: "/root".into(),
                lifecycle_status: CollabAgentState {
                    path: Some("/root/worker".into()),
                    agent_nickname: None,
                    agent_role: None,
                    lifecycle_status: ThreadLifecycleStatus::completed(None),
                    message: Some("completed".into()),
                },
            }]
        );
    }

    #[test]
    fn typed_unknown_inter_agent_history_is_ignored() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "raw json should not leak".into(),
            InterAgentOperation::Unknown,
        );
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::InterAgentCommunicationCompleted(
                protocol::protocol::InterAgentCommunicationDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "typed-unknown-collab".into(),
                    communication,
                    completed_at_ms: 123,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn event_driven_tool_replay_does_not_override_specialized_items() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".into(),
                namespace: None,
                arguments: r#"{"cmd":"ls"}"#.into(),
                call_id: "call-1".into(),
            }),
            RolloutItem::EventMsg(EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "call-1".into(),
                process_id: Some("pid-1".into()),
                turn_id: "turn-1".into(),
                completed_at_ms: 0,
                command: vec!["ls".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown { cmd: "ls".into() }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: String::new(),
                stderr: String::new(),
                aggregated_output: String::new(),
                exit_code: 0,
                duration: Duration::ZERO,
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Completed,
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                call_id: "call-1".into(),
                output: FunctionCallOutputPayload::from_text("ok".into()),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::CommandExecution {
                id: "call-1".into(),
                command: "ls".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-1".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "ls".into(),
                }],
                aggregated_output: None,
                exit_code: Some(0),
                duration_ms: Some(0),
            }
        );
    }

    #[test]
    fn dedicated_display_event_ignores_legacy_response_item_fallback() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::CommandWait {
                id: None,
                command_id: "cmd-1".into(),
                status: protocol::models::CommandWaitStatus::Completed,
                notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
                exit_code: Some(0),
                wall_time_seconds: 1.25,
                wait_timeout_ms: 250,
                created_at_ms: 1234,
            }),
            RolloutItem::EventMsg(EventMsg::CommandWaitCompleted(
                protocol::protocol::CommandWaitDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "wait-1".into(),
                    command_id: "cmd-1".into(),
                    status: protocol::models::CommandWaitStatus::Completed,
                    notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                    lifecycle_at_ms: 5678,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CommandWait {
                id: "wait-1".into(),
                command_id: "cmd-1".into(),
                status: app_server_protocol::CommandWaitStatus::Completed,
                notification: Some(app_server_protocol::CommandWaitNotificationKind::Exit),
                exit_code: Some(0),
                wall_time_seconds: 1.25,
                wait_timeout_ms: 250,
                created_at_ms: 1234,
            }]
        );
    }

    #[test]
    fn dedicated_display_event_ignores_all_legacy_response_item_fallbacks() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::CommandWait {
                id: None,
                command_id: "cmd-1".into(),
                status: protocol::models::CommandWaitStatus::Completed,
                notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
                exit_code: Some(0),
                wall_time_seconds: 1.25,
                wait_timeout_ms: 250,
                created_at_ms: 1234,
            }),
            RolloutItem::ResponseItem(ResponseItem::CommandWriteStdin {
                id: None,
                command_id: "cmd-2".into(),
                bytes_written: 4,
                contains_newline: true,
                created_at_ms: 2234,
            }),
            RolloutItem::EventMsg(EventMsg::CommandWaitCompleted(
                protocol::protocol::CommandWaitDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "wait-1".into(),
                    command_id: "cmd-1".into(),
                    status: protocol::models::CommandWaitStatus::Completed,
                    notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                    lifecycle_at_ms: 5678,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::CommandWait {
                id: "wait-1".into(),
                command_id: "cmd-1".into(),
                status: app_server_protocol::CommandWaitStatus::Completed,
                notification: Some(app_server_protocol::CommandWaitNotificationKind::Exit),
                exit_code: Some(0),
                wall_time_seconds: 1.25,
                wait_timeout_ms: 250,
                created_at_ms: 1234,
            }]
        );
    }

    #[test]
    fn dedicated_display_event_ignores_repeated_legacy_response_item_fallbacks() {
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
            RolloutItem::ResponseItem(ResponseItem::EventDrivenTool {
                id: None,
                trigger: trigger.clone(),
            }),
            RolloutItem::ResponseItem(ResponseItem::EventDrivenTool {
                id: None,
                trigger: trigger.clone(),
            }),
            RolloutItem::EventMsg(EventMsg::EventDrivenToolCompleted(
                protocol::protocol::EventDrivenToolDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "trigger-dedicated".into(),
                    trigger,
                    completed_at_ms: 5678,
                },
            )),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::EventDrivenTool {
                id: "trigger-dedicated".into(),
                tool: "fs_subscribe".into(),
                title: "File watch triggered".into(),
                text: "build.log changed".into(),
            }]
        );
    }

    #[test]
    fn legacy_event_driven_tool_response_message_is_not_displayed_as_agent_message() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("process-exit-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "<event_driven_tool>{\"tool\":\"process_exit_subscribe\",\"title\":\"Process exited\",\"text\":\"Session 42 exited with code 0\"}</event_driven_tool>".into(),
                }],
                phase: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn active_schedule_subscription_metadata_rebuilds_builtin_monitor_item() {
        let items = vec![RolloutItem::SessionMeta(
            session_meta_with_schedule_subscription("sub-schedule"),
        )];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "active-subscription:sub-schedule".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({
                    "schedule": {
                        "kind": "every_interval",
                        "interval_ms": 60000,
                    },
                    "label": "standup",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "subscription_id": "sub-schedule",
                })),
            }]
        );
    }

    #[test]
    fn active_schedule_subscription_metadata_rebuilds_message_argument() {
        let items = vec![RolloutItem::SessionMeta(
            session_meta_with_schedule_subscription_message(
                "sub-schedule",
                "Clean checkout targets and report the result.",
            ),
        )];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "active-subscription:sub-schedule".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({
                    "schedule": {
                        "kind": "every_interval",
                        "interval_ms": 60000,
                    },
                    "label": "standup",
                    "message": "Clean checkout targets and report the result.",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "subscription_id": "sub-schedule",
                })),
            }]
        );
    }

    #[test]
    fn active_schedule_subscription_metadata_does_not_duplicate_history_item() {
        let output = serde_json::json!({
            "subscription_id": "sub-schedule",
            "next_fire_at": "2026-07-27T10:00:00Z",
            "schedule_summary": "Every 60 seconds",
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-1".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: serde_json::json!({
                        "schedule": {
                            "kind": "every_interval",
                            "interval_ms": 60000,
                        },
                        "label": "standup",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
            RolloutItem::SessionMeta(session_meta_with_schedule_subscription("sub-schedule")),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "builtin-1".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({
                    "schedule": {
                        "kind": "every_interval",
                        "interval_ms": 60000,
                    },
                    "label": "standup",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(output),
            }]
        );
    }

    #[test]
    fn active_schedule_subscription_metadata_before_compact_is_not_revived() {
        let items = vec![
            RolloutItem::SessionMeta(session_meta_with_schedule_subscription("sub-schedule")),
            RolloutItem::Compacted(CompactedItem {
                message: String::new(),
                replacement_history: None,
                visible_replacement_history_len: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert!(
            turns[0]
                .items
                .iter()
                .all(|item| {
                    !matches!(
                        item,
                        ThreadItem::BuiltinToolCall { tool, .. } if tool == "schedule_subscribe"
                    )
                })
        );
    }

    #[test]
    fn latest_empty_subscription_metadata_deactivates_historical_schedule_item() {
        let subscribe_output = serde_json::json!({
            "subscription_id": "sub-schedule",
            "next_fire_at": "2026-07-27T10:00:00Z",
            "schedule_summary": "Every 60 seconds",
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-subscribe".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: serde_json::json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(subscribe_output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
            RolloutItem::SessionMeta(session_meta_with_subscriptions(Vec::new())),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 2);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "builtin-subscribe".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({}),
                status: DynamicToolCallStatus::Completed,
                output: Some(subscribe_output),
            }]
        );
        assert_eq!(
            turns[1].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "active-subscription:sub-schedule:inactive".into(),
                tool: "schedule_unsubscribe".into(),
                arguments: serde_json::json!({
                    "subscription_id": "sub-schedule",
                }),
                status: DynamicToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "subscription_id": "sub-schedule",
                    "unsubscribed": true,
                })),
            }]
        );
    }

    #[test]
    fn subscription_metadata_none_does_not_deactivate_historical_schedule_item() {
        let subscribe_output = serde_json::json!({
            "subscription_id": "sub-schedule",
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "builtin-subscribe".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: serde_json::json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(subscribe_output.clone()),
                    lifecycle_at_ms: 123,
                },
            )),
            RolloutItem::SessionMeta(protocol::protocol::SessionMetaLine {
                meta: protocol::protocol::SessionMeta {
                    subscriptions: None,
                    ..Default::default()
                },
                git: None,
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::BuiltinToolCall {
                id: "builtin-subscribe".into(),
                tool: "schedule_subscribe".into(),
                arguments: serde_json::json!({}),
                status: DynamicToolCallStatus::Completed,
                output: Some(subscribe_output),
            }]
        );
    }

    fn session_meta_with_schedule_subscription(
        subscription_id: &str,
    ) -> protocol::protocol::SessionMetaLine {
        session_meta_with_schedule_subscription_message(subscription_id, "")
    }

    fn session_meta_with_schedule_subscription_message(
        subscription_id: &str,
        message: &str,
    ) -> protocol::protocol::SessionMetaLine {
        session_meta_with_subscriptions(vec![protocol::subscriptions::PersistedSubscription::Schedule {
            subscription_id: subscription_id.to_string(),
            schedule: protocol::subscriptions::ScheduleSpec::EveryInterval {
                interval_ms: 60_000,
            },
            label: Some("standup".to_string()),
            message: (!message.is_empty()).then(|| message.to_string()),
        }])
    }

    fn session_meta_with_subscriptions(
        subscriptions: Vec<protocol::subscriptions::PersistedSubscription>,
    ) -> protocol::protocol::SessionMetaLine {
        protocol::protocol::SessionMetaLine {
            meta: protocol::protocol::SessionMeta {
                subscriptions: Some(subscriptions),
                ..Default::default()
            },
            git: None,
        }
    }
