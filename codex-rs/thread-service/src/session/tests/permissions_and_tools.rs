#[tokio::test]
async fn request_permissions_is_auto_denied_when_granular_policy_blocks_tool_requests() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: false,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let response = session
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
        .await;

    assert_eq!(
        response,
        Some(protocol::request_permissions::RequestPermissionsResponse {
            permissions: RequestPermissionProfile::default(),
            scope: PermissionGrantScope::Turn,
            strict_auto_review: false,
        })
    );
    assert!(
        tokio::time::timeout(StdDuration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "request_permissions should not emit an event when granular.request_permissions is false"
    );
}

#[tokio::test]
async fn submit_with_id_captures_current_span_trace_context() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(1);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let codex = Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: completed_session_loop_termination(),
    };

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let expected_trace = async {
        let expected_trace =
            current_span_w3c_trace_context().expect("current span should have trace context");
        codex
            .submit_with_id(Submission {
                id: "sub-1".into(),
                op: Op::Interrupt,
                trace: None,
            })
            .await
            .expect("submit should succeed");
        expected_trace
    }
    .instrument(request_span)
    .await;

    let submitted = rx_sub.recv().await.expect("submission");
    assert_eq!(submitted.trace, Some(expected_trace));
}

#[tokio::test]
async fn new_default_turn_captures_current_span_trace_id() {
    let (session, _turn_context) = make_session_and_context().await;

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let turn_context_item = async {
        let expected_trace_id = Span::current()
            .context()
            .span()
            .span_context()
            .trace_id()
            .to_string();
        let turn_context = session.new_default_turn().await;
        let turn_context_item = turn_context.to_turn_context_item();
        assert_eq!(turn_context_item.trace_id, Some(expected_trace_id));
        turn_context_item
    }
    .instrument(request_span)
    .await;

    assert_eq!(
        turn_context_item.trace_id.as_deref(),
        Some("00000000000000000000000000000011")
    );
}

#[test]
fn submission_dispatch_span_prefers_submission_trace_context() {
    let _trace_test_context = install_test_tracing("codex-core-tests");

    let ambient_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000033-0000000000000044-01".into()),
        tracestate: None,
    };
    let ambient_span = info_span!("ambient");
    assert!(set_parent_from_w3c_trace_context(
        &ambient_span,
        &ambient_parent
    ));

    let submission_trace = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000055-0000000000000066-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let dispatch_span = ambient_span.in_scope(|| {
        submission_dispatch_span(&Submission {
            id: "sub-1".into(),
            op: Op::Interrupt,
            trace: Some(submission_trace),
        })
    });

    let trace_id = dispatch_span.context().span().span_context().trace_id();
    assert_eq!(
        trace_id,
        TraceId::from_hex("00000000000000000000000000000055").expect("trace id")
    );
}

#[test]
fn submission_dispatch_span_uses_debug_for_realtime_audio() {
    let _trace_test_context = install_test_tracing("codex-core-tests");

    let dispatch_span = submission_dispatch_span(&Submission {
        id: "sub-1".into(),
        op: Op::RealtimeConversationAudio(ConversationAudioParams {
            frame: RealtimeAudioFrame {
                data: "ZmFrZQ==".into(),
                sample_rate: 16_000,
                num_channels: 1,
                samples_per_channel: Some(160),
                item_id: None,
            },
        }),
        trace: None,
    });

    assert_eq!(
        dispatch_span.metadata().expect("span metadata").level(),
        &tracing::Level::DEBUG
    );
}

#[test]
fn op_kind_distinguishes_turn_ops() {
    assert_eq!(
        Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        }
        .kind(),
        "override_turn_context"
    );
    assert_eq!(
        Op::UserInput {
            environments: None,
            items: vec![],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        }
        .kind(),
        "user_input"
    );
    assert_eq!(
        Op::UserInputWithTurnContext {
            environments: None,
            items: vec![],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            cwd: None,
            workspace_roots: None,
            profile_workspace_roots: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            active_permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            model_provider: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        }
        .kind(),
        "user_input_with_turn_context"
    );
}

#[tokio::test]
async fn user_turn_updates_approvals_reviewer() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let config = session.get_config().await;

    handlers::user_input_or_turn(
        &session,
        "sub-1".to_string(),
        Op::UserTurn {
            environments: None,
            items: vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: config.cwd.to_path_buf(),
            approval_policy: config.permissions.approval_policy.value(),
            approvals_reviewer: Some(config_service::types::ApprovalsReviewer::AutoReview),
            sandbox_policy: config.legacy_sandbox_policy(),
            permission_profile: None,
            model: turn_context.model_info.slug.clone(),
            effort: config.model_reasoning_effort,
            summary: config.model_reasoning_summary,
            service_tier: None,
            final_output_json_schema: None,
            collaboration_mode: None,
            personality: config.personality,
        },
    )
    .await;

    let state = session.state.lock().await;
    assert_eq!(
        state.session_configuration.approvals_reviewer,
        config_service::types::ApprovalsReviewer::AutoReview
    );
}

#[tokio::test]
async fn turn_environments_set_primary_environment() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let selected_cwd =
        AbsolutePathBuf::try_from(session.get_config().await.cwd.as_path().join("selected"))
            .expect("absolute path");

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: "local".to_string(),
                    cwd: selected_cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect("turn should start");

    let turn_environments = &turn_context.environments;
    assert_eq!(turn_environments.turn_environments.len(), 1);
    let turn_environment = turn_context
        .environments
        .primary()
        .expect("primary environment should be set");
    assert!(std::sync::Arc::ptr_eq(
        &turn_environment.environment,
        &turn_environments.turn_environments[0].environment
    ));
    assert!(!turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd.as_path(), selected_cwd.as_path());
    assert_eq!(turn_context.config.cwd.as_path(), selected_cwd.as_path());
}

#[tokio::test]
async fn default_turn_overlays_session_cwd_onto_stored_thread_environments() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let session_cwd = session.get_config().await.cwd.clone();
    let selected_cwd =
        AbsolutePathBuf::try_from(session_cwd.as_path().join("selected")).expect("absolute path");

    {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = vec![TurnEnvironmentSelection {
            environment_id: "local".to_string(),
            cwd: selected_cwd.clone(),
        }];
    }

    let turn_context = session.new_default_turn().await;

    let turn_environments = &turn_context.environments;
    assert_eq!(turn_environments.turn_environments.len(), 1);
    let turn_environment = turn_context
        .environments
        .primary()
        .expect("primary environment should be set");
    assert!(std::sync::Arc::ptr_eq(
        &turn_environment.environment,
        &turn_environments.turn_environments[0].environment
    ));
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session_cwd);
    assert_eq!(turn_context.config.cwd, session_cwd);
}

#[tokio::test]
async fn default_turn_honors_empty_stored_thread_environments() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let session_cwd = session.get_config().await.cwd.clone();

    {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = Vec::new();
    }

    let turn_context = session.new_default_turn().await;

    assert!(turn_context.environments.primary().is_none());
    assert!(turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session_cwd);
    assert_eq!(turn_context.config.cwd, session_cwd);
    assert_eq!(turn_context.environments.turn_environments.len(), 0);
}

#[tokio::test]
async fn primary_environment_uses_first_turn_environment() {
    let (_session, mut turn_context) = make_session_and_context().await;
    let first_environment = turn_context.environments.turn_environments[0].clone();
    #[allow(deprecated)]
    let second_cwd = turn_context.cwd.join("second");
    turn_context
        .environments
        .turn_environments
        .push(TurnEnvironment {
            environment_id: "second".to_string(),
            environment: Arc::clone(&first_environment.environment),
            cwd: second_cwd.clone(),
            shell: None,
        });

    assert_eq!(
        turn_context
            .environments
            .primary()
            .expect("primary environment")
            .environment_id,
        first_environment.environment_id
    );
    assert_eq!(
        turn_context
            .environments
            .turn_environments
            .iter()
            .find(|environment| environment.environment_id == "second")
            .expect("second environment")
            .cwd,
        second_cwd
    );
    assert_eq!(turn_context.environments.turn_environments.len(), 2);
    assert_eq!(
        turn_context.environments.turn_environments[1].cwd,
        second_cwd
    );
}

