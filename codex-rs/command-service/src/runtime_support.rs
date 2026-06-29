//! Shared sandbox transform helpers used by command-service internals.

use crate::exec_request::ExecOptions;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_permissions_runtime::ExecApprovalRequirement;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxRuntime;
use codex_sandboxing_api::SandboxTransformError;
use codex_sandboxing_api::SandboxTransformRequest;
use codex_sandboxing_api::SandboxType;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum ToolError {
    Rejected(String),
    Codex(codex_protocol::error::CodexErr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SandboxOverride {
    NoOverride,
    BypassSandboxFirstAttempt,
}

pub(crate) fn sandbox_override_for_first_attempt(
    sandbox_permissions: SandboxPermissions,
    exec_approval_requirement: &ExecApprovalRequirement,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> SandboxOverride {
    if matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip {
            bypass_sandbox: true,
            ..
        }
    ) {
        return SandboxOverride::BypassSandboxFirstAttempt;
    }

    if file_system_sandbox_policy.has_denied_read_restrictions() {
        return SandboxOverride::NoOverride;
    }

    if sandbox_permissions.requires_escalated_permissions() {
        SandboxOverride::BypassSandboxFirstAttempt
    } else {
        SandboxOverride::NoOverride
    }
}

pub(crate) fn managed_network_for_sandbox_permissions(
    network: Option<&SharedNetworkProxyRuntime>,
    sandbox_permissions: SandboxPermissions,
) -> Option<SharedNetworkProxyRuntime> {
    if sandbox_permissions.requires_escalated_permissions() {
        None
    } else {
        network.cloned()
    }
}

pub(crate) fn wants_no_sandbox_approval(policy: AskForApproval) -> bool {
    match policy {
        AskForApproval::OnFailure => true,
        AskForApproval::UnlessTrusted => true,
        AskForApproval::Never => false,
        AskForApproval::OnRequest => false,
        AskForApproval::Granular(granular_config) => granular_config.sandbox_approval,
    }
}

pub(crate) struct SandboxAttempt<'a> {
    pub sandbox: SandboxType,
    pub permissions: &'a PermissionProfile,
    pub enforce_managed_network: bool,
    pub sandbox_runtime: &'a dyn SandboxRuntime,
    pub sandbox_cwd: &'a codex_utils_absolute_path::AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<&'a std::path::PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub network_denial_cancellation_token: Option<CancellationToken>,
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
