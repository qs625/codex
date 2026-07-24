use anyhow::Result;
use app_server::in_process;
use app_server::in_process::InProcessStartArgs;
use app_server_protocol::ClientInfo;
use app_server_protocol::ClientRequest;
use app_server_protocol::ItemCompletedNotification;
use app_server_protocol::ItemStartedNotification;
use app_server_protocol::InitializeCapabilities;
use app_server_protocol::InitializeParams;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCMessage;
use app_server_protocol::JSONRPCNotification;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::SessionSource;
use app_server_protocol::SortDirection;
use app_server_protocol::ThreadLifecycleActiveFlag;
use app_server_protocol::ThreadForkParams;
use app_server_protocol::ThreadForkResponse;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadListParams;
use app_server_protocol::ThreadListResponse;
use app_server_protocol::ThreadNameUpdatedNotification;
use app_server_protocol::ThreadReadParams;
use app_server_protocol::ThreadReadResponse;
use app_server_protocol::ThreadResumeParams;
use app_server_protocol::ThreadResumeResponse;
use app_server_protocol::ThreadSetNameParams;
use app_server_protocol::ThreadSetNameResponse;
use app_server_protocol::ThreadSkill;
use app_server_protocol::ThreadSkillKind;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadLifecycleFinalStatus;
use app_server_protocol::ThreadLifecycleStatus;
use app_server_protocol::ThreadSource;
use app_server_protocol::ThreadTurnsItemsListParams;
use app_server_protocol::ThreadTurnsListParams;
use app_server_protocol::ThreadTurnsListResponse;
use app_server_protocol::TurnItemsView;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use app_server_protocol::TurnStatus;
use app_server_protocol::UserInput;
use app_test_support::McpProcess;
use app_test_support::create_fake_rollout_with_text_elements;
use app_test_support::create_fake_rollout_with_token_usage;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::create_mock_responses_server_sequence_unchecked;
use app_test_support::rollout_path;
use app_test_support::test_absolute_path;
use app_test_support::to_response;
use app_test_support::write_mock_responses_config_toml;
use codex_arg0::Arg0DispatchPaths;
use config_service::CloudRequirementsLoader;
use config_service::LoaderOverrides;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use core_test_support::responses;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use pretty_assertions::assert_eq;
use protocol::models::BaseInstructions;
use protocol::protocol::AgentMessageEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::ExternalTerminalStatus;
use protocol::protocol::ExternalTerminalStatusEvent;
use protocol::protocol::RolloutItem;
use protocol::protocol::SessionSource as ProtocolSessionSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadContextUsageCategoryBreakdown;
use protocol::protocol::ThreadContextUsageLoadedSkills;
use protocol::protocol::ThreadContextUsageToolBreakdown;
use protocol::protocol::ThreadContextUsageToolBucket;
use protocol::protocol::ThreadContextUsageUpdatedEvent;
use protocol::protocol::ThreadMemoryMode;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::UserMessageEvent;
use protocol::user_input::ByteRange;
use protocol::user_input::TextElement;
use rollout::ARCHIVED_SESSIONS_SUBDIR;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use tokio::time::Instant;
use tokio::time::sleep;
use thread_service::config::ConfigBuilder;
use thread_store::AppendThreadItemsParams;
use thread_store::CreateThreadParams;
use thread_store::InMemoryThreadStore;
use thread_store::ThreadEventPersistenceMode;
use thread_store::ThreadMetadataPatch;
use thread_store::ThreadPersistenceMetadata;
use thread_store::ThreadStore;
use thread_store::UpdateThreadMetadataParams;
use tokio::time::timeout;
use uuid::Uuid;

#[cfg(windows)]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
#[cfg(not(windows))]
const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn write_fake_claude_cli(bin_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bin_dir)?;
    let fake_claude = bin_dir.join("claude");
    std::fs::write(
        &fake_claude,
        "#!/bin/sh\n# Test double for hidden external root thread/read coverage.\nsleep 30\n",
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
    let paths = std::iter::once(path.to_path_buf()).chain(std::env::split_paths(&original_path));
    Ok(std::env::join_paths(paths)?.to_string_lossy().into_owned())
}

async fn start_external_root_read_mcp(codex_home: &Path, fake_bin: &Path) -> Result<McpProcess> {
    let server = create_mock_responses_server_sequence_unchecked(Vec::new()).await;
    create_config_toml(codex_home, &server.uri())?;
    write_fake_claude_cli(fake_bin)?;
    let test_path = prepend_path_env(fake_bin)?;
    let mut mcp =
        McpProcess::new_with_env(codex_home, &[("PATH", Some(test_path.as_str()))]).await?;
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
            limit: None,
            sort_key: None,
            sort_direction: None,
            model_providers: Some(Vec::new()),
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

async fn try_read_thread(
    mcp: &mut McpProcess,
    thread_id: &str,
    include_turns: bool,
) -> Result<Option<app_server_protocol::Thread>> {
    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread_id.to_string(),
            include_turns,
        })
        .await?;
    let message = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_or_error_message(RequestId::Integer(read_id)),
    )
    .await??;
    match message {
        JSONRPCMessage::Response(response) => {
            let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(response)?;
            Ok(Some(thread))
        }
        JSONRPCMessage::Error(error)
            if error
                .error
                .message
                .contains("includeTurns is unavailable before first user message") =>
        {
            Ok(None)
        }
        JSONRPCMessage::Error(error) => anyhow::bail!("thread/read failed: {:?}", error.error),
        JSONRPCMessage::Notification(_) | JSONRPCMessage::Request(_) => {
            unreachable!("read_stream_until_response_or_error_message filters these variants")
        }
    }
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
        &vec![UserInput::Text {
            text: expected_text.to_string(),
            text_elements: Vec::new(),
        }]
    );
}

