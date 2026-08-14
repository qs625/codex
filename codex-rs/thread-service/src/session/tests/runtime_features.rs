#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_startup_context_then_first_turn_diff_snapshot() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let first_forked_request = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let mut builder = test_codex().with_config(|config| {
        config.permissions.approval_policy =
            config_service::Constrained::allow_any(AskForApproval::OnRequest);
    });
    let initial = builder.build(&server).await?;
    let rollout_path = initial
        .session_configured
        .rollout_path
        .clone()
        .expect("rollout path");

    initial
        .codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "fork seed".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    wait_for_event(&initial.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;
    // Forking reads the persisted rollout JSONL, so force the completed source turn to disk
    // before snapshotting from it.
    initial.codex.ensure_rollout_materialized().await;
    initial
        .codex
        .flush_rollout()
        .await
        .expect("source rollout should flush before fork");

    let mut fork_config = initial.config.clone();
    fork_config.permissions.approval_policy =
        config_service::Constrained::allow_any(AskForApproval::UnlessTrusted);
    let forked = initial
        .thread_service
        .fork_thread(
            usize::MAX,
            fork_config.clone(),
            rollout_path,
            /*thread_source*/ None,
            /*persist_extended_history*/ false,
            /*parent_trace*/ None,
        )
        .await?;

    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Plan,
        settings: Settings {
            model: forked.session_configured.model.clone(),
            reasoning_effort: None,
            developer_instructions: Some("Fork turn collaboration instructions.".to_string()),
        },
    };
    forked
        .thread
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: Some(AskForApproval::Never),
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collaboration_mode),
            personality: None,
        })
        .await?;

    forked
        .thread
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "after fork".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    wait_for_event(&forked.thread, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let request = first_forked_request.single_request();
    let snapshot = context_snapshot::format_labeled_requests_snapshot(
        "First request after fork when startup preserves the parent baseline, the fork changes approval policy, and the first forked turn enters plan mode.",
        &[("First Forked Turn Request", &request)],
        &ContextSnapshotOptions::default()
            .render_mode(ContextSnapshotRenderMode::KindWithTextPrefix { max_chars: 96 })
            .strip_capability_instructions()
            .strip_agents_md_user_context(),
    );

    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path("snapshots");
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        insta::assert_snapshot!(
            "codex_core__codex_tests__fork_startup_context_then_first_turn_diff",
            snapshot
        );
    });

    Ok(())
}

#[tokio::test]
async fn record_initial_history_forked_hydrates_previous_turn_settings() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_model = "forked-rollout-model";
    let previous_context_item = TurnContextItem {
        turn_id: Some(turn_context.sub_id.clone()),
        trace_id: turn_context.trace_id.clone(),
        #[allow(deprecated)]
        cwd: turn_context.cwd.to_path_buf(),
        current_date: turn_context.current_date.clone(),
        timezone: turn_context.timezone.clone(),
        approval_policy: turn_context.approval_policy.value(),
        sandbox_policy: turn_context.sandbox_policy(),
        permission_profile: None,
        network: None,
        file_system_sandbox_policy: None,
        model: previous_model.to_string(),
        personality: turn_context.personality,
        collaboration_mode: Some(turn_context.collaboration_mode.clone()),
        realtime_active: Some(turn_context.realtime_active),
        effort: turn_context.reasoning_effort,
        summary: turn_context.reasoning_summary,
        user_instructions: None,
        developer_instructions: None,
        final_output_json_schema: None,
        truncation_policy: Some(turn_context.truncation_policy),
    };
    let turn_id = previous_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let rollout_items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "forked seed".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(previous_context_item.clone()),
        RolloutItem::EventMsg(EventMsg::TurnComplete(
            protocol::protocol::TurnCompleteEvent {
                turn_id,
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            },
        )),
    ];

    session
        .record_initial_history(InitialHistory::Forked(rollout_items))
        .await;

    let history = session.clone_history().await;
    assert_eq!(
        session.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: previous_model.to_string(),
            realtime_active: Some(turn_context.realtime_active),
        })
    );
    assert_eq!(history.raw_items(), &[]);
    assert_eq!(
        serde_json::to_value(session.reference_context_item().await)
            .expect("serialize fork reference context item"),
        serde_json::to_value(Some(previous_context_item))
            .expect("serialize expected reference context item")
    );
}

#[tokio::test]
async fn thread_rollback_drops_last_turn_from_history() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    let rollout_path = attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    let turn_1 = vec![
        user_message("turn 1 user"),
        assistant_message("turn 1 assistant"),
    ];
    let turn_2 = vec![
        user_message("turn 2 user"),
        assistant_message("turn 2 assistant"),
    ];
    let mut full_history = Vec::new();
    full_history.extend(initial_context.clone());
    full_history.extend(turn_1.clone());
    full_history.extend(turn_2);
    sess.replace_history(full_history.clone(), Some(tc.to_turn_context_item()))
        .await;
    let rollout_items: Vec<RolloutItem> = full_history
        .into_iter()
        .map(RolloutItem::ResponseItem)
        .collect();
    sess.persist_rollout_items(&rollout_items).await;
    sess.set_previous_turn_settings(Some(PreviousTurnSettings {
        model: "stale-model".to_string(),
        realtime_active: Some(tc.realtime_active),
    }))
    .await;
    {
        let mut state = sess.state.lock().await;
        state.set_reference_context_item(Some(tc.to_turn_context_item()));
    }

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    let mut expected = Vec::new();
    expected.extend(initial_context);
    expected.extend(turn_1);

    let history = sess.clone_history().await;
    assert_eq!(expected, history.raw_items());
    assert_eq!(sess.previous_turn_settings().await, None);
    assert!(sess.reference_context_item().await.is_none());

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    assert!(resumed.history.iter().any(|item| {
        matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback))
            if rollback.num_turns == 1
        )
    }));
}

#[tokio::test]
async fn thread_rollback_clears_history_when_num_turns_exceeds_existing_turns() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    let turn_1 = vec![user_message("turn 1 user")];
    let mut full_history = Vec::new();
    full_history.extend(initial_context.clone());
    full_history.extend(turn_1);
    sess.replace_history(full_history.clone(), Some(tc.to_turn_context_item()))
        .await;
    let rollout_items: Vec<RolloutItem> = full_history
        .into_iter()
        .map(RolloutItem::ResponseItem)
        .collect();
    sess.persist_rollout_items(&rollout_items).await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 99).await;

    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 99);

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn thread_rollback_fails_without_persisted_thread_history() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(
        error_event.message,
        "thread rollback requires persisted thread history"
    );
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );
    assert_eq!(sess.clone_history().await.raw_items(), initial_context);
}

