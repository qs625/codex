use super::*;

#[tokio::test]
async fn turn_event_counts_completed_tool_items() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ false,
    )
    .await;

    let completed_tool_items = vec![
        sample_command_execution_item(CommandExecutionStatus::Completed, Some(0), Some(1)),
        ThreadItem::FileChange {
            id: "file-change-1".to_string(),
            changes: Vec::new(),
            status: PatchApplyStatus::Completed,
        },
        ThreadItem::McpToolCall {
            id: "mcp-1".to_string(),
            server: "server".to_string(),
            tool: "search".to_string(),
            status: McpToolCallStatus::Completed,
            arguments: json!({}),
            mcp_app_resource_uri: None,
            result: None,
            error: None,
            duration_ms: Some(2),
        },
        ThreadItem::DynamicToolCall {
            id: "dynamic-1".to_string(),
            namespace: None,
            tool: "render".to_string(),
            arguments: json!({}),
            status: DynamicToolCallStatus::Completed,
            content_items: None,
            success: Some(true),
            duration_ms: Some(3),
        },
        ThreadItem::CollabAgentToolCall {
            id: "collab-1".to_string(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::Completed,
            sender_thread_id: "thread-2".to_string(),
            sender_path: "/root/worker".to_string(),
            receiver_thread_ids: vec!["thread-child".to_string()],
            receiver_paths: vec!["/root/worker/child".to_string()],
            timeout_ms: None,
            prompt: Some("help".to_string()),
            model: Some("gpt-5".to_string()),
            reasoning_effort: None,
            agents_states: Default::default(),
        },
        ThreadItem::WebSearch {
            id: "web-1".to_string(),
            query: "codex".to_string(),
            action: None,
        },
        ThreadItem::ImageGeneration {
            id: "image-1".to_string(),
            status: "completed".to_string(),
            revised_prompt: None,
            result: "ok".to_string(),
            saved_path: None,
        },
    ];

    for item in completed_tool_items {
        reducer
            .ingest(
                AnalyticsFact::Notification(Box::new(ServerNotification::ItemCompleted(
                    ItemCompletedNotification {
                        thread_id: "thread-2".to_string(),
                        turn_id: "turn-2".to_string(),
                        completed_at_ms: 1_000,
                        item,
                    },
                ))),
                &mut out,
            )
            .await;
    }

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    let turn_event = out
        .iter()
        .find(|event| matches!(event, TrackEventRequest::TurnEvent(_)))
        .expect("turn event should be emitted");
    let payload = serde_json::to_value(turn_event).expect("serialize turn event");
    assert_eq!(payload["event_params"]["total_tool_call_count"], json!(7));
    assert_eq!(payload["event_params"]["shell_command_count"], json!(1));
    assert_eq!(payload["event_params"]["file_change_count"], json!(1));
    assert_eq!(payload["event_params"]["mcp_tool_call_count"], json!(1));
    assert_eq!(payload["event_params"]["dynamic_tool_call_count"], json!(1));
    assert_eq!(
        payload["event_params"]["subagent_tool_call_count"],
        json!(1)
    );
    assert_eq!(payload["event_params"]["web_search_count"], json!(1));
    assert_eq!(payload["event_params"]["image_generation_count"], json!(1));
}

#[tokio::test]
async fn item_completed_without_turn_state_does_not_create_turn_state() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::ItemCompleted(
                ItemCompletedNotification {
                    thread_id: "thread-2".to_string(),
                    turn_id: "turn-2".to_string(),
                    completed_at_ms: 1_000,
                    item: sample_command_execution_item(
                        CommandExecutionStatus::Completed,
                        Some(0),
                        Some(1),
                    ),
                },
            ))),
            &mut out,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    assert!(out.is_empty());
}

#[tokio::test]
async fn accepted_steers_increment_turn_steer_count() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ false,
    )
    .await;

    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(4),
                request: Box::new(sample_turn_steer_request(
                    "thread-2", "turn-2", /*request_id*/ 4,
                )),
            },
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(4),
                response: Box::new(sample_turn_steer_response("turn-2")),
            },
            &mut out,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(5),
                request: Box::new(sample_turn_steer_request(
                    "thread-2", "turn-2", /*request_id*/ 5,
                )),
            },
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ErrorResponse {
                connection_id: 7,
                request_id: RequestId::Integer(5),
                error: no_active_turn_steer_error(),
                error_type: Some(no_active_turn_steer_error_type()),
            },
            &mut out,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(6),
                request: Box::new(sample_turn_steer_request(
                    "thread-2", "turn-2", /*request_id*/ 6,
                )),
            },
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(6),
                response: Box::new(sample_turn_steer_response("turn-2")),
            },
            &mut out,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    let turn_event = out
        .iter()
        .find(|event| matches!(event, TrackEventRequest::TurnEvent(_)))
        .expect("turn event should be emitted");
    let payload = serde_json::to_value(turn_event).expect("serialize turn event");
    assert_eq!(payload["event_params"]["steer_count"], json!(2));
}

#[tokio::test]
async fn turn_does_not_emit_without_required_prerequisites() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ false,
        /*include_resolved_config*/ true,
        /*include_started*/ false,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;
    assert!(out.is_empty());

    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ false,
        /*include_started*/ false,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;
    assert!(out.is_empty());
}

#[tokio::test]
async fn turn_lifecycle_emits_failed_turn_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Failed,
                Some(app_server_protocol::CodexErrorInfo::BadRequest),
            ))),
            &mut out,
        )
        .await;

    assert_eq!(out.len(), 1);
    let payload = serde_json::to_value(&out[0]).expect("serialize turn event");
    assert_eq!(payload["event_params"]["status"], json!("failed"));
    assert_eq!(payload["event_params"]["turn_error"], json!("badRequest"));
}

#[tokio::test]
async fn turn_lifecycle_emits_interrupted_turn_event_without_error() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Interrupted,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    assert_eq!(out.len(), 1);
    let payload = serde_json::to_value(&out[0]).expect("serialize turn event");
    assert_eq!(payload["event_params"]["status"], json!("interrupted"));
    assert_eq!(payload["event_params"]["turn_error"], json!(null));
}

#[tokio::test]
async fn turn_completed_without_started_notification_emits_null_started_at() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ false,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    let payload = serde_json::to_value(&out[0]).expect("serialize turn event");
    assert_eq!(payload["event_params"]["started_at"], json!(null));
    assert_eq!(payload["event_params"]["duration_ms"], json!(1234));
    assert_eq!(payload["event_params"]["input_tokens"], json!(null));
    assert_eq!(payload["event_params"]["cached_input_tokens"], json!(null));
    assert_eq!(payload["event_params"]["output_tokens"], json!(null));
    assert_eq!(
        payload["event_params"]["reasoning_output_tokens"],
        json!(null)
    );
    assert_eq!(payload["event_params"]["total_tokens"], json!(null));
}
