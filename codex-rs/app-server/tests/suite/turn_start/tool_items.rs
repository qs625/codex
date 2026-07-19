use super::*;

#[tokio::test]
async fn turn_start_file_change_approval_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

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
        create_apply_patch_sse_response(patch, "patch-call")?,
        create_final_assistant_message_sse_response("patch applied")?,
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

    let start_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "apply patch".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let started_file_change = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.clone().expect("item/started params"))?;
            if let ThreadItem::FileChange { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::FileChange {
        ref id,
        status,
        ref changes,
    } = started_file_change
    else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call");
    assert_eq!(status, PatchApplyStatus::InProgress);
    let started_changes = changes.clone();

    let server_req = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::FileChangeRequestApproval { request_id, params } = server_req else {
        panic!("expected FileChangeRequestApproval request")
    };
    assert_eq!(params.item_id, "patch-call");
    assert_eq!(params.thread_id, thread.id);
    assert_eq!(params.turn_id, turn.id);
    let resolved_request_id = request_id.clone();
    let expected_readme_path = workspace.join("README.md");
    let expected_readme_path = expected_readme_path.to_string_lossy().into_owned();
    pretty_assertions::assert_eq!(
        started_changes,
        vec![app_server_protocol::FileUpdateChange {
            path: expected_readme_path.clone(),
            kind: PatchChangeKind::Add,
            diff: "new line\n".to_string(),
        }]
    );

    mcp.send_response(
        request_id,
        serde_json::to_value(FileChangeRequestApprovalResponse {
            decision: FileChangeApprovalDecision::Accept,
        })?,
    )
    .await?;
    let mut saw_resolved = false;
    let mut completed_file_change: Option<ThreadItem> = None;
    while completed_file_change.is_none() {
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
            "item/completed" => {
                let completed: ItemCompletedNotification = serde_json::from_value(
                    notification.params.clone().expect("item/completed params"),
                )?;
                if let ThreadItem::FileChange { .. } = completed.item {
                    assert!(saw_resolved, "serverRequest/resolved should arrive first");
                    completed_file_change = Some(completed.item);
                }
            }
            _ => {}
        }
    }
    let completed_file_change =
        completed_file_change.expect("file change completion should be observed");
    let ThreadItem::FileChange { ref id, status, .. } = completed_file_change else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call");
    assert_eq!(status, PatchApplyStatus::Completed);

    let readme_contents = std::fs::read_to_string(expected_readme_path)?;
    assert_eq!(readme_contents, "new line\n");

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_does_not_stream_apply_patch_change_updates_without_feature_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let call_id = "patch-call";
    let item_id = "fc-patch-call";
    let patch = "*** Begin Patch\n*** Add File: live.txt\n+live line\n*** End Patch\n";
    let patch_delta_1 = "*** Begin Patch\n*** Add File: live.txt\n+live";
    let patch_delta_2 = " line\n*** End Patch\n";
    let responses = vec![
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "custom_tool_call",
                    "id": item_id,
                    "call_id": call_id,
                    "name": "apply_patch",
                    "input": "",
                    "status": "in_progress"
                }
            }),
            serde_json::json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": item_id,
                "call_id": call_id,
                "delta": patch_delta_1,
            }),
            serde_json::json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": item_id,
                "call_id": call_id,
                "delta": patch_delta_2,
            }),
            responses::ev_apply_patch_custom_tool_call(call_id, patch),
            responses::ev_completed("resp-1"),
        ]),
        create_final_assistant_message_sse_response("patch applied")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(&codex_home, &server.uri(), "never", &BTreeMap::default())?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "apply patch".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    assert!(
        !mcp.pending_notification_methods()
            .iter()
            .any(|method| method == "item/fileChange/patchUpdated")
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_streams_apply_patch_change_updates_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let call_id = "patch-call";
    let item_id = "fc-patch-call";
    let patch = "*** Begin Patch\n*** Add File: live.txt\n+live line\n*** End Patch\n";
    let patch_delta_1 = "*** Begin Patch\n*** Add File: live.txt\n+live";
    let patch_delta_2 = " line\n*** End Patch\n";
    let responses = vec![
        responses::sse(vec![
            responses::ev_response_created("resp-1"),
            serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call",
                    "id": "fc-other-call",
                    "call_id": "other-call",
                    "name": "not_apply_patch",
                    "arguments": "",
                    "status": "in_progress"
                }
            }),
            serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "item_id": "fc-other-call",
                "delta": r#"{"input":"*** Begin Patch\n*** Add File: ignored.txt\n+ignored"#,
            }),
            serde_json::json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "custom_tool_call",
                    "id": item_id,
                    "call_id": call_id,
                    "name": "apply_patch",
                    "input": "",
                    "status": "in_progress"
                }
            }),
            serde_json::json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": item_id,
                "call_id": call_id,
                "delta": patch_delta_1,
            }),
            serde_json::json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": item_id,
                "call_id": call_id,
                "delta": patch_delta_2,
            }),
            responses::ev_apply_patch_custom_tool_call(call_id, patch),
            responses::ev_completed("resp-1"),
        ]),
        create_final_assistant_message_sse_response("patch applied")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(
        &codex_home,
        &server.uri(),
        "never",
        &BTreeMap::from([
            (Feature::ApplyPatchStreamingEvents, true),
            (Feature::Plugins, false),
            (Feature::RemoteModels, false),
            (Feature::ShellSnapshot, false),
        ]),
    )?;
    write_models_cache(&codex_home)?;
    let cache_path = codex_home.join("models_cache.json");
    let mut cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
    let models = cache["models"]
        .as_array_mut()
        .expect("models_cache.json models should be an array");
    let model = models
        .first_mut()
        .expect("models_cache.json should contain at least one model");
    model["slug"] = serde_json::Value::from("mock-model");
    model["display_name"] = serde_json::Value::from("mock-model");
    model["apply_patch_tool_type"] = serde_json::Value::from("freeform");
    std::fs::write(&cache_path, serde_json::to_string_pretty(&cache)?)?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "apply patch".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let mut streamed_content = String::new();
    while streamed_content != "live line\n" {
        let delta_notif = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/fileChange/patchUpdated"),
        )
        .await??;
        let delta: FileChangePatchUpdatedNotification = serde_json::from_value(
            delta_notif
                .params
                .clone()
                .expect("item/fileChange/patchUpdated params"),
        )?;
        assert_eq!(delta.thread_id, thread.id);
        assert_eq!(delta.turn_id, turn.id);
        assert_eq!(delta.item_id, call_id);
        let change = delta
            .changes
            .iter()
            .find(|change| change.path == "live.txt")
            .expect("live.txt change");
        assert!(matches!(change.kind, PatchChangeKind::Add));
        streamed_content = change.diff.clone();
    }

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_emits_spawn_agent_item_with_model_metadata_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    const CHILD_PROMPT: &str = "child: do work";
    const PARENT_PROMPT: &str = "spawn a child and continue";
    const SPAWN_CALL_ID: &str = "spawn-call-1";
    const REQUESTED_MODEL: &str = "gpt-5.2";
    const REQUESTED_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Low;

    let server = responses::start_mock_server().await;
    let spawn_args = serde_json::to_string(&json!({
        "message": CHILD_PROMPT,
        "model": REQUESTED_MODEL,
        "reasoning_effort": REQUESTED_REASONING_EFFORT,
    }))?;
    let _parent_turn = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| body_contains(req, PARENT_PROMPT),
        responses::sse(vec![
            responses::ev_response_created("resp-turn1-1"),
            responses::ev_function_call(SPAWN_CALL_ID, "spawn_agent", &spawn_args),
            responses::ev_completed("resp-turn1-1"),
        ]),
    )
    .await;
    let _child_turn = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| {
            body_contains(req, CHILD_PROMPT) && !body_contains(req, SPAWN_CALL_ID)
        },
        responses::sse(vec![
            responses::ev_response_created("resp-child-1"),
            responses::ev_assistant_message("msg-child-1", "child done"),
            responses::ev_completed("resp-child-1"),
        ]),
    )
    .await;
    let _parent_follow_up = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| body_contains(req, SPAWN_CALL_ID),
        responses::sse(vec![
            responses::ev_response_created("resp-turn1-2"),
            responses::ev_assistant_message("msg-turn1-2", "parent done"),
            responses::ev_completed("resp-turn1-2"),
        ]),
    )
    .await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Collab, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.3-codex".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: PARENT_PROMPT.to_string(),
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
    let turn: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    let spawn_started = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.expect("item/started params"))?;
            if let ThreadItem::CollabAgentToolCall { id, .. } = &started.item
                && id == SPAWN_CALL_ID
            {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    assert_eq!(
        spawn_started,
        ThreadItem::CollabAgentToolCall {
            id: SPAWN_CALL_ID.to_string(),
            tool: CollabAgentTool::SpawnAgent,
            status: CollabAgentToolCallStatus::InProgress,
            sender_thread_id: thread.id.clone(),
            sender_path: "/root".to_string(),
            receiver_thread_ids: Vec::new(),
            receiver_paths: Vec::new(),
            prompt: Some(CHILD_PROMPT.to_string()),
            model: Some(REQUESTED_MODEL.to_string()),
            reasoning_effort: Some(REQUESTED_REASONING_EFFORT),
            timeout_ms: None,
            agents_states: HashMap::new(),
        }
    );

    let (child_started_thread_id, spawn_completed) = timeout(DEFAULT_READ_TIMEOUT, async {
        let mut child_started_thread_id = None;
        loop {
            let JSONRPCMessage::Notification(notification) = mcp.read_next_message().await? else {
                continue;
            };
            match notification.method.as_str() {
                "thread/started" => {
                    let started: ThreadStartedNotification = serde_json::from_value(
                        notification.params.expect("thread/started params"),
                    )?;
                    if started.thread.thread_source == Some(ThreadSource::Subagent) {
                        child_started_thread_id = Some(started.thread.id);
                    }
                }
                "item/completed" => {
                    let completed: ItemCompletedNotification = serde_json::from_value(
                        notification.params.expect("item/completed params"),
                    )?;
                    if let ThreadItem::CollabAgentToolCall { id, .. } = &completed.item
                        && id == SPAWN_CALL_ID
                    {
                        return Ok::<(Option<String>, ThreadItem), anyhow::Error>((
                            child_started_thread_id,
                            completed.item,
                        ));
                    }
                }
                _ => {}
            }
        }
    })
    .await??;
    let ThreadItem::CollabAgentToolCall {
        id,
        tool,
        status,
        sender_thread_id,
        sender_path: _,
        receiver_thread_ids,
        receiver_paths: _,
        prompt,
        model,
        reasoning_effort,
        timeout_ms: _,
        agents_states,
    } = spawn_completed
    else {
        unreachable!("loop ensures we break on collab agent tool call items");
    };
    let receiver_thread_id = receiver_thread_ids
        .first()
        .cloned()
        .expect("spawn completion should include child thread id");
    assert_eq!(id, SPAWN_CALL_ID);
    assert_eq!(tool, CollabAgentTool::SpawnAgent);
    assert_eq!(status, CollabAgentToolCallStatus::Completed);
    assert_eq!(sender_thread_id, thread.id);
    assert_eq!(receiver_thread_ids, vec![receiver_thread_id.clone()]);
    assert_eq!(prompt, Some(CHILD_PROMPT.to_string()));
    assert_eq!(model, Some(REQUESTED_MODEL.to_string()));
    assert_eq!(reasoning_effort, Some(REQUESTED_REASONING_EFFORT));
    assert!(
        child_started_thread_id.is_none()
            || child_started_thread_id == Some(receiver_thread_id.clone()),
        "unexpected child started thread id: {child_started_thread_id:?}"
    );
    let agent_state = agents_states
        .get(&receiver_thread_id)
        .expect("spawn completion should include child agent state");
    assert!(
        matches!(
            agent_state.lifecycle_status,
            ThreadLifecycleStatus::Initializing | ThreadLifecycleStatus::Active { .. }
        ),
        "child agent should still be initializing or already running, got {:?}",
        agent_state.lifecycle_status
    );

    let turn_completed = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let turn_completed_notif = mcp
                .read_stream_until_notification_message("turn/completed")
                .await?;
            let turn_completed: TurnCompletedNotification = serde_json::from_value(
                turn_completed_notif.params.expect("turn/completed params"),
            )?;
            if turn_completed.thread_id == thread.id && turn_completed.turn.id == turn.turn.id {
                return Ok::<TurnCompletedNotification, anyhow::Error>(turn_completed);
            }
        }
    })
    .await??;
    assert_eq!(turn_completed.thread_id, thread.id);
    assert_eq!(turn_completed.turn.id, turn.turn.id);

    Ok(())
}