#[tokio::test]
async fn thread_rollback_recomputes_previous_turn_settings_and_reference_context_from_replay() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let first_context_item = tc.to_turn_context_item();
    let first_turn_id = first_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let mut rolled_back_context_item = first_context_item.clone();
    rolled_back_context_item.turn_id = Some("rolled-back-turn".to_string());
    rolled_back_context_item.model = "rolled-back-model".to_string();
    let rolled_back_turn_id = rolled_back_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let turn_one_user = user_message("turn 1 user");
    let turn_one_assistant = assistant_message("turn 1 assistant");
    let turn_two_user = user_message("turn 2 user");
    let turn_two_assistant = assistant_message("turn 2 assistant");

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: first_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "turn 1 user".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(turn_one_user.clone()),
        RolloutItem::ResponseItem(turn_one_assistant.clone()),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: first_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: rolled_back_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "turn 2 user".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(rolled_back_context_item),
        RolloutItem::ResponseItem(turn_two_user),
        RolloutItem::ResponseItem(turn_two_assistant),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: rolled_back_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;
    sess.replace_history(
        vec![assistant_message("stale history")],
        Some(first_context_item.clone()),
    )
    .await;
    sess.set_previous_turn_settings(Some(PreviousTurnSettings {
        model: "stale-model".to_string(),
        realtime_active: None,
    }))
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    assert_eq!(
        sess.clone_history().await.raw_items(),
        vec![turn_one_user, turn_one_assistant]
    );
    assert_eq!(
        sess.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: tc.model_info.slug.clone(),
            realtime_active: Some(tc.realtime_active),
        })
    );
    assert_eq!(
        serde_json::to_value(sess.reference_context_item().await)
            .expect("serialize replay reference context item"),
        serde_json::to_value(Some(first_context_item))
            .expect("serialize expected reference context item")
    );
}

#[tokio::test]
async fn thread_rollback_restores_cleared_reference_context_item_after_compaction() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let first_context_item = tc.to_turn_context_item();
    let first_turn_id = first_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let compact_turn_id = "compact-turn".to_string();
    let rolled_back_turn_id = "rolled-back-turn".to_string();
    let compacted_history = vec![
        user_message("turn 1 user"),
        user_message("summary after compaction"),
    ];

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: first_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 1 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 1 user")),
        RolloutItem::ResponseItem(assistant_message("turn 1 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: first_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: compact_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::Compacted(CompactedItem {
            message: "summary after compaction".to_string(),
            replacement_history: Some(compacted_history.clone()),
            visible_replacement_history_len: None,
        }),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: compact_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: rolled_back_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 2 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(TurnContextItem {
            turn_id: Some(rolled_back_turn_id.clone()),
            model: "rolled-back-model".to_string(),
            ..first_context_item.clone()
        }),
        RolloutItem::ResponseItem(user_message("turn 2 user")),
        RolloutItem::ResponseItem(assistant_message("turn 2 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: rolled_back_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;
    sess.replace_history(
        vec![assistant_message("stale history")],
        Some(first_context_item),
    )
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    assert_eq!(sess.clone_history().await.raw_items(), compacted_history);
    assert!(sess.reference_context_item().await.is_none());
}

#[tokio::test]
async fn thread_rollback_persists_marker_and_replays_cumulatively() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    let rollout_path = attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;
    let turn_context_item = tc.to_turn_context_item();

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-1".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 1 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 1 user")),
        RolloutItem::ResponseItem(assistant_message("turn 1 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-1".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-2".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 2 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 2 user")),
        RolloutItem::ResponseItem(assistant_message("turn 2 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-2".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-3".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 3 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item),
        RolloutItem::ResponseItem(user_message("turn 3 user")),
        RolloutItem::ResponseItem(assistant_message("turn 3 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-3".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let first_rollback = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(first_rollback.num_turns, 1);
    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let second_rollback = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(second_rollback.num_turns, 1);

    assert_eq!(
        sess.clone_history().await.raw_items(),
        vec![
            user_message("turn 1 user"),
            assistant_message("turn 1 assistant")
        ]
    );

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let rollback_markers = resumed
        .history
        .iter()
        .filter(|item| matches!(item, RolloutItem::EventMsg(EventMsg::ThreadRolledBack(_))))
        .count();
    assert_eq!(rollback_markers, 2);
}

#[tokio::test]
async fn thread_rollback_fails_when_turn_in_progress() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    *sess.active_turn.lock().await = Some(crate::state::ActiveTurn::default());
    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn thread_rollback_fails_when_num_turns_is_zero() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 0).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(error_event.message, "num_turns must be >= 1");
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn set_rate_limits_retains_previous_credits() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let mut state = SessionState::new(session_configuration);
    let initial = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 10.0,
            window_minutes: Some(15),
            resets_at: Some(1_700),
        }),
        secondary: None,
        credits: Some(CreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("10.00".to_string()),
        }),
        plan_type: Some(protocol::account::PlanType::Plus),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(initial.clone());

    let update = RateLimitSnapshot {
        limit_id: Some("codex_other".to_string()),
        limit_name: Some("codex_other".to_string()),
        primary: Some(RateLimitWindow {
            used_percent: 40.0,
            window_minutes: Some(30),
            resets_at: Some(1_800),
        }),
        secondary: Some(RateLimitWindow {
            used_percent: 5.0,
            window_minutes: Some(60),
            resets_at: Some(1_900),
        }),
        credits: None,
        plan_type: None,
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(update.clone());

    assert_eq!(
        state.latest_rate_limits,
        Some(RateLimitSnapshot {
            limit_id: Some("codex_other".to_string()),
            limit_name: Some("codex_other".to_string()),
            primary: update.primary.clone(),
            secondary: update.secondary,
            credits: initial.credits,
            plan_type: initial.plan_type,
            rate_limit_reached_type: None,
        })
    );
}

#[tokio::test]
async fn set_rate_limits_updates_plan_type_when_present() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let mut state = SessionState::new(session_configuration);
    let initial = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 15.0,
            window_minutes: Some(20),
            resets_at: Some(1_600),
        }),
        secondary: Some(RateLimitWindow {
            used_percent: 5.0,
            window_minutes: Some(45),
            resets_at: Some(1_650),
        }),
        credits: Some(CreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("15.00".to_string()),
        }),
        plan_type: Some(protocol::account::PlanType::Plus),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(initial.clone());

    let update = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 35.0,
            window_minutes: Some(25),
            resets_at: Some(1_700),
        }),
        secondary: None,
        credits: None,
        plan_type: Some(protocol::account::PlanType::Pro),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(update.clone());

    assert_eq!(
        state.latest_rate_limits,
        Some(RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: None,
            primary: update.primary,
            secondary: update.secondary,
            credits: initial.credits,
            plan_type: update.plan_type,
            rate_limit_reached_type: None,
        })
    );
}

