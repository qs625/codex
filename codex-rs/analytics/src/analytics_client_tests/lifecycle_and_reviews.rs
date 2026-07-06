use super::*;

#[tokio::test]
async fn initialize_caches_client_and_thread_lifecycle_publishes_once_initialized() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(1),
                response: Box::new(sample_thread_start_response(
                    "thread-no-client",
                    /*ephemeral*/ false,
                    "gpt-5",
                )),
            },
            &mut events,
        )
        .await;
    assert!(events.is_empty(), "thread events should require initialize");

    reducer
        .ingest(
            AnalyticsFact::Initialize {
                connection_id: 7,
                params: InitializeParams {
                    client_info: ClientInfo {
                        name: "codex-tui".to_string(),
                        title: None,
                        version: "1.0.0".to_string(),
                    },
                    capabilities: Some(InitializeCapabilities {
                        experimental_api: false,
                        request_attestation: false,
                        opt_out_notification_methods: None,
                    }),
                },
                product_client_id: DEFAULT_ORIGINATOR.to_string(),
                runtime: CodexRuntimeMetadata {
                    codex_rs_version: "0.99.0".to_string(),
                    runtime_os: "linux".to_string(),
                    runtime_os_version: "24.04".to_string(),
                    runtime_arch: "x86_64".to_string(),
                },
                rpc_transport: AppServerRpcTransport::Websocket,
            },
            &mut events,
        )
        .await;
    assert!(events.is_empty(), "initialize should not publish by itself");

    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(2),
                response: Box::new(sample_thread_resume_response(
                    "thread-1", /*ephemeral*/ true, "gpt-5",
                )),
            },
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_thread_initialized");
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["product_client_id"],
        DEFAULT_ORIGINATOR
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["client_name"],
        "codex-tui"
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["client_version"],
        "1.0.0"
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["rpc_transport"],
        "websocket"
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["experimental_api_enabled"],
        false
    );
    assert_eq!(
        payload[0]["event_params"]["runtime"]["codex_rs_version"],
        "0.99.0"
    );
    assert_eq!(payload[0]["event_params"]["runtime"]["runtime_os"], "linux");
    assert_eq!(
        payload[0]["event_params"]["runtime"]["runtime_os_version"],
        "24.04"
    );
    assert_eq!(
        payload[0]["event_params"]["runtime"]["runtime_arch"],
        "x86_64"
    );
}

#[tokio::test]
async fn unrelated_client_requests_are_ignored_by_reducer() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(3),
                request: Box::new(ClientRequest::ThreadArchive {
                    request_id: RequestId::Integer(3),
                    params: ThreadArchiveParams {
                        thread_id: "thread-2".to_string(),
                    },
                }),
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(3),
                response: Box::new(sample_turn_start_response("turn-2")),
            },
            &mut events,
        )
        .await;

    assert!(
        events.is_empty(),
        "unrelated requests must not create pending turn state"
    );
}

#[tokio::test]
async fn unrelated_client_responses_are_ignored_by_reducer() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_initialize(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(9),
                response: Box::new(ClientResponsePayload::ThreadArchive(
                    ThreadArchiveResponse {},
                )),
            },
            &mut events,
        )
        .await;

    assert!(events.is_empty());
}

