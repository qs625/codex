use anyhow::Result;
use app_server_protocol::AskForApproval;
use app_server_protocol::DeprecationNoticeNotification;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCMessage;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::McpServerStartupState;
use app_server_protocol::McpServerStatusUpdatedNotification;
use app_server_protocol::RequestId;
use app_server_protocol::SandboxMode;
use app_server_protocol::ServerNotification;
use app_server_protocol::ThreadItem;
use app_server_protocol::ThreadLifecycleStatus;
use app_server_protocol::ThreadListParams;
use app_server_protocol::ThreadListResponse;
use app_server_protocol::ThreadResumeParams;
use app_server_protocol::ThreadResumeResponse;
use app_server_protocol::ThreadSource;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::ThreadStartedNotification;
use app_server_protocol::ThreadStatusChangedNotification;
use app_server_protocol::Turn;
use app_server_protocol::TurnEnvironmentParams;
use app_server_protocol::TurnStatus;
use app_test_support::ChatGptAuthFixture;
use app_test_support::McpProcess;
use app_test_support::PathBufExt;
use app_test_support::create_mock_responses_server_repeating_assistant;
use app_test_support::to_response;
use app_test_support::write_chatgpt_auth;
use codex_exec_server::LOCAL_FS;
use codex_git_info::resolve_root_git_project_for_trust;
use codex_login::REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR;
use config_service::loader::project_trust_key;
use config_service::types::AuthCredentialsStoreMode;
use pretty_assertions::assert_eq;
use protocol::config_types::TrustLevel;
use protocol::openai_models::ReasoningEffort;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;
use thread_service::config::set_project_trust_level;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

use super::analytics::assert_basic_thread_initialized_event;
use super::analytics::mount_analytics_capture;
use super::analytics::thread_initialized_event;
use super::analytics::wait_for_analytics_payload;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const INVALID_REQUEST_ERROR_CODE: i64 = -32600;
const STARTUP_NOTIFICATION_QUIET_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(200);

fn write_workflow(root: &Path, id: &str, description: &str) -> Result<()> {
    let workflow_dir = root.join(id);
    std::fs::create_dir_all(&workflow_dir)?;
    std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")?;
    std::fs::write(
        workflow_dir.join("WORKFLOW.md"),
        format!(
            r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
        ),
    )?;
    Ok(())
}

async fn assert_no_startup_injected_context_replay(
    mcp: &mut McpProcess,
    thread_id: &str,
) -> Result<()> {
    loop {
        let message =
            match timeout(STARTUP_NOTIFICATION_QUIET_TIMEOUT, mcp.read_next_message()).await {
                Ok(result) => result?,
                Err(_) => return Ok(()),
            };
        if is_injected_context_item_completed_for_thread(&message, thread_id) {
            anyhow::bail!("thread/start should not replay Init Context as item/completed");
        }
    }
}

fn is_injected_context_item_completed_for_thread(
    message: &JSONRPCMessage,
    thread_id: &str,
) -> bool {
    let JSONRPCMessage::Notification(notification) = message else {
        return false;
    };
    if notification.method != "item/completed" {
        return false;
    }
    let Some(params) = notification.params.as_ref() else {
        return false;
    };
    params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(Value::as_str)
            == Some("injectedContext")
}

