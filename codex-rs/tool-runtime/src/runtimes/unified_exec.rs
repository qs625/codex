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
use crate::managed_network_for_sandbox_permissions;
use crate::maybe_wrap_shell_lc_with_snapshot;
use codex_command_runtime::ExecCapturePolicy;
use codex_command_runtime::ExecExpiration;
use codex_command_runtime::ExecOptions;
use codex_command_runtime::NoopSpawnLifecycle;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::UnifiedExecProcess;
use codex_protocol::error::CodexErr;
use codex_protocol::error::SandboxErr;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxablePreference;
use codex_shell_command::canonicalize_command_for_approval;
use codex_shell_command::powershell::prefix_powershell_script_with_utf8;
use codex_tool_config::ToolUserShellType;
use codex_tool_config::UnifiedExecShellMode;
use codex_tool_runtime_api::ExecCommandApprovalMode;
pub use codex_tool_runtime_api::PreparedUnifiedExecSpawn;
pub use codex_tool_runtime_api::UnifiedExecApprovalKey;
pub use codex_tool_runtime_api::UnifiedExecRequest;
pub use codex_tool_runtime_api::UnifiedExecRuntimeHost;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

/// Runtime adapter that keeps policy and sandbox orchestration on the
/// unified-exec side while delegating process startup to the host.
pub struct UnifiedExecRuntime<Host> {
    host: Host,
    shell_mode: UnifiedExecShellMode,
}

pub fn unified_exec_options(
    network_denial_cancellation_token: Option<CancellationToken>,
) -> ExecOptions {
    let mut expiration = ExecExpiration::DefaultTimeout;
    if let Some(cancellation) = network_denial_cancellation_token {
        expiration = expiration.with_cancellation(cancellation);
    }
    ExecOptions {
        expiration,
        capture_policy: ExecCapturePolicy::ShellTool,
    }
}

impl<Host> UnifiedExecRuntime<Host> {
    pub fn new(host: Host, shell_mode: UnifiedExecShellMode) -> Self {
        Self { host, shell_mode }
    }
}

impl<Host> Sandboxable for UnifiedExecRuntime<Host>
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

impl<Host> Approvable<UnifiedExecRequest> for UnifiedExecRuntime<Host>
where
    Host: UnifiedExecRuntimeHost,
{
    type Session = Host::Session;
    type Turn = Host::Turn;
    type ApprovalKey = UnifiedExecApprovalKey;

    fn approval_keys(&self, req: &UnifiedExecRequest) -> Vec<Self::ApprovalKey> {
        vec![UnifiedExecApprovalKey {
            command: canonicalize_command_for_approval(&req.command),
            cwd: req.cwd.clone(),
            tty: req.tty,
            sandbox_permissions: req.sandbox_permissions,
            additional_permissions: req.additional_permissions.clone(),
        }]
    }

    fn start_approval_async<'a>(
        &'a mut self,
        req: &'a UnifiedExecRequest,
        ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let keys = self.approval_keys(req);
        self.host.start_unified_exec_approval_async(req, ctx, keys)
    }

    fn exec_approval_requirement(
        &self,
        req: &UnifiedExecRequest,
    ) -> Option<ExecApprovalRequirement> {
        Some(req.exec_approval_requirement.clone())
    }

    fn permission_request_payload(
        &self,
        req: &UnifiedExecRequest,
    ) -> Option<PermissionRequestPayload> {
        Some(PermissionRequestPayload::bash(
            req.hook_command.clone(),
            req.justification.clone(),
        ))
    }

    fn sandbox_permissions(&self, req: &UnifiedExecRequest) -> SandboxPermissions {
        req.sandbox_permissions
    }

    fn approval_preapproved(&self, req: &UnifiedExecRequest) -> bool {
        matches!(req.approval_mode, ExecCommandApprovalMode::AlreadyApproved)
    }
}