#[tokio::test]
async fn empty_turn_environments_clear_primary_environment() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![]),
                ..Default::default()
            },
        )
        .await
        .expect("turn should start");

    assert!(turn_context.environments.primary().is_none());
    assert!(turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session.get_config().await.cwd);
    assert_eq!(turn_context.config.cwd, session.get_config().await.cwd);
}

#[tokio::test]
async fn unknown_turn_environment_returns_error() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let original_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };

    let err = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: "missing".to_string(),
                    cwd: original_configuration.cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect_err("unknown environment should fail");

    let current_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
    assert!(err.to_string().contains("missing"));
    assert_eq!(current_configuration.cwd, original_configuration.cwd);
    assert_eq!(
        current_configuration.environments,
        original_configuration.environments
    );
}

#[tokio::test]
async fn duplicate_turn_environment_returns_error_without_mutating_session() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let original_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };

    let err = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![
                    TurnEnvironmentSelection {
                        environment_id: "local".to_string(),
                        cwd: original_configuration.cwd.clone(),
                    },
                    TurnEnvironmentSelection {
                        environment_id: "local".to_string(),
                        cwd: original_configuration.cwd.join("second"),
                    },
                ]),
                ..Default::default()
            },
        )
        .await
        .expect_err("duplicate environment should fail");

    let current_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
    assert!(err.to_string().contains("duplicate"));
    assert_eq!(current_configuration.cwd, original_configuration.cwd);
    assert_eq!(
        current_configuration.environments,
        original_configuration.environments
    );
}