#[test]
fn prefers_structured_content_when_present() {
    let ctr = McpCallToolResult {
        // Content present but should be ignored because structured_content is set.
        content: vec![text_block("ignored")],
        is_error: None,
        structured_content: Some(json!({
            "ok": true,
            "value": 42
        })),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&json!({
                "ok": true,
                "value": 42
            }))
            .unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[tokio::test]
async fn includes_timed_out_message() {
    let exec = ExecToolCallOutput {
        exit_code: 0,
        stdout: StreamOutput::new(String::new()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new("Command output".to_string()),
        duration: StdDuration::from_secs(1),
        timed_out: true,
    };
    let (_, turn_context) = make_session_and_context().await;

    let out = format_exec_output_str(&exec, turn_context.truncation_policy);

    assert_eq!(
        out,
        "command timed out after 1000 milliseconds\nCommand output"
    );
}

#[tokio::test]
async fn turn_context_with_model_updates_model_fields() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.reasoning_effort = Some(ReasoningEffortConfig::Minimal);
    let updated = turn_context
        .with_model("gpt-5.4".to_string(), &session.services.model_service)
        .await;
    let expected_model_info = session
        .services
        .model_service
        .get_model_info("gpt-5.4")
        .await
        .expect("model info");

    assert_eq!(updated.config.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(updated.collaboration_mode.model(), "gpt-5.4");
    assert_eq!(updated.model_info, expected_model_info);
    assert_eq!(
        updated.reasoning_effort,
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.collaboration_mode.reasoning_effort(),
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.config.model_reasoning_effort,
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.truncation_policy,
        expected_model_info.truncation_policy.into()
    );
}

#[test]
fn falls_back_to_content_when_structured_is_null() {
    let ctr = McpCallToolResult {
        content: vec![text_block("hello"), text_block("world")],
        is_error: None,
        structured_content: Some(serde_json::Value::Null),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&vec![text_block("hello"), text_block("world")]).unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_reflects_is_error_true() {
    let ctr = McpCallToolResult {
        content: vec![text_block("unused")],
        is_error: Some(true),
        structured_content: Some(json!({ "message": "bad" })),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&json!({ "message": "bad" })).unwrap(),
        ),
        success: Some(false),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_true_with_no_error_and_content_used() {
    let ctr = McpCallToolResult {
        content: vec![text_block("alpha")],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&vec![text_block("alpha")]).unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

async fn wait_for_thread_rolled_back(rx: &async_channel::Receiver<Event>) -> ThreadRolledBackEvent {
    let deadline = StdDuration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        match evt.msg {
            EventMsg::ThreadRolledBack(payload) => return payload,
            _ => continue,
        }
    }
}

async fn wait_for_thread_rollback_failed(rx: &async_channel::Receiver<Event>) -> ErrorEvent {
    let deadline = StdDuration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        match evt.msg {
            EventMsg::Error(payload)
                if payload.codex_error_info == Some(CodexErrorInfo::ThreadRollbackFailed) =>
            {
                return payload;
            }
            _ => continue,
        }
    }
}

async fn attach_thread_persistence(session: &mut Session) -> PathBuf {
    let config = session.get_config().await;
    let live_thread = LiveThread::create(
        Arc::clone(&session.services.thread_store),
        CreateThreadParams {
            thread_id: session.conversation_id,
            forked_from_id: None,
            source: SessionSource::Exec,
            thread_source: None,
            base_instructions: BaseInstructions::default(),
            dynamic_tools: Vec::new(),
            metadata: ThreadPersistenceMetadata {
                cwd: Some(config.cwd.to_path_buf()),
                model_provider: config.model_provider_id.clone(),
                memory_mode: if config.memories.generate_memories {
                    ThreadMemoryMode::Enabled
                } else {
                    ThreadMemoryMode::Disabled
                },
                root_agent_role: None,
                root_agent_path: None,
            },
            event_persistence_mode: ThreadEventPersistenceMode::Limited,
        },
    )
    .await
    .expect("create thread persistence");
    session.services.live_thread = Some(Arc::new(live_thread));
    session.ensure_rollout_materialized().await;
    session
        .flush_rollout()
        .await
        .expect("attached rollout should flush");
    session
        .current_rollout_path()
        .await
        .expect("load rollout path")
        .expect("thread should have rollout path")
}

fn text_block(s: &str) -> serde_json::Value {
    json!({
        "type": "text",
        "text": s,
    })
}

async fn build_test_config(codex_home: &Path) -> Config {
    ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .fallback_cwd(Some(codex_home.to_path_buf()))
        .build()
        .await
        .expect("load default test config")
}

fn session_telemetry(
    conversation_id: ThreadId,
    config: &Config,
    model_info: &ModelInfo,
    session_source: SessionSource,
) -> SessionTelemetry {
    SessionTelemetry::new(
        conversation_id,
        get_model_offline_for_tests(config.model.as_deref()).as_str(),
        model_info.slug.as_str(),
        /*account_id*/ None,
        Some("test@test.com".to_string()),
        Some(TelemetryAuthMode::Chatgpt),
        "test_originator".to_string(),
        /*log_user_prompts*/ false,
        "test".to_string(),
        session_source,
    )
}

#[tokio::test]
async fn session_settings_null_service_tier_update_clears_service_tier() {
    let session_configuration = make_session_configuration_for_tests().await;

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            service_tier: Some(None),
            ..Default::default()
        })
        .expect("null service tier update should apply");

    assert_eq!(updated.service_tier, None);
}

#[tokio::test]
async fn session_settings_model_provider_update_switches_provider() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let mut config = (*session_configuration.original_config_do_not_use).clone();
    let provider = ModelProviderInfo {
        name: "Corp".to_string(),
        base_url: Some("https://corp.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    config
        .model_providers
        .insert("corp".to_string(), provider.clone());
    session_configuration.original_config_do_not_use = Arc::new(config);

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            model_provider: Some("corp".to_string()),
            ..Default::default()
        })
        .expect("model provider update should apply");

    assert_eq!(updated.provider, provider);
    assert_eq!(updated.original_config_do_not_use.model_provider_id, "corp");
    assert_eq!(updated.original_config_do_not_use.model_provider, provider);
}

