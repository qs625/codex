use std::sync::Arc;

use crate::sandboxing::ExecRequest;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_runtime_support::ApprovalCtx;
use crate::tool_runtime_support::SandboxAttempt;
use crate::tool_runtime_support::SandboxAttemptExt;
use crate::tool_runtime_support::ToolCtx;
use crate::tool_runtime_support::ToolError;
use crate::tool_runtime_support::with_cached_approval;
use crate::unified_exec::ExecServerEnvConfig;
use crate::unified_exec::SpawnLifecycleHandle;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::UnifiedExecProcess;
use crate::unified_exec::UnifiedExecProcessManager;
use codex_command_runtime::ExecOptions;
use codex_exec_server_api::ExecEnvironment;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::protocol::ReviewDecision;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_tool_runtime_api::PreparedUnifiedExecSpawn;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::UnifiedExecApprovalKey;
use codex_tool_runtime_api::UnifiedExecRequest;
use codex_tool_runtime_api::UnifiedExecRuntimeHost;
use futures::future::BoxFuture;

pub(crate) struct ThreadUnifiedExecRuntimeHost<'a> {
    pub(crate) manager: &'a UnifiedExecProcessManager,
}

impl UnifiedExecRuntimeHost for ThreadUnifiedExecRuntimeHost<'_> {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type ExecRequest = ExecRequest;
    type NetworkApprovalTrigger = ToolRuntimeNetworkApprovalTrigger;

    fn user_shell(&self, session: &Self::Session) -> RuntimeShell {
        session.runtime_shell()
    }

    fn network_approval_trigger(
        &self,
        req: &UnifiedExecRequest,
        ctx: &ToolCtx,
    ) -> Self::NetworkApprovalTrigger {
        ToolRuntimeNetworkApprovalTrigger {
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
        attempt.env_for(command, options, network)
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
            crate::unified_exec::zsh_fork_backend::maybe_prepare_unified_exec(
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
