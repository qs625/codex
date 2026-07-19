use super::*;

#[tokio::test]
async fn thread_resume_prefers_persisted_git_metadata_for_local_threads() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    let config_toml = codex_home.path().join("config.toml");
    std::fs::write(
        &config_toml,
        format!(
            r#"
model = "gpt-5.3-codex"
approval_policy = "never"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[features]
personality = true
sqlite = true

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#,
            server.uri()
        ),
    )?;

    let repo_path = codex_home.path().join("repo");
    std::fs::create_dir_all(&repo_path)?;
    assert!(
        Command::new("git")
            .args(["init"])
            .arg(&repo_path)
            .status()?
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_path)
            .args(["checkout", "-B", "master"])
            .status()?
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_path)
            .args(["config", "user.name", "Test User"])
            .status()?
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_path)
            .args(["config", "user.email", "test@example.com"])
            .status()?
            .success()
    );
    std::fs::write(repo_path.join("README.md"), "test\n")?;
    assert!(
        Command::new("git")
            .current_dir(&repo_path)
            .args(["add", "README.md"])
            .status()?
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&repo_path)
            .args(["commit", "-m", "initial"])
            .status()?
            .success()
    );
    let head_branch = Command::new("git")
        .current_dir(&repo_path)
        .args(["branch", "--show-current"])
        .output()?;
    assert_eq!(
        String::from_utf8(head_branch.stdout)?.trim(),
        "master",
        "test repo should stay on master to verify resume ignores live HEAD"
    );

    let thread_id = Uuid::new_v4().to_string();
    let conversation_id = ThreadId::from_string(&thread_id)?;
    let rollout_path = rollout_path(codex_home.path(), "2025-01-05T12-00-00", &thread_id);
    let rollout_dir = rollout_path.parent().expect("rollout parent directory");
    std::fs::create_dir_all(rollout_dir)?;
    let session_meta = SessionMeta {
        id: conversation_id,
        forked_from_id: None,
        timestamp: "2025-01-05T12:00:00Z".to_string(),
        cwd: repo_path.clone(),
        originator: "codex".to_string(),
        cli_version: "0.0.0".to_string(),
        source: RolloutSessionSource::Cli,
        thread_source: None,
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
        model_provider: Some("mock_provider".to_string()),
        base_instructions: None,
        dynamic_tools: None,
        memory_mode: None,
        subscriptions: None,
    };
    std::fs::write(
        &rollout_path,
        [
            json!({
                "timestamp": "2025-01-05T12:00:00Z",
                "type": "session_meta",
                "payload": serde_json::to_value(SessionMetaLine {
                    meta: session_meta,
                    git: None,
                })?,
            })
            .to_string(),
            json!({
                "timestamp": "2025-01-05T12:00:00Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{"type": "input_text", "text": "Saved user message"}]
                }
            })
            .to_string(),
            json!({
                "timestamp": "2025-01-05T12:00:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "Saved user message",
                    "kind": "plain"
                }
            })
            .to_string(),
        ]
        .join("\n")
            + "\n",
    )?;
    let state_db =
        StateRuntime::init(codex_home.path().to_path_buf(), "mock_provider".into()).await?;
    state_db
        .mark_backfill_complete(/*last_watermark*/ None)
        .await?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let update_id = mcp
        .send_thread_metadata_update_request(ThreadMetadataUpdateParams {
            thread_id: thread_id.clone(),
            git_info: Some(ThreadMetadataGitInfoUpdateParams {
                sha: None,
                branch: Some(Some("feature/pr-branch".to_string())),
                origin_url: None,
            }),
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(update_id)),
    )
    .await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(
        thread
            .git_info
            .as_ref()
            .and_then(|git| git.branch.as_deref()),
        Some("feature/pr-branch")
    );

    Ok(())
}

