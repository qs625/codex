use std::sync::Arc;

use crate::function_tool::FunctionCallError;
use crate::original_image_detail::can_request_original_image_detail;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::events::CoreToolEventHost;
use crate::tools::orchestrator::CoreToolOrchestratorHost;
use crate::tools::runtimes::CoreApplyPatchEnvironment;
use crate::tools::runtimes::CoreToolRuntimeHost;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_features::Feature;
use codex_file_system::FileSystemSandboxContext;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::dynamic_tools::DynamicToolCallRequest;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::items::ImageViewItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::openai_models::InputModality;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::DynamicToolCallResponseEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_tool_planning::ToolName;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::FunctionToolHost;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_runtime_api::ToolSandboxContext;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use std::time::Instant;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::warn;

#[cfg(test)]
pub(crate) use codex_tool_runtime::ApplyPatchToolOutput;

/// Core adapter for the host-neutral apply-patch handler owned by
/// `codex-tool-runtime`.
#[derive(Clone, Copy, Default)]
pub struct CoreApplyPatchHandlerHost;

impl ApplyPatchDiffContext for TurnContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.features.enabled(Feature::ApplyPatchStreamingEvents)
    }
}

impl ApplyPatchHandlerHost for CoreApplyPatchHandlerHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;
    type RuntimeHost = CoreToolRuntimeHost;
    type OrchestratorHost = CoreToolOrchestratorHost;
    type EventHost<'a> = CoreToolEventHost<'a>;

    fn runtime_host(&self) -> Self::RuntimeHost {
        CoreToolRuntimeHost
    }

    fn orchestrator_host(&self) -> Self::OrchestratorHost {
        CoreToolOrchestratorHost
    }

    fn sandbox_runtime(&self, session: &Self::Session) -> SharedSandboxRuntime {
        Arc::clone(&session.services.sandbox_runtime)
    }

    fn tool_sandbox_context(&self, turn: &Self::Turn) -> ToolSandboxContext {
        ToolSandboxContext {
            turn_id: turn.sub_id.clone(),
            telemetry: turn.session_telemetry.clone(),
            file_system_sandbox_policy: turn.file_system_sandbox_policy(),
            network_sandbox_policy: turn.network_sandbox_policy(),
            permission_profile: turn.permission_profile.clone(),
            managed_network_active: turn.network.is_some(),
            #[allow(deprecated)]
            cwd: turn.cwd.clone(),
            codex_linux_sandbox_exe: turn.codex_linux_sandbox_exe.clone(),
            use_legacy_landlock: turn.features.use_legacy_landlock(),
            windows_sandbox_level: turn.windows_sandbox_level,
            windows_sandbox_private_desktop: turn
                .config
                .permissions
                .windows_sandbox_private_desktop,
        }
    }

    fn approval_policy(&self, turn: &Self::Turn) -> AskForApproval {
        turn.approval_policy.value()
    }

    fn permission_profile(&self, turn: &Self::Turn) -> PermissionProfile {
        turn.permission_profile()
    }

    fn file_system_sandbox_policy(&self, turn: &Self::Turn) -> FileSystemSandboxPolicy {
        turn.file_system_sandbox_policy()
    }

    fn windows_sandbox_level(&self, turn: &Self::Turn) -> WindowsSandboxLevel {
        turn.windows_sandbox_level
    }

    fn file_system_sandbox_context(
        &self,
        turn: &Self::Turn,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        turn.file_system_sandbox_context(additional_permissions, cwd)
    }

    fn resolve_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        resolve_tool_environment(turn.as_ref(), environment_id).map(|environment| {
            environment.map(|turn_environment| ResolvedApplyPatchEnvironment {
                cwd: turn_environment.cwd.clone(),
                environment: CoreApplyPatchEnvironment::new(turn_environment.clone()),
            })
        })
    }

    async fn permission_grants(&self, session: &Self::Session) -> ToolPermissionGrants {
        ToolPermissionGrants {
            session: session.granted_session_permissions().await,
            turn: session.granted_turn_permissions().await,
        }
    }

    fn event_host<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        tracker: Option<&'a Self::Tracker>,
    ) -> Self::EventHost<'a> {
        CoreToolEventHost::new(session.as_ref(), turn.as_ref(), tracker)
    }
}

