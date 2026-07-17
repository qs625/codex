use super::*;

#[tokio::test]
async fn thread_resume_can_skip_turns_for_metadata_only_resume() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Vec::new(),
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id.clone(),
            exclude_turns: true,
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
    assert!(thread.turns.is_empty());

    Ok(())
}

#[tokio::test]
async fn thread_resume_keeps_paused_goal_paused() -> Result<()> {
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
                text: "materialize this thread".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let goal_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread.id,
                "objective": "keep polishing",
                "status": "paused",
            })),
        )
        .await?;
    let goal_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(goal_id)),
    )
    .await??;
    let _goal: ThreadGoalSetResponse = to_response(goal_resp)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/updated"),
    )
    .await??;
    mcp.clear_message_buffer();

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
    let _resume: ThreadResumeResponse = to_response(resume_resp)?;
    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/updated"),
    )
    .await??;
    let notification: ServerNotification = notification.try_into()?;
    let ServerNotification::ThreadGoalUpdated(notification) = notification else {
        anyhow::bail!("expected thread goal update notification");
    };
    assert_eq!(notification.goal.status, ThreadGoalStatus::Paused);
    assert!(
        !mcp.pending_notification_methods()
            .iter()
            .any(|method| method == "turn/started"),
        "paused goal should not continue after thread resume"
    );

    Ok(())
}

#[tokio::test]
async fn thread_goal_set_preserves_budget_limited_same_objective() -> Result<()> {
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
                text: "materialize this thread".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let goal_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread.id,
                "objective": "keep polishing",
                "status": "budgetLimited",
                "tokenBudget": 10,
            })),
        )
        .await?;
    let goal_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(goal_id)),
    )
    .await??;
    let goal: ThreadGoalSetResponse = to_response(goal_resp)?;
    assert_eq!(goal.goal.status, ThreadGoalStatus::BudgetLimited);

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/updated"),
    )
    .await??;

    let replacement_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread.id,
                "objective": "keep polishing",
            })),
        )
        .await?;
    let replacement_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(replacement_id)),
    )
    .await??;
    let replacement: ThreadGoalSetResponse = to_response(replacement_resp)?;

    assert_eq!(replacement.goal.status, ThreadGoalStatus::BudgetLimited);
    assert_eq!(replacement.goal.token_budget, Some(10));
    assert_eq!(replacement.goal.tokens_used, 0);
    assert_eq!(replacement.goal.time_used_seconds, 0);

    Ok(())
}

#[tokio::test]
async fn thread_goal_set_edits_objective_without_resetting_usage() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;
    let config_path = codex_home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        &config_path,
        config.replace("personality = true\n", "personality = true\ngoals = true\n"),
    )?;
    let thread_id = create_fake_rollout(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "materialized thread",
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut mcp = McpProcess::new_without_managed_config(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let goal_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread_id,
                "objective": "keep polishing",
                "status": "active",
                "tokenBudget": 40,
            })),
        )
        .await?;
    let goal_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(goal_id)),
    )
    .await??;
    let goal: ThreadGoalSetResponse = to_response(goal_resp)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/updated"),
    )
    .await??;

    let state_db =
        StateRuntime::init(codex_home.path().to_path_buf(), "mock_provider".into()).await?;
    let thread_id = ThreadId::from_string(&thread_id)?;
    let persisted_goal = state_db
        .get_thread_goal(thread_id)
        .await?
        .expect("goal should exist");
    state_db
        .account_thread_goal_usage(
            thread_id,
            /*time_delta_seconds*/ 12,
            /*token_delta*/ 50,
            state::ThreadGoalAccountingMode::ActiveOnly,
            Some(persisted_goal.goal_id.as_str()),
        )
        .await?;

    let edit_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread_id.to_string(),
                "objective": "keep polishing with clearer wording",
                "status": "active",
                "tokenBudget": 40,
            })),
        )
        .await?;
    let edit_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(edit_id)),
    )
    .await??;
    let edit: ThreadGoalSetResponse = to_response(edit_resp)?;
    let updated_goal = state_db
        .get_thread_goal(thread_id)
        .await?
        .expect("goal should still exist");

    assert_eq!(persisted_goal.goal_id, updated_goal.goal_id);
    assert_eq!(edit.goal.objective, "keep polishing with clearer wording");
    assert_eq!(edit.goal.status, ThreadGoalStatus::BudgetLimited);
    assert_eq!(edit.goal.token_budget, Some(40));
    assert_eq!(edit.goal.tokens_used, 50);
    assert_eq!(edit.goal.time_used_seconds, 12);
    assert_eq!(edit.goal.created_at, goal.goal.created_at);

    Ok(())
}

