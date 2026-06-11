use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::handlers::workflow_spec::WORKFLOW_DESCRIBE_TOOL_NAME;
use crate::tools::handlers::workflow_spec::WORKFLOW_LIST_TOOL_NAME;
use crate::tools::handlers::workflow_spec::create_workflow_describe_tool;
use crate::tools::handlers::workflow_spec::create_workflow_list_tool;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::workflows::load_workflow_registry;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;

pub struct WorkflowListHandler;
pub struct WorkflowDescribeHandler;

#[derive(Debug, Deserialize)]
struct WorkflowDescribeArgs {
    workflow: String,
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

fn json_output<T: serde::Serialize>(value: &T) -> Result<FunctionToolOutput, FunctionCallError> {
    serde_json::to_string_pretty(value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize workflow tool output: {err}"))
        })
}
