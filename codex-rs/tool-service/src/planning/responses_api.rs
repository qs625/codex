#[cfg(test)]
use crate::LoadableToolSpec;
#[cfg(test)]
use crate::ResponsesApiNamespace;
#[cfg(test)]
use crate::ResponsesApiNamespaceTool;
use crate::ResponsesApiTool;
use crate::ToolDefinition;
use crate::ToolName;
use crate::parse_dynamic_tool;
use crate::parse_mcp_tool;
use mcp_types::McpTool;
use protocol::dynamic_tools::DynamicToolSpec;

pub fn dynamic_tool_to_responses_api_tool(
    tool: &DynamicToolSpec,
) -> Result<ResponsesApiTool, serde_json::Error> {
    Ok(tool_definition_to_responses_api_tool(parse_dynamic_tool(
        tool,
    )?))
}

pub fn mcp_tool_to_responses_api_tool(
    tool_name: &ToolName,
    tool: &McpTool,
) -> Result<ResponsesApiTool, serde_json::Error> {
    Ok(tool_definition_to_responses_api_tool(
        parse_mcp_tool(tool)?.renamed(tool_name.name.clone()),
    ))
}

pub fn mcp_tool_to_deferred_responses_api_tool(
    tool_name: &ToolName,
    tool: &McpTool,
) -> Result<ResponsesApiTool, serde_json::Error> {
    Ok(tool_definition_to_responses_api_tool(
        parse_mcp_tool(tool)?
            .renamed(tool_name.name.clone())
            .into_deferred(),
    ))
}

pub fn tool_definition_to_responses_api_tool(tool_definition: ToolDefinition) -> ResponsesApiTool {
    ResponsesApiTool {
        name: tool_definition.name,
        description: tool_definition.description,
        strict: false,
        defer_loading: tool_definition.defer_loading.then_some(true),
        parameters: tool_definition.input_schema,
        output_schema: tool_definition.output_schema,
    }
}

#[cfg(test)]
#[path = "responses_api_tests.rs"]
mod tests;