#[tokio::test]
async fn thread_goal_clear_deletes_goal_and_notifies() -> Result<()> {
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
                text: "materialize this thread".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let _turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let goal_id = mcp
        .send_raw_request(
            "thread/goal/set",
            Some(json!({
                "threadId": thread.id,
                "objective": "keep polishing",
            })),
        )
        .await?;
    let goal_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(goal_id)),
    )
    .await??;
    let _goal: ThreadGoalSetResponse = to_response(goal_resp)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/updated"),
    )
    .await??;

    let clear_id = mcp
        .send_raw_request(
            "thread/goal/clear",
            Some(json!({
                "threadId": thread.id,
            })),
        )
        .await?;
    let clear_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(clear_id)),
    )
    .await??;
    let clear: ThreadGoalClearResponse = to_response(clear_resp)?;
    assert!(clear.cleared);

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/goal/cleared"),
    )
    .await??;

    let get_id = mcp
        .send_raw_request(
            "thread/goal/get",
            Some(json!({
                "threadId": thread.id,
            })),
        )
        .await?;
    let get_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(get_id)),
    )
    .await??;
    let get: app_server_protocol::ThreadGoalGetResponse = to_response(get_resp)?;
    assert_eq!(None, get.goal);

    let clear_again_id = mcp
        .send_raw_request(
            "thread/goal/clear",
            Some(json!({
                "threadId": thread.id,
            })),
        )
        .await?;
    let clear_again_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(clear_again_id)),
    )
    .await??;
    let clear_again: ThreadGoalClearResponse = to_response(clear_again_resp)?;
    assert!(!clear_again.cleared);

    Ok(())
}

#[tokio::test]
async fn thread_resume_emits_restored_token_usage_before_next_turn() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
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
    let response_token_usage = thread.token_usage.expect("thread/resume token usage");
    assert_eq!(response_token_usage.total.total_tokens, 150);
    assert_eq!(response_token_usage.last.total_tokens, 90);
    assert_eq!(response_token_usage.model_context_window, Some(200_000));

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/tokenUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadTokenUsageUpdated(notification) = parsed else {
        panic!("expected thread/tokenUsage/updated notification");
    };

    assert_eq!(notification.thread_id, thread.id);
    assert_eq!(notification.turn_id, thread.turns[0].id);
    assert_eq!(notification.token_usage.total.total_tokens, 150);
    assert_eq!(notification.token_usage.total.input_tokens, 120);
    assert_eq!(notification.token_usage.total.cached_input_tokens, 20);
    assert_eq!(notification.token_usage.total.output_tokens, 30);
    assert_eq!(notification.token_usage.total.reasoning_output_tokens, 10);
    assert_eq!(notification.token_usage.last.total_tokens, 90);
    assert_eq!(notification.token_usage.model_context_window, Some(200_000));

    Ok(())
}

#[tokio::test]
async fn thread_resume_recomputes_context_usage_without_persisted_snapshot() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
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
    let response_context_usage = thread.context_usage.expect("thread/resume context usage");
    assert!(response_context_usage.total_bytes > 0);
    assert!(response_context_usage.categories.user_messages > 0);
    assert_eq!(response_context_usage.budget_used_percent, Some(0));

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/contextUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadContextUsageUpdated(notification) = parsed else {
        panic!("expected thread/contextUsage/updated notification");
    };

    assert_eq!(notification.thread_id, thread.id);
    assert_eq!(notification.turn_id, thread.turns[0].id);
    assert!(notification.context_usage.total_bytes > 0);
    assert!(notification.context_usage.categories.user_messages > 0);
    assert_eq!(notification.context_usage.budget_used_percent, Some(0));

    Ok(())
}

