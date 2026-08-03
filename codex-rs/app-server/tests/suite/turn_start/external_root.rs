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

async fn start_named_external_root_thread(
    mcp: &mut McpProcess,
    cwd: &Path,
    task_name: &str,
) -> Result<String> {
    start_named_external_root_thread_with_provider(mcp, cwd, task_name, "claude_cli").await
}

async fn start_named_external_root_thread_with_provider(
    mcp: &mut McpProcess,
    cwd: &Path,
    task_name: &str,
    provider: &str,
) -> Result<String> {
    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            thread_provider: Some(provider.to_string()),
            cwd: Some(cwd.to_string_lossy().into_owned()),
            task_name: Some(task_name.to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    assert_eq!(thread.model_provider, provider);
    assert_eq!(thread.thread_source, Some(ThreadSource::User));
    let expected_agent_path = format!("/{task_name}");
    assert_eq!(thread.agent_path.as_deref(), Some(expected_agent_path.as_str()));
    assert_eq!(thread.agent_role.as_deref(), Some(provider));
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

async fn start_external_root_mcp_with_assistant_output(
    codex_home: &TempDir,
    fake_bin: &TempDir,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_claude_cli_with_assistant_output(fake_bin.path())?;
    let test_path = prepend_path_env(fake_bin.path())?;
    let mut mcp =
        McpProcess::new_with_env(codex_home.path(), &[("PATH", Some(test_path.as_str()))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

fn write_fake_claude_cli_with_input_capture(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        "#!/bin/sh\n# Test double for external root provider input coverage.\nif IFS= read -r line; then\n  if [ -n \"$FAKE_CLAUDE_STDIN_LOG\" ]; then\n    printf '%s\\n' \"$line\" > \"$FAKE_CLAUDE_STDIN_LOG\"\n  fi\n  echo 'External assistant done'\nfi\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_claude)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_claude, permissions)?;
    }
    Ok(())
}

async fn start_external_root_mcp_with_input_capture(
    codex_home: &TempDir,
    fake_bin: &TempDir,
    capture_path: &Path,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_claude_cli_with_input_capture(fake_bin.path())?;
    let test_path = prepend_path_env(fake_bin.path())?;
    let capture_path = capture_path.to_string_lossy().into_owned();
    let mut mcp = McpProcess::new_with_env(
        codex_home.path(),
        &[
            ("PATH", Some(test_path.as_str())),
            ("FAKE_CLAUDE_STDIN_LOG", Some(capture_path.as_str())),
        ],
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

fn write_fake_codex_cli_with_input_capture(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_codex = bin_dir.join("codex");
    std::fs::write(
        &fake_codex,
        r#"#!/bin/sh
# Test double for codex_cli app-server stdio input coverage.
if IFS= read -r _initialize; then
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"codexHome":"/tmp/fake-codex","platformFamily":"unix","platformOs":"macos","userAgent":"fake-codex-cli"}}'
fi
IFS= read -r _initialized || exit 0
if IFS= read -r _thread_start; then
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"fake-codex-thread"}}}'
fi
if IFS= read -r line; then
  if [ -n "$FAKE_CODEX_STDIN_LOG" ]; then
    printf '%s\n' "$line" > "$FAKE_CODEX_STDIN_LOG"
  fi
  printf '%s\n' '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"fake-codex-thread","turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":1,"completedAt":null,"durationMs":null}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"fake-codex-thread","turnId":"fake-codex-turn","item":{"type":"agentMessage","id":"fake-codex-agent-message","text":"Codex external assistant done","phase":null,"memoryCitation":null},"completedAtMs":2000}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"fake-codex-thread","turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":1000}}}'
fi
"#,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_codex)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_codex, permissions)?;
    }
    Ok(())
}

fn write_fake_codex_cli_with_error_notification(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_codex = bin_dir.join("codex");
    std::fs::write(
        &fake_codex,
        r#"#!/bin/sh
# Test double for real codex_cli app-server error notifications.
if IFS= read -r _initialize; then
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"codexHome":"/tmp/fake-codex","platformFamily":"unix","platformOs":"macos","userAgent":"fake-codex-cli"}}'
fi
IFS= read -r _initialized || exit 0
if IFS= read -r _thread_start; then
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"fake-codex-thread"}}}'
fi
if IFS= read -r _turn_start; then
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":null,"completedAt":null,"durationMs":null}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"fake-codex-thread","turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":1,"completedAt":null,"durationMs":null}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"error","params":{"threadId":"fake-codex-thread","turnId":"fake-codex-turn","error":{"message":"Authentication failed"},"additionalDetails":"unexpected status 401 Unauthorized","willRetry":false}}'
fi
"#,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_codex)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_codex, permissions)?;
    }
    Ok(())
}