#[tokio::test]
async fn compaction_event_ingests_custom_fact() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();
    let parent_thread_id = protocol::ThreadId::from_string("22222222-2222-2222-2222-222222222222")
        .expect("valid parent thread id");

    reducer
        .ingest(
            AnalyticsFact::Initialize {
                connection_id: 7,
                params: InitializeParams {
                    client_info: ClientInfo {
                        name: "codex-tui".to_string(),
                        title: None,
                        version: "1.0.0".to_string(),
                    },
                    capabilities: Some(InitializeCapabilities {
                        experimental_api: false,
                        request_attestation: false,
                        opt_out_notification_methods: None,
                    }),
                },
                product_client_id: DEFAULT_ORIGINATOR.to_string(),
                runtime: sample_runtime_metadata(),
                rpc_transport: AppServerRpcTransport::Websocket,
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(2),
                response: Box::new(sample_thread_resume_response_with_source(
                    "thread-1",
                    /*ephemeral*/ false,
                    "gpt-5",
                    AppServerSessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                        parent_thread_id,
                        depth: 1,
                        agent_path: None,
                        agent_nickname: None,
                        agent_role: None,
                    }),
                    Some(AppServerThreadSource::Subagent),
                )),
            },
            &mut events,
        )
        .await;
    events.clear();

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::Compaction(Box::new(
                CodexCompactionEvent {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-compact".to_string(),
                    trigger: CompactionTrigger::Manual,
                    reason: CompactionReason::UserRequested,
                    implementation: CompactionImplementation::Responses,
                    phase: CompactionPhase::StandaloneTurn,
                    strategy: CompactionStrategy::Memento,
                    status: CompactionStatus::Failed,
                    error: Some("context limit exceeded".to_string()),
                    active_context_tokens_before: 131_000,
                    active_context_tokens_after: 131_000,
                    started_at: 100,
                    completed_at: 101,
                    duration_ms: Some(1200),
                },
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_compaction_event");
    assert_eq!(payload[0]["event_params"]["thread_id"], "thread-1");
    assert_eq!(payload[0]["event_params"]["turn_id"], "turn-compact");
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["product_client_id"],
        DEFAULT_ORIGINATOR
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["client_name"],
        "codex-tui"
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["rpc_transport"],
        "websocket"
    );
    assert_eq!(
        payload[0]["event_params"]["runtime"]["codex_rs_version"],
        "0.1.0"
    );
    assert_eq!(payload[0]["event_params"]["thread_source"], "subagent");
    assert_eq!(
        payload[0]["event_params"]["subagent_source"],
        "thread_spawn"
    );
    assert_eq!(
        payload[0]["event_params"]["parent_thread_id"],
        "22222222-2222-2222-2222-222222222222"
    );
    assert_eq!(payload[0]["event_params"]["trigger"], "manual");
    assert_eq!(payload[0]["event_params"]["reason"], "user_requested");
    assert_eq!(payload[0]["event_params"]["implementation"], "responses");
    assert_eq!(payload[0]["event_params"]["phase"], "standalone_turn");
    assert_eq!(payload[0]["event_params"]["strategy"], "memento");
    assert_eq!(payload[0]["event_params"]["status"], "failed");
}

#[tokio::test]
async fn guardian_review_event_ingests_custom_fact_with_optional_target_item() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::Initialize {
                connection_id: 7,
                params: InitializeParams {
                    client_info: ClientInfo {
                        name: "codex-tui".to_string(),
                        title: None,
                        version: "1.0.0".to_string(),
                    },
                    capabilities: Some(InitializeCapabilities {
                        experimental_api: false,
                        request_attestation: false,
                        opt_out_notification_methods: None,
                    }),
                },
                product_client_id: DEFAULT_ORIGINATOR.to_string(),
                runtime: sample_runtime_metadata(),
                rpc_transport: AppServerRpcTransport::Websocket,
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(1),
                response: Box::new(sample_thread_start_response(
                    "thread-guardian",
                    /*ephemeral*/ false,
                    "gpt-5",
                )),
            },
            &mut events,
        )
        .await;
    events.clear();

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::GuardianReview(Box::new(
                GuardianReviewEventParams {
                    thread_id: "thread-guardian".to_string(),
                    turn_id: "turn-guardian".to_string(),
                    review_id: "review-guardian".to_string(),
                    target_item_id: None,
                    approval_request_source: GuardianApprovalRequestSource::DelegatedSubagent,
                    reviewed_action: GuardianReviewedAction::NetworkAccess {
                        protocol: NetworkApprovalProtocol::Https,
                        port: 443,
                    },
                    reviewed_action_truncated: false,
                    decision: GuardianReviewDecision::Denied,
                    terminal_status: GuardianReviewTerminalStatus::TimedOut,
                    failure_reason: Some(GuardianReviewFailureReason::Timeout),
                    risk_level: None,
                    user_authorization: None,
                    outcome: None,
                    guardian_thread_id: None,
                    guardian_session_kind: None,
                    guardian_model: None,
                    guardian_reasoning_effort: None,
                    had_prior_review_context: None,
                    review_timeout_ms: 90_000,
                    tool_call_count: None,
                    time_to_first_token_ms: None,
                    completion_latency_ms: Some(90_000),
                    started_at: 100,
                    completed_at: Some(190),
                    input_tokens: None,
                    cached_input_tokens: None,
                    output_tokens: None,
                    reasoning_output_tokens: None,
                    total_tokens: None,
                },
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_guardian_review");
    assert_eq!(payload[0]["event_params"]["thread_id"], "thread-guardian");
    assert_eq!(payload[0]["event_params"]["turn_id"], "turn-guardian");
    assert_eq!(payload[0]["event_params"]["review_id"], "review-guardian");
    assert_eq!(payload[0]["event_params"]["target_item_id"], json!(null));
    assert_eq!(
        payload[0]["event_params"]["approval_request_source"],
        "delegated_subagent"
    );
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["product_client_id"],
        DEFAULT_ORIGINATOR
    );
    assert_eq!(
        payload[0]["event_params"]["runtime"]["codex_rs_version"],
        "0.1.0"
    );
    assert_eq!(
        payload[0]["event_params"]["reviewed_action"]["type"],
        "network_access"
    );
    assert_eq!(
        payload[0]["event_params"]["reviewed_action"]["protocol"],
        "https"
    );
    assert_eq!(payload[0]["event_params"]["reviewed_action"]["port"], 443);
    assert!(payload[0]["event_params"].get("retry_reason").is_none());
    assert!(payload[0]["event_params"].get("rationale").is_none());
    assert!(
        payload[0]["event_params"]["reviewed_action"]
            .get("target")
            .is_none()
    );
    assert!(
        payload[0]["event_params"]["reviewed_action"]
            .get("host")
            .is_none()
    );
    assert_eq!(payload[0]["event_params"]["terminal_status"], "timed_out");
    assert_eq!(payload[0]["event_params"]["failure_reason"], "timeout");
    assert_eq!(payload[0]["event_params"]["review_timeout_ms"], 90_000);
}

