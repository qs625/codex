use super::*;
use app_server_protocol::ItemCompletedNotification;
use app_test_support::create_exec_command_sse_response;
use app_test_support::create_mock_responses_server_sequence;
use codex_features::Feature;
use std::collections::BTreeMap;

fn create_config_toml_with_sandbox(
    codex_home: &std::path::Path,
    server_uri: &str,
    approval_policy: &str,
    features: &BTreeMap<Feature, bool>,
    sandbox_mode: &str,
) -> std::io::Result<()> {
    let mut feature_lines = String::from("personality = true\n");
    for (feature, enabled) in features {
        let name = feature.key();
        feature_lines.push_str(&format!("{name} = {enabled}\n"));
    }

    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "gpt-5.3-codex"
approval_policy = "{approval_policy}"
sandbox_mode = "{sandbox_mode}"

model_provider = "mock_provider"

[features]
{feature_lines}

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

#[tokio::test]
async fn thread_resume_replays_pending_command_execution_request_approval() -> Result<()> {
    let responses = vec![
        create_final_assistant_message_sse_response("seeded")?,
        create_shell_command_sse_response(
            vec![
                "python3".to_string(),
                "-c".to_string(),
                "print(42)".to_string(),
            ],
            /*workdir*/ None,
            Some(5000),
            "call-1",
        )?,
        create_final_assistant_message_sse_response("done")?,
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut primary = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, primary.initialize()).await??;

    let start_id = primary
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.4".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let seed_turn_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "seed history".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(seed_turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    primary.clear_message_buffer();

    let running_turn_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "run command".to_string(),
                text_elements: Vec::new(),
            }],
            approval_policy: Some(AskForApproval::UnlessTrusted),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(running_turn_id)),
    )
    .await??;

    let original_request = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::CommandExecutionRequestApproval { .. } = &original_request else {
        panic!("expected CommandExecutionRequestApproval request, got {original_request:?}");
    };

    let resume_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed_thread,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed_thread.id, thread.id);
    assert!(
        resumed_thread
            .turns
            .iter()
            .any(|turn| matches!(turn.status, TurnStatus::InProgress))
    );

    let replayed_request = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_request_message(),
    )
    .await??;
    pretty_assertions::assert_eq!(replayed_request, original_request);

    let ServerRequest::CommandExecutionRequestApproval { request_id, .. } = replayed_request else {
        panic!("expected CommandExecutionRequestApproval request");
    };
    primary
        .send_response(
            request_id,
            serde_json::to_value(CommandExecutionRequestApprovalResponse {
                decision: CommandExecutionApprovalDecision::Accept,
            })?,
        )
        .await?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    wait_for_responses_request_count(&server, /*expected_count*/ 3).await?;

    Ok(())
}

#[tokio::test]
async fn thread_resume_replays_pending_file_change_request_approval() -> Result<()> {
    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let patch = r#"*** Begin Patch
*** Add File: README.md
+new line
*** End Patch
"#;
    let responses = vec![
        create_final_assistant_message_sse_response("seeded")?,
        create_apply_patch_sse_response(patch, "patch-call")?,
        create_final_assistant_message_sse_response("done")?,
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;
    create_config_toml(&codex_home, &server.uri())?;

    let mut primary = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, primary.initialize()).await??;

    let start_id = primary
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.4".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let seed_turn_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "seed history".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(seed_turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    primary.clear_message_buffer();

    let running_turn_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "apply patch".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            approval_policy: Some(AskForApproval::UnlessTrusted),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(running_turn_id)),
    )
    .await??;

    let original_started = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let notification = primary
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(notification.params.clone().expect("item/started params"))?;
            if let ThreadItem::FileChange { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let expected_readme_path = workspace.join("README.md");
    let expected_file_change = ThreadItem::FileChange {
        id: "patch-call".to_string(),
        changes: vec![app_server_protocol::FileUpdateChange {
            path: expected_readme_path.to_string_lossy().into_owned(),
            kind: PatchChangeKind::Add,
            diff: "new line\n".to_string(),
        }],
        status: PatchApplyStatus::InProgress,
    };
    assert_eq!(original_started, expected_file_change);

    let original_request = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::FileChangeRequestApproval { .. } = &original_request else {
        panic!("expected FileChangeRequestApproval request, got {original_request:?}");
    };
    primary.clear_message_buffer();

    let resume_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed_thread,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed_thread.id, thread.id);
    assert!(
        resumed_thread
            .turns
            .iter()
            .any(|turn| matches!(turn.status, TurnStatus::InProgress))
    );

    let replayed_request = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_request_message(),
    )
    .await??;
    assert_eq!(replayed_request, original_request);

    let ServerRequest::FileChangeRequestApproval { request_id, .. } = replayed_request else {
        panic!("expected FileChangeRequestApproval request");
    };
    primary
        .send_response(
            request_id,
            serde_json::to_value(FileChangeRequestApprovalResponse {
                decision: FileChangeApprovalDecision::Accept,
            })?,
        )
        .await?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    wait_for_responses_request_count(&server, /*expected_count*/ 3).await?;

    Ok(())
}