fn has_single_user_message_turn(thread: &app_server_protocol::Thread, expected_text: &str) -> bool {
    if thread.turns.len() != 1 {
        return false;
    }
    let turn = &thread.turns[0];
    turn.items.iter().any(|item| match item {
        ThreadItem::UserMessage { content, .. } => {
            content
                == &vec![UserInput::Text {
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
    let mut last_thread = None;
    loop {
        let thread = try_read_thread(mcp, thread_id, /*include_turns*/ true).await?;
        if let Some(thread) = thread {
            if has_single_user_message_turn(&thread, expected_text) {
                return Ok(thread);
            }
            last_thread = Some(thread);
        }
        if Instant::now() >= deadline {
            if let Some(thread) = last_thread {
                return Ok(thread);
            }
            let thread = read_thread(mcp, thread_id, /*include_turns*/ false).await?;
            return Ok(thread);
        }
        sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn thread_read_returns_summary_without_turns() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let preview = "Saved user message";
    let text_elements = [TextElement::new(
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

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(thread.id, conversation_id);
    assert_eq!(thread.preview, preview);
    assert_eq!(thread.model_provider, "mock_provider");
    assert!(!thread.ephemeral, "stored rollouts should not be ephemeral");
    assert!(thread.path.as_ref().expect("thread path").is_absolute());
    assert_eq!(thread.cwd, test_absolute_path("/"));
    assert_eq!(thread.cli_version, "0.0.0");
    assert_eq!(thread.source, SessionSource::Cli);
    assert_eq!(thread.git_info, None);
    assert_eq!(thread.turns.len(), 0);
    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::NotLoaded);

    Ok(())
}

#[tokio::test]
async fn thread_read_hidden_external_root_preserves_metadata_before_turns() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_read_mcp(codex_home.path(), fake_bin.path()).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;

    let summary = read_thread(&mut mcp, &thread_id, /*include_turns*/ false).await?;
    assert_external_root_metadata(&summary, &thread_id, codex_home.path());
    assert_eq!(summary.turns, Vec::new());
    assert_ne!(summary.lifecycle_status, ThreadLifecycleStatus::NotLoaded);

    let with_turns = read_thread(&mut mcp, &thread_id, /*include_turns*/ true).await?;
    assert_external_root_metadata(&with_turns, &thread_id, codex_home.path());
    assert_eq!(with_turns.turns, Vec::new());
    assert_ne!(with_turns.lifecycle_status, ThreadLifecycleStatus::NotLoaded);

    Ok(())
}

#[tokio::test]
async fn thread_read_hidden_external_root_restores_text_turn_after_restart() -> Result<()> {
    let codex_home = TempDir::new()?;
    let fake_bin = TempDir::new()?;
    let mut mcp = start_external_root_read_mcp(codex_home.path(), fake_bin.path()).await?;
    let thread_id = start_hidden_external_root_thread(&mut mcp, codex_home.path()).await?;
    let input_text = "Hello persisted external root";

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text {
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

    drop(mcp);

    let mut restarted = start_external_root_read_mcp(codex_home.path(), fake_bin.path()).await?;
    let reloaded_summary = read_thread(&mut restarted, &thread_id, /*include_turns*/ false).await?;
    assert_external_root_metadata(&reloaded_summary, &thread_id, codex_home.path());
    assert_eq!(reloaded_summary.turns, Vec::new());
    assert_eq!(
        reloaded_summary.lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Interrupted,
        }
    );

    let reloaded_with_turns =
        read_thread(&mut restarted, &thread_id, /*include_turns*/ true).await?;
    assert_external_root_metadata(&reloaded_with_turns, &thread_id, codex_home.path());
    assert_single_user_message_turn(&reloaded_with_turns, input_text);
    assert_eq!(
        reloaded_with_turns.lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Interrupted,
        }
    );

    let reloaded_listed = list_threads(&mut restarted).await?;
    let listed = reloaded_listed
        .iter()
        .find(|thread| thread.id == thread_id)
        .expect("thread/list should include restarted external root");
    assert_external_root_metadata(listed, &thread_id, codex_home.path());
    assert_eq!(
        listed.lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: ThreadLifecycleFinalStatus::Interrupted,
        }
    );

    Ok(())
}

#[tokio::test]
async fn thread_read_can_include_turns() -> Result<()> {
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

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(thread.turns.len(), 1);
    let turn = &thread.turns[0];
    assert_eq!(turn.status, TurnStatus::Completed);
    assert_eq!(turn.items_view, TurnItemsView::Full);
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
    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::NotLoaded);

    Ok(())
}

#[tokio::test]
async fn thread_read_restores_usage_without_notifications() -> Result<()> {
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

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    let token_usage = thread.token_usage.expect("thread/read token usage");
    assert_eq!(token_usage.total.total_tokens, 150);
    assert_eq!(token_usage.last.total_tokens, 90);
    assert_eq!(token_usage.model_context_window, Some(200_000));

    let context_usage = thread.context_usage.expect("thread/read context usage");
    assert_eq!(context_usage.total_bytes, 123456);
    assert_eq!(context_usage.budget_used_percent, Some(61));
    assert_eq!(context_usage.categories.tool_calls, 14);
    assert_eq!(context_usage.loaded_skills.loaded_count, 1);
    assert_eq!(context_usage.tool_breakdown.commands.input, 42);
    assert_eq!(context_usage.tool_breakdown.commands.output, 7);

    Ok(())
}

#[tokio::test]
async fn thread_read_keeps_restored_context_usage_after_thread_resume() -> Result<()> {
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
    let resume_context_usage = thread
        .context_usage
        .expect("thread/resume should restore context usage");
    assert_eq!(resume_context_usage.total_bytes, 123456);
    assert_eq!(resume_context_usage.budget_used_percent, Some(61));
    assert_eq!(resume_context_usage.tool_breakdown.commands.input, 42);

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    let context_usage = thread
        .context_usage
        .expect("thread/read should preserve restored context usage after resume");
    assert_eq!(context_usage.total_bytes, 123456);
    assert_eq!(context_usage.budget_used_percent, Some(61));
    assert_eq!(context_usage.categories.tool_calls, 14);
    assert_eq!(context_usage.loaded_skills.loaded_count, 1);
    assert_eq!(context_usage.tool_breakdown.commands.input, 42);

    Ok(())
}

#[tokio::test]
async fn thread_turns_list_can_page_backward_and_forward() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        filename_ts,
        "2025-01-05T12:00:00Z",
        "first",
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let rollout_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    append_user_message(rollout_path.as_path(), "2025-01-05T12:01:00Z", "second")?;
    append_user_message(rollout_path.as_path(), "2025-01-05T12:02:00Z", "third")?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: conversation_id.clone(),
            cursor: None,
            limit: Some(2),
            sort_direction: Some(SortDirection::Desc),
            items_view: None,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse {
        data,
        next_cursor,
        backwards_cursor,
    } = to_response::<ThreadTurnsListResponse>(read_resp)?;
    assert_eq!(turn_user_texts(&data), vec!["third", "second"]);
    assert!(
        data.iter()
            .all(|turn| turn.items_view == TurnItemsView::Summary)
    );
    let next_cursor = next_cursor.expect("expected nextCursor for older turns");
    let backwards_cursor = backwards_cursor.expect("expected backwardsCursor for newest turn");

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: conversation_id.clone(),
            cursor: Some(next_cursor),
            limit: Some(10),
            sort_direction: Some(SortDirection::Desc),
            items_view: None,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse { data, .. } = to_response::<ThreadTurnsListResponse>(read_resp)?;
    assert_eq!(turn_user_texts(&data), vec!["first"]);

    append_user_message(rollout_path.as_path(), "2025-01-05T12:03:00Z", "fourth")?;

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: conversation_id,
            cursor: Some(backwards_cursor),
            limit: Some(10),
            sort_direction: Some(SortDirection::Asc),
            items_view: None,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse { data, .. } = to_response::<ThreadTurnsListResponse>(read_resp)?;
    assert_eq!(turn_user_texts(&data), vec!["third", "fourth"]);

    Ok(())
}

#[tokio::test]
async fn thread_turns_list_supports_requested_items_view() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        filename_ts,
        "2025-01-05T12:00:00Z",
        "first",
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let rollout_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    append_agent_message(rollout_path.as_path(), "2025-01-05T12:01:00Z", "draft")?;
    append_agent_message(rollout_path.as_path(), "2025-01-05T12:02:00Z", "final")?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let full = read_single_turn_items_view(
        &mut mcp,
        conversation_id.as_str(),
        Some(TurnItemsView::Full),
    )
    .await?;
    assert_eq!(full.items_view, TurnItemsView::Full);
    assert_eq!(
        turn_agent_texts(std::slice::from_ref(&full)),
        vec!["draft", "final"]
    );

    let summary = read_single_turn_items_view(
        &mut mcp,
        conversation_id.as_str(),
        Some(TurnItemsView::Summary),
    )
    .await?;
    assert_eq!(summary.items_view, TurnItemsView::Summary);
    assert_eq!(
        turn_user_texts(std::slice::from_ref(&summary)),
        vec!["first"]
    );
    assert_eq!(
        turn_agent_texts(std::slice::from_ref(&summary)),
        vec!["final"]
    );

    let not_loaded = read_single_turn_items_view(
        &mut mcp,
        conversation_id.as_str(),
        Some(TurnItemsView::NotLoaded),
    )
    .await?;
    assert_eq!(not_loaded.items_view, TurnItemsView::NotLoaded);
    assert!(not_loaded.items.is_empty());
    assert_eq!(not_loaded.id, full.id);
    assert_eq!(not_loaded.status, full.status);
    assert_eq!(not_loaded.started_at, full.started_at);
    assert_eq!(not_loaded.completed_at, full.completed_at);
    assert_eq!(not_loaded.duration_ms, full.duration_ms);

    Ok(())
}

#[test]
fn thread_turns_list_reads_store_history_without_rollout_path() -> Result<()> {
    run_current_thread_test_with_stack(async {
        let codex_home = TempDir::new()?;
        let thread_id = protocol::ThreadId::from_string("00000000-0000-4000-8000-000000000123")?;
        let store_id = Uuid::new_v4().to_string();
        create_config_toml_with_thread_store(codex_home.path(), &store_id)?;
        let store = InMemoryThreadStore::for_id(store_id.clone());
        let _in_memory_store = InMemoryThreadStoreId { store_id };
        seed_pathless_store_thread(&store, thread_id).await?;

        let loader_overrides = LoaderOverrides::without_managed_config_for_tests();
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .loader_overrides(loader_overrides.clone())
            .build()
            .await?;
        let client = in_process::start(InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides,
            strict_config: false,
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(config_service::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli.into(),
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-app-server-tests".to_string(),
                    title: None,
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
            },
            channel_capacity: in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;

        let result = client
            .request(ClientRequest::ThreadTurnsList {
                request_id: RequestId::Integer(1),
                params: ThreadTurnsListParams {
                    thread_id: thread_id.to_string(),
                    cursor: None,
                    limit: Some(10),
                    sort_direction: Some(SortDirection::Asc),
                    items_view: None,
                },
            })
            .await?
            .expect("thread/turns/list should succeed");
        let ThreadTurnsListResponse { data, .. } = serde_json::from_value(result)?;

        assert_eq!(turn_user_texts(&data), vec!["history from store"]);

        client.shutdown().await?;
        Ok(())
    })
}

#[test]
fn thread_read_loaded_include_turns_uses_live_history_without_rollout_path() -> Result<()> {
    run_current_thread_test_with_stack(async {
        let codex_home = TempDir::new()?;
        let store_id = Uuid::new_v4().to_string();
        create_config_toml_with_thread_store(codex_home.path(), &store_id)?;
        let store = InMemoryThreadStore::for_id(store_id.clone());
        let _in_memory_store = InMemoryThreadStoreId { store_id };

        let loader_overrides = LoaderOverrides::without_managed_config_for_tests();
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .loader_overrides(loader_overrides.clone())
            .build()
            .await?;
        let client = in_process::start(InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides,
            strict_config: false,
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(config_service::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli.into(),
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-app-server-tests".to_string(),
                    title: None,
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
            },
            channel_capacity: in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;

        let result = client
            .request(ClientRequest::ThreadStart {
                request_id: RequestId::Integer(1),
                params: ThreadStartParams {
                    model: Some("mock-model".to_string()),
                    ..Default::default()
                },
            })
            .await?
            .expect("thread/start should succeed");
        let ThreadStartResponse { thread, .. } = serde_json::from_value(result)?;
        assert_eq!(thread.path, None);

        let thread_id = protocol::ThreadId::from_string(&thread.id)?;
        store
            .append_items(AppendThreadItemsParams {
                thread_id,
                items: store_history_items(),
            })
            .await?;

        let result = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(2),
                params: ThreadReadParams {
                    thread_id: thread.id,
                    include_turns: true,
                },
            })
            .await?
            .expect("thread/read should succeed");
        let ThreadReadResponse { thread, .. } = serde_json::from_value(result)?;

        assert_eq!(turn_user_texts(&thread.turns), Vec::<String>::new());

        client.shutdown().await?;
        Ok(())
    })
}

#[test]
fn thread_list_includes_store_thread_without_rollout_path() -> Result<()> {
    run_current_thread_test_with_stack(async {
        let codex_home = TempDir::new()?;
        let thread_id = protocol::ThreadId::from_string("00000000-0000-4000-8000-000000000124")?;
        let store_id = Uuid::new_v4().to_string();
        create_config_toml_with_thread_store(codex_home.path(), &store_id)?;
        let store = InMemoryThreadStore::for_id(store_id.clone());
        let _in_memory_store = InMemoryThreadStoreId { store_id };
        seed_pathless_store_thread(&store, thread_id).await?;

        let loader_overrides = LoaderOverrides::without_managed_config_for_tests();
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .loader_overrides(loader_overrides.clone())
            .build()
            .await?;
        let client = in_process::start(InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides,
            strict_config: false,
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(config_service::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli.into(),
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-app-server-tests".to_string(),
                    title: None,
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
            },
            channel_capacity: in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;

        let result = client
            .request(ClientRequest::ThreadList {
                request_id: RequestId::Integer(1),
                params: ThreadListParams {
                    cursor: None,
                    limit: Some(10),
                    sort_key: None,
                    sort_direction: None,
                    model_providers: Some(Vec::new()),
                    source_kinds: None,
                    archived: None,
                    cwd: None,
                    use_state_db_only: false,
                    search_term: None,
                },
            })
            .await?
            .expect("thread/list should succeed");
        let ThreadListResponse { data, .. } = serde_json::from_value(result)?;

        assert_eq!(data.len(), 1);
        let thread = &data[0];
        assert_eq!(thread.id, thread_id.to_string());
        assert_eq!(thread.path, None);
        assert_eq!(thread.preview, "");
        assert_eq!(thread.name.as_deref(), Some("named pathless thread"));

        client.shutdown().await?;
        Ok(())
    })
}

#[test]
fn thread_read_and_list_project_external_root_terminal_facts_without_live_runtime() -> Result<()> {
    run_current_thread_test_with_stack(async {
        let codex_home = TempDir::new()?;
        let completed_id =
            protocol::ThreadId::from_string("00000000-0000-4000-8000-000000000125")?;
        let errored_id = protocol::ThreadId::from_string("00000000-0000-4000-8000-000000000126")?;
        let store_id = Uuid::new_v4().to_string();
        create_config_toml_with_thread_store(codex_home.path(), &store_id)?;
        let store = InMemoryThreadStore::for_id(store_id.clone());
        let _in_memory_store = InMemoryThreadStoreId { store_id };
        seed_external_root_store_thread(
            &store,
            completed_id,
            codex_home.path(),
            vec![RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: "turn-completed".to_string(),
                last_agent_message: Some("done".to_string()),
                completed_at: Some(1),
                duration_ms: None,
                time_to_first_token_ms: None,
            }))],
        )
        .await?;
        seed_external_root_store_thread(
            &store,
            errored_id,
            codex_home.path(),
            vec![RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(
                ExternalTerminalStatusEvent {
                    thread_id: errored_id,
                    turn_id: "turn-errored".to_string(),
                    status: ExternalTerminalStatus::Errored,
                    message: Some("provider failed".to_string()),
                    terminal_at_ms: 1,
                },
            ))],
        )
        .await?;

        let loader_overrides = LoaderOverrides::without_managed_config_for_tests();
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .loader_overrides(loader_overrides.clone())
            .build()
            .await?;
        let client = in_process::start(InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides,
            strict_config: false,
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(config_service::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli.into(),
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-app-server-tests".to_string(),
                    title: None,
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
            },
            channel_capacity: in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;

        let completed_status = ThreadLifecycleStatus::completed(Some("done".to_string()));
        let errored_status = ThreadLifecycleStatus::errored(Some("provider failed".to_string()));

        let completed_summary_result = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(1),
                params: ThreadReadParams {
                    thread_id: completed_id.to_string(),
                    include_turns: false,
                },
            })
            .await?
            .expect("completed thread/read summary should succeed");
        let ThreadReadResponse {
            thread: completed_summary,
            ..
        } = serde_json::from_value(completed_summary_result)?;
        assert_eq!(completed_summary.lifecycle_status, completed_status);

        let completed_with_turns_result = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(2),
                params: ThreadReadParams {
                    thread_id: completed_id.to_string(),
                    include_turns: true,
                },
            })
            .await?
            .expect("completed thread/read with turns should succeed");
        let ThreadReadResponse {
            thread: completed_with_turns,
            ..
        } = serde_json::from_value(completed_with_turns_result)?;
        assert_eq!(completed_with_turns.lifecycle_status, completed_status);
        assert_eq!(
            turn_user_texts(&completed_with_turns.turns),
            vec!["history from store"]
        );

        let errored_summary_result = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(3),
                params: ThreadReadParams {
                    thread_id: errored_id.to_string(),
                    include_turns: false,
                },
            })
            .await?
            .expect("errored thread/read summary should succeed");
        let ThreadReadResponse {
            thread: errored_summary,
            ..
        } = serde_json::from_value(errored_summary_result)?;
        assert_eq!(errored_summary.lifecycle_status, errored_status);

        let errored_with_turns_result = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(4),
                params: ThreadReadParams {
                    thread_id: errored_id.to_string(),
                    include_turns: true,
                },
            })
            .await?
            .expect("errored thread/read with turns should succeed");
        let ThreadReadResponse {
            thread: errored_with_turns,
            ..
        } = serde_json::from_value(errored_with_turns_result)?;
        assert_eq!(errored_with_turns.lifecycle_status, errored_status);
        assert_eq!(
            turn_user_texts(&errored_with_turns.turns),
            vec!["history from store"]
        );

        let list_result = client
            .request(ClientRequest::ThreadList {
                request_id: RequestId::Integer(5),
                params: ThreadListParams {
                    cursor: None,
                    limit: Some(10),
                    sort_key: None,
                    sort_direction: None,
                    model_providers: Some(Vec::new()),
                    source_kinds: None,
                    archived: None,
                    cwd: None,
                    use_state_db_only: false,
                    search_term: None,
                },
            })
            .await?
            .expect("thread/list should succeed");
        let ThreadListResponse { data, .. } = serde_json::from_value(list_result)?;
        let listed_completed = data
            .iter()
            .find(|thread| thread.id == completed_id.to_string())
            .expect("thread/list should include completed external root");
        assert_eq!(listed_completed.lifecycle_status, completed_status);
        let listed_errored = data
            .iter()
            .find(|thread| thread.id == errored_id.to_string())
            .expect("thread/list should include errored external root");
        assert_eq!(listed_errored.lifecycle_status, errored_status);

        client.shutdown().await?;
        Ok(())
    })
}