fn assert_single_completed_init_context_turn(turns: &[Turn], context: &str) {
    let mut injected_context_count = 0;
    let mut init_context_text = String::new();
    for turn in turns {
        for item in &turn.items {
            if let ThreadItem::InjectedContext {
                title, sections, ..
            } = item
            {
                injected_context_count += 1;
                assert_eq!(title, "Init Context", "{context}");
                assert_eq!(turn.status, TurnStatus::Completed, "{context}");
                for section in sections {
                    init_context_text.push_str(&section.text);
                    init_context_text.push('\n');
                }
            }
        }
    }
    assert_eq!(
        injected_context_count, 1,
        "{context} should include exactly one Init Context display item"
    );
    for expected in [
        "<external_agent_tools>",
        "独立的外部 CLI agent 协作总线",
        "模型 API tool config",
        "spawn_external_agent",
        "followup_external_task",
        "poll_external_event",
        "list_external_agents",
        "close_external_agent",
        "\"parameters\"",
        "\"provider\"",
    ] {
        assert!(
            init_context_text.contains(expected),
            "{context} should include external agent tool spec text {expected}, got {init_context_text}"
        );
    }
    for unexpected in [
        "<model_visible_tools>",
        "\"name\": \"exec_command\"",
        "\"name\": \"apply_patch\"",
        "\"name\": \"spawn_agent\"",
        "\"name\": \"followup_task\"",
        "\"name\": \"poll_event\"",
    ] {
        assert!(
            !init_context_text.contains(unexpected),
            "{context} should not include native/model-visible tool spec text {unexpected}, got {init_context_text}"
        );
    }
}

#[tokio::test]
async fn thread_start_deprecates_persist_extended_history_true() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            persist_extended_history: true,
            ..Default::default()
        })
        .await?;

    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("deprecationNotice"),
    )
    .await??;
    let notice: DeprecationNoticeNotification = serde_json::from_value(
        notification
            .params
            .expect("deprecationNotice params should be present"),
    )?;
    assert_eq!(
        notice.summary,
        "persistExtendedHistory is deprecated and ignored"
    );

    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;

    Ok(())
}