#[tokio::test]
async fn turn_start_emits_spawn_agent_item_with_effective_role_model_metadata_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    const CHILD_PROMPT: &str = "child: do work";
    const PARENT_PROMPT: &str = "spawn a child and continue";
    const SPAWN_CALL_ID: &str = "spawn-call-1";
    const REQUESTED_MODEL: &str = "gpt-5.2";
    const REQUESTED_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Low;
    const ROLE_MODEL: &str = "gpt-5.4";
    const ROLE_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::High;

    let server = responses::start_mock_server().await;
    let spawn_args = serde_json::to_string(&json!({
        "message": CHILD_PROMPT,
        "agent_type": "custom",
        "model": REQUESTED_MODEL,
        "reasoning_effort": REQUESTED_REASONING_EFFORT,
    }))?;
    let _parent_turn = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| body_contains(req, PARENT_PROMPT),
        responses::sse(vec![
            responses::ev_response_created("resp-turn1-1"),
            responses::ev_function_call(SPAWN_CALL_ID, "spawn_agent", &spawn_args),
            responses::ev_completed("resp-turn1-1"),
        ]),
    )
    .await;
    let _child_turn = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| {
            body_contains(req, CHILD_PROMPT) && !body_contains(req, SPAWN_CALL_ID)
        },
        responses::sse(vec![
            responses::ev_response_created("resp-child-1"),
            responses::ev_assistant_message("msg-child-1", "child done"),
            responses::ev_completed("resp-child-1"),
        ]),
    )
    .await;
    let _parent_follow_up = responses::mount_sse_once_match(
        &server,
        |req: &wiremock::Request| body_contains(req, SPAWN_CALL_ID),
        responses::sse(vec![
            responses::ev_response_created("resp-turn1-2"),
            responses::ev_assistant_message("msg-turn1-2", "parent done"),
            responses::ev_completed("resp-turn1-2"),
        ]),
    )
    .await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Collab, true)]),
    )?;
    std::fs::write(
        codex_home.path().join("custom-role.toml"),
        format!("model = \"{ROLE_MODEL}\"\nmodel_reasoning_effort = \"{ROLE_REASONING_EFFORT}\"\n",),
    )?;
    let config_path = codex_home.path().join("config.toml");
    let base_config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        &config_path,
        format!(
            r#"{base_config}

[agents.custom]
description = "Custom role"
config_file = "./custom-role.toml"
"#
        ),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.3-codex".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: PARENT_PROMPT.to_string(),
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
    let turn: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    let spawn_completed = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let completed_notif = mcp
                .read_stream_until_notification_message("item/completed")
                .await?;
            let completed: ItemCompletedNotification =
                serde_json::from_value(completed_notif.params.expect("item/completed params"))?;
            if let ThreadItem::CollabAgentToolCall { id, .. } = &completed.item
                && id == SPAWN_CALL_ID
            {
                return Ok::<ThreadItem, anyhow::Error>(completed.item);
            }
        }
    })
    .await??;
    let ThreadItem::CollabAgentToolCall {
        id,
        tool,
        status,
        sender_thread_id,
        sender_path: _,
        receiver_thread_ids,
        receiver_paths: _,
        prompt,
        model,
        reasoning_effort,
        timeout_ms: _,
        agents_states,
    } = spawn_completed
    else {
        unreachable!("loop ensures we break on collab agent tool call items");
    };
    let receiver_thread_id = receiver_thread_ids
        .first()
        .cloned()
        .expect("spawn completion should include child thread id");
    assert_eq!(id, SPAWN_CALL_ID);
    assert_eq!(tool, CollabAgentTool::SpawnAgent);
    assert_eq!(status, CollabAgentToolCallStatus::Completed);
    assert_eq!(sender_thread_id, thread.id);
    assert_eq!(receiver_thread_ids, vec![receiver_thread_id.clone()]);
    assert_eq!(prompt, Some(CHILD_PROMPT.to_string()));
    assert_eq!(model, Some(ROLE_MODEL.to_string()));
    assert_eq!(reasoning_effort, Some(ROLE_REASONING_EFFORT));
    let agent_state = agents_states
        .get(&receiver_thread_id)
        .expect("spawn completion should include child agent state");
    assert!(
        matches!(
            agent_state.lifecycle_status,
            ThreadLifecycleStatus::Initializing | ThreadLifecycleStatus::Active { .. }
        ),
        "child agent should still be initializing or already running, got {:?}",
        agent_state.lifecycle_status
    );

    let turn_completed = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let turn_completed_notif = mcp
                .read_stream_until_notification_message("turn/completed")
                .await?;
            let turn_completed: TurnCompletedNotification = serde_json::from_value(
                turn_completed_notif.params.expect("turn/completed params"),
            )?;
            if turn_completed.thread_id == thread.id && turn_completed.turn.id == turn.turn.id {
                return Ok::<TurnCompletedNotification, anyhow::Error>(turn_completed);
            }
        }
    })
    .await??;
    assert_eq!(turn_completed.thread_id, thread.id);

    Ok(())
}

