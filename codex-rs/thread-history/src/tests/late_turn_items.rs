use super::*;

#[test]
fn client_recovery_event_replays_as_typed_thread_item() {
    let first_identity = "11111111-1111-4111-8111-111111111111";
    let second_identity = "22222222-2222-4222-8222-222222222222";
    let first = ClientRecoveryRecordedEvent {
        id: format!("client-recovery:{first_identity}"),
        turn_id: format!("client-recovery:{first_identity}"),
        recovery_identity: Some(
            first_identity.into(),
        ),
        launcher_claim_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
        launcher_evidence_version: Some(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ),
        transaction_id: "tx-1".into(),
        request_id: "req-1".into(),
        failed_build_id: "build-bad".into(),
        failed_build_hash: "bad-hash".into(),
        source_commit: "source-commit".into(),
        requested_by_thread_id: None,
        mode: "full".into(),
        failure_phase: "ready-timeout".into(),
        exit_code: Some(1),
        signal: None,
        ready_timeout_ms: Some(30_000),
        log_path: Some("/tmp/recovery.log".into()),
        transaction_path: Some("/tmp/transaction.json".into()),
        recovered_build_id: "build-good".into(),
        prompt: "Inspect the rollback evidence.".into(),
        evidence_path: "/tmp/evidence.json".into(),
        recorded_at_ms: 42,
    };
    let mut second = first.clone();
    second.id = format!("client-recovery:{second_identity}");
    second.turn_id = second.id.clone();
    second.recovery_identity = Some(second_identity.into());
    second.launcher_claim_id =
        Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into());
    second.launcher_evidence_version = Some(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
    );
    second.prompt = "Inspect the second rollback evidence.".into();
    second.evidence_path = "/tmp/evidence-2.json".into();
    second.recorded_at_ms = 43;

    let turns = build_turns_from_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(first)),
        RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(second)),
    ]);

    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[0].items[0],
        ThreadItem::ClientRecovery {
            id: format!("client-recovery:{first_identity}"),
            recovery_identity: Some(first_identity.into()),
            launcher_claim_id: Some(
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            ),
            launcher_evidence_version: Some(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            ),
            transaction_id: "tx-1".into(),
            request_id: "req-1".into(),
            failed_build_id: "build-bad".into(),
            failed_build_hash: "bad-hash".into(),
            source_commit: "source-commit".into(),
            requested_by_thread_id: None,
            mode: "full".into(),
            failure_phase: "ready-timeout".into(),
            exit_code: Some(1),
            signal: None,
            ready_timeout_ms: Some(30_000),
            log_path: Some("/tmp/recovery.log".into()),
            transaction_path: Some("/tmp/transaction.json".into()),
            recovered_build_id: "build-good".into(),
            prompt: "Inspect the rollback evidence.".into(),
            evidence_path: "/tmp/evidence.json".into(),
            recorded_at_ms: 42,
        }
    );
    assert!(matches!(
        &turns[1].items[0],
        ThreadItem::ClientRecovery {
            recovery_identity: Some(recovery_identity),
            transaction_id,
            request_id,
            ..
        } if recovery_identity == second_identity
            && transaction_id == "tx-1"
            && request_id == "req-1"
    ));
}

