use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolCallSource;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::multi_agents_v2;
use crate::tools::handlers::parse_arguments;
use codex_tools::WORKFLOW_ABORT_TOOL_NAME;
use codex_tools::WORKFLOW_DESCRIBE_TOOL_NAME;
use codex_tools::WORKFLOW_LIST_TOOL_NAME;
use codex_tools::WORKFLOW_RESUME_TOOL_NAME;
use codex_tools::WORKFLOW_START_TOOL_NAME;
use codex_tools::WORKFLOW_STATUS_TOOL_NAME;
use codex_tools::create_workflow_abort_tool;
use codex_tools::create_workflow_describe_tool;
use codex_tools::create_workflow_list_tool;
use codex_tools::create_workflow_resume_tool;
use codex_tools::create_workflow_start_tool;
use codex_tools::create_workflow_status_tool;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::workflow_runs::WorkflowAgentBinding;
use crate::workflow_runs::WorkflowRun;
use crate::workflow_runs::WorkflowRunStatus;
use crate::workflow_runs::WorkflowRuntimeBridge;
use crate::workflow_runs::WorkflowRuntimeError;
use crate::workflow_runs::WorkflowRuntimeRequest;
use crate::workflows::load_workflow_registry;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressEvent;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub struct WorkflowListHandler;
pub struct WorkflowDescribeHandler;
pub struct WorkflowStartHandler;
pub struct WorkflowStatusHandler;
pub struct WorkflowResumeHandler;
pub struct WorkflowAbortHandler;

#[derive(Debug, Deserialize)]
struct WorkflowDescribeArgs {
    workflow: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowStartArgs {
    workflow: String,
    #[serde(default)]
    inputs: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct WorkflowStatusArgs {
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowResumeArgs {
    run_id: String,
    #[serde(default)]
    inputs: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct WorkflowAbortArgs {
    run_id: String,
    #[serde(default)]
    reason: Option<String>,
}

struct CodexWorkflowRuntimeBridge {
    session: Arc<crate::session::session::Session>,
    turn: Arc<crate::session::turn_context::TurnContext>,
    cancellation_token: CancellationToken,
    tracker: SharedTurnDiffTracker,
}

impl WorkflowRuntimeBridge for CodexWorkflowRuntimeBridge {
    fn call(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<Value, WorkflowRuntimeError>> + Send + '_>> {
        Box::pin(async move { self.handle_request(request).await })
    }
}

impl CodexWorkflowRuntimeBridge {
    async fn handle_request(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        match request.method.as_str() {
            "agent.spawn" => self.spawn_agent(request).await,
            "agent.followup" => self.followup_agent(request).await,
            "agent.wait" => self.wait_agent(request).await,
            "shell.exec" => Err(WorkflowRuntimeError::unsupported(
                "wf.shell is not connected to exec_command in this phase; use an agent to request shell work",
            )),
            method => Err(WorkflowRuntimeError::unsupported(format!(
                "unsupported workflow runtime method `{method}`"
            ))),
        }
    }

    async fn spawn_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let agent_id = required_string(&request.params, "id")?;
        let options = request.params.get("options").cloned().unwrap_or(Value::Null);
        let message = required_string(&options, "message")?;
        let mut args = serde_json::json!({
            "message": message,
            "task_name": workflow_agent_task_name(&request.run_id, &agent_id),
        });
        copy_option_string(&mut args, "agent_type", &options, &["type", "agent_type"]);
        copy_option_string(&mut args, "cwd", &options, &["cwd"]);
        copy_option_string(&mut args, "model", &options, &["model"]);
        copy_option_string(
            &mut args,
            "reasoning_effort",
            &options,
            &["reasoningEffort", "reasoning_effort"],
        );
        copy_option_string(
            &mut args,
            "service_tier",
            &options,
            &["serviceTier", "service_tier"],
        );
        copy_option_string(&mut args, "agent_mode", &options, &["agentMode", "agent_mode"]);
        copy_option_string(&mut args, "fork_turns", &options, &["forkTurns", "fork_turns"]);

        let invocation = self.invocation("spawn_agent", &request, args)?;
        let result = multi_agents_v2::handle_workflow_spawn_agent(invocation)
            .await
            .map_err(runtime_error_from_tool_error)?;
        let agent_path = required_string(&result, "task_name")?;
        serde_json::to_value(WorkflowAgentBinding {
            stage_id: Some(agent_id.clone()),
            agent_id,
            agent_path,
            workflow_id: Some(request.workflow_id),
            run_id: Some(request.run_id),
            thread_id: None,
            status: None,
            options,
        })
        .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))
    }

    async fn followup_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let target = required_string(&request.params, "target")?;
        let message = required_string(&request.params, "message")?;
        let args = serde_json::json!({
            "target": target.clone(),
            "message": message.clone(),
        });
        multi_agents_v2::handle_workflow_followup_task(
            self.invocation("followup_task", &request, args)?,
            target,
            message,
        )
        .await
        .map_err(runtime_error_from_tool_error)
    }

