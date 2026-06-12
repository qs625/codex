#![allow(clippy::expect_used)]

use anyhow::Result;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::RolloutLine;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use std::fs;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_control_tools_manage_run_state() -> Result<()> {
    let server = start_mock_server().await;
    let test = test_codex().build(&server).await?;
    write_workflow(
        &test.codex_home_path().join("workflows"),
        "feature-dev",
        "Home workflow",
    )?;
    write_workflow(
        &test.cwd_path().join(".codex/workflows"),
        "feature-dev",
        "Project workflow",
    )?;
    write_workflow(
        &test.codex_home_path().join("workflows"),
        "home-dev",
        "Home only workflow",
    )?;

    let mut responses = Vec::new();
    push_workflow_tool_turn(
        &mut responses,
        1,
        "call-start",
        "workflow_start",
        json!({
            "workflow": "feature-dev",
            "inputs": {"objective": "ship"}
        }),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        2,
        "call-status",
        "workflow_status",
        json!({"run_id": "wf_1"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        3,
        "call-resume",
        "workflow_resume",
        json!({
            "run_id": "wf_1",
            "inputs": {"objective": "resume"}
        }),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        4,
        "call-abort",
        "workflow_abort",
        json!({
            "run_id": "wf_1",
            "reason": "done"
        }),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        5,
        "call-resume-aborted",
        "workflow_resume",
        json!({"run_id": "wf_1"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        6,
        "call-start-home",
        "workflow_start",
        json!({"workflow": "home-dev"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        7,
        "call-start-unknown",
        "workflow_start",
        json!({"workflow": "missing"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        8,
        "call-status-empty",
        "workflow_status",
        json!({"run_id": ""}),
    )?;
    let mock = mount_sse_sequence(&server, responses).await;

    for prompt in [
        "start the feature-dev workflow",
        "check the feature-dev workflow run",
        "resume the feature-dev workflow run",
        "abort the feature-dev workflow run",
        "try resuming the aborted feature-dev workflow run",
        "start the home-dev workflow",
        "try starting a missing workflow",
        "try checking an empty workflow run id",
    ] {
        test.submit_turn_with_permission_profile(prompt, PermissionProfile::Disabled)
            .await?;
    }

    let start = function_tool_output_json(&mock, "call-start");
    assert_eq!(start["runId"], "wf_1");
    assert_eq!(start["workflow"]["id"], "feature-dev");
    assert_eq!(start["workflow"]["description"], "Project workflow");
    assert_eq!(start["status"], "running");
    assert_eq!(start["runnerStatus"], "control_plane_started");
    assert_eq!(start["inputs"], json!({"objective": "ship"}));

    let status = function_tool_output_json(&mock, "call-status");
    assert_eq!(status, start);

    let resumed = function_tool_output_json(&mock, "call-resume");
    assert_eq!(resumed["runId"], "wf_1");
    assert_eq!(resumed["revision"], 2);
    assert_eq!(resumed["runnerStatus"], "control_plane_resumed");
    assert_eq!(resumed["inputs"], json!({"objective": "resume"}));

    let aborted = function_tool_output_json(&mock, "call-abort");
    assert_eq!(aborted["status"], "aborted");
    assert_eq!(aborted["runnerStatus"], "aborted");
    assert_eq!(aborted["abortReason"], "done");

    let resume_aborted = mock
        .function_call_output_text("call-resume-aborted")
        .expect("resume aborted output");
    assert!(
        resume_aborted.contains("workflow run `wf_1` is aborted"),
        "unexpected output: {resume_aborted}"
    );

    let home = function_tool_output_json(&mock, "call-start-home");
    assert_eq!(home["runId"], "wf_2");
    assert_eq!(home["workflow"]["id"], "home-dev");
    assert_eq!(home["workflow"]["description"], "Home only workflow");
    assert_eq!(home["workflow"]["source"], "home");

    let unknown = mock
        .function_call_output_text("call-start-unknown")
        .expect("unknown workflow output");
    assert!(
        unknown.contains("unknown workflow `missing`"),
        "unexpected output: {unknown}"
    );
    let empty_status = mock
        .function_call_output_text("call-status-empty")
        .expect("empty status output");
    assert!(
        empty_status.contains("run_id must not be empty"),
        "unexpected output: {empty_status}"
    );

    test.codex.flush_rollout().await?;
    let rollout_path = test.codex.rollout_path().expect("rollout path");
    let progress = workflow_progress_items(&rollout_path)?;
    assert_eq!(
        progress,
        vec![
            expected_progress(
                "wf_1",
                "feature-dev",
                WorkflowRunProgressKind::Started,
                "running",
                "control_plane_started",
            ),
            expected_progress(
                "wf_1",
                "feature-dev",
                WorkflowRunProgressKind::Resumed,
                "running",
                "control_plane_resumed",
            ),
            expected_progress(
                "wf_1",
                "feature-dev",
                WorkflowRunProgressKind::Aborted,
                "aborted",
                "aborted",
            ),
            expected_progress(
                "wf_2",
                "home-dev",
                WorkflowRunProgressKind::Started,
                "running",
                "control_plane_started",
            ),
        ],
    );

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ExpectedWorkflowProgress {
    run_id: String,
    workflow_id: String,
    kind: WorkflowRunProgressKind,
    status: String,
    runner_status: String,
}

fn push_workflow_tool_turn(
    responses: &mut Vec<String>,
    index: usize,
    call_id: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<()> {
    responses.push(sse(vec![
        ev_response_created(&format!("resp-{index}-tool")),
        ev_function_call(call_id, tool_name, &serde_json::to_string(&arguments)?),
        ev_completed(&format!("resp-{index}-tool")),
    ]));
    responses.push(sse(vec![
        ev_response_created(&format!("resp-{index}-done")),
        ev_assistant_message(&format!("msg-{index}"), "done"),
        ev_completed(&format!("resp-{index}-done")),
    ]));
    Ok(())
}

fn function_tool_output_json(
    mock: &core_test_support::responses::ResponseMock,
    call_id: &str,
) -> Value {
    let text = mock
        .function_call_output_text(call_id)
        .expect("function call output");
    serde_json::from_str(&text).expect("workflow tool output JSON")
}

fn workflow_progress_items(path: &std::path::Path) -> Result<Vec<ExpectedWorkflowProgress>> {
    let text = std::fs::read_to_string(path)?;
    let mut items = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let rollout: RolloutLine = serde_json::from_str(line)?;
        if let RolloutItem::ResponseItem(ResponseItem::WorkflowRunProgress { event, .. }) =
            rollout.item
        {
            let status = event
                .status
                .as_str()
                .expect("workflow progress status should be a string")
                .to_string();
            items.push(ExpectedWorkflowProgress {
                run_id: event.run_id,
                workflow_id: event.workflow_id,
                kind: event.kind,
                status,
                runner_status: event.runner_status,
            });
        }
    }
    Ok(items)
}

fn expected_progress(
    run_id: &str,
    workflow_id: &str,
    kind: WorkflowRunProgressKind,
    status: &str,
    runner_status: &str,
) -> ExpectedWorkflowProgress {
    ExpectedWorkflowProgress {
        run_id: run_id.to_string(),
        workflow_id: workflow_id.to_string(),
        kind,
        status: status.to_string(),
        runner_status: runner_status.to_string(),
    }
}

fn write_workflow(root: &std::path::Path, id: &str, description: &str) -> Result<()> {
    let workflow_dir = root.join(id);
    fs::create_dir_all(&workflow_dir)?;
    fs::write(workflow_dir.join("workflow.ts"), "export default {};")?;
    fs::write(
        workflow_dir.join("workflow.json"),
        format!(
            r#"{{
  "id": "{id}",
  "name": "Feature Dev",
  "description": "{description}",
  "entry": "workflow.ts",
  "inputs": {{"objective": {{"type": "string"}}}}
}}"#
        ),
    )?;
    Ok(())
}
