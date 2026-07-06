use super::*;

#[tokio::test]
async fn turn_start_exec_approval_toggle_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().to_path_buf();

    // Mock server: first turn requests a shell call (elicitation), then completes.
    // Second turn same, but we'll set approval_policy=never to avoid elicitation.
    let responses = vec![
        create_shell_command_sse_response(
            vec![
                "python3".to_string(),
                "-c".to_string(),
                "print(42)".to_string(),
            ],
            /*workdir*/ None,
            Some(5000),
            "call1",
        )?,
        create_final_assistant_message_sse_response("done 1")?,
        create_shell_command_sse_response(
            vec![
                "python3".to_string(),
                "-c".to_string(),
                "print(42)".to_string(),
            ],
            /*workdir*/ None,
            Some(5000),
            "call2",
        )?,
        create_final_assistant_message_sse_response("done 2")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    // Default approval is untrusted to force elicitation on first turn.
    create_config_toml(
        codex_home.as_path(),
        &server.uri(),
        "untrusted",
        &BTreeMap::default(),
    )?;

    let mut mcp = McpProcess::new(codex_home.as_path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // thread/start
    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    // turn/start — expect CommandExecutionRequestApproval request from server
    let first_turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "run python".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    // Acknowledge RPC
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_turn_id)),
    )
    .await??;

    // Receive elicitation
    let server_req = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::CommandExecutionRequestApproval { request_id, params } = server_req else {
        panic!("expected CommandExecutionRequestApproval request");
    };
    assert_eq!(params.item_id, "call1");
    let resolved_request_id = request_id.clone();

    // Approve and wait for task completion
    mcp.send_response(
        request_id,
        serde_json::to_value(CommandExecutionRequestApprovalResponse {
            decision: CommandExecutionApprovalDecision::Accept,
        })?,
    )
    .await?;
    let mut saw_resolved = false;
    loop {
        let message = timeout(DEFAULT_READ_TIMEOUT, mcp.read_next_message()).await??;
        let JSONRPCMessage::Notification(notification) = message else {
            continue;
        };
        match notification.method.as_str() {
            "serverRequest/resolved" => {
                let resolved: ServerRequestResolvedNotification = serde_json::from_value(
                    notification
                        .params
                        .clone()
                        .expect("serverRequest/resolved params"),
                )?;
                assert_eq!(resolved.thread_id, thread.id);
                assert_eq!(resolved.request_id, resolved_request_id);
                saw_resolved = true;
            }
            "turn/completed" => {
                assert!(saw_resolved, "serverRequest/resolved should arrive first");
                break;
            }
            _ => {}
        }
    }

    // Second turn with approval_policy=never should not elicit approval
    let second_turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "run python again".to_string(),
                text_elements: Vec::new(),
            }],
            approval_policy: Some(app_server_protocol::AskForApproval::Never),
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::DangerFullAccess),
            model: Some("mock-model".to_string()),
            effort: Some(ReasoningEffort::Medium),
            summary: Some(ReasoningSummary::Auto),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_turn_id)),
    )
    .await??;

    // Ensure we do NOT receive a CommandExecutionRequestApproval request before task completes
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_exec_approval_decline_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().to_path_buf();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let responses = vec![
        create_shell_command_sse_response(
            vec![
                "python3".to_string(),
                "-c".to_string(),
                "print(42)".to_string(),
            ],
            /*workdir*/ None,
            Some(5000),
            "call-decline",
        )?,
        create_final_assistant_message_sse_response("done")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(
        codex_home.as_path(),
        &server.uri(),
        "untrusted",
        &BTreeMap::default(),
    )?;

    let mut mcp = McpProcess::new(codex_home.as_path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
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
            input: vec![V2UserInput::Text {
                text: "run python".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let started_command_execution = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.clone().expect("item/started params"))?;
            if let ThreadItem::CommandExecution { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::CommandExecution { id, status, .. } = started_command_execution else {
        unreachable!("loop ensures we break on command execution items");
    };
    assert_eq!(id, "call-decline");
    assert_eq!(status, CommandExecutionStatus::InProgress);

    let server_req = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::CommandExecutionRequestApproval { request_id, params } = server_req else {
        panic!("expected CommandExecutionRequestApproval request")
    };
    assert_eq!(params.item_id, "call-decline");
    assert_eq!(params.thread_id, thread.id);
    assert_eq!(params.turn_id, turn.id);

    mcp.send_response(
        request_id,
        serde_json::to_value(CommandExecutionRequestApprovalResponse {
            decision: CommandExecutionApprovalDecision::Decline,
        })?,
    )
    .await?;

    let completed_command_execution = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let completed_notif = mcp
                .read_stream_until_notification_message("item/completed")
                .await?;
            let completed: ItemCompletedNotification = serde_json::from_value(
                completed_notif
                    .params
                    .clone()
                    .expect("item/completed params"),
            )?;
            if let ThreadItem::CommandExecution { .. } = completed.item {
                return Ok::<ThreadItem, anyhow::Error>(completed.item);
            }
        }
    })
    .await??;
    let ThreadItem::CommandExecution {
        id,
        status,
        exit_code,
        aggregated_output,
        ..
    } = completed_command_execution
    else {
        unreachable!("loop ensures we break on command execution items");
    };
    assert_eq!(id, "call-decline");
    assert_eq!(status, CommandExecutionStatus::Declined);
    assert!(exit_code.is_none());
    assert!(aggregated_output.is_none());

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_updates_sandbox_and_cwd_between_turns_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace_root = tmp.path().join("workspace");
    std::fs::create_dir(&workspace_root)?;
    let first_cwd = workspace_root.join("turn1");
    let second_cwd = workspace_root.join("turn2");
    std::fs::create_dir(&first_cwd)?;
    std::fs::create_dir(&second_cwd)?;

    let responses = vec![
        create_shell_command_sse_response(
            vec!["echo".to_string(), "first".to_string(), "turn".to_string()],
            /*workdir*/ None,
            Some(5000),
            "call-first",
        )?,
        create_final_assistant_message_sse_response("done first")?,
        create_shell_command_sse_response(
            vec!["echo".to_string(), "second".to_string(), "turn".to_string()],
            /*workdir*/ None,
            Some(5000),
            "call-second",
        )?,
        create_final_assistant_message_sse_response("done second")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(
        &codex_home,
        &server.uri(),
        "untrusted",
        &BTreeMap::default(),
    )?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // thread/start
    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    // first turn with workspace-write sandbox and first_cwd
    let first_turn = mcp
        .send_turn_start_request(TurnStartParams {
            environments: None,
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "first turn".to_string(),
                text_elements: Vec::new(),
            }],
            responsesapi_client_metadata: None,
            cwd: Some(first_cwd.clone()),
            runtime_workspace_roots: None,
            approval_policy: Some(app_server_protocol::AskForApproval::Never),
            approvals_reviewer: None,
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![first_cwd.try_into()?],
                network_access: false,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            }),
            permissions: None,
            model: Some("mock-model".to_string()),
            model_provider: None,
            effort: Some(ReasoningEffort::Medium),
            summary: Some(ReasoningSummary::Auto),
            service_tier: None,
            personality: None,
            output_schema: None,
            collaboration_mode: None,
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_turn)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    mcp.clear_message_buffer();

    // second turn with workspace-write and second_cwd, ensure exec begins in second_cwd
    let second_turn = mcp
        .send_turn_start_request(TurnStartParams {
            environments: None,
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "second turn".to_string(),
                text_elements: Vec::new(),
            }],
            responsesapi_client_metadata: None,
            cwd: Some(second_cwd.clone()),
            runtime_workspace_roots: None,
            approval_policy: Some(app_server_protocol::AskForApproval::Never),
            approvals_reviewer: None,
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::DangerFullAccess),
            permissions: None,
            model: Some("mock-model".to_string()),
            model_provider: None,
            effort: Some(ReasoningEffort::Medium),
            summary: Some(ReasoningSummary::Auto),
            service_tier: None,
            personality: None,
            output_schema: None,
            collaboration_mode: None,
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_turn)),
    )
    .await??;

    let command_exec_item = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let item_started_notification = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let params = item_started_notification
                .params
                .clone()
                .expect("item/started params");
            let item_started: ItemStartedNotification =
                serde_json::from_value(params).expect("deserialize item/started notification");
            if matches!(item_started.item, ThreadItem::CommandExecution { .. }) {
                return Ok::<ThreadItem, anyhow::Error>(item_started.item);
            }
        }
    })
    .await??;
    let ThreadItem::CommandExecution {
        cwd,
        command,
        status,
        ..
    } = command_exec_item
    else {
        unreachable!("loop ensures we break on command execution items");
    };
    assert_eq!(cwd.as_path(), second_cwd.as_path());
    let expected_command = format_with_current_shell_display("echo second turn");
    assert_eq!(command, expected_command);
    assert_eq!(status, CommandExecutionStatus::InProgress);

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn turn_start_permission_profile_rebinds_runtime_workspace_roots_between_turns() -> Result<()>
{
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let old_root = tmp.path().join("old-root");
    let new_root = tmp.path().join("new-root");
    std::fs::create_dir(&old_root)?;
    std::fs::create_dir(&new_root)?;
    let old_root_text = old_root.to_string_lossy().into_owned();
    let new_root_text = new_root.to_string_lossy().into_owned();

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_assistant_message("msg-1", "done first"),
                responses::ev_completed("resp-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-2"),
                responses::ev_assistant_message("msg-2", "done second"),
                responses::ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let server_uri = server.uri();
    std::fs::write(
        codex_home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
default_permissions = "dev"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[permissions.dev.filesystem.":workspace_roots"]
"." = "write"
"#
        ),
    )?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let first_turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "select dev profile".to_string(),
                text_elements: Vec::new(),
            }],
            runtime_workspace_roots: Some(vec![old_root]),
            permissions: Some("dev".to_string().into()),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let second_turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "write in new root".to_string(),
                text_elements: Vec::new(),
            }],
            runtime_workspace_roots: Some(vec![new_root]),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_turn_id)),
    )
    .await??;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2, "expected two Responses API requests");
    let latest_permissions_instructions =
        |request: &core_test_support::responses::ResponsesRequest| {
            request
                .message_input_texts("developer")
                .into_iter()
                .rev()
                .find(|text| text.contains("<permissions instructions>"))
                .expect("permissions instructions")
        };
    let first_permissions = latest_permissions_instructions(&requests[0]);
    assert!(first_permissions.contains(&old_root_text));
    assert!(
        !first_permissions.contains(&new_root_text),
        "first turn should materialize the initial runtime workspace root"
    );

    let second_permissions = latest_permissions_instructions(&requests[1]);
    assert!(second_permissions.contains(&new_root_text));
    assert!(
        !second_permissions.contains(&old_root_text),
        "second turn should rebind :workspace_roots to the updated runtime workspace root"
    );

    Ok(())
}