#[tokio::test]
async fn spawn_task_turn_span_inherits_dispatch_trace_context() {
    struct TraceCaptureTask {
        captured_trace: Arc<std::sync::Mutex<Option<W3cTraceContext>>>,
    }

    impl SessionTask for TraceCaptureTask {
        fn kind(&self) -> TaskKind {
            TaskKind::Regular
        }

        fn span_name(&self) -> &'static str {
            "session_task.trace_capture"
        }

        async fn run(
            self: Arc<Self>,
            _session: Arc<SessionTaskContext>,
            _ctx: Arc<TurnContext>,
            _input: Vec<UserInput>,
            _cancellation_token: CancellationToken,
        ) -> Option<String> {
            let mut trace = self
                .captured_trace
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *trace = current_span_w3c_trace_context();
            None
        }
    }

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = tracing::info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let submission_trace =
        async { current_span_w3c_trace_context().expect("request span should have trace context") }
            .instrument(request_span)
            .await;

    let dispatch_span = submission_dispatch_span(&Submission {
        id: "sub-1".into(),
        op: Op::Interrupt,
        trace: Some(submission_trace.clone()),
    });
    let dispatch_span_id = dispatch_span.context().span().span_context().span_id();

    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let captured_trace = Arc::new(std::sync::Mutex::new(None));

    async {
        sess.spawn_task(
            Arc::clone(&tc),
            vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }],
            TraceCaptureTask {
                captured_trace: Arc::clone(&captured_trace),
            },
        )
        .await;
    }
    .instrument(dispatch_span)
    .await;

    let evt = tokio::time::timeout(StdDuration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for turn completion")
        .expect("event");
    assert!(matches!(evt.msg, EventMsg::TurnComplete(_)));

    let task_trace = captured_trace
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .expect("turn task should capture the current span trace context");
    let submission_context =
        codex_otel::context_from_w3c_trace_context(&submission_trace).expect("submission");
    let task_context = codex_otel::context_from_w3c_trace_context(&task_trace).expect("task trace");

    assert_eq!(
        task_context.span().span_context().trace_id(),
        submission_context.span().span_context().trace_id()
    );
    assert_ne!(
        task_context.span().span_context().span_id(),
        dispatch_span_id
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn shutdown_complete_does_not_append_to_thread_store_after_shutdown() {
    let (mut session, _turn_context) = make_session_and_context().await;
    let store = Arc::new(thread_store::InMemoryThreadStore::default());
    let thread_store: Arc<dyn thread_store_api::ThreadStore> = store.clone();
    let config = session.get_config().await;
    let live_thread = LiveThread::create(
        Arc::clone(&thread_store),
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
    session.services.thread_store = thread_store;
    session.services.live_thread = Some(Arc::new(live_thread));
    let session = Arc::new(session);

    assert!(handlers::shutdown(&session, "sub-1".to_string()).await);

    assert_eq!(
        thread_store::InMemoryThreadStoreCalls {
            create_thread: 1,
            shutdown_thread: 1,
            ..Default::default()
        },
        store.calls().await
    );
}

#[tokio::test]
async fn submission_loop_channel_close_emits_thread_stop_lifecycle() {
    struct SessionStopMarker;
    struct ThreadStopMarker;

    struct ThreadStopRecorder {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        expected_thread_id: ThreadId,
    }

    impl codex_extension_api::ThreadLifecycleContributor<crate::config::Config> for ThreadStopRecorder {
        fn on_thread_stop(&self, input: codex_extension_api::ThreadStopInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            assert!(input.session_store.get::<SessionStopMarker>().is_some());
            assert!(input.thread_store.get::<ThreadStopMarker>().is_some());
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.thread_lifecycle_contributor(Arc::new(ThreadStopRecorder {
        calls: Arc::clone(&calls),
        expected_thread_id: session.conversation_id,
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionStopMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadStopMarker);

    let (tx_sub, rx_sub) = async_channel::bounded(1);
    drop(tx_sub);
    let session = Arc::new(session);
    submission_loop(session, Arc::clone(&turn_context.config), rx_sub).await;

    assert_eq!(1, calls.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn submission_loop_channel_close_aborts_active_turn_before_thread_stop_lifecycle() {
    struct LifecycleRecorder {
        calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
        expected_thread_id: ThreadId,
        expected_turn_id: String,
    }

    impl codex_extension_api::ThreadLifecycleContributor<crate::config::Config> for LifecycleRecorder {
        fn on_thread_stop(&self, input: codex_extension_api::ThreadStopInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push("thread_stop");
        }
    }

    impl codex_extension_api::TurnLifecycleContributor for LifecycleRecorder {
        fn on_turn_abort(&self, input: codex_extension_api::TurnAbortInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            assert_eq!(self.expected_turn_id, input.turn_store.level_id());
            assert_eq!(TurnAbortReason::Interrupted, input.reason);
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push("turn_abort");
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = Arc::new(LifecycleRecorder {
        calls: Arc::clone(&calls),
        expected_thread_id: session.conversation_id,
        expected_turn_id: turn_context.sub_id.clone(),
    });
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.thread_lifecycle_contributor(recorder.clone());
    builder.turn_lifecycle_contributor(recorder);
    session.services.extensions = Arc::new(builder.build());

    let session = Arc::new(session);
    session
        .spawn_task(
            Arc::new(turn_context),
            Vec::new(),
            NeverEndingTask {
                kind: TaskKind::Regular,
                listen_to_cancellation_token: true,
            },
        )
        .await;

    let (tx_sub, rx_sub) = async_channel::bounded(1);
    drop(tx_sub);
    submission_loop(Arc::clone(&session), session.get_config().await, rx_sub).await;

    assert_eq!(
        vec!["turn_abort", "thread_stop"],
        *calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    );
}

#[tokio::test]
async fn shutdown_and_wait_allows_multiple_waiters() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(4);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = rx_sub.recv().await.expect("shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        tokio::time::sleep(StdDuration::from_millis(50)).await;
    });
    let codex = Arc::new(Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
    });

    let waiter_1 = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };
    let waiter_2 = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };

    waiter_1
        .await
        .expect("first shutdown waiter join")
        .expect("first shutdown waiter");
    waiter_2
        .await
        .expect("second shutdown waiter join")
        .expect("second shutdown waiter");
}

#[tokio::test]
async fn shutdown_and_wait_waits_when_shutdown_is_already_in_progress() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(4);
    drop(rx_sub);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let (shutdown_complete_tx, shutdown_complete_rx) = tokio::sync::oneshot::channel();
    let session_loop_handle = tokio::spawn(async move {
        let _ = shutdown_complete_rx.await;
    });
    let codex = Arc::new(Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
    });

    let waiter = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };

    tokio::time::sleep(StdDuration::from_millis(10)).await;
    assert!(!waiter.is_finished());

    shutdown_complete_tx
        .send(())
        .expect("session loop should still be waiting to terminate");

    waiter
        .await
        .expect("shutdown waiter join")
        .expect("shutdown waiter");
}

#[tokio::test]
async fn shutdown_and_wait_shuts_down_cached_guardian_subagent() {
    let (parent_session, parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);
    let parent_config = Arc::clone(&parent_turn_context.config);
    let (parent_tx_sub, parent_rx_sub) = async_channel::bounded(4);
    let (_parent_tx_event, parent_rx_event) = async_channel::unbounded();
    let (_parent_status_tx, parent_agent_status) = watch::channel(AgentStatus::PendingInit);
    let parent_session_for_loop = Arc::clone(&parent_session);
    let parent_session_loop_handle = tokio::spawn(async move {
        submission_loop(parent_session_for_loop, parent_config, parent_rx_sub).await;
    });
    let parent_codex = Codex {
        tx_sub: parent_tx_sub,
        rx_event: parent_rx_event,
        agent_status: parent_agent_status,
        session: Arc::clone(&parent_session),
        session_loop_termination: session_loop_termination_from_handle(parent_session_loop_handle),
    };

    let (child_session, _child_turn_context) = make_session_and_context().await;
    let (child_tx_sub, child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let (child_shutdown_tx, child_shutdown_rx) = tokio::sync::oneshot::channel();
    let child_session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = child_rx_sub
            .recv()
            .await
            .expect("child shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        child_shutdown_tx
            .send(())
            .expect("child shutdown signal should be delivered");
    });
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .cache_session_for_test(Arc::clone(&parent_session), child_codex, child_reuse_key)
        .await;

    parent_codex
        .shutdown_and_wait()
        .await
        .expect("parent shutdown should succeed");

    child_shutdown_rx
        .await
        .expect("guardian subagent should receive a shutdown op");
}

#[tokio::test]
async fn cached_guardian_subagent_exposes_its_rollout_path() {
    let (parent_session, _parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);

    let (mut child_session, _child_turn_context) = make_session_and_context().await;
    let child_rollout_path = attach_thread_persistence(&mut child_session).await;
    let (child_tx_sub, _child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let child_session_loop_handle = tokio::spawn(async {});
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .cache_session_for_test(Arc::clone(&parent_session), child_codex, child_reuse_key)
        .await;

    assert_eq!(
        parent_session
            .guardian_review_session
            .trunk_rollout_path()
            .await,
        Some(child_rollout_path)
    );
}

#[tokio::test]
async fn shutdown_and_wait_shuts_down_tracked_ephemeral_guardian_review() {
    let (parent_session, parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);
    let parent_config = Arc::clone(&parent_turn_context.config);
    let (parent_tx_sub, parent_rx_sub) = async_channel::bounded(4);
    let (_parent_tx_event, parent_rx_event) = async_channel::unbounded();
    let (_parent_status_tx, parent_agent_status) = watch::channel(AgentStatus::PendingInit);
    let parent_session_for_loop = Arc::clone(&parent_session);
    let parent_session_loop_handle = tokio::spawn(async move {
        submission_loop(parent_session_for_loop, parent_config, parent_rx_sub).await;
    });
    let parent_codex = Codex {
        tx_sub: parent_tx_sub,
        rx_event: parent_rx_event,
        agent_status: parent_agent_status,
        session: Arc::clone(&parent_session),
        session_loop_termination: session_loop_termination_from_handle(parent_session_loop_handle),
    };

    let (child_session, _child_turn_context) = make_session_and_context().await;
    let (child_tx_sub, child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let (child_shutdown_tx, child_shutdown_rx) = tokio::sync::oneshot::channel();
    let child_session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = child_rx_sub
            .recv()
            .await
            .expect("child shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        child_shutdown_tx
            .send(())
            .expect("child shutdown signal should be delivered");
    });
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .register_ephemeral_session_for_test(
            Arc::clone(&parent_session),
            child_codex,
            child_reuse_key,
        )
        .await;

    parent_codex
        .shutdown_and_wait()
        .await
        .expect("parent shutdown should succeed");

    child_shutdown_rx
        .await
        .expect("ephemeral guardian review should receive a shutdown op");
}

async fn make_session_and_context_with_auth_and_config_and_rx<F>(
    auth: CodexAuth,
    dynamic_tools: Vec<DynamicToolSpec>,
    configure_config: F,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
)
where
    F: FnOnce(&mut Config),
{
    let codex_home = tempfile::tempdir().expect("create temp dir");
    make_session_and_context_with_auth_config_home_and_rx(
        auth,
        dynamic_tools,
        codex_home.path(),
        configure_config,
    )
    .await
}

async fn make_session_and_context_with_auth_config_home_and_rx<F>(
    auth: CodexAuth,
    dynamic_tools: Vec<DynamicToolSpec>,
    codex_home: &Path,
    configure_config: F,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
)
where
    F: FnOnce(&mut Config),
{
    let (tx_event, rx_event) = async_channel::unbounded();
    let mut config = build_test_config(codex_home).await;
    configure_config(&mut config);
    let state_db: Option<crate::StateDbHandle> = if config.features.enabled(Feature::Goals) {
        Some(
            state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                .await
                .expect("goal tests should initialize sqlite state db")
                as crate::StateDbHandle,
        )
    } else {
        None
    };
    let config = Arc::new(config);
    let thread_id = ThreadId::default();
    let auth_manager = AuthManager::from_auth_for_testing(auth);
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
        dynamic_tools,
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
        codex_login::model_provider_auth_manager(Some(Arc::clone(&auth_manager)));
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
        state_db: state_db.clone(),
        live_thread: None,
        thread_store: Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            state_db,
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
    let session = Arc::new(Session {
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
        last_system_error_message: Mutex::new(None),
        model_observed_display_events: Mutex::new(HashMap::new()),
        scheduler: Mutex::new(()),
        force_wait_command_for_tests: std::sync::atomic::AtomicBool::new(false),
        goal_continuation_before_launch_hook: Mutex::new(None),
        goal_runtime: codex_agent_runtime::GoalRuntimeState::new(),
        guardian_review_session: crate::session::session::approval_review_session_impl::GuardianReviewSessionManager::default(),
        services,
        next_internal_sub_id: AtomicU64::new(0),
        thread_wait: crate::session::thread_wait::ThreadWaitState::default(),
    });
    let _ = session.self_weak.set(Arc::downgrade(&session));
    let mut turn_context = turn_context;
    turn_context.session = Arc::downgrade(&session);
    let turn_context = Arc::new(turn_context);

    (session, turn_context, rx_event)
}

pub(crate) async fn make_session_and_context_with_dynamic_tools_and_rx(
    dynamic_tools: Vec<DynamicToolSpec>,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
) {
    make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        dynamic_tools,
        |_config| {},
    )
    .await
}

async fn make_goal_session_and_context_with_rx() -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
    tempfile::TempDir,
) {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let (session, turn_context, rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config
                .features
                .enable(Feature::Goals)
                .expect("goal mode should be enableable in tests");
        },
    )
    .await;
    upsert_goal_test_thread(session.as_ref()).await;
    (session, turn_context, rx, codex_home)
}

#[tokio::test]
async fn active_goal_runtime_can_reserve_idle_turn_for_continuation() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Write a benchmark note".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

    sess.maybe_continue_goal_if_idle_runtime().await;

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if sess.active_turn.lock().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("goal continuation should reserve an idle turn");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
    Ok(())
}

