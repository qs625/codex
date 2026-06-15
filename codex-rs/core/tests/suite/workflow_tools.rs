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
use std::path::Path;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workflow_control_tools_manage_run_state() -> Result<()> {
    let server = start_mock_server().await;
    let test = test_codex().build(&server).await?;
    write_workflow(
        &test.codex_home_path().join("workflows"),
        "feature-dev",
        "Home workflow",
        long_running_workflow_entry(),
    )?;
    write_workflow(
        &test.cwd_path().join(".codex/workflows"),
        "feature-dev",
        "Project workflow",
        long_running_workflow_entry(),
    )?;
    write_workflow(
        &test.cwd_path().join(".codex/workflows"),
        "fail-dev",
        "Failing workflow",
        fail_then_resume_workflow_entry(),
    )?;
    write_workflow(
        &test.codex_home_path().join("workflows"),
        "home-dev",
        "Home only workflow",
        completed_workflow_entry(),
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
        "call-start-fail",
        "workflow_start",
        json!({"workflow": "fail-dev"}),
    )?;
    let mock = mount_sse_sequence(&server, responses).await;

    for prompt in [
        "start the feature-dev workflow",
        "start the fail-dev workflow",
    ] {
        test.submit_turn_with_permission_profile(prompt, PermissionProfile::Disabled)
            .await?;
    }

    let start = function_tool_output_json(&mock, "call-start");
    let start_run_id = start["runId"]
        .as_str()
        .expect("start run id should be a string");
    assert!(start_run_id.starts_with("wf_"));
    assert_eq!(start["workflow"]["id"], "feature-dev");
    assert_eq!(start["workflow"]["description"], "Project workflow");
    assert_eq!(start["status"], "running");
    assert_eq!(start["runnerStatus"], "runner_starting");
    assert_eq!(start["inputs"], json!({"objective": "ship"}));

    let failed_start = function_tool_output_json(&mock, "call-start-fail");
    let failed_run_id = failed_start["runId"]
        .as_str()
        .expect("failed workflow run id should be a string");
    assert!(failed_run_id.starts_with("wf_"));
    assert_ne!(failed_run_id, start_run_id);
    assert_eq!(failed_start["workflow"]["id"], "fail-dev");
    assert_eq!(failed_start["runnerStatus"], "runner_starting");
    wait_for_persisted_workflow_status(test.codex_home_path(), failed_run_id, "failed").await?;

    let mut responses = Vec::new();
    push_workflow_tool_turn(
        &mut responses,
        3,
        "call-status",
        "workflow_status",
        json!({"run_id": start_run_id}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        4,
        "call-abort",
        "workflow_abort",
        json!({
            "run_id": start_run_id,
            "reason": "done"
        }),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        5,
        "call-resume",
        "workflow_resume",
        json!({
            "run_id": failed_run_id,
            "inputs": {"objective": "resume"}
        }),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        6,
        "call-resume-aborted",
        "workflow_resume",
        json!({"run_id": start_run_id}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        7,
        "call-start-home",
        "workflow_start",
        json!({"workflow": "home-dev"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        8,
        "call-start-unknown",
        "workflow_start",
        json!({"workflow": "missing"}),
    )?;
    push_workflow_tool_turn(
        &mut responses,
        9,
        "call-status-empty",
        "workflow_status",
        json!({"run_id": ""}),
    )?;
    let mock = mount_sse_sequence(&server, responses).await;

    for prompt in [
        "check the feature-dev workflow run",
        "abort the feature-dev workflow run",
        "resume the failed workflow run",
        "try resuming the aborted feature-dev workflow run",
        "start the home-dev workflow",
        "try starting a missing workflow",
        "try checking an empty workflow run id",
    ] {
        test.submit_turn_with_permission_profile(prompt, PermissionProfile::Disabled)
            .await?;
    }

    let status = function_tool_output_json(&mock, "call-status");
    assert_eq!(status["runId"], start_run_id);
    assert_eq!(status["workflow"]["id"], "feature-dev");

    let aborted = function_tool_output_json(&mock, "call-abort");
    assert_eq!(aborted["runId"], start_run_id);
    assert_eq!(aborted["status"], "aborted");
    assert_eq!(aborted["runnerStatus"], "aborted");
    assert_eq!(aborted["abortReason"], "done");

    let resumed = function_tool_output_json(&mock, "call-resume");
    assert_eq!(resumed["runId"], failed_run_id);
    assert_eq!(resumed["status"], "running");
    assert_eq!(resumed["runnerStatus"], "runner_resuming");
    assert_eq!(resumed["inputs"], json!({"objective": "resume"}));

    let resume_aborted = mock
        .function_call_output_text("call-resume-aborted")
        .expect("resume aborted output");
    assert!(
        resume_aborted.contains(&format!("workflow run `{start_run_id}` is aborted")),
        "unexpected output: {resume_aborted}"
    );

    let home = function_tool_output_json(&mock, "call-start-home");
    let home_run_id = home["runId"]
        .as_str()
        .expect("home run id should be a string");
    assert!(home_run_id.starts_with("wf_"));
    assert_ne!(home_run_id, start_run_id);
    assert_eq!(home["workflow"]["id"], "home-dev");
    assert_eq!(home["workflow"]["description"], "Home only workflow");
    assert_eq!(home["workflow"]["source"], "home");
    assert_eq!(home["runnerStatus"], "runner_starting");

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
    assert!(
        progress.contains(&expected_progress(
            start_run_id,
            "feature-dev",
            WorkflowRunProgressKind::Started,
            "running",
            "runner_starting",
        )),
        "missing feature workflow start progress: {progress:?}"
    );
    assert!(
        progress.contains(&expected_progress(
            failed_run_id,
            "fail-dev",
            WorkflowRunProgressKind::Started,
            "running",
            "runner_starting",
        )),
        "missing failed workflow start progress: {progress:?}"
    );
    assert!(
        progress.contains(&expected_progress(
            start_run_id,
            "feature-dev",
            WorkflowRunProgressKind::Aborted,
            "aborted",
            "aborted",
        )),
        "missing feature workflow abort progress: {progress:?}"
    );
    assert!(
        progress.contains(&expected_progress(
            failed_run_id,
            "fail-dev",
            WorkflowRunProgressKind::Resumed,
            "running",
            "runner_resuming",
        )),
        "missing failed workflow resume progress: {progress:?}"
    );
    assert!(
        progress.contains(&expected_progress(
            home_run_id,
            "home-dev",
            WorkflowRunProgressKind::Started,
            "running",
            "runner_starting",
        )),
        "missing home workflow start progress: {progress:?}"
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

async fn wait_for_persisted_workflow_status(
    codex_home: &Path,
    run_id: &str,
    expected_status: &str,
) -> Result<()> {
    let run_path = codex_home.join("workflow-runs").join(run_id).join("run.json");
    let mut last_status = None;
    let mut last_error = None;
    for _ in 0..50 {
        match tokio::fs::read_to_string(&run_path).await {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(run) => {
                    last_status = run
                        .get("status")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    last_error = None;
                    if last_status.as_deref() == Some(expected_status) {
                        return Ok(());
                    }
                }
                Err(err) => {
                    last_error = Some(err.to_string());
                }
            },
            Err(err) => {
                last_error = Some(err.to_string());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!(
        "workflow run {run_id} did not reach status {expected_status}; last status: {last_status:?}; last error: {last_error:?}"
    );
}

fn write_workflow(
    root: &std::path::Path,
    id: &str,
    description: &str,
    entry_body: &str,
) -> Result<()> {
    let workflow_dir = root.join(id);
    fs::create_dir_all(&workflow_dir)?;
    fs::write(workflow_dir.join("workflow.ts"), entry_body)?;
    fs::write(
        workflow_dir.join("WORKFLOW.md"),
        format!(
            r#"---
id: {id}
name: Feature Dev
description: {description}
entry: workflow.ts
inputs:
  objective:
    type: string
---
Use this workflow from the workflow tools tests.
"#
        ),
    )?;
    Ok(())
}

fn completed_workflow_entry() -> &'static str {
    r#"import { defineWorkflow } from "@codex/workflow";

export default defineWorkflow({
  async run() {
    return { ok: true };
  }
});
"#
}

fn long_running_workflow_entry() -> &'static str {
    r#"import { defineWorkflow } from "@codex/workflow";

export default defineWorkflow({
  async run() {
    await new Promise((resolve) => setTimeout(resolve, 30_000));
  }
});
"#
}

fn fail_then_resume_workflow_entry() -> &'static str {
    r#"import { defineWorkflow } from "@codex/workflow";

export default defineWorkflow({
  async run(wf) {
    if (wf.mode === "start") {
      throw new Error("start failed");
    }
    return { resumed: true };
  }
});
"#
}