#[tokio::test]
async fn thread_start_accepts_native_thread_provider_and_rejects_external_provider() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let native_req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            thread_provider: Some("native".to_string()),
            ..Default::default()
        })
        .await?;
    let native_response = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(native_req_id)),
    )
    .await??;
    let _native_started: ThreadStartResponse = to_response(native_response)?;

    for external_provider in ["claude_cli", "opencode", "codex_cli"] {
        let external_req_id = mcp
            .send_thread_start_request(ThreadStartParams {
                thread_provider: Some(external_provider.to_string()),
                ..Default::default()
            })
            .await?;
        let external_error = timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_error_message(RequestId::Integer(external_req_id)),
        )
        .await??;
        assert_eq!(external_error.error.code, INVALID_REQUEST_ERROR_CODE);
        assert!(
            external_error
                .error
                .message
                .contains("does not support thread/start yet")
        );
    }

    let unknown_req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            thread_provider: Some("unknown_provider".to_string()),
            ..Default::default()
        })
        .await?;
    let unknown_error = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(unknown_req_id)),
    )
    .await??;
    assert_eq!(unknown_error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert!(
        unknown_error
            .error
            .message
            .contains("unknown thread provider 'unknown_provider' for thread/start")
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_creates_thread_and_emits_started() -> Result<()> {
    // Provide a mock server and config so model wiring is valid.
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    // Start server and initialize.
    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    // Start a thread with an explicit model override.
    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.2".to_string()),
            developer_instructions: Some(
                "Agent type file body: always inspect the active task.".to_string(),
            ),
            thread_source: Some(ThreadSource::User),
            ..Default::default()
        })
        .await?;

    // Expect a proper JSON-RPC response with a thread id.
    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let resp_result = resp.result.clone();
    let ThreadStartResponse {
        thread,
        model_provider,
        ..
    } = to_response::<ThreadStartResponse>(resp)?;
    assert!(
        !thread.session_id.is_empty(),
        "session id should not be empty"
    );
    assert!(!thread.id.is_empty(), "thread id should not be empty");
    assert!(
        thread.preview.is_empty(),
        "new threads should start with an empty preview"
    );
    assert_eq!(model_provider, "mock_provider");
    assert!(
        thread.created_at > 0,
        "created_at should be a positive UNIX timestamp"
    );
    assert!(
        !thread.ephemeral,
        "new persistent threads should not be ephemeral"
    );
    assert_eq!(
        thread.lifecycle_status,
        ThreadLifecycleStatus::completed(None)
    );
    assert_eq!(thread.thread_source, Some(ThreadSource::User));
    assert_single_completed_init_context_turn(
        &thread.turns,
        "thread/start response should include initial context display turns",
    );
    let thread_path = thread.path.clone().expect("thread path should be present");
    assert!(thread_path.is_absolute(), "thread path should be absolute");
    assert!(
        thread_path.exists(),
        "fresh thread rollout should be materialized at thread start"
    );

    // Wire contract: thread title field is `name`, serialized as null when unset.
    let thread_json = resp_result
        .get("thread")
        .and_then(Value::as_object)
        .expect("thread/start result.thread must be an object");
    assert_eq!(
        thread_json.get("sessionId").and_then(Value::as_str),
        Some(thread.session_id.as_str()),
        "new threads should serialize `sessionId` on the thread object"
    );
    assert_eq!(
        thread_json.get("name"),
        Some(&Value::Null),
        "new threads should serialize `name: null`"
    );
    assert_eq!(
        resp_result.get("sessionId"),
        None,
        "thread/start should not serialize a top-level `sessionId`"
    );
    assert_eq!(
        thread_json.get("ephemeral").and_then(Value::as_bool),
        Some(false),
        "new persistent threads should serialize `ephemeral: false`"
    );
    assert_eq!(
        thread_json.get("threadSource").and_then(Value::as_str),
        Some("user"),
        "new threads should serialize the caller-supplied thread origin"
    );
    assert_eq!(thread.name, None);

    // A corresponding thread/started notification should arrive.
    let deadline = tokio::time::Instant::now() + DEFAULT_READ_TIMEOUT;
    let notif = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = timeout(remaining, mcp.read_next_message()).await??;
        let JSONRPCMessage::Notification(notif) = message else {
            continue;
        };
        if notif.method == "thread/status/changed" {
            let status_changed: ThreadStatusChangedNotification =
                serde_json::from_value(notif.params.expect("params must be present"))?;
            if status_changed.thread_id == thread.id {
                anyhow::bail!(
                    "thread/start should introduce the thread without a preceding thread/status/changed"
                );
            }
            continue;
        }
        if notif.method == "thread/started" {
            break notif;
        }
    };
    let started_params = notif.params.clone().expect("params must be present");
    let started_thread_json = started_params
        .get("thread")
        .and_then(Value::as_object)
        .expect("thread/started params.thread must be an object");
    assert_eq!(
        started_thread_json.get("name"),
        Some(&Value::Null),
        "thread/started should serialize `name: null` for new threads"
    );
    assert_eq!(
        started_thread_json
            .get("ephemeral")
            .and_then(Value::as_bool),
        Some(false),
        "thread/started should serialize `ephemeral: false` for new persistent threads"
    );
    assert_eq!(
        started_thread_json
            .get("threadSource")
            .and_then(Value::as_str),
        Some("user"),
        "thread/started should preserve the caller-supplied thread origin"
    );
    let started: ThreadStartedNotification =
        serde_json::from_value(notif.params.expect("params must be present"))?;
    assert_eq!(started.thread, thread);
    assert_no_startup_injected_context_replay(&mut mcp, &thread.id).await?;

    Ok(())
}

