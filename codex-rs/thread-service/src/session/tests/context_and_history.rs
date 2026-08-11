fn file_system_policy_with_unreadable_glob(turn_context: &TurnContext) -> FileSystemSandboxPolicy {
    #[allow(deprecated)]
    let mut policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &turn_context.sandbox_policy(),
        &turn_context.cwd,
    );
    #[allow(deprecated)]
    let cwd_display = turn_context.cwd.as_path().display().to_string();
    policy.entries.push(FileSystemSandboxEntry {
        path: FileSystemPath::GlobPattern {
            pattern: format!("{cwd_display}/**/*.env"),
        },
        access: FileSystemAccessMode::None,
    });
    policy
}

#[tokio::test]
async fn turn_context_item_omits_legacy_equivalent_file_system_sandbox_policy() {
    let (_session, turn_context) = make_session_and_context().await;

    let item = turn_context.to_turn_context_item();

    assert_eq!(item.file_system_sandbox_policy, None);
    assert_eq!(
        item.permission_profile,
        Some(turn_context.permission_profile())
    );
}

#[tokio::test]
async fn turn_context_item_stores_split_file_system_sandbox_policy_when_different() {
    let (_session, mut turn_context) = make_session_and_context().await;
    let file_system_sandbox_policy = file_system_policy_with_unreadable_glob(&turn_context);
    turn_context.permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        turn_context.permission_profile.enforcement(),
        &file_system_sandbox_policy,
        turn_context.network_sandbox_policy(),
    );

    let item = turn_context.to_turn_context_item();

    assert_eq!(
        item.file_system_sandbox_policy,
        Some(file_system_sandbox_policy)
    );
    assert_eq!(
        item.permission_profile,
        Some(turn_context.permission_profile())
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_injects_full_context_when_baseline_missing()
 {
    let (session, turn_context) = make_session_and_context().await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    let history = session.clone_history().await;
    let initial_context = session
        .build_initial_context_for_external_agent_tools(&turn_context)
        .await;
    assert_eq!(history.raw_items().to_vec(), initial_context);

    let current_context = session.reference_context_item().await;
    assert_eq!(
        serde_json::to_value(current_context).expect("serialize current context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected context item")
    );
}

#[tokio::test]
async fn record_context_updates_emits_injected_context_with_agent_file_instructions() {
    let agent_file_instructions = "Agent type file body: always inspect the active task.";
    let role_dir = tempfile::tempdir().expect("agent role tempdir");
    let role_path = role_dir.path().join("project-pm.agent.md");
    std::fs::write(
        &role_path,
        format!(
            "---\nname: project-pm\ndescription: Project PM role.\n---\n{agent_file_instructions}\n"
        ),
    )
    .expect("write agent role file");
    let (session, mut turn_context, rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("test-api-key"),
        Vec::new(),
        |config| {
            config.agent_roles.insert(
                "project-pm".to_string(),
                crate::config::AgentRoleConfig {
                    description: Some("Project PM role.".to_string()),
                    source_path: Some(role_path.clone()),
                    ..Default::default()
                },
            );
        },
    )
    .await;
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: ThreadId::default(),
        depth: 1,
        agent_path: Some("/root/project_pm".parse().expect("agent path")),
        agent_nickname: Some("project_pm".to_string()),
        agent_role: Some("project-pm".to_string()),
    });
    {
        let mut state = session.state.lock().await;
        state.session_configuration.session_source = session_source.clone();
    }
    Arc::get_mut(&mut turn_context)
        .expect("turn context should not be shared")
        .session_source = session_source;

    session
        .record_context_updates_and_set_reference_context_item(turn_context.as_ref())
        .await;

    let mut injected_context = None;
    for _ in 0..10 {
        let event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting for injected context event")
            .expect("event");
        if let EventMsg::ItemCompleted(ItemCompletedEvent {
            item: TurnItem::InjectedContext(item),
            ..
        }) = event.msg
        {
            injected_context = Some(item);
            break;
        }
    }
    let injected_context = injected_context.expect("expected injected context display item");

    assert_eq!(injected_context.title, "Init Context");
    assert!(
        injected_context
            .sections
            .iter()
            .any(|section| section.label == "Developer"
                && section.text.contains(agent_file_instructions)),
        "expected injected context to include agent file instructions, got {injected_context:?}"
    );

    let reference_context_item = session
        .reference_context_item()
        .await
        .expect("expected reference context item");
    let reference_developer_instructions = reference_context_item
        .developer_instructions
        .expect("expected persisted developer instructions");
    assert!(
        reference_developer_instructions.contains(agent_file_instructions),
        "expected reference context to persist agent file instructions, got {reference_developer_instructions:?}"
    );
}

#[tokio::test]
async fn process_compacted_history_reinjects_agent_file_instructions_into_initial_context() {
    let agent_file_instructions = "Compact agent type body: keep role instructions visible.";
    let role_dir = tempfile::tempdir().expect("agent role tempdir");
    let role_path = role_dir.path().join("compact-role.agent.md");
    std::fs::write(
        &role_path,
        format!(
            "---\nname: compact-role\ndescription: Compact role.\n---\n{agent_file_instructions}\n"
        ),
    )
    .expect("write agent role file");
    let (session, mut turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("test-api-key"),
        Vec::new(),
        |config| {
            config.agent_roles.insert(
                "compact-role".to_string(),
                crate::config::AgentRoleConfig {
                    description: Some("Compact role.".to_string()),
                    source_path: Some(role_path.clone()),
                    ..Default::default()
                },
            );
        },
    )
    .await;
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: ThreadId::default(),
        depth: 1,
        agent_path: Some("/root/compact_role".parse().expect("agent path")),
        agent_nickname: Some("compact_role".to_string()),
        agent_role: Some("compact-role".to_string()),
    });
    {
        let mut state = session.state.lock().await;
        state.session_configuration.session_source = session_source.clone();
    }
    Arc::get_mut(&mut turn_context)
        .expect("turn context should not be shared")
        .session_source = session_source;
    let compacted_history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    }];

    let refreshed = crate::compact::process_compacted_history(
        &session,
        turn_context.as_ref(),
        compacted_history,
        crate::compact::InitialContextInjection::BeforeLastUserMessage,
    )
    .await;
    let refreshed_text = refreshed
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|part| match part {
                        ContentItem::InputText { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        refreshed_text.contains(agent_file_instructions),
        "expected compacted history to preserve agent file instructions, got {refreshed_text:?}"
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_reinjects_full_context_after_clear()
{
    let (session, turn_context) = make_session_and_context().await;
    let compacted_summary = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!("{}\nsummary", crate::compact::SUMMARY_PREFIX),
        }],
        phase: None,
    };
    session
        .record_into_history(std::slice::from_ref(&compacted_summary), &turn_context)
        .await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }
    session
        .replace_history(
            vec![compacted_summary.clone()],
            /*reference_context_item*/ None,
        )
        .await;

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;

    let history = session.clone_history().await;
    let mut expected_history = vec![compacted_summary];
    expected_history.extend(
        session
            .build_initial_context_for_external_agent_tools(&turn_context)
            .await,
    );
    assert_eq!(history.raw_items().to_vec(), expected_history);
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_baseline_without_emitting_diffs()
 {
    let (mut session, previous_context) = make_session_and_context().await;
    let next_model = if previous_context.model_info.slug == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let turn_context = previous_context
        .with_model(next_model.to_string(), &session.services.model_service)
        .await;
    let previous_context_item = previous_context.to_turn_context_item();
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(Some(previous_context_item.clone()));
    }
    let rollout_path = attach_thread_persistence(&mut session).await;

    let update_items = session
        .build_settings_update_items(Some(&previous_context_item), &turn_context)
        .await;
    assert_eq!(update_items, Vec::new());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;

    assert_eq!(
        session.clone_history().await.raw_items().to_vec(),
        Vec::new()
    );
    assert_eq!(
        serde_json::to_value(session.reference_context_item().await)
            .expect("serialize current context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected context item")
    );
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });
    assert_eq!(
        serde_json::to_value(persisted_turn_context)
            .expect("serialize persisted turn context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected turn context item")
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_split_file_system_policy_to_rollout()
 {
    let (mut session, mut turn_context) = make_session_and_context().await;
    let file_system_sandbox_policy = file_system_policy_with_unreadable_glob(&turn_context);
    turn_context.permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        turn_context.permission_profile.enforcement(),
        &file_system_sandbox_policy,
        turn_context.network_sandbox_policy(),
    );
    let rollout_path = attach_thread_persistence(&mut session).await;

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_file_system_sandbox_policy = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => ctx.file_system_sandbox_policy.clone(),
        _ => None,
    });
    assert_eq!(
        persisted_file_system_sandbox_policy,
        Some(file_system_sandbox_policy)
    );
}

#[tokio::test]
async fn build_initial_context_prepends_model_switch_message() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_turn_settings = PreviousTurnSettings {
        model: "previous-regular-model".to_string(),
        realtime_active: None,
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;

    let ResponseItem::Message { role, content, .. } = &initial_context[0] else {
        panic!("expected developer message");
    };
    assert_eq!(role, "developer");
    let [ContentItem::InputText { text }, ..] = content.as_slice() else {
        panic!("expected developer text");
    };
    assert!(text.contains("<model_switch>"));
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_full_reinjection_to_rollout()
 {
    let (mut session, previous_context) = make_session_and_context().await;
    let next_model = if previous_context.model_info.slug == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let turn_context = previous_context
        .with_model(next_model.to_string(), &session.services.model_service)
        .await;
    let rollout_path = attach_thread_persistence(&mut session).await;

    session
        .persist_rollout_items(&[RolloutItem::EventMsg(EventMsg::UserMessage(
            UserMessageEvent {
                message: "seed rollout".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        ))])
        .await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }

    session
        .set_previous_turn_settings(Some(PreviousTurnSettings {
            model: previous_context.model_info.slug.clone(),
            realtime_active: Some(previous_context.realtime_active),
        }))
        .await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });

    assert_eq!(
        serde_json::to_value(persisted_turn_context)
            .expect("serialize persisted turn context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected turn context item")
    );
}

