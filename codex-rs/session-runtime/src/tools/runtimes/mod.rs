/*
Module: runtimes

Concrete ToolRuntime implementations for specific tools. Each runtime stays
small and focused and reuses the orchestrator for approvals + sandbox + retry.
*/
use crate::shell::Shell;
use crate::shell::ShellType;
use crate::tools::sandboxing::ApprovalCtx;
use crate::tools::sandboxing::SandboxAttempt;
use crate::tools::sandboxing::SandboxAttemptExt;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::with_cached_approval;
use crate::unified_exec::ExecServerEnvConfig;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::UnifiedExecProcess;
use crate::unified_exec::UnifiedExecProcessManager;
use codex_command_runtime::ExecOptions;
use codex_command_runtime::SpawnLifecycleHandle;
use codex_exec_server_api::ExecEnvironment;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_tool_config::ToolUserShellType;
use codex_tool_runtime_api::ApplyPatchApprovalKey;
use codex_tool_runtime_api::ApplyPatchApprovalRequest;
use codex_tool_runtime_api::ApplyPatchEnvironment;
use codex_tool_runtime_api::ApplyPatchRequest;
use codex_tool_runtime_api::ApplyPatchRuntimeHost;
use codex_tool_runtime_api::PreparedUnifiedExecSpawn;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::RuntimeShellSnapshot;
use codex_tool_runtime_api::ShellApprovalKey;
use codex_tool_runtime_api::ShellRequest;
use codex_tool_runtime_api::ShellRuntimeHost;
use codex_tool_runtime_api::UnifiedExecApprovalKey;
use codex_tool_runtime_api::UnifiedExecRequest;
use codex_tool_runtime_api::UnifiedExecRuntimeHost;
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) use codex_tool_runtime::build_sandbox_command;
pub(crate) use codex_tool_runtime::exec_env_for_sandbox_permissions;

pub(crate) mod shell;

pub(crate) fn maybe_wrap_shell_lc_with_snapshot(
    command: &[String],
    session_shell: &Shell,
    cwd: &AbsolutePathBuf,
    explicit_env_overrides: &HashMap<String, String>,
    env: &HashMap<String, String>,
) -> Vec<String> {
    codex_tool_runtime::maybe_wrap_shell_lc_with_snapshot(
        command,
        &runtime_shell(session_shell),
        cwd,
        explicit_env_overrides,
        env,
    )
}

pub(crate) fn runtime_shell(session_shell: &Shell) -> RuntimeShell {
    RuntimeShell {
        shell_type: runtime_shell_type(&session_shell.shell_type),
        shell_path: session_shell.shell_path.clone(),
        shell_snapshot: session_shell
            .shell_snapshot()
            .map(|snapshot| RuntimeShellSnapshot {
                path: snapshot.path.clone(),
                cwd: snapshot.cwd.clone(),
            }),
    }
}

pub(crate) fn runtime_shell_type(shell_type: &ShellType) -> ToolUserShellType {
    match shell_type {
        ShellType::Zsh => ToolUserShellType::Zsh,
        ShellType::Bash => ToolUserShellType::Bash,
        ShellType::PowerShell => ToolUserShellType::PowerShell,
        ShellType::Sh => ToolUserShellType::Sh,
        ShellType::Cmd => ToolUserShellType::Cmd,
    }
}

#[derive(Clone, Copy)]
pub struct CoreToolRuntimeHost;

pub(crate) struct CoreUnifiedExecRuntimeHost<'a> {
    pub(crate) manager: &'a UnifiedExecProcessManager,
}

pub(crate) struct CoreApplyPatchEnvironment {
    turn_environment: crate::session::turn_context::TurnEnvironment,
}

impl CoreApplyPatchEnvironment {
    pub(crate) fn new(
        turn_environment: crate::session::turn_context::TurnEnvironment,
    ) -> Arc<Self> {
        Arc::new(Self { turn_environment })
    }
}

impl ApplyPatchEnvironment for CoreApplyPatchEnvironment {
    fn environment_id(&self) -> &str {
        &self.turn_environment.environment_id
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.turn_environment.environment.get_filesystem()
    }
}

fn transform_sandbox_attempt(
    attempt: &SandboxAttempt<'_>,
    command: SandboxCommand,
    options: ExecOptions,
    network: Option<SharedNetworkProxyRuntime>,
) -> Result<crate::sandboxing::ExecRequest, SandboxTransformError> {
    attempt.env_for(command, options, network)
}

fn core_runtime_shell(session: &Arc<crate::session::session::Session>) -> RuntimeShell {
    runtime_shell(session.user_shell().as_ref())
}

impl ShellRuntimeHost for CoreToolRuntimeHost {
    type Session = Arc<crate::session::session::Session>;
    type Turn = Arc<crate::session::turn_context::TurnContext>;
    type ExecRequest = crate::sandboxing::ExecRequest;
    type StdoutStream = crate::exec::StdoutStream;
    type NetworkApprovalTrigger = crate::guardian::GuardianNetworkAccessTrigger;

    fn user_shell(&self, session: &Self::Session) -> RuntimeShell {
        core_runtime_shell(session)
    }

