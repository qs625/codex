//! End-to-end compaction flow tests.
//!
//! Phases:
//! 1) Arrange: mock local Responses SSE compact flow + config.
//! 2) Act: start a thread and submit multiple turns to trigger auto-compaction.
//! 3) Assert: verify item/started + item/completed notifications for context compaction.

#![expect(clippy::expect_used)]

use anyhow::Result;
use app_server_protocol::ItemCompletedNotification;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCNotification;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ThreadCompactStartParams;
use app_server_protocol::ThreadCompactStartResponse;
use app_server_protocol::ThreadContextUsageUpdatedNotification;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::UserInput as V2UserInput;
use app_test_support::ChatGptAuthFixture;
use app_test_support::McpProcess;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use app_test_support::write_mock_responses_config_toml;
use config_service::types::AuthCredentialsStoreMode;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use tempfile::TempDir;
use tokio::time::timeout;

// macOS and Windows Bazel CI can spend tens of seconds starting app-server
// subprocesses or processing test RPCs under load.
#[cfg(any(target_os = "macos", windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
#[cfg(not(any(target_os = "macos", windows)))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const AUTO_COMPACT_LIMIT: i64 = 1_000;
const COMPACT_PROMPT: &str = "Summarize the conversation.";
const INVALID_REQUEST_ERROR_CODE: i64 = -32600;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_local_emits_started_and_completed_items() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let sse1 = responses::sse(vec![
        responses::ev_assistant_message("m1", "FIRST_REPLY"),
        responses::ev_completed_with_tokens("r1", /*total_tokens*/ 70_000),
    ]);
    let sse2 = responses::sse(vec![
        responses::ev_assistant_message("m2", "SECOND_REPLY"),
        responses::ev_completed_with_tokens("r2", /*total_tokens*/ 330_000),
    ]);
    let sse3 = responses::sse(vec![
        responses::ev_assistant_message("m3", "LOCAL_SUMMARY"),
        responses::ev_completed_with_tokens("r3", /*total_tokens*/ 200),
    ]);
    let sse4 = responses::sse(vec![
        responses::ev_assistant_message("m4", "FINAL_REPLY"),
        responses::ev_completed_with_tokens("r4", /*total_tokens*/ 120),
    ]);
    responses::mount_sse_sequence(&server, vec![sse1, sse2, sse3, sse4]).await;

    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        AUTO_COMPACT_LIMIT,
        /*requires_openai_auth*/ None,
        "mock_provider",
        COMPACT_PROMPT,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;
    for message in ["first", "second", "third"] {
        send_turn_and_wait(&mut mcp, &thread_id, message).await?;
    }

    let started = wait_for_context_compaction_started(&mut mcp).await?;
    wait_for_compact_context_usage_updated(&mut mcp, &thread_id).await?;
    let completed = wait_for_context_compaction_completed(&mut mcp).await?;

    assert_context_compaction_lifecycle(started, completed, &thread_id, "SECOND_REPLY")?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_compaction_with_chatgpt_auth_still_uses_local_compact() -> Result<()> {
    skip_if_no_network!(Ok(()));
    const CHATGPT_AUTO_COMPACT_LIMIT: i64 = 200_000;

    let server = responses::start_mock_server().await;
    let sse1 = responses::sse(vec![
        responses::ev_assistant_message("m1", "FIRST_REPLY"),
        responses::ev_completed_with_tokens("r1", /*total_tokens*/ 70_000),
    ]);
    let sse2 = responses::sse(vec![
        responses::ev_assistant_message("m2", "SECOND_REPLY"),
        responses::ev_completed_with_tokens("r2", /*total_tokens*/ 330_000),
    ]);
    let sse3 = responses::sse(vec![
        responses::ev_assistant_message("m3", "LOCAL_SUMMARY"),
        responses::ev_completed_with_tokens("r3", /*total_tokens*/ 200),
    ]);
    let sse4 = responses::sse(vec![
        responses::ev_assistant_message("m4", "FINAL_REPLY"),
        responses::ev_completed_with_tokens("r4", /*total_tokens*/ 120),
    ]);
    let responses_log = responses::mount_sse_sequence(&server, vec![sse1, sse2, sse3, sse4]).await;

    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        CHATGPT_AUTO_COMPACT_LIMIT,
        Some(true),
        "mock_provider",
        COMPACT_PROMPT,
    )?;
    write_chatgpt_auth(
        codex_home.path(),
        ChatGptAuthFixture::new("access-chatgpt").plan_type("pro"),
        AuthCredentialsStoreMode::File,
    )?;

    let mut mcp = McpProcess::new_with_env(codex_home.path(), &[("OPENAI_API_KEY", None)]).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;
    for message in ["first", "second", "third"] {
        send_turn_and_wait(&mut mcp, &thread_id, message).await?;
    }

    let started = wait_for_context_compaction_started(&mut mcp).await?;
    let completed = wait_for_context_compaction_completed(&mut mcp).await?;

    assert_context_compaction_lifecycle(started, completed, &thread_id, "SECOND_REPLY")?;

    let response_requests = responses_log.requests();
    assert_eq!(response_requests.len(), 4);
    assert!(
        response_requests[2]
            .body_json()
            .to_string()
            .contains(COMPACT_PROMPT)
    );
    assert!(
        response_requests[3]
            .body_json()
            .to_string()
            .contains("LOCAL_SUMMARY")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thread_compact_start_triggers_compaction_and_returns_empty_response() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let sse = responses::sse(vec![
        responses::ev_assistant_message("m1", "MANUAL_COMPACT_SUMMARY"),
        responses::ev_completed_with_tokens("r1", /*total_tokens*/ 200),
    ]);
    responses::mount_sse_sequence(&server, vec![sse]).await;

    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        AUTO_COMPACT_LIMIT,
        /*requires_openai_auth*/ None,
        "mock_provider",
        COMPACT_PROMPT,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let thread_id = start_thread(&mut mcp).await?;
    let compact_id = mcp
        .send_thread_compact_start_request(ThreadCompactStartParams {
            thread_id: thread_id.clone(),
        })
        .await?;
    let compact_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(compact_id)),
    )
    .await??;
    let _compact: ThreadCompactStartResponse =
        to_response::<ThreadCompactStartResponse>(compact_resp)?;

    let started = wait_for_context_compaction_started(&mut mcp).await?;
    let completed = wait_for_context_compaction_completed(&mut mcp).await?;

    assert_context_compaction_lifecycle(started, completed, &thread_id, "MANUAL_COMPACT_SUMMARY")?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thread_compact_start_rejects_invalid_thread_id() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        AUTO_COMPACT_LIMIT,
        /*requires_openai_auth*/ None,
        "mock_provider",
        COMPACT_PROMPT,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_compact_start_request(ThreadCompactStartParams {
            thread_id: "not-a-thread-id".to_string(),
        })
        .await?;
    let error: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(error.error.message.contains("invalid thread id"));

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn thread_compact_start_rejects_unknown_thread_id() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let codex_home = TempDir::new()?;
    write_mock_responses_config_toml(
        codex_home.path(),
        &server.uri(),
        &BTreeMap::default(),
        AUTO_COMPACT_LIMIT,
        /*requires_openai_auth*/ None,
        "mock_provider",
        COMPACT_PROMPT,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_compact_start_request(ThreadCompactStartParams {
            thread_id: "67e55044-10b1-426f-9247-bb680e5fe0c8".to_string(),
        })
        .await?;
    let error: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(error.error.message.contains("thread not found"));

    Ok(())
}

