use std::time::Instant;

use codex_protocol::config_types::ModeKind;
use codex_protocol::dynamic_tools::DynamicToolCallRequest;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::items::ImageViewItem;
use codex_protocol::items::TurnItem;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::DynamicToolCallResponseEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_tool_planning::ToolName;
use codex_tool_runtime_api::FunctionToolSessionRuntime;
use codex_tool_runtime_api::FunctionToolTurn;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::turn_timing::now_unix_timestamp_ms;

impl FunctionToolTurn for TurnContext {
    fn function_tool_collaboration_mode(&self) -> ModeKind {
        self.collaboration_mode_kind()
    }

    fn function_tool_cwd(&self) -> AbsolutePathBuf {
        self.legacy_cwd()
    }

    fn function_tool_is_non_root_agent(&self) -> bool {
        self.is_non_root_agent()
    }

    fn function_tool_supports_image_input(&self) -> bool {
        self.supports_image_input()
    }

    fn function_tool_can_request_original_image_detail(&self) -> bool {
        self.can_request_original_image_detail()
    }
}

impl FunctionToolSessionRuntime<std::sync::Arc<TurnContext>> for Session {
    async fn function_tool_session_collaboration_mode(session: &std::sync::Arc<Self>) -> ModeKind {
        session.collaboration_mode_kind().await
    }

    async fn function_tool_emit_plan_update(
        session: &std::sync::Arc<Self>,
        turn: &std::sync::Arc<TurnContext>,
        args: UpdatePlanArgs,
    ) {
        session
            .send_event(turn.as_ref(), EventMsg::PlanUpdate(args))
            .await;
    }

    async fn function_tool_emit_image_view(
        session: &std::sync::Arc<Self>,
        turn: &std::sync::Arc<TurnContext>,
        call_id: String,
        path: AbsolutePathBuf,
    ) {
        let item = TurnItem::ImageView(ImageViewItem { id: call_id, path });
        session.emit_turn_item_started(turn.as_ref(), &item).await;
        session.emit_turn_item_completed(turn.as_ref(), item).await;
    }

    async fn function_tool_request_permissions(
        session: &std::sync::Arc<Self>,
        turn: &std::sync::Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        session
            .request_permissions(turn, call_id, args, cancellation_token)
            .await
    }

    async fn function_tool_request_user_input(
        session: &std::sync::Arc<Self>,
        turn: &std::sync::Arc<TurnContext>,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
    }

    async fn function_tool_request_dynamic_tool(
        session: &std::sync::Arc<Self>,
        turn: &std::sync::Arc<TurnContext>,
        call_id: String,
        tool_name: ToolName,
        arguments: Value,
    ) -> Option<DynamicToolResponse> {
        request_dynamic_tool(
            session.as_ref(),
            turn.as_ref(),
            call_id,
            tool_name,
            arguments,
        )
        .await
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "active turn checks and dynamic tool response registration must remain atomic"
)]
async fn request_dynamic_tool(
    session: &Session,
    turn_context: &TurnContext,
    call_id: String,
    tool_name: ToolName,
    arguments: Value,
) -> Option<DynamicToolResponse> {
    let namespace = tool_name.namespace;
    let tool = tool_name.name;
    let turn_id = turn_context.turn_id();
    let (tx_response, rx_response) = oneshot::channel();
    let event_id = call_id.clone();
    if session
        .register_pending_dynamic_tool_response(call_id.clone(), tx_response)
        .await
    {
        warn!("Overwriting existing pending dynamic tool call for call_id: {event_id}");
    }

    let started_at = Instant::now();
    let started_at_ms = now_unix_timestamp_ms();
    let event = EventMsg::DynamicToolCallRequest(DynamicToolCallRequest {
        call_id: call_id.clone(),
        turn_id: turn_id.clone(),
        started_at_ms,
        namespace: namespace.clone(),
        tool: tool.clone(),
        arguments: arguments.clone(),
    });
    session.send_event(turn_context, event).await;
    let response = rx_response.await.ok();

    let response_event = match &response {
        Some(response) => EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
            call_id,
            turn_id,
            completed_at_ms: now_unix_timestamp_ms(),
            namespace,
            tool,
            arguments,
            content_items: response.content_items.clone(),
            success: response.success,
            error: None,
            duration: started_at.elapsed(),
        }),
        None => EventMsg::DynamicToolCallResponse(DynamicToolCallResponseEvent {
            call_id,
            turn_id,
            completed_at_ms: now_unix_timestamp_ms(),
            namespace,
            tool,
            arguments,
            content_items: Vec::new(),
            success: false,
            error: Some("dynamic tool call was cancelled before receiving a response".to_string()),
            duration: started_at.elapsed(),
        }),
    };
    session.send_event(turn_context, response_event).await;

    response
}