    fn stdout_stream(&self, ctx: &ToolCtx) -> Option<Self::StdoutStream> {
        Some(crate::exec::StdoutStream {
            sub_id: ctx.turn.sub_id.clone(),
            call_id: ctx.call_id.clone(),
            tx_event: ctx.session.get_tx_event(),
        })
    }

    fn network_approval_trigger(
        &self,
        req: &ShellRequest,
        ctx: &ToolCtx,
    ) -> Self::NetworkApprovalTrigger {
        crate::guardian::GuardianNetworkAccessTrigger {
            call_id: ctx.call_id.clone(),
            tool_name: codex_tool_runtime::flat_tool_name(&ctx.tool_name).into_owned(),
            command: req.command.clone(),
            cwd: req.cwd.clone(),
            sandbox_permissions: req.sandbox_permissions,
            additional_permissions: req.additional_permissions.clone(),
            justification: req.justification.clone(),
            tty: None,
        }
    }

    fn start_shell_approval_async<'a>(
        &'a self,
        req: &'a ShellRequest,
        ctx: ApprovalCtx<'a>,
        keys: Vec<ShellApprovalKey>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let command = req.command.clone();
        let cwd = req.cwd.clone();
        let retry_reason = ctx.retry_reason.clone();
        let reason = retry_reason.clone().or_else(|| req.justification.clone());
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let guardian_review_id = ctx.guardian_review_id.clone();
        Box::pin(async move {
            if let Some(review_id) = guardian_review_id {
                return crate::guardian::review_approval_request(
                    session,
                    turn,
                    review_id,
                    crate::guardian::GuardianApprovalRequest::Shell {
                        id: call_id,
                        command,
                        cwd: cwd.clone(),
                        sandbox_permissions: req.sandbox_permissions,
                        additional_permissions: req.additional_permissions.clone(),
                        justification: req.justification.clone(),
                    },
                    retry_reason,
                )
                .await;
            }
            with_cached_approval(&session.services, "shell", keys, move || async move {
                let available_decisions = None;
                session
                    .request_command_approval(
                        turn,
                        call_id,
                        /*approval_id*/ None,
                        command,
                        cwd,
                        reason,
                        ctx.network_approval_context.clone(),
                        req.exec_approval_requirement
                            .proposed_execpolicy_amendment()
                            .cloned(),
                        req.additional_permissions.clone(),
                        available_decisions,
                    )
                    .await
            })
            .await
        })
    }

    fn transform_sandbox_attempt(
        &self,
        attempt: &SandboxAttempt<'_>,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<Self::ExecRequest, SandboxTransformError> {
        transform_sandbox_attempt(attempt, command, options, network)
    }

    fn execute_env<'a>(
        &'a self,
        exec_request: Self::ExecRequest,
        stdout_stream: Option<Self::StdoutStream>,
    ) -> BoxFuture<'a, codex_protocol::error::Result<ExecToolCallOutput>> {
        Box::pin(async move { crate::sandboxing::execute_env(exec_request, stdout_stream).await })
    }

    fn maybe_run_shell_command_zsh_fork<'a>(
        &'a self,
        req: &'a ShellRequest,
        attempt: &'a SandboxAttempt<'_>,
        ctx: &'a ToolCtx,
        command: &'a [String],
    ) -> BoxFuture<'a, Result<Option<ExecToolCallOutput>, ToolError>> {
        Box::pin(async move {
            shell::zsh_fork_backend::maybe_run_shell_command(req, attempt, ctx, command).await
        })
    }
}

impl ApplyPatchRuntimeHost for CoreToolRuntimeHost {
    type Session = Arc<crate::session::session::Session>;
    type Turn = Arc<crate::session::turn_context::TurnContext>;
    type NetworkApprovalTrigger = crate::guardian::GuardianNetworkAccessTrigger;

    fn start_apply_patch_approval_async<'a>(
        &'a self,
        req: &'a ApplyPatchRequest,
        ctx: ApprovalCtx<'a>,
        keys: Vec<ApplyPatchApprovalKey>,
        approval_request: ApplyPatchApprovalRequest,
    ) -> BoxFuture<'a, ReviewDecision> {
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let retry_reason = ctx.retry_reason.clone();
        let changes = req.changes.clone();
        let guardian_review_id = ctx.guardian_review_id.clone();
        Box::pin(async move {
            if let Some(review_id) = guardian_review_id {
                return crate::guardian::review_approval_request(
                    session,
                    turn,
                    review_id,
                    crate::guardian::GuardianApprovalRequest::ApplyPatch {
                        id: call_id,
                        cwd: approval_request.cwd,
                        files: approval_request.files,
                        patch: approval_request.patch,
                    },
                    retry_reason,
                )
                .await;
            }
            if req.permissions_preapproved && retry_reason.is_none() {
                return ReviewDecision::Approved;
            }
            if let Some(reason) = retry_reason {
                let rx_approve = session
                    .request_patch_approval(
                        turn,
                        call_id,
                        changes.clone(),
                        Some(reason),
                        /*grant_root*/ None,
                    )
                    .await;
                return rx_approve.await.unwrap_or_default();
            }

            with_cached_approval(&session.services, "apply_patch", keys, || async move {
                let rx_approve = session
                    .request_patch_approval(
                        turn, call_id, changes, /*reason*/ None, /*grant_root*/ None,
                    )
                    .await;
                rx_approve.await.unwrap_or_default()
            })
            .await
        })
    }
}

