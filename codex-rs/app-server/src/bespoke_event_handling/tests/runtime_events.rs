use super::*;

    #[tokio::test]
    async fn test_handle_error_records_message() -> Result<()> {
        let conversation_id = ThreadId::new();
        let thread_state = new_thread_state();

        handle_error(
            conversation_id,
            TurnError {
                message: "boom".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::InternalServerError),
                additional_details: None,
            },
            &thread_state,
        )
        .await;

        let turn_summary = find_and_remove_turn_summary(conversation_id, &thread_state).await;
        assert_eq!(
            turn_summary.last_error,
            Some(TurnError {
                message: "boom".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::InternalServerError),
                additional_details: None,
            })
        );
        Ok(())
    }
    #[tokio::test]
    async fn turn_started_omits_active_snapshot_items() -> Result<()> {
        let codex_home = TempDir::new()?;
        let config = load_default_config_for_test(&codex_home).await;
        let thread_service = Arc::new(
            thread_service::test_support::thread_service_with_models_provider_and_home(
                CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            ),
        );
        let thread_service::NewThread {
            thread_id: conversation_id,
            thread: conversation,
            ..
        } = thread_service.start_thread(config.clone()).await?;
        let thread_state = new_thread_state();
        {
            let mut state = thread_state.lock().await;
            state.track_current_turn_event(
                "turn-1",
                &EventMsg::TurnStarted(protocol::protocol::TurnStartedEvent {
                    turn_id: "turn-1".to_string(),
                    started_at: Some(42),
                    model_context_window: None,
                    collaboration_mode_kind: Default::default(),
                }),
            );
            state.track_current_turn_event(
                "turn-1",
                &EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
                    message: "already tracked".to_string(),
                    images: None,
                    local_images: Vec::new(),
                    skills: Vec::new(),
                    text_elements: Vec::new(),
                }),
            );
        }
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            conversation_id,
        );

        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::TurnStarted(protocol::protocol::TurnStartedEvent {
                    turn_id: "turn-1".to_string(),
                    started_at: Some(42),
                    model_context_window: None,
                    collaboration_mode_kind: Default::default(),
                }),
            },
            conversation_id,
            conversation,
            thread_service,
            outgoing,
            thread_state,
            thread_watch_manager,
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnStarted(n)) => {
                assert_eq!(n.turn.id, "turn-1");
                assert_eq!(n.turn.items_view, TurnItemsView::NotLoaded);
                assert!(n.turn.items.is_empty());
            }
            other => bail!("unexpected message: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn command_wait_completed_emits_command_wait_thread_item() -> Result<()> {
        let codex_home = TempDir::new()?;
        let config = load_default_config_for_test(&codex_home).await;
        let thread_service = Arc::new(
            thread_service::test_support::thread_service_with_models_provider_and_home(
                CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            ),
        );
        let thread_service::NewThread {
            thread_id: conversation_id,
            thread: conversation,
            ..
        } = thread_service.start_thread(config).await?;
        let thread_state = new_thread_state();
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = test_outgoing(tx);

        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::CommandWaitCompleted(protocol::protocol::CommandWaitDisplayEvent {
                    thread_id: conversation_id,
                    turn_id: "turn-1".to_string(),
                    id: "wait-1".to_string(),
                    command_id: "cmd-1".to_string(),
                    status: protocol::models::CommandWaitStatus::Completed,
                    notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                    lifecycle_at_ms: 5678,
                }),
            },
            conversation_id,
            conversation,
            thread_service,
            outgoing,
            thread_state,
            thread_watch_manager,
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-1");
                assert_eq!(payload.completed_at_ms, 5678);
                assert_eq!(
                    payload.item,
                    ThreadItem::CommandWait {
                        id: "wait-1".to_string(),
                        command_id: "cmd-1".to_string(),
                        status: app_server_protocol::CommandWaitStatus::Completed,
                        notification: Some(app_server_protocol::CommandWaitNotificationKind::Exit,),
                        exit_code: Some(0),
                        wall_time_seconds: 1.25,
                        wait_timeout_ms: 250,
                        created_at_ms: 1234,
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn builtin_poll_event_emits_started_and_completed_thread_items() -> Result<()> {
        let codex_home = TempDir::new()?;
        let config = load_default_config_for_test(&codex_home).await;
        let thread_service = Arc::new(
            thread_service::test_support::thread_service_with_models_provider_and_home(
                CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            ),
        );
        let thread_service::NewThread {
            thread_id: conversation_id,
            thread: conversation,
            ..
        } = thread_service.start_thread(config).await?;
        let thread_state = new_thread_state();
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = test_outgoing(tx);
        let started_output = json!({
            "initialTimeoutMs": 50,
            "currentTimeoutMs": 50,
            "hardCapTimeoutMs": 1000,
        });

        let started = protocol::protocol::BuiltinToolCallDisplayEvent {
            thread_id: conversation_id,
            turn_id: "turn-1".to_string(),
            id: "builtin-poll-1".to_string(),
            tool: "poll_event".to_string(),
            arguments: json!({}),
            status: protocol::protocol::BuiltinToolCallStatus::InProgress,
            output: Some(started_output.clone()),
            lifecycle_at_ms: 1234,
        };
        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::BuiltinToolCallStarted(started.clone()),
            },
            conversation_id,
            conversation.clone(),
            thread_service.clone(),
            outgoing.clone(),
            thread_state.clone(),
            thread_watch_manager.clone(),
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemStarted(payload)) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-1");
                assert_eq!(payload.started_at_ms, started.lifecycle_at_ms);
                assert_eq!(
                    payload.item,
                    ThreadItem::BuiltinToolCall {
                        id: "builtin-poll-1".to_string(),
                        tool: "poll_event".to_string(),
                        arguments: json!({}),
                        status: app_server_protocol::DynamicToolCallStatus::InProgress,
                        output: Some(started_output),
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        let completed = protocol::protocol::BuiltinToolCallDisplayEvent {
            thread_id: conversation_id,
            turn_id: "turn-1".to_string(),
            id: "builtin-poll-1".to_string(),
            tool: "poll_event".to_string(),
            arguments: json!({}),
            status: protocol::protocol::BuiltinToolCallStatus::Completed,
            output: Some(json!({
                "timedOut": false,
                "sourceHint": "user_input",
                "waitedMs": 14,
                "initialTimeoutMs": 50,
                "currentTimeoutMs": 50,
                "hardCapTimeoutMs": 1000,
            })),
            lifecycle_at_ms: 1248,
        };
        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::BuiltinToolCallCompleted(completed.clone()),
            },
            conversation_id,
            conversation,
            thread_service,
            outgoing,
            thread_state,
            thread_watch_manager,
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-1");
                assert_eq!(payload.completed_at_ms, completed.lifecycle_at_ms);
                assert_eq!(
                    payload.item,
                    ThreadItem::BuiltinToolCall {
                        id: "builtin-poll-1".to_string(),
                        tool: "poll_event".to_string(),
                        arguments: json!({}),
                        status: app_server_protocol::DynamicToolCallStatus::Completed,
                        output: completed.output,
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn builtin_poll_event_failure_emits_completed_failed_thread_item() -> Result<()> {
        let codex_home = TempDir::new()?;
        let config = load_default_config_for_test(&codex_home).await;
        let thread_service = Arc::new(
            thread_service::test_support::thread_service_with_models_provider_and_home(
                CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            ),
        );
        let thread_service::NewThread {
            thread_id: conversation_id,
            thread: conversation,
            ..
        } = thread_service.start_thread(config).await?;
        let thread_state = new_thread_state();
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = test_outgoing(tx);

        let failed = protocol::protocol::BuiltinToolCallDisplayEvent {
            thread_id: conversation_id,
            turn_id: "turn-1".to_string(),
            id: "builtin-poll-failed".to_string(),
            tool: "poll_event".to_string(),
            arguments: json!({}),
            status: protocol::protocol::BuiltinToolCallStatus::Failed,
            output: Some(json!({
                "error": "thread wait backend unavailable",
            })),
            lifecycle_at_ms: 1300,
        };
        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::BuiltinToolCallCompleted(failed.clone()),
            },
            conversation_id,
            conversation,
            thread_service,
            outgoing,
            thread_state,
            thread_watch_manager,
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-1");
                assert_eq!(payload.completed_at_ms, failed.lifecycle_at_ms);
                assert_eq!(
                    payload.item,
                    ThreadItem::BuiltinToolCall {
                        id: "builtin-poll-failed".to_string(),
                        tool: "poll_event".to_string(),
                        arguments: json!({}),
                        status: app_server_protocol::DynamicToolCallStatus::Failed,
                        output: failed.output,
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn thread_skills_updated_emits_thread_scoped_notification() -> Result<()> {
        let codex_home = TempDir::new()?;
        let config = load_default_config_for_test(&codex_home).await;
        let thread_service = Arc::new(
            thread_service::test_support::thread_service_with_models_provider_and_home(
                CodexAuth::create_dummy_chatgpt_auth_for_testing(),
                config.model_provider.clone(),
                config.codex_home.to_path_buf(),
                Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            ),
        );
        let thread_service::NewThread {
            thread_id: conversation_id,
            thread: conversation,
            ..
        } = thread_service.start_thread(config.clone()).await?;
        let thread_state = new_thread_state();
        let thread_watch_manager = ThreadWatchManager::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            conversation_id,
        );

        apply_bespoke_event_handling(
            Event {
                id: "turn-1".to_string(),
                msg: EventMsg::ThreadSkillsUpdated(ThreadSkillsUpdatedEvent {
                    skills: vec![ThreadSkill {
                        name: "demo".to_string(),
                        path: "/tmp/demo/SKILL.md".to_string(),
                        kind: ThreadSkillKind::All,
                    }],
                }),
            },
            conversation_id,
            conversation,
            thread_service,
            outgoing,
            thread_state,
            thread_watch_manager,
            Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
            "test-provider".to_string(),
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ThreadSkillsUpdated(n)) => {
                assert_eq!(n.thread_id, conversation_id.to_string());
                assert_eq!(n.skills.len(), 1);
                assert_eq!(n.skills[0].name, "demo");
                assert_eq!(n.skills[0].path, "/tmp/demo/SKILL.md");
            }
            other => bail!("expected thread/skills/updated, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_completed_without_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "complete1".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let thread_state = new_thread_state();
        {
            let mut state = thread_state.lock().await;
            state.track_current_turn_event(
                &event_turn_id,
                &EventMsg::TurnStarted(protocol::protocol::TurnStartedEvent {
                    turn_id: event_turn_id.clone(),
                    started_at: Some(42),
                    model_context_window: None,
                    collaboration_mode_kind: Default::default(),
                }),
            );
            state.track_current_turn_event(
                &event_turn_id,
                &EventMsg::TurnComplete(turn_complete_event(&event_turn_id)),
            );
        }

        handle_turn_complete(
            conversation_id,
            event_turn_id.clone(),
            turn_complete_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Completed);
                assert_eq!(n.turn.items_view, TurnItemsView::NotLoaded);
                assert!(n.turn.items.is_empty());
                assert_eq!(n.turn.error, None);
                assert_eq!(n.turn.started_at, Some(42));
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }
    #[tokio::test]
    async fn test_handle_turn_interrupted_emits_interrupted_with_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "interrupt1".to_string();
        let thread_state = new_thread_state();
        handle_error(
            conversation_id,
            TurnError {
                message: "oops".to_string(),
                codex_error_info: None,
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_turn_interrupted(
            conversation_id,
            event_turn_id.clone(),
            turn_aborted_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Interrupted);
                assert_eq!(n.turn.error, None);
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_failed_with_error() -> Result<()> {
        let conversation_id = ThreadId::new();
        let event_turn_id = "complete_err1".to_string();
        let thread_state = new_thread_state();
        handle_error(
            conversation_id,
            TurnError {
                message: "bad".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::Other),
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_turn_complete(
            conversation_id,
            event_turn_id.clone(),
            turn_complete_event(&event_turn_id),
            &outgoing,
            &thread_state,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, event_turn_id);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "bad".to_string(),
                        codex_error_info: Some(V2CodexErrorInfo::Other),
                        additional_details: None,
                    })
                );
                assert_eq!(n.turn.completed_at, Some(TEST_TURN_COMPLETED_AT));
                assert_eq!(n.turn.duration_ms, Some(TEST_TURN_DURATION_MS));
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_plan_update_emits_notification_for_v2() -> Result<()> {
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let update = UpdatePlanArgs {
            explanation: Some("need plan".to_string()),
            plan: vec![
                PlanItemArg {
                    step: "first".to_string(),
                    status: StepStatus::Pending,
                },
                PlanItemArg {
                    step: "second".to_string(),
                    status: StepStatus::Completed,
                },
            ],
        };

        let conversation_id = ThreadId::new();

        handle_turn_plan_update(conversation_id, "turn-123", update, &outgoing).await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnPlanUpdated(n)) => {
                assert_eq!(n.thread_id, conversation_id.to_string());
                assert_eq!(n.turn_id, "turn-123");
                assert_eq!(n.explanation.as_deref(), Some("need plan"));
                assert_eq!(n.plan.len(), 2);
                assert_eq!(n.plan[0].step, "first");
                assert_eq!(n.plan[0].status, TurnPlanStepStatus::Pending);
                assert_eq!(n.plan[1].step, "second");
                assert_eq!(n.plan[1].status, TurnPlanStepStatus::Completed);
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_token_count_event_emits_usage_and_rate_limits() -> Result<()> {
        let conversation_id = ThreadId::new();
        let turn_id = "turn-123".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        let info = TokenUsageInfo {
            total_token_usage: TokenUsage {
                input_tokens: 100,
                cached_input_tokens: 25,
                output_tokens: 50,
                reasoning_output_tokens: 9,
                total_tokens: 200,
            },
            last_token_usage: TokenUsage {
                input_tokens: 10,
                cached_input_tokens: 5,
                output_tokens: 7,
                reasoning_output_tokens: 1,
                total_tokens: 23,
            },
            model_context_window: Some(4096),
        };
        let rate_limits = RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: None,
            primary: Some(RateLimitWindow {
                used_percent: 42.5,
                window_minutes: Some(15),
                resets_at: Some(1700000000),
            }),
            secondary: None,
            credits: Some(CreditsSnapshot {
                has_credits: true,
                unlimited: false,
                balance: Some("5".to_string()),
            }),
            plan_type: None,
            rate_limit_reached_type: None,
        };

        handle_token_count_event(
            conversation_id,
            turn_id.clone(),
            TokenCountEvent {
                info: Some(info),
                rate_limits: Some(rate_limits),
            },
            &outgoing,
        )
        .await;

        let first = recv_broadcast_message(&mut rx).await?;
        match first {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ThreadTokenUsageUpdated(payload),
            ) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, turn_id);
                let usage = payload.token_usage;
                assert_eq!(usage.total.total_tokens, 200);
                assert_eq!(usage.total.cached_input_tokens, 25);
                assert_eq!(usage.last.output_tokens, 7);
                assert_eq!(usage.model_context_window, Some(4096));
            }
            other => bail!("unexpected notification: {other:?}"),
        }

        let second = recv_broadcast_message(&mut rx).await?;
        match second {
            OutgoingMessage::AppServerNotification(
                ServerNotification::AccountRateLimitsUpdated(payload),
            ) => {
                assert_eq!(payload.rate_limits.limit_id.as_deref(), Some("codex"));
                assert_eq!(payload.rate_limits.limit_name, None);
                assert!(payload.rate_limits.primary.is_some());
                assert!(payload.rate_limits.credits.is_some());
            }
            other => bail!("unexpected notification: {other:?}"),
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_token_count_event_without_usage_info() -> Result<()> {
        let conversation_id = ThreadId::new();
        let turn_id = "turn-456".to_string();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        handle_token_count_event(
            conversation_id,
            turn_id.clone(),
            TokenCountEvent {
                info: None,
                rate_limits: None,
            },
            &outgoing,
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no notifications should be emitted when token usage info is absent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_complete_emits_error_multiple_turns() -> Result<()> {
        // Conversation A will have two turns; Conversation B will have one turn.
        let conversation_a = ThreadId::new();
        let conversation_b = ThreadId::new();
        let thread_state = new_thread_state();

        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );

        // Turn 1 on conversation A
        let a_turn1 = "a_turn1".to_string();
        handle_error(
            conversation_a,
            TurnError {
                message: "a1".to_string(),
                codex_error_info: Some(V2CodexErrorInfo::BadRequest),
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        handle_turn_complete(
            conversation_a,
            a_turn1.clone(),
            turn_complete_event(&a_turn1),
            &outgoing,
            &thread_state,
        )
        .await;

        // Turn 1 on conversation B
        let b_turn1 = "b_turn1".to_string();
        handle_error(
            conversation_b,
            TurnError {
                message: "b1".to_string(),
                codex_error_info: None,
                additional_details: None,
            },
            &thread_state,
        )
        .await;
        handle_turn_complete(
            conversation_b,
            b_turn1.clone(),
            turn_complete_event(&b_turn1),
            &outgoing,
            &thread_state,
        )
        .await;

        // Turn 2 on conversation A
        let a_turn2 = "a_turn2".to_string();
        handle_turn_complete(
            conversation_a,
            a_turn2.clone(),
            turn_complete_event(&a_turn2),
            &outgoing,
            &thread_state,
        )
        .await;

        // Verify: A turn 1
        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, a_turn1);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "a1".to_string(),
                        codex_error_info: Some(V2CodexErrorInfo::BadRequest),
                        additional_details: None,
                    })
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        // Verify: B turn 1
        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, b_turn1);
                assert_eq!(n.turn.status, TurnStatus::Failed);
                assert_eq!(
                    n.turn.error,
                    Some(TurnError {
                        message: "b1".to_string(),
                        codex_error_info: None,
                        additional_details: None,
                    })
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        // Verify: A turn 2
        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnCompleted(n)) => {
                assert_eq!(n.turn.id, a_turn2);
                assert_eq!(n.turn.status, TurnStatus::Completed);
                assert_eq!(n.turn.error, None);
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_handle_turn_diff_emits_v2_notification() -> Result<()> {
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let unified_diff = "--- a\n+++ b\n".to_string();
        let conversation_id = ThreadId::new();

        handle_turn_diff(
            conversation_id,
            "turn-1",
            TurnDiffEvent {
                unified_diff: unified_diff.clone(),
            },
            &outgoing,
        )
        .await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::TurnDiffUpdated(
                notification,
            )) => {
                assert_eq!(notification.thread_id, conversation_id.to_string());
                assert_eq!(notification.turn_id, "turn-1");
                assert_eq!(notification.diff, unified_diff);
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }

    #[tokio::test]
    async fn test_hook_prompt_response_item_emits_item_completed() -> Result<()> {
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let conversation_id = ThreadId::new();
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            conversation_id,
        );
        let item = build_hook_prompt_message(&[
            HookPromptFragment::from_single_hook("Retry with tests.", "hook-run-1"),
            HookPromptFragment::from_single_hook("Then summarize cleanly.", "hook-run-2"),
        ])
        .expect("hook prompt message");

        maybe_emit_hook_prompt_item_completed(conversation_id, "turn-1", &item, &outgoing).await;

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(
                notification,
            )) => {
                assert_eq!(notification.thread_id, conversation_id.to_string());
                assert_eq!(notification.turn_id, "turn-1");
                assert_eq!(
                    notification.item,
                    ThreadItem::HookPrompt {
                        id: notification.item.id().to_string(),
                        fragments: vec![
                            app_server_protocol::HookPromptFragment {
                                text: "Retry with tests.".into(),
                                hook_run_id: "hook-run-1".into(),
                            },
                            app_server_protocol::HookPromptFragment {
                                text: "Then summarize cleanly.".into(),
                                hook_run_id: "hook-run-2".into(),
                            },
                        ],
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no extra messages expected");
        Ok(())
    }
