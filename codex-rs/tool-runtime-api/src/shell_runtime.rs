use crate::ApprovalCtx;
use crate::ExecApprovalRequirement;
use crate::RuntimeShell;
use crate::SandboxAttempt;
use crate::ToolCtx;
use crate::ToolError;
use codex_command_runtime::ExecOptions;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_tool_config::ToolUserShellType;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct ShellRequest {
    pub command: Vec<String>,
    pub shell_type: Option<ToolUserShellType>,
    pub hook_command: String,
    pub cwd: AbsolutePathBuf,
    pub timeout_ms: Option<u64>,
    pub env: HashMap<String, String>,
    pub explicit_env_overrides: HashMap<String, String>,
    pub network: Option<SharedNetworkProxyRuntime>,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[cfg(unix)]
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub exec_approval_requirement: ExecApprovalRequirement,
}

/// Selects `ShellRuntime` behavior for different callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellRuntimeBackend {
    /// Legacy backend for the `shell_command` tool.
    ShellCommandClassic,
    /// zsh-fork backend for the `shell_command` tool.
    ShellCommandZshFork,
}

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct ShellApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

/// Host bridge for shell runtime effects that belong to composition roots.
///
/// Implementations connect the tool-domain runtime to session approval UI,
/// sandbox transformation, stdout streaming, and process execution without
/// making `codex-tool-runtime` depend on the core session or turn types.
pub trait ShellRuntimeHost: Send + Sync {
    type Session: Send + Sync;
    type Turn: Send + Sync;
    type ExecRequest: Send + 'static;
    type StdoutStream: Send + 'static;
    type NetworkApprovalTrigger;

    fn user_shell(&self, session: &Self::Session) -> RuntimeShell;

    fn stdout_stream(&self, ctx: &ToolCtx<Self::Session, Self::Turn>)
    -> Option<Self::StdoutStream>;

    fn network_approval_trigger(
        &self,
        req: &ShellRequest,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Self::NetworkApprovalTrigger;

    fn start_shell_approval_async<'a>(
        &'a self,
        req: &'a ShellRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
        keys: Vec<ShellApprovalKey>,
    ) -> BoxFuture<'a, ReviewDecision>;

    fn transform_sandbox_attempt(
        &self,
        attempt: &SandboxAttempt<'_>,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<Self::ExecRequest, SandboxTransformError>;

    fn execute_env<'a>(
        &'a self,
        exec_request: Self::ExecRequest,
        stdout_stream: Option<Self::StdoutStream>,
    ) -> BoxFuture<'a, codex_protocol::error::Result<ExecToolCallOutput>>;

    fn maybe_run_shell_command_zsh_fork<'a>(
        &'a self,
        req: &'a ShellRequest,
        attempt: &'a SandboxAttempt<'_>,
        ctx: &'a ToolCtx<Self::Session, Self::Turn>,
        command: &'a [String],
    ) -> BoxFuture<'a, Result<Option<ExecToolCallOutput>, ToolError>> {
        let _ = (req, attempt, ctx, command);
        Box::pin(async { Ok(None) })
    }
}