#[tokio::test]
async fn run_user_shell_command_does_not_set_reference_context_item() {
    let (session, _turn_context, rx) = make_session_and_context_with_rx().await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }

    handlers::run_user_shell_command(&session, "sub-id".to_string(), "echo shell".to_string())
        .await;

    let deadline = StdDuration::from_secs(15);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        if matches!(evt.msg, EventMsg::TurnComplete(_)) {
            break;
        }
    }

    assert!(
        session.reference_context_item().await.is_none(),
        "standalone shell tasks should not mutate previous context"
    );
}

#[tokio::test]
async fn realtime_conversation_list_voices_emits_builtin_list() {
    let (session, _turn_context, rx) = make_session_and_context_with_rx().await;

    handlers::realtime_conversation_list_voices(&session, "sub-id".to_string()).await;

    let event = rx.recv().await.expect("event");
    let voices = match event.msg {
        EventMsg::RealtimeConversationListVoicesResponse(
            RealtimeConversationListVoicesResponseEvent { voices },
        ) => voices,
        msg => panic!("expected list voices response, got {msg:?}"),
    };
    assert_eq!(
        voices,
        RealtimeVoicesList {
            v1: vec![
                RealtimeVoice::Juniper,
                RealtimeVoice::Maple,
                RealtimeVoice::Spruce,
                RealtimeVoice::Ember,
                RealtimeVoice::Vale,
                RealtimeVoice::Breeze,
                RealtimeVoice::Arbor,
                RealtimeVoice::Sol,
                RealtimeVoice::Cove,
            ],
            v2: vec![
                RealtimeVoice::Alloy,
                RealtimeVoice::Ash,
                RealtimeVoice::Ballad,
                RealtimeVoice::Coral,
                RealtimeVoice::Echo,
                RealtimeVoice::Sage,
                RealtimeVoice::Shimmer,
                RealtimeVoice::Verse,
                RealtimeVoice::Marin,
                RealtimeVoice::Cedar,
            ],
            default_v1: RealtimeVoice::Cove,
            default_v2: RealtimeVoice::Marin,
        },
    );
}

#[derive(Clone, Copy)]
struct NeverEndingTask {
    kind: TaskKind,
    listen_to_cancellation_token: bool,
}

impl SessionTask for NeverEndingTask {
    fn kind(&self) -> TaskKind {
        self.kind
    }

    fn span_name(&self) -> &'static str {
        "session_task.never_ending"
    }

    async fn run(
        self: Arc<Self>,
        _session: Arc<SessionTaskContext>,
        _ctx: Arc<TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        if self.listen_to_cancellation_token {
            cancellation_token.cancelled().await;
            return None;
        }
        loop {
            sleep(Duration::from_secs(60)).await;
        }
    }
}

#[derive(Clone)]
struct CommandExitNotificationOnFinishTask {
    notification: ResponseItem,
    observed_event: EventMsg,
}

impl SessionTask for CommandExitNotificationOnFinishTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.command_exit_notification_on_finish"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        _ctx: Arc<TurnContext>,
        _input: Vec<UserInput>,
        _cancellation_token: CancellationToken,
    ) -> Option<String> {
        let session = session.clone_session();
        thread_service_api::ThreadSessionCapability::append_conversation_item_with_observed_event(
            session.as_ref(),
            self.notification.clone(),
            self.observed_event.clone(),
        )
        .await
        .expect("append command exit notification");
        Some("command still finishing".to_string())
    }
}

#[derive(Clone, Copy)]
struct GuardianDeniedApprovalTask;

impl SessionTask for GuardianDeniedApprovalTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.guardian_denied_approval"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let session = session.clone_session();
        for _ in 0..3 {
            crate::guardian::record_guardian_denial_for_test(&session, &ctx, &ctx.sub_id).await;
        }

        cancellation_token.cancelled().await;
        None
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_auto_review_interrupts_after_three_consecutive_denials() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "trigger guardian denials".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(Arc::clone(&tc), input, GuardianDeniedApprovalTask)
        .await;

    let mut observed = Vec::new();
    let aborted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::TurnAborted(event) = &event.msg {
                let event = event.clone();
                observed.push(EventMsg::TurnAborted(event.clone()));
                break event;
            }
            observed.push(event.msg);
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "guardian denial circuit breaker should interrupt the turn; observed events: {observed:?}"
        )
    });
    assert_eq!(aborted.reason, TurnAbortReason::Interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_helper_review_interrupts_after_three_consecutive_denials() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "keep turn active for helper reviews".to_string(),
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

    let session_for_review = Arc::clone(&sess);
    let turn_for_review = Arc::clone(&tc);
    let turn_id = tc.sub_id.clone();
    let review_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("helper review runtime");
        runtime.block_on(async move {
            for _ in 0..3 {
                crate::guardian::record_guardian_denial_for_test(
                    &session_for_review,
                    &turn_for_review,
                    &turn_id,
                )
                .await;
            }
        });
    });
    review_thread.join().expect("helper review thread");

    let mut observed = Vec::new();
    let aborted = timeout(StdDuration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::TurnAborted(event) = &event.msg {
                let event = event.clone();
                observed.push(EventMsg::TurnAborted(event.clone()));
                break event;
            }
            observed.push(event.msg);
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "helper review circuit breaker should interrupt the turn; observed events: {observed:?}"
        )
    });
    assert_eq!(aborted.reason, TurnAbortReason::Interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[test_log::test]
async fn abort_regular_task_emits_turn_aborted_only() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    // Interrupts persist a model-visible `<turn_aborted>` marker into history, but there is no
    // separate client-visible event for that marker (only `EventMsg::TurnAborted`).
    let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("event");
    match evt.msg {
        EventMsg::TurnAborted(e) => assert_eq!(TurnAbortReason::Interrupted, e.reason),
        other => panic!("unexpected event: {other:?}"),
    }
    // No extra events should be emitted after an abort.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn abort_gracefully_emits_turn_aborted_only() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
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

    // Even if tasks handle cancellation gracefully, interrupts still result in `TurnAborted`
    // being the only client-visible signal.
    let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("event");
    match evt.msg {
        EventMsg::TurnAborted(e) => assert_eq!(TurnAbortReason::Interrupted, e.reason),
        other => panic!("unexpected event: {other:?}"),
    }
    // No extra events should be emitted after an abort.
    assert!(rx.try_recv().is_err());
}

async fn recv_pending_input_lifecycle_event(rx: &async_channel::Receiver<Event>) -> Event {
    loop {
        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("expected pending input lifecycle event")
            .expect("channel open");
        if matches!(&event.msg, EventMsg::ThreadContextUsageUpdated(_)) {
            continue;
        }
        assert!(
            !matches!(&event.msg, EventMsg::RawResponseItem(_)),
            "pending input lifecycle should not emit raw response items"
        );
        return event;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_finish_emits_turn_item_lifecycle_for_leftover_pending_user_input() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    while rx.try_recv().is_ok() {}

    sess.inject_hook_inspectable_items(vec![ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    }])
    .await
    .expect("inject pending input into active turn");

    sess.on_task_finished(Arc::clone(&tc), /*last_agent_message*/ None)
        .await;

    let history = sess.clone_history().await;
    let expected = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    };
    assert!(
        history.raw_items().iter().any(|item| item == &expected),
        "expected pending input to be persisted into history on turn completion"
    );

    let first = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        first.msg,
        EventMsg::ItemStarted(ItemStartedEvent {
            item: TurnItem::UserMessage(UserMessageItem { content, .. }),
            ..
        }) if content == vec![UserInput::Text {
            text: "late pending input".to_string(),
            text_elements: Vec::new(),
        }]
    ));

    let second = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        second.msg,
        EventMsg::ItemCompleted(ItemCompletedEvent {
            item: TurnItem::UserMessage(UserMessageItem { content, .. }),
            ..
        }) if content == vec![UserInput::Text {
            text: "late pending input".to_string(),
            text_elements: Vec::new(),
        }]
    ));

    let third = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        third.msg,
        EventMsg::UserMessage(UserMessageEvent {
            message,
            images,
            text_elements,
            local_images,
            ..
        }) if message == "late pending input"
            && images == Some(Vec::new())
            && text_elements.is_empty()
            && local_images.is_empty()
    ));

    let fourth = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        fourth.msg,
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id,
            last_agent_message: None,
            time_to_first_token_ms: None,
            ..
        }) if turn_id == tc.sub_id
    ));
}