#[tokio::test]
async fn goal_continuation_reservation_clears_if_goal_stops_before_launch() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Write a benchmark note".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should be persisted");
    let goal_id = goal.goal_id.clone();

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    *sess.goal_continuation_before_launch_hook.lock().await = Some(Arc::new(
        crate::session::session::GoalContinuationBeforeLaunchHook {
            started_tx: Mutex::new(Some(started_tx)),
            continue_rx: Mutex::new(Some(continue_rx)),
        },
    ));

    let continuation = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.maybe_continue_goal_if_idle_runtime().await;
        })
    };
    started_rx
        .await
        .expect("continuation should reserve before launch");

    state_db
        .update_thread_goal(
            sess.conversation_id,
            state_api::ThreadGoalUpdate {
                objective: None,
                status: Some(state_api::ThreadGoalStatus::Paused),
                token_budget: None,
                expected_goal_id: Some(goal_id.clone()),
            },
        )
        .await?
        .expect("goal pause should succeed");

    continue_tx
        .send(())
        .expect("continuation hook should still be waiting");
    continuation.await.expect("continuation task should exit");
    assert!(
        sess.active_turn.lock().await.is_none(),
        "stale goal continuation reservation should be cleared"
    );

    Ok(())
}

#[tokio::test]
async fn goal_continuation_reservation_keeps_new_mailbox_input_out_of_reserved_turn()
-> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Write a benchmark note".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    *sess.goal_continuation_before_launch_hook.lock().await = Some(Arc::new(
        crate::session::session::GoalContinuationBeforeLaunchHook {
            started_tx: Mutex::new(Some(started_tx)),
            continue_rx: Mutex::new(Some(continue_rx)),
        },
    ));

    let continuation = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.maybe_continue_goal_if_idle_runtime().await;
        })
    };
    started_rx
        .await
        .expect("continuation should reserve before launch");

    let reserved_turn_state = {
        let active_turn = sess.active_turn.lock().await;
        let active_turn = active_turn
            .as_ref()
            .expect("goal continuation should reserve an active turn");
        assert!(
            active_turn.tasks.is_empty(),
            "hook should pause before start_task launches the continuation task"
        );
        Arc::clone(&active_turn.turn_state)
    };

    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "new mailbox input".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(true);
    assert!(
        !sess.enqueue_mailbox_communication(communication).await,
        "reserved continuation should not claim that mailbox input started a turn"
    );
    assert!(
        sess.has_pending_mailbox_items().await,
        "new mailbox input should remain buffered for a later turn"
    );

    let reserved_pending_input = reserved_turn_state.lock().await.pending_input().to_vec();
    assert_eq!(1, reserved_pending_input.len());
    let PendingInputItem::HookInspectable(ResponseItem::Message { content, .. }) =
        &reserved_pending_input[0]
    else {
        panic!("expected reserved continuation input to stay isolated");
    };
    let [ContentItem::InputText { text }] = content.as_slice() else {
        panic!("expected one goal continuation text item");
    };
    assert!(text.contains("<goal_context>"));

    continue_tx
        .send(())
        .expect("continuation hook should still be waiting");
    continuation
        .await
        .expect("continuation task should finish launching");
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test]
async fn goal_continuation_reservation_keeps_queued_input_for_follow_up_turn() -> anyhow::Result<()>
{
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Write a benchmark note".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    *sess.goal_continuation_before_launch_hook.lock().await = Some(Arc::new(
        crate::session::session::GoalContinuationBeforeLaunchHook {
            started_tx: Mutex::new(Some(started_tx)),
            continue_rx: Mutex::new(Some(continue_rx)),
        },
    ));

    let continuation = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.maybe_continue_goal_if_idle_runtime().await;
        })
    };
    started_rx
        .await
        .expect("continuation should reserve before launch");

    let queued_item = PendingInputItem::from(ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "queued after reservation".to_string(),
        }],
        phase: None,
    });
    sess.queue_response_items_for_next_turn(vec![queued_item])
        .await;
    assert!(
        sess.has_queued_response_items_for_next_turn().await,
        "queued input should remain pending until a later regular turn"
    );

    continue_tx
        .send(())
        .expect("continuation hook should still be waiting");
    continuation
        .await
        .expect("continuation task should finish launching");
    assert!(
        sess.has_queued_response_items_for_next_turn().await,
        "continuation launch should not consume queued next-turn input"
    );
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

async fn upsert_goal_test_thread(session: &Session) {
    let config = session.get_config().await;
    let state_db = session
        .state_db()
        .expect("goal test session should have a state db");
    let mut builder = state::ThreadMetadataBuilder::new(
        session.conversation_id,
        config
            .codex_home
            .join("goal-test-rollout.jsonl")
            .to_path_buf(),
        chrono::Utc::now(),
        SessionSource::Cli,
    );
    builder.cwd = config.cwd.to_path_buf();
    builder.model_provider = Some(config.model_provider_id.clone());
    let metadata = builder.build(config.model_provider_id.as_str());
    state_db
        .upsert_thread(&metadata)
        .await
        .expect("goal test thread should be upserted");
}

// Like make_session_and_context, but returns Arc<Session> and the event receiver
// so tests can assert on emitted events.
pub(crate) async fn make_session_and_context_with_rx() -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
) {
    make_session_and_context_with_dynamic_tools_and_rx(Vec::new()).await
}

#[tokio::test]
async fn refresh_mcp_servers_is_deferred_until_next_turn() {
    let (session, turn_context) = make_session_and_context().await;
    let old_token = session.mcp_startup_cancellation_token().await;
    assert!(!old_token.is_cancelled());

    let mcp_oauth_credentials_store_mode =
        serde_json::to_value(OAuthCredentialsStoreMode::Auto).expect("serialize store mode");
    let refresh_config = McpServerRefreshConfig {
        mcp_servers: json!({}),
        mcp_oauth_credentials_store_mode,
    };
    {
        let mut guard = session.pending_mcp_server_refresh_config.lock().await;
        *guard = Some(refresh_config);
    }

    assert!(!old_token.is_cancelled());
    assert!(
        session
            .pending_mcp_server_refresh_config
            .lock()
            .await
            .is_some()
    );

    session
        .refresh_mcp_servers_if_requested(&turn_context, /*elicitation_reviewer*/ None)
        .await;

    assert!(old_token.is_cancelled());
    assert!(
        session
            .pending_mcp_server_refresh_config
            .lock()
            .await
            .is_none()
    );
    let new_token = session.mcp_startup_cancellation_token().await;
    assert!(!new_token.is_cancelled());
}

#[tokio::test]
async fn spawn_task_does_not_update_previous_turn_settings_for_non_run_turn_tasks() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    sess.set_previous_turn_settings(/*previous_turn_settings*/ None)
        .await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];

    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
    assert_eq!(sess.previous_turn_settings().await, None);
}