fn run_current_thread_test_with_stack<Fut>(future: Fut) -> Result<()>
where
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    thread::Builder::new()
        .stack_size(4 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(future)
        })?
        .join()
        .unwrap_or_else(|err| panic!("thread_read test thread should not panic: {err:?}"))
}

#[test]
fn thread_read_includes_persisted_thread_skills_for_pathless_store_threads() -> Result<()> {
    run_current_thread_test_with_stack(async {
        let codex_home = TempDir::new()?;
        let thread_id = protocol::ThreadId::from_string("00000000-0000-4000-8000-000000000125")?;
        let store_id = Uuid::new_v4().to_string();
        create_config_toml_with_thread_store(codex_home.path(), &store_id)?;
        let store = InMemoryThreadStore::for_id(store_id.clone());
        let _in_memory_store = InMemoryThreadStoreId { store_id };
        seed_pathless_store_thread(&store, thread_id).await?;
        store
            .update_thread_metadata(UpdateThreadMetadataParams {
                thread_id,
                patch: ThreadMetadataPatch {
                    skills: Some(vec![protocol::protocol::ThreadSkill {
                        name: "demo".to_string(),
                        path: "/tmp/demo/SKILL.md".to_string(),
                        kind: protocol::protocol::ThreadSkillKind::All,
                    }]),
                    ..Default::default()
                },
                include_archived: true,
            })
            .await?;

        let loader_overrides = LoaderOverrides::without_managed_config_for_tests();
        let config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(codex_home.path().to_path_buf()))
            .loader_overrides(loader_overrides.clone())
            .build()
            .await?;
        let client = in_process::start(InProcessStartArgs {
            arg0_paths: Arg0DispatchPaths::default(),
            config: Arc::new(config),
            cli_overrides: Vec::new(),
            loader_overrides,
            strict_config: false,
            cloud_requirements: CloudRequirementsLoader::default(),
            thread_config_loader: Arc::new(config_service::NoopThreadConfigLoader),
            feedback: CodexFeedback::new(),
            log_db: None,
            state_db: None,
            environment_manager: Arc::new(EnvironmentManager::default_for_tests()),
            config_warnings: Vec::new(),
            session_source: SessionSource::Cli.into(),
            enable_codex_api_key_env: false,
            initialize: InitializeParams {
                client_info: ClientInfo {
                    name: "codex-app-server-tests".to_string(),
                    title: None,
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                    ..Default::default()
                }),
            },
            channel_capacity: in_process::DEFAULT_IN_PROCESS_CHANNEL_CAPACITY,
        })
        .await?;

        let read = client
            .request(ClientRequest::ThreadRead {
                request_id: RequestId::Integer(1),
                params: ThreadReadParams {
                    thread_id: thread_id.to_string(),
                    include_turns: false,
                },
            })
            .await?
            .expect("thread/read should succeed");
        let ThreadReadResponse {
            thread: read_thread,
        } = serde_json::from_value(read)?;
        assert_eq!(
            read_thread.skills,
            vec![ThreadSkill {
                name: "demo".to_string(),
                path: "/tmp/demo/SKILL.md".to_string(),
                kind: ThreadSkillKind::All,
            }]
        );

        let resume_error = client
            .request(ClientRequest::ThreadResume {
                request_id: RequestId::Integer(2),
                params: ThreadResumeParams {
                    thread_id: thread_id.to_string(),
                    ..Default::default()
                },
            })
            .await?
            .expect_err("thread/resume should reject pathless store threads");
        assert!(
            resume_error.message.contains("rollout path missing"),
            "expected missing rollout path error, got: {resume_error:?}"
        );

        client.shutdown().await?;
        Ok(())
    })
}

