use crate::ApprovalCtx;
use crate::ExecApprovalRequirement;
use crate::RuntimeShell;
use crate::SandboxAttempt;
use crate::ToolCtx;
use crate::ToolError;
use codex_command_runtime::ExecOptions;
use codex_command_runtime::ExecServerEnvConfig;
use codex_command_runtime::SpawnLifecycleHandle;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::UnifiedExecProcess;
use codex_exec_server_api::ExecEnvironment;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_tool_config::ToolUserShellType;
use codex_tool_config::ZshForkConfig;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;

/// Request payload used by the unified-exec runtime after approvals and
/// sandbox preferences have been resolved for the current turn.
#[derive(Clone)]
pub struct UnifiedExecRequest {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
    pub hook_command: String,
    pub process_id: i32,
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub env: HashMap<String, String>,
    pub exec_server_env_config: Option<ExecServerEnvConfig>,
    pub explicit_env_overrides: HashMap<String, String>,
    pub network: Option<SharedNetworkProxyRuntime>,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[cfg(unix)]
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub exec_approval_requirement: ExecApprovalRequirement,
}

/// Cache key for approval decisions that can be reused across equivalent
/// unified-exec launches.
#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct UnifiedExecApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

pub struct PreparedUnifiedExecSpawn<ExecRequest> {
    pub exec_request: ExecRequest,
    pub spawn_lifecycle: SpawnLifecycleHandle,
}

/// Host bridge for unified-exec runtime effects owned by composition roots.
pub trait UnifiedExecRuntimeHost: Send + Sync {
    type Session: Send + Sync;
    type Turn: Send + Sync;
    type ExecRequest: Send + 'static;
    type NetworkApprovalTrigger;

    fn user_shell(&self, session: &Self::Session) -> RuntimeShell;

    fn network_approval_trigger(
        &self,
        req: &UnifiedExecRequest,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Self::NetworkApprovalTrigger;

    fn start_unified_exec_approval_async<'a>(
        &'a self,
        req: &'a UnifiedExecRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
        keys: Vec<UnifiedExecApprovalKey>,
    ) -> BoxFuture<'a, ReviewDecision>;

    fn transform_sandbox_attempt(
        &self,
        attempt: &SandboxAttempt<'_>,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<Self::ExecRequest, SandboxTransformError>;

    fn maybe_prepare_unified_exec_zsh_fork<'a>(
        &'a self,
        req: &'a UnifiedExecRequest,
        attempt: &'a SandboxAttempt<'_>,
        ctx: &'a ToolCtx<Self::Session, Self::Turn>,
        exec_request: Self::ExecRequest,
        zsh_fork_config: &'a ZshForkConfig,
    ) -> BoxFuture<'a, Result<Option<PreparedUnifiedExecSpawn<Self::ExecRequest>>, ToolError>>;

    fn open_session_with_exec_env<'a>(
        &'a self,
        process_id: i32,
        request: &'a Self::ExecRequest,
        exec_server_env_config: Option<&'a ExecServerEnvConfig>,
        tty: bool,
        spawn_lifecycle: SpawnLifecycleHandle,
        environment: &'a dyn ExecEnvironment,
    ) -> BoxFuture<'a, Result<UnifiedExecProcess, UnifiedExecError>>;
}