#[tokio::test]
async fn build_settings_update_items_emits_environment_item_for_network_changes() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;

    let mut config = (*current_context.config).clone();
    let mut requirements = config.config_layer_stack.requirements().clone();
    requirements.network = Some(Sourced::new(
        NetworkConstraints {
            domains: Some(NetworkDomainPermissionsToml {
                entries: std::collections::BTreeMap::from([
                    (
                        "api.example.com".to_string(),
                        NetworkDomainPermissionToml::Allow,
                    ),
                    (
                        "blocked.example.com".to_string(),
                        NetworkDomainPermissionToml::Deny,
                    ),
                ]),
            }),
            ..Default::default()
        },
        RequirementSource::CloudRequirements,
    ));
    let layers = config
        .config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .cloned()
        .collect();
    config.config_layer_stack = ConfigLayerStack::new(
        layers,
        requirements,
        config.config_layer_stack.requirements_toml().clone(),
    )
    .expect("rebuild config layer stack with network requirements");
    current_context.config = Arc::new(config);

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let environment_update = user_input_texts(&update_items)
        .into_iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("environment update item should be emitted");
    assert!(environment_update.contains(
        "<network enabled=\"true\"><allowed>api.example.com</allowed><denied>blocked.example.com</denied></network>"
    ));
}

