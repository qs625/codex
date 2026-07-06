use super::*;

#[test]
fn normalize_path_for_skill_id_repo_scoped_uses_relative_path() {
    let repo_root = PathBuf::from("/repo/root");
    let skill_path = PathBuf::from("/repo/root/.codex/skills/doc/SKILL.md");

    let path = normalize_path_for_skill_id(
        Some("https://example.com/repo.git"),
        Some(repo_root.as_path()),
        skill_path.as_path(),
    );

    assert_eq!(path, ".codex/skills/doc/SKILL.md");
}

#[test]
fn normalize_path_for_skill_id_user_scoped_uses_absolute_path() {
    let skill_path = PathBuf::from("/Users/abc/.codex/skills/doc/SKILL.md");

    let path = normalize_path_for_skill_id(
        /*repo_url*/ None,
        /*repo_root*/ None,
        skill_path.as_path(),
    );
    let expected = expected_absolute_path(&skill_path);

    assert_eq!(path, expected);
}

#[test]
fn normalize_path_for_skill_id_admin_scoped_uses_absolute_path() {
    let skill_path = PathBuf::from("/etc/codex/skills/doc/SKILL.md");

    let path = normalize_path_for_skill_id(
        /*repo_url*/ None,
        /*repo_root*/ None,
        skill_path.as_path(),
    );
    let expected = expected_absolute_path(&skill_path);

    assert_eq!(path, expected);
}

#[test]
fn normalize_path_for_skill_id_repo_root_not_in_skill_path_uses_absolute_path() {
    let repo_root = PathBuf::from("/repo/root");
    let skill_path = PathBuf::from("/other/path/.codex/skills/doc/SKILL.md");

    let path = normalize_path_for_skill_id(
        Some("https://example.com/repo.git"),
        Some(repo_root.as_path()),
        skill_path.as_path(),
    );
    let expected = expected_absolute_path(&skill_path);

    assert_eq!(path, expected);
}

#[test]
fn app_mentioned_event_serializes_expected_shape() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let event = TrackEventRequest::AppMentioned(CodexAppMentionedEventRequest {
        event_type: "codex_app_mentioned",
        event_params: codex_app_metadata(
            &tracking,
            AppInvocation {
                connector_id: Some("calendar".to_string()),
                app_name: Some("Calendar".to_string()),
                invocation_type: Some(InvocationType::Explicit),
            },
        ),
    });

    let payload = serde_json::to_value(&event).expect("serialize app mentioned event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_app_mentioned",
            "event_params": {
                "connector_id": "calendar",
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "app_name": "Calendar",
                "product_client_id": originator().value,
                "invoke_type": "explicit",
                "model_slug": "gpt-5"
            }
        })
    );
}

#[test]
fn app_used_event_serializes_expected_shape() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-2".to_string(),
        turn_id: "turn-2".to_string(),
    };
    let event = TrackEventRequest::AppUsed(CodexAppUsedEventRequest {
        event_type: "codex_app_used",
        event_params: codex_app_metadata(
            &tracking,
            AppInvocation {
                connector_id: Some("drive".to_string()),
                app_name: Some("Google Drive".to_string()),
                invocation_type: Some(InvocationType::Implicit),
            },
        ),
    });

    let payload = serde_json::to_value(&event).expect("serialize app used event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_app_used",
            "event_params": {
                "connector_id": "drive",
                "thread_id": "thread-2",
                "turn_id": "turn-2",
                "app_name": "Google Drive",
                "product_client_id": originator().value,
                "invoke_type": "implicit",
                "model_slug": "gpt-5"
            }
        })
    );
}

#[test]
fn accepted_line_fingerprints_event_serializes_expected_shape() {
    let event = TrackEventRequest::AcceptedLineFingerprints(Box::new(
        CodexAcceptedLineFingerprintsEventRequest {
            event_type: "codex_accepted_line_fingerprints",
            event_params: CodexAcceptedLineFingerprintsEventParams {
                event_type: "codex.accepted_line_fingerprints",
                turn_id: "turn-1".to_string(),
                thread_id: "thread-1".to_string(),
                product_surface: Some("codex".to_string()),
                model_slug: Some("gpt-5.1-codex".to_string()),
                completed_at: 1710000000,
                repo_hash: Some("repo-hash-1".to_string()),
                accepted_added_lines: 42,
                accepted_deleted_lines: 40,
                line_fingerprints: Vec::new(),
            },
        },
    ));

    let payload = serde_json::to_value(&event).expect("serialize accepted line fingerprints event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_accepted_line_fingerprints",
            "event_params": {
                "event_type": "codex.accepted_line_fingerprints",
                "turn_id": "turn-1",
                "thread_id": "thread-1",
                "product_surface": "codex",
                "model_slug": "gpt-5.1-codex",
                "completed_at": 1710000000,
                "repo_hash": "repo-hash-1",
                "accepted_added_lines": 42,
                "accepted_deleted_lines": 40,
                "line_fingerprints": []
            }
        })
    );
}

