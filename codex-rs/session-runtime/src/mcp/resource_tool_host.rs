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
use codex_session_api::SessionMcpResourceCaller;
use serde_json::Value;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

impl SessionMcpResourceCaller<Arc<TurnContext>> for Session {
    async fn list_resources(
        self: Arc<Self>,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourcesResult, String> {
        Session::list_resources(self.as_ref(), server, params)
            .await
            .map_err(|err| format!("{err:#}"))
    }

    async fn list_all_resources(self: Arc<Self>) -> HashMap<String, Vec<Resource>> {
        Session::list_all_resources(self.as_ref()).await
    }

    async fn list_resource_templates(
        self: Arc<Self>,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourceTemplatesResult, String> {
        Session::list_resource_templates(self.as_ref(), server, params)
            .await
            .map_err(|err| format!("{err:#}"))
    }

    async fn list_all_resource_templates(
        self: Arc<Self>,
    ) -> HashMap<String, Vec<ResourceTemplate>> {
        Session::list_all_resource_templates(self.as_ref()).await
    }

    async fn read_resource(
        self: Arc<Self>,
        server: &str,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, String> {
        Session::read_resource(self.as_ref(), server, params)
            .await
            .map_err(|err| format!("{err:#}"))
    }

    async fn emit_mcp_resource_tool_call_begin(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: &str,
        invocation: McpInvocation,
    ) {
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
        self.emit_turn_item_started(turn.as_ref(), &item).await;
    }

    async fn emit_mcp_resource_tool_call_end(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: &str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) {
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
        self.emit_turn_item_completed(turn.as_ref(), item).await;
    }
}