#[tokio::test]
async fn task_finish_restarts_turn_for_leftover_pending_user_input() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    sess.inject_hook_inspectable_items(vec![ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    }])
    .await
    .expect("inject pending input into active turn");

    sess.on_task_finished(Arc::clone(&tc), /*last_agent_message*/ None)
        .await;

    timeout(Duration::from_secs(5), async {
        loop {
            if sess.active_turn_context_and_cancellation_token().await.is_some() {
                break;
            }
            let event = rx.recv().await.expect("event");
            if matches!(
                event.msg,
                EventMsg::TurnComplete(TurnCompleteEvent { turn_id, .. }) if turn_id != tc.sub_id
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late pending input should restart a follow-up turn");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn task_finish_prioritizes_thread_pending_work_without_losing_leftover_input() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    sess.queue_response_items_for_next_turn(vec![PendingInputItem::from(
        ResponseInputItem::Message {
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "queued next turn work".to_string(),
            }],
            phase: None,
        },
    )])
    .await;

    sess.inject_hook_inspectable_items(vec![ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    }])
    .await
    .expect("inject pending input into active turn");

    sess.on_task_finished(Arc::clone(&tc), /*last_agent_message*/ None)
        .await;

    let history = sess.clone_history().await;
    let expected = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    };
    assert!(
        history.raw_items().iter().any(|item| item == &expected),
        "expected leftover pending input to be persisted before the follow-up turn starts"
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if sess.active_turn_context_and_cancellation_token().await.is_some() {
                break;
            }
            let event = rx.recv().await.expect("event");
            if matches!(
                event.msg,
                EventMsg::TurnComplete(TurnCompleteEvent { turn_id, .. }) if turn_id != tc.sub_id
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("thread-level pending work should restart a follow-up turn");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compact_task_continues_pending_input_with_regularized_metadata() {
    #[derive(Clone)]
    struct CompactContinuationProbeTask {
        started_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<()>>>>,
        continue_rx: Arc<tokio::sync::Mutex<Option<oneshot::Receiver<()>>>>,
        observed_regular_phase: Arc<tokio::sync::Mutex<bool>>,
        drained_inputs: Arc<tokio::sync::Mutex<Vec<PendingInputItem>>>,
    }

    impl SessionTask for CompactContinuationProbeTask {
        fn kind(&self) -> TaskKind {
            TaskKind::Compact
        }

        fn span_name(&self) -> &'static str {
            "session_task.compact_probe"
        }

        async fn run(
            self: Arc<Self>,
            session: Arc<SessionTaskContext>,
            ctx: Arc<TurnContext>,
            _input: Vec<UserInput>,
            cancellation_token: CancellationToken,
        ) -> Option<String> {
            if let Some(tx) = self.started_tx.lock().await.take() {
                let _ = tx.send(());
            }
            if let Some(rx) = self.continue_rx.lock().await.take() {
                let _ = rx.await;
            }

            let observed_regular_phase = Arc::clone(&self.observed_regular_phase);
            let drained_inputs = Arc::clone(&self.drained_inputs);
            crate::tasks::continue_compact_turn_after_success(
                session,
                ctx,
                cancellation_token,
                move |sess, ctx, _turn_extension_data, _cancellation_token| {
                    let observed_regular_phase = Arc::clone(&observed_regular_phase);
                    let drained_inputs = Arc::clone(&drained_inputs);
                    async move {
                        let active = sess.active_turn.lock().await;
                        let active_turn = active
                            .as_ref()
                            .expect("active turn should exist during compact continuation");
                        let (sub_id, task) =
                            active_turn.tasks.first().expect("active task should exist");
                        *observed_regular_phase.lock().await = *sub_id == ctx.sub_id
                            && task.kind == TaskKind::Regular
                            && task.records_turn_token_usage_on_span;
                        drop(active);
                        *drained_inputs.lock().await = sess.get_pending_input().await;
                        Some("continued".to_string())
                    }
                },
            )
            .await
        }
    }

    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let (started_tx, started_rx) = oneshot::channel();
    let (continue_tx, continue_rx) = oneshot::channel();
    let observed_regular_phase = Arc::new(tokio::sync::Mutex::new(false));
    let drained_inputs = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        CompactContinuationProbeTask {
            started_tx: Arc::new(tokio::sync::Mutex::new(Some(started_tx))),
            continue_rx: Arc::new(tokio::sync::Mutex::new(Some(continue_rx))),
            observed_regular_phase: Arc::clone(&observed_regular_phase),
            drained_inputs: Arc::clone(&drained_inputs),
        },
    )
    .await;

    started_rx
        .await
        .expect("probe compact task should signal that it has started");

    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "compact mailbox continuation".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.enqueue_mailbox_communication(communication.clone()).await;

    continue_tx
        .send(())
        .expect("probe compact task should still be waiting for continuation");

    let completed = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::TurnComplete(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("compact continuation probe should complete the same turn");

    assert_eq!(completed.turn_id, tc.sub_id);
    assert!(
        *observed_regular_phase.lock().await,
        "compact continuation should switch the active task metadata to regular before draining pending input"
    );
    assert_eq!(
        *drained_inputs.lock().await,
        vec![PendingInputItem::from(communication)],
        "compact continuation should consume the pending mailbox input within the same turn"
    );
    timeout(Duration::from_secs(5), async {
        loop {
            if sess.active_turn.lock().await.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("compact continuation probe should eventually clear the active turn");
}

#[tokio::test]
async fn trigger_turn_mailbox_input_starts_idle_turn() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;

    crate::session::handlers::inter_agent_communication(
        &sess,
        "idle-trigger-turn".to_string(),
        InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("worker path should parse"),
            AgentPath::root(),
            Vec::new(),
            "wake idle turn".to_string(),
            protocol::protocol::InterAgentOperation::Unknown,
        )
        .with_trigger_turn(true),
    )
    .await;

    timeout(Duration::from_secs(2), async {
        loop {
            if sess.active_turn_context_and_cancellation_token().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("trigger-turn mailbox input should start an idle turn");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_event_driven_tool_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let trigger = EventDrivenToolTrigger {
        tool: "fs_subscribe".to_string(),
        title: "File watch triggered".to_string(),
        text: "build.log changed".to_string(),
    };

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::EventDrivenTool {
            id: Some("typed-event-driven-tool".to_string()),
            trigger: trigger.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::EventDrivenToolCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    assert_eq!(completed.thread_id, sess.conversation_id);
    assert_eq!(completed.turn_id, tc.sub_id);
    assert!(completed.completed_at_ms > 0);
    assert_eq!(completed.id, "typed-event-driven-tool");
    assert_eq!(completed.trigger, trigger);
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_command_wait_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::CommandWait {
            id: None,
            command_id: "cmd-1".to_string(),
            status: protocol::models::CommandWaitStatus::Completed,
            notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
            exit_code: Some(0),
            wall_time_seconds: 1.25,
            wait_timeout_ms: 250,
            created_at_ms: 1234,
        }],
    )
    .await;

    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::CommandWaitCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected response item completed event");

    assert_eq!(completed.thread_id, sess.conversation_id);
    assert_eq!(completed.turn_id, tc.sub_id);
    assert!(completed.lifecycle_at_ms > 0);
    assert_eq!(completed.id.starts_with("response-item-"), true);
    assert_eq!(completed.command_id, "cmd-1");
    assert_eq!(
        completed.status,
        protocol::models::CommandWaitStatus::Completed
    );
    assert_eq!(
        completed.notification,
        Some(protocol::models::CommandWaitNotificationKind::Exit)
    );
    assert_eq!(completed.exit_code, Some(0));
    assert_eq!(completed.wall_time_seconds, 1.25);
    assert_eq!(completed.wait_timeout_ms, 250);
    assert_eq!(completed.created_at_ms, 1234);
}

#[tokio::test]
async fn record_conversation_items_does_not_emit_item_completed_for_structured_response_item() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let trigger = EventDrivenToolTrigger {
        tool: "fs_subscribe".to_string(),
        title: "File watch triggered".to_string(),
        text: "build.log changed".to_string(),
    };

    sess.record_conversation_items(
        &tc,
        &[ResponseItem::EventDrivenTool {
            id: Some("typed-event-driven-tool".to_string()),
            trigger,
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await;

    assert!(
        completed.is_err(),
        "plain conversation recording should not emit a structured completed item"
    );
}

#[tokio::test]
async fn record_response_item_emits_item_completed_for_hook_prompt() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let hook_prompt_message = build_hook_prompt_message(&[HookPromptFragment::from_single_hook(
        "Retry with the requested change.",
        "hook-run-1",
    )])
    .expect("hook prompt message");

    sess.record_response_item_and_emit_turn_item(&tc, hook_prompt_message)
        .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    let TurnItem::HookPrompt(item) = completed.item else {
        panic!("expected HookPrompt item");
    };
    assert_eq!(
        item.fragments,
        vec![HookPromptFragment {
            text: "Retry with the requested change.".to_string(),
            hook_run_id: "hook-run-1".to_string(),
        }]
    );
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_event_command_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let event = EventCommandEvent {
        subscription_id: "sub-command".to_string(),
        kind: EventCommandEventKind::Output,
        label: Some("build log".to_string()),
        command: "tail -f /tmp/build.log".to_string(),
        cwd: Some("/repo".to_string()),
        line: Some("done".to_string()),
        sequence: Some(1),
        exit_code: None,
        signal: None,
        message: None,
        truncated: false,
        created_at: 1,
    };

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::EventCommandEvent {
            id: Some("typed-event-command".to_string()),
            event: event.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::EventCommandEventCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    assert_eq!(completed.id, "typed-event-command");
    assert_eq!(completed.event, event);
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_inter_agent_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::InterAgentCommunication {
            id: Some("typed-collab".to_string()),
            communication: communication.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::InterAgentCommunicationCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected response item completed event");

    assert_eq!(completed.id, "typed-collab");
    assert_eq!(completed.communication, communication);
}

#[tokio::test]
async fn explicit_record_conversation_items_ignores_unknown_collab_message() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "raw update".to_string(),
        InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::InterAgentCommunication {
            id: Some("typed-unknown-collab".to_string()),
            communication,
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await;

    assert!(
        completed.is_err(),
        "unknown collab communication should not emit a structured completed item"
    );
}

#[tokio::test]
async fn steer_input_returns_active_turn_id() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let steer_input = vec![UserInput::Text {
        text: "steer".to_string(),
        text_elements: Vec::new(),
    }];
    let turn_id = sess
        .steer_input(
            steer_input,
            Some(&tc.sub_id),
            /*responsesapi_client_metadata*/ None,
        )
        .await
        .expect("steering with matching expected turn id should succeed");

    assert_eq!(turn_id, tc.sub_id);
    assert!(sess.has_pending_input().await);
}

#[tokio::test]
async fn prepend_pending_input_keeps_older_tail_ahead_of_newer_input() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let blocked = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "blocked queued prompt".to_string(),
        }],
        phase: None,
    };
    let later = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "later queued prompt".to_string(),
        }],
        phase: None,
    };
    let newer = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "newer queued prompt".to_string(),
        }],
        phase: None,
    };

    sess.inject_hook_inspectable_items(vec![blocked.clone(), later.clone()])
        .await
        .expect("inject initial pending input into active turn");

    let drained = sess.get_pending_input().await;
    assert_eq!(
        drained,
        vec![
            PendingInputItem::from(blocked),
            PendingInputItem::from(later.clone()),
        ]
    );

    sess.inject_hook_inspectable_items(vec![newer.clone()])
        .await
        .expect("inject newer pending input into active turn");

    let mut drained_iter = drained.into_iter();
    let _blocked = drained_iter.next().expect("blocked prompt should exist");
    sess.prepend_pending_input(drained_iter.collect())
        .await
        .expect("requeue later pending input at the front of the queue");

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(later), PendingInputItem::from(newer)]
    );
}

