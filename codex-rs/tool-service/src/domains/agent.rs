use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_planning::SpawnAgentToolOptions;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_close_agent_tool_v2;
use codex_tool_planning::create_followup_task_tool;
use codex_tool_planning::create_list_agents_tool;
use codex_tool_planning::create_report_agent_job_result_tool;
use codex_tool_planning::create_spawn_agent_tool_v2;
use codex_tool_planning::create_spawn_agents_on_csv_tool;
use codex_tool_planning::create_wait_agent_tool_v2;
use codex_tool_types::ToolName;

use crate::context::TypedToolSpecRequest;

const SPAWN_AGENT_TOOL_NAME: &str = "spawn_agent";
const FOLLOWUP_TASK_TOOL_NAME: &str = "followup_task";
const WAIT_AGENT_TOOL_NAME: &str = "wait_agent";
const LIST_AGENTS_TOOL_NAME: &str = "list_agents";
const CLOSE_AGENT_TOOL_NAME: &str = "close_agent";
const SPAWN_AGENTS_ON_CSV_TOOL_NAME: &str = "spawn_agents_on_csv";
const REPORT_AGENT_JOB_RESULT_TOOL_NAME: &str = "report_agent_job_result";

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![
        create_spawn_agent_tool_v2(SpawnAgentToolOptions {
            available_models: request.config.available_models.clone(),
            agent_type_description: request.params.default_agent_type_description.to_string(),
            hide_agent_type_model_reasoning: false,
            include_usage_hint: false,
            usage_hint_text: None,
            max_concurrent_threads_per_session: None,
        }),
        create_followup_task_tool(),
        create_wait_agent_tool_v2(),
        create_list_agents_tool(),
        create_close_agent_tool_v2(),
        create_spawn_agents_on_csv_tool(),
        create_report_agent_job_result_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            SPAWN_AGENT_TOOL_NAME
                | FOLLOWUP_TASK_TOOL_NAME
                | WAIT_AGENT_TOOL_NAME
                | LIST_AGENTS_TOOL_NAME
                | CLOSE_AGENT_TOOL_NAME
                | SPAWN_AGENTS_ON_CSV_TOOL_NAME
                | REPORT_AGENT_JOB_RESULT_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) fn dispatch(call: ToolCall) -> Result<AnyToolResult, FunctionCallError> {
    Err(FunctionCallError::Fatal(format!(
        "tool domain agent is not migrated into ToolService yet for {}",
        call.tool_name
    )))
}