#[tokio::test]
async fn thread_resume_and_read_interrupt_incomplete_rollout_turn_when_thread_is_idle() -> Result<()>
{
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Vec::new(),
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let rollout_file_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let persisted_rollout = std::fs::read_to_string(&rollout_file_path)?;
    let turn_id = "incomplete-turn";
    let appended_rollout = [
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            }))?,
        })
        .to_string(),
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::AgentMessage(AgentMessageEvent {
                message: "Still running".to_string(),
                phase: None,
                memory_citation: None,
            }))?,
        })
        .to_string(),
    ]
    .join("\n");
    std::fs::write(
        &rollout_file_path,
        format!("{persisted_rollout}{appended_rollout}\n"),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
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

    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::completed(None));
    assert_eq!(thread.turns.len(), 2);
    assert_eq!(thread.turns[0].status, TurnStatus::Completed);
    assert_eq!(thread.turns[1].id, turn_id);
    assert_eq!(thread.turns[1].status, TurnStatus::Interrupted);

    let second_resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
            ..Default::default()
        })
        .await?;
    let second_resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed_again,
        ..
    } = to_response::<ThreadResumeResponse>(second_resume_resp)?;

    assert_eq!(resumed_again.lifecycle_status, ThreadLifecycleStatus::completed(None));
    assert_eq!(resumed_again.turns.len(), 2);
    assert_eq!(resumed_again.turns[1].id, turn_id);
    assert_eq!(resumed_again.turns[1].status, TurnStatus::Interrupted);

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: resumed_again.id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse {
        thread: read_thread,
        ..
    } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(read_thread.lifecycle_status, ThreadLifecycleStatus::completed(None));
    assert_eq!(read_thread.turns.len(), 2);
    assert_eq!(read_thread.turns[1].id, turn_id);
    assert_eq!(read_thread.turns[1].status, TurnStatus::Interrupted);

    Ok(())
}

#[tokio::test]
async fn thread_resume_without_overrides_does_not_change_updated_at_or_mtime() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    let rollout = setup_rollout_fixture(codex_home.path(), &server.uri())?;
    let thread_id = rollout.conversation_id.clone();

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse {
        thread: before_resume,
        ..
    } = to_response::<ThreadReadResponse>(read_resp)?;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread_id.clone(),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(thread.updated_at, before_resume.updated_at);
    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::completed(None));

    let after_modified = std::fs::metadata(&rollout.rollout_file_path)?.modified()?;
    assert_eq!(after_modified, rollout.before_modified);

    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
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

    let after_turn_modified = std::fs::metadata(&rollout.rollout_file_path)?.modified()?;
    assert!(after_turn_modified > rollout.before_modified);

    Ok(())
}

#[tokio::test]
async fn thread_resume_keeps_in_flight_turn_streaming() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
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

    let mut secondary = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, secondary.initialize()).await??;

    let turn_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "respond with docs".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/started"),
    )
    .await??;

    let resume_id = secondary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        secondary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed_thread,
        ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_ne!(resumed_thread.lifecycle_status, ThreadLifecycleStatus::NotLoaded);

    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn thread_resume_rejects_history_when_thread_is_running() -> Result<()> {
    let server = responses::start_mock_server().await;
    let first_body = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]);
    let second_response = responses::sse_response(responses::sse(vec![
        responses::ev_response_created("resp-2"),
        responses::ev_assistant_message("msg-2", "Done"),
        responses::ev_completed("resp-2"),
    ]))
    .set_delay(std::time::Duration::from_millis(500));
    let _first_response_mock = responses::mount_sse_once(&server, first_body).await;
    let _second_response_mock = responses::mount_response_once(&server, second_response).await;
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

    let thread_id = thread.id.clone();
    let running_turn_request_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "keep running".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let running_turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(running_turn_request_id)),
    )
    .await??;
    let TurnStartResponse { turn: running_turn } =
        to_response::<TurnStartResponse>(running_turn_resp)?;
    assert_eq!(running_turn.items_view, TurnItemsView::NotLoaded);
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/started"),
    )
    .await??;

    let resume_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread_id.clone(),
            history: Some(vec![ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "history override".to_string(),
                }],
                phase: None,
            }]),
            ..Default::default()
        })
        .await?;
    let resume_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_error_message(RequestId::Integer(resume_id)),
    )
    .await??;
    assert!(
        resume_err.error.message.contains("cannot resume thread")
            && resume_err.error.message.contains("with history")
            && resume_err.error.message.contains("running"),
        "unexpected resume error: {}",
        resume_err.error.message
    );

    primary
        .interrupt_turn_and_wait_for_aborted(thread_id, running_turn.id, DEFAULT_READ_TIMEOUT)
        .await?;

    Ok(())
}