#[tokio::test]
async fn thread_start_preserves_client_supplied_root_agent_path() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            task_name: Some("/owner_dev".to_string()),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;
    assert_eq!(thread.agent_path.as_deref(), Some("/owner_dev"));

    let notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/started"),
    )
    .await??;
    let started: ThreadStartedNotification = serde_json::from_value(
        notification
            .params
            .expect("thread/started params should be present"),
    )?;
    assert_eq!(started.thread.id, thread.id);
    assert_eq!(started.thread.agent_path.as_deref(), Some("/owner_dev"));

    let list_req_id = mcp
        .send_thread_list_request(ThreadListParams {
            cursor: None,
            limit: None,
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
        mcp.read_stream_until_response_message(RequestId::Integer(list_req_id)),
    )
    .await??;
    let listed = to_response::<ThreadListResponse>(list_resp)?
        .data
        .into_iter()
        .find(|listed| listed.id == thread.id)
        .expect("created thread should appear in thread/list");
    assert_eq!(listed.agent_path.as_deref(), Some("/owner_dev"));
    drop(mcp);

    let mut reloaded_mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, reloaded_mcp.initialize()).await??;

    let resume_req_id = reloaded_mcp
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: thread.id.clone(),
            ..Default::default()
        })
        .await?;
    let resume_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        reloaded_mcp.read_stream_until_response_message(RequestId::Integer(resume_req_id)),
    )
    .await??;
    let ThreadResumeResponse {
        thread: resumed, ..
    } = to_response::<ThreadResumeResponse>(resume_resp)?;
    assert_eq!(resumed.agent_path.as_deref(), Some("/owner_dev"));

    let resumed_list_req_id = reloaded_mcp
        .send_thread_list_request(ThreadListParams {
            cursor: None,
            limit: None,
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
    let resumed_list_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        reloaded_mcp.read_stream_until_response_message(RequestId::Integer(resumed_list_req_id)),
    )
    .await??;
    let resumed_listed = to_response::<ThreadListResponse>(resumed_list_resp)?
        .data
        .into_iter()
        .find(|listed| listed.id == thread.id)
        .expect("resumed thread should appear in thread/list after restart");
    assert_eq!(resumed_listed.agent_path.as_deref(), Some("/owner_dev"));

    Ok(())
}

#[tokio::test]
async fn thread_start_resolves_runtime_workspace_roots_against_cwd() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let cwd_tmp = TempDir::new()?;
    let cwd = cwd_tmp.path().to_path_buf();
    let relative_root = PathBuf::from("extra-root");
    std::fs::create_dir_all(cwd.join(&relative_root))?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(cwd.to_string_lossy().to_string()),
            runtime_workspace_roots: Some(vec![relative_root.clone()]),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse {
        cwd: response_cwd,
        runtime_workspace_roots,
        ..
    } = to_response::<ThreadStartResponse>(resp)?;

    assert_eq!(response_cwd, cwd.abs());
    assert_eq!(
        runtime_workspace_roots,
        vec![cwd_tmp.path().join(relative_root).abs()]
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_excludes_profile_workspace_roots_from_runtime_workspace_roots() -> Result<()>
{
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let profile_root = TempDir::new()?;
    create_config_toml_with_profile_workspace_root(
        codex_home.path(),
        &server.uri(),
        profile_root.path(),
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(cwd.path().to_string_lossy().to_string()),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse {
        runtime_workspace_roots,
        ..
    } = to_response::<ThreadStartResponse>(resp)?;

    assert_eq!(
        runtime_workspace_roots,
        vec![cwd.path().to_path_buf().abs()]
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_rejects_unknown_environment_as_invalid_request() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            environments: Some(vec![TurnEnvironmentParams {
                environment_id: "missing".to_string(),
                cwd: codex_home.path().to_path_buf().try_into()?,
            }]),
            ..Default::default()
        })
        .await?;

    let error: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;

    assert_eq!(error.id, RequestId::Integer(request_id));
    assert_eq!(error.error.code, INVALID_REQUEST_ERROR_CODE);
    assert_eq!(error.error.message, "unknown turn environment id `missing`");

    Ok(())
}

#[tokio::test]
async fn thread_start_response_includes_loaded_instruction_sources() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;
    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    let instruction_dir = workspace.path().join("memory");
    std::fs::create_dir_all(&instruction_dir)?;
    let project_instruction_path = instruction_dir.join("project-understanding.md");
    let user_instruction_path = instruction_dir.join("user-preferences.md");
    std::fs::write(&project_instruction_path, "project instructions")?;
    std::fs::write(&user_instruction_path, "user instructions")?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
instruction_files = [
  "memory/project-understanding.md",
  "memory/user-preferences.md",
]
"#,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            ..Default::default()
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let ThreadStartResponse {
        instruction_sources,
        ..
    } = to_response::<ThreadStartResponse>(response)?;

    let instruction_sources = instruction_sources
        .into_iter()
        .map(normalize_path_for_comparison)
        .collect::<Vec<_>>();
    let expected_instruction_sources = vec![
        std::fs::canonicalize(project_instruction_path)?,
        std::fs::canonicalize(user_instruction_path)?,
    ]
    .into_iter()
    .map(normalize_path_for_comparison)
    .collect::<Vec<_>>();

    assert_eq!(instruction_sources, expected_instruction_sources);

    Ok(())
}

