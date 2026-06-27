use std::collections::HashMap;
use std::time::Instant;

use codex_mcp_tool_types::ToolInfo;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::models::function_call_output_content_items_to_text;
use codex_protocol::protocol::McpInvocation;
use codex_thread_api::McpResourceApi;
use codex_thread_api::SessionMcpToolCaller;
use codex_thread_api::SessionMcpToolTurn;
use codex_thread_api::ThreadCapability;
use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_planning::ResponsesApiNamespace;
use codex_tool_planning::ResponsesApiNamespaceTool;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_list_mcp_resource_templates_tool;
use codex_tool_planning::create_list_mcp_resources_tool;
use codex_tool_planning::create_read_mcp_resource_tool;
use codex_tool_planning::mcp_tool_to_deferred_responses_api_tool;
use codex_tool_planning::mcp_tool_to_responses_api_tool;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime::McpToolOutput;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_service_api::AnyToolResult;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

use crate::context::TypedToolSpecRequest;

const LIST_MCP_RESOURCES_TOOL_NAME: &str = "list_mcp_resources";
const LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME: &str = "list_mcp_resource_templates";
const READ_MCP_RESOURCE_TOOL_NAME: &str = "read_mcp_resource";

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    if request.params.mcp_tools.is_some() {
        specs.push(create_list_mcp_resources_tool());
        specs.push(create_list_mcp_resource_templates_tool());
        specs.push(create_read_mcp_resource_tool());
    }

    specs.extend(
        request
            .params
            .mcp_tools
            .into_iter()
            .flatten()
            .filter_map(|tool| tool_info_to_spec(tool, /*deferred*/ false)),
    );
    specs.extend(
        request
            .params
            .deferred_mcp_tools
            .into_iter()
            .flatten()
            .filter_map(|tool| tool_info_to_spec(tool, /*deferred*/ true)),
    );
    specs
}

pub(crate) fn owns_tool_name(request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    if request
        .params
        .mcp_tools
        .into_iter()
        .flatten()
        .chain(request.params.deferred_mcp_tools.into_iter().flatten())
        .any(|tool| tool.canonical_tool_name() == *tool_name)
    {
        return true;
    }

    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            LIST_MCP_RESOURCES_TOOL_NAME
                | LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME
                | READ_MCP_RESOURCE_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(request: &TypedToolSpecRequest<'_>, call: &ToolCall) -> bool {
    request
        .params
        .mcp_tools
        .into_iter()
        .flatten()
        .chain(request.params.deferred_mcp_tools.into_iter().flatten())
        .find(|tool| tool.canonical_tool_name() == call.tool_name)
        .is_some_and(|tool| tool.supports_parallel_tool_calls)
        || matches!(
            call.tool_name.name.as_str(),
            LIST_MCP_RESOURCES_TOOL_NAME
                | LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME
                | READ_MCP_RESOURCE_TOOL_NAME
        )
}

pub(crate) async fn dispatch(
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    mcp_resource_api: Arc<dyn McpResourceApi>,
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result: Box<dyn ToolOutput> = match call.tool_name.name.as_str() {
        LIST_MCP_RESOURCES_TOOL_NAME => Box::new(
            dispatch_list_mcp_resources(mcp_resource_api.as_ref(), turn.as_ref(), &call).await?,
        ),
        LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME => Box::new(
            dispatch_list_mcp_resource_templates(mcp_resource_api.as_ref(), turn.as_ref(), &call)
                .await?,
        ),
        READ_MCP_RESOURCE_TOOL_NAME => Box::new(
            dispatch_read_mcp_resource(mcp_resource_api.as_ref(), turn.as_ref(), &call).await?,
        ),
        _ => Box::new(
            dispatch_mcp_tool_call(session, turn, mcp_tools, deferred_mcp_tools, &call).await?,
        ),
    };

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result,
        post_tool_use_payload: None,
    })
}