#[tokio::test]
async fn turn_start_file_change_approval_accept_for_session_persists_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let patch_1 = r#"*** Begin Patch
*** Add File: README.md
+new line
*** End Patch
"#;
    let patch_2 = r#"*** Begin Patch
*** Update File: README.md
@@
-new line
+updated line
*** End Patch
"#;

    let responses = vec![
        create_apply_patch_sse_response(patch_1, "patch-call-1")?,
        create_final_assistant_message_sse_response("patch 1 applied")?,
        create_apply_patch_sse_response(patch_2, "patch-call-2")?,
        create_final_assistant_message_sse_response("patch 2 applied")?,
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

    let start_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    // First turn: expect FileChangeRequestApproval, respond with AcceptForSession, and verify the file exists.
    let turn_1_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "apply patch 1".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    let turn_1_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_1_req)),
    )
    .await??;
    let TurnStartResponse { turn: turn_1 } = to_response::<TurnStartResponse>(turn_1_resp)?;

    let started_file_change_1 = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.clone().expect("item/started params"))?;
            if let ThreadItem::FileChange { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::FileChange { id, status, .. } = started_file_change_1 else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call-1");
    assert_eq!(status, PatchApplyStatus::InProgress);

    let server_req = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::FileChangeRequestApproval { request_id, params } = server_req else {
        panic!("expected FileChangeRequestApproval request")
    };
    assert_eq!(params.item_id, "patch-call-1");
    assert_eq!(params.thread_id, thread.id);
    assert_eq!(params.turn_id, turn_1.id);

    mcp.send_response(
        request_id,
        serde_json::to_value(FileChangeRequestApprovalResponse {
            decision: FileChangeApprovalDecision::AcceptForSession,
        })?,
    )
    .await?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("item/completed"),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let readme_path = workspace.join("README.md");
    assert_eq!(std::fs::read_to_string(&readme_path)?, "new line\n");

    // Second turn: apply a patch to the same file. Approval should be skipped due to AcceptForSession.
    let turn_2_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "apply patch 2".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_2_req)),
    )
    .await??;

    let started_file_change_2 = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.clone().expect("item/started params"))?;
            if let ThreadItem::FileChange { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::FileChange { id, status, .. } = started_file_change_2 else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call-2");
    assert_eq!(status, PatchApplyStatus::InProgress);

    // If the server incorrectly emits FileChangeRequestApproval, the helper below will error
    // (it bails on unexpected JSONRPCMessage::Request), causing the test to fail.
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("item/completed"),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    assert_eq!(std::fs::read_to_string(readme_path)?, "updated line\n");

    Ok(())
}