    async fn wait_agent(
        &self,
        request: WorkflowRuntimeRequest,
    ) -> Result<Value, WorkflowRuntimeError> {
        let target = required_string(&request.params, "target")?;
        let args = serde_json::json!({ "target": target.clone() });
        multi_agents_v2::handle_workflow_wait_agent(self.invocation("wait_agent", &request, args)?)
            .await
            .map_err(runtime_error_from_tool_error)
    }

    fn invocation(
        &self,
        tool_name: &str,
        request: &WorkflowRuntimeRequest,
        arguments: Value,
    ) -> Result<ToolInvocation, WorkflowRuntimeError> {
        let arguments = serde_json::to_string(&arguments)
            .map_err(|err| WorkflowRuntimeError::invalid_request(err.to_string()))?;
        Ok(ToolInvocation {
            session: Arc::clone(&self.session),
            turn: Arc::clone(&self.turn),
            cancellation_token: self.cancellation_token.clone(),
            tracker: Arc::clone(&self.tracker),
            call_id: format!(
                "workflow:{}:{}:{}",
                request.run_id, request.rpc_id, tool_name
            ),
            tool_name: ToolName::plain(tool_name),
            source: ToolCallSource::Direct,
            payload: ToolPayload::Function { arguments },
        })
    }
}

fn required_string(value: &Value, field: &str) -> Result<String, WorkflowRuntimeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| WorkflowRuntimeError::invalid_request(format!("missing `{field}`")))
}

fn copy_option_string(target: &mut Value, target_field: &str, source: &Value, fields: &[&str]) {
    let Some(value) = fields
        .iter()
        .find_map(|field| source.get(*field).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    target[target_field] = Value::String(value.to_string());
}

fn workflow_agent_task_name(run_id: &str, stage_id: &str) -> String {
    format!(
        "workflow_{}_{}_{}",
        path_safe_segment(run_id),
        path_safe_segment(stage_id),
        stable_hex_hash(stage_id)
    )
}

fn path_safe_segment(value: &str) -> String {
    let mut output = String::new();
    let mut previous_was_underscore = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        let safe = if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            previous_was_underscore = false;
            Some(ch)
        } else if !previous_was_underscore {
            previous_was_underscore = true;
            Some('_')
        } else {
            None
        };
        if let Some(ch) = safe {
            output.push(ch);
        }
    }

    let trimmed = output.trim_matches('_');
    if trimmed.is_empty() {
        "agent".to_string()
    } else {
        trimmed.to_string()
    }
}

fn stable_hex_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn runtime_error_from_tool_error(error: FunctionCallError) -> WorkflowRuntimeError {
    match error {
        FunctionCallError::RespondToModel(message) => {
            WorkflowRuntimeError::invalid_request(message)
        }
        FunctionCallError::Fatal(message) => WorkflowRuntimeError {
            code: "runtime_error".to_string(),
            message,
        },
    }
}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowListHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_LIST_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_list_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { turn, payload, .. } = invocation;
        match payload {
            ToolPayload::Function { .. } => {
                let registry = load_workflow_registry(&turn.config);
                json_output(&registry)
            }
            _ => Err(FunctionCallError::RespondToModel(
                "workflow_list handler received unsupported payload".to_string(),
            )),
        }
    }
}

impl ToolHandler for WorkflowListHandler {}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowDescribeHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_DESCRIBE_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_describe_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation { turn, payload, .. } = invocation;
        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "workflow_describe handler received unsupported payload".to_string(),
                ));
            }
        };
        let args: WorkflowDescribeArgs = parse_arguments(&arguments)?;
        let workflow = args.workflow.trim();
        if workflow.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "workflow must not be empty".to_string(),
            ));
        }

        let registry = load_workflow_registry(&turn.config);
        let details = registry
            .details(workflow)
            .map_err(FunctionCallError::RespondToModel)?;
        json_output(&details)
    }
}

impl ToolHandler for WorkflowDescribeHandler {}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowStartHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_START_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_start_tool())
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            cancellation_token,
            tracker,
            payload,
            ..
        } = invocation;
        let arguments = function_arguments(payload, WORKFLOW_START_TOOL_NAME)?;
        let args: WorkflowStartArgs = parse_arguments(&arguments)?;
        let workflow = args.workflow.trim();
        if workflow.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "workflow must not be empty".to_string(),
            ));
        }

        let registry = load_workflow_registry(&turn.config);
        let updates = session.workflow_runs.subscribe();
        let bridge: Arc<dyn WorkflowRuntimeBridge> = Arc::new(CodexWorkflowRuntimeBridge {
            session: Arc::clone(&session),
            turn: Arc::clone(&turn),
            cancellation_token: cancellation_token.clone(),
            tracker: Arc::clone(&tracker),
        });
        let run = session
            .workflow_runs
            .start_with_bridge(&registry, workflow, args.inputs.unwrap_or(Value::Null), bridge)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Started).await;
        record_terminal_workflow_progress(session, turn, updates, run.run_id.clone());
        json_output(&run)
    }
}

