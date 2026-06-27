use std::sync::Arc;

use crate::adapters::SessionCapabilityAdapter;
use crate::adapters::TurnCapabilityAdapter;
use codex_hooks_api::PermissionRequestDecision;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_tool_runtime_api::NetworkApprovalMode;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::PermissionRequestPayload;
use codex_tool_runtime_api::ToolError;
use codex_tool_runtime_api::ToolOrchestratorHost;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Default)]
pub(crate) struct CommandServiceToolOrchestratorHost;

impl ToolOrchestratorHost<
    Arc<SessionCapabilityAdapter>,
    Arc<TurnCapabilityAdapter>,
    ToolRuntimeNetworkApprovalTrigger,
> for CommandServiceToolOrchestratorHost
{
    type ActiveNetworkApproval = Arc<dyn ToolRuntimeNetworkApprovalHandle>;
    type DeferredNetworkApproval = Arc<dyn ToolRuntimeNetworkApprovalHandle>;

    fn strict_auto_review_enabled_for_turn<'a>(
        &'a self,
        session: &'a Arc<SessionCapabilityAdapter>,
    ) -> impl std::future::Future<Output = bool> + Send + 'a {
        session.strict_auto_review_enabled_for_turn()
    }

    fn routes_approval_to_guardian(&self, turn: &Arc<TurnCapabilityAdapter>) -> bool {
        turn.routes_approval_to_guardian()
    }

    fn new_guardian_review_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn guardian_rejection_message<'a>(
        &'a self,
        session: &'a Arc<SessionCapabilityAdapter>,
        review_id: &'a str,
    ) -> impl std::future::Future<Output = String> + Send + 'a {
        session.guardian_rejection_message(review_id)
    }

    fn guardian_timeout_message(&self) -> String {
        "guardian review timed out".to_string()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        session: &'a Arc<SessionCapabilityAdapter>,
        turn: &'a Arc<TurnCapabilityAdapter>,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl std::future::Future<Output = Option<PermissionRequestDecision>> + Send + 'a {
        session.run_permission_request_hooks(
            turn.as_ref(),
            permission_request_run_id,
            permission_request,
        )
    }

    fn begin_network_approval<'a>(
        &'a self,
        session: &'a Arc<SessionCapabilityAdapter>,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> impl std::future::Future<Output = Option<Self::ActiveNetworkApproval>> + Send + 'a {
        session.begin_tool_network_approval(turn_id, managed_network_active, spec)
    }

    fn active_network_approval_mode(
        &self,
        active: &Self::ActiveNetworkApproval,
    ) -> NetworkApprovalMode {
        active.mode()
    }

    fn active_network_approval_cancellation_token(
        &self,
        active: &Self::ActiveNetworkApproval,
    ) -> CancellationToken {
        active.cancellation_token()
    }

    fn into_deferred_network_approval(
        &self,
        active: Self::ActiveNetworkApproval,
    ) -> Option<Self::DeferredNetworkApproval> {
        (active.mode() == NetworkApprovalMode::Deferred).then_some(active)
    }

    fn finish_immediate_network_approval<'a>(
        &'a self,
        _session: &'a Arc<SessionCapabilityAdapter>,
        active: Self::ActiveNetworkApproval,
    ) -> impl std::future::Future<Output = Result<(), ToolError>> + Send + 'a {
        async move { active.finish().await }
    }

    fn finish_deferred_network_approval<'a>(
        &'a self,
        _session: &'a Arc<SessionCapabilityAdapter>,
        deferred: Option<Self::DeferredNetworkApproval>,
    ) -> impl std::future::Future<Output = Result<(), ToolError>> + Send + 'a {
        async move {
            let Some(deferred) = deferred else {
                return Ok(());
            };
            deferred.finish().await
        }
    }
}