#[tokio::test]
async fn queued_response_items_for_next_turn_move_into_next_active_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let queued_item = ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "queued before wake".to_string(),
        }],
        phase: None,
    };

    sess.queue_response_items_for_next_turn(vec![PendingInputItem::from(queued_item.clone())])
        .await;

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(queued_item)]
    );
}

#[tokio::test]
async fn idle_interrupt_does_not_wake_queued_next_turn_items() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let queued_item = ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "queued before interrupt".to_string(),
        }],
        phase: None,
    };

    sess.queue_response_items_for_next_turn(vec![PendingInputItem::from(queued_item)])
        .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    assert!(sess.active_turn.lock().await.is_none());
    assert!(sess.has_queued_response_items_for_next_turn().await);
}

#[tokio::test]
async fn abort_empty_active_turn_preserves_pending_input() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let pending_item = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    };
    let turn_state = {
        let mut active = sess.active_turn.lock().await;
        let active_turn = active.get_or_insert_with(ActiveTurn::default);
        Arc::clone(&active_turn.turn_state)
    };
    turn_state
        .lock()
        .await
        .push_pending_input(PendingInputItem::from(pending_item.clone()));

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert!(sess.active_turn.lock().await.is_none());
    assert_eq!(
        turn_state.lock().await.take_pending_input(),
        vec![PendingInputItem::from(pending_item)]
    );
}

#[tokio::test]
async fn interrupt_accounts_active_goal_before_pausing() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let goal = GoalService
        .get_thread_goal(sess.as_ref())
        .await
        .map_err(anyhow::Error::msg)?
        .expect("goal should remain persisted after interrupt");
    assert_eq!(protocol::protocol::ThreadGoalStatus::Paused, goal.status);
    assert_eq!(70, goal.tokens_used);

    assert!(sess.active_turn.lock().await.is_none());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_goal_continuation_runs_again_after_no_tool_turn() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a benchmark note"}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Draft ready."),
                ev_completed("resp-2"),
            ]),
            sse(vec![
                ev_assistant_message("msg-2", "I am still working on the benchmark note."),
                ev_completed("resp-3"),
            ]),
            sse(vec![
                ev_response_created("resp-4"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed("resp-4"),
            ]),
            sse(vec![
                ev_assistant_message("msg-3", "Goal complete."),
                ev_completed("resp-5"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a benchmark note".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    let mut completed_turns = 0;
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                completed_turns += 1;
                if completed_turns == 3 {
                    return anyhow::Ok(());
                }
            }
        }
    })
    .await??;

    let continuation_request = responses
        .requests()
        .into_iter()
        .find(|request| request.body_contains_text("<goal_context>"))
        .expect("expected a goal continuation request");
    let body = continuation_request.body_json();
    let goal_context_message = body["input"]
        .as_array()
        .expect("input should be an array")
        .iter()
        .find(|item| item.to_string().contains("<goal_context>"))
        .expect("goal context message should be present");
    assert_eq!(goal_context_message["role"].as_str(), Some("user"));
    assert!(
        goal_context_message
            .to_string()
            .contains("Continue working toward the active thread goal.")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_request_user_input_does_not_spawn_extra_goal_continuation() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
        config
            .features
            .enable(Feature::DefaultModeRequestUserInput)
            .expect("default-mode request_user_input should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a benchmark note"}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Draft ready."),
                ev_completed("resp-2"),
            ]),
            sse(vec![
                ev_response_created("resp-3"),
                ev_function_call(
                    "call-ask-user",
                    "request_user_input",
                    r#"{"questions":[{"header":"Choice","id":"next_step","question":"Pick one","options":[{"label":"Outline","description":"Start with an outline."},{"label":"Draft","description":"Write a full draft."}]}]}"#,
                ),
                ev_completed("resp-3"),
            ]),
            sse(vec![
                ev_response_created("resp-4"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed("resp-4"),
            ]),
            sse(vec![
                ev_assistant_message("msg-2", "Goal complete."),
                ev_completed("resp-5"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a benchmark note".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;

    let request_user_input_event = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::RequestUserInput(event) => Some(event.clone()),
        _ => None,
    })
    .await;
    assert_eq!(3, responses.requests().len());
    assert!(
        timeout(Duration::from_millis(200), test.codex.next_event())
            .await
            .is_err(),
        "waiting for request_user_input should keep the turn open without emitting more events"
    );
    assert_eq!(
        3,
        responses.requests().len(),
        "waiting for request_user_input should not start another continuation request"
    );

    test.codex
        .submit(Op::UserInputAnswer {
            id: request_user_input_event.turn_id,
            response: RequestUserInputResponse {
                answers: std::collections::HashMap::from([(
                    "next_step".to_string(),
                    RequestUserInputAnswer {
                        answers: vec!["Outline".to_string()],
                    },
                )]),
            },
        })
        .await?;

    let mut completed_turns = 0;
    timeout(Duration::from_secs(8), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                completed_turns += 1;
                if completed_turns == 1 {
                    return anyhow::Ok(());
                }
            }
        }
    })
    .await??;

    assert_eq!(5, responses.requests().len());

    Ok(())
}

async fn set_total_token_usage(sess: &Session, total_token_usage: TokenUsage) {
    let mut state = sess.state.lock().await;
    state.set_token_info(Some(TokenUsageInfo {
        total_token_usage,
        last_token_usage: TokenUsage::default(),
        model_context_window: None,
    }));
}

fn post_goal_token_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 50,
        cached_input_tokens: 10,
        output_tokens: 30,
        reasoning_output_tokens: 5,
        total_tokens: 75,
    }
}

async fn goal_test_state_db(sess: &Session) -> anyhow::Result<crate::StateDbHandle> {
    if let Some(state_db) = sess.state_db() {
        return Ok(state_db);
    }
    let config = sess.get_config().await;
    state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
        .await
        .map(|state_db| state_db as crate::StateDbHandle)
}

#[tokio::test]
async fn budget_limited_accounting_steers_active_turn_without_aborting() -> anyhow::Result<()> {
    let (sess, tc, rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            Some(10),
        )
        .await
        .map_err(anyhow::Error::msg)?;
    GoalService
        .begin_turn_goal_accounting(sess.as_ref(), tc.as_ref(), TokenUsage::default())
        .await
        .map_err(anyhow::Error::msg)?;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    while rx.try_recv().is_ok() {}

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 20,
            cached_input_tokens: 0,
            output_tokens: 5,
            reasoning_output_tokens: 0,
            total_tokens: 25,
        },
    )
    .await;

    GoalService
        .account_non_goal_tool_completed(sess.as_ref(), tc.as_ref(), "exec_command")
        .await
        .map_err(anyhow::Error::msg)?;

    let pending_input = sess.get_pending_input().await;
    let [PendingInputItem::HookInspectable(ResponseItem::Message { role, content, .. })] =
        pending_input.as_slice()
    else {
        panic!("expected one budget-limit steering message, got {pending_input:#?}");
    };
    assert_eq!("user", role);
    let [ContentItem::InputText { text }] = content.as_slice() else {
        panic!("expected one text span in budget-limit steering message, got {content:#?}");
    };
    assert!(text.starts_with("<goal_context>"));
    assert!(text.trim_end().ends_with("</goal_context>"));
    assert!(text.contains("budget_limited"));
    assert!(text.to_lowercase().contains("wrap up this turn soon"));
    assert!(sess.active_turn.lock().await.is_some());
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event.msg, EventMsg::TurnAborted(_)),
            "budget limit should steer the active turn instead of aborting it"
        );
    }

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted after accounting");
    assert_eq!(state_api::ThreadGoalStatus::BudgetLimited, goal.status);
    assert_eq!(25, goal.tokens_used);

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 30,
            cached_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
            total_tokens: 40,
        },
    )
    .await;
    GoalService
        .account_goal_mutation_completed(sess.as_ref(), tc.as_ref())
        .await
        .map_err(anyhow::Error::msg)?;

    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted after follow-up accounting");
    assert_eq!(state_api::ThreadGoalStatus::BudgetLimited, goal.status);
    assert_eq!(40, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_goal_mutation_accounts_active_turn_before_status_change() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    GoalService
        .prepare_external_goal_mutation(sess.as_ref())
        .await
        .map_err(anyhow::Error::msg)?;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(70, goal.tokens_used);

    let previous_goal = goal.clone();
    let goal_id = goal.goal_id.clone();
    let updated_goal = state_db
        .update_thread_goal(
            sess.conversation_id,
            state_api::ThreadGoalUpdate {
                objective: None,
                status: Some(state_api::ThreadGoalStatus::Complete),
                token_budget: None,
                expected_goal_id: Some(goal_id),
            },
        )
        .await?
        .expect("goal status update should succeed");
    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal: updated_goal,
                previous_status: ExternalGoalPreviousStatus::from(&previous_goal),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    assert!(sess.active_turn.lock().await.is_some());
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(state_api::ThreadGoalStatus::Complete, goal.status);
    assert_eq!(70, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_objective_change_steers_active_turn() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let old_goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Keep improving the benchmark",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ Some(10_000),
        )
        .await?;
    let new_goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Write a concise benchmark summary",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ Some(10_000),
        )
        .await?;

    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal: new_goal,
                previous_status: ExternalGoalPreviousStatus::from(&old_goal),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    let pending_input = sess.get_pending_input().await;
    assert!(
        pending_input.iter().any(|item| {
            matches!(
                item,
                PendingInputItem::HookInspectable(ResponseItem::Message { role, content, .. })
                    if role == "user"
                        && content.iter().any(|content| matches!(
                            content,
                            ContentItem::InputText { text }
                                if text.starts_with("<goal_context>")
                                    && text.trim_end().ends_with("</goal_context>")
                                    && text.contains("The active thread goal objective was edited")
                                    && text.contains("Write a concise benchmark summary")
                        ))
            )
        }),
        "expected objective-updated steering prompt in pending input: {pending_input:?}"
    );

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_active_goal_set_marks_current_turn_for_accounting() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Keep improving the benchmark",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ None,
        )
        .await?;
    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal,
                previous_status: ExternalGoalPreviousStatus::NewGoal,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 65,
            cached_input_tokens: 10,
            output_tokens: 40,
            reasoning_output_tokens: 5,
            total_tokens: 110,
        },
    )
    .await;
    GoalService
        .account_non_goal_tool_completed(sess.as_ref(), tc.as_ref(), "exec_command")
        .await
        .map_err(anyhow::Error::msg)?;

    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(state_api::ThreadGoalStatus::Active, goal.status);
    assert_eq!(25, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_goal_accounts_current_turn_tokens_before_tool_response() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a report","token_budget":500}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-2"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed_with_tokens("resp-2", /*total_tokens*/ 580),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Goal complete."),
                ev_completed("resp-3"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a report".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;

    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                return anyhow::Ok(());
            }
        }
    })
    .await??;

    let complete_output = responses
        .function_call_output_text("call-complete-goal")
        .expect("complete tool output should be sent to the model");
    let complete_output: serde_json::Value = serde_json::from_str(&complete_output)?;
    assert_eq!(complete_output["goal"]["tokensUsed"], 580);
    assert_eq!(complete_output["goal"]["status"], "complete");
    assert_eq!(complete_output["remainingTokens"], 0);
    assert_eq!(
        complete_output["completionBudgetReport"],
        "Goal achieved. Report final budget usage to the user: tokens used: 580 of 500."
    );
    let requests = responses.requests();
    let completion_followup_request = requests
        .last()
        .expect("completion tool output should be sent in a follow-up request");
    assert!(
        !completion_followup_request.body_contains_text("budget_limited"),
        "completion follow-up should not include budget-limit steering"
    );

    let state_db = state::StateRuntime::init(
        test.config.sqlite_home.clone(),
        test.config.model_provider_id.clone(),
    )
    .await?;
    let persisted_goal = state_db
        .get_thread_goal(test.session_configured.thread_id)
        .await?
        .expect("goal should be persisted");
    assert_eq!(state_api::ThreadGoalStatus::Complete, persisted_goal.status);
    assert_eq!(580, persisted_goal.tokens_used);

    Ok(())
}

