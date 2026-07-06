use super::*;

#[tokio::test]
async fn thread_resume_rejects_unmaterialized_thread() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // Start a thread.
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

    // Fresh started threads are already materialized and resumable.
    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
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
async fn turn_start_updates_runtime_workspace_roots_for_loaded_thread() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let extra_root_tmp = TempDir::new()?;
    let extra_root = extra_root_tmp.path().join("extra-root");
    std::fs::create_dir_all(&extra_root)?;

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
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            runtime_workspace_roots: Some(vec![extra_root.clone(), extra_root.join(".")]),
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

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id,
            exclude_turns: true,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        runtime_workspace_roots,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(
        runtime_workspace_roots,
        vec![AbsolutePathBuf::from_absolute_path(extra_root)?]
    );

    Ok(())
}

#[tokio::test]
async fn thread_goal_get_rejects_unmaterialized_thread() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let config_path = codex_home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        &config_path,
        config.replace("personality = true\n", "personality = true\ngoals = true\n"),
    )?;

    let mut mcp = McpProcess::new_without_managed_config(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.2-codex".to_string()),
            ephemeral: Some(true),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let goal_id = mcp
        .send_raw_request(
            "thread/goal/get",
            Some(json!({
                "threadId": thread.id,
            })),
        )
        .await?;
    let goal_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(goal_id)),
    )
    .await??;
    assert!(
        goal_err
            .error
            .message
            .contains("ephemeral thread does not support goals"),
        "unexpected goal/get error: {}",
        goal_err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn thread_resume_tracks_thread_initialized_analytics() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_with_chatgpt_base_url(codex_home.path(), &server.uri(), &server.uri())?;
    mount_analytics_capture(&server, codex_home.path()).await?;

    let conversation_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    set_thread_source_on_fake_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        &conversation_id,
        "user",
    )?;

    let mut mcp = McpProcess::new_without_managed_config(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert!(
        !thread.session_id.is_empty(),
        "session id should not be empty"
    );
    assert_eq!(thread.thread_source, Some(ThreadSource::User));

    let payload = wait_for_analytics_payload(&server, DEFAULT_READ_TIMEOUT).await?;
    let event = thread_initialized_event(&payload)?;
    assert_basic_thread_initialized_event(event, &thread.id, "gpt-5.3-codex", "resumed", "user");
    assert_eq!(event["event_params"]["thread_source"], "user");
    Ok(())
}

fn set_thread_source_on_fake_rollout(
    codex_home: &std::path::Path,
    filename_ts: &str,
    thread_id: &str,
    thread_source: &str,
) -> Result<()> {
    let path = rollout_path(codex_home, filename_ts, thread_id);
    let contents = std::fs::read_to_string(&path)?;
    let mut lines = contents.lines();
    let session_meta = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("fake rollout missing session meta"))?;
    let mut session_meta: serde_json::Value = serde_json::from_str(session_meta)?;
    session_meta["payload"]["thread_source"] = serde_json::json!(thread_source);
    let remaining = lines.collect::<Vec<_>>().join("\n");
    std::fs::write(&path, format!("{session_meta}\n{remaining}\n"))?;
    Ok(())
}