#[tokio::test]
async fn environment_context_uses_session_shell_when_environment_shell_is_absent() {
    let (mut session, mut turn_context) = make_session_and_context().await;
    session.services.user_shell = Arc::new(crate::runtime_shell_model::Shell {
        shell_type: crate::runtime_shell_model::ShellType::PowerShell,
        shell_path: PathBuf::from("powershell"),
        shell_snapshot: crate::runtime_shell_model::empty_shell_snapshot_receiver(),
    });
    for environment in &mut turn_context.environments.turn_environments {
        environment.shell = None;
    }

    let session_shell = session.user_shell();
    let environment_context = crate::context::environment_context_from_turn_context(
        &turn_context,
        session_shell.as_ref(),
    )
    .render();
    assert!(
        environment_context.contains("<shell>powershell</shell>"),
        "{environment_context}"
    );

    let primary_environment = turn_context
        .environments
        .turn_environments
        .first_mut()
        .expect("primary environment");
    primary_environment.shell = Some("cmd".to_string());

    let environment_context = crate::context::environment_context_from_turn_context(
        &turn_context,
        session_shell.as_ref(),
    )
    .render();
    assert!(
        environment_context.contains("<shell>cmd</shell>"),
        "{environment_context}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_environment_item_for_time_changes() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.current_date = Some("2026-02-27".to_string());
    current_context.timezone = Some("Europe/Berlin".to_string());

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let environment_update = user_input_texts(&update_items)
        .into_iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("environment update item should be emitted");
    assert!(environment_update.contains("<current_date>2026-02-27</current_date>"));
    assert!(environment_update.contains("<timezone>Europe/Berlin</timezone>"));
}

#[tokio::test]
async fn build_settings_update_items_omits_environment_item_when_disabled() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    let mut config = (*current_context.config).clone();
    config.include_environment_context = false;
    current_context.config = Arc::new(config);
    current_context.current_date = Some("2026-02-27".to_string());

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let user_texts = user_input_texts(&update_items);
    assert!(
        !user_texts
            .iter()
            .any(|text| text.contains("<environment_context>")),
        "did not expect environment context updates when disabled, got {user_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_realtime_start_when_session_becomes_live() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = true;

    let update_items = session
        .build_settings_update_items(
            Some(&previous_context.to_turn_context_item()),
            &current_context,
        )
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected a realtime start update, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_realtime_end_when_session_stops_being_live() {
    let (session, mut previous_context) = make_session_and_context().await;
    previous_context.realtime_active = true;
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = false;

    let update_items = session
        .build_settings_update_items(
            Some(&previous_context.to_turn_context_item()),
            &current_context,
        )
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected a realtime end update, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_uses_previous_turn_settings_for_realtime_end() {
    let (session, previous_context) = make_session_and_context().await;
    let mut previous_context_item = previous_context.to_turn_context_item();
    previous_context_item.realtime_active = None;
    let previous_turn_settings = PreviousTurnSettings {
        model: previous_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = false;

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let update_items = session
        .build_settings_update_items(Some(&previous_context_item), &current_context)
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected a realtime end update from previous turn settings, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_uses_previous_realtime_state() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.realtime_active = true;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected initial context to describe active realtime state, got {developer_texts:?}"
    );

    let previous_context_item = turn_context.to_turn_context_item();
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(Some(previous_context_item));
    }
    let resumed_context = session.build_initial_context(&turn_context).await;
    let resumed_developer_texts = developer_input_texts(&resumed_context);
    assert!(
        !resumed_developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "did not expect a duplicate realtime update, got {resumed_developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_emits_standalone_multiagent_context() {
    let (session, turn_context) = make_session_and_context().await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let user_texts = user_input_texts(&initial_context);
    let environment_context = user_texts
        .iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("expected environment context");
    let multiagent_context = user_texts
        .iter()
        .find(|text| text.contains("<multiagent_context>"))
        .expect("expected multiagent context");

    assert!(
        !environment_context.contains("<subagents>"),
        "did not expect subagents in environment context, got {environment_context}"
    );
    assert!(
        multiagent_context
            .contains("<current_thread_canonical_path>/root</current_thread_canonical_path>"),
        "expected root canonical path in multiagent context, got {multiagent_context}"
    );
}

#[test]
fn external_agent_tool_specs_context_section_filters_native_model_api_tools() {
    let provider_schema =
        tool_service_api::JsonSchema::string(Some("External agent provider.".to_string()));
    let spawn_external_agent =
        tool_service_api::ToolSpec::Function(tool_service_api::ResponsesApiTool {
            name: "spawn_external_agent".to_string(),
            description: "Spawn an external code agent.".to_string(),
            strict: false,
            defer_loading: None,
            parameters: tool_service_api::JsonSchema::object(
                std::collections::BTreeMap::from([("provider".to_string(), provider_schema)]),
                Some(vec!["provider".to_string()]),
                /*additional_properties*/ None,
            ),
            output_schema: None,
        });
    let mut specs = vec![spawn_external_agent];
    for tool_name in [
        "followup_external_task",
        "poll_external_event",
        "list_external_agents",
        "close_external_agent",
    ] {
        specs.push(tool_service_api::ToolSpec::Function(
            tool_service_api::ResponsesApiTool {
                name: tool_name.to_string(),
                description: "External code-agent tool.".to_string(),
                strict: false,
                defer_loading: None,
                parameters: tool_service_api::JsonSchema::object(
                    std::collections::BTreeMap::new(),
                    /*required*/ None,
                    /*additional_properties*/ None,
                ),
                output_schema: None,
            },
        ));
    }
    for tool_name in [
        "exec_command",
        "apply_patch",
        "spawn_agent",
        "followup_task",
    ] {
        specs.push(tool_service_api::ToolSpec::Function(
            tool_service_api::ResponsesApiTool {
                name: tool_name.to_string(),
                description: "Native model API tool.".to_string(),
                strict: false,
                defer_loading: None,
                parameters: tool_service_api::JsonSchema::object(
                    std::collections::BTreeMap::new(),
                    /*required*/ None,
                    /*additional_properties*/ None,
                ),
                output_schema: None,
            },
        ));
    }

    let tool_specs_section = Session::external_agent_tool_specs_context_section(&specs)
        .expect("expected external agent tool specs section");

    for tool_name in [
        "spawn_external_agent",
        "followup_external_task",
        "poll_external_event",
        "list_external_agents",
        "close_external_agent",
    ] {
        assert!(
            tool_specs_section.contains(tool_name),
            "expected external tool spec for {tool_name}, got {tool_specs_section}"
        );
    }
    for tool_name in [
        "exec_command",
        "apply_patch",
        "spawn_agent",
        "followup_task",
    ] {
        assert!(
            !tool_specs_section.contains(&format!("\"name\": \"{tool_name}\"")),
            "did not expect native model API tool {tool_name} in external section, got {tool_specs_section}"
        );
    }
    assert!(
        tool_specs_section.contains("\"parameters\""),
        "expected serialized tool parameters schema, got {tool_specs_section}"
    );
    assert!(
        tool_specs_section.contains("\"provider\""),
        "expected spawn_external_agent provider schema, got {tool_specs_section}"
    );
    assert!(
        tool_specs_section.contains("<external_agent_tools>")
            && tool_specs_section.contains("</external_agent_tools>"),
        "expected external agent tools wrapper, got {tool_specs_section}"
    );
    assert!(
        !tool_specs_section.contains("<model_visible_tools>"),
        "did not expect legacy model-visible tools wrapper, got {tool_specs_section}"
    );
    assert!(
        tool_specs_section.contains("独立的外部 CLI agent 协作总线")
            && tool_specs_section.contains("模型 API tool config"),
        "expected Chinese external tool bus guidance, got {tool_specs_section}"
    );
}

#[test]
fn external_agent_tool_specs_context_section_truncates_non_ascii_on_char_boundary() {
    let spec = tool_service_api::ToolSpec::Function(tool_service_api::ResponsesApiTool {
        name: "spawn_external_agent".to_string(),
        description: "说明".repeat(30_000),
        strict: false,
        defer_loading: None,
        parameters: tool_service_api::JsonSchema::object(
            std::collections::BTreeMap::new(),
            /*required*/ None,
            /*additional_properties*/ None,
        ),
        output_schema: None,
    });

    let section = Session::external_agent_tool_specs_context_section(&[spec])
        .expect("expected tool specs section");

    assert!(
        section.contains("... truncated ..."),
        "expected truncated marker, got section with len {}",
        section.len()
    );
    assert!(
        section.contains("<external_agent_tools>"),
        "expected valid external agent tools wrapper"
    );
}

#[tokio::test]
async fn build_initial_context_uses_root_scope_agent_metadata_path() {
    let (session, turn_context) = make_session_and_context().await;
    {
        let mut state = session.state.lock().await;
        state.session_configuration.root_agent_metadata =
            Some(codex_agent_runtime::AgentMetadata {
                agent_path: Some(
                    protocol::AgentPath::try_from("/owner_dev").expect("valid agent path"),
                ),
                agent_role: Some("feature-owner".to_string()),
                ..Default::default()
            });
    }

    let initial_context = session.build_initial_context(&turn_context).await;
    let user_texts = user_input_texts(&initial_context);
    let multiagent_context = user_texts
        .iter()
        .find(|text| text.contains("<multiagent_context>"))
        .expect("expected multiagent context");

    assert!(
        multiagent_context
            .contains("<current_thread_canonical_path>/owner_dev</current_thread_canonical_path>"),
        "expected root-scope agent canonical path in multiagent context, got {multiagent_context}"
    );
}

async fn make_multi_agent_v2_usage_hint_test_session() -> (Arc<Session>, Arc<TurnContext>) {
    let (session, turn_context, _rx_event) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.multi_agent_v2.root_agent_usage_hint_text = Some("Root guidance.".to_string());
            config.multi_agent_v2.subagent_usage_hint_text = Some("Subagent guidance.".to_string());
        },
    )
    .await;
    (session, turn_context)
}

struct PromptExtensionTestContributor;
struct PromptExtensionTestState;

impl codex_extension_api::ContextContributor for PromptExtensionTestContributor {
    fn contribute<'a>(
        &'a self,
        _session_store: &'a codex_extension_api::ExtensionData,
        thread_store: &'a codex_extension_api::ExtensionData,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Vec<codex_extension_api::PromptFragment>> + Send + 'a>,
    > {
        Box::pin(async move {
            thread_store
                .get::<PromptExtensionTestState>()
                .is_some()
                .then(|| {
                    codex_extension_api::PromptFragment::developer_policy(
                        "prompt extension enabled",
                    )
                })
                .into_iter()
                .collect()
        })
    }
}

fn prompt_extension_test_registry()
-> Arc<codex_extension_api::ExtensionRegistry<crate::config::Config>> {
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.prompt_contributor(Arc::new(PromptExtensionTestContributor));
    Arc::new(builder.build())
}

#[tokio::test]
async fn build_initial_context_includes_prompt_fragments_from_extensions() {
    let (mut session, turn_context) = make_session_and_context().await;
    session.services.extensions = prompt_extension_test_registry();
    session
        .services
        .thread_extension_data
        .insert(PromptExtensionTestState);

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_messages = developer_message_texts(&initial_context);

    assert!(
        developer_messages
            .iter()
            .flatten()
            .any(|text| *text == "prompt extension enabled"),
        "expected prompt extension developer text, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_omits_prompt_fragments_without_extension_state() {
    let (mut session, turn_context) = make_session_and_context().await;
    session.services.extensions = prompt_extension_test_registry();

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_messages = developer_message_texts(&initial_context);

    assert!(
        !developer_messages
            .iter()
            .flatten()
            .any(|text| *text == "prompt extension enabled"),
        "did not expect prompt extension developer text, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_root_usage_hint_as_developer_message() {
    let (session, turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "expected standalone root usage hint developer message, got {developer_messages:?}"
    );
    assert!(
        !developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Subagent guidance."]),
        "did not expect subagent usage hint for root thread, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_subagent_usage_hint_as_developer_message() {
    let (session, mut turn_context) = make_multi_agent_v2_usage_hint_test_session().await;
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: ThreadId::new(),
        depth: 1,
        agent_path: Some(AgentPath::try_from("/root/worker").expect("agent path should parse")),
        agent_nickname: Some("worker".to_string()),
        agent_role: None,
    });
    session
        .state
        .lock()
        .await
        .session_configuration
        .session_source = session_source.clone();
    Arc::get_mut(&mut turn_context)
        .expect("turn context should not be shared")
        .session_source = session_source;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Subagent guidance."]),
        "expected standalone subagent usage hint developer message, got {developer_messages:?}"
    );
    assert!(
        !developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "did not expect root usage hint for subagent thread, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_usage_hints_when_feature_disabled() {
    let (session, turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "expected root usage hint even when legacy feature is disabled, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn configured_multi_agent_v2_usage_hint_texts_returns_configured_texts() {
    let (session, _turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let hint_texts = session.configured_multi_agent_v2_usage_hint_texts().await;

    assert_eq!(
        hint_texts,
        vec![
            "Root guidance.".to_string(),
            "Subagent guidance.".to_string()
        ]
    );
}

#[tokio::test]
async fn build_initial_context_omits_default_image_save_location_with_image_history() {
    let (session, turn_context) = make_session_and_context().await;
    session
        .replace_history(
            vec![ResponseItem::ImageGenerationCall {
                id: "ig-test".to_string(),
                status: "completed".to_string(),
                revised_prompt: Some("a tiny blue square".to_string()),
                result: "Zm9v".to_string(),
            }],
            /*reference_context_item*/ None,
        )
        .await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        !developer_texts
            .iter()
            .any(|text| text.contains("Generated images are saved to")),
        "expected initial context to omit image save instructions even with image history, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_omits_default_image_save_location_without_image_history() {
    let (session, turn_context) = make_session_and_context().await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        !developer_texts
            .iter()
            .any(|text| text.contains("Generated images are saved to")),
        "expected initial context to omit image save instructions without image history, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_trims_skill_metadata_from_context_window_budget() {
    let (session, mut turn_context) = make_session_and_context().await;
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![
        SkillMetadata {
            name: "admin-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/admin-skill/SKILL.md").abs(),
            scope: SkillScope::Admin,
            plugin_id: None,
        },
        SkillMetadata {
            name: "repo-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
            scope: SkillScope::Repo,
            plugin_id: None,
        },
    ];
    turn_context.model_info.context_window = Some(100);
    turn_context.turn_skills = TurnSkillsContext::new(Arc::new(outcome));

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("Exceeded skills context budget")),
        "expected skill budget warning to stay out of the initial context, got {developer_texts:?}"
    );
    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("- admin-skill:") && !text.contains("- repo-skill:")),
        "expected no skill metadata entries to fit the tiny budget, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_loads_skills_from_current_cwd_local_roots() {
    fn write_skill(root: &Path, dir: &str, name: &str, description: &str) {
        let skill_dir = root.join(dir);
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n"),
        )
        .expect("write skill");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let child_cwd = codex_home.path().join("child");
    std::fs::create_dir_all(&child_cwd).expect("create child cwd");
    write_skill(
        &child_cwd.join(".codex/skills"),
        "cwd-dot-codex",
        "cwd-dot-codex-skill",
        "from cwd .codex",
    );
    write_skill(
        &child_cwd.join(".agents/skills"),
        "cwd-dot-agents",
        "cwd-dot-agents-skill",
        "from cwd .agents",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = child_cwd.abs();
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("- cwd-dot-codex-skill:")),
        "expected cwd .codex skill in initial context, got {developer_texts:?}"
    );
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("- cwd-dot-agents-skill:")),
        "expected cwd .agents skill in initial context, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_loads_project_workflows() {
    fn write_workflow(root: &Path, id: &str, description: &str) {
        let workflow_dir = root.join(id);
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
        std::fs::write(
            workflow_dir.join("WORKFLOW.md"),
            format!(
                r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    write_workflow(
        &repo_cwd.join(".codex/workflows"),
        "feature-dev",
        "structured feature workflow",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = repo_cwd.abs();
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<workflows_instructions>")
                && text.contains("- feature-dev (project)")
                && text.contains("structured feature workflow")
                && text
                    .contains("Use this workflow when feature work needs a structured process.")),
        "expected project workflow in initial context, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_loads_disabled_project_workflows_for_display_only() {
    fn write_workflow(root: &Path, id: &str, description: &str) {
        let workflow_dir = root.join(id);
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
        std::fs::write(
            workflow_dir.join("WORKFLOW.md"),
            format!(
                r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    let dot_codex = repo_cwd.join(".codex");
    let active_cwd = codex_home.path().join("active-cwd");
    std::fs::create_dir_all(&active_cwd).expect("create active cwd");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    write_workflow(
        &dot_codex.join("workflows"),
        "feature-dev",
        "structured feature workflow",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = active_cwd.abs();
            config.config_layer_stack = config_service::ConfigLayerStack::new(
                vec![config_service::ConfigLayerEntry::new_disabled(
                    codex_config_types::ConfigLayerSource::Project {
                        dot_codex_folder: dot_codex.abs(),
                    },
                    toml::Value::Table(toml::map::Map::new()),
                    "disabled".to_string(),
                )],
                Default::default(),
                Default::default(),
            )
            .expect("config layer stack");
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<workflows_instructions>")
                && text.contains("- feature-dev (project)")
                && text.contains("structured feature workflow")),
        "expected disabled project workflow in initial context, got {developer_texts:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn build_initial_context_skips_disabled_project_workflow_symlinks_that_escape_repo() {
    fn write_workflow(root: &Path, id: &str, description: &str) {
        let workflow_dir = root.join(id);
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
        std::fs::write(
            workflow_dir.join("WORKFLOW.md"),
            format!(
                r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    let dot_codex = repo_cwd.join(".codex");
    let external_root = codex_home.path().join("external-workflow");
    let active_cwd = codex_home.path().join("active-cwd");
    std::fs::create_dir_all(&active_cwd).expect("create active cwd");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    std::fs::create_dir_all(dot_codex.join("workflows")).expect("create workflows root");
    write_workflow(&external_root, "feature-dev", "structured feature workflow");
    std::os::unix::fs::symlink(
        &external_root,
        dot_codex.join("workflows").join("feature-dev"),
    )
    .expect("create escaping workflow symlink");

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = active_cwd.abs();
            config.config_layer_stack = config_service::ConfigLayerStack::new(
                vec![config_service::ConfigLayerEntry::new_disabled(
                    codex_config_types::ConfigLayerSource::Project {
                        dot_codex_folder: dot_codex.abs(),
                    },
                    toml::Value::Table(toml::map::Map::new()),
                    "disabled".to_string(),
                )],
                Default::default(),
                Default::default(),
            )
            .expect("config layer stack");
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("- feature-dev (project)")),
        "expected escaping workflow symlink to stay out of initial context, got {developer_texts:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn build_initial_context_skips_disabled_project_workflow_markdown_symlinks_that_escape_repo()
{
    fn write_workflow_entry(workflow_dir: &Path) {
        std::fs::create_dir_all(workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    let dot_codex = repo_cwd.join(".codex");
    let workflow_dir = dot_codex.join("workflows").join("feature-dev");
    let external_markdown_dir = codex_home.path().join("external-workflow-doc");
    let active_cwd = codex_home.path().join("active-cwd");
    std::fs::create_dir_all(&active_cwd).expect("create active cwd");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    std::fs::create_dir_all(&external_markdown_dir).expect("create external workflow doc dir");
    write_workflow_entry(&workflow_dir);
    std::fs::write(
        external_markdown_dir.join("WORKFLOW.md"),
        r#"---
id: feature-dev
name: Feature Development
description: escaped workflow
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#,
    )
    .expect("write external workflow markdown");
    std::os::unix::fs::symlink(
        external_markdown_dir.join("WORKFLOW.md"),
        workflow_dir.join("WORKFLOW.md"),
    )
    .expect("create escaping workflow markdown symlink");

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = active_cwd.abs();
            config.config_layer_stack = config_service::ConfigLayerStack::new(
                vec![config_service::ConfigLayerEntry::new_disabled(
                    codex_config_types::ConfigLayerSource::Project {
                        dot_codex_folder: dot_codex.abs(),
                    },
                    toml::Value::Table(toml::map::Map::new()),
                    "disabled".to_string(),
                )],
                Default::default(),
                Default::default(),
            )
            .expect("config layer stack");
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("- feature-dev (project)")),
        "expected escaping workflow markdown symlink to stay out of initial context, got {developer_texts:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn build_initial_context_skips_disabled_project_workflow_entry_symlinks_that_escape_repo() {
    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    let dot_codex = repo_cwd.join(".codex");
    let workflow_dir = dot_codex.join("workflows").join("feature-dev");
    let external_entry_dir = codex_home.path().join("external-workflow-entry");
    let active_cwd = codex_home.path().join("active-cwd");
    std::fs::create_dir_all(&active_cwd).expect("create active cwd");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
    std::fs::create_dir_all(&external_entry_dir).expect("create external workflow entry dir");
    std::fs::write(
        workflow_dir.join("WORKFLOW.md"),
        r#"---
id: feature-dev
name: Feature Development
description: escaped workflow entry
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#,
    )
    .expect("write workflow markdown");
    std::fs::write(
        external_entry_dir.join("workflow.ts"),
        "export default { external: true };",
    )
    .expect("write external workflow entry");
    std::os::unix::fs::symlink(
        external_entry_dir.join("workflow.ts"),
        workflow_dir.join("workflow.ts"),
    )
    .expect("create escaping workflow entry symlink");

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = active_cwd.abs();
            config.config_layer_stack = config_service::ConfigLayerStack::new(
                vec![config_service::ConfigLayerEntry::new_disabled(
                    codex_config_types::ConfigLayerSource::Project {
                        dot_codex_folder: dot_codex.abs(),
                    },
                    toml::Value::Table(toml::map::Map::new()),
                    "disabled".to_string(),
                )],
                Default::default(),
                Default::default(),
            )
            .expect("config layer stack");
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("- feature-dev (project)")),
        "expected escaping workflow entry symlink to stay out of initial context, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_keeps_enabled_workflow_when_disabled_project_duplicates_id() {
    fn write_workflow(root: &Path, id: &str, description: &str) {
        let workflow_dir = root.join(id);
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
        std::fs::write(
            workflow_dir.join("WORKFLOW.md"),
            format!(
                r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    let repo_dot_codex = repo_cwd.join(".codex");
    let disabled_repo_root = codex_home.path().join("disabled-repo");
    let disabled_dot_codex = disabled_repo_root.join(".codex");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    write_workflow(
        &repo_dot_codex.join("workflows"),
        "feature-dev",
        "enabled workflow description",
    );
    write_workflow(
        &disabled_dot_codex.join("workflows"),
        "feature-dev",
        "disabled workflow description",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = repo_cwd.abs();
            config.config_layer_stack = config_service::ConfigLayerStack::new(
                vec![config_service::ConfigLayerEntry::new_disabled(
                    codex_config_types::ConfigLayerSource::Project {
                        dot_codex_folder: disabled_dot_codex.abs(),
                    },
                    toml::Value::Table(toml::map::Map::new()),
                    "disabled".to_string(),
                )],
                Default::default(),
                Default::default(),
            )
            .expect("config layer stack");
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);
    let workflow_section = developer_texts
        .iter()
        .find(|text| text.contains("<workflows_instructions>"))
        .expect("expected workflows instructions section");

    assert!(
        workflow_section.contains("enabled workflow description"),
        "expected enabled workflow to remain visible, got {workflow_section:?}"
    );
    assert!(
        !workflow_section.contains("disabled workflow description"),
        "expected disabled duplicate workflow to stay hidden, got {workflow_section:?}"
    );
}

#[test]
fn emit_thread_start_skill_metrics_records_enabled_kept_and_truncated_values() {
    let session_telemetry = test_session_telemetry_without_metadata();
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![SkillMetadata {
        name: "repo-skill".to_string(),
        description: "desc".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    }];
    let rendered = build_available_skills(
        &outcome,
        SkillMetadataBudget::Characters(1),
        SkillRenderSideEffects::ThreadStart {
            session_telemetry: &session_telemetry,
        },
    )
    .expect("skills should render");

    assert_eq!(
        rendered.warning_message,
        Some(
            "Exceeded skills context budget. All skill descriptions were removed and 1 additional skill was not included in the model-visible skills list."
                .to_string()
        )
    );
    let snapshot = session_telemetry
        .snapshot_metrics()
        .expect("runtime metrics snapshot");
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_ENABLED_TOTAL_METRIC),
        1
    );
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_KEPT_TOTAL_METRIC), 0);
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_TRUNCATED_METRIC), 1);
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC),
        4
    );
}

#[test]
fn emit_thread_start_skill_metrics_records_description_truncated_chars_without_omitted_skills() {
    let session_telemetry = test_session_telemetry_without_metadata();
    let alpha = SkillMetadata {
        name: "alpha-skill".to_string(),
        description: "abcdef".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/alpha-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    };
    let beta = SkillMetadata {
        name: "beta-skill".to_string(),
        description: "uvwxyz".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/beta-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    };
    let minimum_skill_line_cost = |skill: &SkillMetadata| {
        let path = skill.path_to_skills_md.to_string_lossy().replace('\\', "/");
        format!("- {}: (file: {})\n", skill.name, path)
            .chars()
            .count()
    };
    let minimum_budget = minimum_skill_line_cost(&alpha) + minimum_skill_line_cost(&beta);
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![alpha, beta];

    let rendered = build_available_skills(
        &outcome,
        SkillMetadataBudget::Characters(minimum_budget + 6),
        SkillRenderSideEffects::ThreadStart {
            session_telemetry: &session_telemetry,
        },
    )
    .expect("skills should render");

    assert_eq!(rendered.report.omitted_count, 0);
    assert_eq!(rendered.report.truncated_description_chars, 8);
    let snapshot = session_telemetry
        .snapshot_metrics()
        .expect("runtime metrics snapshot");
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_TRUNCATED_METRIC), 0);
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC),
        8
    );
}