#[tokio::test]
async fn thread_resume_with_overrides_defers_updated_at_until_turn_start() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let RestartedThreadFixture {
        mut mcp,
        thread_id,
        rollout_file_path,
        updated_at,
    } = start_materialized_thread_and_restart(codex_home.path(), "materialize").await?;
    let expected_updated_at_rfc3339 = "2025-01-07T00:00:00Z";
    set_rollout_mtime(rollout_file_path.as_path(), expected_updated_at_rfc3339)?;
    let before_modified = std::fs::metadata(&rollout_file_path)?.modified()?;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id,
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed_thread,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(resumed_thread.updated_at, updated_at);
    assert_eq!(resumed_thread.status, ThreadStatus::Complete);

    let after_resume_modified = std::fs::metadata(&rollout_file_path)?.modified()?;
    assert_eq!(after_resume_modified, before_modified);

    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: resumed_thread.id,
            input: vec![UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let after_turn_modified = std::fs::metadata(&rollout_file_path)?.modified()?;
    assert!(after_turn_modified > before_modified);

    Ok(())
}

#[tokio::test]
async fn thread_resume_fails_when_required_mcp_server_fails_to_initialize() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    let rollout = setup_rollout_fixture(codex_home.path(), &server.uri())?;
    create_config_toml_with_required_broken_mcp(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: rollout.conversation_id,
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(resume_id)),
    )
    .await??;

    assert!(
        err.error
            .message
            .contains("required MCP servers failed to initialize"),
        "unexpected error message: {}",
        err.error.message
    );
    assert!(
        err.error.message.contains("required_broken"),
        "unexpected error message: {}",
        err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn thread_resume_surfaces_cloud_requirements_load_errors() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/wham/config/requirements"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("content-type", "text/html")
                .set_body_string("<html>nope</html>"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "code": "refresh_token_invalidated" }
        })))
        .mount(&server)
        .await;

    let codex_home = TempDir::new()?;
    let model_server = create_mock_responses_server_repeating_assistant("Done").await;
    let chatgpt_base_url = format!("{}/backend-api", server.uri());
    create_config_toml_with_chatgpt_base_url(
        codex_home.path(),
        &model_server.uri(),
        &chatgpt_base_url,
    )?;
    write_chatgpt_auth(
        codex_home.path(),
        ChatGptAuthFixture::new("chatgpt-token")
            .refresh_token("stale-refresh-token")
            .plan_type("business")
            .chatgpt_user_id("user-123")
            .chatgpt_account_id("account-123")
            .account_id("account-123"),
        AuthCredentialsStoreMode::File,
    )?;
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Vec::new(),
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let refresh_token_url = format!("{}/oauth/token", server.uri());
    let mut mcp = McpProcess::new_with_env(
        codex_home.path(),
        &[
            ("OPENAI_API_KEY", None),
            (
                REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR,
                Some(refresh_token_url.as_str()),
            ),
        ],
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id,
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(resume_id)),
    )
    .await??;

    assert!(
        err.error.message.contains("failed to load configuration"),
        "unexpected error message: {}",
        err.error.message
    );
    assert_eq!(
        err.error.data,
        Some(json!({
            "reason": "cloudRequirements",
            "errorCode": "Auth",
            "action": "relogin",
            "statusCode": 401,
            "detail": "Your access token could not be refreshed because your refresh token was revoked. Please log out and sign in again.",
        }))
    );

    Ok(())
}

#[tokio::test]
async fn thread_resume_uses_path_over_invalid_thread_id() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.4".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "materialize".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let thread_path = thread.path.clone().expect("thread path");
    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: "not-a-valid-thread-id".to_string(),
            path: Some(thread_path.to_path_buf()),
            ..Default::default()
        })
        .await?;

    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed.id, thread.id);

    Ok(())
}

#[tokio::test]
async fn thread_resume_can_load_source_by_external_path() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    let external_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let thread_id = create_fake_rollout(
        external_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "external path history",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let thread_path = rollout_path(external_home.path(), "2025-01-05T12-00-00", &thread_id);

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: "not-a-valid-thread-id".to_string(),
            path: Some(thread_path.clone()),
            ..Default::default()
        })
        .await?;

    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed.id, thread_id);
    let resumed_path = resumed.path.as_ref().expect("resumed thread path");
    assert_eq!(
        normalized_existing_path(resumed_path)?,
        normalized_existing_path(&thread_path)?
    );
    assert_eq!(resumed.preview, "external path history");
    assert_eq!(resumed.status, ThreadStatus::Complete);

    Ok(())
}