#[tokio::test]
async fn item_lifecycle_notifications_publish_command_execution_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_started_notification(
                "thread-1", "turn-1",
            ))),
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::ItemStarted(
                ItemStartedNotification {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    started_at_ms: 1_000,
                    item: sample_command_execution_item(
                        CommandExecutionStatus::InProgress,
                        /*exit_code*/ None,
                        /*duration_ms*/ None,
                    ),
                },
            ))),
            &mut events,
        )
        .await;
    assert!(
        events.is_empty(),
        "tool item event should emit on completion"
    );

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::ItemCompleted(
                ItemCompletedNotification {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    completed_at_ms: 1_045,
                    item: sample_command_execution_item_with_actions(
                        CommandExecutionStatus::Completed,
                        Some(0),
                        Some(42),
                        vec![
                            CommandAction::Read {
                                command: "cat README.md".to_string(),
                                name: "README.md".to_string(),
                                path: test_path_buf("/tmp/README.md").abs(),
                            },
                            CommandAction::ListFiles {
                                command: "ls".to_string(),
                                path: None,
                            },
                            CommandAction::Search {
                                command: "rg TODO".to_string(),
                                query: Some("TODO".to_string()),
                                path: None,
                            },
                            CommandAction::Unknown {
                                command: "cargo test".to_string(),
                            },
                        ],
                    ),
                },
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_command_execution_event");
    assert_eq!(payload[0]["event_params"]["thread_id"], "thread-1");
    assert_eq!(payload[0]["event_params"]["turn_id"], "turn-1");
    assert_eq!(payload[0]["event_params"]["item_id"], "item-1");
    assert_eq!(payload[0]["event_params"]["tool_name"], "shell");
    assert_eq!(
        payload[0]["event_params"]["command_execution_source"],
        "agent"
    );
    assert_eq!(payload[0]["event_params"]["terminal_status"], "completed");
    assert_eq!(
        payload[0]["event_params"]["final_approval_outcome"],
        "unknown"
    );
    assert_eq!(
        payload[0]["event_params"]["failure_kind"],
        serde_json::Value::Null
    );
    assert_eq!(payload[0]["event_params"]["exit_code"], 0);
    assert_eq!(payload[0]["event_params"]["command_total_action_count"], 4);
    assert_eq!(payload[0]["event_params"]["command_read_action_count"], 1);
    assert_eq!(
        payload[0]["event_params"]["command_list_files_action_count"],
        1
    );
    assert_eq!(payload[0]["event_params"]["command_search_action_count"], 1);
    assert_eq!(
        payload[0]["event_params"]["command_unknown_action_count"],
        1
    );
    assert_eq!(payload[0]["event_params"]["started_at_ms"], 1_000);
    assert_eq!(payload[0]["event_params"]["completed_at_ms"], 1_045);
    assert_eq!(payload[0]["event_params"]["duration_ms"], 45);
    assert_eq!(payload[0]["event_params"]["execution_duration_ms"], 42);
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["client_name"],
        "codex-tui"
    );
    assert_eq!(payload[0]["event_params"]["thread_source"], "user");
}