#[tokio::test]
async fn session_settings_model_update_infers_model_option_provider() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let mut config = (*session_configuration.original_config_do_not_use).clone();
    let provider = ModelProviderInfo {
        name: "ModelHub GPT".to_string(),
        base_url: Some("https://modelhub.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    config
        .model_providers
        .insert("modelhub-gpt".to_string(), provider.clone());
    config
        .model_options
        .push(config_service::config_toml::ModelOptionToml {
            model: "gpt-5.5-2026-04-24".to_string(),
            provider: "modelhub-gpt".to_string(),
            ..Default::default()
        });
    session_configuration.original_config_do_not_use = Arc::new(config);

    let collaboration_mode = session_configuration.collaboration_mode.with_updates(
        Some("gpt-5.5-2026-04-24".to_string()),
        None,
        /*developer_instructions*/ None,
    );
    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .expect("model option provider should be inferred from model update");

    assert_eq!(updated.provider, provider);
    assert_eq!(
        updated.original_config_do_not_use.model_provider_id,
        "modelhub-gpt"
    );
    assert_eq!(updated.original_config_do_not_use.model_provider, provider);
}

#[tokio::test]
async fn per_turn_config_preserves_explicit_modelhub_model_and_provider() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let mut config = (*session_configuration.original_config_do_not_use).clone();
    let provider = ModelProviderInfo {
        name: "ModelHub GPT".to_string(),
        base_url: Some("https://modelhub.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    config
        .model_providers
        .insert("modelhub-gpt".to_string(), provider.clone());
    session_configuration.original_config_do_not_use = Arc::new(config);

    let collaboration_mode = session_configuration.collaboration_mode.with_updates(
        Some("gpt-5.5-2026-04-24".to_string()),
        None,
        /*developer_instructions*/ None,
    );
    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            model_provider: Some("modelhub-gpt".to_string()),
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .expect("explicit modelhub model/provider update should apply");

    let per_turn_config = Session::build_per_turn_config(&updated, updated.cwd.clone());

    assert_eq!(per_turn_config.model.as_deref(), Some("gpt-5.5-2026-04-24"));
    assert_eq!(per_turn_config.model_provider_id, "modelhub-gpt");
    assert_eq!(per_turn_config.model_provider, provider);
}

#[tokio::test]
async fn session_settings_legacy_fast_service_tier_update_uses_priority_request_value() {
    let session_configuration = make_session_configuration_for_tests().await;

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            service_tier: Some(Some("fast".to_string())),
            ..Default::default()
        })
        .expect("legacy fast service tier update should apply");

    assert_eq!(
        updated.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );
}

pub(crate) async fn make_session_configuration_for_tests() -> SessionConfiguration {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };

    SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    }
}

fn turn_environments_for_tests(
    environment: &Arc<dyn exec_server_api::ExecEnvironment>,
    cwd: &codex_utils_absolute_path::AbsolutePathBuf,
) -> crate::environment_selection::ResolvedTurnEnvironments {
    crate::environment_selection::ResolvedTurnEnvironments {
        turn_environments: vec![TurnEnvironment {
            environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
            environment: Arc::clone(environment),
            cwd: cwd.clone(),
            shell: None,
        }],
    }
}

#[tokio::test]
async fn session_configuration_apply_preserves_profile_file_system_policy_on_cwd_only_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let project_root = workspace.path().join("project");
    let original_cwd = project_root.join("subdir");
    let docs_dir = original_cwd.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("create docs dir");
    let docs_dir = docs_dir.abs();

    session_configuration.cwd = original_cwd.abs();
    let sandbox_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: Vec::new(),
        network_access: false,
        exclude_tmpdir_env_var: true,
        exclude_slash_tmp: true,
    };
    let file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: docs_dir },
            access: FileSystemAccessMode::Read,
        },
    ]);
    let network_sandbox_policy = NetworkSandboxPolicy::from(&sandbox_policy);
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
                &file_system_sandbox_policy,
                network_sandbox_policy,
            ),
        )
        .expect("set permission profile");
    let expected_file_system_sandbox_policy = file_system_sandbox_policy
        .materialize_project_roots_with_workspace_roots(&session_configuration.workspace_roots);

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(project_root),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.file_system_sandbox_policy(),
        expected_file_system_sandbox_policy
    );
}

#[tokio::test]
async fn session_configuration_apply_permission_profile_preserves_existing_deny_read_entries() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let cwd = tempfile::tempdir().expect("create temp dir");
    session_configuration.cwd = cwd.path().abs();

    let workspace_policy = SandboxPolicy::new_workspace_write_policy();
    let deny_entry = FileSystemSandboxEntry {
        path: FileSystemPath::GlobPattern {
            pattern: "**/*.env".to_string(),
        },
        access: FileSystemAccessMode::None,
    };
    let mut existing_file_system_policy =
        FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
            &workspace_policy,
            session_configuration.cwd.as_path(),
        );
    existing_file_system_policy.glob_scan_max_depth = Some(2);
    existing_file_system_policy.entries.push(deny_entry.clone());
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&workspace_policy),
                &existing_file_system_policy,
                NetworkSandboxPolicy::Restricted,
            ),
        )
        .expect("set permission profile");

    let requested_file_system_policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &workspace_policy,
        session_configuration.cwd.as_path(),
    );
    let permission_profile = protocol::models::PermissionProfile::from_runtime_permissions(
        &requested_file_system_policy,
        NetworkSandboxPolicy::Restricted,
    );
    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            permission_profile: Some(permission_profile),
            ..Default::default()
        })
        .expect("permission profile update should succeed");

    let mut expected_file_system_policy = requested_file_system_policy
        .materialize_project_roots_with_workspace_roots(&session_configuration.workspace_roots);
    expected_file_system_policy.glob_scan_max_depth = Some(2);
    expected_file_system_policy.entries.push(deny_entry);
    assert_eq!(
        updated.file_system_sandbox_policy(),
        expected_file_system_policy
    );
}

#[tokio::test]
async fn session_configuration_apply_permission_profile_accepts_direct_write_roots() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let cwd = tempfile::tempdir().expect("create cwd");
    session_configuration.cwd = cwd.path().abs();
    let external_write_dir = tempfile::tempdir().expect("create external write root");
    let external_write_path = AbsolutePathBuf::from_absolute_path(
        codex_utils_absolute_path::canonicalize_preserving_symlinks(external_write_dir.path())
            .expect("canonical temp dir"),
    )
    .expect("canonical temp dir should be absolute");
    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: external_write_path.clone(),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            permission_profile: Some(permission_profile.clone()),
            ..Default::default()
        })
        .expect("permission profile update should accept direct runtime permissions");

    assert_eq!(updated.permission_profile(), permission_profile);
    assert_eq!(
        updated.file_system_sandbox_policy(),
        file_system_sandbox_policy
    );
    assert_eq!(
        updated.sandbox_policy(),
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![external_write_path],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
}

#[tokio::test]
async fn session_configuration_apply_rebinds_symbolic_profile_to_updated_workspace_roots() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let old_root = tempfile::tempdir().expect("create old root");
    let new_root = tempfile::tempdir().expect("create new root");
    let profile_root = tempfile::tempdir().expect("create profile root");
    let old_root = old_root.path().abs();
    let new_root = new_root.path().abs();
    let profile_root = profile_root.path().abs();
    session_configuration.workspace_roots = vec![old_root.clone()];

    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            workspace_roots: Some(vec![new_root.clone()]),
            permission_profile: Some(permission_profile),
            active_permission_profile: Some(ActivePermissionProfile::new("dev")),
            profile_workspace_roots: Some(vec![profile_root.clone()]),
            ..Default::default()
        })
        .expect("permission profile update should succeed");

    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
    assert_eq!(
        updated.active_permission_profile(),
        Some(ActivePermissionProfile::new("dev"))
    );
    assert_eq!(updated.profile_workspace_roots(), &[profile_root]);
}