#[tokio::test]
async fn reducer_emits_large_accepted_line_aggregates_without_fingerprints() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut events,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ true,
    )
    .await;
    events.clear();

    let mut diff = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -0,0 +1,20000 @@
"
    .to_string();
    for index in 0..20_000 {
        diff.push_str(&format!("+let value_{index} = {index};\n"));
    }

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::TurnDiffUpdated(
                TurnDiffUpdatedNotification {
                    thread_id: "thread-2".to_string(),
                    turn_id: "turn-2".to_string(),
                    diff,
                },
            ))),
            &mut events,
        )
        .await;
    assert!(events.is_empty());

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut events,
        )
        .await;

    let accepted_line_events = events
        .iter()
        .filter_map(|event| match event {
            TrackEventRequest::AcceptedLineFingerprints(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(accepted_line_events.len(), 1);
    let event = accepted_line_events[0];
    assert_eq!(event.event_params.turn_id, "turn-2");
    assert_eq!(event.event_params.thread_id, "thread-2");
    assert_eq!(event.event_params.accepted_added_lines, 20_000);
    assert_eq!(event.event_params.accepted_deleted_lines, 0);
    assert!(event.event_params.line_fingerprints.is_empty());
    assert!(serde_json::to_vec(event).expect("serialize event").len() < 2_100_000);
}

#[tokio::test]
async fn reducer_emits_accepted_line_fingerprints_once_from_latest_turn_diff_on_completion() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut events,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ true,
    )
    .await;
    events.clear();

    for line in ["let old_value = 1;", "let latest_value = 2;"] {
        let diff = format!(
            "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -0,0 +1 @@
+{line}
"
        );
        reducer
            .ingest(
                AnalyticsFact::Notification(Box::new(ServerNotification::TurnDiffUpdated(
                    TurnDiffUpdatedNotification {
                        thread_id: "thread-2".to_string(),
                        turn_id: "turn-2".to_string(),
                        diff,
                    },
                ))),
                &mut events,
            )
            .await;
    }
    assert!(events.is_empty());

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut events,
        )
        .await;

    let accepted_line_events = events
        .iter()
        .filter_map(|event| match event {
            TrackEventRequest::AcceptedLineFingerprints(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(accepted_line_events.len(), 1);
    let event = accepted_line_events[0];
    assert_eq!(event.event_params.accepted_added_lines, 1);
    assert!(event.event_params.line_fingerprints.is_empty());
}

#[test]
fn compaction_event_serializes_expected_shape() {
    let event = TrackEventRequest::Compaction(Box::new(CodexCompactionEventRequest {
        event_type: "codex_compaction_event",
        event_params: crate::events::codex_compaction_event_params(
            CodexCompactionEvent {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                trigger: CompactionTrigger::Auto,
                reason: CompactionReason::ContextLimit,
                implementation: CompactionImplementation::ResponsesCompact,
                phase: CompactionPhase::MidTurn,
                strategy: CompactionStrategy::Memento,
                status: CompactionStatus::Completed,
                error: None,
                active_context_tokens_before: 120_000,
                active_context_tokens_after: 18_000,
                started_at: 100,
                completed_at: 106,
                duration_ms: Some(6543),
            },
            sample_app_server_client_metadata(),
            sample_runtime_metadata(),
            Some(ThreadSource::User),
            /*subagent_source*/ None,
            /*parent_thread_id*/ None,
        ),
    }));

    let payload = serde_json::to_value(&event).expect("serialize compaction event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_compaction_event",
            "event_params": {
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "app_server_client": {
                    "product_client_id": DEFAULT_ORIGINATOR,
                    "client_name": "codex-tui",
                    "client_version": "1.0.0",
                    "rpc_transport": "stdio",
                    "experimental_api_enabled": true
                },
                "runtime": {
                    "codex_rs_version": "0.1.0",
                    "runtime_os": "macos",
                    "runtime_os_version": "15.3.1",
                    "runtime_arch": "aarch64"
                },
                "thread_source": "user",
                "subagent_source": null,
                "parent_thread_id": null,
                "trigger": "auto",
                "reason": "context_limit",
                "implementation": "responses_compact",
                "phase": "mid_turn",
                "strategy": "memento",
                "status": "completed",
                "error": null,
                "active_context_tokens_before": 120000,
                "active_context_tokens_after": 18000,
                "started_at": 100,
                "completed_at": 106,
                "duration_ms": 6543
            }
        })
    );
}