#[tokio::test]
async fn command_execution_approval_response_publishes_user_review_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_command_approval_request(
                    /*request_id*/ 41, /*approval_id*/ None,
                )),
            },
            &mut events,
        )
        .await;
    assert!(events.is_empty());

    reducer
        .ingest(
            AnalyticsFact::ServerResponse {
                completed_at_ms: 1_042,
                response: Box::new(sample_command_approval_response(
                    /*request_id*/ 41,
                    CommandExecutionApprovalDecision::Accept,
                )),
            },
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_review_event");
    assert_eq!(payload[0]["event_params"]["thread_id"], "thread-1");
    assert_eq!(payload[0]["event_params"]["turn_id"], "turn-1");
    assert_eq!(payload[0]["event_params"]["item_id"], "item-1");
    assert_eq!(payload[0]["event_params"]["review_id"], "user:41");
    assert_eq!(payload[0]["event_params"]["thread_source"], "user");
    assert_eq!(
        payload[0]["event_params"]["subject_kind"],
        "command_execution"
    );
    assert_eq!(
        payload[0]["event_params"]["subject_name"],
        "command_execution"
    );
    assert_eq!(payload[0]["event_params"]["reviewer"], "user");
    assert_eq!(payload[0]["event_params"]["trigger"], "initial");
    assert_eq!(payload[0]["event_params"]["status"], "approved");
    assert_eq!(payload[0]["event_params"]["started_at_ms"], 1_000);
    assert_eq!(payload[0]["event_params"]["completed_at_ms"], 1_042);
    assert_eq!(payload[0]["event_params"]["duration_ms"], 42);
}

#[tokio::test]
async fn permissions_reviews_emit_events_without_denormalizing_onto_tool_items() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_permissions_approval_request(/*request_id*/ 51)),
            },
            &mut events,
        )
        .await;
    assert!(events.is_empty());

    reducer
        .ingest(
            AnalyticsFact::EffectivePermissionsApprovalResponse {
                completed_at_ms: 1_042,
                request_id: RequestId::Integer(51),
                response: Box::new(sample_effective_permissions_approval_response(
                    CoreRequestPermissionProfile::default(),
                    CorePermissionGrantScope::Turn,
                )),
            },
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_review_event");
    assert_eq!(payload[0]["event_params"]["review_id"], "user:51");
    assert_eq!(payload[0]["event_params"]["subject_kind"], "permissions");
    assert_eq!(payload[0]["event_params"]["reviewer"], "user");
    assert_eq!(payload[0]["event_params"]["status"], "denied");
    assert_eq!(payload[0]["event_params"]["resolution"], "none");

    events.clear();
    ingest_completed_command_execution_item(&mut reducer, &mut events, "thread-1", "permissions-1")
        .await;

    let payload = serde_json::to_value(&events[0]).expect("serialize tool item event");
    assert_eq!(payload["event_params"]["item_id"], "permissions-1");
    assert_eq!(payload["event_params"]["review_count"], 0);
    assert_eq!(payload["event_params"]["user_review_count"], 0);
    assert_eq!(payload["event_params"]["guardian_review_count"], 0);
}

#[tokio::test]
async fn effective_session_permissions_response_publishes_session_user_review_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_permissions_approval_request(/*request_id*/ 52)),
            },
            &mut events,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::EffectivePermissionsApprovalResponse {
                completed_at_ms: 1_042,
                request_id: RequestId::Integer(52),
                response: Box::new(sample_effective_permissions_approval_response(
                    CoreRequestPermissionProfile {
                        network: Some(CoreNetworkPermissions {
                            enabled: Some(true),
                        }),
                        file_system: None,
                    },
                    CorePermissionGrantScope::Session,
                )),
            },
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_review_event");
    assert_eq!(payload[0]["event_params"]["review_id"], "user:52");
    assert_eq!(payload[0]["event_params"]["subject_kind"], "permissions");
    assert_eq!(payload[0]["event_params"]["reviewer"], "user");
    assert_eq!(payload[0]["event_params"]["status"], "approved");
    assert_eq!(payload[0]["event_params"]["resolution"], "session_approval");
}

