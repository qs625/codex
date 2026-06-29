use std::collections::HashMap;
use std::time::Duration;

use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::protocol::McpInvocation;
use mcp_service_api::McpRuntimeFuture;
use mcp_service_api::McpServiceApi;
use mcp_service_api::McpToolCallOutcome;
use thread_service_api::ThreadRuntimeCapability;

#[derive(Clone, Default)]
pub struct McpService;

impl McpServiceApi for McpService {
    fn call_tool<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> McpRuntimeFuture<'a, McpToolCallOutcome> {
        Box::pin(async move {
            let (result, tool_input) = capability
                .call_mcp_tool(call_id, server, tool_name, hook_tool_name, arguments)
                .await;
            McpToolCallOutcome {
                result,
                tool_input,
            }
        })
    }

    fn list_resources<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourcesResult, String>> {
        capability.list_resources(server, params)
    }

    fn list_all_resources<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>> {
        capability.list_all_resources()
    }

    fn list_resource_templates<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourceTemplatesResult, String>> {
        capability.list_resource_templates(server, params)
    }

    fn list_all_resource_templates<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>> {
        capability.list_all_resource_templates()
    }

    fn read_resource<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, Result<ReadResourceResult, String>> {
        capability.read_resource(server, params)
    }

    fn emit_mcp_resource_tool_call_begin<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> McpRuntimeFuture<'a, ()> {
        capability.emit_mcp_resource_tool_call_begin(call_id, invocation)
    }

    fn emit_mcp_resource_tool_call_end<'a>(
        &self,
        capability: &'a dyn ThreadRuntimeCapability,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> McpRuntimeFuture<'a, ()> {
        capability.emit_mcp_resource_tool_call_end(call_id, invocation, duration, result)
    }
}