#[test]
fn app_used_dedupe_is_keyed_by_turn_and_connector() {
    let (sender, _receiver) = mpsc::channel(1);
    let queue = AnalyticsEventsQueue {
        sender,
        app_used_emitted_keys: Arc::new(Mutex::new(HashSet::new())),
        plugin_used_emitted_keys: Arc::new(Mutex::new(HashSet::new())),
    };
    let app = AppInvocation {
        connector_id: Some("calendar".to_string()),
        app_name: Some("Calendar".to_string()),
        invocation_type: Some(InvocationType::Implicit),
    };

    let turn_1 = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let turn_2 = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-2".to_string(),
    };

    assert_eq!(queue.should_enqueue_app_used(&turn_1, &app), true);
    assert_eq!(queue.should_enqueue_app_used(&turn_1, &app), false);
    assert_eq!(queue.should_enqueue_app_used(&turn_2, &app), true);
}

#[test]
fn thread_initialized_event_serializes_expected_shape() {
    let event = TrackEventRequest::ThreadInitialized(ThreadInitializedEvent {
        event_type: "codex_thread_initialized",
        event_params: ThreadInitializedEventParams {
            thread_id: "thread-0".to_string(),
            app_server_client: CodexAppServerClientMetadata {
                product_client_id: DEFAULT_ORIGINATOR.to_string(),
                client_name: Some("codex-tui".to_string()),
                client_version: Some("1.0.0".to_string()),
                rpc_transport: AppServerRpcTransport::Stdio,
                experimental_api_enabled: Some(true),
            },
            runtime: CodexRuntimeMetadata {
                codex_rs_version: "0.1.0".to_string(),
                runtime_os: "macos".to_string(),
                runtime_os_version: "15.3.1".to_string(),
                runtime_arch: "aarch64".to_string(),
            },
            model: "gpt-5".to_string(),
            ephemeral: true,
            thread_source: Some(ThreadSource::User),
            initialization_mode: ThreadInitializationMode::New,
            subagent_source: None,
            parent_thread_id: None,
            created_at: 1,
        },
    });

    let payload = serde_json::to_value(&event).expect("serialize thread initialized event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_thread_initialized",
            "event_params": {
                "thread_id": "thread-0",
                "app_server_client": {
                    "product_client_id": DEFAULT_ORIGINATOR,
                    "client_name": "codex-tui",
                    "client_version": "1.0.0",
                    "rpc_transport": "stdio",
                    "experimental_api_enabled": true
                },
                "runtime": {
                    "codex_rs_version": "0.1.0",
                    "runtime_os": "macos",
                    "runtime_os_version": "15.3.1",
                    "runtime_arch": "aarch64"
                },
                "model": "gpt-5",
                "ephemeral": true,
                "thread_source": "user",
                "initialization_mode": "new",
                "subagent_source": null,
                "parent_thread_id": null,
                "created_at": 1
            }
        })
    );
}

