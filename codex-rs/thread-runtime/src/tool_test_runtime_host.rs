use crate::tool_runtime_support::ApprovalCtx;
use crate::tool_runtime_support::SandboxAttempt;
use crate::tool_runtime_support::SandboxAttemptExt;
use crate::tool_runtime_support::ToolCtx;
use crate::tool_runtime_support::ToolError;
use crate::tool_runtime_support::with_cached_approval;
use codex_command_runtime::ExecOptions;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::protocol::ReviewDecision;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_tool_runtime_api::ApplyPatchApprovalKey;
use codex_tool_runtime_api::ApplyPatchApprovalRequest;
use codex_tool_runtime_api::ApplyPatchRequest;
use codex_tool_runtime_api::ApplyPatchRuntimeHost;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::ShellApprovalKey;
use codex_tool_runtime_api::ShellRequest;
use codex_tool_runtime_api::ShellRuntimeHost;
use futures::future::BoxFuture;
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
#[path = "tool_test_runtime_host_zsh_fork_backend.rs"]
mod tool_test_runtime_host_zsh_fork_backend;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
pub struct CoreToolRuntimeHost;

fn transform_sandbox_attempt(
    attempt: &SandboxAttempt<'_>,
    command: SandboxCommand,
    options: ExecOptions,
    network: Option<SharedNetworkProxyRuntime>,
) -> Result<crate::sandboxing::ExecRequest, SandboxTransformError> {
    attempt.env_for(command, options, network)
}

fn core_runtime_shell(session: &Arc<crate::session::session::Session>) -> RuntimeShell {
    crate::runtime_shell::runtime_shell(session.user_shell().as_ref())
}

#[cfg(any(test, feature = "test-support"))]
impl ShellRuntimeHost for CoreToolRuntimeHost {
    type Session = Arc<crate::session::session::Session>;
    type Turn = Arc<crate::session::turn_context::TurnContext>;
    type ExecRequest = crate::sandboxing::ExecRequest;
    type StdoutStream = crate::exec::StdoutStream;
    type NetworkApprovalTrigger = ToolRuntimeNetworkApprovalTrigger;

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
        ToolRuntimeNetworkApprovalTrigger {
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
            tool_test_runtime_host_zsh_fork_backend::maybe_run_shell_command(
                req, attempt, ctx, command,
            )
            .await
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ApplyPatchRuntimeHost for CoreToolRuntimeHost {
    type Session = Arc<crate::session::session::Session>;
    type Turn = Arc<crate::session::turn_context::TurnContext>;
    type NetworkApprovalTrigger = ToolRuntimeNetworkApprovalTrigger;

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

#[cfg(all(test, unix))]
#[path = "tool_test_runtime_host_tests.rs"]
mod tests;