#[tokio::test]
async fn thread_start_with_project_context_displays_initial_context() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;
    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(project_config_dir.join("workflows"))?;
    let instruction_dir = workspace.path().join("memory");
    std::fs::create_dir_all(&instruction_dir)?;
    let project_instruction_path = instruction_dir.join("project-understanding.md");
    let user_instruction_path = instruction_dir.join("user-preferences.md");
    std::fs::write(
        &project_instruction_path,
        "Project understanding: payment API, cache invalidation, and release checklist.",
    )?;
    std::fs::write(
        &user_instruction_path,
        "User preference: keep migrations separate from behavior changes.",
    )?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
instruction_files = [
  "memory/project-understanding.md",
  "memory/user-preferences.md",
]
"#,
    )?;
    write_workflow(
        &project_config_dir.join("workflows"),
        "feature-dev",
        "structured feature workflow",
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            environments: Some(Vec::new()),
            ..Default::default()
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(response)?;

    assert_single_completed_init_context_turn(
        &thread.turns,
        "project thread/start response should include initial context display turns",
    );
    let started = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("thread/started"),
    )
    .await??;
    let started: ThreadStartedNotification =
        serde_json::from_value(started.params.expect("params must be present"))?;
    assert_eq!(started.thread.id, thread.id);
    assert_single_completed_init_context_turn(
        &started.thread.turns,
        "project thread/started notification should include initial context display turns",
    );
    assert_no_startup_injected_context_replay(&mut mcp, &thread.id).await?;

    Ok(())
}

#[cfg(windows)]
fn normalize_path_for_comparison(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let path = path.display().to_string();
    PathBuf::from(path.strip_prefix(r"\\?\").unwrap_or(&path))
}

#[cfg(not(windows))]
fn normalize_path_for_comparison(path: impl AsRef<Path>) -> PathBuf {
    path.as_ref().to_path_buf()
}

#[tokio::test]
async fn thread_start_tracks_thread_initialized_analytics() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_with_chatgpt_base_url(codex_home.path(), &server.uri(), &server.uri())?;
    mount_analytics_capture(&server, codex_home.path()).await?;

    let mut mcp = McpProcess::new_without_managed_config(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            thread_source: Some(ThreadSource::User),
            ..Default::default()
        })
        .await?;
    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;

    let payload = wait_for_analytics_payload(&server, DEFAULT_READ_TIMEOUT).await?;
    assert_eq!(payload["events"].as_array().expect("events array").len(), 1);
    let event = thread_initialized_event(&payload)?;
    assert_basic_thread_initialized_event(event, &thread.id, "mock-model", "new", "user");
    Ok(())
}

#[tokio::test]
async fn thread_start_respects_project_config_from_cwd() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
model_reasoning_effort = "high"
"#,
    )?;
    set_project_trust_level(codex_home.path(), workspace.path(), TrustLevel::Trusted)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().to_string_lossy().into_owned()),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse {
        reasoning_effort, ..
    } = to_response::<ThreadStartResponse>(resp)?;

    assert_eq!(reasoning_effort, Some(ReasoningEffort::High));
    Ok(())
}

#[tokio::test]
async fn thread_start_accepts_arbitrary_service_tier_id() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let service_tier_id = "experimental-tier-id".to_string();
    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            service_tier: Some(Some(service_tier_id.clone())),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { service_tier, .. } = to_response::<ThreadStartResponse>(resp)?;

    assert_eq!(service_tier, Some(service_tier_id));
    Ok(())
}

#[tokio::test]
async fn thread_start_accepts_metrics_service_name() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            service_name: Some("my_app_server_client".to_string()),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;
    assert!(!thread.id.is_empty(), "thread id should not be empty");

    Ok(())
}