#[tokio::test]
async fn aborted_server_request_publishes_aborted_user_review_event_once() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_command_approval_request(
                    /*request_id*/ 61, /*approval_id*/ None,
                )),
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequestAborted {
                completed_at_ms: 1_042,
                request_id: RequestId::Integer(61),
            },
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_params"]["review_id"], "user:61");
    assert_eq!(payload[0]["event_params"]["status"], "aborted");
    assert_eq!(payload[0]["event_params"]["resolution"], "none");

    events.clear();
    reducer
        .ingest(
            AnalyticsFact::ServerResponse {
                completed_at_ms: 1_043,
                response: Box::new(sample_command_approval_response(
                    /*request_id*/ 61,
                    CommandExecutionApprovalDecision::Accept,
                )),
            },
            &mut events,
        )
        .await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn guardian_completed_notification_publishes_review_event_with_thread_metadata() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_guardian_review_completed(
                "guardian-review-1",
                Some("item-1"),
                GuardianApprovalReviewStatus::Denied,
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events[0]).expect("serialize review event");
    assert_eq!(payload["event_type"], "codex_review_event");
    assert_eq!(payload["event_params"]["review_id"], "guardian-review-1");
    assert_eq!(payload["event_params"]["item_id"], "item-1");
    assert_eq!(payload["event_params"]["thread_source"], "user");
    assert_eq!(payload["event_params"]["subject_kind"], "command_execution");
    assert_eq!(payload["event_params"]["reviewer"], "guardian");
    assert_eq!(payload["event_params"]["status"], "denied");
    assert_eq!(payload["event_params"]["started_at_ms"], 1_000);
    assert_eq!(payload["event_params"]["completed_at_ms"], 1_042);
    assert_eq!(payload["event_params"]["duration_ms"], 42);
}

#[tokio::test]
async fn terminal_reviews_denormalize_counts_onto_tool_item_events() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_command_approval_request(
                    /*request_id*/ 71, /*approval_id*/ None,
                )),
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ServerResponse {
                completed_at_ms: 1_042,
                response: Box::new(sample_command_approval_response(
                    /*request_id*/ 71,
                    CommandExecutionApprovalDecision::AcceptForSession,
                )),
            },
            &mut events,
        )
        .await;
    events.clear();

    ingest_completed_command_execution_item(&mut reducer, &mut events, "thread-1", "item-1").await;

    let payload = serde_json::to_value(&events[0]).expect("serialize tool item event");
    assert_eq!(payload["event_params"]["review_count"], 1);
    assert_eq!(payload["event_params"]["user_review_count"], 1);
    assert_eq!(payload["event_params"]["guardian_review_count"], 0);
    assert_eq!(
        payload["event_params"]["final_approval_outcome"],
        "user_approved_for_session"
    );
}

#[tokio::test]
async fn item_review_summaries_do_not_cross_threads_with_reused_item_ids() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(2),
                response: Box::new(sample_thread_start_response(
                    "thread-2", /*ephemeral*/ false, "gpt-5",
                )),
            },
            &mut events,
        )
        .await;
    events.clear();

    reducer
        .ingest(
            AnalyticsFact::ServerRequest {
                connection_id: 7,
                request: Box::new(sample_command_approval_request(
                    /*request_id*/ 72, /*approval_id*/ None,
                )),
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ServerResponse {
                completed_at_ms: 1_042,
                response: Box::new(sample_command_approval_response(
                    /*request_id*/ 72,
                    CommandExecutionApprovalDecision::Accept,
                )),
            },
            &mut events,
        )
        .await;
    events.clear();

    ingest_completed_command_execution_item(&mut reducer, &mut events, "thread-2", "item-1").await;

    let payload = serde_json::to_value(&events[0]).expect("serialize tool item event");
    assert_eq!(payload["event_params"]["thread_id"], "thread-2");
    assert_eq!(payload["event_params"]["item_id"], "item-1");
    assert_eq!(payload["event_params"]["review_count"], 0);
    assert_eq!(payload["event_params"]["user_review_count"], 0);
    assert_eq!(payload["event_params"]["guardian_review_count"], 0);
    assert_eq!(payload["event_params"]["final_approval_outcome"], "unknown");
}

#[test]
fn subagent_thread_started_review_serializes_expected_shape() {
    let event = TrackEventRequest::ThreadInitialized(subagent_thread_started_event_request(
        SubAgentThreadStartedInput {
            thread_id: "thread-review".to_string(),
            parent_thread_id: None,
            product_client_id: "codex-tui".to_string(),
            client_name: "codex-tui".to_string(),
            client_version: "1.0.0".to_string(),
            model: "gpt-5".to_string(),
            ephemeral: false,
            subagent_source: SubAgentSource::Review,
            created_at: 123,
        },
    ));

    let payload = serde_json::to_value(&event).expect("serialize review subagent event");
    assert_eq!(payload["event_params"]["thread_source"], "subagent");
    assert_eq!(
        payload["event_params"]["app_server_client"]["product_client_id"],
        "codex-tui"
    );
    assert_eq!(
        payload["event_params"]["app_server_client"]["client_name"],
        "codex-tui"
    );
    assert_eq!(
        payload["event_params"]["app_server_client"]["client_version"],
        "1.0.0"
    );
    assert_eq!(
        payload["event_params"]["app_server_client"]["rpc_transport"],
        "in_process"
    );
    assert_eq!(payload["event_params"]["created_at"], 123);
    assert_eq!(payload["event_params"]["initialization_mode"], "new");
    assert_eq!(payload["event_params"]["subagent_source"], "review");
    assert_eq!(payload["event_params"]["parent_thread_id"], json!(null));
}

