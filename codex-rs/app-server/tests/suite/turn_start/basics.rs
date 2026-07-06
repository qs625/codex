use super::*;

#[tokio::test]
async fn turn_start_sends_originator_header() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.initialize_with_client_info(ClientInfo {
            name: TEST_ORIGINATOR.to_string(),
            title: Some("Codex VS Code Extension".to_string()),
            version: "0.1.0".to_string(),
        }),
    )
    .await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            thread_source: Some(ThreadSource::User),
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
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
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

    let requests = server
        .received_requests()
        .await
        .expect("failed to fetch received requests");
    assert!(!requests.is_empty());
    for request in requests {
        let originator = request
            .headers
            .get("originator")
            .expect("originator header missing");
        assert_eq!(originator.to_str()?, TEST_ORIGINATOR);
    }

    Ok(())
}

#[tokio::test]
async fn turn_start_emits_user_message_item_with_text_elements() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            thread_source: Some(ThreadSource::User),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let text_elements = vec![TextElement::new(
        ByteRange { start: 0, end: 5 },
        Some("<note>".to_string()),
    )];
    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: text_elements.clone(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;

    let user_message_item = timeout(DEFAULT_READ_TIMEOUT, async {
        loop {
            let notification = mcp
                .read_stream_until_notification_message("item/started")
                .await?;
            let params = notification.params.expect("item/started params");
            let item_started: ItemStartedNotification =
                serde_json::from_value(params).expect("deserialize item/started notification");
            if let ThreadItem::UserMessage { .. } = item_started.item {
                return Ok::<ThreadItem, anyhow::Error>(item_started.item);
            }
        }
    })
    .await??;

    match user_message_item {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                vec![V2UserInput::Text {
                    text: "Hello".to_string(),
                    text_elements,
                }]
            );
        }
        other => panic!("expected user message item, got {other:?}"),
    }

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_emits_thread_scoped_warning_notification_for_trimmed_skills() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;
    write_models_cache(codex_home.path())?;
    let cache_path = codex_home.path().join("models_cache.json");
    let mut cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache_path)?)?;
    let models = cache["models"]
        .as_array_mut()
        .expect("models_cache.json models should be an array");
    let entry = models
        .first_mut()
        .expect("models cache should not be empty");
    let model = entry["slug"]
        .as_str()
        .expect("model slug should be present")
        .to_string();
    entry["context_window"] = serde_json::Value::from(100);
    std::fs::write(&cache_path, serde_json::to_string_pretty(&cache)?)?;
    let config_path = codex_home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)?;
    std::fs::write(
        &config_path,
        config.replace("model = \"mock-model\"", &format!("model = \"{model}\"")),
    )?;
    write_test_skill(codex_home.path(), "alpha-skill")?;
    write_test_skill(codex_home.path(), "beta-skill")?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams::default())
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
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;

    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("warning"),
    )
    .await??;
    let params = notification.params.expect("warning params");
    let warning: WarningNotification =
        serde_json::from_value(params).expect("deserialize warning notification");
    assert_eq!(warning.thread_id.as_deref(), Some(thread.id.as_str()));
    assert!(
        warning.message.starts_with(
            "Exceeded skills context budget of 2%. All skill descriptions were removed and "
        ) && warning
            .message
            .ends_with(" additional skills were not included in the model-visible skills list."),
        "unexpected warning message: {}",
        warning.message
    );

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let requests = server
        .received_requests()
        .await
        .expect("failed to fetch received requests");
    let request = requests
        .last()
        .expect("expected at least one model request");
    assert!(
        body_contains(request, "## Skills"),
        "expected outgoing request to include the skills section"
    );
    assert!(
        !body_contains(request, "- alpha-skill:") && !body_contains(request, "- beta-skill:"),
        "expected trimmed skills to be omitted from the outgoing request body"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_sends_service_tier_id_to_model_request() -> Result<()> {
    let server = responses::start_mock_server().await;
    let body = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]);
    let response_mock = responses::mount_sse_once(&server, body).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_models_cache(codex_home.path())?;
    let service_tier_model = all_model_presets()
        .iter()
        .find(|preset| preset.show_in_picker && !preset.service_tiers.is_empty())
        .expect("bundled model catalog should include a picker model with service tiers");
    let service_tier_id = service_tier_model.service_tiers[0].id.clone();

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some(service_tier_model.id.clone()),
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
            thread_id: thread.id,
            service_tier: Some(Some(service_tier_id.clone())),
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
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

    assert_eq!(
        response_mock.single_request().body_json()["service_tier"],
        json!(service_tier_id)
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_omits_empty_instruction_overrides_from_model_request() -> Result<()> {
    let server = responses::start_mock_server().await;
    let body = responses::sse(vec![
        responses::ev_response_created("resp-1"),
        responses::ev_assistant_message("msg-1", "Done"),
        responses::ev_completed("resp-1"),
    ]);
    let response_mock = responses::mount_sse_once(&server, body).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            // TODO(aibrahim): Replace empty string instruction overrides with explicit tri-state
            // app-server semantics: omitted, explicitly none, or explicit value.
            config: Some(HashMap::from([(
                "include_permissions_instructions".to_string(),
                json!(false),
            )])),
            base_instructions: Some(String::new()),
            developer_instructions: Some(String::new()),
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
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
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

    let request_body = response_mock.single_request().body_json();
    let empty_developer_input_texts = request_body["input"]
        .as_array()
        .expect("input array")
        .iter()
        .filter(|item| item.get("role").and_then(serde_json::Value::as_str) == Some("developer"))
        .filter_map(|item| item.get("content").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|content| {
            content.get("type").and_then(serde_json::Value::as_str) == Some("input_text")
        })
        .filter_map(|content| content.get("text").and_then(serde_json::Value::as_str))
        .filter(|text| text.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        json!({
            "hasInstructions": request_body.get("instructions").is_some(),
            "emptyDeveloperInputTexts": empty_developer_input_texts,
        }),
        json!({
            "hasInstructions": false,
            "emptyDeveloperInputTexts": [],
        })
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_tracks_turn_event_analytics() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml_with_chatgpt_base_url(
        codex_home.path(),
        &server.uri(),
        &server.uri(),
    )?;
    mount_analytics_capture(&server, codex_home.path()).await?;

    let mut mcp = McpProcess::new_without_managed_config(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            thread_source: Some(ThreadSource::User),
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
            input: vec![V2UserInput::Image {
                url: "https://example.com/a.png".to_string(),
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

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    let event = wait_for_analytics_event(&server, DEFAULT_READ_TIMEOUT, "codex_turn_event").await?;
    assert_eq!(event["event_params"]["thread_id"], thread.id);
    assert_eq!(event["event_params"]["turn_id"], turn.id);
    assert_eq!(
        event["event_params"]["app_server_client"]["product_client_id"],
        DEFAULT_CLIENT_NAME
    );
    assert_eq!(event["event_params"]["model"], "mock-model");
    assert_eq!(event["event_params"]["model_provider"], "mock_provider");
    assert_eq!(event["event_params"]["sandbox_policy"], "read_only");
    assert_eq!(event["event_params"]["ephemeral"], false);
    assert_eq!(event["event_params"]["thread_source"], "user");
    assert_eq!(event["event_params"]["initialization_mode"], "new");
    assert_eq!(
        event["event_params"]["subagent_source"],
        serde_json::Value::Null
    );
    assert_eq!(
        event["event_params"]["parent_thread_id"],
        serde_json::Value::Null
    );
    assert_eq!(event["event_params"]["num_input_images"], 1);
    assert_eq!(event["event_params"]["status"], "completed");
    assert!(event["event_params"]["started_at"].as_u64().is_some());
    assert!(event["event_params"]["completed_at"].as_u64().is_some());
    assert!(event["event_params"]["duration_ms"].as_u64().is_some());
    assert_eq!(event["event_params"]["input_tokens"], 0);
    assert_eq!(event["event_params"]["cached_input_tokens"], 0);
    assert_eq!(event["event_params"]["output_tokens"], 0);
    assert_eq!(event["event_params"]["reasoning_output_tokens"], 0);
    assert_eq!(event["event_params"]["total_tokens"], 0);

    Ok(())
}

#[tokio::test]
async fn turn_start_accepts_text_at_limit_with_mention_item() -> Result<()> {
    let responses = vec![create_final_assistant_message_sse_response("Done")?];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
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
            thread_id: thread.id,
            input: vec![
                V2UserInput::Text {
                    text: "x".repeat(MAX_USER_INPUT_TEXT_CHARS),
                    text_elements: Vec::new(),
                },
                V2UserInput::Mention {
                    name: "Demo App".to_string(),
                    path: "app://demo-app".to_string(),
                },
            ],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;
    assert_eq!(turn.status, TurnStatus::InProgress);

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn turn_start_rejects_combined_oversized_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        "http://localhost/unused",
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let first = "x".repeat(MAX_USER_INPUT_TEXT_CHARS / 2);
    let second = "y".repeat(MAX_USER_INPUT_TEXT_CHARS / 2 + 1);
    let actual_chars = first.chars().count() + second.chars().count();

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id,
            input: vec![
                V2UserInput::Text {
                    text: first,
                    text_elements: Vec::new(),
                },
                V2UserInput::Text {
                    text: second,
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
    assert_eq!(data["actual_chars"], actual_chars);

    let turn_started = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await;
    assert!(
        turn_started.is_err(),
        "did not expect a turn/started notification for rejected input"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_rejects_invalid_permission_selection_before_starting_turn() -> Result<()> {
    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        "http://localhost/unused",
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;
    std::fs::write(
        codex_home.path().join("managed_config.toml"),
        "sandbox_mode = \"read-only\"\n",
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
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
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            permissions: Some(
                BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS
                    .to_string()
                    .into(),
            ),
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(err.error.message.contains("invalid turn context override"));
    assert!(
        err.error.message.contains("allowed set [ReadOnly]"),
        "unexpected error message: {}",
        err.error.message
    );
    let turn_started = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await;
    assert!(
        turn_started.is_err(),
        "did not expect a turn/started notification after rejected permissions selection"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_rejects_unknown_environment_before_starting_turn() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
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
            thread_id: thread.id,
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
                text_elements: Vec::new(),
            }],
            environments: Some(vec![TurnEnvironmentParams {
                environment_id: "missing".to_string(),
                cwd: codex_home.path().to_path_buf().try_into()?,
            }]),
            ..Default::default()
        })
        .await?;
    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(turn_req)),
    )
    .await??;

    assert_eq!(err.id, RequestId::Integer(turn_req));
    assert_eq!(err.error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(err.error.message, "unknown turn environment id `missing`");
    let turn_started = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await;
    assert!(
        turn_started.is_err(),
        "did not expect a turn/started notification after rejected environments"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_emits_notifications_and_accepts_model_override() -> Result<()> {
    // Provide a mock server and config so model wiring is valid.
    // Three Codex turns hit the mock model (session start + two turn/start calls).
    let responses = vec![
        create_final_assistant_message_sse_response("Done")?,
        create_final_assistant_message_sse_response("Done")?,
        create_final_assistant_message_sse_response("Done")?,
    ];
    let server = create_mock_responses_server_sequence_unchecked(responses).await;

    let codex_home = TempDir::new()?;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::from([(Feature::Personality, true)]),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // Start a thread and capture its id.
    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    // Start a turn with only input and thread_id set (no overrides).
    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "Hello".to_string(),
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
    assert!(!turn.id.is_empty());

    // Expect a turn/started notification.
    let notif: JSONRPCNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await??;
    let started: TurnStartedNotification =
        serde_json::from_value(notif.params.expect("params must be present"))?;
    assert_eq!(started.thread_id, thread.id);
    assert_eq!(
        started.turn.status,
        app_server_protocol::TurnStatus::InProgress
    );
    assert_eq!(started.turn.id, turn.id);
    assert_eq!(started.turn.items_view, TurnItemsView::NotLoaded);
    assert!(started.turn.items.is_empty());

    let completed_notif: JSONRPCNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed: TurnCompletedNotification = serde_json::from_value(
        completed_notif
            .params
            .expect("turn/completed params must be present"),
    )?;
    assert_eq!(completed.thread_id, thread.id);
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);
    assert_eq!(completed.turn.items_view, TurnItemsView::NotLoaded);
    assert!(completed.turn.items.is_empty());

    // Send a second turn that exercises the overrides path: change the model.
    let turn_req2 = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "Second".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model-override".to_string()),
            ..Default::default()
        })
        .await?;
    let turn_resp2: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req2)),
    )
    .await??;
    let TurnStartResponse { turn: turn2 } = to_response::<TurnStartResponse>(turn_resp2)?;
    assert!(!turn2.id.is_empty());
    // Ensure the second turn has a different id than the first.
    assert_ne!(turn.id, turn2.id);

    let notif2: JSONRPCNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await??;
    let started2: TurnStartedNotification =
        serde_json::from_value(notif2.params.expect("params must be present"))?;
    assert_eq!(started2.thread_id, thread.id);
    assert_eq!(started2.turn.id, turn2.id);
    assert_eq!(started2.turn.status, TurnStatus::InProgress);
    assert_eq!(started2.turn.items_view, TurnItemsView::NotLoaded);
    assert!(started2.turn.items.is_empty());

    let completed_notif2: JSONRPCNotification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed2: TurnCompletedNotification = serde_json::from_value(
        completed_notif2
            .params
            .expect("turn/completed params must be present"),
    )?;
    assert_eq!(completed2.thread_id, thread.id);
    assert_eq!(completed2.turn.id, turn2.id);
    assert_eq!(completed2.turn.status, TurnStatus::Completed);
    assert_eq!(completed2.turn.items_view, TurnItemsView::NotLoaded);
    assert!(completed2.turn.items.is_empty());

    Ok(())
}