#[tokio::test]
async fn queue_only_mailbox_mail_waits_for_next_turn_after_answer_boundary() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "late queue-only update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone()).await;

    assert!(
        !sess.has_pending_input().await,
        "queue-only mailbox mail should stay buffered once the current turn emitted its answer"
    );
    assert_eq!(sess.get_pending_input().await, Vec::new());

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn typed_queue_only_inter_agent_message_does_not_trigger_idle_turn() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queue-only typed update".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    sess.enqueue_async_input(PendingInputItem::from(
        ResponseItem::InterAgentCommunication {
            id: Some("typed-queue-only".to_string()),
            communication: communication.clone(),
        },
    ))
    .await;

    assert!(
        !sess.has_trigger_turn_mailbox_items().await,
        "queue-only typed inter-agent message should not request a new idle turn"
    );
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn pending_mailbox_input_can_be_peeked_without_consuming() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "already pending".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false);

    sess.enqueue_mailbox_communication(communication.clone()).await;

    let found = sess
        .find_pending_input(|item| match item {
            PendingInputItem::InterAgentCommunication(mail)
                if mail.author == communication.author =>
            {
                Some(mail.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(found, Some(communication.clone()));
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn inter_agent_unknown_communication_does_not_emit_live_collab_item() -> anyhow::Result<()> {
    let parent_thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: parent_thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await?;
    let _configured = rx_event.recv().await?;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "internal update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);

    crate::session::handlers::inter_agent_communication(
        &session,
        "unknown-mail".to_string(),
        communication,
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let result = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn inter_agent_child_completion_live_item_waits_for_typed_recording() -> anyhow::Result<()> {
    let (session, turn_context, rx_event) = make_session_and_context_with_rx().await;
    let parent_thread_id = session.thread_id();
    let child_thread_id = ThreadId::new();
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id)
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));

    crate::session::handlers::inter_agent_communication(
        &session,
        "child-completion-turn".to_string(),
        communication.clone(),
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let immediate_completed = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            match event.msg {
                EventMsg::InterAgentCommunicationCompleted(_) => return anyhow::Ok(()),
                EventMsg::ItemCompleted(completed)
                    if matches!(completed.item, protocol::items::TurnItem::CollabAgentMessage(_)) =>
                {
                    return anyhow::Ok(());
                }
                _ => {}
            }
        }
    })
    .await;
    assert!(
        immediate_completed.is_err(),
        "child completion should not emit a raw live collab item before typed pending input is recorded"
    );

    hooks::record_pending_input(
        session.as_ref(),
        turn_context.as_ref(),
        hooks::PendingInputRecord::InterAgentCommunication {
            pending_input: PendingInputItem::from(communication.clone()),
        },
    )
    .await;

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let event = rx_event.recv().await?;
            match event.msg {
                EventMsg::InterAgentCommunicationCompleted(completed) => {
                    return anyhow::Ok(completed);
                }
                EventMsg::ItemCompleted(completed)
                    if matches!(completed.item, protocol::items::TurnItem::CollabAgentMessage(_)) =>
                {
                    anyhow::bail!("child completion should use InterAgentCommunicationCompleted, not legacy ItemCompleted");
                }
                _ => {}
            }
        }
    })
    .await??;
    assert_eq!(completed.thread_id, parent_thread_id);
    assert_eq!(completed.turn_id, turn_context.sub_id.clone());
    assert!(completed.completed_at_ms > 0);
    assert_eq!(completed.communication, communication);

    let duplicate_completed = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            match event.msg {
                EventMsg::InterAgentCommunicationCompleted(_) => return anyhow::Ok(()),
                EventMsg::ItemCompleted(completed)
                    if matches!(completed.item, protocol::items::TurnItem::CollabAgentMessage(_)) =>
                {
                    return anyhow::Ok(());
                }
                _ => {}
            }
        }
    })
    .await;
    assert!(
        duplicate_completed.is_err(),
        "recording one typed child completion should emit exactly one live collab item"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_consumes_child_completion_like_other_pending_input() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    let child_thread_id = ThreadId::new();
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(true)
    .with_thread_ids(child_thread_id, sess.thread_id())
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));
    sess.enqueue_mailbox_communication(communication).await;

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    assert!(
        !sess.has_pending_mailbox_items().await,
        "turn start should drain child completion from the mailbox like any other pending input"
    );
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn clearing_stale_child_completion_preserves_non_completion_messages() {
    let (sess, _tc, _rx_event) = make_session_and_context_with_rx().await;
    let parent_thread_id = sess.thread_id();
    let child_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/worker").expect("worker path should parse");

    let stale_completion = InterAgentCommunication::new(
        child_agent_path.clone(),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id)
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));
    let progress_update = InterAgentCommunication::new(
        child_agent_path,
        AgentPath::root(),
        Vec::new(),
        "still working".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id);

    sess.enqueue_mailbox_communication(stale_completion).await;
    sess.enqueue_mailbox_communication(progress_update.clone()).await;

    let removed = sess
        .clear_child_completion_pending_input(child_thread_id)
        .await;

    assert_eq!(removed, 1);
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(progress_update)],
        "only the stale child completion should be removed"
    );

    assert_eq!(
        sess.get_pending_input().await,
        Vec::new(),
        "removing child completion input must not resurrect the old child completion"
    );
}

#[tokio::test]
async fn aborting_turn_drops_turn_scoped_child_completion_input() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    let parent_thread_id = sess.thread_id();
    let child_thread_id = ThreadId::new();
    let stale_completion = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id)
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;
    sess.prepend_pending_input(vec![PendingInputItem::from(stale_completion)])
        .await
        .expect("active turn should accept pending input");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert!(
        !sess.has_pending_input().await,
        "aborting the turn should drop turn-scoped child completion input"
    );
}

#[tokio::test]
async fn clearing_stale_child_completion_from_idle_queue_preserves_other_idle_input() {
    let (sess, _tc, _rx_event) = make_session_and_context_with_rx().await;
    let parent_thread_id = sess.thread_id();
    let child_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/worker").expect("worker path should parse");

    let stale_completion = InterAgentCommunication::new(
        child_agent_path.clone(),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id)
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));
    let progress_update = InterAgentCommunication::new(
        child_agent_path,
        AgentPath::root(),
        Vec::new(),
        "still queued".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id);

    sess.queue_response_items_for_next_turn(vec![
        PendingInputItem::from(stale_completion),
        PendingInputItem::from(progress_update.clone()),
    ])
    .await;

    let removed = sess
        .clear_child_completion_pending_input(child_thread_id)
        .await;

    assert_eq!(removed, 1);
    assert_eq!(
        sess.take_queued_response_items_for_next_turn().await,
        vec![PendingInputItem::from(progress_update)],
        "idle non-completion input should be preserved"
    );
}