impl<Host> ToolRuntime<UnifiedExecRequest, UnifiedExecProcess> for UnifiedExecRuntime<Host>
where
    Host: UnifiedExecRuntimeHost,
{
    type NetworkApprovalTrigger = Host::NetworkApprovalTrigger;

    fn sandbox_cwd<'a>(&self, req: &'a UnifiedExecRequest) -> Option<&'a AbsolutePathBuf> {
        Some(&req.sandbox_cwd)
    }

    fn network_approval_spec(
        &self,
        req: &UnifiedExecRequest,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Option<NetworkApprovalSpec<Self::NetworkApprovalTrigger>> {
        managed_network_for_sandbox_permissions(req.network.as_ref(), req.sandbox_permissions)?;
        Some(NetworkApprovalSpec {
            network: req.network.clone(),
            mode: NetworkApprovalMode::Deferred,
            trigger: self.host.network_approval_trigger(req, ctx),
            command: req.hook_command.clone(),
        })
    }

    async fn run(
        &mut self,
        req: &UnifiedExecRequest,
        attempt: &SandboxAttempt<'_>,
        ctx: &ToolCtx<Self::Session, Self::Turn>,
    ) -> Result<UnifiedExecProcess, ToolError> {
        let base_command = &req.command;
        let session_shell = self.host.user_shell(&ctx.session);
        let managed_network =
            managed_network_for_sandbox_permissions(req.network.as_ref(), req.sandbox_permissions);
        let mut env = exec_env_for_sandbox_permissions(&req.env, req.sandbox_permissions);
        if let Some(network) = managed_network.as_ref() {
            network.apply_to_env(&mut env);
        }
        let environment_is_remote = req.environment.is_remote();
        let command = if environment_is_remote {
            base_command.to_vec()
        } else {
            maybe_wrap_shell_lc_with_snapshot(
                base_command,
                &session_shell,
                &req.cwd,
                &req.explicit_env_overrides,
                &env,
            )
        };
        let command = disable_powershell_profile_for_elevated_windows_sandbox(
            &command,
            Some(req.shell_type),
            attempt.sandbox,
            attempt.windows_sandbox_level,
        );
        let command = if matches!(session_shell.shell_type, ToolUserShellType::PowerShell) {
            prefix_powershell_script_with_utf8(&command)
        } else {
            command
        };

        if let UnifiedExecShellMode::ZshFork(zsh_fork_config) = &self.shell_mode {
            let command =
                build_sandbox_command(&command, &req.cwd, &env, req.additional_permissions.clone())
                    .map_err(|_| ToolError::Rejected("missing command line for PTY".to_string()))?;
            let options = unified_exec_options(attempt.network_denial_cancellation_token.clone());
            let exec_request = self
                .host
                .transform_sandbox_attempt(attempt, command, options, managed_network.clone())
                .map_err(|err| ToolError::Codex(err.into()))?;
            match self
                .host
                .maybe_prepare_unified_exec_zsh_fork(
                    req,
                    attempt,
                    ctx,
                    exec_request,
                    zsh_fork_config,
                )
                .await?
            {
                Some(prepared) => {
                    if req.environment.is_remote() {
                        return Err(ToolError::Rejected(
                            "unified_exec zsh-fork is not supported for remote environments"
                                .to_string(),
                        ));
                    }
                    return self
                        .host
                        .open_session_with_exec_env(
                            req.process_id,
                            &prepared.exec_request,
                            req.exec_server_env_config.as_ref(),
                            req.tty,
                            prepared.spawn_lifecycle,
                            req.environment.as_ref(),
                        )
                        .await
                        .map_err(unified_exec_error_to_tool_error);
                }
                None => {
                    tracing::warn!(
                        "UnifiedExec ZshFork backend specified, but conditions for using it were not met, falling back to direct execution",
                    );
                }
            }
        }
        let command =
            build_sandbox_command(&command, &req.cwd, &env, req.additional_permissions.clone())
                .map_err(|_| ToolError::Rejected("missing command line for PTY".to_string()))?;
        let options = unified_exec_options(attempt.network_denial_cancellation_token.clone());
        let exec_request = self
            .host
            .transform_sandbox_attempt(attempt, command, options, managed_network)
            .map_err(|err| ToolError::Codex(err.into()))?;
        self.host
            .open_session_with_exec_env(
                req.process_id,
                &exec_request,
                req.exec_server_env_config.as_ref(),
                req.tty,
                Box::new(NoopSpawnLifecycle),
                req.environment.as_ref(),
            )
            .await
            .map_err(unified_exec_error_to_tool_error)
    }
}

fn unified_exec_error_to_tool_error(err: UnifiedExecError) -> ToolError {
    match err {
        UnifiedExecError::SandboxDenied { output, .. } => {
            ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output: Box::new(output),
                network_policy_decision: None,
            }))
        }
        other => ToolError::Rejected(other.to_string()),
    }
}