#[tokio::test]
async fn turn_start_file_change_approval_decline_v2() -> Result<()> {
    skip_if_no_network!(Ok(()));

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
        create_apply_patch_sse_response(patch, "patch-call")?,
        create_final_assistant_message_sse_response("patch declined")?,
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

    let start_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "apply patch".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some(workspace.clone()),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let started_file_change = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let started_notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification =
                serde_json::from_value(started_notif.params.clone().expect("item/started params"))?;
            if let ThreadItem::FileChange { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::FileChange {
        ref id,
        status,
        ref changes,
    } = started_file_change
    else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call");
    assert_eq!(status, PatchApplyStatus::InProgress);
    let started_changes = changes.clone();

    let server_req = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let ServerRequest::FileChangeRequestApproval { request_id, params } = server_req else {
        panic!("expected FileChangeRequestApproval request")
    };
    assert_eq!(params.item_id, "patch-call");
    assert_eq!(params.thread_id, thread.id);
    assert_eq!(params.turn_id, turn.id);
    let expected_readme_path = workspace.join("README.md");
    let expected_readme_path_str = expected_readme_path.to_string_lossy().into_owned();
    pretty_assertions::assert_eq!(
        started_changes,
        vec![app_server_protocol::FileUpdateChange {
            path: expected_readme_path_str.clone(),
            kind: PatchChangeKind::Add,
            diff: "new line\n".to_string(),
        }]
    );

    mcp.send_response(
        request_id,
        serde_json::to_value(FileChangeRequestApprovalResponse {
            decision: FileChangeApprovalDecision::Decline,
        })?,
    )
    .await?;

    let completed_file_change = timeout(DEFAULT_READ_TIMEOUT, async {
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
            if let ThreadItem::FileChange { .. } = completed.item {
                return Ok::<ThreadItem, anyhow::Error>(completed.item);
            }
        }
    })
    .await??;
    let ThreadItem::FileChange { ref id, status, .. } = completed_file_change else {
        unreachable!("loop ensures we break on file change items");
    };
    assert_eq!(id, "patch-call");
    assert_eq!(status, PatchApplyStatus::Declined);

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    assert!(
        !expected_readme_path.exists(),
        "declined patch should not be applied"
    );

    Ok(())
}