#[test]
fn subagent_thread_started_thread_spawn_serializes_parent_thread_id() {
    let parent_thread_id = protocol::ThreadId::from_string("11111111-1111-1111-1111-111111111111")
        .expect("valid thread id");
    let event = TrackEventRequest::ThreadInitialized(subagent_thread_started_event_request(
        SubAgentThreadStartedInput {
            thread_id: "thread-spawn".to_string(),
            parent_thread_id: None,
            product_client_id: "codex-tui".to_string(),
            client_name: "codex-tui".to_string(),
            client_version: "1.0.0".to_string(),
            model: "gpt-5".to_string(),
            ephemeral: true,
            subagent_source: SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            },
            created_at: 124,
        },
    ));

    let payload = serde_json::to_value(&event).expect("serialize thread spawn subagent event");
    assert_eq!(payload["event_params"]["thread_source"], "subagent");
    assert_eq!(payload["event_params"]["subagent_source"], "thread_spawn");
    assert_eq!(
        payload["event_params"]["parent_thread_id"],
        "11111111-1111-1111-1111-111111111111"
    );
}

#[test]
fn subagent_thread_started_memory_consolidation_serializes_expected_shape() {
    let event = TrackEventRequest::ThreadInitialized(subagent_thread_started_event_request(
        SubAgentThreadStartedInput {
            thread_id: "thread-memory".to_string(),
            parent_thread_id: None,
            product_client_id: "codex-tui".to_string(),
            client_name: "codex-tui".to_string(),
            client_version: "1.0.0".to_string(),
            model: "gpt-5".to_string(),
            ephemeral: false,
            subagent_source: SubAgentSource::MemoryConsolidation,
            created_at: 125,
        },
    ));

    let payload =
        serde_json::to_value(&event).expect("serialize memory consolidation subagent event");
    assert_eq!(
        payload["event_params"]["subagent_source"],
        "memory_consolidation"
    );
    assert_eq!(payload["event_params"]["parent_thread_id"], json!(null));
}

#[test]
fn subagent_thread_started_other_serializes_expected_shape() {
    let event = TrackEventRequest::ThreadInitialized(subagent_thread_started_event_request(
        SubAgentThreadStartedInput {
            thread_id: "thread-guardian".to_string(),
            parent_thread_id: None,
            product_client_id: "codex-tui".to_string(),
            client_name: "codex-tui".to_string(),
            client_version: "1.0.0".to_string(),
            model: "gpt-5".to_string(),
            ephemeral: false,
            subagent_source: SubAgentSource::Other("guardian".to_string()),
            created_at: 126,
        },
    ));

    let payload = serde_json::to_value(&event).expect("serialize other subagent event");
    assert_eq!(payload["event_params"]["subagent_source"], "guardian");
    assert_eq!(payload["event_params"]["parent_thread_id"], json!(null));
}

#[test]
fn subagent_thread_started_other_serializes_explicit_parent_thread_id() {
    let event = TrackEventRequest::ThreadInitialized(subagent_thread_started_event_request(
        SubAgentThreadStartedInput {
            thread_id: "thread-guardian".to_string(),
            parent_thread_id: Some("parent-thread-guardian".to_string()),
            product_client_id: "codex-tui".to_string(),
            client_name: "codex-tui".to_string(),
            client_version: "1.0.0".to_string(),
            model: "gpt-5".to_string(),
            ephemeral: false,
            subagent_source: SubAgentSource::Other("guardian".to_string()),
            created_at: 126,
        },
    ));

    let payload = serde_json::to_value(&event).expect("serialize auto-review subagent event");
    assert_eq!(payload["event_params"]["subagent_source"], "guardian");
    assert_eq!(
        payload["event_params"]["parent_thread_id"],
        "parent-thread-guardian"
    );
}
