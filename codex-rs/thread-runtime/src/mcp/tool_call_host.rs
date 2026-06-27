use std::sync::Arc;

use crate::mcp::tool_call::handle_mcp_tool_call;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_thread_api::McpToolCallOutcome;
use codex_thread_api::SessionMcpToolCaller;
use codex_thread_api::SessionMcpToolTurn;
use codex_utils_output_truncation::TruncationPolicy;

impl SessionMcpToolCaller for Session {
    async fn call_mcp_tool(
        self: Arc<Self>,
        turn: &dyn SessionMcpToolTurn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> McpToolCallOutcome {
        let turn = turn
            .as_any()
            .downcast_ref::<TurnContext>()
            .expect("SessionMcpToolTurn must be implemented by TurnContext");
        let handled = handle_mcp_tool_call(
            self,
            turn,
            call_id,
            server,
            tool_name,
            hook_tool_name,
            arguments,
        )
        .await;
        McpToolCallOutcome {
            result: handled.result,
            tool_input: handled.tool_input,
        }
    }
}

impl SessionMcpToolTurn for TurnContext {
    fn mcp_original_image_detail_supported(&self) -> bool {
        self.can_request_original_image_detail()
    }

    fn mcp_truncation_policy(&self) -> TruncationPolicy {
        self.truncation_policy()
    }
}
