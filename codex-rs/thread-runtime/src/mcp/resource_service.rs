use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use codex_protocol::items::McpToolCallError;
use codex_protocol::items::McpToolCallItem;
use codex_protocol::items::McpToolCallStatus;
use codex_protocol::items::TurnItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::protocol::McpInvocation;
use codex_thread_api::McpResourceApi;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ThreadCapability;
use serde_json::Value;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

#[derive(Clone, Default)]
pub struct McpResourceService;

impl McpResourceApi for McpResourceService {
    fn list_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>> {
        let session = session_from_capability(capability);
        Box::pin(async move {
            session
                .list_resources(server, params)
                .await
                .map_err(|err| format!("{err:#}"))
        })
    }

    fn list_all_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, HashMap<String, Vec<Resource>>> {
        let session = session_from_capability(capability);
        Box::pin(async move { session.list_all_resources().await })
    }

    fn list_resource_templates<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>> {
        let session = session_from_capability(capability);
        Box::pin(async move {
            session
                .list_resource_templates(server, params)
                .await
                .map_err(|err| format!("{err:#}"))
        })
    }

    fn list_all_resource_templates(
        &self,
        capability: &'_ dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<ResourceTemplate>>> {
        let session = session_from_capability(capability);
        Box::pin(async move { session.list_all_resource_templates().await })
    }

    fn read_resource<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>> {
        let session = session_from_capability(capability);
        Box::pin(async move {
            session
                .read_resource(server, params)
                .await
                .map_err(|err| format!("{err:#}"))
        })
    }

    fn emit_mcp_resource_tool_call_begin<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> SessionCapabilityFuture<'a, ()> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            let McpInvocation {
                server,
                tool,
                arguments,
            } = invocation;
            let item = TurnItem::McpToolCall(McpToolCallItem {
                id: call_id.to_string(),
                server,
                tool,
                arguments: arguments.unwrap_or(Value::Null),
                mcp_app_resource_uri: None,
                status: McpToolCallStatus::InProgress,
                result: None,
                error: None,
                duration: None,
            });
            session.emit_turn_item_started(turn, &item).await;
        })
    }

    fn emit_mcp_resource_tool_call_end<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> SessionCapabilityFuture<'a, ()> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            let (status, result, error) = match result {
                Ok(result) if result.is_error.unwrap_or(false) => {
                    (McpToolCallStatus::Failed, Some(result), None)
                }
                Ok(result) => (McpToolCallStatus::Completed, Some(result), None),
                Err(message) => (
                    McpToolCallStatus::Failed,
                    None,
                    Some(McpToolCallError { message }),
                ),
            };
            let McpInvocation {
                server,
                tool,
                arguments,
            } = invocation;
            let item = TurnItem::McpToolCall(McpToolCallItem {
                id: call_id.to_string(),
                server,
                tool,
                arguments: arguments.unwrap_or(Value::Null),
                mcp_app_resource_uri: None,
                status,
                result,
                error,
                duration: Some(duration),
            });
            session.emit_turn_item_completed(turn, item).await;
        })
    }
}

fn session_from_capability(capability: &dyn ThreadCapability) -> Arc<Session> {
    turn_context_from_capability(capability).session_arc()
}

fn turn_context_from_capability(capability: &dyn ThreadCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("mcp resource capability must be backed by TurnContext")
}