fn tool_info_to_spec(tool: &ToolInfo, deferred: bool) -> Option<ToolSpec> {
    let tool_name = tool.canonical_tool_name();
    let namespace_name = tool_name.namespace.as_ref()?;
    let function = if deferred {
        mcp_tool_to_deferred_responses_api_tool(&tool_name, &tool.tool).ok()?
    } else {
        mcp_tool_to_responses_api_tool(&tool_name, &tool.tool).ok()?
    };
    let description = tool
        .namespace_description
        .as_deref()
        .map(str::trim)
        .filter(|value: &&str| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            tool.connector_name
                .as_deref()
                .map(str::trim)
                .filter(|value: &&str| !value.is_empty())
                .map(|value| format!("Tools for working with {value}."))
        })
        .unwrap_or_default();

    Some(ToolSpec::Namespace(ResponsesApiNamespace {
        name: namespace_name.clone(),
        description,
        tools: vec![ResponsesApiNamespaceTool::Function(function)],
    }))
}

async fn dispatch_mcp_tool_call(
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
    call: &ToolCall,
) -> Result<McpToolOutput, FunctionCallError> {
    let tool_info = mcp_tools
        .into_iter()
        .flatten()
        .chain(deferred_mcp_tools.into_iter().flatten())
        .find(|tool| tool.canonical_tool_name() == call.tool_name)
        .cloned()
        .ok_or_else(|| {
            FunctionCallError::Fatal(format!("unsupported MCP tool {}", call.tool_name))
        })?;

    let payload = match &call.payload {
        ToolPayload::Function { arguments } => arguments.clone(),
        _ => {
            return Err(FunctionCallError::RespondToModel(
                "mcp handler received unsupported payload".to_string(),
            ));
        }
    };

    let started = Instant::now();
    let outcome = session
        .call_mcp_tool(
            turn.as_ref(),
            call.call_id.clone(),
            tool_info.server_name.clone(),
            tool_info.tool.name.to_string(),
            call.tool_name.to_string(),
            payload,
        )
        .await;

    Ok(McpToolOutput {
        result: outcome.result,
        tool_input: outcome.tool_input,
        wall_time: started.elapsed(),
        original_image_detail_supported: turn.mcp_original_image_detail_supported(),
        truncation_policy: turn.mcp_truncation_policy(),
    })
}

async fn dispatch_list_mcp_resources(
    service: &dyn McpResourceApi,
    turn: &dyn ThreadCapability,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let raw_arguments = function_arguments(&call.payload, LIST_MCP_RESOURCES_TOOL_NAME)?;
    let parsed_arguments = parse_arguments(&raw_arguments)?;
    let args: ListResourcesArgs = parse_optional_args(call)?;
    let server = normalize_optional_string(args.server);
    let cursor = normalize_optional_string(args.cursor);
    let invocation = McpInvocation {
        server: server.clone().unwrap_or_else(|| "codex".to_string()),
        tool: LIST_MCP_RESOURCES_TOOL_NAME.to_string(),
        arguments: parsed_arguments,
    };

    service
        .emit_mcp_resource_tool_call_begin(turn, &call.call_id, invocation.clone())
        .await;
    let start = Instant::now();
    let payload_result: Result<ListResourcesPayload, FunctionCallError> = async {
        if let Some(server_name) = server.clone() {
            let params = cursor.clone().map(|value| PaginatedRequestParams {
                cursor: Some(value),
            });
            let result = service
                .list_resources(turn, &server_name, params)
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!("resources/list failed: {err}"))
                })?;
            Ok(ListResourcesPayload::from_single_server(server_name, result))
        } else {
            if cursor.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "cursor can only be used when a server is specified".to_string(),
                ));
            }
            Ok(ListResourcesPayload::from_all_servers(
                service.list_all_resources(turn).await,
            ))
        }
    }
    .await;

    finish_mcp_resource_call(service, turn, &call.call_id, invocation, start, payload_result).await
}

