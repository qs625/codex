use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_protocol::protocol::FileChange;
use codex_tool_runtime_api::ExecApprovalRequirement;
use codex_thread_api::ToolServiceSessionRef;
use codex_thread_api::ToolServiceTurnRef;
use codex_tool_runtime_api::ApplyPatchApprovalKey;
use codex_tool_runtime_api::ApplyPatchApprovalRequest;

/// Boxed future returned by object-safe approval service APIs.
pub type ApprovalServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecCommandApprovalOutcome {
    ContinueInRuntime,
    Preapproved,
}

/// Apply-patch approval request routed through the approval service.
pub struct ApplyPatchApprovalDispatch {
    pub session: Arc<dyn ToolServiceSessionRef>,
    pub turn: Arc<dyn ToolServiceTurnRef>,
    pub call_id: String,
    pub approval_keys: Vec<ApplyPatchApprovalKey>,
    pub approval_request: ApplyPatchApprovalRequest,
    pub changes: HashMap<PathBuf, FileChange>,
    pub permissions_preapproved: bool,
    pub retry_reason: Option<String>,
}

pub struct ExecCommandApprovalDispatch {
    pub session: Arc<dyn ToolServiceSessionRef>,
    pub turn: Arc<dyn ToolServiceTurnRef>,
    pub call_id: String,
    pub command: Vec<String>,
    pub hook_command: String,
    pub cwd: std::path::PathBuf,
    pub reason: Option<String>,
    pub justification: Option<String>,
    pub sandbox_permissions: codex_protocol::models::SandboxPermissions,
    pub additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
    pub tty: bool,
    pub exec_approval_requirement: ExecApprovalRequirement,
    pub approval_keys: Vec<codex_tool_runtime_api::UnifiedExecApprovalKey>,
    pub network_approval_context: Option<codex_protocol::approvals::NetworkApprovalContext>,
}

/// Global approval service API.
///
/// Tool and other domain services should depend on this trait instead of
/// reaching into thread/session runtime types for approval orchestration.
pub trait ApprovalServiceApi: Send + Sync + 'static {
    fn request_apply_patch_approval(
        &self,
        request: ApplyPatchApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<(), String>>;

    fn request_exec_command_approval(
        &self,
        request: ExecCommandApprovalDispatch,
    ) -> ApprovalServiceFuture<'_, Result<ExecCommandApprovalOutcome, String>>;
}