#[tokio::test]
async fn build_initial_context_emits_thread_start_skill_warning_on_repeated_builds() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    let mut turn_context = Arc::into_inner(turn_context).expect("sole turn context owner");
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![
        SkillMetadata {
            name: "admin-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/admin-skill/SKILL.md").abs(),
            scope: SkillScope::Admin,
            plugin_id: None,
        },
        SkillMetadata {
            name: "repo-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
            scope: SkillScope::Repo,
            plugin_id: None,
        },
    ];
    turn_context.model_info.context_window = Some(100);
    turn_context.turn_skills = TurnSkillsContext::new(Arc::new(outcome));

    let _ = session.build_initial_context(&turn_context).await;
    let warning_event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("warning event should arrive")
        .expect("warning event should be readable");
    assert!(matches!(
        warning_event.msg,
        EventMsg::Warning(WarningEvent { message })
            if message == "Exceeded skills context budget of 2%. All skill descriptions were removed and 2 additional skills were not included in the model-visible skills list."
    ));

    let _ = session.build_initial_context(&turn_context).await;
    let warning_event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("warning event should arrive on repeated build")
        .expect("warning event should be readable");
    assert!(matches!(
        warning_event.msg,
        EventMsg::Warning(WarningEvent { message })
            if message == "Exceeded skills context budget of 2%. All skill descriptions were removed and 2 additional skills were not included in the model-visible skills list."
    ));
}