#[test]
fn command_execution_event_serializes_expected_shape() {
    let event = TrackEventRequest::CommandExecution(CodexCommandExecutionEventRequest {
        event_type: "codex_command_execution_event",
        event_params: CodexCommandExecutionEventParams {
            base: CodexToolItemEventBase {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "item-1".to_string(),
                app_server_client: CodexAppServerClientMetadata {
                    product_client_id: "codex_tui".to_string(),
                    client_name: Some("codex-tui".to_string()),
                    client_version: Some("1.2.3".to_string()),
                    rpc_transport: AppServerRpcTransport::Websocket,
                    experimental_api_enabled: Some(true),
                },
                runtime: CodexRuntimeMetadata {
                    codex_rs_version: "0.99.0".to_string(),
                    runtime_os: "macos".to_string(),
                    runtime_os_version: "15.3.1".to_string(),
                    runtime_arch: "aarch64".to_string(),
                },
                thread_source: Some(ThreadSource::User),
                subagent_source: None,
                parent_thread_id: None,
                tool_name: "shell".to_string(),
                started_at_ms: 123_000,
                completed_at_ms: 125_000,
                duration_ms: Some(2000),
                execution_duration_ms: Some(1900),
                review_count: 0,
                guardian_review_count: 0,
                user_review_count: 0,
                final_approval_outcome: FinalApprovalOutcome::NotNeeded,
                terminal_status: ToolItemTerminalStatus::Completed,
                failure_kind: None,
                requested_additional_permissions: false,
                requested_network_access: false,
            },
            command_execution_source: CommandExecutionSource::Agent,
            exit_code: Some(0),
            command_total_action_count: 4,
            command_read_action_count: 1,
            command_list_files_action_count: 1,
            command_search_action_count: 1,
            command_unknown_action_count: 1,
        },
    });

    let payload = serde_json::to_value(&event).expect("serialize command execution event");
    assert_eq!(
        payload,
        json!({
            "event_type": "codex_command_execution_event",
            "event_params": {
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "item_id": "item-1",
                "app_server_client": {
                    "product_client_id": "codex_tui",
                    "client_name": "codex-tui",
                    "client_version": "1.2.3",
                    "rpc_transport": "websocket",
                    "experimental_api_enabled": true
                },
                "runtime": {
                    "codex_rs_version": "0.99.0",
                    "runtime_os": "macos",
                    "runtime_os_version": "15.3.1",
                    "runtime_arch": "aarch64"
                },
                "thread_source": "user",
                "subagent_source": null,
                "parent_thread_id": null,
                "tool_name": "shell",
                "started_at_ms": 123000,
                "completed_at_ms": 125000,
                "duration_ms": 2000,
                "execution_duration_ms": 1900,
                "review_count": 0,
                "guardian_review_count": 0,
                "user_review_count": 0,
                "final_approval_outcome": "not_needed",
                "terminal_status": "completed",
                "failure_kind": null,
                "requested_additional_permissions": false,
                "requested_network_access": false,
                "command_execution_source": "agent",
                "exit_code": 0,
                "command_total_action_count": 4,
                "command_read_action_count": 1,
                "command_list_files_action_count": 1,
                "command_search_action_count": 1,
                "command_unknown_action_count": 1
            }
        })
    );
}

#[test]
fn review_event_serializes_expected_shape() {
    let event = TrackEventRequest::ReviewEvent(CodexReviewEventRequest {
        event_type: "codex_review_event",
        event_params: CodexReviewEventParams {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            item_id: None,
            review_id: "review-1".to_string(),
            app_server_client: CodexAppServerClientMetadata {
                product_client_id: "codex_tui".to_string(),
                client_name: Some("codex-tui".to_string()),
                client_version: Some("1.2.3".to_string()),
                rpc_transport: AppServerRpcTransport::Websocket,
                experimental_api_enabled: Some(true),
            },
            runtime: CodexRuntimeMetadata {
                codex_rs_version: "0.99.0".to_string(),
                runtime_os: "macos".to_string(),
                runtime_os_version: "15.3.1".to_string(),
                runtime_arch: "aarch64".to_string(),
            },
            thread_source: Some(ThreadSource::Subagent),
            subagent_source: Some("thread_spawn".to_string()),
            parent_thread_id: Some("parent-thread-1".to_string()),
            subject_kind: ReviewSubjectKind::NetworkAccess,
            subject_name: "network_access".to_string(),
            reviewer: Reviewer::User,
            trigger: ReviewTrigger::NetworkPolicyDenial,
            status: ReviewStatus::Approved,
            resolution: ReviewResolution::NetworkPolicyAmendment,
            started_at_ms: 123,
            completed_at_ms: 125,
            duration_ms: Some(2),
        },
    });

    let payload = serde_json::to_value(&event).expect("serialize review event");
    assert_eq!(
        payload,
        json!({
            "event_type": "codex_review_event",
            "event_params": {
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "item_id": null,
                "review_id": "review-1",
                "app_server_client": {
                    "product_client_id": "codex_tui",
                    "client_name": "codex-tui",
                    "client_version": "1.2.3",
                    "rpc_transport": "websocket",
                    "experimental_api_enabled": true
                },
                "runtime": {
                    "codex_rs_version": "0.99.0",
                    "runtime_os": "macos",
                    "runtime_os_version": "15.3.1",
                    "runtime_arch": "aarch64"
                },
                "thread_source": "subagent",
                "subagent_source": "thread_spawn",
                "parent_thread_id": "parent-thread-1",
                "subject_kind": "network_access",
                "subject_name": "network_access",
                "reviewer": "user",
                "trigger": "network_policy_denial",
                "status": "approved",
                "resolution": "network_policy_amendment",
                "started_at_ms": 123,
                "completed_at_ms": 125,
                "duration_ms": 2
            }
        })
    );
}