#[tokio::test]
async fn session_configuration_apply_retargets_implicit_workspace_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let old_root = tempfile::tempdir().expect("create old root");
    let new_root = tempfile::tempdir().expect("create new root");
    let extra_root = tempfile::tempdir().expect("create extra root");
    let old_root = old_root.path().abs();
    let new_root = new_root.path().abs();
    let extra_root = extra_root.path().abs();
    session_configuration.cwd = old_root.clone();
    session_configuration.workspace_roots = vec![old_root.clone(), extra_root.clone()];

    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );
    session_configuration
        .set_permission_profile_for_tests(permission_profile)
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(new_root.to_path_buf()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.workspace_roots,
        vec![new_root.clone(), extra_root.clone()]
    );
    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(updated_policy.can_write_path_with_cwd(extra_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
}

#[cfg_attr(windows, ignore)]
#[tokio::test]
async fn new_default_turn_uses_config_aware_skills_for_role_overrides() {
    let (session, _turn_context) = make_session_and_context().await;
    let parent_config = session.get_config().await;
    let codex_home = parent_config.codex_home.clone();
    let skill_dir = codex_home.join("skills").join("demo");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\nname: demo-skill\ndescription: demo description\n---\n\n# Body\n",
    )
    .expect("write skill");

    let skill_fs = session
        .services
        .environment_manager
        .default_environment()
        .map(|environment| environment.get_filesystem())
        .unwrap_or_else(|| std::sync::Arc::clone(&codex_file_system::LOCAL_FS));
    let parent_outcome = session
        .services
        .skill_service
        .skills_for_cwd(
            &crate::build_skill_service_input_from_config(&parent_config, Vec::new()),
            /*force_reload*/ true,
            Some(Arc::clone(&skill_fs)),
        )
        .await;
    let parent_skill = parent_outcome
        .skills
        .iter()
        .find(|skill| skill.name == "demo-skill")
        .expect("demo skill should be discovered");
    assert_eq!(parent_outcome.is_skill_enabled(parent_skill), true);

    let role_path = codex_home.join("skills-role.toml");
    std::fs::write(
        &role_path,
        format!(
            r#"developer_instructions = "Stay focused"

[[skills.config]]
path = "{}"
enabled = false
"#,
            skill_path.display()
        ),
    )
    .expect("write role config");

    let mut child_config = (*parent_config).clone();
    child_config.agent_roles.insert(
        "custom".to_string(),
        crate::config::AgentRoleConfig {
            description: None,
            config_file: Some(role_path.to_path_buf()),
            nickname_candidates: None,
            ..Default::default()
        },
    );
    crate::agent::role::apply_role_to_config(&mut child_config, Some("custom"))
        .await
        .expect("custom role should apply");

    {
        let mut state = session.state.lock().await;
        state.session_configuration.original_config_do_not_use = Arc::new(child_config);
    }

    let child_turn = session
        .new_default_turn_with_sub_id("role-skill-turn".to_string())
        .await;
    let child_skill = child_turn
        .turn_skills
        .outcome
        .skills
        .iter()
        .find(|skill| skill.name == "demo-skill")
        .expect("demo skill should be discovered");
    assert_eq!(
        child_turn.turn_skills.outcome.is_skill_enabled(child_skill),
        false
    );
}

#[tokio::test]
async fn session_configuration_apply_retargets_legacy_workspace_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let original_cwd = workspace.path().join("repo-a").abs();
    let project_root = workspace.path().join("repo-b").abs();
    session_configuration.cwd = original_cwd.clone();
    session_configuration.workspace_roots = vec![session_configuration.cwd.clone()];
    let sandbox_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: Vec::new(),
        network_access: false,
        exclude_tmpdir_env_var: true,
        exclude_slash_tmp: true,
    };
    let file_system_sandbox_policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &sandbox_policy,
        &session_configuration.cwd,
    );
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
                &file_system_sandbox_policy,
                NetworkSandboxPolicy::from(&sandbox_policy),
            ),
        )
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(project_root.to_path_buf()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(updated.workspace_roots, vec![project_root.clone()]);
    assert!(
        updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(project_root.as_path(), updated.cwd.as_path()),
        "cwd-only update should keep the new cwd writable"
    );
    assert!(
        !updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(original_cwd.as_path(), updated.cwd.as_path()),
        "cwd-only update should not keep the old implicit cwd writable"
    );
}

#[tokio::test]
async fn session_configuration_apply_preserves_absolute_cwd_write_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let original_cwd = workspace.path().join("repo-a");
    let next_cwd = workspace.path().join("repo-b");
    std::fs::create_dir_all(&original_cwd).expect("create original cwd");
    std::fs::create_dir_all(&next_cwd).expect("create next cwd");
    let original_cwd = original_cwd.abs();

    session_configuration.cwd = original_cwd.clone();
    let file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: original_cwd.clone(),
            },
            access: FileSystemAccessMode::Write,
        },
    ]);
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::Managed,
                &file_system_sandbox_policy,
                NetworkSandboxPolicy::Restricted,
            ),
        )
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(next_cwd.clone()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.file_system_sandbox_policy(),
        file_system_sandbox_policy
    );
    assert!(
        updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(original_cwd.as_path(), updated.cwd.as_path()),
        "absolute grant to the old cwd must remain writable"
    );
    assert!(
        !updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(next_cwd.as_path(), updated.cwd.as_path()),
        "cwd-only update must not reinterpret an absolute old-cwd grant as :workspace_roots"
    );
}

#[tokio::test]
async fn session_update_settings_does_not_rewrite_sticky_environment_cwds() {
    let (session, turn_context) = make_session_and_context().await;
    #[allow(deprecated)]
    let updated_cwd = turn_context.cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let session_cwd = {
        let state = session.state.lock().await;
        state.session_configuration.cwd.clone()
    };
    let config = session.get_config().await;
    let next_turn = session.new_default_turn().await;

    assert_eq!(session_cwd, updated_cwd);
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    #[allow(deprecated)]
    let next_turn_cwd = next_turn.cwd.clone();
    assert_eq!(config.cwd, turn_cwd);
    assert_eq!(next_turn_cwd, updated_cwd);
    assert_eq!(next_turn.config.cwd, updated_cwd);
}

#[tokio::test]
async fn relative_cwd_update_without_environments_resolves_under_session_cwd() {
    let (session, _turn_context) = make_session_and_context().await;
    let original_cwd = {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = Vec::new();
        state.session_configuration.cwd.clone()
    };
    let updated_cwd = original_cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let state = session.state.lock().await;
    assert_eq!(state.session_configuration.cwd, updated_cwd);
    assert!(state.session_configuration.environments.is_empty());
}

