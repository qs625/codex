use codex_tool_types::ToolName;
use codex_tool_types::ToolSpec;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;

use crate::context::TypedToolSpecRequest;

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    let Some(extension_tools) = request.params.extension_tools else {
        return Vec::new();
    };

    extension_tools
        .tool_contributors
        .iter()
        .flat_map(|contributor| {
            contributor.tools(extension_tools.session_store, extension_tools.thread_store)
        })
        .filter(|tool| tool.exposure().is_direct())
        .filter_map(|tool| tool.spec())
        .collect()
}

pub(crate) fn owns_tool_name(request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    let Some(extension_tools) = request.params.extension_tools else {
        return false;
    };

    extension_tools
        .tool_contributors
        .iter()
        .flat_map(|contributor| {
            contributor.tools(extension_tools.session_store, extension_tools.thread_store)
        })
        .any(|tool| tool.tool_name() == *tool_name)
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
        "tool domain extension is not migrated into ToolService yet for {}",
        call.tool_name
    )))
}