async fn start_thread(mcp: &mut McpProcess) -> Result<String> {
    let thread_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;
    Ok(thread.id)
}

async fn send_turn_and_wait(mcp: &mut McpProcess, thread_id: &str, text: &str) -> Result<String> {
    let turn_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.to_string(),
            input: vec![V2UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_id)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;
    wait_for_turn_completed(mcp, &turn.id).await?;
    Ok(turn.id)
}

async fn wait_for_turn_completed(mcp: &mut McpProcess, turn_id: &str) -> Result<()> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("turn/completed"),
        )
        .await??;
        let completed: TurnCompletedNotification =
            serde_json::from_value(notification.params.clone().expect("turn/completed params"))?;
        if completed.turn.id == turn_id {
            return Ok(());
        }
    }
}

async fn wait_for_compact_context_usage_updated(
    mcp: &mut McpProcess,
    thread_id: &str,
) -> Result<()> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("thread/contextUsage/updated"),
        )
        .await??;
        let updated: ThreadContextUsageUpdatedNotification = serde_json::from_value(
            notification
                .params
                .clone()
                .expect("thread/contextUsage/updated params"),
        )?;
        if updated.thread_id == thread_id && updated.context_usage.categories.compact > 0 {
            return Ok(());
        }
    }
}

fn assert_context_compaction_lifecycle(
    started: ItemStartedNotification,
    completed: ItemCompletedNotification,
    thread_id: &str,
    expected_summary: &str,
) -> Result<()> {
    let ThreadItem::ContextCompaction {
        id: started_id,
        replacement_history: started_replacement_history,
    } = started.item
    else {
        unreachable!("started item should be context compaction");
    };
    let ThreadItem::ContextCompaction {
        id: completed_id,
        replacement_history: completed_replacement_history,
    } = completed.item
    else {
        unreachable!("completed item should be context compaction");
    };

    assert_eq!(started.thread_id, thread_id);
    assert_eq!(completed.thread_id, thread_id);
    assert_eq!(started_id, completed_id);
    assert!(started_replacement_history.is_none());

    let completed_replacement_history = completed_replacement_history
        .expect("completed compact item should include replacement history");
    let completed_replacement_history_json = serde_json::to_string(&completed_replacement_history)?;
    assert!(
        completed_replacement_history_json.contains(expected_summary),
        "replacement history should contain compact summary {expected_summary:?}: {completed_replacement_history_json}"
    );

    Ok(())
}

async fn wait_for_context_compaction_started(
    mcp: &mut McpProcess,
) -> Result<ItemStartedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/started"),
        )
        .await??;
        let started: ItemStartedNotification =
            serde_json::from_value(notification.params.clone().expect("item/started params"))?;
        if let ThreadItem::ContextCompaction { .. } = started.item {
            return Ok(started);
        }
    }
}

async fn wait_for_context_compaction_completed(
    mcp: &mut McpProcess,
) -> Result<ItemCompletedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/completed"),
        )
        .await??;
        let completed: ItemCompletedNotification =
            serde_json::from_value(notification.params.clone().expect("item/completed params"))?;
        if let ThreadItem::ContextCompaction { .. } = completed.item {
            return Ok(completed);
        }
    }
}