#[tokio::test]
async fn cwd_update_does_not_rewrite_sticky_environment_cwd() {
    let (session, _turn_context) = make_session_and_context().await;
    let (original_cwd, environment_cwd) = {
        let mut state = session.state.lock().await;
        let original_cwd = state.session_configuration.cwd.clone();
        let environment_cwd = original_cwd.join("environment");
        state.session_configuration.environments = vec![TurnEnvironmentSelection {
            environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
            cwd: environment_cwd.clone(),
        }];
        (original_cwd, environment_cwd)
    };
    let updated_cwd = original_cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let state = session.state.lock().await;
    assert_eq!(state.session_configuration.cwd, updated_cwd);
    assert_eq!(
        state.session_configuration.environments[0].cwd,
        environment_cwd
    );
}

#[tokio::test]
async fn absolute_cwd_update_with_turn_environment_is_allowed() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let absolute_cwd = {
        let state = session.state.lock().await;
        state.session_configuration.cwd.join("absolute-turn")
    };
    std::fs::create_dir_all(absolute_cwd.as_path()).expect("create absolute turn dir");

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                cwd: Some(absolute_cwd.to_path_buf()),
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
                    cwd: absolute_cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect("absolute cwd with explicit environments should succeed");

    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, absolute_cwd);
    assert_eq!(turn_context.config.cwd, absolute_cwd);
    assert_eq!(turn_context.environments.turn_environments.len(), 1);
}

#[tokio::test]
async fn session_new_fails_when_zsh_fork_enabled_without_zsh_path() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    config
        .features
        .enable(Feature::ShellZshFork)
        .expect("test config should allow shell_zsh_fork");
    config.zsh_path = None;
    let config = Arc::new(config);

    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, _rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let result = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        InitialHistory::New,
        SessionSource::Exec,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks_api::DisabledHookRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        AgentControl::default(),
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        /*state_db*/ None,
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await;

    let err = match result {
        Ok(_) => panic!("expected startup to fail"),
        Err(err) => err,
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("zsh fork feature enabled, but `zsh_path` is not configured"));
}

// todo: use online model info
pub(crate) async fn make_session_and_context() -> (Session, TurnContext) {
    let (tx_event, _rx_event) = async_channel::unbounded();
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let thread_id = ThreadId::default();
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let models_manager = models_manager_with_provider(
        config.codex_home.to_path_buf(),
        auth_manager.clone(),
        config.model_provider.clone(),
    );
    let agent_control = AgentControl::default();
    let exec_policy = Arc::new(ExecPolicyManager::default());
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };
    let per_turn_config =
        Session::build_per_turn_config(&session_configuration, session_configuration.cwd.clone());
    let model_info = construct_model_info_offline_for_tests(
        session_configuration.collaboration_mode.model(),
        &per_turn_config.to_models_manager_config(),
    );
    let session_telemetry = Arc::new(session_telemetry(
        thread_id,
        config.as_ref(),
        &model_info,
        session_configuration.session_source.clone(),
    )) as session_telemetry_api::SharedSessionTelemetry;

    let state = SessionState::new(session_configuration.clone());
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let network_approval: Arc<dyn SessionNetworkApprovalApi> =
        Arc::new(NetworkApprovalService::default());
    let environment: Arc<dyn exec_server_api::ExecEnvironment> = Arc::new(
        codex_exec_server::Environment::create_for_tests(/*exec_server_url*/ None)
            .expect("create environment"),
    );
    let command_service_state = Arc::new(command_service::CommandSessionState::new(
        config.background_terminal_max_timeout,
    ));
    let session_extension_data =
        codex_extension_api::ExtensionData::new(agent_control.session_id().to_string());
    session_extension_data.insert(command_service_state.manager_handle());
    let thread_extension_data = codex_extension_api::ExtensionData::new(thread_id.to_string());
    let provider_auth_manager =
        codex_login::model_provider_auth_manager(Some(auth_manager.clone()));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let api_runtime_factory: SharedApiRuntimeFactory =
        Arc::new(model_service::DefaultApiRuntimeFactory);
    let model_service: SharedModelServiceApi =
        Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::clone(&api_runtime_factory),
            provider_auth_manager: provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&model_provider_factory),
            default_provider: Some(session_configuration.provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: None,
        }));
    let model_client_api = model_service
        .create_client(CreateModelClientRequest {
            selection: ModelSelectionPolicy {
                requested_model: Some(session_configuration.collaboration_mode.model().to_string()),
                provider_hint: Some(config.model_provider_id.clone()),
                allow_default_fallback: true,
                refresh: ModelCatalogRefresh::OnlineIfUncached,
            },
            installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
            session_id: thread_id.into(),
            thread_id,
            session_source: session_configuration.session_source.clone(),
            reasoning_effort: session_configuration.collaboration_mode.reasoning_effort(),
            service_tier: crate::session::turn::model_service_tier(
                session_configuration.service_tier.as_deref(),
            ),
            verbosity: config.model_verbosity,
            chat_completions_max_tokens_by_model: config
                .model_options
                .iter()
                .filter(|model_option| model_option.provider == config.model_provider_id)
                .filter_map(|model_option| {
                    model_option
                        .max_tokens
                        .map(|max_tokens| (model_option.model.clone(), max_tokens))
                })
                .collect(),
            enable_request_compression: config.features.enabled(Feature::EnableRequestCompression),
            include_timing_metrics: config.features.enabled(Feature::RuntimeMetrics),
            beta_features_header: Session::build_model_client_beta_features_header(config.as_ref()),
        })
        .await
        .expect("create model client api for tests");

    let services = SessionServices {
        mcp_connection_manager: Arc::new(RwLock::new(Box::new(
            mcp_service::McpConnectionManager::new_uninitialized_with_permission_profile(
                &config.permissions.approval_policy,
                config.permissions.permission_profile(),
            ),
        ))),
        mcp_auth_runtime: Arc::new(mcp_service::DefaultMcpAuthRuntime),
        mcp_connection_runtime_factory: Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        network_proxy_runtime_factory: Arc::new(
            codex_network_proxy::DefaultNetworkProxyRuntimeFactory,
        ),
        mcp_startup_cancellation_token: Mutex::new(CancellationToken::new()),
        command_service_state,
        command_service_api: Arc::new(command_service::CommandService::new()),
        shell_zsh_path: None,
        main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
        analytics_events_client: AnalyticsEventsClient::disabled(),
        hooks: std::sync::RwLock::new(Arc::new(Hooks::new(HooksConfig {
            legacy_notify_argv: config.notify.clone(),
            ..HooksConfig::default()
        })) as Arc<dyn hooks_api::HookRuntime>),
        hook_runtime_factory: Arc::new(hooks::HooksRuntimeFactory),
        rollout_thread_trace: rollout_trace::ThreadTraceContext::disabled(),
        user_shell: Arc::new(default_user_shell()),
        shell_snapshot_tx: watch::channel(None).0,
        show_raw_agent_reasoning: config.show_raw_agent_reasoning,
        exec_policy,
        exec_policy_loader: Arc::new(crate::EmptyExecPolicyLoader),
        auth_runtime: auth_manager.clone(),
        provider_auth_manager,
        model_provider_factory,
        api_runtime_factory,
        session_telemetry_factory: Arc::new(codex_otel::OtelSessionTelemetryFactory),
        memory_tool_developer_instructions_provider: Arc::new(
            memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider,
        ),
        model_service,
        sandbox_runtime: Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        session_telemetry: session_telemetry.clone(),
        tool_approvals: Mutex::new(ApprovalStore::default()),
        guardian_rejections: Mutex::new(std::collections::HashMap::new()),
        guardian_rejection_circuit_breaker: Mutex::new(Default::default()),
        runtime_handle: tokio::runtime::Handle::current(),
        skill_service,
        plugins_manager,
        mcp_service: Arc::new(mcp_service::McpService::new(Arc::new(
            approval_service::ApprovalService,
        ))),
        extensions: Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        session_extension_data,
        thread_extension_data,
        agent_control,
        network_proxy: None,
        network_approval: Arc::clone(&network_approval),
        state_db: None,
        live_thread: None,
        thread_store: Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        live_thread_factory: Arc::new(thread_store::DefaultLiveThreadFactory),
        attestation_provider: None,
        active_event_subscriptions: Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        model_client_api,
        openai_file_uploader: Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        code_mode_service: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        code_mode_runtime_factory: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        approval_service: Arc::new(approval_service::ApprovalService),
        goal_service: Arc::new(goal_service::GoalService),
        tool_service: Arc::new(DisabledToolServiceForTests),
        environment_manager: Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    };

    let effective_skill_roots = services
        .plugins_manager
        .effective_skill_roots_for_config(&per_turn_config.plugins_config_input())
        .await;
    let skills_input =
        crate::build_skill_service_input_from_config(&per_turn_config, effective_skill_roots);
    let skill_fs = environment.get_filesystem();
    let skills_outcome = Arc::new(
        services
            .skill_service
            .skills_for_config(&skills_input, Some(Arc::clone(&skill_fs)))
            .await,
    );
    let available_models = models_manager
        .try_list_models()
        .expect("available models for tests");
    let turn_environments = turn_environments_for_tests(&environment, &session_configuration.cwd);
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let turn_context = Session::make_turn_context(
        thread_id,
        SessionId::from(thread_id),
        Some(auth_runtime),
        codex_login::model_provider_auth_manager(Some(Arc::clone(&auth_manager))),
        services.model_provider_factory.as_ref(),
        &session_telemetry,
        session_configuration.provider.clone(),
        &session_configuration,
        services.user_shell.as_ref(),
        services.shell_zsh_path.as_ref(),
        services.main_execve_wrapper_exe.as_ref(),
        per_turn_config,
        model_info,
        &available_models,
        /*network*/ None,
        turn_environments,
        session_configuration.cwd.clone(),
        "turn_id".to_string(),
        skills_outcome,
        /*goal_tools_supported*/ true,
    );

    let (mailbox, mailbox_rx) = crate::Mailbox::new();
    let session = Session {
        self_weak: std::sync::OnceLock::new(),
        conversation_id: thread_id,
        installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
        tx_event,
        agent_status: agent_status_tx,
        out_of_band_elicitation_paused: watch::channel(false).0,
        state: Mutex::new(state),
        managed_network_proxy_refresh_lock: Semaphore::new(/*permits*/ 1),
        features: config.features.clone(),
        pending_mcp_server_refresh_config: Mutex::new(None),
        conversation: Arc::new(RealtimeConversationManager::new()),
        active_turn: Mutex::new(None),
        mailbox,
        mailbox_rx: Mutex::new(mailbox_rx),
        idle_pending_input: Mutex::new(Vec::new()),
        last_parent_child_notification_status: Mutex::new(None),
        model_observed_display_events: Mutex::new(HashMap::new()),
        scheduler: Mutex::new(()),
        force_wait_command_for_tests: std::sync::atomic::AtomicBool::new(false),
        goal_continuation_before_launch_hook: Mutex::new(None),
        goal_runtime: codex_agent_runtime::GoalRuntimeState::new(),
        guardian_review_session: crate::session::session::approval_review_session_impl::GuardianReviewSessionManager::default(),
        services,
        next_internal_sub_id: AtomicU64::new(0),
        thread_wait: crate::session::thread_wait::ThreadWaitState::default(),
    };

    (session, turn_context)
}

