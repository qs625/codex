use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use codex_protocol::protocol::SessionSource;
use codex_rollout_trace::ExecutionStatus;
use codex_rollout_trace::ThreadStartedTraceMetadata;
use codex_rollout_trace::ToolCallRequester;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use crate::session::session::Session;
use crate::session::tests::dispatch_tool_via_tool_service;
use crate::session::tests::make_session_and_context;
use crate::session::turn_context::TurnContext;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolPayload;

#[tokio::test]
async fn dispatch_lifecycle_trace_records_direct_and_code_mode_requesters() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (mut session, turn) = make_session_and_context().await;
    attach_test_trace(&mut session, &turn, temp.path())?;
    session.services.rollout_thread_trace.start_code_cell_trace(
        turn.sub_id.as_str(),
        "cell-1",
        "call-code",
        "await tools.test_tool({})",
    );

    let session = Arc::new(session);
    let turn = Arc::new(turn);
    dispatch_tool_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn),
        "direct-call",
        codex_tool_types::ToolName::plain("get_goal"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: "{}".to_string(),
        },
    )
        .await?;
    dispatch_tool_via_tool_service(
        session,
        turn,
        "code-mode-call",
        codex_tool_types::ToolName::plain("get_goal"),
        ToolCallSource::CodeMode {
            cell_id: "cell-1".to_string(),
            runtime_tool_call_id: "tool-1".to_string(),
        },
        ToolPayload::Function {
            arguments: "{}".to_string(),
        },
    )
        .await?;

    let replayed = codex_rollout_trace::replay_bundle(single_bundle_dir(temp.path())?)?;
    assert_eq!(
        replayed.tool_calls["direct-call"].model_visible_call_id,
        Some("direct-call".to_string()),
    );
    assert_eq!(
        replayed.tool_calls["direct-call"].requester,
        ToolCallRequester::Model,
    );
    assert!(
        replayed.tool_calls["direct-call"]
            .raw_invocation_payload_id
            .is_some(),
        "dispatch tracing should keep the tool invocation payload",
    );
    assert!(
        replayed.tool_calls["direct-call"]
            .raw_result_payload_id
            .is_some(),
        "direct calls should keep the model-facing result payload",
    );
    assert_eq!(
        replayed.tool_calls["code-mode-call"].model_visible_call_id,
        None,
    );
    assert_eq!(
        replayed.tool_calls["code-mode-call"].code_mode_runtime_tool_id,
        Some("tool-1".to_string()),
    );
    assert_eq!(
        replayed.tool_calls["code-mode-call"].requester,
        ToolCallRequester::CodeCell {
            code_cell_id: "code_cell:call-code".to_string(),
        },
    );
    assert!(
        replayed.tool_calls["code-mode-call"]
            .raw_result_payload_id
            .is_some(),
        "code-mode calls should keep the result returned to JavaScript",
    );

    Ok(())
}

#[tokio::test]
async fn dispatch_lifecycle_trace_records_unsupported_tool_failures() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (mut session, turn) = make_session_and_context().await;
    attach_test_trace(&mut session, &turn, temp.path())?;

    let session = Arc::new(session);
    let turn = Arc::new(turn);
    let result = dispatch_tool_via_tool_service(
        session,
        turn,
        "unsupported-call",
        codex_tool_types::ToolName::plain("missing_tool"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: "{}".to_string(),
        },
    )
    .await;

    assert!(matches!(result, Err(FunctionCallError::RespondToModel(_))));
    let replayed = codex_rollout_trace::replay_bundle(single_bundle_dir(temp.path())?)?;
    let tool_call = &replayed.tool_calls["unsupported-call"];
    assert_eq!(tool_call.execution.status, ExecutionStatus::Failed);
    assert!(tool_call.raw_result_payload_id.is_some());

    Ok(())
}

#[tokio::test]
async fn dispatch_lifecycle_trace_records_incompatible_payload_failures() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (mut session, turn) = make_session_and_context().await;
    attach_test_trace(&mut session, &turn, temp.path())?;

    let session = Arc::new(session);
    let turn = Arc::new(turn);
    let result = dispatch_tool_via_tool_service(
        session,
        turn,
        "incompatible-call",
        codex_tool_types::ToolName::plain("get_goal"),
        ToolCallSource::Direct,
        ToolPayload::Custom {
            input: "{}".to_string(),
        },
    )
    .await;

    assert!(matches!(result, Err(FunctionCallError::Fatal(_))));
    let replayed = codex_rollout_trace::replay_bundle(single_bundle_dir(temp.path())?)?;
    let tool_call = &replayed.tool_calls["incompatible-call"];
    assert_eq!(tool_call.execution.status, ExecutionStatus::Failed);
    assert!(tool_call.raw_result_payload_id.is_some());

    Ok(())
}

#[tokio::test]
async fn direct_goal_tool_without_code_cell_traces_only_the_tool_call() -> anyhow::Result<()> {
    let temp = TempDir::new()?;
    let (mut session, turn) = make_session_and_context().await;
    attach_test_trace(&mut session, &turn, temp.path())?;

    let session = Arc::new(session);
    let turn = Arc::new(turn);
    dispatch_tool_via_tool_service(
        session,
        turn,
        "goal-call",
        codex_tool_types::ToolName::plain("get_goal"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: "{}".to_string(),
        },
    )
        .await?;

    let replayed = codex_rollout_trace::replay_bundle(single_bundle_dir(temp.path())?)?;
    assert_eq!(replayed.code_cells.len(), 0);
    assert!(
        replayed.tool_calls["goal-call"]
            .raw_result_payload_id
            .is_some()
    );

    Ok(())
}

fn attach_test_trace(session: &mut Session, turn: &TurnContext, root: &Path) -> anyhow::Result<()> {
    let thread_id = session.conversation_id;
    let rollout_thread_trace =
        codex_rollout_trace::ThreadTraceContext::start_root_in_root_for_test(
            root,
            ThreadStartedTraceMetadata {
                thread_id: thread_id.to_string(),
                agent_path: "/root".to_string(),
                task_name: None,
                nickname: None,
                agent_role: None,
                session_source: SessionSource::Exec,
                cwd: PathBuf::from("/workspace"),
                rollout_path: None,
                model: "gpt-test".to_string(),
                provider_name: "test-provider".to_string(),
                approval_policy: "never".to_string(),
                sandbox_policy: "danger-full-access".to_string(),
            },
        )?;
    rollout_thread_trace.record_codex_turn_started(turn.sub_id.as_str());
    session.services.rollout_thread_trace = rollout_thread_trace;
    Ok(())
}

fn single_bundle_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let mut entries = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    assert_eq!(entries.len(), 1);
    Ok(entries.remove(0))
}