async fn dispatch_list_mcp_resource_templates(
    service: &dyn McpResourceApi,
    turn: &dyn ThreadCapability,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let raw_arguments =
        function_arguments(&call.payload, LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME)?;
    let parsed_arguments = parse_arguments(&raw_arguments)?;
    let args: ListResourceTemplatesArgs = parse_optional_args(call)?;
    let server = normalize_optional_string(args.server);
    let cursor = normalize_optional_string(args.cursor);
    let invocation = McpInvocation {
        server: server.clone().unwrap_or_else(|| "codex".to_string()),
        tool: LIST_MCP_RESOURCE_TEMPLATES_TOOL_NAME.to_string(),
        arguments: parsed_arguments,
    };

    service
        .emit_mcp_resource_tool_call_begin(turn, &call.call_id, invocation.clone())
        .await;
    let start = Instant::now();
    let payload_result: Result<ListResourceTemplatesPayload, FunctionCallError> = async {
        if let Some(server_name) = server.clone() {
            let params = cursor.clone().map(|value| PaginatedRequestParams {
                cursor: Some(value),
            });
            let result = service
                .list_resource_templates(turn, &server_name, params)
                .await
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "resources/templates/list failed: {err}"
                    ))
                })?;
            Ok(ListResourceTemplatesPayload::from_single_server(
                server_name,
                result,
            ))
        } else {
            if cursor.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "cursor can only be used when a server is specified".to_string(),
                ));
            }
            Ok(ListResourceTemplatesPayload::from_all_servers(
                service.list_all_resource_templates(turn).await,
            ))
        }
    }
    .await;

    finish_mcp_resource_call(service, turn, &call.call_id, invocation, start, payload_result).await
}

async fn dispatch_read_mcp_resource(
    service: &dyn McpResourceApi,
    turn: &dyn ThreadCapability,
    call: &ToolCall,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let raw_arguments = function_arguments(&call.payload, READ_MCP_RESOURCE_TOOL_NAME)?;
    let parsed_arguments = parse_arguments(&raw_arguments)?;
    let args: ReadResourceArgs = parse_required_args(call)?;
    let server = normalize_required_string("server", args.server)?;
    let uri = normalize_required_string("uri", args.uri)?;
    let invocation = McpInvocation {
        server: server.clone(),
        tool: READ_MCP_RESOURCE_TOOL_NAME.to_string(),
        arguments: parsed_arguments,
    };

    service
        .emit_mcp_resource_tool_call_begin(turn, &call.call_id, invocation.clone())
        .await;
    let start = Instant::now();
    let payload_result: Result<ReadResourcePayload, FunctionCallError> = async {
        let result = service
            .read_resource(turn, &server, ReadResourceRequestParams { uri: uri.clone() })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("resources/read failed: {err}"))
            })?;
        Ok(ReadResourcePayload {
            server,
            uri,
            result,
        })
    }
    .await;

    finish_mcp_resource_call(service, turn, &call.call_id, invocation, start, payload_result).await
}

async fn finish_mcp_resource_call<Payload>(
    service: &dyn McpResourceApi,
    turn: &dyn ThreadCapability,
    call_id: &str,
    invocation: McpInvocation,
    start: Instant,
    payload_result: Result<Payload, FunctionCallError>,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    Payload: Serialize,
{
    match payload_result {
        Ok(payload) => match serialize_function_output(payload) {
            Ok(output) => {
                let content =
                    function_call_output_content_items_to_text(&output.body).unwrap_or_default();
                service
                    .emit_mcp_resource_tool_call_end(
                        turn,
                        call_id,
                        invocation,
                        start.elapsed(),
                        Ok(call_tool_result_from_content(&content, output.success)),
                    )
                    .await;
                Ok(output)
            }
            Err(err) => {
                let message = err.to_string();
                service
                    .emit_mcp_resource_tool_call_end(
                        turn,
                        call_id,
                        invocation,
                        start.elapsed(),
                        Err(message.clone()),
                    )
                    .await;
                Err(err)
            }
        },
        Err(err) => {
            let message = err.to_string();
            service
                .emit_mcp_resource_tool_call_end(
                    turn,
                    call_id,
                    invocation,
                    start.elapsed(),
                    Err(message.clone()),
                )
                .await;
            Err(err)
        }
    }
}

fn call_tool_result_from_content(content: &str, success: Option<bool>) -> CallToolResult {
    CallToolResult {
        content: vec![serde_json::json!({"type": "text", "text": content})],
        structured_content: None,
        is_error: success.map(|value| !value),
        meta: None,
    }
}

fn function_arguments(payload: &ToolPayload, tool_name: &str) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments.clone()),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} handler received unsupported payload"
        ))),
    }
}