async fn make_session_with_config(
    mutator: impl FnOnce(&mut Config),
) -> anyhow::Result<Arc<Session>> {
    let (session, _rx_event) = make_session_with_config_and_rx(mutator).await?;
    Ok(session)
}

async fn load_latest_config_for_session(session: &Session) -> Config {
    let config = session.get_config().await;
    ConfigBuilder::default()
        .codex_home(config.codex_home.to_path_buf())
        .fallback_cwd(Some(config.cwd.to_path_buf()))
        .build()
        .await
        .expect("load latest config for session")
}

async fn make_session_with_config_and_rx(
    mutator: impl FnOnce(&mut Config),
) -> anyhow::Result<(Arc<Session>, async_channel::Receiver<Event>)> {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    mutator(&mut config);
    let config = Arc::new(config);
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let session = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        InitialHistory::New,
        SessionSource::Exec,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks::HooksRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        AgentControl::default(),
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        /*state_db*/ None,
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await?;

    Ok((session, rx_event))
}

async fn make_session_with_history_source_and_agent_control_and_rx(
    initial_history: InitialHistory,
    session_source: SessionSource,
    agent_control: AgentControl,
) -> anyhow::Result<(Arc<Session>, async_channel::Receiver<Event>)> {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    config.ephemeral = true;
    let config = Arc::new(config);
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approval_policy_is_session_override: false,
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        permission_profile_is_session_override: false,
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: session_source.clone(),
        thread_source: None,
        root_agent_metadata: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let state_db =
        state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
            .await
            .expect("state db should initialize");

    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let session = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        initial_history,
        session_source,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks_api::DisabledHookRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        agent_control,
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            Some(state_db.clone()),
        )),
        Some(state_db),
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await?;

    Ok((session, rx_event))
}

#[tokio::test]
async fn resumed_root_session_uses_thread_id_as_session_id() {
    let thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await
    .expect("resume should succeed");

    assert_eq!(session.thread_id(), thread_id);
    assert_eq!(session.session_id(), SessionId::from(thread_id));

    let event = rx_event.recv().await.expect("session configured event");
    let EventMsg::SessionConfigured(event) = event.msg else {
        panic!("expected session configured event");
    };
    assert_eq!(event.session_id, SessionId::from(thread_id));
    assert_eq!(event.thread_id, thread_id);
}

