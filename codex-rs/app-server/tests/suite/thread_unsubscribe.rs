use anyhow::Result;
use app_server_protocol::DynamicToolCallOutputContentItem;
use app_server_protocol::DynamicToolCallParams;
use app_server_protocol::DynamicToolCallResponse;
use app_server_protocol::DynamicToolSpec;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ServerRequest;
use app_server_protocol::ThreadClosedNotification;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadListParams;
use app_server_protocol::ThreadListResponse;
use app_server_protocol::ThreadLoadedListParams;
use app_server_protocol::ThreadLoadedListResponse;
use app_server_protocol::ThreadReadParams;
use app_server_protocol::ThreadReadResponse;
use app_server_protocol::ThreadResumeParams;
use app_server_protocol::ThreadResumeResponse;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadLifecycleStatus;
use app_server_protocol::ThreadSource;
use app_server_protocol::ThreadStatusChangedNotification;
use app_server_protocol::ThreadUnsubscribeParams;
use app_server_protocol::ThreadUnsubscribeResponse;
use app_server_protocol::ThreadUnsubscribeStatus;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::UserInput as V2UserInput;
use app_test_support::McpProcess;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::to_response;
use core_test_support::responses;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::time::Instant;
use tokio::time::sleep;
use tokio::time::timeout;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const TEST_THREAD_UNLOADING_DELAY_MS_ENV: &str = "CODEX_APP_SERVER_THREAD_UNLOADING_DELAY_MS";

#[tokio::test]
async fn thread_unsubscribe_keeps_thread_loaded_until_idle_timeout() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;

    let unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(unsubscribe_id)),
    )
    .await??;
    let unsubscribe = to_response::<ThreadUnsubscribeResponse>(unsubscribe_resp)?;
    assert_eq!(unsubscribe.status, ThreadUnsubscribeStatus::Unsubscribed);

    assert!(
        timeout(
            std::time::Duration::from_millis(250),
            mcp.read_stream_until_notification_message("thread/closed"),
        )
        .await
        .is_err()
    );

    let list_id = mcp
        .send_thread_loaded_list_request(ThreadLoadedListParams::default())
        .await?;
    let list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(list_id)),
    )
    .await??;
    let ThreadLoadedListResponse { data, next_cursor } =
        to_response::<ThreadLoadedListResponse>(list_resp)?;
    assert_eq!(data, vec![thread_id]);
    assert_eq!(next_cursor, None);

    Ok(())
}

#[tokio::test]
async fn thread_unsubscribe_hidden_external_root_closes_loaded_state_and_preserves_read_history() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_unsubscribe_mcp(codex_home.path(), fake_bin.path()).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;
    let input_text = "close keeps external root history";

    assert_loaded_threads(&mut mcp, &[thread_id.clone()]).await?;

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
    let _: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    let live_read = wait_for_thread_user_message(&mut mcp, &thread_id, input_text).await?;
    assert_external_root_metadata(&live_read, &thread_id, codex_home.path());
    assert_single_user_message_turn(&live_read, input_text);

    let unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(unsubscribe_id)),
    )
    .await??;
    let unsubscribe = to_response::<ThreadUnsubscribeResponse>(unsubscribe_resp)?;
    assert_eq!(unsubscribe.status, ThreadUnsubscribeStatus::Unsubscribed);

    wait_for_thread_closed(&mut mcp, &thread_id).await?;
    assert_loaded_threads(&mut mcp, &[]).await?;

    let closed_summary = read_thread(&mut mcp, &thread_id, /*include_turns*/ false).await?;
    assert_external_root_metadata(&closed_summary, &thread_id, codex_home.path());
    assert_eq!(closed_summary.turns, Vec::new());
    assert_eq!(
        closed_summary.lifecycle_status,
        ThreadLifecycleStatus::NotLoaded
    );

    let closed_with_turns = read_thread(&mut mcp, &thread_id, /*include_turns*/ true).await?;
    assert_external_root_metadata(&closed_with_turns, &thread_id, codex_home.path());
    assert_single_user_message_turn(&closed_with_turns, input_text);
    assert_eq!(
        closed_with_turns.lifecycle_status,
        ThreadLifecycleStatus::NotLoaded
    );

    let listed = list_threads(&mut mcp).await?;
    assert_eq!(
        listed
            .iter()
            .map(|thread| thread.id.as_str())
            .collect::<Vec<_>>(),
        vec![thread_id.as_str()]
    );
    assert_external_root_metadata(&listed[0], &thread_id, codex_home.path());

    drop(mcp);

    let mut restarted =
        start_external_root_unsubscribe_mcp(codex_home.path(), fake_bin.path()).await?;
    assert_loaded_threads(&mut restarted, &[]).await?;

    let reloaded_summary = read_thread(&mut restarted, &thread_id, /*include_turns*/ false).await?;
    assert_external_root_metadata(&reloaded_summary, &thread_id, codex_home.path());
    assert_eq!(reloaded_summary.turns, Vec::new());
    assert_eq!(
        reloaded_summary.lifecycle_status,
        ThreadLifecycleStatus::NotLoaded
    );

    let reloaded_with_turns =
        read_thread(&mut restarted, &thread_id, /*include_turns*/ true).await?;
    assert_external_root_metadata(&reloaded_with_turns, &thread_id, codex_home.path());
    assert_single_user_message_turn(&reloaded_with_turns, input_text);
    assert_eq!(
        reloaded_with_turns.lifecycle_status,
        ThreadLifecycleStatus::NotLoaded
    );

    let reloaded_listed = list_threads(&mut restarted).await?;
    assert_eq!(
        reloaded_listed
            .iter()
            .map(|thread| thread.id.as_str())
            .collect::<Vec<_>>(),
        vec![thread_id.as_str()]
    );
    assert_external_root_metadata(&reloaded_listed[0], &thread_id, codex_home.path());

    Ok(())
}