fn write_fake_codex_cli_with_display_items(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_codex = bin_dir.join("codex");
    std::fs::write(
        &fake_codex,
        r#"#!/bin/sh
# Test double for codex_cli app-server display item bridging.
if IFS= read -r _initialize; then
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"codexHome":"/tmp/fake-codex","platformFamily":"unix","platformOs":"macos","userAgent":"fake-codex-cli"}}'
fi
IFS= read -r _initialized || exit 0
if IFS= read -r _thread_start; then
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"fake-codex-thread"}}}'
fi
if IFS= read -r _turn_start; then
  printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":null,"completedAt":null,"durationMs":null}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"fake-codex-thread","turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"inProgress","error":null,"startedAt":1,"completedAt":null,"durationMs":null}}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"fake-codex-thread","turnId":"fake-codex-turn","item":{"type":"reasoning","id":"fake-reasoning","summary":["checking project shape"],"content":[]},"completedAtMs":1500}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"fake-codex-thread","turnId":"fake-codex-turn","item":{"type":"eventDrivenToolCall","id":"fake-tool","tool":"read_file","arguments":{"path":"Cargo.toml"},"status":"completed","output":{"ok":true}},"completedAtMs":1700}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"fake-codex-thread","turnId":"fake-codex-turn","item":{"type":"agentMessage","id":"fake-codex-agent-message","text":"Codex final answer","phase":null,"memoryCitation":null},"completedAtMs":2000}}'
  printf '%s\n' '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"fake-codex-thread","turn":{"id":"fake-codex-turn","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":1000}}}'
fi
"#,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&fake_codex)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_codex, permissions)?;
    }
    Ok(())
}

async fn start_external_root_mcp_with_codex_input_capture(
    codex_home: &TempDir,
    fake_bin: &TempDir,
    capture_path: &Path,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_codex_cli_with_input_capture(fake_bin.path())?;
    let test_path = prepend_path_env(fake_bin.path())?;
    let capture_path = capture_path.to_string_lossy().into_owned();
    let mut mcp = McpProcess::new_with_env(
        codex_home.path(),
        &[
            ("PATH", Some(test_path.as_str())),
            ("FAKE_CODEX_STDIN_LOG", Some(capture_path.as_str())),
        ],
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

async fn start_external_root_mcp_with_codex_display_items(
    codex_home: &TempDir,
    fake_bin: &TempDir,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_codex_cli_with_display_items(fake_bin.path())?;
    let test_path = prepend_path_env(fake_bin.path())?;
    let mut mcp =
        McpProcess::new_with_env(codex_home.path(), &[("PATH", Some(test_path.as_str()))]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

async fn start_external_root_mcp_with_codex_error_notification(
    codex_home: &TempDir,
    fake_bin: &TempDir,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    create_config_toml(
        codex_home.path(),
        &server.uri(),
        "never",
        &BTreeMap::default(),
    )?;
    write_fake_codex_cli_with_error_notification(fake_bin.path())?;
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

async fn expect_no_loaded_threads(mcp: &mut McpProcess) -> Result<()> {
    let loaded_req = mcp
        .send_thread_loaded_list_request(ThreadLoadedListParams::default())
        .await?;
    let loaded_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(loaded_req)),
    )
    .await??;
    let ThreadLoadedListResponse { data, .. } =
        to_response::<ThreadLoadedListResponse>(loaded_resp)?;
    assert_eq!(data, Vec::<String>::new());
    Ok(())
}

fn injected_assistant_item(text: &str) -> Result<serde_json::Value> {
    let item = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    };
    Ok(serde_json::to_value(item)?)
}

async fn read_external_root_item_completed(
    mcp: &mut McpProcess,
    expected_thread_id: &str,
) -> Result<ItemCompletedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/completed"),
        )
        .await??;
        let completed: ItemCompletedNotification =
            serde_json::from_value(notification.params.unwrap_or_default())?;
        if completed.thread_id == expected_thread_id {
            return Ok(completed);
        }
    }
}

async fn read_external_root_turn_started(
    mcp: &mut McpProcess,
    expected_thread_id: &str,
) -> Result<TurnStartedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("turn/started"),
        )
        .await??;
        let started: TurnStartedNotification =
            serde_json::from_value(notification.params.unwrap_or_default())?;
        if started.thread_id == expected_thread_id {
            return Ok(started);
        }
    }
}