#[tokio::test]
async fn thread_read_can_return_archived_threads_by_id() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let preview = "Archived saved user message";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        filename_ts,
        "2025-01-05T12:00:00Z",
        preview,
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let active_rollout_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    let archived_dir = codex_home.path().join(ARCHIVED_SESSIONS_SUBDIR);
    std::fs::create_dir_all(&archived_dir)?;
    let archived_rollout_path =
        archived_dir.join(active_rollout_path.file_name().expect("rollout file name"));
    std::fs::rename(&active_rollout_path, &archived_rollout_path)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(thread.id, conversation_id);
    assert_eq!(thread.preview, preview);
    let path = thread.path.expect("thread path");
    assert_eq!(path.canonicalize()?, archived_rollout_path.canonicalize()?);

    Ok(())
}

#[tokio::test]
async fn thread_turns_list_rejects_cursor_when_anchor_turn_is_rolled_back() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let filename_ts = "2025-01-05T12-00-00";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        filename_ts,
        "2025-01-05T12:00:00Z",
        "first",
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;
    let rollout_path = rollout_path(codex_home.path(), filename_ts, &conversation_id);
    append_user_message(rollout_path.as_path(), "2025-01-05T12:01:00Z", "second")?;
    append_user_message(rollout_path.as_path(), "2025-01-05T12:02:00Z", "third")?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: conversation_id.clone(),
            cursor: None,
            limit: Some(2),
            sort_direction: Some(SortDirection::Desc),
            items_view: None,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse {
        backwards_cursor, ..
    } = to_response::<ThreadTurnsListResponse>(read_resp)?;
    let backwards_cursor = backwards_cursor.expect("expected backwardsCursor for newest turn");

    append_thread_rollback(
        rollout_path.as_path(),
        "2025-01-05T12:03:00Z",
        /*num_turns*/ 1,
    )?;

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: conversation_id,
            cursor: Some(backwards_cursor),
            limit: Some(10),
            sort_direction: Some(SortDirection::Asc),
            items_view: None,
        })
        .await?;
    let read_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(read_id)),
    )
    .await??;

    assert_eq!(
        read_err.error.message,
        "invalid cursor: anchor turn is no longer present"
    );

    Ok(())
}