#[tokio::test]
async fn thread_resume_supports_history_and_overrides() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let RestartedThreadFixture {
        mut mcp, thread_id, ..
    } = start_materialized_thread_and_restart(codex_home.path(), "seed history").await?;

    let history_text = "Hello from history";
    let history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: history_text.to_string(),
        }],
        phase: None,
    }];

    // Resume with explicit history and override the model.
    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id,
            history: Some(history),
            model: Some("mock-model".to_string()),
            model_provider: Some("mock_provider".to_string()),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed,
        model_provider,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert!(!resumed.id.is_empty());
    assert_eq!(model_provider, "mock_provider");
    assert_eq!(resumed.preview, history_text);
    assert_eq!(resumed.status, ThreadStatus::Complete);

    Ok(())
}

#[tokio::test]
#[cfg_attr(windows, ignore = "process id reporting differs on Windows")]
async fn thread_read_after_restart_keeps_unified_exec_command_execution_items() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let responses = vec![
        create_exec_command_sse_response("uexec-reload-1")?,
        create_final_assistant_message_sse_response("done")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    let codex_home = TempDir::new()?;
    create_config_toml_with_sandbox(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::UnifiedExec, true)]),
        "danger-full-access",
    )?;

    let mut first_mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, first_mcp.initialize()).await??;

    let start_id = first_mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_id = first_mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "run a command".to_string(),
                text_elements: Vec::new(),
            }],
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::DangerFullAccess),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;

    timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let notif = first_mcp
                .read_stream_until_notification_message("item/completed")
                .await?;
            let completed: ItemCompletedNotification = serde_json::from_value(
                notif
                    .params
                    .clone()
                    .expect("item/completed should include params"),
            )?;
            if let ThreadItem::CommandExecution { id, .. } = &completed.item
                && id == "uexec-reload-1"
            {
                return Ok::<(), anyhow::Error>(());
            }
        }
    })
    .await??;

    timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    drop(first_mcp);

    let mut second_mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, second_mcp.initialize()).await??;

    let read_id = second_mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        second_mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    let command_items = thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter_map(|item| match item {
            ThreadItem::CommandExecution {
                id,
                source,
                status,
                exit_code,
                ..
            } if id == "uexec-reload-1" => Some((
                id.clone(),
                source.clone(),
                status.clone(),
                *exit_code,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        command_items,
        vec![(
            "uexec-reload-1".to_string(),
            app_server_protocol::CommandExecutionSource::UnifiedExecStartup,
            app_server_protocol::CommandExecutionStatus::Completed,
            Some(0),
        )]
    );

    Ok(())
}

struct RestartedThreadFixture {
    mcp: McpProcess,
    thread_id: String,
    rollout_file_path: PathBuf,
    updated_at: i64,
}

async fn start_materialized_thread_and_restart(
    codex_home: &Path,
    seed_text: &str,
) -> Result<RestartedThreadFixture> {
    let mut first_mcp = McpProcess::new(codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, first_mcp.initialize()).await??;

    let start_id = first_mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.4".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let materialize_turn_id = first_mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: seed_text.to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_response_message(RequestId::Integer(materialize_turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let read_id = first_mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        first_mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    let thread_id = thread.id;
    let rollout_file_path = thread
        .path
        .ok_or_else(|| anyhow::anyhow!("thread path missing from thread/start response"))?;
    let updated_at = thread.updated_at;

    drop(first_mcp);

    let mut second_mcp = McpProcess::new(codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, second_mcp.initialize()).await??;

    Ok(RestartedThreadFixture {
        mcp: second_mcp,
        thread_id,
        rollout_file_path: rollout_file_path.to_path_buf(),
        updated_at,
    })
}

#[tokio::test]
async fn thread_resume_accepts_personality_override() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let first_body = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]);
    let second_body = responses::sse(vec![
        responses::ev_response_created("resp-2"),
        responses::ev_assistant_message("msg-2", "Done"),
        responses::ev_completed("resp-2"),
    ]);
    let response_mock = responses::mount_sse_sequence(&server, vec![first_body, second_body]).await;

    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut primary = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, primary.initialize()).await??;

    let start_id = primary
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.3-codex".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let materialize_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "seed history".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(materialize_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let mut secondary = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, secondary.initialize()).await??;

    let resume_id = secondary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id,
            model: Some("gpt-5.3-codex".to_string()),
            personality: Some(Personality::Friendly),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        secondary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let resume: ThreadResumeResponse = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resume.thread.status, ThreadStatus::Complete);

    let turn_id = secondary
        .send_turn_start_request(TurnStartParams {
            thread_id: resume.thread.id,
            input: vec![UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        secondary.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;

    timeout(
        DEFAULT_READ_TIMEOUT,
        secondary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let requests = response_mock.requests();
    let request = requests
        .last()
        .expect("expected request for resumed thread turn");
    let developer_texts = request.message_input_texts("developer");
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<personality_spec>")),
        "expected a personality update message in developer input, got {developer_texts:?}"
    );
    let instructions_text = request.instructions_text();
    assert!(
        instructions_text.contains(CODEX_5_2_INSTRUCTIONS_TEMPLATE_DEFAULT),
        "expected default base instructions from history, got {instructions_text:?}"
    );

    Ok(())
}