async fn read_external_root_turn_completed(
    mcp: &mut McpProcess,
    expected_thread_id: &str,
) -> Result<TurnCompletedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("turn/completed"),
        )
        .await??;
        let completed: TurnCompletedNotification =
            serde_json::from_value(notification.params.unwrap_or_default())?;
        if completed.thread_id == expected_thread_id {
            return Ok(completed);
        }
    }
}

fn assert_external_provider_init_context_item(
    item: &ThreadItem,
    input_text: &str,
    agent_path: &str,
    agent_role: &str,
) {
    let ThreadItem::InjectedContext {
        title,
        preview,
        sections,
        ..
    } = item
    else {
        panic!("expected external provider init context item, got {item:?}");
    };
    assert_eq!(title, "Init Context");
    assert_eq!(preview, "External provider initialization context");
    let context_text = sections
        .iter()
        .map(|section| format!("{}\n{}", section.label, section.text))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(context_text.contains(input_text));
    assert!(context_text.contains("spawn_external_agent"));
    assert!(context_text.contains("external_tool_call"));
    assert!(context_text.contains("Current external agent metadata"));
    assert!(context_text.contains(&format!("agent_path: {agent_path}")));
    assert!(context_text.contains(&format!("agent_role: {agent_role}")));
}

fn assert_turn_contains_external_provider_init_context(
    items: &[ThreadItem],
    input_text: &str,
    agent_path: &str,
    agent_role: &str,
) {
    let init_context = items
        .iter()
        .find(|item| {
            matches!(item, ThreadItem::InjectedContext { title, .. } if title == "Init Context")
        })
        .expect("expected persisted external provider init context item");
    assert_external_provider_init_context_item(init_context, input_text, agent_path, agent_role);
}

#[tokio::test]
async fn codex_cli_error_notification_is_visible_and_terminal() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp =
        start_external_root_mcp_with_codex_error_notification(&codex_home, &fake_bin).await?;
    let thread_id = start_named_external_root_thread_with_provider(
        &mut mcp,
        codex_home.path(),
        "dotfiles",
        "codex_cli",
    )
    .await?;
    let input_text = "Explain this project";

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: input_text.to_string(),
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

    let user_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(user_item.turn_id, turn.id);
    match user_item.item {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                vec![V2UserInput::Text {
                    text: input_text.to_string(),
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected codex_cli external user message item, got {other:?}"),
    }

    let init_context_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(init_context_item.turn_id, turn.id);
    assert_external_provider_init_context_item(
        &init_context_item.item,
        input_text,
        "/dotfiles",
        "codex_cli",
    );

    let error_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(error_item.turn_id, turn.id);
    let error_text = match error_item.item {
        ThreadItem::AgentMessage { text, .. } => text,
        other => panic!("expected codex_cli external error message item, got {other:?}"),
    };
    assert!(error_text.contains("codex_cli app-server error"));
    assert!(error_text.contains("Authentication failed"));
    assert!(error_text.contains("401 Unauthorized"));

    let completed = read_external_root_turn_completed(&mut mcp, &thread_id).await?;
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Failed);
    let error = completed
        .turn
        .error
        .expect("failed codex_cli turn should include an error");
    assert!(error.message.contains("Authentication failed"));

    let read_req = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_req)),
    )
    .await??;
    let read: ThreadReadResponse = to_response(read_resp)?;
    let persisted_turn = read
        .thread
        .turns
        .first()
        .expect("thread/read should include failed external turn");
    assert_eq!(persisted_turn.status, TurnStatus::Failed);
    assert!(persisted_turn.items.iter().any(|item| matches!(
        item,
        ThreadItem::AgentMessage { text, .. }
            if text.contains("Authentication failed") && text.contains("401 Unauthorized")
    )));
    assert_turn_contains_external_provider_init_context(
        &persisted_turn.items,
        input_text,
        "/dotfiles",
        "codex_cli",
    );

    Ok(())
}