#[tokio::test]
async fn thread_resume_ignores_persisted_zero_context_usage_snapshot() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Some("mock_provider"),
    )?;
    let rollout_file_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let zero_context_usage = json!({
        "timestamp": meta_rfc3339,
        "type": "event_msg",
        "payload": serde_json::to_value(EventMsg::ThreadContextUsageUpdated(
            ThreadContextUsageUpdatedEvent {
                usage: ThreadContextUsage {
                    total_bytes: 0,
                    budget_used_percent: Some(0),
                    categories: ThreadContextUsageCategoryBreakdown {
                        compact: 0,
                        skills_metadata: 0,
                        concrete_skills: 0,
                        tools_metadata: 0,
                        tool_calls: 0,
                        user_messages: 0,
                        llm_messages: 0,
                        reasoning: 0,
                    },
                    loaded_skills: ThreadContextUsageLoadedSkills {
                        loaded_count: 0,
                        total_count: Some(0),
                        skills: Vec::new(),
                    },
                    tool_breakdown: ThreadContextUsageToolBreakdown::default(),
                },
            }
        ))?,
    })
    .to_string();
    std::fs::write(
        &rollout_file_path,
        format!(
            "{}{}\n",
            std::fs::read_to_string(&rollout_file_path)?,
            zero_context_usage
        ),
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
    let response_context_usage = thread.context_usage.expect("thread/resume context usage");
    assert!(response_context_usage.total_bytes > 0);
    assert!(response_context_usage.categories.user_messages > 0);

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/contextUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadContextUsageUpdated(notification) = parsed else {
        panic!("expected thread/contextUsage/updated notification");
    };
    assert!(notification.context_usage.total_bytes > 0);
    assert!(notification.context_usage.categories.user_messages > 0);

    Ok(())
}

#[tokio::test]
async fn thread_resume_emits_restored_context_usage_before_next_turn() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Some("mock_provider"),
    )?;
    let rollout_file_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let persisted_rollout = std::fs::read_to_string(&rollout_file_path)?;
    let appended_rollout = json!({
        "timestamp": meta_rfc3339,
        "type": "event_msg",
        "payload": serde_json::to_value(EventMsg::ThreadContextUsageUpdated(
            ThreadContextUsageUpdatedEvent {
                usage: ThreadContextUsage {
                    total_bytes: 123456,
                    budget_used_percent: Some(61),
                    categories: ThreadContextUsageCategoryBreakdown {
                        compact: 10,
                        skills_metadata: 11,
                        concrete_skills: 12,
                        tools_metadata: 13,
                        tool_calls: 14,
                        user_messages: 15,
                        llm_messages: 16,
                        reasoning: 17,
                    },
                    loaded_skills: ThreadContextUsageLoadedSkills {
                        loaded_count: 1,
                        total_count: Some(4),
                        skills: Vec::new(),
                    },
                    tool_breakdown: ThreadContextUsageToolBreakdown {
                        commands: ThreadContextUsageToolBucket {
                            input: 42,
                            output: 7,
                        },
                        ..Default::default()
                    },
                },
            },
        ))?,
    })
    .to_string();
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
    let response_context_usage = thread.context_usage.expect("thread/resume context usage");
    assert_eq!(response_context_usage.total_bytes, 123456);
    assert_eq!(response_context_usage.budget_used_percent, Some(61));
    assert_eq!(response_context_usage.categories.tool_calls, 14);
    assert_eq!(response_context_usage.loaded_skills.loaded_count, 1);
    assert_eq!(response_context_usage.tool_breakdown.commands.input, 42);

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/contextUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadContextUsageUpdated(notification) = parsed else {
        panic!("expected thread/contextUsage/updated notification");
    };

    assert_eq!(notification.thread_id, thread.id);
    assert_eq!(notification.turn_id, thread.turns[0].id);
    assert_eq!(notification.token_usage.total.total_tokens, 150);
    assert_eq!(notification.token_usage.last.total_tokens, 90);
    assert_eq!(notification.token_usage.model_context_window, Some(200_000));
    assert_eq!(notification.context_usage.total_bytes, 123456);
    assert_eq!(notification.context_usage.budget_used_percent, Some(61));
    assert_eq!(notification.context_usage.categories.tool_calls, 14);
    assert_eq!(notification.context_usage.loaded_skills.loaded_count, 1);
    assert_eq!(notification.context_usage.tool_breakdown.commands.input, 42);

    Ok(())
}

