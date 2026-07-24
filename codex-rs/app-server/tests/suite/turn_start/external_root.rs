use super::*;

async fn start_hidden_external_root_thread(mcp: &mut McpProcess, cwd: &Path) -> Result<String> {
    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            thread_provider: Some("claude_cli".to_string()),
            cwd: Some(cwd.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    assert_eq!(thread.model_provider, "claude_cli");
    assert_eq!(thread.thread_source, Some(ThreadSource::User));
    assert_eq!(thread.agent_path, None);
    assert_eq!(thread.agent_role, None);
    assert!(matches!(
        thread.lifecycle_status,
        ThreadLifecycleStatus::Active { .. }
    ));
    Ok(thread.id)
}

async fn start_external_root_mcp(codex_home: &TempDir, fake_bin: &TempDir) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_claude_cli(fake_bin.path())?;
    let test_path = prepend_path_env(fake_bin.path())?;
    let mut mcp =
        McpProcess::new_with_env(codex_home.path(), &[("PATH", Some(test_path.as_str()))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

async fn expect_external_root_native_only_error(
    mcp: &mut McpProcess,
    request_id: i64,
    method: &str,
) -> Result<()> {
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        err.error.message.contains("thread provider 'claude_cli'")
            && err.error.message.contains(method)
            && err.error.message.contains("does not support")
            && err.error.message.contains("external root threads"),
        "{}",
        err.error.message
    );
    assert!(
        !err.error.message.contains("thread not found"),
        "{}",
        err.error.message
    );
    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_accepts_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "Hello external root".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    assert!(!turn.id.is_empty(), "turn id should not be empty");
    assert_eq!(turn.status, TurnStatus::InProgress);
    assert_eq!(turn.items, Vec::<ThreadItem>::new());
    assert_eq!(turn.items_view, TurnItemsView::NotLoaded);
    assert_eq!(turn.error, None);

    Ok(())
}

#[tokio::test]
async fn external_root_rejects_native_only_active_ops() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let compact_req = mcp
        .send_thread_compact_start_request(ThreadCompactStartParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, compact_req, "thread/compact/start").await?;

    let shell_req = mcp
        .send_thread_shell_command_request(ThreadShellCommandParams {
            thread_id: thread_id.clone(),
            command: "echo should-not-run".to_string(),
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, shell_req, "thread/shellCommand").await?;

    let clean_req = mcp
        .send_thread_background_terminals_clean_request(ThreadBackgroundTerminalsCleanParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    expect_external_root_native_only_error(
        &mut mcp,
        clean_req,
        "thread/backgroundTerminals/clean",
    )
    .await?;

    let guardian_req = mcp
        .send_thread_approve_guardian_denied_action_request(
            ThreadApproveGuardianDeniedActionParams {
                thread_id: thread_id.clone(),
                event: json!({
                    "id": "guardian-denied-1",
                    "target_item_id": "guardian-target-1",
                    "turn_id": "turn-1",
                    "started_at_ms": 0,
                    "completed_at_ms": 1,
                    "status": "denied",
                    "risk_level": "high",
                    "user_authorization": "low",
                    "rationale": "Would run a command on an external root thread.",
                    "decision_source": "agent",
                    "action": {
                        "type": "command",
                        "source": "shell",
                        "command": "echo should-not-run",
                        "cwd": codex_home.path().to_string_lossy(),
                    },
                }),
            },
        )
        .await?;
    expect_external_root_native_only_error(
        &mut mcp,
        guardian_req,
        "thread/approveGuardianDeniedAction",
    )
    .await?;

    let rollback_req = mcp
        .send_thread_rollback_request(ThreadRollbackParams {
            thread_id: thread_id.clone(),
            num_turns: 1,
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, rollback_req, "thread/rollback").await?;

    let second_rollback_req = mcp
        .send_thread_rollback_request(ThreadRollbackParams {
            thread_id,
            num_turns: 1,
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, second_rollback_req, "thread/rollback")
        .await?;

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_rejects_non_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Image {
                url: "https://example.com/image.png".to_string(),
            }],
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        err.error
            .message
            .contains("external root turn/start only supports text input"),
        "{}",
        err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_rejects_empty_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![
                V2UserInput::Text {
                    text: String::new(),
                    text_elements: Vec::new(),
                },
                V2UserInput::Text {
                    text: String::new(),
                    text_elements: Vec::new(),
                },
            ],
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(
        err.error.message,
        "external root turn/start input text must not be empty"
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_counts_text_join_separator_in_input_limit() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![
                V2UserInput::Text {
                    text: "x".repeat(MAX_USER_INPUT_TEXT_CHARS),
                    text_elements: Vec::new(),
                },
                V2UserInput::Text {
                    text: String::new(),
                    text_elements: Vec::new(),
                },
            ],
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_PARAMS_ERROR_CODE);
    assert_eq!(
        err.error.message,
        format!("Input exceeds the maximum length of {MAX_USER_INPUT_TEXT_CHARS} characters.")
    );
    let data = err.error.data.expect("expected structured error data");
    assert_eq!(data["input_error_code"], INPUT_TOO_LARGE_ERROR_CODE);
    assert_eq!(data["max_chars"], MAX_USER_INPUT_TEXT_CHARS);
    assert_eq!(data["actual_chars"], MAX_USER_INPUT_TEXT_CHARS + 1);

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_rejects_text_elements() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "Hello external root".to_string(),
                text_elements: vec![TextElement::new(
                    ByteRange { start: 0, end: 5 },
                    Some("<note>".to_string()),
                )],
            }],
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        err.error
            .message
            .contains("text elements are not supported"),
        "{}",
        err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_rejects_native_only_params() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "Hello external root".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model-override".to_string()),
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        err.error
            .message
            .contains("external root turn/start does not support model"),
        "{}",
        err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_rejects_second_input_while_active() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let first_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "First external root input".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_req)),
    )
    .await??;

    let second_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "Second external root input".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(second_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        err.error
            .message
            .contains("external root thread already has an active turn"),
        "{}",
        err.error.message
    );

    Ok(())
}
