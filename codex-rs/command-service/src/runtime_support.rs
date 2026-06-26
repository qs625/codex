//! Shared approvals and sandboxing traits used by tool runtimes.
//!
//! Consolidates the approval flow primitives (`ApprovalDecision`, `ApprovalStore`,
//! `ApprovalCtx`, `Approvable`) together with the sandbox orchestration traits
//! and helpers (`Sandboxable`, `ToolRuntime`, `SandboxAttempt`, etc.).

use crate::exec_request::ExecOptions;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_sandboxing_api::SandboxTransformRequest;
use crate::adapters::SessionCapabilityAdapter;
use crate::adapters::TurnCapabilityAdapter;
pub(crate) type ApprovalCtx<'a> = codex_tool_runtime_api::ApprovalCtx<
    'a,
    std::sync::Arc<SessionCapabilityAdapter>,
    std::sync::Arc<TurnCapabilityAdapter>,
>;
pub(crate) use codex_tool_runtime_api::PermissionRequestPayload;
pub(crate) use codex_tool_runtime_api::SandboxAttempt;
pub(crate) type ToolCtx = codex_tool_runtime_api::ToolCtx<
    std::sync::Arc<SessionCapabilityAdapter>,
    std::sync::Arc<TurnCapabilityAdapter>,
>;
pub(crate) use codex_tool_runtime_api::ToolError;
pub(crate) use codex_tool_runtime_api::managed_network_for_sandbox_permissions;

pub(crate) fn permission_request_hook_payload(
    payload: PermissionRequestPayload,
) -> codex_hooks::PermissionRequestHookPayload {
    codex_hooks::PermissionRequestHookPayload {
        tool_name: payload.tool_name.name().to_string(),
        matcher_aliases: payload.tool_name.matcher_aliases().to_vec(),
        tool_input: payload.tool_input,
    }
}

pub(crate) trait SandboxAttemptExt {
    fn env_for(
        &self,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<crate::exec_request::ExecRequest, SandboxTransformError>;
}

impl SandboxAttemptExt for SandboxAttempt<'_> {
    fn env_for(
        &self,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<crate::exec_request::ExecRequest, SandboxTransformError> {
        let network_snapshot = network.as_ref().map(|network| network.runtime_snapshot());
        self.sandbox_runtime
            .transform(SandboxTransformRequest {
                command,
                permissions: self.permissions,
                sandbox: self.sandbox,
                enforce_managed_network: self.enforce_managed_network,
                network: network_snapshot.as_ref(),
                sandbox_policy_cwd: self.sandbox_cwd,
                codex_linux_sandbox_exe: self
                    .codex_linux_sandbox_exe
                    .map(std::path::PathBuf::as_path),
                use_legacy_landlock: self.use_legacy_landlock,
                windows_sandbox_level: self.windows_sandbox_level,
                windows_sandbox_private_desktop: self.windows_sandbox_private_desktop,
            })
            .map(|request| {
                let windows_sandbox_policy_cwd =
                    codex_utils_absolute_path::AbsolutePathBuf::try_from(
                        self.sandbox_cwd.to_path_buf(),
                    )
                    .unwrap_or_else(|_| request.cwd.clone());
                crate::exec_request::ExecRequest::from_sandbox_exec_request(
                    request,
                    options,
                    windows_sandbox_policy_cwd,
                    network,
                )
            })
    }
}