#[tokio::test]
async fn resumed_subagent_session_keeps_inherited_session_id() {
    let parent_thread_id = ThreadId::new();
    let parent_session_id = SessionId::from(parent_thread_id);
    let thread_id = ThreadId::new();
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth: 1,
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
    });
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        session_source,
        AgentControl::default().with_session_id(parent_session_id),
    )
    .await
    .expect("resume should succeed");

    assert_eq!(session.thread_id(), thread_id);
    assert_eq!(session.session_id(), parent_session_id);

    let event = rx_event.recv().await.expect("session configured event");
    let EventMsg::SessionConfigured(event) = event.msg else {
        panic!("expected session configured event");
    };
    assert_eq!(event.session_id, parent_session_id);
    assert_eq!(event.thread_id, thread_id);
}

#[tokio::test]
async fn notify_request_permissions_response_ignores_unmatched_call_id() {
    let (session, _turn_context) = make_session_and_context().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());

    session
        .notify_request_permissions_response(
            "missing",
            protocol::request_permissions::RequestPermissionsResponse {
                permissions: RequestPermissionProfile {
                    network: Some(protocol::models::NetworkPermissions {
                        enabled: Some(true),
                    }),
                    ..RequestPermissionProfile::default()
                },
                scope: PermissionGrantScope::Turn,
                strict_auto_review: false,
            },
        )
        .await;

    assert_eq!(session.granted_turn_permissions().await, None);
}

#[tokio::test]
async fn record_granted_request_permissions_for_turn_uses_originating_turn() {
    let (session, _turn_context) = make_session_and_context().await;
    let originating_active_turn = ActiveTurn::default();
    let originating_turn_state = Arc::clone(&originating_active_turn.turn_state);
    *session.active_turn.lock().await = Some(originating_active_turn);

    let current_active_turn = ActiveTurn::default();
    let current_turn_state = Arc::clone(&current_active_turn.turn_state);
    *session.active_turn.lock().await = Some(current_active_turn);

    let requested_permissions = RequestPermissionProfile {
        network: Some(protocol::models::NetworkPermissions {
            enabled: Some(true),
        }),
        ..RequestPermissionProfile::default()
    };
    session
        .record_granted_request_permissions_for_turn(
            &protocol::request_permissions::RequestPermissionsResponse {
                permissions: requested_permissions.clone(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: false,
            },
            Some(&originating_turn_state),
        )
        .await;

    assert_eq!(
        originating_turn_state.lock().await.granted_permissions(),
        Some(requested_permissions.into())
    );
    assert_eq!(current_turn_state.lock().await.granted_permissions(), None);
    assert_eq!(session.granted_turn_permissions().await, None);
}

#[tokio::test]
async fn enable_strict_auto_review_for_turn_uses_originating_turn() {
    let (session, _turn_context) = make_session_and_context().await;
    let originating_active_turn = ActiveTurn::default();
    let originating_turn_state = Arc::clone(&originating_active_turn.turn_state);
    *session.active_turn.lock().await = Some(originating_active_turn);

    let requested_permissions = RequestPermissionProfile {
        network: Some(protocol::models::NetworkPermissions {
            enabled: Some(true),
        }),
        ..RequestPermissionProfile::default()
    };
    session
        .record_granted_request_permissions_for_turn(
            &protocol::request_permissions::RequestPermissionsResponse {
                permissions: requested_permissions.clone(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: true,
            },
            Some(&originating_turn_state),
        )
        .await;

    assert!(
        originating_turn_state
            .lock()
            .await
            .strict_auto_review_enabled()
    );
}

#[tokio::test]
async fn request_permissions_emits_event_when_granular_policy_allows_requests() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let expected_response = protocol::request_permissions::RequestPermissionsResponse {
        permissions: RequestPermissionProfile {
            network: Some(protocol::models::NetworkPermissions {
                enabled: Some(true),
            }),
            ..RequestPermissionProfile::default()
        },
        scope: PermissionGrantScope::Turn,
        strict_auto_review: false,
    };

    let handle = tokio::spawn({
        let session = Arc::clone(&session);
        let turn_context = Arc::clone(&turn_context);
        let call_id = call_id.clone();
        async move {
            session
                .request_permissions(
                    &turn_context,
                    call_id,
                    protocol::request_permissions::RequestPermissionsArgs {
                        reason: Some("need network".to_string()),
                        permissions: RequestPermissionProfile {
                            network: Some(protocol::models::NetworkPermissions {
                                enabled: Some(true),
                            }),
                            ..RequestPermissionProfile::default()
                        },
                    },
                    CancellationToken::new(),
                )
                .await
        }
    });

    let request_event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
        .await
        .expect("request_permissions event timed out")
        .expect("request_permissions event missing");
    let EventMsg::RequestPermissions(request) = request_event.msg else {
        panic!("expected request_permissions event");
    };
    assert_eq!(request.call_id, call_id);
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(request.cwd, Some(turn_cwd));

    session
        .notify_request_permissions_response(&request.call_id, expected_response.clone())
        .await;

    let response = tokio::time::timeout(StdDuration::from_secs(1), handle)
        .await
        .expect("request_permissions future timed out")
        .expect("request_permissions join error");

    assert_eq!(response, Some(expected_response));
}

#[tokio::test]
async fn request_permissions_response_materializes_session_cwd_grants_before_recording() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let requested_permissions = RequestPermissionProfile {
        file_system: Some(FileSystemPermissions {
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            }],
            glob_scan_max_depth: None,
        }),
        ..Default::default()
    };

    let handle = tokio::spawn({
        let session = Arc::clone(&session);
        let turn_context = Arc::clone(&turn_context);
        let call_id = call_id.clone();
        let requested_permissions = requested_permissions.clone();
        async move {
            session
                .request_permissions(
                    &turn_context,
                    call_id,
                    protocol::request_permissions::RequestPermissionsArgs {
                        reason: Some("need cwd write".to_string()),
                        permissions: requested_permissions,
                    },
                    CancellationToken::new(),
                )
                .await
        }
    });

    let request_event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
        .await
        .expect("request_permissions event timed out")
        .expect("request_permissions event missing");
    let EventMsg::RequestPermissions(request) = request_event.msg else {
        panic!("expected request_permissions event");
    };
    let request_cwd = request.cwd.clone().expect("request cwd");

    session
        .notify_request_permissions_response(
            &request.call_id,
            protocol::request_permissions::RequestPermissionsResponse {
                permissions: request.permissions,
                scope: PermissionGrantScope::Session,
                strict_auto_review: false,
            },
        )
        .await;

    let expected_permissions = RequestPermissionProfile {
        file_system: Some(FileSystemPermissions::from_read_write_roots(
            /*read*/ None,
            Some(vec![request_cwd]),
        )),
        ..Default::default()
    };
    let expected_response = protocol::request_permissions::RequestPermissionsResponse {
        permissions: expected_permissions.clone(),
        scope: PermissionGrantScope::Session,
        strict_auto_review: false,
    };

    let response = tokio::time::timeout(StdDuration::from_secs(1), handle)
        .await
        .expect("request_permissions future timed out")
        .expect("request_permissions join error");

    assert_eq!(response, Some(expected_response));
    assert_eq!(
        session.granted_session_permissions().await,
        Some(expected_permissions.into())
    );
}