#[tokio::test]
async fn thread_resume_skips_restored_token_usage_when_turns_are_excluded() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        Some("mock_provider"),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let first_resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id.clone(),
            ..Default::default()
        })
        .await?;
    let first_resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_resume_id)),
    )
    .await??;
    let ThreadResumeResponse { thread, .. } =
        to_response::<ThreadResumeResponse>(first_resume_resp)?;
    let expected_turn_id = thread.turns[0].id.clone();

    let first_note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/tokenUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = first_note.try_into()?;
    let ServerNotification::ThreadTokenUsageUpdated(notification) = parsed else {
        panic!("expected thread/tokenUsage/updated notification");
    };
    assert_eq!(notification.turn_id, expected_turn_id);

    let second_resume_id = mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: conversation_id,
            exclude_turns: true,
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
    assert!(resumed_again.turns.is_empty());

    let second_note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/tokenUsage/updated"),
    )
    .await;
    assert!(
        second_note.is_err(),
        "excludeTurns=true should not replay token usage"
    );

    Ok(())
}

#[tokio::test]
async fn thread_resume_token_usage_replay_ignores_stale_interrupted_tail_turn() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Some("mock_provider"),
    )?;
    let rollout_file_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let persisted_rollout = std::fs::read_to_string(&rollout_file_path)?;
    let stale_turn_id = "incomplete-turn-after-token-usage";
    let appended_rollout = [
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: stale_turn_id.to_string(),
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

    assert_eq!(thread.turns.len(), 2);
    assert_eq!(thread.turns[0].status, TurnStatus::Completed);
    assert_eq!(thread.turns[1].id, stale_turn_id);
    assert_eq!(thread.turns[1].status, TurnStatus::Interrupted);

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/tokenUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadTokenUsageUpdated(notification) = parsed else {
        panic!("expected thread/tokenUsage/updated notification");
    };

    assert_eq!(notification.thread_id, thread.id);
    assert_eq!(notification.turn_id, thread.turns[0].id);
    assert_ne!(notification.turn_id, stale_turn_id);
    assert_eq!(notification.token_usage.total.total_tokens, 150);
    assert_eq!(notification.token_usage.last.total_tokens, 90);

    Ok(())
}

#[tokio::test]
async fn thread_resume_token_usage_replay_can_belong_to_interrupted_turn() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let meta_rfc3339 = "2025-01-05T12:00:00Z";
    let conversation_id = create_fake_rollout_with_token_usage(
        codex_home.path(),
        filename_ts,
        meta_rfc3339,
        "Saved user message",
        Some("mock_provider"),
    )?;
    let rollout_file_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let persisted_rollout = std::fs::read_to_string(&rollout_file_path)?;
    let interrupted_turn_id = "interrupted-turn-with-token-usage";
    let appended_rollout = [
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: interrupted_turn_id.to_string(),
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
                message: "Interrupted after usage".to_string(),
                phase: None,
                memory_citation: None,
            }))?,
        })
        .to_string(),
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::TokenCount(TokenCountEvent {
                info: Some(TokenUsageInfo {
                    total_token_usage: TokenUsage {
                        input_tokens: 180,
                        cached_input_tokens: 40,
                        output_tokens: 50,
                        reasoning_output_tokens: 15,
                        total_tokens: 230,
                    },
                    last_token_usage: TokenUsage {
                        input_tokens: 90,
                        cached_input_tokens: 30,
                        output_tokens: 40,
                        reasoning_output_tokens: 12,
                        total_tokens: 130,
                    },
                    model_context_window: Some(200_000),
                }),
                rate_limits: None,
            }))?,
        })
        .to_string(),
        json!({
            "timestamp": meta_rfc3339,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::TurnAborted(TurnAbortedEvent {
                turn_id: Some(interrupted_turn_id.to_string()),
                reason: TurnAbortReason::Interrupted,
                completed_at: None,
                duration_ms: None,
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

    assert_eq!(thread.turns.len(), 2);
    assert_eq!(thread.turns[0].status, TurnStatus::Completed);
    assert_eq!(thread.turns[1].id, interrupted_turn_id);
    assert_eq!(thread.turns[1].status, TurnStatus::Interrupted);

    let note = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/tokenUsage/updated"),
    )
    .await??;
    let parsed: ServerNotification = note.try_into()?;
    let ServerNotification::ThreadTokenUsageUpdated(notification) = parsed else {
        panic!("expected thread/tokenUsage/updated notification");
    };

    assert_eq!(notification.thread_id, thread.id);
    assert_eq!(notification.turn_id, interrupted_turn_id);
    assert_eq!(notification.token_usage.total.total_tokens, 230);
    assert_eq!(notification.token_usage.last.total_tokens, 130);

    Ok(())
}
