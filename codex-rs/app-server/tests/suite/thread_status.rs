use anyhow::Result;
use app_server_protocol::ClientInfo;
use app_server_protocol::InitializeCapabilities;
use app_server_protocol::JSONRPCMessage;
use app_server_protocol::JSONRPCNotification;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ThreadIdleReason;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadListParams;
use app_server_protocol::ThreadListResponse;
use app_server_protocol::ThreadLoadedListParams;
use app_server_protocol::ThreadLoadedListResponse;
use app_server_protocol::ThreadReadParams;
use app_server_protocol::ThreadReadResponse;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadStatus;
use app_server_protocol::ThreadStatusChangedNotification;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::UserInput as V2UserInput;
use app_test_support::McpProcess;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence;
use app_test_support::to_response;
use core_test_support::responses;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thread_status_changed_emits_runtime_updates() -> Result<()> {
    let codex_home = TempDir::new()?;
    let responses = vec![create_final_assistant_message_sse_response("done")?];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp =
        McpProcess::new_with_env(codex_home.path(), &[("RUST_LOG", Some("info"))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

    let turn_start_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "collect status updates".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let turn_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response(turn_start_resp)?;

    let mut saw_active_running = false;
    let mut saw_idle_after_turn = false;
    let deadline = tokio::time::Instant::now() + DEFAULT_READ_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = match timeout(remaining, mcp.read_next_message()).await {
            Ok(Ok(message)) => message,
            _ => break,
        };
        match message {
            JSONRPCMessage::Notification(JSONRPCNotification {
                method,
                params: Some(params),
            }) if method == "thread/status/changed" => {
                let notification: ThreadStatusChangedNotification = serde_json::from_value(params)?;
                if notification.thread_id != thread.id {
                    continue;
                }
                match notification.status {
                    ThreadStatus::Active { .. } => {
                        saw_active_running = true;
                    }
                    ThreadStatus::Idle { .. } => {}
                    ThreadStatus::Complete => {
                        if saw_active_running {
                            saw_idle_after_turn = true;
                        }
                    }
                    ThreadStatus::SystemError => {
                        if saw_active_running {
                            saw_idle_after_turn = true;
                        }
                    }
                    ThreadStatus::NotLoaded => {
                        if saw_active_running {
                            saw_idle_after_turn = true;
                        }
                    }
                }
            }
            _ => {}
        }

        if saw_active_running && saw_idle_after_turn {
            break;
        }
    }

    assert!(
        saw_active_running,
        "expected running active flag in thread/status/changed notifications"
    );
    assert!(
        saw_idle_after_turn,
        "expected idle status after turn completion in thread/status/changed notifications"
    );
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn thread_status_changed_can_be_opted_out() -> Result<()> {
    let codex_home = TempDir::new()?;
    let responses = vec![create_final_assistant_message_sse_response("done")?];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    let message = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.initialize_with_capabilities(
            ClientInfo {
                name: "codex_vscode".to_string(),
                title: Some("Codex VS Code Extension".to_string()),
                version: "0.1.0".to_string(),
            },
            Some(InitializeCapabilities {
                experimental_api: true,
                request_attestation: false,
                opt_out_notification_methods: Some(vec!["thread/status/changed".to_string()]),
            }),
        ),
    )
    .await??;
    let JSONRPCMessage::Response(_) = message else {
        anyhow::bail!("expected initialize response, got {message:?}");
    };

    let thread_start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

    let turn_start_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "run once".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let turn_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response(turn_start_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let status_update = timeout(
        std::time::Duration::from_millis(500),
        mcp.read_stream_until_notification_message("thread/status/changed"),
    )
    .await;
    match status_update {
        Err(_) => {}
        Ok(Ok(notification)) => {
            anyhow::bail!(
                "thread/status/changed should be filtered by optOutNotificationMethods; got: {notification:?}"
            );
        }
        Ok(Err(err)) => {
            anyhow::bail!(
                "expected timeout waiting for filtered thread/status/changed, got: {err}"
            );
        }
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thread_read_stays_active_while_event_subscription_is_pending() -> Result<()> {
    let codex_home = TempDir::new()?;
    let schedule_args = serde_json::to_string(&json!({
        "schedule": {
            "kind": "once_after",
            "delay_ms": 60_000u64
        }
    }))?;
    let responses = vec![
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_function_call("call-schedule", "schedule_subscribe", &schedule_args),
            responses::ev_completed("resp-1"),
        ]),
        create_final_assistant_message_sse_response("scheduled")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp =
        McpProcess::new_with_env(codex_home.path(), &[("RUST_LOG", Some("info"))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

    let turn_start_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "schedule a reminder".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let turn_start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response(turn_start_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let thread_read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id.clone(),
            include_turns: true,
        })
        .await?;
    let thread_read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_read_id)),
    )
    .await??;
    let ThreadReadResponse { thread } = to_response(thread_read_resp)?;

    assert_eq!(
        thread.status,
        ThreadStatus::Idle {
            reason: ThreadIdleReason::WaitCommand,
        },
    );
    let schedule_item = thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .find(|item| {
            matches!(
                item,
                ThreadItem::BuiltinToolCall {
                    id,
                    tool,
                    status,
                    output: Some(_),
                    ..
                } if id == "call-schedule"
                    && tool == "schedule_subscribe"
                    && *status == app_server_protocol::DynamicToolCallStatus::Completed
            )
        });
    assert!(
        schedule_item.is_some(),
        "schedule_subscribe should be replayed as a builtin tool thread item"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn startup_restores_threads_with_persisted_event_subscriptions() -> Result<()> {
    let codex_home = TempDir::new()?;
    let schedule_args = serde_json::to_string(&json!({
        "schedule": {
            "kind": "once_after",
            "delay_ms": 60_000u64
        }
    }))?;
    let responses = vec![
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_function_call("call-schedule", "schedule_subscribe", &schedule_args),
            responses::ev_completed("resp-1"),
        ]),
        create_final_assistant_message_sse_response("scheduled")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(codex_home.path(), &server.uri())?;

    let thread_id = {
        let mut first_mcp =
            McpProcess::new_with_env(codex_home.path(), &[("RUST_LOG", Some("info"))]).await?;
        timeout(DEFAULT_READ_TIMEOUT, first_mcp.initialize()).await??;

        let thread_start_id = first_mcp
            .send_thread_start_request(ThreadStartParams {
                model: Some("mock-model".to_string()),
                ..Default::default()
            })
            .await?;
        let thread_start_resp: JSONRPCResponse = timeout(
            DEFAULT_READ_TIMEOUT,
            first_mcp.read_stream_until_response_message(RequestId::Integer(thread_start_id)),
        )
        .await??;
        let ThreadStartResponse { thread, .. } = to_response(thread_start_resp)?;

        let turn_start_id = first_mcp
            .send_turn_start_request(TurnStartParams {
                thread_id: thread.id.clone(),
                input: vec![V2UserInput::Text {
                    text: "schedule a reminder".to_string(),
                    text_elements: Vec::new(),
                }],
                model: Some("mock-model".to_string()),
                ..Default::default()
            })
            .await?;
        let turn_start_resp: JSONRPCResponse = timeout(
            DEFAULT_READ_TIMEOUT,
            first_mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
        )
        .await??;
        let _: TurnStartResponse = to_response(turn_start_resp)?;

        timeout(
            DEFAULT_READ_TIMEOUT,
            first_mcp.read_stream_until_notification_message("turn/completed"),
        )
        .await??;

        thread.id
    };

    let mut second_mcp =
        McpProcess::new_with_env(codex_home.path(), &[("RUST_LOG", Some("info"))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, second_mcp.initialize()).await??;

    let loaded_list_id = second_mcp
        .send_thread_loaded_list_request(ThreadLoadedListParams::default())
        .await?;
    let loaded_list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        second_mcp.read_stream_until_response_message(RequestId::Integer(loaded_list_id)),
    )
    .await??;
    let ThreadLoadedListResponse { data, .. } = to_response(loaded_list_resp)?;
    assert!(
        data.contains(&thread_id),
        "restored thread should be loaded after app-server restart"
    );

    let list_id = second_mcp
        .send_thread_list_request(ThreadListParams {
            cursor: None,
            limit: Some(10),
            sort_key: None,
            sort_direction: None,
            model_providers: Some(Vec::new()),
            source_kinds: None,
            archived: None,
            cwd: None,
            use_state_db_only: false,
            search_term: None,
        })
        .await?;
    let list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        second_mcp.read_stream_until_response_message(RequestId::Integer(list_id)),
    )
    .await??;
    let ThreadListResponse { data, .. } = to_response(list_resp)?;
    let restored_thread = data
        .into_iter()
        .find(|thread| thread.id == thread_id)
        .expect("thread/list should include the restored thread");
    assert_eq!(
        restored_thread.status,
        ThreadStatus::Idle {
            reason: ThreadIdleReason::WaitCommand,
        },
    );

    Ok(())
}

fn create_config_toml(codex_home: &std::path::Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "untrusted"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[features]
collaboration_modes = true

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}