#[tokio::test]
async fn handle_output_item_done_records_image_save_history_message() {
    let (session, turn_context) = make_session_and_context().await;
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "ig_history_records_message";
    let expected_saved_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        call_id,
    );
    let _ = std::fs::remove_file(&expected_saved_path);
    let item = ResponseItem::ImageGenerationCall {
        id: call_id.to_string(),
        status: "completed".to_string(),
        revised_prompt: Some("a tiny blue square".to_string()),
        result: "Zm9v".to_string(),
    };

    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(
            turn_context.sub_id.clone(),
        )),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };
    handle_output_item_done(&mut ctx, item.clone(), /*previously_active_item*/ None)
        .await
        .expect("image generation item should succeed");

    let history = session.clone_history().await;
    let image_output_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        "<image_id>",
    );
    let image_output_dir = image_output_path
        .parent()
        .expect("generated image path should have a parent");
    let image_message: ResponseItem =
        ContextualUserFragment::into(codex_context_manager::ImageGenerationInstructions::new(
            image_output_dir.display(),
            image_output_path.display(),
        ));
    assert_eq!(history.raw_items(), &[image_message, item]);
    assert_eq!(
        std::fs::read(&expected_saved_path).expect("saved file"),
        b"foo"
    );
    let _ = std::fs::remove_file(&expected_saved_path);
}

#[tokio::test]
async fn handle_output_item_done_skips_image_save_message_when_save_fails() {
    let (session, turn_context) = make_session_and_context().await;
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "ig_history_no_message";
    let expected_saved_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        call_id,
    );
    let _ = std::fs::remove_file(&expected_saved_path);
    let item = ResponseItem::ImageGenerationCall {
        id: call_id.to_string(),
        status: "completed".to_string(),
        revised_prompt: Some("broken payload".to_string()),
        result: "_-8".to_string(),
    };

    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(
            turn_context.sub_id.clone(),
        )),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };
    handle_output_item_done(&mut ctx, item.clone(), /*previously_active_item*/ None)
        .await
        .expect("image generation item should still complete");

    let history = session.clone_history().await;
    assert_eq!(history.raw_items(), &[item]);
    assert!(!expected_saved_path.exists());
}

#[tokio::test]
async fn build_initial_context_uses_previous_turn_settings_for_realtime_end() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_turn_settings = PreviousTurnSettings {
        model: turn_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected initial context to describe an ended realtime session, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_restates_realtime_start_when_reference_context_is_missing() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.realtime_active = true;
    let previous_turn_settings = PreviousTurnSettings {
        model: turn_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected initial context to restate active realtime when the reference context is missing, got {developer_texts:?}"
    );
}