#[test]
fn legacy_recovery_handled_event_does_not_reorder_reloaded_items() {
    let legacy_id = "client-recovery:legacy-tx:legacy-request";
    let legacy = ClientRecoveryRecordedEvent {
        id: legacy_id.into(),
        turn_id: "legacy-turn".into(),
        recovery_identity: None,
        launcher_claim_id: None,
        launcher_evidence_version: None,
        transaction_id: "legacy-tx".into(),
        request_id: "legacy-request".into(),
        failed_build_id: "legacy-failed".into(),
        failed_build_hash: "legacy-hash".into(),
        source_commit: "legacy-commit".into(),
        requested_by_thread_id: None,
        mode: "full".into(),
        failure_phase: "launch".into(),
        exit_code: None,
        signal: None,
        ready_timeout_ms: None,
        log_path: None,
        transaction_path: None,
        recovered_build_id: "legacy-recovered".into(),
        prompt: "Inspect legacy recovery evidence.".into(),
        evidence_path: "/tmp/legacy-evidence.json".into(),
        recorded_at_ms: 40,
    };
    let modern_identity = "22222222-2222-4222-8222-222222222222";
    let mut modern = legacy.clone();
    modern.id = format!("client-recovery:{modern_identity}");
    modern.turn_id = "modern-turn".into();
    modern.recovery_identity = Some(modern_identity.into());
    modern.launcher_claim_id =
        Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into());
    modern.launcher_evidence_version = Some(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .into(),
    );
    modern.transaction_id = "modern-tx".into();
    modern.request_id = "modern-request".into();
    modern.recorded_at_ms = 42;

    let turns = build_turns_from_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(legacy)),
        RolloutItem::EventMsg(EventMsg::ClientRecoveryHandled(
            ClientRecoveryHandledEvent {
                recovery_id: legacy_id.into(),
                recovery_identity: None,
                turn_id: "legacy-turn".into(),
                handled_at_ms: 41,
            },
        )),
        RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(modern)),
    ]);

    assert_eq!(
        turns
            .iter()
            .map(|turn| turn.id.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-turn", "modern-turn"]
    );
    assert!(matches!(
        &turns[0].items[0],
        ThreadItem::ClientRecovery {
            recovery_identity: None,
            launcher_claim_id: None,
            launcher_evidence_version: None,
            transaction_id,
            request_id,
            ..
        } if transaction_id == "legacy-tx" && request_id == "legacy-request"
    ));
    assert!(matches!(
        &turns[1].items[0],
        ThreadItem::ClientRecovery {
            recovery_identity: Some(identity),
            launcher_claim_id: Some(claim_id),
            launcher_evidence_version: Some(evidence_version),
            ..
        } if identity == modern_identity
            && claim_id == "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
            && evidence_version
                == "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    ));
}

    #[test]
    fn raw_assistant_response_item_does_not_update_current_turn_display() {
        let events = [
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
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
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn rollout_response_items_do_not_rebuild_display_items() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("msg-1".into()),
                role: "assistant".into(),
                content: vec![ContentItem::OutputText {
                    text: "hello from replay".into(),
                }],
                phase: Some(CoreMessagePhase::FinalAnswer),
            }),
            RolloutItem::ResponseItem(ResponseItem::EventDrivenTool {
                id: Some("trigger-1".into()),
                trigger: EventDrivenToolTrigger {
                    tool: "schedule_subscribe".into(),
                    title: "Schedule triggered".into(),
                    text: "tick".into(),
                },
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items, Vec::<ThreadItem>::new());
    }

    #[test]
    fn rollout_turn_context_restores_following_implicit_user_turn_id() {
        let items = vec![
            RolloutItem::TurnContext(turn_context_item_with_id("turn-from-context")),
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
        assert_eq!(turns[0].id, "turn-from-context");
        assert_eq!(
            turns[0].items,
            vec![ThreadItem::UserMessage {
                id: "item-1".into(),
                content: vec![UserInput::Text {
                    text: "hello".into(),
                    text_elements: Vec::new(),
                }],
            }]
        );
    }

    #[test]
    fn rollout_turn_context_ignores_initial_response_item_context() {
        let items = vec![
            RolloutItem::ResponseItem(ResponseItem::Message {
                id: Some("developer-context".into()),
                role: "developer".into(),
                content: vec![ContentItem::InputText {
                    text: "<permissions instructions>\nSandbox: workspace-write\n</permissions instructions>"
                        .into(),
                }],
                phase: None,
            }),
            RolloutItem::TurnContext(turn_context_item_with_id("turn-from-context")),
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
        assert_eq!(turns[0].id, "turn-from-context");
        assert_eq!(turns[0].items.len(), 1);
        assert!(matches!(turns[0].items[0], ThreadItem::UserMessage { .. }));
    }

    #[test]
    fn preserves_legitimate_repeated_legacy_agent_messages() {
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-a".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "repeat me".into(),
                phase: Some(CoreMessagePhase::Commentary),
                memory_citation: None,
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "repeat me".into(),
                phase: Some(CoreMessagePhase::Commentary),
                memory_citation: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            vec![
                ThreadItem::AgentMessage {
                    id: "item-1".into(),
                    text: "repeat me".into(),
                    phase: Some(CoreMessagePhase::Commentary),
                    memory_citation: None,
                },
                ThreadItem::AgentMessage {
                    id: "item-2".into(),
                    text: "repeat me".into(),
                    phase: Some(CoreMessagePhase::Commentary),
                    memory_citation: None,
                },
            ]
        );
    }

    #[test]
    fn reconstructs_declined_exec_and_patch_items() {
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
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-declined".into(),
                process_id: Some("pid-2".into()),
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
                stderr: "exec command rejected by user".into(),
                aggregated_output: "exec command rejected by user".into(),
                exit_code: -1,
                duration: Duration::ZERO,
                formatted_output: String::new(),
                status: CoreExecCommandStatus::Declined,
            }),
            EventMsg::PatchApplyEnd(PatchApplyEndEvent {
                call_id: "patch-declined".into(),
                turn_id: "turn-1".into(),
                stdout: String::new(),
                stderr: "patch rejected by user".into(),
                success: false,
                changes: [(
                    PathBuf::from("README.md"),
                    protocol::protocol::FileChange::Add {
                        content: "hello\n".into(),
                    },
                )]
                .into_iter()
                .collect(),
                status: CorePatchApplyStatus::Declined,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 3);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "exec-declined".into(),
                command: "ls".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-2".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Declined,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "ls".into(),
                }],
                aggregated_output: Some("exec command rejected by user".into()),
                exit_code: Some(-1),
                duration_ms: Some(0),
            }
        );
        assert_eq!(
            turns[0].items[2],
            ThreadItem::FileChange {
                id: "patch-declined".into(),
                changes: vec![FileUpdateChange {
                    path: "README.md".into(),
                    kind: PatchChangeKind::Add,
                    diff: "hello\n".into(),
                }],
                status: PatchApplyStatus::Declined,
            }
        );
    }

    #[test]
    fn reconstructs_declined_guardian_command_item() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "review this command".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-exec".into(),
                target_item_id: Some("guardian-exec".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 1_000,
                completed_at_ms: None,
                status: GuardianAssessmentStatus::InProgress,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: None,
                action: serde_json::from_value(serde_json::json!({
                    "type": "command",
                    "source": "shell",
                    "command": "rm -rf /tmp/guardian",
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-exec".into(),
                target_item_id: Some("guardian-exec".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 1_000,
                completed_at_ms: Some(1_042),
                status: GuardianAssessmentStatus::Denied,
                risk_level: Some(protocol::protocol::GuardianRiskLevel::High),
                user_authorization: Some(protocol::protocol::GuardianUserAuthorization::Low),
                rationale: Some("Would delete user data.".into()),
                decision_source: Some(protocol::protocol::GuardianAssessmentDecisionSource::Agent),
                action: serde_json::from_value(serde_json::json!({
                    "type": "command",
                    "source": "shell",
                    "command": "rm -rf /tmp/guardian",
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
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
            ThreadItem::CommandExecution {
                id: "guardian-exec".into(),
                command: "rm -rf /tmp/guardian".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Declined,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "rm -rf /tmp/guardian".into(),
                }],
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            }
        );
    }

    #[test]
    fn reconstructs_in_progress_guardian_execve_item() {
        let events = vec![
            EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }),
            EventMsg::UserMessage(UserMessageEvent {
                message: "run a subcommand".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::GuardianAssessment(GuardianAssessmentEvent {
                id: "review-guardian-execve".into(),
                target_item_id: Some("guardian-execve".into()),
                turn_id: "turn-1".into(),
                started_at_ms: 2_000,
                completed_at_ms: None,
                status: GuardianAssessmentStatus::InProgress,
                risk_level: None,
                user_authorization: None,
                rationale: None,
                decision_source: None,
                action: serde_json::from_value(serde_json::json!({
                    "type": "execve",
                    "source": "shell",
                    "program": "/bin/rm",
                    "argv": ["/usr/bin/rm", "-f", "/tmp/file.sqlite"],
                    "cwd": test_path_buf("/tmp"),
                }))
                .expect("guardian action"),
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
            ThreadItem::CommandExecution {
                id: "guardian-execve".into(),
                command: "/bin/rm -f /tmp/file.sqlite".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: None,
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::InProgress,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "/bin/rm -f /tmp/file.sqlite".into(),
                }],
                aggregated_output: None,
                exit_code: None,
                duration_ms: None,
            }
        );
    }

    #[test]
    fn assigns_late_exec_completion_to_original_turn() {
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
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-late".into(),
                process_id: Some("pid-42".into()),
                turn_id: "turn-a".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "done".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo done".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: "done\n".into(),
                stderr: String::new(),
                aggregated_output: "done\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(5),
                formatted_output: "done\n".into(),
                status: CoreExecCommandStatus::Completed,
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
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(turns[1].items.len(), 1);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CommandExecution {
                id: "exec-late".into(),
                command: "echo done".into(),
                cwd: test_path_buf("/tmp").abs(),
                process_id: Some("pid-42".into()),
                source: CommandExecutionSource::Agent,
                status: CommandExecutionStatus::Completed,
                initial_wait_ms: None,
                notify_on: None,
                command_actions: vec![CommandAction::Unknown {
                    command: "echo done".into(),
                }],
                aggregated_output: Some("done\n".into()),
                exit_code: Some(0),
                duration_ms: Some(5),
            }
        );
    }

    #[test]
    fn drops_late_turn_scoped_item_for_unknown_turn_id() {
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
            EventMsg::ExecCommandEnd(ExecCommandEndEvent {
                call_id: "exec-unknown-turn".into(),
                process_id: Some("pid-42".into()),
                turn_id: "turn-missing".into(),
                completed_at_ms: 0,
                command: vec!["echo".into(), "done".into()],
                cwd: test_path_buf("/tmp").abs(),
                parsed_cmd: vec![ParsedCommand::Unknown {
                    cmd: "echo done".into(),
                }],
                source: ExecCommandSource::Agent,
                interaction_input: None,
                initial_wait_ms: None,
                notify_on: None,
                stdout: "done\n".into(),
                stderr: String::new(),
                aggregated_output: "done\n".into(),
                exit_code: 0,
                duration: Duration::from_millis(5),
                formatted_output: "done\n".into(),
                status: CoreExecCommandStatus::Completed,
            }),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-b".into(),
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        ];

        let mut builder = ThreadHistoryBuilder::new();
        for event in &events {
            builder.handle_event(event);
        }
        let turns = builder.finish();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, "turn-a");
        assert_eq!(turns[1].id, "turn-b");
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(turns[1].items.len(), 1);
        assert_eq!(
            turns[1].items[0],
            ThreadItem::UserMessage {
                id: "item-2".into(),
                content: vec![UserInput::Text {
                    text: "second".into(),
                    text_elements: Vec::new(),
                }],
            }
        );
    }