#[tokio::test]
async fn thread_read_returns_forked_from_id_for_forked_threads() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        "Saved user message",
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let fork_id = mcp
        .send_thread_fork_request(ThreadForkParams {
            thread_id: conversation_id.clone(),
            ..Default::default()
        })
        .await?;
    let fork_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(fork_id)),
    )
    .await??;
    let ThreadForkResponse { thread: forked, .. } = to_response::<ThreadForkResponse>(fork_resp)?;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: forked.id,
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(thread.forked_from_id, Some(conversation_id));

    Ok(())
}

#[tokio::test]
async fn thread_read_loaded_thread_returns_precomputed_path_before_materialization() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

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
    let thread_path = thread.path.clone().expect("thread path");
    assert!(
        thread_path.exists(),
        "fresh thread rollout should be materialized at thread start"
    );

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread: read, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(read.id, thread.id);
    assert_eq!(read.path, Some(thread_path));
    assert!(read.preview.is_empty());
    assert_eq!(read.turns.len(), 0);
    assert_eq!(read.lifecycle_status, ThreadLifecycleStatus::completed(None));

    Ok(())
}

#[tokio::test]
async fn thread_name_set_is_reflected_in_read_list_and_resume() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let preview = "Saved user message";
    let conversation_id = create_fake_rollout_with_text_elements(
        codex_home.path(),
        "2025-01-05T12-00-00",
        "2025-01-05T12:00:00Z",
        preview,
        vec![],
        Some("mock_provider"),
        /*git_info*/ None,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // Set a user-facing thread title.
    let new_name = "My renamed thread";
    let set_id = mcp
        .send_thread_set_name_request(ThreadSetNameParams {
            thread_id: conversation_id.clone(),
            name: new_name.to_string(),
        })
        .await?;
    let set_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(set_id)),
    )
    .await??;
    let _: ThreadSetNameResponse = to_response::<ThreadSetNameResponse>(set_resp)?;
    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/name/updated"),
    )
    .await??;
    let notification: ThreadNameUpdatedNotification =
        serde_json::from_value(notification.params.expect("thread/name/updated params"))?;
    assert_eq!(notification.thread_id, conversation_id);
    assert_eq!(notification.thread_name.as_deref(), Some(new_name));

    // Read should now surface `thread.name`, and the wire payload must include `name`.
    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: conversation_id.clone(),
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let read_result = read_resp.result.clone();
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;
    assert_eq!(thread.id, conversation_id);
    assert_eq!(thread.name.as_deref(), Some(new_name));
    let thread_json = read_result
        .get("thread")
        .and_then(Value::as_object)
        .expect("thread/read result.thread must be an object");
    assert_eq!(
        thread_json.get("name").and_then(Value::as_str),
        Some(new_name),
        "thread/read must serialize `thread.name` on the wire"
    );
    assert_eq!(
        thread_json.get("ephemeral").and_then(Value::as_bool),
        Some(false),
        "thread/read must serialize `thread.ephemeral` on the wire"
    );

    // List should also surface the name.
    let list_id = mcp
        .send_thread_list_request(ThreadListParams {
            cursor: None,
            limit: Some(50),
            sort_key: None,
            sort_direction: None,
            model_providers: Some(vec!["mock_provider".to_string()]),
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
    let list_result = list_resp.result.clone();
    let ThreadListResponse { data, .. } = to_response::<ThreadListResponse>(list_resp)?;
    let listed = data
        .iter()
        .find(|t| t.id == conversation_id)
        .expect("thread/list should include the created thread");
    assert_eq!(listed.name.as_deref(), Some(new_name));
    let listed_json = list_result
        .get("data")
        .and_then(Value::as_array)
        .expect("thread/list result.data must be an array")
        .iter()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(&conversation_id))
        .and_then(Value::as_object)
        .expect("thread/list should include the created thread as an object");
    assert_eq!(
        listed_json.get("name").and_then(Value::as_str),
        Some(new_name),
        "thread/list must serialize `thread.name` on the wire"
    );
    assert_eq!(
        listed_json.get("ephemeral").and_then(Value::as_bool),
        Some(false),
        "thread/list must serialize `thread.ephemeral` on the wire"
    );

    // Resume should also surface the name.
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
    let resume_result = resume_resp.result.clone();
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed.id, conversation_id);
    assert_eq!(resumed.name.as_deref(), Some(new_name));
    let resumed_json = resume_result
        .get("thread")
        .and_then(Value::as_object)
        .expect("thread/resume result.thread must be an object");
    assert_eq!(
        resumed_json.get("name").and_then(Value::as_str),
        Some(new_name),
        "thread/resume must serialize `thread.name` on the wire"
    );
    assert_eq!(
        resumed_json.get("ephemeral").and_then(Value::as_bool),
        Some(false),
        "thread/resume must serialize `thread.ephemeral` on the wire"
    );

    Ok(())
}

