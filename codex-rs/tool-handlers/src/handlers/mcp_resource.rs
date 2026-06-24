use std::collections::HashMap;
use std::time::Instant;

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
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_list_mcp_resource_templates_tool;
use codex_tool_planning::create_list_mcp_resources_tool;
use codex_tool_planning::create_read_mcp_resource_tool;
use codex_tool_runtime_api::McpResourceHost;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;

pub struct ListMcpResourcesHandler<Host> {
    host: Host,
}

impl<Host> ListMcpResourcesHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

pub struct ListMcpResourceTemplatesHandler<Host> {
    host: Host,
}

impl<Host> ListMcpResourceTemplatesHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

pub struct ReadMcpResourceHandler<Host> {
    host: Host,
}

impl<Host> ReadMcpResourceHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
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

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ListMcpResourcesHandler<Host>
where
    Host: McpResourceHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_mcp_resources")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_list_mcp_resources_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "list_mcp_resources")?;
            let arguments = parse_arguments(arguments.as_str())?;
            let args: ListResourcesArgs = parse_args_with_default(arguments.clone())?;
            let server = normalize_optional_string(args.server);
            let cursor = normalize_optional_string(args.cursor);
            let invocation = McpInvocation {
                server: server.clone().unwrap_or_else(|| "codex".to_string()),
                tool: "list_mcp_resources".to_string(),
                arguments: arguments.clone(),
            };

            self.host
                .emit_mcp_tool_call_begin(&session, &turn, &call_id, invocation.clone())
                .await;
            let start = Instant::now();
            let payload_result: Result<ListResourcesPayload, FunctionCallError> = async {
                if let Some(server_name) = server.clone() {
                    let params = cursor.clone().map(|value| PaginatedRequestParams {
                        cursor: Some(value),
                    });
                    let result = self
                        .host
                        .list_resources(&session, &server_name, params)
                        .await
                        .map_err(|err| {
                            FunctionCallError::RespondToModel(format!(
                                "resources/list failed: {err}"
                            ))
                        })?;
                    Ok(ListResourcesPayload::from_single_server(
                        server_name,
                        result,
                    ))
                } else {
                    if cursor.is_some() {
                        return Err(FunctionCallError::RespondToModel(
                            "cursor can only be used when a server is specified".to_string(),
                        ));
                    }
                    Ok(ListResourcesPayload::from_all_servers(
                        self.host.list_all_resources(&session).await,
                    ))
                }
            }
            .await;

            finish_mcp_resource_call(
                &self.host,
                &session,
                &turn,
                &call_id,
                invocation,
                start,
                payload_result,
            )
            .await
        })
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ListMcpResourceTemplatesHandler<Host>
where
    Host: McpResourceHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_mcp_resource_templates")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_list_mcp_resource_templates_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "list_mcp_resource_templates")?;
            let arguments = parse_arguments(arguments.as_str())?;
            let args: ListResourceTemplatesArgs = parse_args_with_default(arguments.clone())?;
            let server = normalize_optional_string(args.server);
            let cursor = normalize_optional_string(args.cursor);
            let invocation = McpInvocation {
                server: server.clone().unwrap_or_else(|| "codex".to_string()),
                tool: "list_mcp_resource_templates".to_string(),
                arguments: arguments.clone(),
            };

            self.host
                .emit_mcp_tool_call_begin(&session, &turn, &call_id, invocation.clone())
                .await;
            let start = Instant::now();
            let payload_result: Result<ListResourceTemplatesPayload, FunctionCallError> = async {
                if let Some(server_name) = server.clone() {
                    let params = cursor.clone().map(|value| PaginatedRequestParams {
                        cursor: Some(value),
                    });
                    let result = self
                        .host
                        .list_resource_templates(&session, &server_name, params)
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
                        self.host.list_all_resource_templates(&session).await,
                    ))
                }
            }
            .await;

            finish_mcp_resource_call(
                &self.host,
                &session,
                &turn,
                &call_id,
                invocation,
                start,
                payload_result,
            )
            .await
        })
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ReadMcpResourceHandler<Host>
where
    Host: McpResourceHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("read_mcp_resource")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_read_mcp_resource_tool())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let arguments = function_arguments(metadata.payload, "read_mcp_resource")?;
            let arguments = parse_arguments(arguments.as_str())?;
            let args: ReadResourceArgs = parse_args(arguments.clone())?;
            let server = normalize_required_string("server", args.server)?;
            let uri = normalize_required_string("uri", args.uri)?;
            let invocation = McpInvocation {
                server: server.clone(),
                tool: "read_mcp_resource".to_string(),
                arguments: arguments.clone(),
            };

            self.host
                .emit_mcp_tool_call_begin(&session, &turn, &call_id, invocation.clone())
                .await;
            let start = Instant::now();
            let payload_result: Result<ReadResourcePayload, FunctionCallError> = async {
                let result = self
                    .host
                    .read_resource(
                        &session,
                        &server,
                        ReadResourceRequestParams { uri: uri.clone() },
                    )
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

            finish_mcp_resource_call(
                &self.host,
                &session,
                &turn,
                &call_id,
                invocation,
                start,
                payload_result,
            )
            .await
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ListMcpResourcesHandler<Host>
where
    Host: McpResourceHost,
{
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ListMcpResourceTemplatesHandler<Host>
where
    Host: McpResourceHost,
{
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ReadMcpResourceHandler<Host>
where
    Host: McpResourceHost,
{
}

async fn finish_mcp_resource_call<Host, Payload>(
    host: &Host,
    session: &Host::Session,
    turn: &Host::Turn,
    call_id: &str,
    invocation: McpInvocation,
    start: Instant,
    payload_result: Result<Payload, FunctionCallError>,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    Host: McpResourceHost,
    Payload: Serialize,
{
    match payload_result {
        Ok(payload) => match serialize_function_output(payload) {
            Ok(output) => {
                let content =
                    function_call_output_content_items_to_text(&output.body).unwrap_or_default();
                host.emit_mcp_tool_call_end(
                    session,
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
                host.emit_mcp_tool_call_end(
                    session,
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
            host.emit_mcp_tool_call_end(
                session,
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

fn function_arguments(payload: ToolPayload, tool_name: &str) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} handler received unsupported payload"
        ))),
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

fn parse_args<T>(arguments: Option<Value>) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    match arguments {
        Some(value) => serde_json::from_value(value).map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
        }),
        None => Err(FunctionCallError::RespondToModel(
            "failed to parse function arguments: expected value".to_string(),
        )),
    }
}

fn parse_args_with_default<T>(arguments: Option<Value>) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned + Default,
{
    match arguments {
        Some(value) => parse_args(Some(value)),
        None => Ok(T::default()),
    }
}

#[cfg(test)]
#[path = "mcp_resource_tests.rs"]
mod tests;
