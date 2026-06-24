use crate::guardian::GuardianNetworkAccessTrigger;
use crate::guardian::guardian_rejection_message;
use crate::guardian::guardian_timeout_message;
use crate::guardian::new_guardian_review_id;
use crate::guardian::routes_approval_to_guardian;
use crate::hook_runtime::run_permission_request_hooks;
use crate::network_approval::ActiveNetworkApproval;
use crate::network_approval::DeferredNetworkApproval;
use crate::network_approval::NetworkApprovalMode;
use crate::network_approval::NetworkApprovalSpec;
use crate::network_approval::begin_network_approval;
use crate::network_approval::finish_deferred_network_approval;
use crate::network_approval::finish_immediate_network_approval;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::sandboxing::PermissionRequestPayload;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::ToolRuntime;
use codex_hooks_api::PermissionRequestDecision;
use codex_protocol::protocol::AskForApproval;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_tool_runtime_api::OrchestratorRunResult;
use codex_tool_runtime_api::ToolOrchestratorHost;
use codex_tool_runtime_api::ToolSandboxContext;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(crate) struct ToolOrchestrator {
    inner: codex_tool_runtime::ToolOrchestrator<CoreToolOrchestratorHost>,
}

impl ToolOrchestrator {
    pub fn new(sandbox_runtime: SharedSandboxRuntime) -> Self {
        Self {
            inner: codex_tool_runtime::ToolOrchestrator::new(
                CoreToolOrchestratorHost,
                sandbox_runtime,
            ),
        }
    }

    pub async fn run<Rq, Out, T>(
        &mut self,
        tool: &mut T,
        req: &Rq,
        tool_ctx: &ToolCtx,
        turn_ctx: &TurnContext,
        approval_policy: AskForApproval,
    ) -> Result<OrchestratorRunResult<Out, DeferredNetworkApproval>, ToolError>
    where
        T: ToolRuntime<
                Rq,
                Out,
                Session = Arc<Session>,
                Turn = Arc<TurnContext>,
                NetworkApprovalTrigger = GuardianNetworkAccessTrigger,
            >,
    {
        let sandbox_context = ToolSandboxContext {
            turn_id: turn_ctx.sub_id.clone(),
            telemetry: turn_ctx.session_telemetry.clone(),
            file_system_sandbox_policy: turn_ctx.file_system_sandbox_policy(),
            network_sandbox_policy: turn_ctx.network_sandbox_policy(),
            permission_profile: turn_ctx.permission_profile.clone(),
            managed_network_active: turn_ctx.network.is_some(),
            #[allow(deprecated)]
            cwd: turn_ctx.cwd.clone(),
            codex_linux_sandbox_exe: turn_ctx.codex_linux_sandbox_exe.clone(),
            use_legacy_landlock: turn_ctx.features.use_legacy_landlock(),
            windows_sandbox_level: turn_ctx.windows_sandbox_level,
            windows_sandbox_private_desktop: turn_ctx
                .config
                .permissions
                .windows_sandbox_private_desktop,
        };

        self.inner
            .run(tool, req, tool_ctx, &sandbox_context, approval_policy)
            .await
    }
}

#[derive(Clone, Copy)]
pub struct CoreToolOrchestratorHost;

impl ToolOrchestratorHost<Arc<Session>, Arc<TurnContext>, GuardianNetworkAccessTrigger>
    for CoreToolOrchestratorHost
{
    type ActiveNetworkApproval = ActiveNetworkApproval;
    type DeferredNetworkApproval = DeferredNetworkApproval;

    async fn strict_auto_review_enabled_for_turn(&self, session: &Arc<Session>) -> bool {
        session.strict_auto_review_enabled_for_turn().await
    }

    fn routes_approval_to_guardian(&self, turn: &Arc<TurnContext>) -> bool {
        routes_approval_to_guardian(turn.as_ref())
    }

    fn new_guardian_review_id(&self) -> String {
        new_guardian_review_id()
    }

    async fn guardian_rejection_message(&self, session: &Arc<Session>, review_id: &str) -> String {
        guardian_rejection_message(session.as_ref(), review_id).await
    }

    fn guardian_timeout_message(&self) -> String {
        guardian_timeout_message()
    }

    async fn run_permission_request_hooks(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        permission_request_run_id: &str,
        permission_request: PermissionRequestPayload,
    ) -> Option<PermissionRequestDecision> {
        run_permission_request_hooks(session, turn, permission_request_run_id, permission_request)
            .await
    }

    async fn begin_network_approval(
        &self,
        session: &Arc<Session>,
        turn_id: &str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec>,
    ) -> Option<ActiveNetworkApproval> {
        begin_network_approval(session, turn_id, managed_network_active, spec).await
    }

    fn active_network_approval_mode(&self, active: &ActiveNetworkApproval) -> NetworkApprovalMode {
        active.mode()
    }

    fn active_network_approval_cancellation_token(
        &self,
        active: &ActiveNetworkApproval,
    ) -> CancellationToken {
        active.cancellation_token()
    }

    fn into_deferred_network_approval(
        &self,
        active: ActiveNetworkApproval,
    ) -> Option<DeferredNetworkApproval> {
        active.into_deferred()
    }

    async fn finish_immediate_network_approval(
        &self,
        session: &Arc<Session>,
        active: ActiveNetworkApproval,
    ) -> Result<(), ToolError> {
        finish_immediate_network_approval(session, active).await
    }

    async fn finish_deferred_network_approval(
        &self,
        session: &Arc<Session>,
        deferred: Option<DeferredNetworkApproval>,
    ) -> Result<(), ToolError> {
        finish_deferred_network_approval(session, deferred).await
    }
}
