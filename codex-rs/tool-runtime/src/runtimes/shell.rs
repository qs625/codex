use crate::Approvable;
use crate::ApprovalCtx;
use crate::ExecApprovalRequirement;
use crate::NetworkApprovalMode;
use crate::NetworkApprovalSpec;
use crate::PermissionRequestPayload;
use crate::SandboxAttempt;
use crate::Sandboxable;
use crate::ToolCtx;
use crate::ToolError;
use crate::ToolRuntime;
use crate::build_sandbox_command;
use crate::disable_powershell_profile_for_elevated_windows_sandbox;
use crate::exec_env_for_sandbox_permissions;
use crate::flat_tool_name;
use crate::managed_network_for_sandbox_permissions;
use crate::maybe_wrap_shell_lc_with_snapshot;
use codex_command_runtime::ExecCapturePolicy;
use codex_command_runtime::ExecExpiration;
use codex_command_runtime::ExecOptions;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxablePreference;
use codex_shell_command::canonicalize_command_for_approval;
use codex_shell_command::powershell::prefix_powershell_script_with_utf8;
use codex_tool_config::ToolUserShellType;
pub use codex_tool_runtime_api::ShellApprovalKey;
pub use codex_tool_runtime_api::ShellRequest;
pub use codex_tool_runtime_api::ShellRuntimeBackend;
pub use codex_tool_runtime_api::ShellRuntimeHost;
use futures::future::BoxFuture;

pub struct ShellRuntime<Host> {
    backend: ShellRuntimeBackend,
    host: Host,
}

impl<Host> ShellRuntime<Host> {
    pub fn for_shell_command(host: Host, backend: ShellRuntimeBackend) -> Self {
        Self { backend, host }
    }
}

impl<Host> Sandboxable for ShellRuntime<Host>
where
    Host: Send + Sync,
{
    fn sandbox_preference(&self) -> SandboxablePreference {
        SandboxablePreference::Auto
    }

    fn escalate_on_failure(&self) -> bool {
        true
    }
}

impl<Host> Approvable<ShellRequest> for ShellRuntime<Host>
where
    Host: ShellRuntimeHost,
{
    type Session = Host::Session;
    type Turn = Host::Turn;
    type ApprovalKey = ShellApprovalKey;

    fn approval_keys(&self, req: &ShellRequest) -> Vec<Self::ApprovalKey> {
        vec![ShellApprovalKey {
            command: canonicalize_command_for_approval(&req.command),
            cwd: req.cwd.clone(),
            sandbox_permissions: req.sandbox_permissions,
            additional_permissions: req.additional_permissions.clone(),
        }]
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a ShellRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        self.host.start_shell_approval_async(req, ctx, keys)
    }

    fn exec_approval_requirement(&self, req: &ShellRequest) -> Option<ExecApprovalRequirement> {
        Some(req.exec_approval_requirement.clone())
    }

    fn permission_request_payload(&self, req: &ShellRequest) -> Option<PermissionRequestPayload> {
        Some(PermissionRequestPayload::bash(
            req.hook_command.clone(),
            req.justification.clone(),
        ))
    }

    fn sandbox_permissions(&self, req: &ShellRequest) -> SandboxPermissions {
        req.sandbox_permissions
    }
}

impl<Host> ToolRuntime<ShellRequest, ExecToolCallOutput> for ShellRuntime<Host>
where
    Host: ShellRuntimeHost,
{
    type NetworkApprovalTrigger = Host::NetworkApprovalTrigger;

    fn network_approval_spec(
        &self,
        req: &ShellRequest,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Option<NetworkApprovalSpec<Self::NetworkApprovalTrigger>> {
        managed_network_for_sandbox_permissions(req.network.as_ref(), req.sandbox_permissions)?;
        Some(NetworkApprovalSpec {
            network: req.network.clone(),
            mode: NetworkApprovalMode::Immediate,
            trigger: self.host.network_approval_trigger(req, ctx),
            command: req.hook_command.clone(),
        })
    }

    async fn run(
        &mut self,
        req: &ShellRequest,
        attempt: &SandboxAttempt<'_>,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Result<ExecToolCallOutput, ToolError> {
        let session_shell = self.host.user_shell(&ctx.session);
        let managed_network =
            managed_network_for_sandbox_permissions(req.network.as_ref(), req.sandbox_permissions);
        let env = exec_env_for_sandbox_permissions(&req.env, req.sandbox_permissions);
        let command = maybe_wrap_shell_lc_with_snapshot(
            &req.command,
            &session_shell,
            &req.cwd,
            &req.explicit_env_overrides,
            &env,
        );
        let command = disable_powershell_profile_for_elevated_windows_sandbox(
            &command,
            req.shell_type,
            attempt.sandbox,
            attempt.windows_sandbox_level,
        );
        let command = if matches!(session_shell.shell_type, ToolUserShellType::PowerShell) {
            prefix_powershell_script_with_utf8(&command)
        } else {
            command
        };

        if self.backend == ShellRuntimeBackend::ShellCommandZshFork {
            match self
                .host
                .maybe_run_shell_command_zsh_fork(req, attempt, ctx, &command)
                .await?
            {
                Some(out) => return Ok(out),
                None => {
                    tracing::warn!(
                        "ZshFork backend specified, but conditions for using it were not met, falling back to normal execution",
                    );
                }
            }
        }

        let command =
            build_sandbox_command(&command, &req.cwd, &env, req.additional_permissions.clone())?;
        let mut expiration: ExecExpiration = req.timeout_ms.into();
        if let Some(cancellation) = attempt.network_denial_cancellation_token.clone() {
            expiration = expiration.with_cancellation(cancellation);
        }
        let options = ExecOptions {
            expiration,
            capture_policy: ExecCapturePolicy::ShellTool,
        };
        let exec_request = self
            .host
            .transform_sandbox_attempt(attempt, command, options, managed_network)
            .map_err(|err| ToolError::Codex(err.into()))?;
        let out = self
            .host
            .execute_env(exec_request, self.host.stdout_stream(ctx))
            .await
            .map_err(ToolError::Codex)?;
        Ok(out)
    }
}

pub fn shell_network_approval_command_name(tool_name: &codex_tool_planning::ToolName) -> String {
    flat_tool_name(tool_name).into_owned()
}