#[tokio::test]
async fn thread_unsubscribe_during_turn_keeps_turn_running() -> Result<()> {
    let call_id = "deterministic-wait-call";
    let tool_name = "deterministic_wait";
    let tool_args = json!({});
    let tool_call_arguments = serde_json::to_string(&tool_args)?;

    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let working_directory = tmp.path().join("workdir");
    std::fs::create_dir(&working_directory)?;

    let (server, mut completions) = start_streaming_sse_server(vec![
        vec![StreamingSseChunk {
            gate: None,
            body: responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_function_call(call_id, tool_name, &tool_call_arguments),
                responses::ev_completed("resp-1"),
            ]),
        }],
        vec![StreamingSseChunk {
            gate: None,
            body: responses::sse(vec![
                responses::ev_response_created("resp-2"),
                responses::ev_assistant_message("msg-1", "Done"),
                responses::ev_completed("resp-2"),
            ]),
        }],
    ])
    .await;
    let first_response_completed = completions.remove(0);
    let final_response_completed = completions.remove(0);
    create_config_toml(&codex_home, server.uri())?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            dynamic_tools: Some(vec![DynamicToolSpec {
                namespace: None,
                name: tool_name.to_string(),
                description: "Deterministic wait tool".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false,
                }),
                defer_loading: false,
            }]),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    let thread_id = thread.id;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "run deterministic tool".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: Some(working_directory),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let _: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        server.wait_for_request_count(/*count*/ 1),
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, first_response_completed).await??;

    let started = timeout(
        DEFAULT_READ_TIMEOUT,
        wait_for_dynamic_tool_started(&mut mcp, call_id),
    )
    .await??;
    assert_eq!(started.thread_id, thread_id);

    let request = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_request_message(),
    )
    .await??;
    let (request_id, params) = match request {
        ServerRequest::DynamicToolCall { request_id, params } => (request_id, params),
        other => panic!("expected DynamicToolCall request, got {other:?}"),
    };
    assert_eq!(
        params,
        DynamicToolCallParams {
            thread_id: thread_id.clone(),
            turn_id: started.turn_id,
            call_id: call_id.to_string(),
            namespace: None,
            tool: tool_name.to_string(),
            arguments: tool_args,
        }
    );

    let unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(unsubscribe_id)),
    )
    .await??;
    let unsubscribe = to_response::<ThreadUnsubscribeResponse>(unsubscribe_resp)?;
    assert_eq!(unsubscribe.status, ThreadUnsubscribeStatus::Unsubscribed);

    let closed_while_tool_call_blocked = timeout(
        std::time::Duration::from_millis(250),
        mcp.read_stream_until_notification_message("thread/closed"),
    );
    let closed_while_tool_call_blocked = closed_while_tool_call_blocked.await;
    assert!(closed_while_tool_call_blocked.is_err());

    let response = DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: "dynamic-ok".to_string(),
        }],
        success: true,
    };
    mcp.send_response(request_id, serde_json::to_value(response)?)
        .await?;

    timeout(
        DEFAULT_READ_TIMEOUT,
        server.wait_for_request_count(/*count*/ 2),
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, final_response_completed).await??;
    server.shutdown().await;

    Ok(())
}