#[tokio::test]
async fn poll_event_wakes_for_user_input() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let waiter = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.poll_event(thread_service_api::ThreadPollEventRequest {
                initial_timeout_ms: Some(100),
                hard_cap_timeout_ms: Some(400),
            })
            .await
            .expect("poll_event should succeed")
        })
    };
    tokio::task::yield_now().await;

    sess.steer_input(
        vec![UserInput::Text {
            text: "wake".to_string(),
            text_elements: Vec::new(),
        }],
        None,
        None,
    )
    .await
    .expect("user input should steer active turn");

    let result = timeout(Duration::from_secs(2), waiter)
        .await
        .expect("poll_event should finish")
        .expect("poll_event task");
    assert!(!result.timed_out);
    assert_eq!(result.source_hint.as_deref(), Some("user_input"));

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn poll_event_returns_immediately_for_existing_pending_command_output() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.enqueue_async_input(PendingInputItem::from(
        ResponseItem::CommandExecutionNotification {
            id: Some("cmd-output-1".to_string()),
            command_item_id: "cmd-1".to_string(),
            kind: protocol::models::CommandExecutionNotificationKind::Output,
            message: "Command output notification received.".to_string(),
            output: Some("wake".to_string()),
            exit_code: None,
            created_at_ms: 1234,
        },
    ))
    .await;

    let result = sess
        .poll_event(thread_service_api::ThreadPollEventRequest {
            initial_timeout_ms: Some(100),
            hard_cap_timeout_ms: Some(400),
        })
        .await
        .expect("poll_event should succeed");
    assert!(!result.timed_out);
    assert_eq!(result.waited_ms, 0);
    assert_eq!(result.source_hint.as_deref(), Some("command_output"));
    match result.event {
        Some(thread_service_api::ThreadPollEvent::CommandExecutionNotification {
            command_item_id,
            kind,
            message,
            output,
            exit_code,
            created_at_ms,
        }) => {
            assert_eq!(command_item_id, "cmd-1");
            assert_eq!(
                kind,
                protocol::models::CommandExecutionNotificationKind::Output
            );
            assert_eq!(message, "Command output notification received.");
            assert_eq!(output.as_deref(), Some("wake"));
            assert_eq!(exit_code, None);
            assert_eq!(created_at_ms, 1234);
        }
        other => panic!("expected command output payload, got {other:?}"),
    }
    assert_eq!(result.events.len(), 1);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn poll_event_wakes_for_child_completion() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let waiter = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.poll_event(thread_service_api::ThreadPollEventRequest {
                initial_timeout_ms: Some(100),
                hard_cap_timeout_ms: Some(400),
            })
            .await
            .expect("poll_event should succeed")
        })
    };
    tokio::task::yield_now().await;

    sess.enqueue_mailbox_communication(
        InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("worker path should parse"),
            AgentPath::root(),
            Vec::new(),
            "wake".to_string(),
            protocol::protocol::InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(false)
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "worker final output".to_string(),
        ))),
    )
    .await;

    let result = timeout(Duration::from_secs(2), waiter)
        .await
        .expect("poll_event should finish")
        .expect("poll_event task");
    assert!(!result.timed_out);
    assert_eq!(result.source_hint.as_deref(), Some("child_completion"));
    match result.event {
        Some(thread_service_api::ThreadPollEvent::InterAgentCommunication { communication }) => {
            assert_eq!(communication.content, "wake");
            assert_eq!(
                communication.status,
                Some(protocol::protocol::AgentStatus::Completed(Some(
                    "worker final output".to_string()
                )))
            );
        }
        other => panic!("expected child completion payload, got {other:?}"),
    }
    assert_eq!(result.events.len(), 1);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn poll_event_lists_later_child_completion_while_older_completion_is_pending() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.enqueue_mailbox_communication(
        InterAgentCommunication::new(
            AgentPath::try_from("/root/explorer").expect("explorer path should parse"),
            AgentPath::root(),
            Vec::new(),
            "explorer done".to_string(),
            protocol::protocol::InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(false)
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "explorer done".to_string(),
        ))),
    )
    .await;

    let first = sess
        .poll_event(thread_service_api::ThreadPollEventRequest {
            initial_timeout_ms: Some(100),
            hard_cap_timeout_ms: Some(400),
        })
        .await
        .expect("first poll_event should succeed");
    assert_eq!(first.events.len(), 1);

    sess.enqueue_mailbox_communication(
        InterAgentCommunication::new(
            AgentPath::try_from("/root/owner").expect("owner path should parse"),
            AgentPath::root(),
            Vec::new(),
            "owner done".to_string(),
            protocol::protocol::InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(false)
        .with_status(protocol::protocol::AgentStatus::Completed(Some(
            "owner done".to_string(),
        ))),
    )
    .await;

    let second = sess
        .poll_event(thread_service_api::ThreadPollEventRequest {
            initial_timeout_ms: Some(100),
            hard_cap_timeout_ms: Some(400),
        })
        .await
        .expect("second poll_event should succeed");
    let authors = second
        .events
        .iter()
        .map(|event| match event {
            thread_service_api::ThreadPollEvent::InterAgentCommunication { communication } => {
                communication.author.as_str()
            }
            other => panic!("expected inter-agent payload, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(authors, vec!["/root/explorer", "/root/owner"]);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn poll_event_wakes_for_command_exit_notification() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let waiter = {
        let sess = Arc::clone(&sess);
        tokio::spawn(async move {
            sess.poll_event(thread_service_api::ThreadPollEventRequest {
                initial_timeout_ms: Some(100),
                hard_cap_timeout_ms: Some(400),
            })
            .await
            .expect("poll_event should succeed")
        })
    };
    tokio::task::yield_now().await;

    sess.enqueue_async_input(PendingInputItem::from(
        ResponseItem::CommandExecutionNotification {
            id: Some("cmd-exit-1".to_string()),
            command_item_id: "cmd-1".to_string(),
            kind: protocol::models::CommandExecutionNotificationKind::Exit,
            message: "Command exit notification received.".to_string(),
            output: None,
            exit_code: Some(0),
            created_at_ms: 1234,
        },
    ))
    .await;

    let result = timeout(Duration::from_secs(2), waiter)
        .await
        .expect("poll_event should finish")
        .expect("poll_event task");
    assert!(!result.timed_out);
    assert_eq!(result.source_hint.as_deref(), Some("command_exit"));
    match result.event {
        Some(thread_service_api::ThreadPollEvent::CommandExecutionNotification {
            command_item_id,
            kind,
            message,
            output,
            exit_code,
            created_at_ms,
        }) => {
            assert_eq!(command_item_id, "cmd-1");
            assert_eq!(
                kind,
                protocol::models::CommandExecutionNotificationKind::Exit
            );
            assert_eq!(message, "Command exit notification received.");
            assert_eq!(output, None);
            assert_eq!(exit_code, Some(0));
            assert_eq!(created_at_ms, 1234);
        }
        other => panic!("expected command exit payload, got {other:?}"),
    }
    assert_eq!(result.events.len(), 1);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn deferred_command_exit_display_waits_for_request_construction_consumption() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let command = vec!["sh".to_string(), "-c".to_string(), "echo done".to_string()];
    let notification = ResponseItem::CommandExecutionNotification {
        id: Some("cmd-1:notification:exit".to_string()),
        command_item_id: "cmd-1".to_string(),
        kind: protocol::models::CommandExecutionNotificationKind::Exit,
        message: "Command exit notification received.".to_string(),
        output: Some("done\n".to_string()),
        exit_code: Some(0),
        created_at_ms: 1234,
    };
    let exec_end = EventMsg::ExecCommandEnd(protocol::protocol::ExecCommandEndEvent {
        call_id: "cmd-1".to_string(),
        process_id: Some("1".to_string()),
        turn_id: tc.sub_id.clone(),
        completed_at_ms: 1235,
        command: command.clone(),
        #[allow(deprecated)]
        cwd: tc.cwd.clone(),
        parsed_cmd: codex_shell_utils::parse_command::parse_command(&command),
        source: protocol::protocol::ExecCommandSource::UnifiedExecStartup,
        interaction_input: None,
        initial_wait_ms: Some(1000),
        notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
        stdout: "done\n".to_string(),
        stderr: String::new(),
        aggregated_output: "done\n".to_string(),
        exit_code: 0,
        duration: Duration::from_millis(10),
        formatted_output: "done\n".to_string(),
        status: protocol::protocol::ExecCommandStatus::Completed,
    });

    thread_service_api::ThreadSessionCapability::append_conversation_item_with_observed_event(
        sess.as_ref(),
        notification,
        exec_end,
    )
    .await
    .expect("append pending command notification");

    let poll_result = sess
        .poll_event(thread_service_api::ThreadPollEventRequest {
            initial_timeout_ms: Some(100),
            hard_cap_timeout_ms: Some(400),
        })
        .await
        .expect("poll_event should wake");
    assert!(!poll_result.timed_out);
    assert_eq!(poll_result.source_hint.as_deref(), Some("command_exit"));
    assert!(matches!(
        poll_result.event,
        Some(thread_service_api::ThreadPollEvent::CommandExecutionNotification {
            kind: protocol::models::CommandExecutionNotificationKind::Exit,
            ..
        })
    ));

    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event.msg, EventMsg::ExecCommandEnd(_)),
            "ExecCommandEnd must not be displayed before request construction consumes the notification"
        );
    }

    let pending_input = sess.get_pending_input().await;
    assert_eq!(pending_input.len(), 1);
    for pending_input_item in pending_input {
        match hooks::inspect_pending_input(sess.as_ref(), tc.as_ref(), pending_input_item).await {
            hooks::PendingInputHookDisposition::Accepted(pending_input) => {
                hooks::record_pending_input(sess.as_ref(), tc.as_ref(), *pending_input).await;
            }
            hooks::PendingInputHookDisposition::Blocked { .. } => {
                panic!("command notification should not be blocked")
            }
        }
    }

    timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event channel open");
            assert!(
                !matches!(
                    event.msg,
                    EventMsg::CommandExecutionNotificationCompleted(_)
                ),
                "command notification display must not precede the deferred ExecCommandEnd"
            );
            if matches!(event.msg, EventMsg::ExecCommandEnd(_)) {
                break;
            }
        }
    })
    .await
    .expect("deferred exec end should be emitted before the notification display");
    timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event channel open");
            if matches!(
                event.msg,
                EventMsg::CommandExecutionNotificationCompleted(_)
            ) {
                break;
            }
        }
    })
    .await
    .expect("notification display should be emitted");

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leftover_command_exit_display_is_followed_by_provider_request_with_notification() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let responses = mount_sse_sequence(
        &server,
        vec![sse(vec![
            ev_assistant_message("msg-followup", "Saw the command exit."),
            ev_completed("resp-followup"),
        ])],
    )
    .await;
    let (sess, rx) = make_session_with_config_and_rx(|config| {
        config.model_provider.base_url = Some(format!("{}/v1", server.uri()));
        config.model_provider.supports_websockets = false;
        config.model_providers.insert(
            config.model_provider_id.clone(),
            config.model_provider.clone(),
        );
    })
    .await?;
    let tc = sess.new_default_turn().await;

    let command = vec!["sh".to_string(), "-c".to_string(), "echo done".to_string()];
    let notification = ResponseItem::CommandExecutionNotification {
        id: Some("cmd-1:notification:exit".to_string()),
        command_item_id: "cmd-1".to_string(),
        kind: protocol::models::CommandExecutionNotificationKind::Exit,
        message: "Command exit notification received.".to_string(),
        output: Some("done\n".to_string()),
        exit_code: Some(0),
        created_at_ms: 1234,
    };
    let exec_end = EventMsg::ExecCommandEnd(protocol::protocol::ExecCommandEndEvent {
        call_id: "cmd-1".to_string(),
        process_id: Some("1".to_string()),
        turn_id: tc.sub_id.clone(),
        completed_at_ms: 1235,
        command: command.clone(),
        #[allow(deprecated)]
        cwd: tc.cwd.clone(),
        parsed_cmd: codex_shell_utils::parse_command::parse_command(&command),
        source: protocol::protocol::ExecCommandSource::UnifiedExecStartup,
        interaction_input: None,
        initial_wait_ms: Some(0),
        notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
        stdout: "done\n".to_string(),
        stderr: String::new(),
        aggregated_output: "done\n".to_string(),
        exit_code: 0,
        duration: Duration::from_millis(10),
        formatted_output: "done\n".to_string(),
        status: protocol::protocol::ExecCommandStatus::Completed,
    });

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        CommandExitNotificationOnFinishTask {
            notification,
            observed_event: exec_end,
        },
    )
    .await;

    timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event channel open");
            if matches!(
                event.msg,
                EventMsg::CommandExecutionNotificationCompleted(_)
            ) {
                break;
            }
        }
    })
    .await
    .expect("command notification display should be emitted before follow-up assertion");

    let followup_request = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut observed_events = Vec::new();
        loop {
            if let Some(request) = responses.last_request() {
                break request;
            }
            while let Ok(event) = rx.try_recv() {
                observed_events.push(format!("{:?}", event.msg));
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "follow-up provider request did not start; observed events after display: {observed_events:#?}"
            );
            tokio::task::yield_now().await;
        }
    };
    let body = followup_request.body_json();
    let notification_item = body["input"]
        .as_array()
        .expect("provider request input should be an array")
        .iter()
        .find(|item| {
            item.get("command_item_id")
                .and_then(serde_json::Value::as_str)
                == Some("cmd-1")
        })
        .unwrap_or_else(|| {
            panic!("provider request input should include command notification: {body}")
        });
    assert_eq!(
        notification_item
            .get("kind")
            .and_then(serde_json::Value::as_str),
        Some("exit")
    );
    assert_eq!(
        notification_item
            .get("message")
            .and_then(serde_json::Value::as_str),
        Some("Command exit notification received.")
    );
    assert_eq!(
        notification_item
            .get("exit_code")
            .and_then(serde_json::Value::as_i64),
        Some(0)
    );

    Ok(())
}

