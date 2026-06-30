use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_guardian::GuardianApprovalRequest;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::approvals::NetworkApprovalContext;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use codex_utils_absolute_path::AbsolutePathBuf;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;

/// Boxed future returned by object-safe approval service APIs.
pub type ApprovalServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecCommandApprovalOutcome {
    ContinueInRuntime,
    Preapproved,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub struct ApplyPatchApprovalKey {
    pub environment_id: String,
    pub path: AbsolutePathBuf,
}

/// Routed apply-patch approval payload owned by approval-service.
pub struct ApplyPatchApprovalRequest {
    pub cwd: AbsolutePathBuf,
    pub files: Vec<AbsolutePathBuf>,
    pub patch: String,
}

/// Apply-patch approval request routed through the approval service.
pub struct ApplyPatchApprovalDispatch {
    pub session: Arc<dyn ThreadSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub call_id: String,
    pub approval_keys: Vec<ApplyPatchApprovalKey>,
    pub approval_request: ApplyPatchApprovalRequest,
    pub changes: HashMap<PathBuf, FileChange>,
    pub permissions_preapproved: bool,
    pub retry_reason: Option<String>,
}

pub struct ExecCommandApprovalDispatch {
    pub session: Arc<dyn ThreadSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub call_id: String,
    pub command: Vec<String>,
    pub hook_command: String,
    pub cwd: std::path::PathBuf,
    pub reason: Option<String>,
    pub justification: Option<String>,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub tty: bool,
    pub exec_approval_requirement: ExecCommandApprovalRequirement,
    pub approval_keys: Vec<ExecCommandApprovalKey>,
    pub network_approval_context: Option<NetworkApprovalContext>,
}

pub struct GuardianReviewDispatch {
    pub session: Arc<dyn ThreadSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
    pub review_id: String,
    pub request: GuardianApprovalRequest,
    pub retry_reason: Option<String>,
}

pub struct GuardianReviewResult {
    pub decision: ReviewDecision,
    pub decline_message: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ExecCommandApprovalRequirement {
    Skip {
        bypass_sandbox: bool,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    NeedsApproval {
        reason: Option<String>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    },
    Forbidden {
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub struct ExecCommandApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
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

    fn review_guardian_request(
        &self,
        request: GuardianReviewDispatch,
    ) -> ApprovalServiceFuture<'_, GuardianReviewResult>;
}