#[tokio::test]
async fn codex_cli_display_items_are_visible_and_recoverable() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp =
        start_external_root_mcp_with_codex_display_items(&codex_home, &fake_bin).await?;
    let thread_id = start_named_external_root_thread_with_provider(
        &mut mcp,
        codex_home.path(),
        "dotfiles",
        "codex_cli",
    )
    .await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "Explain this project".to_string(),
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

    let _user_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    let init_context_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_external_provider_init_context_item(
        &init_context_item.item,
        "Explain this project",
        "/dotfiles",
        "codex_cli",
    );

    let reasoning_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(reasoning_item.turn_id, turn.id);
    match reasoning_item.item {
        ThreadItem::Reasoning { summary, content, .. } => {
            assert_eq!(summary, vec!["checking project shape".to_string()]);
            assert!(content.is_empty());
        }
        other => panic!("expected codex_cli reasoning item, got {other:?}"),
    }

    let tool_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(tool_item.turn_id, turn.id);
    match tool_item.item {
        ThreadItem::EventDrivenToolCall {
            id,
            tool,
            arguments,
            status,
            output,
        } => {
            assert_eq!(id, "fake-tool");
            assert_eq!(tool, "read_file");
            assert_eq!(arguments, serde_json::json!({"path": "Cargo.toml"}));
            assert_eq!(status, DynamicToolCallStatus::Completed);
            assert_eq!(output, Some(serde_json::json!({"ok": true})));
        }
        other => panic!("expected codex_cli tool display item, got {other:?}"),
    }

    let assistant_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(assistant_item.turn_id, turn.id);
    match assistant_item.item {
        ThreadItem::AgentMessage { text, .. } => {
            assert_eq!(text, "Codex final answer");
        }
        other => panic!("expected codex_cli final assistant item, got {other:?}"),
    }

    let completed = read_external_root_turn_completed(&mut mcp, &thread_id).await?;
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);

    let read_req = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_req)),
    )
    .await??;
    let read: ThreadReadResponse = to_response(read_resp)?;
    let persisted_turn = read
        .thread
        .turns
        .first()
        .expect("thread/read should include external display turn");
    assert!(persisted_turn.items.iter().any(|item| matches!(
        item,
        ThreadItem::Reasoning { summary, .. }
            if summary == &vec!["checking project shape".to_string()]
    )));
    assert!(persisted_turn.items.iter().any(|item| matches!(
        item,
        ThreadItem::EventDrivenToolCall { id, tool, status, .. }
            if id == "fake-tool" && tool == "read_file" && *status == DynamicToolCallStatus::Completed
    )));
    assert!(persisted_turn.items.iter().any(|item| matches!(
        item,
        ThreadItem::AgentMessage { text, .. } if text == "Codex final answer"
    )));
    assert_turn_contains_external_provider_init_context(
        &persisted_turn.items,
        "Explain this project",
        "/dotfiles",
        "codex_cli",
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_accepts_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp_with_assistant_output(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
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

    let started = read_external_root_turn_started(&mut mcp, &thread_id).await?;
    assert_eq!(started.turn.id, turn.id);
    assert_eq!(started.turn.status, TurnStatus::InProgress);

    let user_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(user_item.turn_id, turn.id);
    match user_item.item {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                vec![V2UserInput::Text {
                    text: "Hello external root".to_string(),
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected external user message item, got {other:?}"),
    }

    let init_context_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(init_context_item.turn_id, turn.id);
    assert_external_provider_init_context_item(
        &init_context_item.item,
        "Hello external root",
        "/root",
        "claude_cli",
    );

    let assistant_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(assistant_item.turn_id, turn.id);
    match assistant_item.item {
        ThreadItem::AgentMessage { text, .. } => {
            assert_eq!(text, "External assistant done");
        }
        other => panic!("expected external assistant message item, got {other:?}"),
    }

    let completed = read_external_root_turn_completed(&mut mcp, &thread_id).await?;
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);

    Ok(())
}

#[tokio::test]
async fn named_external_root_turn_start_accepts_text_input() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let provider_input_log = codex_home.path().join("provider-input.jsonl");
    let mut mcp =
        start_external_root_mcp_with_input_capture(&codex_home, &fake_bin, &provider_input_log)
            .await?;
    let thread_id =
        start_named_external_root_thread(&mut mcp, codex_home.path(), "foo_project").await?;
    let input_text = "Hello named external root";

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: input_text.to_string(),
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

    let user_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(user_item.turn_id, turn.id);
    match user_item.item {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                vec![V2UserInput::Text {
                    text: input_text.to_string(),
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected named external user message item, got {other:?}"),
    }

    let init_context_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(init_context_item.turn_id, turn.id);
    assert_external_provider_init_context_item(
        &init_context_item.item,
        input_text,
        "/foo_project",
        "claude_cli",
    );

    let assistant_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(assistant_item.turn_id, turn.id);
    match assistant_item.item {
        ThreadItem::AgentMessage { text, .. } => {
            assert_eq!(text, "External assistant done");
        }
        other => panic!("expected named external assistant message item, got {other:?}"),
    }

    let completed = read_external_root_turn_completed(&mut mcp, &thread_id).await?;
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);

    let provider_input_line = std::fs::read_to_string(&provider_input_log)?;
    let provider_input: serde_json::Value = serde_json::from_str(provider_input_line.trim())?;
    let provider_content = provider_input["message"]["content"]
        .as_str()
        .expect("provider content should be string");
    assert!(provider_content.contains(input_text));
    assert!(provider_content.contains("spawn_external_agent"));
    assert!(provider_content.contains("external_tool_call"));
    assert!(provider_content.contains("Current external agent metadata"));
    assert!(provider_content.contains("agent_path: /foo_project"));
    assert!(provider_content.contains("agent_role: claude_cli"));

    let read_req = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_req)),
    )
    .await??;
    let read: ThreadReadResponse = to_response(read_resp)?;
    let persisted_turn = read
        .thread
        .turns
        .first()
        .expect("thread/read should include named external turn");
    assert_turn_contains_external_provider_init_context(
        &persisted_turn.items,
        input_text,
        "/foo_project",
        "claude_cli",
    );

    Ok(())
}