impl FunctionToolHost for CoreApplyPatchHandlerHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    fn turn_collaboration_mode(&self, turn: &Self::Turn) -> ModeKind {
        turn.collaboration_mode.mode
    }

    fn turn_cwd(&self, turn: &Self::Turn) -> AbsolutePathBuf {
        #[allow(deprecated)]
        turn.cwd.clone()
    }

    fn turn_id(&self, turn: &Self::Turn) -> String {
        turn.sub_id.clone()
    }

    fn turn_is_non_root_agent(&self, turn: &Self::Turn) -> bool {
        turn.session_source.is_non_root_agent()
    }

    fn turn_supports_image_input(&self, turn: &Self::Turn) -> bool {
        turn.model_info
            .input_modalities
            .contains(&InputModality::Image)
    }

    fn turn_can_request_original_image_detail(&self, turn: &Self::Turn) -> bool {
        can_request_original_image_detail(&turn.model_info)
    }

    async fn session_collaboration_mode(&self, session: &Self::Session) -> ModeKind {
        session.collaboration_mode().await.mode
    }

    async fn emit_plan_update(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        args: UpdatePlanArgs,
    ) {
        session
            .send_event(turn.as_ref(), EventMsg::PlanUpdate(args))
            .await;
    }

    async fn emit_image_view(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: String,
        path: AbsolutePathBuf,
    ) {
        let item = TurnItem::ImageView(ImageViewItem { id: call_id, path });
        session.emit_turn_item_started(turn.as_ref(), &item).await;
        session.emit_turn_item_completed(turn.as_ref(), item).await;
    }

    async fn request_permissions(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        session
            .request_permissions(turn, call_id, args, cancellation_token)
            .await
    }

    async fn request_user_input(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        session
            .request_user_input(turn.as_ref(), call_id, args)
            .await
    }

    async fn request_dynamic_tool(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: String,
        tool_name: ToolName,
        arguments: Value,
    ) -> Option<DynamicToolResponse> {
        request_dynamic_tool(session, turn.as_ref(), call_id, tool_name, arguments).await
    }
}

fn resolve_tool_environment<'a>(
    turn: &'a TurnContext,
    environment_id: Option<&str>,
) -> Result<Option<&'a TurnEnvironment>, FunctionCallError> {
    environment_id.map_or_else(
        || Ok(turn.environments.primary()),
        |environment_id| {
            turn.environments
                .turn_environments
                .iter()
                .find(|environment| environment.environment_id == environment_id)
                .map(Some)
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(format!(
                        "unknown turn environment id `{environment_id}`"
                    ))
                })
        },
    )
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "active turn checks and dynamic tool response registration must remain atomic"
)]
async fn request_dynamic_tool(
    session: &Arc<Session>,
    turn_context: &TurnContext,
    call_id: String,
    tool_name: ToolName,
    arguments: Value,
) -> Option<DynamicToolResponse> {
    let namespace = tool_name.namespace;
    let tool = tool_name.name;
    let turn_id = turn_context.sub_id.clone();
    let (tx_response, rx_response) = oneshot::channel();
    let event_id = call_id.clone();
    let prev_entry = {
        let mut active = session.active_turn.lock().await;
        match active.as_mut() {
            Some(at) => {
                let mut ts = at.turn_state.lock().await;
                ts.insert_pending_dynamic_tool(call_id.clone(), tx_response)
            }
            None => None,
        }
    };
    if prev_entry.is_some() {
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

#[cfg(test)]
#[path = "apply_patch_tool_host_tests.rs"]
mod tests;
