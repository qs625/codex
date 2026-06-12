use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::workflow_spec::WORKFLOW_ABORT_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_DESCRIBE_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_LIST_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_RESUME_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_START_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_STATUS_TOOL_NAME;
use crate::tools::handlers::workflow_spec::create_workflow_abort_tool;
use crate::tools::handlers::workflow_spec::create_workflow_describe_tool;
use crate::tools::handlers::workflow_spec::create_workflow_list_tool;
use crate::tools::handlers::workflow_spec::create_workflow_resume_tool;
use crate::tools::handlers::workflow_spec::create_workflow_start_tool;
use crate::tools::handlers::workflow_spec::create_workflow_status_tool;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::workflow_runs::WorkflowRun;
use crate::workflows::load_workflow_registry;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressEvent;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;
use serde_json::Value;

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
        let run = session
            .workflow_runs
            .start(&registry, workflow, args.inputs.unwrap_or(Value::Null))
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Started).await;
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

        let run = session
            .workflow_runs
            .resume(run_id, args.inputs)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        record_workflow_progress(&session, &turn, &run, WorkflowRunProgressKind::Resumed).await;
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
        .record_conversation_items(turn, std::slice::from_ref(&item))
        .await;
}
