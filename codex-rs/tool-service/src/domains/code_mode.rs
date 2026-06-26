use codex_code_mode_api::PUBLIC_TOOL_NAME;
use codex_code_mode_api::WAIT_TOOL_NAME;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::code_mode_exec_plan_for_specs;
use codex_tool_planning::create_code_mode_tool;
use codex_tool_planning::create_code_mode_wait_tool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;

use crate::context::TypedToolSpecRequest;

pub(crate) fn specs(
    request: &TypedToolSpecRequest<'_>,
    nested_specs: &[ToolSpec],
) -> Vec<ToolSpec> {
    if !request.config.environment_mode.has_environment() {
        return Vec::new();
    }

    let exec_plan = code_mode_exec_plan_for_specs(nested_specs);
    vec![
        create_code_mode_tool(
            &exec_plan.enabled_tools,
            &exec_plan.namespace_descriptions,
            request.config.code_mode_only_enabled,
            /*deferred_tools_available*/ true,
        ),
        create_code_mode_wait_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(tool_name.name.as_str(), PUBLIC_TOOL_NAME | WAIT_TOOL_NAME)
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
        "tool domain code_mode is not migrated into ToolService yet for {}",
        call.tool_name
    )))
}