#[tokio::test]
async fn thread_start_ephemeral_remains_pathless() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;
    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("gpt-5.2".to_string()),
            ephemeral: Some(true),
            ..Default::default()
        })
        .await?;

    let resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
    )
    .await??;
    let resp_result = resp.result.clone();
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(resp)?;
    assert!(
        thread.ephemeral,
        "ephemeral threads should be marked explicitly"
    );
    assert_eq!(
        thread.path, None,
        "ephemeral threads should not expose a path"
    );
    let thread_json = resp_result
        .get("thread")
        .and_then(Value::as_object)
        .expect("thread/start result.thread must be an object");
    assert_eq!(
        thread_json.get("ephemeral").and_then(Value::as_bool),
        Some(true),
        "ephemeral threads should serialize `ephemeral: true`"
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_fails_when_required_mcp_server_fails_to_initialize() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_with_required_broken_mcp(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams::default())
        .await?;

    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(req_id)),
    )
    .await??;

    assert!(
        err.error
            .message
            .contains("required MCP servers failed to initialize"),
        "unexpected error message: {}",
        err.error.message
    );
    assert!(
        err.error.message.contains("required_broken"),
        "unexpected error message: {}",
        err.error.message
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_emits_mcp_server_status_updated_notifications() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_with_optional_broken_mcp(codex_home.path(), &server.uri())?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams::default())
        .await?;

    let _: ThreadStartResponse = to_response(
        timeout(
            DEFAULT_READ_TIMEOUT,
            mcp.read_stream_until_response_message(RequestId::Integer(req_id)),
        )
        .await??,
    )?;

    let starting = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_matching_notification(
            "mcpServer/startupStatus/updated starting",
            |notification| {
                notification.method == "mcpServer/startupStatus/updated"
                    && notification
                        .params
                        .as_ref()
                        .and_then(|params| params.get("name"))
                        .and_then(Value::as_str)
                        == Some("optional_broken")
                    && notification
                        .params
                        .as_ref()
                        .and_then(|params| params.get("status"))
                        .and_then(Value::as_str)
                        == Some("starting")
            },
        ),
    )
    .await??;
    let starting: ServerNotification = starting.try_into()?;
    let ServerNotification::McpServerStatusUpdated(starting) = starting else {
        anyhow::bail!("unexpected notification variant");
    };
    assert_eq!(
        starting,
        McpServerStatusUpdatedNotification {
            name: "optional_broken".to_string(),
            status: McpServerStartupState::Starting,
            error: None,
        }
    );

    let failed = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_matching_notification(
            "mcpServer/startupStatus/updated failed",
            |notification| {
                notification.method == "mcpServer/startupStatus/updated"
                    && notification
                        .params
                        .as_ref()
                        .and_then(|params| params.get("name"))
                        .and_then(Value::as_str)
                        == Some("optional_broken")
                    && notification
                        .params
                        .as_ref()
                        .and_then(|params| params.get("status"))
                        .and_then(Value::as_str)
                        == Some("failed")
            },
        ),
    )
    .await??;
    let failed: ServerNotification = failed.try_into()?;
    let ServerNotification::McpServerStatusUpdated(failed) = failed else {
        anyhow::bail!("unexpected notification variant");
    };
    assert_eq!(failed.name, "optional_broken");
    assert_eq!(failed.status, McpServerStartupState::Failed);
    assert!(
        failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("MCP client for `optional_broken` failed to start")),
        "unexpected MCP startup error: {:?}",
        failed.error
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_surfaces_cloud_requirements_load_errors() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/wham/config/requirements"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("content-type", "text/html")
                .set_body_string("<html>nope</html>"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": { "code": "refresh_token_invalidated" }
        })))
        .mount(&server)
        .await;

    let codex_home = TempDir::new()?;
    let model_server = create_mock_responses_server_repeating_assistant("Done").await;
    let chatgpt_base_url = format!("{}/backend-api", server.uri());
    create_config_toml_with_chatgpt_base_url(
        codex_home.path(),
        &model_server.uri(),
        &chatgpt_base_url,
    )?;
    write_chatgpt_auth(
        codex_home.path(),
        ChatGptAuthFixture::new("chatgpt-token")
            .refresh_token("stale-refresh-token")
            .plan_type("business")
            .chatgpt_user_id("user-123")
            .chatgpt_account_id("account-123")
            .account_id("account-123"),
        AuthCredentialsStoreMode::File,
    )?;

    let refresh_token_url = format!("{}/oauth/token", server.uri());
    let mut mcp = McpProcess::new_with_env(
        codex_home.path(),
        &[
            ("OPENAI_API_KEY", None),
            (
                REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR,
                Some(refresh_token_url.as_str()),
            ),
        ],
    )
    .await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let req_id = mcp
        .send_thread_start_request(ThreadStartParams::default())
        .await?;

    let err: JSONRPCError = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_error_message(RequestId::Integer(req_id)),
    )
    .await??;

    assert!(
        err.error.message.contains("failed to load configuration"),
        "unexpected error message: {}",
        err.error.message
    );
    assert_eq!(
        err.error.data,
        Some(json!({
            "reason": "cloudRequirements",
            "errorCode": "Auth",
            "action": "relogin",
            "statusCode": 401,
            "detail": "Your access token could not be refreshed because your refresh token was revoked. Please log out and sign in again.",
        }))
    );

    Ok(())
}

