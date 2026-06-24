use std::sync::Arc;

use crate::mcp::tool_call::handle_mcp_tool_call;
use crate::original_image_detail::can_request_original_image_detail;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::CoreToolDomainHost;
use codex_tool_runtime_api::McpToolCallHost;
use codex_tool_runtime_api::McpToolCallOutcome;
use codex_utils_output_truncation::TruncationPolicy;

impl McpToolCallHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    async fn call_mcp_tool(
        &self,
        session: Self::Session,
        turn: &Self::Turn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> McpToolCallOutcome {
        let handled = handle_mcp_tool_call(
            session,
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

    fn mcp_original_image_detail_supported(&self, turn: &Self::Turn) -> bool {
        can_request_original_image_detail(&turn.model_info)
    }

    fn mcp_truncation_policy(&self, turn: &Self::Turn) -> TruncationPolicy {
        turn.truncation_policy
    }
}