#[tokio::test]
#[cfg_attr(windows, ignore = "process id reporting differs on Windows")]
async fn command_execution_notifications_include_process_id() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let responses = vec![
        create_exec_command_sse_response("uexec-1")?,
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

    let mut mcp = McpProcess::new(codex_home.path()).await?;
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
                text: "run a command".to_string(),
                text_elements: Vec::new(),
            }],
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::DangerFullAccess),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    let TurnStartResponse { turn: _turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let started_command = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let notif = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let started: ItemStartedNotification = serde_json::from_value(
                notif
                    .params
                    .clone()
                    .expect("item/started should include params"),
            )?;
            if let ThreadItem::CommandExecution { .. } = started.item {
                return Ok::<ThreadItem, anyhow::Error>(started.item);
            }
        }
    })
    .await??;
    let ThreadItem::CommandExecution {
        id,
        process_id: started_process_id,
        status,
        ..
    } = started_command
    else {
        unreachable!("loop ensures we break on command execution items");
    };
    assert_eq!(id, "uexec-1");
    assert_eq!(status, CommandExecutionStatus::InProgress);
    let started_process_id = started_process_id.expect("process id should be present");

    let completed_command = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let notif = mcp
                .read_stream_until_notification_message("item/completed")
                .await?;
            let completed: ItemCompletedNotification = serde_json::from_value(
                notif
                    .params
                    .clone()
                    .expect("item/completed should include params"),
            )?;
            if let ThreadItem::CommandExecution { .. } = completed.item {
                return Ok::<ThreadItem, anyhow::Error>(completed.item);
            }
        }
    })
    .await??;
    let ThreadItem::CommandExecution {
        id: completed_id,
        process_id: completed_process_id,
        status: completed_status,
        exit_code,
        ..
    } = completed_command
    else {
        unreachable!("loop ensures we break on command execution items");
    };
    assert_eq!(completed_id, "uexec-1");
    assert!(
        matches!(
            completed_status,
            CommandExecutionStatus::Completed | CommandExecutionStatus::Failed
        ),
        "unexpected command execution status: {completed_status:?}"
    );
    if completed_status == CommandExecutionStatus::Completed {
        assert_eq!(exit_code, Some(0));
    } else {
        assert!(exit_code.is_some(), "expected exit_code for failed command");
    }
    assert_eq!(
        completed_process_id.as_deref(),
        Some(started_process_id.as_str())
    );

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_with_elevated_override_does_not_persist_project_trust() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let workspace = TempDir::new()?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_request = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            ..Default::default()
        })
        .await?;
    let thread_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_request)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_response)?;

    let turn_request = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            cwd: Some(workspace.path().to_path_buf()),
            sandbox_policy: Some(app_server_protocol::SandboxPolicy::DangerFullAccess),
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_request)),
    )
    .await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let config_toml = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    assert!(!config_toml.contains("trust_level = \"trusted\""));
    assert!(!config_toml.contains(&workspace.path().display().to_string()));

    Ok(())
}
