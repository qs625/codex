use std::sync::Arc;
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
use codex_thread_api::FunctionToolCapability;
use codex_thread_api::SessionCapabilityFuture;
use codex_tool_runtime_api::FunctionToolSessionRuntime;
use codex_tool_runtime_api::FunctionToolTurn;
use codex_tool_types::ToolName;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::turn_timing::now_unix_timestamp_ms;

impl FunctionToolCapability for TurnContext {
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

    fn function_tool_session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, ModeKind> {
        Box::pin(async move { self.session_arc().collaboration_mode_kind().await })
    }

    fn function_tool_emit_plan_update<'a>(
        &'a self,
        args: UpdatePlanArgs,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.session_arc()
                .send_event(self, EventMsg::PlanUpdate(args))
                .await;
        })
    }

    fn function_tool_emit_image_view<'a>(
        &'a self,
        call_id: String,
        path: AbsolutePathBuf,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let session = self.session_arc();
            let item = TurnItem::ImageView(ImageViewItem { id: call_id, path });
            session.emit_turn_item_started(self, &item).await;
            session.emit_turn_item_completed(self, item).await;
        })
    }

    fn function_tool_request_permissions<'a>(
        &'a self,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<'a, Option<RequestPermissionsResponse>> {
        Box::pin(async move {
            let session = self.session_arc();
            let turn = self.self_arc();
            session
                .request_permissions(&turn, call_id, args, cancellation_token)
                .await
        })
    }

    fn function_tool_request_user_input<'a>(
        &'a self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> SessionCapabilityFuture<'a, Option<RequestUserInputResponse>> {
        Box::pin(async move {
            let session = self.session_arc();
            session
                .request_user_input(self, call_id, args)
                .await
        })
    }

    fn function_tool_request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: Value,
    ) -> SessionCapabilityFuture<'a, Option<DynamicToolResponse>> {
        Box::pin(async move {
            request_dynamic_tool(&self.session_arc(), self, call_id, tool_name, arguments).await
        })
    }
}

impl FunctionToolTurn for TurnContext {
    fn function_tool_collaboration_mode(&self) -> ModeKind {
        FunctionToolCapability::function_tool_collaboration_mode(self)
    }

    fn function_tool_cwd(&self) -> AbsolutePathBuf {
        FunctionToolCapability::function_tool_cwd(self)
    }

    fn function_tool_is_non_root_agent(&self) -> bool {
        FunctionToolCapability::function_tool_is_non_root_agent(self)
    }

    fn function_tool_supports_image_input(&self) -> bool {
        FunctionToolCapability::function_tool_supports_image_input(self)
    }

    fn function_tool_can_request_original_image_detail(&self) -> bool {
        self.can_request_original_image_detail()
    }
}

impl FunctionToolSessionRuntime<Arc<TurnContext>> for Session {
    fn function_tool_session_collaboration_mode(
        session: &Arc<Self>,
    ) -> impl std::future::Future<Output = ModeKind> + Send + '_ {
        async move { session.collaboration_mode_kind().await }
    }

    fn function_tool_emit_plan_update<'a>(
        session: &'a Arc<Self>,
        turn: &'a Arc<TurnContext>,
        args: UpdatePlanArgs,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        async move {
            session.send_event(turn.as_ref(), EventMsg::PlanUpdate(args)).await;
        }
    }

    fn function_tool_emit_image_view<'a>(
        session: &'a Arc<Self>,
        turn: &'a Arc<TurnContext>,
        call_id: String,
        path: AbsolutePathBuf,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        async move {
            let item = TurnItem::ImageView(ImageViewItem { id: call_id, path });
            session.emit_turn_item_started(turn.as_ref(), &item).await;
            session.emit_turn_item_completed(turn.as_ref(), item).await;
        }
    }

    fn function_tool_request_permissions<'a>(
        session: &'a Arc<Self>,
        turn: &'a Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> impl std::future::Future<Output = Option<RequestPermissionsResponse>> + Send + 'a {
        async move {
            session
                .request_permissions(turn, call_id, args, cancellation_token)
                .await
        }
    }

    fn function_tool_request_user_input<'a>(
        session: &'a Arc<Self>,
        turn: &'a Arc<TurnContext>,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> impl std::future::Future<Output = Option<RequestUserInputResponse>> + Send + 'a {
        async move { session.request_user_input(turn.as_ref(), call_id, args).await }
    }

    fn function_tool_request_dynamic_tool<'a>(
        session: &'a Arc<Self>,
        turn: &'a Arc<TurnContext>,
        call_id: String,
        tool_name: ToolName,
        arguments: Value,
    ) -> impl std::future::Future<Output = Option<DynamicToolResponse>> + Send + 'a {
        async move { request_dynamic_tool(session, turn.as_ref(), call_id, tool_name, arguments).await }
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