#[tokio::test]
async fn codex_cli_named_external_root_turn_start_sends_context_to_provider() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let provider_input_log = codex_home.path().join("codex-provider-input.jsonl");
    let mut mcp =
        start_external_root_mcp_with_codex_input_capture(&codex_home, &fake_bin, &provider_input_log)
            .await?;
    let thread_id = start_named_external_root_thread_with_provider(
        &mut mcp,
        codex_home.path(),
        "cp_http_api",
        "codex_cli",
    )
    .await?;
    let input_text = "Explain this project";

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: input_text.to_string(),
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

    let user_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(user_item.turn_id, turn.id);
    match user_item.item {
        ThreadItem::UserMessage { content, .. } => {
            assert_eq!(
                content,
                vec![V2UserInput::Text {
                    text: input_text.to_string(),
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected codex_cli external user message item, got {other:?}"),
    }

    let init_context_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(init_context_item.turn_id, turn.id);
    assert_external_provider_init_context_item(
        &init_context_item.item,
        input_text,
        "/cp_http_api",
        "codex_cli",
    );

    let assistant_item = read_external_root_item_completed(&mut mcp, &thread_id).await?;
    assert_eq!(assistant_item.turn_id, turn.id);
    match assistant_item.item {
        ThreadItem::AgentMessage { text, .. } => {
            assert_eq!(text, "Codex external assistant done");
        }
        other => panic!("expected codex_cli external assistant message item, got {other:?}"),
    }

    let completed = read_external_root_turn_completed(&mut mcp, &thread_id).await?;
    assert_eq!(completed.turn.id, turn.id);
    assert_eq!(completed.turn.status, TurnStatus::Completed);

    let provider_input_line = std::fs::read_to_string(&provider_input_log)?;
    let provider_input: serde_json::Value = serde_json::from_str(provider_input_line.trim())?;
    assert_eq!(provider_input["method"], "turn/start");
    let provider_content = provider_input["params"]["input"][0]["text"]
        .as_str()
        .expect("codex provider content should be string");
    assert!(provider_content.contains(input_text));
    assert!(provider_content.contains("spawn_external_agent"));
    assert!(provider_content.contains("external_tool_call"));
    assert!(provider_content.contains("Current external agent metadata"));
    assert!(provider_content.contains("agent_path: /cp_http_api"));
    assert!(provider_content.contains("agent_role: codex_cli"));

    let read_req = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_req)),
    )
    .await??;
    let read: ThreadReadResponse = to_response(read_resp)?;
    let persisted_turn = read
        .thread
        .turns
        .first()
        .expect("thread/read should include codex_cli external turn");
    assert_turn_contains_external_provider_init_context(
        &persisted_turn.items,
        input_text,
        "/cp_http_api",
        "codex_cli",
    );

    Ok(())
}