#[tokio::test]
async fn thread_unsubscribe_preserves_cached_status_before_idle_unload() -> Result<()> {
    let server = responses::start_mock_server().await;
    let _response_mock = responses::mount_sse_once(
        &server,
        responses::sse_failed("resp-1", "server_error", "simulated failure"),
    )
    .await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![V2UserInput::Text {
                text: "fail this turn".to_string(),
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
    let _: TurnStartResponse = to_response::<TurnStartResponse>(turn_resp)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("error"),
    )
    .await??;

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
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;
    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::system_error(None));

    let unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(unsubscribe_id)),
    )
    .await??;
    let unsubscribe = to_response::<ThreadUnsubscribeResponse>(unsubscribe_resp)?;
    assert_eq!(unsubscribe.status, ThreadUnsubscribeStatus::Unsubscribed);
    assert!(
        timeout(
            std::time::Duration::from_millis(250),
            mcp.read_stream_until_notification_message("thread/closed"),
        )
        .await
        .is_err()
    );

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
    let resume: ThreadResumeResponse = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resume.thread.lifecycle_status, ThreadLifecycleStatus::system_error(None));

    Ok(())
}

#[tokio::test]
async fn thread_unsubscribe_reports_not_subscribed_before_idle_unload() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;

    let first_unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let first_unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_unsubscribe_id)),
    )
    .await??;
    let first_unsubscribe = to_response::<ThreadUnsubscribeResponse>(first_unsubscribe_resp)?;
    assert_eq!(first_unsubscribe.status, ThreadUnsubscribeStatus::Unsubscribed);

    let second_unsubscribe_id = mcp
        .send_thread_unsubscribe_request(ThreadUnsubscribeParams { thread_id })
        .await?;
    let second_unsubscribe_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_unsubscribe_id)),
    )
    .await??;
    let second_unsubscribe = to_response::<ThreadUnsubscribeResponse>(second_unsubscribe_resp)?;
    assert_eq!(
        second_unsubscribe.status,
        ThreadUnsubscribeStatus::NotSubscribed
    );

    Ok(())
}

async fn wait_for_dynamic_tool_started(
    mcp: &mut McpProcess,
    call_id: &str,
) -> Result<ItemStartedNotification> {
    loop {
        let notification = mcp
            .read_stream_until_notification_message("item/started")
            .await?;
        let Some(params) = notification.params else {
            continue;
        };
        let started: ItemStartedNotification = serde_json::from_value(params)?;
        if matches!(&started.item, ThreadItem::DynamicToolCall { id, .. } if id == call_id) {
            return Ok(started);
        }
    }
}

fn write_fake_claude_cli(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        "#!/bin/sh\n# Test double for hidden external root close coverage.\nwhile true; do sleep 0.1; done\n",
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

fn prepend_path_env(path: &Path) -> Result<String> {
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let paths =
        std::iter::once(path.to_path_buf()).chain(std::env::split_paths(&original_path));
    Ok(std::env::join_paths(paths)?.to_string_lossy().into_owned())
}

async fn start_external_root_unsubscribe_mcp(
    codex_home: &Path,
    fake_bin: &Path,
) -> Result<McpProcess> {
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    create_config_toml(codex_home, &server.uri())?;
    write_fake_claude_cli(fake_bin)?;
    let test_path = prepend_path_env(fake_bin)?;
    let mut mcp = McpProcess::new_with_env(
        codex_home,
        &[
            ("PATH", Some(test_path.as_str())),
            (TEST_THREAD_UNLOADING_DELAY_MS_ENV, Some("25")),
        ],
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;
    Ok(mcp)
}

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

async fn read_thread(
    mcp: &mut McpProcess,
    thread_id: &str,
    include_turns: bool,
) -> Result<app_server_protocol::Thread> {
    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.to_string(),
            include_turns,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;
    Ok(thread)
}

async fn list_threads(mcp: &mut McpProcess) -> Result<Vec<app_server_protocol::Thread>> {
    let list_id = mcp
        .send_thread_list_request(ThreadListParams {
            cursor: None,
            limit: Some(10),
            sort_key: None,
            sort_direction: None,
            model_providers: None,
            source_kinds: None,
            archived: None,
            cwd: None,
            use_state_db_only: false,
            search_term: None,
        })
        .await?;
    let list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(list_id)),
    )
    .await??;
    let ThreadListResponse { data, .. } = to_response::<ThreadListResponse>(list_resp)?;
    Ok(data)
}