#[tokio::test]
async fn thread_read_include_turns_omits_initial_context_for_fresh_loaded_thread() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

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
    let thread_path = thread.path.clone().expect("thread path");
    assert!(
        thread_path.exists(),
        "fresh thread rollout should be materialized at thread start"
    );

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id.clone(),
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert!(
        thread.turns.is_empty(),
        "fresh loaded thread/read should not expose init-only display turns"
    );

    Ok(())
}

#[tokio::test]
async fn thread_read_after_auto_compaction_preserves_init_context_without_dup_live_assistant_items(
) -> Result<()> {
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
        /*auto_compact_token_limit*/ 1_000,
        /*requires_openai_auth*/ None,
        "mock_provider",
        "Summarize the conversation.",
    )?;

    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    let instruction_dir = workspace.path().join("memory");
    std::fs::create_dir_all(&instruction_dir)?;
    std::fs::write(
        instruction_dir.join("project-understanding.md"),
        "# Project Understanding\nPersisted init context should survive compaction.",
    )?;
    std::fs::write(
        instruction_dir.join("user-preferences.md"),
        "# User Preferences\nNever duplicate live assistant output after restoring Init Context.",
    )?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
instruction_files = [
  "memory/user-preferences.md",
  "memory/project-understanding.md",
]
"#,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let start_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            environments: Some(Vec::new()),
            ..Default::default()
        })
        .await?;
    let start_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(start_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(start_resp)?;

    for message in ["first", "second", "third"] {
        send_turn_and_wait_for_thread_read(&mut mcp, &thread.id, message).await?;
    }
    wait_for_context_compaction_started_for_thread_read(&mut mcp).await?;
    wait_for_context_compaction_completed_for_thread_read(&mut mcp).await?;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id,
            include_turns: true,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert!(
        thread
            .turns
            .iter()
            .flat_map(|turn| turn.items.iter())
            .any(|item| matches!(item, ThreadItem::InjectedContext { .. })),
        "thread/read should preserve an injected init context item after compaction, got {:?}",
        thread_visible_texts(&thread)
    );

    let final_reply_count = thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| matches!(item, ThreadItem::AgentMessage { text, .. } if text == "FINAL_REPLY"))
        .count();
    assert_eq!(final_reply_count, 1);

    Ok(())
}

