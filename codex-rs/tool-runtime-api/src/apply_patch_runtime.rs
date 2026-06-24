use crate::ApprovalCtx;
use crate::ExecApprovalRequirement;
use codex_apply_patch::ApplyPatchAction;
use codex_file_system::ExecutorFileSystem;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq, Hash, serde::Serialize)]
pub struct ApplyPatchApprovalKey {
    pub environment_id: String,
    pub path: AbsolutePathBuf,
}

/// Filesystem/environment boundary needed by the apply-patch runtime.
pub trait ApplyPatchEnvironment: Send + Sync {
    fn environment_id(&self) -> &str;

    fn filesystem(&self) -> Arc<dyn ExecutorFileSystem>;
}

pub struct ApplyPatchRequest {
    pub environment: Arc<dyn ApplyPatchEnvironment>,
    pub action: ApplyPatchAction,
    pub file_paths: Vec<AbsolutePathBuf>,
    pub changes: HashMap<PathBuf, FileChange>,
    pub exec_approval_requirement: ExecApprovalRequirement,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub permissions_preapproved: bool,
}

pub struct ApplyPatchApprovalRequest {
    pub cwd: AbsolutePathBuf,
    pub files: Vec<AbsolutePathBuf>,
    pub patch: String,
}

impl ApplyPatchApprovalRequest {
    pub fn from_request(req: &ApplyPatchRequest) -> Self {
        Self {
            cwd: req.action.cwd.clone(),
            files: req.file_paths.clone(),
            patch: req.action.patch.clone(),
        }
    }
}

/// Host bridge for apply-patch approval effects.
pub trait ApplyPatchRuntimeHost: Send + Sync {
    type Session: Send + Sync;
    type Turn: Send + Sync;
    type NetworkApprovalTrigger;

    fn start_apply_patch_approval_async<'a>(
        &'a self,
        req: &'a ApplyPatchRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
        keys: Vec<ApplyPatchApprovalKey>,
        approval_request: ApplyPatchApprovalRequest,
    ) -> BoxFuture<'a, ReviewDecision>;
}