#[tokio::test]
async fn pending_event_driven_tool_display_waits_for_request_construction_consumption() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let trigger = EventDrivenToolTrigger {
        tool: "fs_subscribe".to_string(),
        title: "File watch triggered".to_string(),
        text: "build.log changed".to_string(),
    };
    sess.enqueue_async_input(PendingInputItem::from(ResponseItem::EventDrivenTool {
        id: Some("subscription-event-1".to_string()),
        trigger: trigger.clone(),
    }))
    .await;

    let poll_result = sess
        .poll_event(thread_service_api::ThreadPollEventRequest {
            initial_timeout_ms: Some(100),
            hard_cap_timeout_ms: Some(400),
        })
        .await
        .expect("poll_event should wake");
    assert!(!poll_result.timed_out);
    assert_eq!(poll_result.source_hint.as_deref(), Some("async_input"));

    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event.msg, EventMsg::EventDrivenToolCompleted(_)),
            "event-driven tool display must not be emitted before request construction consumes it"
        );
    }

    let pending_input = sess.get_pending_input().await;
    assert_eq!(pending_input.len(), 1);
    for pending_input_item in pending_input {
        match hooks::inspect_pending_input(sess.as_ref(), tc.as_ref(), pending_input_item).await {
            hooks::PendingInputHookDisposition::Accepted(pending_input) => {
                hooks::record_pending_input(sess.as_ref(), tc.as_ref(), *pending_input).await;
            }
            hooks::PendingInputHookDisposition::Blocked { .. } => {
                panic!("event-driven tool input should not be blocked")
            }
        }
    }

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event channel open");
            if let EventMsg::EventDrivenToolCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("event-driven tool display should be emitted after consumption");
    assert_eq!(completed.id, "subscription-event-1");
    assert_eq!(completed.trigger, trigger);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn poll_event_backoff_is_thread_scoped_and_resets_after_event() {
    let (sess, _tc, _rx_event) = make_session_and_context_with_rx().await;
    let request = thread_service_api::ThreadPollEventRequest {
        initial_timeout_ms: Some(20),
        hard_cap_timeout_ms: Some(80),
    };

    let first = sess
        .poll_event(request.clone())
        .await
        .expect("first poll_event should succeed");
    assert!(first.timed_out);
    assert_eq!(first.event, None);
    assert_eq!(first.current_timeout_ms, 20);

    let second = sess
        .poll_event(request.clone())
        .await
        .expect("second poll_event should succeed");
    assert!(second.timed_out);
    assert_eq!(second.current_timeout_ms, 40);

    sess.enqueue_async_input(PendingInputItem::from(ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "wake".to_string(),
        }],
        phase: None,
    }))
    .await;
    let wake = sess
        .poll_event(request.clone())
        .await
        .expect("wake poll_event should succeed");
    assert!(!wake.timed_out);
    assert_eq!(wake.source_hint.as_deref(), Some("async_input"));
    let _ = sess.get_pending_input().await;

    let after_reset = sess
        .poll_event(request)
        .await
        .expect("reset poll_event should succeed");
    assert!(after_reset.timed_out);
    assert_eq!(after_reset.current_timeout_ms, 20);
}

#[tokio::test]
async fn inter_agent_send_message_queue_only_does_not_emit_live_collab_item() -> anyhow::Result<()>
{
    let parent_thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: parent_thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await?;
    let _configured = rx_event.recv().await?;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued message".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    crate::session::handlers::inter_agent_communication(
        &session,
        "queued-mail".to_string(),
        communication,
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let result = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn trigger_turn_mailbox_mail_waits_for_next_turn_after_answer_boundary() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "late trigger update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(true))
    .await;

    assert!(
        !sess.has_pending_input().await,
        "trigger-turn mailbox mail should not extend the current turn after its answer boundary"
    );

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert!(sess.has_trigger_turn_mailbox_items().await);
}

#[tokio::test]
async fn steered_input_reopens_async_input_for_current_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone()).await;
    sess.steer_input(
        vec![UserInput::Text {
            text: "follow up".to_string(),
            text_elements: Vec::new(),
        }],
        Some(&tc.sub_id),
        /*responsesapi_client_metadata*/ None,
    )
    .await
    .expect("steered input should be accepted");

    assert_eq!(
        sess.get_pending_input().await,
        vec![
            PendingInputItem::from(ResponseInputItem::from(vec![UserInput::Text {
                text: "follow up".to_string(),
                text_elements: Vec::new(),
            }])),
            PendingInputItem::from(communication),
        ],
    );
}

#[tokio::test]
async fn stale_defer_async_input_does_not_override_steered_input() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone()).await;
    sess.steer_input(
        vec![UserInput::Text {
            text: "follow up".to_string(),
            text_elements: Vec::new(),
        }],
        Some(&tc.sub_id),
        /*responsesapi_client_metadata*/ None,
    )
    .await
    .expect("steered input should be accepted");

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![
            PendingInputItem::from(ResponseInputItem::from(vec![UserInput::Text {
                text: "follow up".to_string(),
                text_elements: Vec::new(),
            }])),
            PendingInputItem::from(communication),
        ],
    );
}

#[tokio::test]
async fn tool_calls_reopen_async_input_for_current_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_async_input_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone()).await;

    let item = ResponseItem::FunctionCall {
        id: None,
        name: "test_tool".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id: "call-1".to_string(),
    };
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&sess),
        turn_context: Arc::clone(&tc),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(tc.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&sess), Arc::clone(&tc)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("tool call should be handled");

    assert!(output.needs_follow_up);
    assert!(output.tool_future.is_some());
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_review_task_emits_exited_then_aborted_and_records_history() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "start review".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(Arc::clone(&tc), input, ReviewTask::new())
        .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    // Aborting a review task should exit review mode before surfacing the abort to the client.
    // We scan for these events (rather than relying on fixed ordering) since unrelated events
    // may interleave.
    let mut exited_review_mode_idx = None;
    let mut turn_aborted_idx = None;
    let mut idx = 0usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        let event_idx = idx;
        idx = idx.saturating_add(1);
        match evt.msg {
            EventMsg::ExitedReviewMode(ev) => {
                assert!(ev.review_output.is_none());
                exited_review_mode_idx = Some(event_idx);
            }
            EventMsg::TurnAborted(ev) => {
                assert_eq!(TurnAbortReason::Interrupted, ev.reason);
                turn_aborted_idx = Some(event_idx);
                break;
            }
            _ => {}
        }
    }
    assert!(
        exited_review_mode_idx.is_some(),
        "expected ExitedReviewMode after abort"
    );
    assert!(
        turn_aborted_idx.is_some(),
        "expected TurnAborted after abort"
    );
    assert!(
        exited_review_mode_idx.unwrap() < turn_aborted_idx.unwrap(),
        "expected ExitedReviewMode before TurnAborted"
    );

    let history = sess.clone_history().await;
    // The `<turn_aborted>` marker is silent in the event stream, so verify it is still
    // recorded in history for the model.
    assert!(
        history.raw_items().iter().any(|item| {
            let ResponseItem::Message { role, content, .. } = item else {
                return false;
            };
            if role != "user" {
                return false;
            }
            content.iter().any(|content_item| {
                let ContentItem::InputText { text } = content_item else {
                    return false;
                };
                TurnAborted::matches_text(text)
            })
        }),
        "expected a model-visible turn aborted marker in history after interrupt"
    );
}