impl ToolHandler for WorkflowStartHandler {}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowStatusHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_STATUS_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_status_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session, payload, ..
        } = invocation;
        let arguments = function_arguments(payload, WORKFLOW_STATUS_TOOL_NAME)?;
        let args: WorkflowStatusArgs = parse_arguments(&arguments)?;
        let run_id = args.run_id.trim();
        if run_id.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "run_id must not be empty".to_string(),
            ));
        }

        let run = session
            .workflow_runs
            .status(run_id)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        json_output(&run)
    }
}

impl ToolHandler for WorkflowStatusHandler {}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowResumeHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_RESUME_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_resume_tool())
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            cancellation_token,
            tracker,
            payload,
            ..
        } = invocation;
        let arguments = function_arguments(payload, WORKFLOW_RESUME_TOOL_NAME)?;
        let args: WorkflowResumeArgs = parse_arguments(&arguments)?;
        let run_id = args.run_id.trim();
        if run_id.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "run_id must not be empty".to_string(),
            ));
        }

        let updates = session.workflow_runs.subscribe();
        let bridge: Arc<dyn WorkflowRuntimeBridge> = Arc::new(CodexWorkflowRuntimeBridge {
            session: Arc::clone(&session),
            turn: Arc::clone(&turn),
            cancellation_token: cancellation_token.clone(),
            tracker: Arc::clone(&tracker),
        });
        let run = session
            .workflow_runs
            .resume_with_bridge(run_id, args.inputs, bridge)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Resumed).await;
        record_terminal_workflow_progress(session, turn, updates, run.run_id.clone());
        json_output(&run)
    }
}

impl ToolHandler for WorkflowResumeHandler {}

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WorkflowAbortHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(WORKFLOW_ABORT_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_workflow_abort_tool())
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;
        let arguments = function_arguments(payload, WORKFLOW_ABORT_TOOL_NAME)?;
        let args: WorkflowAbortArgs = parse_arguments(&arguments)?;
        let run_id = args.run_id.trim();
        if run_id.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "run_id must not be empty".to_string(),
            ));
        }

        let run = session
            .workflow_runs
            .abort(run_id, args.reason)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Aborted).await;
        json_output(&run)
    }
}

impl ToolHandler for WorkflowAbortHandler {}

fn json_output<T: serde::Serialize>(value: &T) -> Result<FunctionToolOutput, FunctionCallError> {
    serde_json::to_string_pretty(value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize workflow tool output: {err}"))
        })
}

fn function_arguments(payload: ToolPayload, tool_name: &str) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} handler received unsupported payload"
        ))),
    }
}

async fn record_workflow_progress(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    run: &WorkflowRun,
    kind: WorkflowRunProgressKind,
) {
    let item = ResponseItem::WorkflowRunProgress {
        id: None,
        event: WorkflowRunProgressEvent {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow.id.clone(),
            status: serde_json::to_value(run.status).unwrap_or(Value::Null),
            runner_status: run.runner_status.clone(),
            kind,
            message: run.message.clone(),
            updated_at: run.updated_at,
        },
    };
    session
        .record_model_items_and_emit_display_events(turn, std::slice::from_ref(&item))
        .await;
}

fn record_terminal_workflow_progress(
    session: std::sync::Arc<crate::session::session::Session>,
    turn: std::sync::Arc<crate::session::turn_context::TurnContext>,
    mut updates: tokio::sync::broadcast::Receiver<WorkflowRun>,
    run_id: String,
) {
    tokio::spawn(async move {
        loop {
            let run = match updates.recv().await {
                Ok(run) => run,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if run.run_id == run_id
                && let Some(kind) = workflow_progress_kind_for_terminal_status(run.status)
            {
                record_workflow_progress(&session, &turn, &run, kind).await;
                break;
            }
        }
    });
}

fn workflow_progress_kind_for_terminal_status(
    status: WorkflowRunStatus,
) -> Option<WorkflowRunProgressKind> {
    match status {
        WorkflowRunStatus::Running => None,
        WorkflowRunStatus::Completed => Some(WorkflowRunProgressKind::Completed),
        WorkflowRunStatus::Failed => Some(WorkflowRunProgressKind::Failed),
        WorkflowRunStatus::Aborted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_agent_task_name_uses_runtime_owned_safe_name() {
        assert_eq!(
            workflow_agent_task_name("wf_1781667698_3", "review/fix Stage"),
            "workflow_wf_1781667698_3_review_fix_stage_6a50e610f22115e3"
        );
        assert_eq!(
            workflow_agent_task_name("wf_1", "../root"),
            "workflow_wf_1_root_fcf22ea2feead752"
        );
        assert_eq!(
            workflow_agent_task_name("wf_1", "!!!"),
            "workflow_wf_1_agent_bbe43c17ca866be2"
        );
    }

    #[test]
    fn workflow_agent_task_name_keeps_colliding_slugs_distinct() {
        assert_ne!(
            workflow_agent_task_name("wf_1", "review/fix"),
            workflow_agent_task_name("wf_1", "review_fix")
        );
        assert_ne!(
            workflow_agent_task_name("wf_1", "review fix"),
            workflow_agent_task_name("wf_1", "review_fix")
        );
    }
}