#[tokio::test]
async fn thread_start_with_elevated_sandbox_trusts_project_and_followup_loads_project_config()
-> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
model_reasoning_effort = "high"
"#,
    )?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let first_request = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(first_request)),
    )
    .await??;

    let second_request = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            ..Default::default()
        })
        .await?;
    let second_response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(second_request)),
    )
    .await??;
    let ThreadStartResponse {
        approval_policy,
        reasoning_effort,
        ..
    } = to_response::<ThreadStartResponse>(second_response)?;

    assert_eq!(approval_policy, AskForApproval::OnRequest);
    assert_eq!(reasoning_effort, Some(ReasoningEffort::High));

    let config_toml = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    let workspace_abs = workspace.path().to_path_buf().abs();
    let trusted_root = resolve_root_git_project_for_trust(LOCAL_FS.as_ref(), &workspace_abs)
        .await
        .unwrap_or(workspace_abs);
    let trusted_root_key = project_trust_key(trusted_root.as_path());
    assert!(config_toml.contains(&trusted_root_key));
    assert!(config_toml.contains("trust_level = \"trusted\""));

    Ok(())
}

#[tokio::test]
async fn thread_start_with_nested_git_cwd_trusts_repo_root() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let repo_root = TempDir::new()?;
    std::fs::create_dir(repo_root.path().join(".git"))?;
    let nested = repo_root.path().join("nested/project");
    std::fs::create_dir_all(&nested)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(nested.display().to_string()),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let config_toml = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    let nested_abs = nested.abs();
    let trusted_root = resolve_root_git_project_for_trust(LOCAL_FS.as_ref(), &nested_abs)
        .await
        .expect("git root should resolve");
    let trusted_root_key = project_trust_key(trusted_root.as_path());
    let nested_key = project_trust_key(&nested);
    assert!(config_toml.contains(&trusted_root_key));
    assert!(!config_toml.contains(&nested_key));

    Ok(())
}

#[tokio::test]
async fn thread_start_with_read_only_sandbox_does_not_persist_project_trust() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let workspace = TempDir::new()?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let config_toml = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    assert!(!config_toml.contains("trust_level = \"trusted\""));
    assert!(!config_toml.contains(&workspace.path().display().to_string()));

    Ok(())
}