#[tokio::test]
async fn thread_resume_uses_path_over_thread_id_when_thread_is_running() -> Result<()> {
    let server = responses::start_mock_server().await;
    let first_body = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]);
    let second_response = responses::sse_response(responses::sse(vec![
        responses::ev_response_created("resp-2"),
        responses::ev_assistant_message("msg-2", "Done"),
        responses::ev_completed("resp-2"),
    ]))
    .set_delay(std::time::Duration::from_millis(500));
    let _first_response_mock = responses::mount_sse_once(&server, first_body).await;
    let _second_response_mock = responses::mount_response_once(&server, second_response).await;
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

    let thread_id = thread.id.clone();
    let running_turn_request_id = primary
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
                text: "keep running".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let running_turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(running_turn_request_id)),
    )
    .await??;
    let TurnStartResponse { turn: running_turn } =
        to_response::<TurnStartResponse>(running_turn_resp)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/started"),
    )
    .await??;

    let other_thread_id = ThreadId::new().to_string();
    let stale_path = rollout_path(codex_home.path(), "2025-01-01T00-00-00", &thread_id);
    std::fs::create_dir_all(stale_path.parent().expect("stale path parent"))?;
    let thread_uuid = Uuid::parse_str(&thread_id)?;
    let mut stale_file = std::fs::File::create(&stale_path)?;
    let stale_meta = json!({
        "timestamp": "2025-01-01T00:00:00Z",
        "type": "session_meta",
        "payload": {
            "id": thread_uuid,
            "timestamp": "2025-01-01T00:00:00Z",
            "cwd": codex_home.path(),
            "originator": "test_originator",
            "cli_version": "test_version",
            "source": "cli",
            "model_provider": "test-provider",
        },
    });
    writeln!(stale_file, "{stale_meta}")?;
    let stale_user_event = json!({
        "timestamp": "2025-01-01T00:00:00Z",
        "type": "event_msg",
        "payload": {
            "type": "user_message",
            "message": "stale history",
            "kind": "plain",
        },
    });
    writeln!(stale_file, "{stale_user_event}")?;

    let stale_resume_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: other_thread_id.clone(),
            path: Some(stale_path),
            ..Default::default()
        })
        .await?;
    let stale_resume_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_error_message(RequestId::Integer(stale_resume_id)),
    )
    .await??;
    assert!(
        stale_resume_err.error.message.contains("stale path"),
        "unexpected resume error: {}",
        stale_resume_err.error.message
    );

    let resume_by_path_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: other_thread_id.clone(),
            path: thread.path,
            ..Default::default()
        })
        .await?;
    let resume_by_path_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(resume_by_path_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_by_path_resp)?;
    assert_eq!(resumed.id, thread_id);

    primary
        .interrupt_turn_and_wait_for_aborted(thread_id, running_turn.id, DEFAULT_READ_TIMEOUT)
        .await?;

    Ok(())
}

#[tokio::test]
async fn thread_resume_rejoins_running_thread_even_with_override_mismatch() -> Result<()> {
    let server = responses::start_mock_server().await;
    let first_response = responses::sse_response(responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]));
    let second_response = responses::sse_response(responses::sse(vec![
        responses::ev_response_created("resp-2"),
        responses::ev_assistant_message("msg-2", "Done"),
        responses::ev_completed("resp-2"),
    ]))
    .set_delay(std::time::Duration::from_millis(500));
    let _response_mock =
        responses::mount_response_sequence(&server, vec![first_response, second_response]).await;
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
                text: "keep running".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(running_turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/started"),
    )
    .await??;

    let resume_id = primary
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
            model: Some("not-the-running-model".to_string()),
            cwd: Some("/tmp".to_string()),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, model, .. } =
        to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(model, "gpt-5.4");
    // The running-thread resume response is queued onto the thread listener task.
    // If the in-flight turn completes before that queued command runs, the response
    // can legitimately observe the thread as idle.
    match &thread.lifecycle_status {
        ThreadLifecycleStatus::Active { active_flags } => {
            assert_eq!(active_flags.as_slice(), [ThreadLifecycleActiveFlag::Running])
        }
        ThreadLifecycleStatus::Final { .. } => {}
        status => panic!("unexpected thread status after running resume: {status:?}"),
    }

    timeout(
        DEFAULT_READ_TIMEOUT,
        primary.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn thread_resume_can_skip_turns_when_thread_is_running() -> Result<()> {
    let server = responses::start_mock_server().await;
    let _response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            responses::ev_assistant_message("msg-1", "Done"),
            responses::ev_completed("resp-1"),
        ]),
    )
    .await;
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

    let turn_id = primary
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
        primary.read_stream_until_response_message(RequestId::Integer(turn_id)),
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
            thread_id: thread.id.clone(),
            exclude_turns: true,
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        secondary.read_stream_until_response_message(RequestId::Integer(resume_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;

    assert_eq!(resumed.id, thread.id);
    assert_eq!(resumed.lifecycle_status, ThreadLifecycleStatus::completed(None));
    assert!(resumed.turns.is_empty());

    Ok(())
}