async fn sample_rollout(
    session: &Session,
    _turn_context: &TurnContext,
) -> (Vec<RolloutItem>, Vec<ResponseItem>) {
    let mut rollout_items = Vec::new();
    let mut live_history = ContextManager::new();

    // Use the same turn_context source as record_initial_history so model_info (and thus
    // personality_spec) matches reconstruction.
    let reconstruction_turn = session.new_default_turn().await;
    let mut initial_context = session
        .build_initial_context(reconstruction_turn.as_ref())
        .await;
    // Ensure personality_spec is present when Personality is enabled, so expected matches
    // what reconstruction produces (build_initial_context may omit it when baked into model).
    if !initial_context.iter().any(|m| {
        matches!(m, ResponseItem::Message { role, content, .. }
        if role == "developer"
            && content.iter().any(|c| {
                matches!(c, ContentItem::InputText { text } if text.contains("<personality_spec>"))
            }))
    }) && let Some(p) = reconstruction_turn.personality
        && session.features.enabled(Feature::Personality)
        && let Some(personality_message) = reconstruction_turn
            .model_info
            .model_messages
            .as_ref()
            .and_then(|m| m.get_personality_message(Some(p)).filter(|s| !s.is_empty()))
    {
        let msg = ContextualUserFragment::into(
            codex_context_manager::PersonalitySpecInstructions::new(personality_message),
        );
        let insert_at = initial_context
            .iter()
            .position(|m| matches!(m, ResponseItem::Message { role, .. } if role == "developer"))
            .map(|i| i + 1)
            .unwrap_or(0);
        initial_context.insert(insert_at, msg);
    }
    for item in &initial_context {
        rollout_items.push(RolloutItem::ResponseItem(item.clone()));
    }
    live_history.record_items(
        initial_context.iter(),
        reconstruction_turn.truncation_policy,
    );

    let user1 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "first user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user1),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user1.clone()));

    let assistant1 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply one".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant1),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant1.clone()));

    let summary1 = "summary one";
    let snapshot1 = live_history
        .clone()
        .for_prompt(&reconstruction_turn.model_info.input_modalities);
    let user_messages1 = collect_user_messages(&snapshot1);
    let rebuilt1 = compact::build_compacted_history(Vec::new(), &user_messages1, summary1);
    live_history.replace(rebuilt1);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary1.to_string(),
        replacement_history: None,
    }));

    let user2 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "second user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user2),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user2.clone()));

    let assistant2 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply two".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant2),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant2.clone()));

    let summary2 = "summary two";
    let snapshot2 = live_history
        .clone()
        .for_prompt(&reconstruction_turn.model_info.input_modalities);
    let user_messages2 = collect_user_messages(&snapshot2);
    let rebuilt2 = compact::build_compacted_history(Vec::new(), &user_messages2, summary2);
    live_history.replace(rebuilt2);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary2.to_string(),
        replacement_history: None,
    }));

    let user3 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "third user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user3),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user3));

    let assistant3 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply three".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant3),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant3));

    (
        rollout_items,
        live_history.for_prompt(&reconstruction_turn.model_info.input_modalities),
    )
}

#[tokio::test]
async fn rejects_escalated_permissions_when_policy_not_on_request() {
    use permissions_service::ExecPolicyApprovalRequest as ExecApprovalRequest;
    use protocol::models::SandboxPermissions;
    use protocol::protocol::AskForApproval;

    let (session, mut turn_context_raw) = make_session_and_context().await;
    // Ensure policy is NOT OnRequest so the early rejection path triggers
    turn_context_raw
        .approval_policy
        .set(AskForApproval::OnFailure)
        .expect("test setup should allow updating approval policy");
    let session = Arc::new(session);
    let mut turn_context = Arc::new(turn_context_raw);

    let command_script = "echo hi";
    let sandbox_permissions = SandboxPermissions::RequireEscalated;

    let call_id = "test-call".to_string();
    #[allow(deprecated)]
    let workdir = Some(turn_context.cwd.to_string_lossy().to_string());
    let resp = dispatch_exec_command_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        &call_id,
        serde_json::json!({
            "cmd": command_script,
            "workdir": workdir,
            "sandbox_permissions": sandbox_permissions,
            "justification": Some("test"),
        }),
    )
    .await;

    let Err(FunctionCallError::RespondToModel(output)) = resp else {
        panic!("expected error result");
    };

    let expected = format!(
        "approval policy is {policy:?}; reject command — you cannot ask for escalated permissions if the approval policy is {policy:?}",
        policy = turn_context.approval_policy.value()
    );

    pretty_assertions::assert_eq!(output, expected);
    pretty_assertions::assert_eq!(session.granted_turn_permissions().await, None);

    // The rejection should not poison the non-escalated path for the same
    // command. Force DangerFullAccess so this check stays focused on approval
    // policy rather than platform-specific sandbox behavior.
    let turn_context_mut = Arc::get_mut(&mut turn_context).expect("unique turn context Arc");
    turn_context_mut.permission_profile = PermissionProfile::Disabled;

    let file_system_sandbox_policy = turn_context.file_system_sandbox_policy();
    let command = session
        .user_shell()
        .derive_exec_args(command_script, turn_context.tools_config.allow_login_shell);
    let exec_approval_requirement = session
        .services
        .exec_policy
        .create_exec_approval_requirement_for_command(ExecApprovalRequest {
            command: &command,
            approval_policy: turn_context.approval_policy.value(),
            permission_profile: turn_context.permission_profile(),
            file_system_sandbox_policy: &file_system_sandbox_policy,
            #[allow(deprecated)]
            sandbox_cwd: turn_context.cwd.as_path(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            prefix_rule: None,
        })
        .await;
    assert!(matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip { .. }
    ));
}
#[tokio::test]
async fn session_start_hooks_only_load_from_trusted_project_layers() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let codex_home = temp.path().join("home");
    let project_root = temp.path().join("project");
    let nested = project_root.join("nested");
    let root_dot_codex = project_root.join(".codex");
    let nested_dot_codex = nested.join(".codex");

    std::fs::create_dir_all(&codex_home)?;
    std::fs::create_dir_all(&nested_dot_codex)?;
    std::fs::write(project_root.join(".git"), "gitdir: here")?;
    write_project_hooks(&root_dot_codex)?;
    write_project_hooks(&nested_dot_codex)?;
    write_project_trust_config(&codex_home, &[(&nested, TrustLevel::Trusted)]).await?;

    let config = ConfigBuilder::default()
        .codex_home(codex_home)
        .fallback_cwd(Some(nested))
        .build()
        .await?;

    let hook_list = hooks::list_hooks(hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config.config_layer_stack,
            ),
        ),
        ..hooks::HooksConfig::default()
    });
    let expected_source_path = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
        nested_dot_codex.join("hooks.json"),
    )?;
    assert_eq!(
        hook_list
            .hooks
            .iter()
            .map(|hook| &hook.source_path)
            .collect::<Vec<_>>(),
        vec![&expected_source_path],
    );
    assert_eq!(
        hook_list.hooks[0].trust_status,
        protocol::protocol::HookTrustStatus::Untrusted
    );
    assert!(preview_session_start_hooks(&config).await?.is_empty());

    Ok(())
}

#[tokio::test]
async fn session_start_hooks_require_project_trust_without_config_toml() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let project_root = temp.path().join("project");
    let nested = project_root.join("nested");
    let dot_codex = project_root.join(".codex");
    std::fs::create_dir_all(&nested)?;
    std::fs::write(project_root.join(".git"), "gitdir: here")?;
    write_project_hooks(&dot_codex)?;

    let cases = [
        ("unknown", Vec::<(&Path, TrustLevel)>::new(), 0_usize),
        (
            "untrusted",
            vec![(&project_root as &Path, TrustLevel::Untrusted)],
            0_usize,
        ),
        (
            "trusted",
            vec![(&project_root as &Path, TrustLevel::Trusted)],
            1_usize,
        ),
    ];

    for (name, trust_entries, expected_hooks) in cases {
        let codex_home = temp.path().join(format!("home_{name}"));
        std::fs::create_dir_all(&codex_home)?;
        write_project_trust_config(&codex_home, &trust_entries).await?;

        let config = ConfigBuilder::default()
            .codex_home(codex_home)
            .fallback_cwd(Some(nested.clone()))
            .build()
            .await?;

        let hook_list = hooks::list_hooks(hooks::HooksConfig {
            feature_enabled: true,
            config_layer_stack: Some(
                crate::config::hook_config_layer_stack_from_config_layer_stack(
                    &config.config_layer_stack,
                ),
            ),
            ..hooks::HooksConfig::default()
        });
        assert_eq!(
            hook_list.hooks.len(),
            expected_hooks,
            "unexpected discovered hook count for {name}",
        );
        assert!(preview_session_start_hooks(&config).await?.is_empty());
        if expected_hooks == 1 {
            assert_eq!(
                hook_list.hooks[0].trust_status,
                protocol::protocol::HookTrustStatus::Untrusted
            );
        }
    }

    Ok(())
}
