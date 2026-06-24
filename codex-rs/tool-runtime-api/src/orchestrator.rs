use crate::NetworkApprovalMode;
use crate::NetworkApprovalSpec;
use crate::PermissionRequestPayload;
use crate::ToolError;
use codex_hooks_api::PermissionRequestDecision;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::future::Future;
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

pub struct ToolSandboxContext {
    pub turn_id: String,
    pub telemetry: SharedSessionTelemetry,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub permission_profile: PermissionProfile,
    pub managed_network_active: bool,
    pub cwd: AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
}

/// Host integration points used by the generic tool orchestrator.
///
/// Implementations adapt approval hooks, guardian review, telemetry-owned
/// network approval state, and cancellation tokens to the host session runtime.
pub trait ToolOrchestratorHost<Session, Turn, Trigger>: Send + Sync {
    type ActiveNetworkApproval;
    type DeferredNetworkApproval;

    fn strict_auto_review_enabled_for_turn<'a>(
        &'a self,
        session: &'a Session,
    ) -> impl Future<Output = bool> + Send + 'a;

    fn routes_approval_to_guardian(&self, turn: &Turn) -> bool;

    fn new_guardian_review_id(&self) -> String;

    fn guardian_rejection_message<'a>(
        &'a self,
        session: &'a Session,
        review_id: &'a str,
    ) -> impl Future<Output = String> + Send + 'a;

    fn guardian_timeout_message(&self) -> String;

    fn run_permission_request_hooks<'a>(
        &'a self,
        session: &'a Session,
        turn: &'a Turn,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<PermissionRequestDecision>> + Send + 'a;

    fn begin_network_approval<'a>(
        &'a self,
        session: &'a Session,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<Trigger>>,
    ) -> impl Future<Output = Option<Self::ActiveNetworkApproval>> + Send + 'a;

    fn active_network_approval_mode(
        &self,
        active: &Self::ActiveNetworkApproval,
    ) -> NetworkApprovalMode;

    fn active_network_approval_cancellation_token(
        &self,
        active: &Self::ActiveNetworkApproval,
    ) -> CancellationToken;

    fn into_deferred_network_approval(
        &self,
        active: Self::ActiveNetworkApproval,
    ) -> Option<Self::DeferredNetworkApproval>;

    fn finish_immediate_network_approval<'a>(
        &'a self,
        session: &'a Session,
        active: Self::ActiveNetworkApproval,
    ) -> impl Future<Output = Result<(), ToolError>> + Send + 'a;

    fn finish_deferred_network_approval<'a>(
        &'a self,
        session: &'a Session,
        deferred: Option<Self::DeferredNetworkApproval>,
    ) -> impl Future<Output = Result<(), ToolError>> + Send + 'a;
}

pub struct OrchestratorRunResult<Out, DeferredNetworkApproval> {
    pub output: Out,
    pub deferred_network_approval: Option<DeferredNetworkApproval>,
}