async fn assert_loaded_threads(mcp: &mut McpProcess, expected: &[String]) -> Result<()> {
    let list_id = mcp
        .send_thread_loaded_list_request(ThreadLoadedListParams::default())
        .await?;
    let list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(list_id)),
    )
    .await??;
    let ThreadLoadedListResponse {
        mut data,
        next_cursor,
    } = to_response::<ThreadLoadedListResponse>(list_resp)?;
    data.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(data, expected);
    assert_eq!(next_cursor, None);
    Ok(())
}

async fn wait_for_thread_closed(mcp: &mut McpProcess, thread_id: &str) -> Result<()> {
    let deadline = Instant::now() + DEFAULT_READ_TIMEOUT;
    loop {
        let status = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("thread/status/changed"),
        )
        .await??;
        let status: ThreadStatusChangedNotification =
            serde_json::from_value(status.params.expect("thread/status/changed params"))?;
        if status.thread_id == thread_id
            && !matches!(
                status.lifecycle_status,
                ThreadLifecycleStatus::Active { .. }
            )
        {
            break;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for inactive status for thread {thread_id}");
        }
    }

    let closed = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/closed"),
    )
    .await??;
    let closed: ThreadClosedNotification =
        serde_json::from_value(closed.params.expect("thread/closed params"))?;
    assert_eq!(closed.thread_id, thread_id);
    Ok(())
}

fn assert_external_root_metadata(
    thread: &app_server_protocol::Thread,
    thread_id: &str,
    cwd: &Path,
) {
    assert_eq!(thread.id, thread_id);
    assert_eq!(thread.model_provider, "claude_cli");
    assert_eq!(thread.thread_source, Some(ThreadSource::User));
    assert_eq!(thread.agent_path, None);
    assert_eq!(thread.agent_role, None);
    let expected_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let actual_cwd =
        std::fs::canonicalize(thread.cwd.as_path()).unwrap_or_else(|_| thread.cwd.to_path_buf());
    assert_eq!(actual_cwd, expected_cwd);
    assert!(!thread.ephemeral);
    assert!(thread.path.as_ref().expect("thread path").is_absolute());
}

fn assert_single_user_message_turn(thread: &app_server_protocol::Thread, expected_text: &str) {
    assert_eq!(thread.turns.len(), 1, "expected one restored turn");
    let turn = &thread.turns[0];
    assert_eq!(turn.items_view, TurnItemsView::Full);
    let user_messages = turn
        .items
        .iter()
        .filter_map(|item| match item {
            ThreadItem::UserMessage { content, .. } => Some(content),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(user_messages.len(), 1, "expected one typed user message");
    assert_eq!(
        user_messages[0],
        &vec![V2UserInput::Text {
            text: expected_text.to_string(),
            text_elements: Vec::new(),
        }]
    );
}

fn has_single_user_message_turn(thread: &app_server_protocol::Thread, expected_text: &str) -> bool {
    if thread.turns.len() != 1 {
        return false;
    }
    thread.turns[0].items.iter().any(|item| match item {
        ThreadItem::UserMessage { content, .. } => {
            content
                == &vec![V2UserInput::Text {
                    text: expected_text.to_string(),
                    text_elements: Vec::new(),
                }]
        }
        _ => false,
    })
}

async fn wait_for_thread_user_message(
    mcp: &mut McpProcess,
    thread_id: &str,
    expected_text: &str,
) -> Result<app_server_protocol::Thread> {
    let deadline = Instant::now() + DEFAULT_READ_TIMEOUT;
    loop {
        let thread = read_thread(mcp, thread_id, /*include_turns*/ true).await?;
        if has_single_user_message_turn(&thread, expected_text) {
            return Ok(thread);
        }
        if Instant::now() >= deadline {
            return Ok(thread);
        }
        sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn create_config_toml(codex_home: &std::path::Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "danger-full-access"

model_provider = "mock_provider"

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

async fn start_thread(mcp: &mut McpProcess) -> Result<String> {
    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;
    Ok(thread.id)
}