#[tokio::test]
async fn thread_start_preserves_untrusted_project_trust() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let workspace = TempDir::new()?;
    let config_path = codex_home.path().join("config.toml");
    let workspace_key = workspace.path().display().to_string();
    let mut config_toml =
        std::fs::read_to_string(&config_path)?.parse::<toml_edit::DocumentMut>()?;
    config_toml["projects"][workspace_key.as_str()]["trust_level"] = toml_edit::value("untrusted");
    std::fs::write(&config_path, config_toml.to_string())?;
    let config_before = std::fs::read_to_string(&config_path)?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        })
        .await?;
    timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let config_after = std::fs::read_to_string(&config_path)?;
    assert_eq!(config_after, config_before);

    Ok(())
}

#[tokio::test]
async fn thread_start_skips_trust_write_when_project_is_already_trusted() -> Result<()> {
    let server = create_mock_responses_server_repeating_assistant("Done").await;

    let codex_home = TempDir::new()?;
    create_config_toml_without_approval_policy(codex_home.path(), &server.uri())?;

    let workspace = TempDir::new()?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    std::fs::write(
        project_config_dir.join("config.toml"),
        r#"
model_reasoning_effort = "high"
"#,
    )?;
    set_project_trust_level(codex_home.path(), workspace.path(), TrustLevel::Trusted)?;
    let config_before = std::fs::read_to_string(codex_home.path().join("config.toml"))?;

    let mut mcp = McpProcess::new(codex_home.path()).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    let request_id = mcp
        .send_thread_start_request(ThreadStartParams {
            cwd: Some(workspace.path().display().to_string()),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        })
        .await?;
    let response: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    let ThreadStartResponse {
        approval_policy,
        reasoning_effort,
        ..
    } = to_response::<ThreadStartResponse>(response)?;

    assert_eq!(approval_policy, AskForApproval::OnRequest);
    assert_eq!(reasoning_effort, Some(ReasoningEffort::High));

    let config_after = std::fs::read_to_string(codex_home.path().join("config.toml"))?;
    assert_eq!(config_after, config_before);

    Ok(())
}

fn create_config_toml_without_approval_policy(
    codex_home: &Path,
    server_uri: &str,
) -> std::io::Result<()> {
    create_config_toml_with_optional_approval_policy(
        codex_home, server_uri, /*approval_policy*/ None,
    )
}

fn create_config_toml_with_optional_approval_policy(
    codex_home: &Path,
    server_uri: &str,
    approval_policy: Option<&str>,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    let approval_policy = approval_policy
        .map(|policy| format!("approval_policy = \"{policy}\"\n"))
        .unwrap_or_default();
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
{approval_policy}sandbox_mode = "read-only"

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

fn create_config_toml_with_profile_workspace_root(
    codex_home: &Path,
    server_uri: &str,
    profile_root: &Path,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    let profile_root_key = profile_root
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
default_permissions = "dev"
model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[permissions.dev.workspace_roots]
"{profile_root_key}" = true

[permissions.dev.filesystem.":workspace_roots"]
"." = "write"
"#,
        ),
    )
}

fn create_config_toml_with_chatgpt_base_url(
    codex_home: &Path,
    server_uri: &str,
    chatgpt_base_url: &str,
) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "never"
sandbox_mode = "read-only"
chatgpt_base_url = "{chatgpt_base_url}"

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

fn create_config_toml_with_required_broken_mcp(
    codex_home: &Path,
    server_uri: &str,
) -> std::io::Result<()> {
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

[mcp_servers.required_broken]
{required_broken_transport}
required = true
"#,
            required_broken_transport = broken_mcp_transport_toml()
        ),
    )
}

fn create_config_toml_with_optional_broken_mcp(
    codex_home: &Path,
    server_uri: &str,
) -> std::io::Result<()> {
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

[mcp_servers.optional_broken]
{optional_broken_transport}
"#,
            optional_broken_transport = broken_mcp_transport_toml()
        ),
    )
}

#[cfg(target_os = "windows")]
fn broken_mcp_transport_toml() -> &'static str {
    r#"command = "cmd"
args = ["/C", "exit 1"]"#
}

#[cfg(not(target_os = "windows"))]
fn broken_mcp_transport_toml() -> &'static str {
    r#"command = "/bin/sh"
args = ["-c", "exit 1"]"#
}