#[tokio::test]
async fn thread_resume_returns_rollout_history() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let preview = "Saved user message";
    let text_elements = vec![TextElement::new(
        ByteRange { start: 0, end: 5 },
        Some("<note>".into()),
    )];
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        preview,
        text_elements
            .iter()
            .map(|elem| serde_json::to_value(elem).expect("serialize text element"))
            .collect(),
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id.clone(),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(thread.id, conversation_id);
    assert_eq!(thread.preview, preview);
    assert_eq!(thread.model_provider, "mock_provider");
    assert!(thread.path.as_ref().expect("thread path").is_absolute());
    assert_eq!(thread.cwd, test_absolute_path("/"));
    assert_eq!(thread.cli_version, "0.0.0");
    assert_eq!(thread.source, SessionSource::Cli);
    assert_eq!(thread.git_info, None);
    assert_eq!(thread.status, ThreadStatus::Complete);

    assert_eq!(
        thread.turns.len(),
        1,
        "expected rollouts to include one turn"
    );
    let turn = &thread.turns[0];
    assert_eq!(turn.status, TurnStatus::Completed);
    assert_eq!(turn.items.len(), 1, "expected user message item");
    match &turn.items[0] {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                &vec![UserInput::Text {
                    text: preview.to_string(),
                    text_elements: text_elements.clone().into_iter().map(Into::into).collect(),
                }]
            );
        }
        other => panic!("expected user message item, got {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn thread_resume_redacts_payloads_for_chatgpt_remote_clients() -> Result<()> {
    for client_name in ["codex_chatgpt_android_remote", "codex_chatgpt_ios_remote"] {
        let remote_thread = resume_redaction_fixture(Some(client_name)).await?;
        let remote_turn = remote_thread
            .turns
            .first()
            .expect("remote resume should include a turn");
        let remote_mcp_item = remote_turn
            .items
            .iter()
            .find(|item| matches!(item, ThreadItem::McpToolCall { .. }))
            .expect("remote resume should include redacted MCP item");
        let ThreadItem::McpToolCall {
            arguments,
            result,
            error,
            ..
        } = remote_mcp_item
        else {
            unreachable!("matched MCP item");
        };
        assert_eq!(arguments, &json!("[redacted]"));
        let result = result.as_ref().expect("redacted MCP result");
        assert_eq!(
            result.content,
            vec![json!({
                "type": "text",
                "text": "[redacted]",
            })]
        );
        assert_eq!(result.structured_content, None);
        assert_eq!(result.meta, None);
        assert_eq!(error, &None);
        assert!(
            !remote_turn
                .items
                .iter()
                .any(|item| matches!(item, ThreadItem::ImageGeneration { .. })),
            "remote resume should drop image generation items for {client_name}"
        );
    }

    let normal_thread = resume_redaction_fixture(Some("some_other_client")).await?;
    let normal_turn = normal_thread
        .turns
        .first()
        .expect("normal resume should include a turn");
    let normal_mcp_item = normal_turn
        .items
        .iter()
        .find(|item| matches!(item, ThreadItem::McpToolCall { .. }))
        .expect("normal resume should include MCP item");
    let ThreadItem::McpToolCall {
        arguments, result, ..
    } = normal_mcp_item
    else {
        unreachable!("matched MCP item");
    };
    assert_eq!(arguments, &json!({"secret":"argument"}));
    let result = result.as_ref().expect("normal MCP result");
    assert_eq!(
        result.content,
        vec![json!({
            "type": "text",
            "text": "secret result",
        })]
    );
    assert_eq!(
        result.structured_content,
        Some(json!({"secret":"structured"}))
    );
    assert_eq!(result.meta, Some(json!({"secret":"meta"})));
    assert!(
        normal_turn.items.iter().any(|item| matches!(
            item,
            ThreadItem::ImageGeneration {
                result,
                revised_prompt,
                ..
            } if result == "base64-image-result"
                && revised_prompt.as_deref() == Some("secret revised prompt")
        )),
        "normal resume should keep image generation items"
    );

    Ok(())
}

async fn resume_redaction_fixture(
    client_name: Option<&str>,
) -> Result<app_server_protocol::Thread> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    append_resume_redaction_history(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        &conversation_id,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    if let Some(client_name) = client_name {
        let _ = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.initialize_with_client_info(ClientInfo {
                name: client_name.to_string(),
                title: None,
                version: "0.1.0".to_string(),
            }),
        )
        .await??;
    } else {
        timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    }

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } = to_response::<ThreadResumeResponse>(resume_resp)?;
    Ok(thread)
}

fn append_resume_redaction_history(
    codex_home: &Path,
    filename_ts: &str,
    meta_rfc3339: &str,
    conversation_id: &str,
) -> Result<()> {
    let rollout_file_path = rollout_path(codex_home, filename_ts, conversation_id);
    let persisted_rollout = std::fs::read_to_string(&rollout_file_path)?;
    let appended_rollout = [
        EventMsg::McpToolCallEnd(McpToolCallEndEvent {
            call_id: "mcp-1".to_string(),
            invocation: McpInvocation {
                server: "docs".to_string(),
                tool: "lookup".to_string(),
                arguments: Some(json!({"secret":"argument"})),
            },
            mcp_app_resource_uri: Some("ui://widget/lookup.html".to_string()),
            duration: Duration::from_millis(8),
            result: Ok(CallToolResult {
                content: vec![json!({
                    "type": "text",
                    "text": "secret result",
                })],
                structured_content: Some(json!({"secret":"structured"})),
                is_error: Some(false),
                meta: Some(json!({"secret":"meta"})),
            }),
        }),
        EventMsg::ImageGenerationEnd(ImageGenerationEndEvent {
            call_id: "ig-1".to_string(),
            status: "completed".to_string(),
            revised_prompt: Some("secret revised prompt".to_string()),
            result: "base64-image-result".to_string(),
            saved_path: Some(test_absolute_path("/tmp/ig-1.png")),
        }),
    ]
    .into_iter()
    .map(|payload| {
        Ok(json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(payload)?,
        })
        .to_string())
    })
    .collect::<Result<Vec<_>>>()?
    .join("\n");
    std::fs::write(
        &rollout_file_path,
        format!("{persisted_rollout}{appended_rollout}\n"),
    )?;
    Ok(())
}