#[tokio::test]
async fn thread_turns_list_omits_initial_context_for_fresh_loaded_thread() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

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
    let thread_path = thread.path.clone().expect("thread path");
    assert!(
        thread_path.exists(),
        "fresh thread rollout should be materialized at thread start"
    );

    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: thread.id,
            cursor: None,
            limit: None,
            sort_direction: None,
            items_view: None,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse { data, .. } = to_response::<ThreadTurnsListResponse>(read_resp)?;

    assert!(
        data.is_empty(),
        "fresh loaded thread/turns/list should not expose init-only display turns"
    );

    Ok(())
}

#[tokio::test]
async fn thread_turns_items_list_returns_unsupported() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let read_id = mcp
        .send_thread_turns_items_list_request(ThreadTurnsItemsListParams {
            thread_id: "thr_123".to_string(),
            turn_id: "turn_456".to_string(),
            cursor: None,
            limit: None,
            sort_direction: None,
        })
        .await?;
    let read_err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(read_id)),
    )
    .await??;

    assert_eq!(read_err.error.code, -32601);
    assert_eq!(
        read_err.error.message,
        "thread/turns/items/list is not supported yet"
    );

    Ok(())
}

#[tokio::test]
async fn thread_read_reports_system_error_idle_flag_after_failed_turn() -> Result<()> {
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

    let turn_start_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "fail this turn".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_start_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response::<TurnStartResponse>(turn_start_response)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("error"),
    )
    .await??;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id,
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(thread.lifecycle_status, ThreadLifecycleStatus::system_error(None),);

    Ok(())
}

#[tokio::test]
async fn thread_read_without_turns_reports_active_loaded_turn() -> Result<()> {
    let (complete_turn_tx, complete_turn_rx) = tokio::sync::oneshot::channel();
    let (server, mut completions) = start_streaming_sse_server(vec![vec![
        StreamingSseChunk {
            gate: None,
            body: responses::sse(vec![responses::ev_response_created("resp-1")]),
        },
        StreamingSseChunk {
            gate: Some(complete_turn_rx),
            body: responses::sse(vec![responses::ev_completed("resp-1")]),
        },
    ]])
    .await;
    let response_completed = completions.remove(0);
    let codex_home = TempDir::new()?;
    create_config_toml(codex_home.path(), server.uri())?;

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

    let turn_start_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "keep this turn running".to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_start_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_start_id)),
    )
    .await??;
    let _: TurnStartResponse = to_response::<TurnStartResponse>(turn_start_response)?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await??;

    let read_id = mcp
        .send_thread_read_request(ThreadReadParams {
            thread_id: thread.id,
            include_turns: false,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadReadResponse { thread, .. } = to_response::<ThreadReadResponse>(read_resp)?;

    assert_eq!(
        thread.lifecycle_status,
        ThreadLifecycleStatus::Active {
            active_flags: vec![ThreadLifecycleActiveFlag::Running],
        }
    );
    assert_eq!(thread.turns, Vec::new());

    let _ = complete_turn_tx.send(());
    timeout(DEFAULT_READ_TIMEOUT, response_completed).await??;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    server.shutdown().await;

    Ok(())
}

fn append_user_message(path: &Path, timestamp: &str, text: &str) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": timestamp,
            "type":"event_msg",
            "payload": {
                "type":"user_message",
                "message": text,
                "text_elements": [],
                "local_images": []
            }
        })
    )
}

fn append_agent_message(path: &Path, timestamp: &str, text: &str) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": serde_json::to_value(EventMsg::AgentMessage(AgentMessageEvent {
                message: text.to_string(),
                phase: None,
                memory_citation: None,
            }))?,
        })
    )?;
    Ok(())
}

fn append_thread_rollback(path: &Path, timestamp: &str, num_turns: u32) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": timestamp,
            "type":"event_msg",
            "payload": {
                "type":"thread_rolled_back",
                "num_turns": num_turns
            }
        })
    )
}

async fn read_single_turn_items_view(
    mcp: &mut McpProcess,
    thread_id: &str,
    items_view: Option<TurnItemsView>,
) -> anyhow::Result<app_server_protocol::Turn> {
    let read_id = mcp
        .send_thread_turns_list_request(ThreadTurnsListParams {
            thread_id: thread_id.to_string(),
            cursor: None,
            limit: Some(10),
            sort_direction: Some(SortDirection::Asc),
            items_view,
        })
        .await?;
    let read_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(read_id)),
    )
    .await??;
    let ThreadTurnsListResponse { mut data, .. } =
        to_response::<ThreadTurnsListResponse>(read_resp)?;
    assert_eq!(data.len(), 1);
    Ok(data.remove(0))
}

