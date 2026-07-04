use crate::planning::WORKFLOW_ABORT_TOOL_NAME;
use crate::planning::WORKFLOW_DESCRIBE_TOOL_NAME;
use crate::planning::WORKFLOW_LIST_TOOL_NAME;
use crate::planning::WORKFLOW_RESUME_TOOL_NAME;
use crate::planning::WORKFLOW_START_TOOL_NAME;
use crate::planning::WORKFLOW_STATUS_TOOL_NAME;
use crate::planning::create_workflow_abort_tool;
use crate::planning::create_workflow_describe_tool;
use crate::planning::create_workflow_list_tool;
use crate::planning::create_workflow_resume_tool;
use crate::planning::create_workflow_start_tool;
use crate::planning::create_workflow_status_tool;
use codex_workflow_api::WorkflowAbortArgs;
use codex_workflow_api::WorkflowApi;
use codex_workflow_api::WorkflowDescribeArgs;
use codex_workflow_api::WorkflowExecutionContext;
use codex_workflow_api::WorkflowResumeArgs;
use codex_workflow_api::WorkflowStartArgs;
use codex_workflow_api::WorkflowStatusArgs;
use codex_workflow_api::workflow_tool_output_json;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use thread_service_api::ThreadTurnCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolSpec;

use crate::output::FunctionToolOutput;

pub(crate) fn specs() -> Vec<ToolSpec> {
    vec![
        create_workflow_list_tool(),
        create_workflow_describe_tool(),
        create_workflow_start_tool(),
        create_workflow_status_tool(),
        create_workflow_resume_tool(),
        create_workflow_abort_tool(),
    ]
}

pub(crate) fn owns_tool_name(tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            WORKFLOW_LIST_TOOL_NAME
                | WORKFLOW_DESCRIBE_TOOL_NAME
                | WORKFLOW_START_TOOL_NAME
                | WORKFLOW_STATUS_TOOL_NAME
                | WORKFLOW_RESUME_TOOL_NAME
                | WORKFLOW_ABORT_TOOL_NAME
        )
}

pub(crate) fn supports_parallel(_call: &ToolCall) -> bool {
    true
}

pub(crate) fn create_diff_consumer(
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) async fn dispatch(
    workflow_api: Arc<dyn WorkflowApi>,
    turn: Arc<dyn ThreadTurnCapability>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        WORKFLOW_LIST_TOOL_NAME => workflow_output(
            workflow_api
                .list_workflows(workflow_discovery_context(&turn)?)
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        WORKFLOW_DESCRIBE_TOOL_NAME => workflow_output(
            workflow_api
                .describe_workflow(
                    workflow_discovery_context(&turn)?,
                    parse_arguments::<WorkflowDescribeArgs>(&call)?,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        WORKFLOW_START_TOOL_NAME => workflow_output(
            workflow_api
                .start_workflow(
                    WorkflowExecutionContext::new(
                        workflow_discovery_context(&turn)?,
                        Some(Arc::clone(&turn)),
                    ),
                    parse_arguments::<WorkflowStartArgs>(&call)?,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        WORKFLOW_STATUS_TOOL_NAME => workflow_output(
            workflow_api
                .workflow_status(parse_arguments::<WorkflowStatusArgs>(&call)?)
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        WORKFLOW_RESUME_TOOL_NAME => workflow_output(
            workflow_api
                .resume_workflow(
                    WorkflowExecutionContext::new(
                        workflow_discovery_context(&turn)?,
                        Some(Arc::clone(&turn)),
                    ),
                    parse_arguments::<WorkflowResumeArgs>(&call)?,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        WORKFLOW_ABORT_TOOL_NAME => workflow_output(
            workflow_api
                .abort_workflow(
                    WorkflowExecutionContext::new(
                        workflow_discovery_context(&turn)?,
                        Some(Arc::clone(&turn)),
                    ),
                    parse_arguments::<WorkflowAbortArgs>(&call)?,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?,
        ),
        _ => Err(FunctionCallError::Fatal(format!(
            "unsupported workflow tool {}",
            call.tool_name
        ))),
    }?;

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

fn workflow_output<T: serde::Serialize>(value: T) -> Result<FunctionToolOutput, FunctionCallError> {
    workflow_tool_output_json(&value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize workflow tool output: {err}"))
        })
}

fn parse_arguments<T: DeserializeOwned>(call: &ToolCall) -> Result<T, FunctionCallError> {
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}

fn workflow_discovery_context(
    turn: &Arc<dyn ThreadTurnCapability>,
) -> Result<codex_workflow_api::WorkflowDiscoveryContext, FunctionCallError> {
    Ok(turn.discovery_context().into())
}