fn normalize_optional_string(input: Option<String>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn normalize_required_string(field: &str, value: String) -> Result<String, FunctionCallError> {
    match normalize_optional_string(Some(value)) {
        Some(normalized) => Ok(normalized),
        None => Err(FunctionCallError::RespondToModel(format!(
            "{field} must be provided"
        ))),
    }
}

fn serialize_function_output<T>(payload: T) -> Result<FunctionToolOutput, FunctionCallError>
where
    T: Serialize,
{
    let content = serde_json::to_string(&payload).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to serialize MCP resource response: {err}"
        ))
    })?;
    Ok(FunctionToolOutput::from_text(content, Some(true)))
}

fn parse_arguments(raw_args: &str) -> Result<Option<Value>, FunctionCallError> {
    if raw_args.trim().is_empty() {
        Ok(None)
    } else {
        let value: Value = serde_json::from_str(raw_args).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
        })?;
        if value.is_null() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }
}

fn parse_required_args<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    match parse_arguments(call.function_arguments()?)? {
        Some(value) => serde_json::from_value(value).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
        }),
        None => Err(FunctionCallError::RespondToModel(
            "failed to parse function arguments: expected value".to_string(),
        )),
    }
}

fn parse_optional_args<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned + Default,
{
    match parse_arguments(call.function_arguments()?)? {
        Some(value) => serde_json::from_value(value).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
        }),
        None => Ok(T::default()),
    }
}

#[derive(Debug, Deserialize, Default)]
struct ListResourcesArgs {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ListResourceTemplatesArgs {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReadResourceArgs {
    server: String,
    uri: String,
}

#[derive(Debug, Serialize)]
struct ResourceWithServer {
    server: String,
    #[serde(flatten)]
    resource: Resource,
}

impl ResourceWithServer {
    fn new(server: String, resource: Resource) -> Self {
        Self { server, resource }
    }
}

#[derive(Debug, Serialize)]
struct ResourceTemplateWithServer {
    server: String,
    #[serde(flatten)]
    template: ResourceTemplate,
}

impl ResourceTemplateWithServer {
    fn new(server: String, template: ResourceTemplate) -> Self {
        Self { server, template }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResourcesPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    resources: Vec<ResourceWithServer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

impl ListResourcesPayload {
    fn from_single_server(server: String, result: ListResourcesResult) -> Self {
        let resources = result
            .resources
            .into_iter()
            .map(|resource| ResourceWithServer::new(server.clone(), resource))
            .collect();
        Self {
            server: Some(server),
            resources,
            next_cursor: result.next_cursor,
        }
    }

    fn from_all_servers(resources_by_server: HashMap<String, Vec<Resource>>) -> Self {
        let mut entries: Vec<(String, Vec<Resource>)> = resources_by_server.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut resources = Vec::new();
        for (server, server_resources) in entries {
            for resource in server_resources {
                resources.push(ResourceWithServer::new(server.clone(), resource));
            }
        }

        Self {
            server: None,
            resources,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResourceTemplatesPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    server: Option<String>,
    resource_templates: Vec<ResourceTemplateWithServer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

impl ListResourceTemplatesPayload {
    fn from_single_server(server: String, result: ListResourceTemplatesResult) -> Self {
        let resource_templates = result
            .resource_templates
            .into_iter()
            .map(|template| ResourceTemplateWithServer::new(server.clone(), template))
            .collect();
        Self {
            server: Some(server),
            resource_templates,
            next_cursor: result.next_cursor,
        }
    }

    fn from_all_servers(templates_by_server: HashMap<String, Vec<ResourceTemplate>>) -> Self {
        let mut entries: Vec<(String, Vec<ResourceTemplate>)> =
            templates_by_server.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut resource_templates = Vec::new();
        for (server, server_templates) in entries {
            for template in server_templates {
                resource_templates.push(ResourceTemplateWithServer::new(server.clone(), template));
            }
        }

        Self {
            server: None,
            resource_templates,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct ReadResourcePayload {
    server: String,
    uri: String,
    #[serde(flatten)]
    result: ReadResourceResult,
}