fn turn_user_texts(turns: &[app_server_protocol::Turn]) -> Vec<&str> {
    turns
        .iter()
        .filter_map(|turn| match turn.items.first()? {
            ThreadItem::UserMessage { content, .. } => match content.first()? {
                UserInput::Text { text, .. } => Some(text.as_str()),
                UserInput::Image { .. }
                | UserInput::LocalImage { .. }
                | UserInput::Skill { .. }
                | UserInput::Mention { .. } => None,
            },
            _ => None,
        })
        .collect()
}

fn turn_agent_texts(turns: &[app_server_protocol::Turn]) -> Vec<&str> {
    turns
        .iter()
        .flat_map(|turn| &turn.items)
        .filter_map(|item| match item {
            ThreadItem::AgentMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

struct InMemoryThreadStoreId {
    store_id: String,
}

impl Drop for InMemoryThreadStoreId {
    fn drop(&mut self) {
        InMemoryThreadStore::remove_id(&self.store_id);
    }
}

async fn seed_pathless_store_thread(
    store: &InMemoryThreadStore,
    thread_id: protocol::ThreadId,
) -> Result<()> {
    store
        .create_thread(CreateThreadParams {
            thread_id,
            forked_from_id: None,
            source: ProtocolSessionSource::Cli,
            thread_source: None,
            base_instructions: BaseInstructions::default(),
            dynamic_tools: Vec::new(),
            metadata: ThreadPersistenceMetadata {
                cwd: None,
                model_provider: "test-provider".to_string(),
                memory_mode: ThreadMemoryMode::Disabled,
                root_agent_role: None,
                root_agent_path: None,
            },
            event_persistence_mode: ThreadEventPersistenceMode::default(),
        })
        .await?;
    store
        .append_items(AppendThreadItemsParams {
            thread_id,
            items: store_history_items(),
        })
        .await?;
    store
        .update_thread_metadata(UpdateThreadMetadataParams {
            thread_id,
            patch: ThreadMetadataPatch {
                name: Some(Some("named pathless thread".to_string())),
                ..Default::default()
            },
            include_archived: true,
        })
        .await?;
    Ok(())
}

async fn seed_external_root_store_thread(
    store: &InMemoryThreadStore,
    thread_id: protocol::ThreadId,
    cwd: &Path,
    mut items: Vec<RolloutItem>,
) -> Result<()> {
    store
        .create_thread(CreateThreadParams {
            thread_id,
            forked_from_id: None,
            source: ProtocolSessionSource::VSCode,
            thread_source: Some(protocol::protocol::ThreadSource::User),
            base_instructions: BaseInstructions::default(),
            dynamic_tools: Vec::new(),
            metadata: ThreadPersistenceMetadata {
                cwd: Some(cwd.to_path_buf()),
                model_provider: "claude_cli".to_string(),
                memory_mode: ThreadMemoryMode::Disabled,
                root_agent_role: None,
                root_agent_path: None,
            },
            event_persistence_mode: ThreadEventPersistenceMode::default(),
        })
        .await?;
    store
        .update_thread_metadata(UpdateThreadMetadataParams {
            thread_id,
            patch: ThreadMetadataPatch {
                model_provider: Some("claude_cli".to_string()),
                source: Some(ProtocolSessionSource::VSCode),
                thread_source: Some(Some(protocol::protocol::ThreadSource::User)),
                cwd: Some(cwd.to_path_buf()),
                ..Default::default()
            },
            include_archived: true,
        })
        .await?;
    let mut history = store_history_items();
    history.append(&mut items);
    store
        .append_items(AppendThreadItemsParams {
            thread_id,
            items: history,
        })
        .await?;
    Ok(())
}

fn store_history_items() -> Vec<RolloutItem> {
    vec![RolloutItem::EventMsg(EventMsg::UserMessage(
        UserMessageEvent {
            message: "history from store".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        },
    ))]
}

fn create_config_toml_with_thread_store(codex_home: &Path, store_id: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
experimental_thread_store = {{ type = "in_memory", id = "{store_id}" }}

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "http://127.0.0.1:1/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
}

// Helper to create a config.toml pointing at the mock model server.
fn create_config_toml(codex_home: &Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"

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

async fn send_turn_and_wait_for_thread_read(
    mcp: &mut McpProcess,
    thread_id: &str,
    text: &str,
) -> Result<String> {
    let turn_request_id = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread_id.to_string(),
            input: vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_request_id)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;
    wait_for_turn_completed_for_thread_read(mcp, &turn.id).await?;
    Ok(turn.id)
}

async fn wait_for_turn_completed_for_thread_read(mcp: &mut McpProcess, turn_id: &str) -> Result<()> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("turn/completed"),
        )
        .await??;
        let completed: TurnCompletedNotification =
            serde_json::from_value(notification.params.ok_or_else(|| {
                anyhow::anyhow!("turn/completed params missing")
            })?)?;
        if completed.turn.id == turn_id {
            return Ok(());
        }
    }
}

async fn wait_for_context_compaction_started_for_thread_read(
    mcp: &mut McpProcess,
) -> Result<ItemStartedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/started"),
        )
        .await??;
        let started: ItemStartedNotification =
            serde_json::from_value(notification.params.ok_or_else(|| {
                anyhow::anyhow!("item/started params missing")
            })?)?;
        if let ThreadItem::ContextCompaction { .. } = started.item {
            return Ok(started);
        }
    }
}

async fn wait_for_context_compaction_completed_for_thread_read(
    mcp: &mut McpProcess,
) -> Result<ItemCompletedNotification> {
    loop {
        let notification: JSONRPCNotification = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_notification_message("item/completed"),
        )
        .await??;
        let completed: ItemCompletedNotification =
            serde_json::from_value(notification.params.ok_or_else(|| {
                anyhow::anyhow!("item/completed params missing")
            })?)?;
        if let ThreadItem::ContextCompaction { .. } = completed.item {
            return Ok(completed);
        }
    }
}

fn thread_visible_texts(thread: &app_server_protocol::Thread) -> Vec<String> {
    thread
        .turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .flat_map(|item| match item {
            ThreadItem::InjectedContext { sections, .. } => sections
                .iter()
                .map(|section| section.text.clone())
                .collect::<Vec<_>>(),
            ThreadItem::UserMessage { content, .. } => content
                .iter()
                .filter_map(|input| match input {
                    UserInput::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            ThreadItem::AgentMessage { text, .. } => vec![text.clone()],
            _ => Vec::new(),
        })
        .collect()
}
