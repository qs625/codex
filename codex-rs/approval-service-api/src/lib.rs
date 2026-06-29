use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_command_service_api::ExecApprovalRequirement;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::error::CodexErr;
use codex_command_service_api::UnifiedExecApprovalKey;
use codex_protocol::protocol::FileChange;
use codex_utils_absolute_path::AbsolutePathBuf;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use tokio_util::sync::CancellationToken;

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
    pub sandbox_permissions: codex_protocol::models::SandboxPermissions,
    pub additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
    pub tty: bool,
    pub exec_approval_requirement: ExecApprovalRequirement,
    pub approval_keys: Vec<UnifiedExecApprovalKey>,
    pub network_approval_context: Option<codex_protocol::approvals::NetworkApprovalContext>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRuntimeNetworkApprovalTrigger {
    pub call_id: String,
    pub tool_name: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_permissions: codex_protocol::models::SandboxPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}

#[derive(Debug)]
pub enum ToolRuntimeNetworkApprovalError {
    Rejected(String),
    Codex(CodexErr),
}

pub trait ToolRuntimeNetworkApprovalHandle: Send + Sync + 'static {
    fn mode(&self) -> NetworkApprovalMode;

    fn registration_id(&self) -> Option<String>;

    fn cancellation_token(&self) -> CancellationToken;

    fn finish<'a>(
        &'a self,
    ) -> ApprovalServiceFuture<'a, Result<(), ToolRuntimeNetworkApprovalError>>;
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
