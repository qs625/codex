use super::*;

    #[test]
    fn guardian_assessment_started_uses_event_turn_id_fallback() {
        let conversation_id = ThreadId::new();
        let action = protocol::protocol::GuardianAssessmentAction::Command {
            source: protocol::protocol::GuardianCommandSource::Shell,
            command: "rm -rf /tmp/example.sqlite".to_string(),
            cwd: test_path_buf("/tmp").abs(),
        };
        let notification = guardian_auto_approval_review_notification(
            &conversation_id,
            "turn-from-event",
            &GuardianAssessmentEvent {
                id: "review-1".to_string(),
                target_item_id: Some("item-1".to_string()),
                turn_id: String::new(),
                started_at_ms: 1_000,
                completed_at_ms: None,
                status: protocol::protocol::GuardianAssessmentStatus::InProgress,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: None,
                action: action.clone(),
            },
        );

        match notification {
            ServerNotification::ItemGuardianApprovalReviewStarted(payload) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-from-event");
                assert_eq!(payload.started_at_ms, 1_000);
                assert_eq!(payload.review_id, "review-1");
                assert_eq!(payload.target_item_id.as_deref(), Some("item-1"));
                assert_eq!(
                    payload.review.status,
                    GuardianApprovalReviewStatus::InProgress
                );
                assert_eq!(payload.review.risk_level, None);
                assert_eq!(payload.review.user_authorization, None);
                assert_eq!(payload.review.rationale, None);
                assert_eq!(payload.action, action.into());
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[test]
    fn guardian_assessment_completed_emits_review_payload() {
        let conversation_id = ThreadId::new();
        let action = protocol::protocol::GuardianAssessmentAction::Command {
            source: protocol::protocol::GuardianCommandSource::Shell,
            command: "rm -rf /tmp/example.sqlite".to_string(),
            cwd: test_path_buf("/tmp").abs(),
        };
        let notification = guardian_auto_approval_review_notification(
            &conversation_id,
            "turn-from-event",
            &GuardianAssessmentEvent {
                id: "review-2".to_string(),
                target_item_id: Some("item-2".to_string()),
                turn_id: "turn-from-assessment".to_string(),
                started_at_ms: 1_000,
                completed_at_ms: Some(1_042),
                status: protocol::protocol::GuardianAssessmentStatus::Denied,
                risk_level: Some(protocol::protocol::GuardianRiskLevel::High),
                user_authorization: Some(protocol::protocol::GuardianUserAuthorization::Low),
                rationale: Some("too risky".to_string()),
                decision_source: Some(protocol::protocol::GuardianAssessmentDecisionSource::Agent),
                action: action.clone(),
            },
        );

        match notification {
            ServerNotification::ItemGuardianApprovalReviewCompleted(payload) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-from-assessment");
                assert_eq!(payload.started_at_ms, 1_000);
                assert_eq!(payload.completed_at_ms, 1_042);
                assert_eq!(payload.review_id, "review-2");
                assert_eq!(payload.target_item_id.as_deref(), Some("item-2"));
                assert_eq!(payload.decision_source, AutoReviewDecisionSource::Agent);
                assert_eq!(payload.review.status, GuardianApprovalReviewStatus::Denied);
                assert_eq!(
                    payload.review.risk_level,
                    Some(app_server_protocol::GuardianRiskLevel::High)
                );
                assert_eq!(
                    payload.review.user_authorization,
                    Some(app_server_protocol::GuardianUserAuthorization::Low)
                );
                assert_eq!(payload.review.rationale.as_deref(), Some("too risky"));
                assert_eq!(payload.action, action.into());
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[test]
    fn guardian_assessment_aborted_emits_completed_review_payload() {
        let conversation_id = ThreadId::new();
        let action = protocol::protocol::GuardianAssessmentAction::NetworkAccess {
            target: "api.openai.com:443".to_string(),
            host: "api.openai.com".to_string(),
            protocol: protocol::protocol::NetworkApprovalProtocol::Https,
            port: 443,
        };
        let notification = guardian_auto_approval_review_notification(
            &conversation_id,
            "turn-from-event",
            &GuardianAssessmentEvent {
                id: "review-3".to_string(),
                target_item_id: None,
                turn_id: "turn-from-assessment".to_string(),
                started_at_ms: 1_000,
                completed_at_ms: Some(1_042),
                status: protocol::protocol::GuardianAssessmentStatus::Aborted,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: Some(protocol::protocol::GuardianAssessmentDecisionSource::Agent),
                action: action.clone(),
            },
        );

        match notification {
            ServerNotification::ItemGuardianApprovalReviewCompleted(payload) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-from-assessment");
                assert_eq!(payload.review_id, "review-3");
                assert_eq!(payload.target_item_id, None);
                assert_eq!(payload.decision_source, AutoReviewDecisionSource::Agent);
                assert_eq!(payload.review.status, GuardianApprovalReviewStatus::Aborted);
                assert_eq!(payload.review.risk_level, None);
                assert_eq!(payload.review.user_authorization, None);
                assert_eq!(payload.review.rationale, None);
                assert_eq!(payload.action, action.into());
            }
            other => panic!("unexpected notification: {other:?}"),
        }
    }

    #[tokio::test]
    async fn command_execution_started_helper_emits_once() -> Result<()> {
        let conversation_id = ThreadId::new();
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
        let completion_item = command_execution_completion_item("printf hi");

        let first_start = start_command_execution_item(
            &conversation_id,
            "turn-1".to_string(),
            "cmd-1".to_string(),
            completion_item.command.clone(),
            completion_item.cwd.clone(),
            completion_item.command_actions.clone(),
            CommandExecutionSource::Agent,
            &outgoing,
            &thread_state,
        )
        .await;
        assert!(first_start);

        let msg = recv_broadcast_message(&mut rx).await?;
        match msg {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemStarted(payload)) => {
                assert_eq!(payload.thread_id, conversation_id.to_string());
                assert_eq!(payload.turn_id, "turn-1");
                assert_eq!(
                    payload.item,
                    ThreadItem::CommandExecution {
                        id: "cmd-1".to_string(),
                        command: completion_item.command.clone(),
                        cwd: completion_item.cwd.clone(),
                        process_id: None,
                        source: CommandExecutionSource::Agent,
                        status: CommandExecutionStatus::InProgress,
                        initial_wait_ms: None,
                        notify_on: None,
                        command_actions: completion_item.command_actions.clone(),
                        aggregated_output: None,
                        exit_code: None,
                        duration_ms: None,
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        let second_start = start_command_execution_item(
            &conversation_id,
            "turn-1".to_string(),
            "cmd-1".to_string(),
            completion_item.command.clone(),
            completion_item.cwd.clone(),
            completion_item.command_actions.clone(),
            CommandExecutionSource::Agent,
            &outgoing,
            &thread_state,
        )
        .await;
        assert!(!second_start);
        assert!(rx.try_recv().is_err(), "duplicate start should not emit");
        Ok(())
    }

    #[tokio::test]
    async fn complete_command_execution_item_emits_declined_once_for_pending_command() -> Result<()>
    {
        let conversation_id = ThreadId::new();
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
        let completion_item = command_execution_completion_item("printf hi");

        start_command_execution_item(
            &conversation_id,
            "turn-1".to_string(),
            "cmd-1".to_string(),
            completion_item.command.clone(),
            completion_item.cwd.clone(),
            completion_item.command_actions.clone(),
            CommandExecutionSource::Agent,
            &outgoing,
            &thread_state,
        )
        .await;
        let _started = recv_broadcast_message(&mut rx).await?;

        complete_command_execution_item(
            &conversation_id,
            "turn-1".to_string(),
            "cmd-1".to_string(),
            completion_item.command.clone(),
            completion_item.cwd.clone(),
            /*process_id*/ None,
            CommandExecutionSource::Agent,
            completion_item.command_actions.clone(),
            CommandExecutionStatus::Declined,
            &outgoing,
            &thread_state,
        )
        .await;

        let completed = recv_broadcast_message(&mut rx).await?;
        match completed {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                let ThreadItem::CommandExecution { id, status, .. } = payload.item else {
                    bail!("expected command execution completion");
                };
                assert_eq!(id, "cmd-1");
                assert_eq!(status, CommandExecutionStatus::Declined);
            }
            other => bail!("unexpected message: {other:?}"),
        }

        complete_command_execution_item(
            &conversation_id,
            "turn-1".to_string(),
            "cmd-1".to_string(),
            completion_item.command,
            completion_item.cwd,
            /*process_id*/ None,
            CommandExecutionSource::Agent,
            completion_item.command_actions,
            CommandExecutionStatus::Declined,
            &outgoing,
            &thread_state,
        )
        .await;
        assert!(
            rx.try_recv().is_err(),
            "completion should not emit after the pending item is cleared"
        );
        Ok(())
    }

    #[tokio::test]
    async fn guardian_command_execution_notifications_wrap_review_lifecycle() -> Result<()> {
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
        let guardian_context = GuardianAssessmentTestContext {
            conversation_id,
            conversation: conversation.clone(),
            thread_service: thread_service.clone(),
            outgoing: outgoing.clone(),
            thread_state: thread_state.clone(),
            thread_watch_manager: thread_watch_manager.clone(),
        };

        guardian_context
            .apply_guardian_assessment_event(guardian_command_assessment(
                "cmd-guardian-approved",
                "turn-guardian-approved",
                GuardianAssessmentStatus::InProgress,
            ))
            .await;
        let first = recv_broadcast_message(&mut rx).await?;
        match first {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemStarted(payload)) => {
                assert_eq!(payload.turn_id, "turn-guardian-approved");
                let ThreadItem::CommandExecution { id, status, .. } = payload.item else {
                    bail!("expected command execution item");
                };
                assert_eq!(id, "cmd-guardian-approved");
                assert_eq!(status, CommandExecutionStatus::InProgress);
            }
            other => bail!("unexpected message: {other:?}"),
        }
        let second = recv_broadcast_message(&mut rx).await?;
        match second {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ItemGuardianApprovalReviewStarted(payload),
            ) => {
                assert_eq!(payload.review_id, "review-cmd-guardian-approved");
                assert_eq!(
                    payload.target_item_id.as_deref(),
                    Some("cmd-guardian-approved")
                );
                assert_eq!(
                    payload.review.status,
                    GuardianApprovalReviewStatus::InProgress
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        guardian_context
            .apply_guardian_assessment_event(guardian_command_assessment(
                "cmd-guardian-approved",
                "turn-guardian-approved",
                GuardianAssessmentStatus::Approved,
            ))
            .await;
        let third = recv_broadcast_message(&mut rx).await?;
        match third {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ItemGuardianApprovalReviewCompleted(payload),
            ) => {
                assert_eq!(payload.review_id, "review-cmd-guardian-approved");
                assert_eq!(
                    payload.target_item_id.as_deref(),
                    Some("cmd-guardian-approved")
                );
                assert_eq!(payload.decision_source, AutoReviewDecisionSource::Agent);
                assert_eq!(
                    payload.review.status,
                    GuardianApprovalReviewStatus::Approved
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }
        assert!(
            rx.try_recv().is_err(),
            "approved review should not complete the command item"
        );

        guardian_context
            .apply_guardian_assessment_event(guardian_command_assessment(
                "cmd-guardian-denied",
                "turn-guardian-denied",
                GuardianAssessmentStatus::InProgress,
            ))
            .await;
        let fourth = recv_broadcast_message(&mut rx).await?;
        match fourth {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemStarted(payload)) => {
                assert_eq!(payload.turn_id, "turn-guardian-denied");
                let ThreadItem::CommandExecution { id, status, .. } = payload.item else {
                    bail!("expected command execution item");
                };
                assert_eq!(id, "cmd-guardian-denied");
                assert_eq!(status, CommandExecutionStatus::InProgress);
            }
            other => bail!("unexpected message: {other:?}"),
        }
        let fifth = recv_broadcast_message(&mut rx).await?;
        match fifth {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ItemGuardianApprovalReviewStarted(payload),
            ) => {
                assert_eq!(payload.review_id, "review-cmd-guardian-denied");
                assert_eq!(
                    payload.target_item_id.as_deref(),
                    Some("cmd-guardian-denied")
                );
                assert_eq!(
                    payload.review.status,
                    GuardianApprovalReviewStatus::InProgress
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        guardian_context
            .apply_guardian_assessment_event(guardian_command_assessment(
                "cmd-guardian-denied",
                "turn-guardian-denied",
                GuardianAssessmentStatus::Denied,
            ))
            .await;
        let sixth = recv_broadcast_message(&mut rx).await?;
        match sixth {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ItemGuardianApprovalReviewCompleted(payload),
            ) => {
                assert_eq!(payload.review_id, "review-cmd-guardian-denied");
                assert_eq!(
                    payload.target_item_id.as_deref(),
                    Some("cmd-guardian-denied")
                );
                assert_eq!(payload.decision_source, AutoReviewDecisionSource::Agent);
                assert_eq!(payload.review.status, GuardianApprovalReviewStatus::Denied);
            }
            other => bail!("unexpected message: {other:?}"),
        }
        let seventh = recv_broadcast_message(&mut rx).await?;
        match seventh {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                let ThreadItem::CommandExecution { id, status, .. } = payload.item else {
                    bail!("expected command execution completion");
                };
                assert_eq!(id, "cmd-guardian-denied");
                assert_eq!(status, CommandExecutionStatus::Declined);
            }
            other => bail!("unexpected message: {other:?}"),
        }

        let mut missing_target = guardian_command_assessment(
            "cmd-guardian-missing-target",
            "turn-guardian-missing-target",
            GuardianAssessmentStatus::InProgress,
        );
        missing_target.target_item_id = None;
        guardian_context
            .apply_guardian_assessment_event(missing_target)
            .await;
        let eighth = recv_broadcast_message(&mut rx).await?;
        match eighth {
            OutgoingMessage::AppServerNotification(
                ServerNotification::ItemGuardianApprovalReviewStarted(payload),
            ) => {
                assert_eq!(payload.review_id, "review-cmd-guardian-missing-target");
                assert_eq!(payload.target_item_id, None);
                assert_eq!(
                    payload.review.status,
                    GuardianApprovalReviewStatus::InProgress
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(rx.try_recv().is_err(), "no extra messages expected");
        thread_service.shutdown_live_thread(conversation_id).await?;
        thread_service.remove_live_thread(conversation_id).await;
        Ok(())
    }