impl UnifiedExecRuntimeHost for CoreUnifiedExecRuntimeHost<'_> {
    type Session = Arc<crate::session::session::Session>;
    type Turn = Arc<crate::session::turn_context::TurnContext>;
    type ExecRequest = crate::sandboxing::ExecRequest;
    type NetworkApprovalTrigger = crate::guardian::GuardianNetworkAccessTrigger;

    fn user_shell(&self, session: &Self::Session) -> RuntimeShell {
        core_runtime_shell(session)
    }

    fn network_approval_trigger(
        &self,
        req: &UnifiedExecRequest,
        ctx: &ToolCtx,
    ) -> Self::NetworkApprovalTrigger {
        crate::guardian::GuardianNetworkAccessTrigger {
            call_id: ctx.call_id.clone(),
            tool_name: codex_tool_runtime::flat_tool_name(&ctx.tool_name).into_owned(),
            command: req.command.clone(),
            cwd: req.cwd.clone(),
            sandbox_permissions: req.sandbox_permissions,
            additional_permissions: req.additional_permissions.clone(),
            justification: req.justification.clone(),
            tty: Some(req.tty),
        }
    }

    fn start_unified_exec_approval_async<'a>(
        &'a self,
        req: &'a UnifiedExecRequest,
        ctx: ApprovalCtx<'a>,
        keys: Vec<UnifiedExecApprovalKey>,
    ) -> BoxFuture<'a, ReviewDecision> {
        let session = ctx.session;
        let turn = ctx.turn;
        let call_id = ctx.call_id.to_string();
        let command = req.command.clone();
        let cwd = req.cwd.clone();
        let retry_reason = ctx.retry_reason.clone();
        let reason = retry_reason.clone().or_else(|| req.justification.clone());
        let guardian_review_id = ctx.guardian_review_id.clone();
        Box::pin(async move {
            if let Some(review_id) = guardian_review_id {
                return crate::guardian::review_approval_request(
                    session,
                    turn,
                    review_id,
                    crate::guardian::GuardianApprovalRequest::ExecCommand {
                        id: call_id,
                        command,
                        cwd: cwd.clone(),
                        sandbox_permissions: req.sandbox_permissions,
                        additional_permissions: req.additional_permissions.clone(),
                        justification: req.justification.clone(),
                        tty: req.tty,
                    },
                    retry_reason,
                )
                .await;
            }
            with_cached_approval(&session.services, "unified_exec", keys, || async move {
                let available_decisions = None;
                session
                    .request_command_approval(
                        turn,
                        call_id,
                        /*approval_id*/ None,
                        command,
                        cwd.clone(),
                        reason,
                        ctx.network_approval_context.clone(),
                        req.exec_approval_requirement
                            .proposed_execpolicy_amendment()
                            .cloned(),
                        req.additional_permissions.clone(),
                        available_decisions,
                    )
                    .await
            })
            .await
        })
    }

    fn transform_sandbox_attempt(
        &self,
        attempt: &SandboxAttempt<'_>,
        command: SandboxCommand,
        options: ExecOptions,
        network: Option<SharedNetworkProxyRuntime>,
    ) -> Result<Self::ExecRequest, SandboxTransformError> {
        transform_sandbox_attempt(attempt, command, options, network)
    }

    fn maybe_prepare_unified_exec_zsh_fork<'a>(
        &'a self,
        req: &'a UnifiedExecRequest,
        attempt: &'a SandboxAttempt<'_>,
        ctx: &'a ToolCtx,
        exec_request: Self::ExecRequest,
        zsh_fork_config: &'a codex_tool_config::ZshForkConfig,
    ) -> BoxFuture<'a, Result<Option<PreparedUnifiedExecSpawn<Self::ExecRequest>>, ToolError>> {
        Box::pin(async move {
            shell::zsh_fork_backend::maybe_prepare_unified_exec(
                req,
                attempt,
                ctx,
                exec_request,
                zsh_fork_config,
            )
            .await
        })
    }

    fn open_session_with_exec_env<'a>(
        &'a self,
        process_id: i32,
        request: &'a Self::ExecRequest,
        exec_server_env_config: Option<&'a ExecServerEnvConfig>,
        tty: bool,
        spawn_lifecycle: SpawnLifecycleHandle,
        environment: &'a dyn ExecEnvironment,
    ) -> BoxFuture<'a, Result<UnifiedExecProcess, UnifiedExecError>> {
        Box::pin(async move {
            self.manager
                .open_session_with_exec_env(
                    process_id,
                    request,
                    exec_server_env_config,
                    tty,
                    spawn_lifecycle,
                    environment,
                )
                .await
        })
    }
}

#[cfg(all(test, unix))]
#[path = "mod_tests.rs"]
mod tests;