#[tokio::test]
async fn external_root_turn_start_ignores_common_model_fields() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "Hello external root with UI model fields".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model-override".to_string()),
            model_provider: Some("mock-provider-override".to_string()),
            effort: Some(ReasoningEffort::High),
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
    expect_external_root_native_only_error(&mut mcp, clean_req, "thread/backgroundTerminals/clean")
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
            thread_id: thread_id.clone(),
            num_turns: 1,
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, second_rollback_req, "thread/rollback")
        .await?;

    let steer_req = mcp
        .send_turn_steer_request(TurnSteerParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "steer external root".to_string(),
                text_elements: Vec::new(),
            }],
            responsesapi_client_metadata: None,
            expected_turn_id: "turn-1".to_string(),
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, steer_req, "turn/steer").await?;

    let inline_review_req = mcp
        .send_review_start_request(ReviewStartParams {
            thread_id: thread_id.clone(),
            delivery: Some(ReviewDelivery::Inline),
            target: ReviewTarget::UncommittedChanges,
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, inline_review_req, "review/start").await?;

    let detached_review_req = mcp
        .send_review_start_request(ReviewStartParams {
            thread_id: thread_id.clone(),
            delivery: Some(ReviewDelivery::Detached),
            target: ReviewTarget::UncommittedChanges,
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, detached_review_req, "review/start").await?;

    let inject_req = mcp
        .send_thread_inject_items_request(ThreadInjectItemsParams {
            thread_id: thread_id.clone(),
            items: vec![injected_assistant_item("should not inject")?],
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, inject_req, "thread/inject_items").await?;

    let interrupt_req = mcp
        .send_turn_interrupt_request(TurnInterruptParams {
            thread_id,
            turn_id: String::new(),
        })
        .await?;
    expect_external_root_native_only_error(&mut mcp, interrupt_req, "turn/interrupt").await?;

    Ok(())
}

#[tokio::test]
async fn persisted_external_root_rejects_turn_processor_native_only_ops() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;
    drop(mcp);

    let mut restarted = start_external_root_mcp(&codex_home, &fake_bin).await?;
    expect_no_loaded_threads(&mut restarted).await?;

    let steer_req = restarted
        .send_turn_steer_request(TurnSteerParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "steer persisted external root".to_string(),
                text_elements: Vec::new(),
            }],
            responsesapi_client_metadata: None,
            expected_turn_id: "turn-1".to_string(),
        })
        .await?;
    expect_external_root_native_only_error(&mut restarted, steer_req, "turn/steer").await?;

    let review_req = restarted
        .send_review_start_request(ReviewStartParams {
            thread_id: thread_id.clone(),
            delivery: Some(ReviewDelivery::Inline),
            target: ReviewTarget::UncommittedChanges,
        })
        .await?;
    expect_external_root_native_only_error(&mut restarted, review_req, "review/start").await?;

    let inject_req = restarted
        .send_thread_inject_items_request(ThreadInjectItemsParams {
            thread_id: thread_id.clone(),
            items: vec![injected_assistant_item("should not inject after restart")?],
        })
        .await?;
    expect_external_root_native_only_error(&mut restarted, inject_req, "thread/inject_items")
        .await?;

    let interrupt_req = restarted
        .send_turn_interrupt_request(TurnInterruptParams {
            thread_id,
            turn_id: String::new(),
        })
        .await?;
    expect_external_root_native_only_error(&mut restarted, interrupt_req, "turn/interrupt").await?;

    expect_no_loaded_threads(&mut restarted).await?;

    Ok(())
}

#[tokio::test]
async fn persisted_external_root_rejects_thread_processor_core_ops_without_restore() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;
    drop(mcp);

    let mut restarted = start_external_root_mcp(&codex_home, &fake_bin).await?;
    expect_no_loaded_threads(&mut restarted).await?;

    let compact_req = restarted
        .send_thread_compact_start_request(ThreadCompactStartParams { thread_id })
        .await?;
    expect_external_root_native_only_error(&mut restarted, compact_req, "thread/compact/start")
        .await?;
    expect_no_loaded_threads(&mut restarted).await?;

    Ok(())
}

#[tokio::test]
async fn persisted_external_root_turn_start_rejects_without_restore() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_mcp(&codex_home, &fake_bin).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;
    drop(mcp);

    let mut restarted = start_external_root_mcp(&codex_home, &fake_bin).await?;
    expect_no_loaded_threads(&mut restarted).await?;

    let turn_req = restarted
        .send_turn_start_request(TurnStartParams {
            thread_id,
            input: vec![V2UserInput::Text {
                text: "resume external root".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    expect_external_root_native_only_error(&mut restarted, turn_req, "turn/start").await?;
    expect_no_loaded_threads(&mut restarted).await?;

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
            cwd: Some(codex_home.path().join("native-only-cwd")),
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
            .contains("external root turn/start does not support cwd"),
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
