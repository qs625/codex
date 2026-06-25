use codex_features::Feature;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_protocol::ThreadId;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_tool_config::ToolUserShellType;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ShellCommandHandlerHost;
use codex_tool_runtime_api::ShellExecutionHost;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use std::collections::HashMap;
use std::sync::Arc;

use crate::CoreApplyPatchHandlerHost;
use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::function_tool::FunctionCallError;
use crate::maybe_emit_implicit_skill_invocation;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::handlers::resolve_workdir_base_path;
use crate::tools::orchestrator::CoreToolOrchestratorHost;
use crate::tools::runtimes::CoreToolRuntimeHost;

impl ShellExecutionHost for CoreApplyPatchHandlerHost {
    type ShellHost = CoreToolRuntimeHost;
    type ShellOrchestratorHost = CoreToolOrchestratorHost;

    fn shell_runtime_host(&self) -> Self::ShellHost {
        CoreToolRuntimeHost
    }

    fn shell_orchestrator_host(&self) -> Self::ShellOrchestratorHost {
        CoreToolOrchestratorHost
    }

    fn primary_environment(
        &self,
        turn: &Arc<TurnContext>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        Ok(turn.primary_apply_patch_environment())
    }

    async fn dependency_env(&self, session: &Arc<Session>) -> HashMap<String, String> {
        session.dependency_env().await
    }

    fn explicit_env_overrides(&self, turn: &Arc<TurnContext>) -> HashMap<String, String> {
        turn.explicit_shell_env_overrides()
    }

    fn exec_permission_approvals_enabled(&self, session: &Arc<Session>) -> bool {
        session.enabled(Feature::ExecPermissionApprovals)
    }

    fn request_permissions_tool_enabled(&self, session: &Arc<Session>) -> bool {
        session.enabled(Feature::RequestPermissionsTool)
    }

    async fn create_exec_approval_requirement(
        &self,
        session: &Arc<Session>,
        request: ExecPolicyApprovalRequest<'_>,
    ) -> codex_tool_runtime_api::ExecApprovalRequirement {
        session.create_exec_approval_requirement(request).await
    }

    fn truncation_policy(&self, turn: &Arc<TurnContext>) -> TruncationPolicy {
        turn.truncation_policy()
    }
}

impl ShellCommandHandlerHost for CoreApplyPatchHandlerHost {
    fn resolve_workdir_base_path(
        &self,
        turn: &Arc<TurnContext>,
        arguments: &str,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        resolve_workdir_base_path(arguments, &turn.legacy_cwd())
    }

    fn parse_shell_command_params(
        &self,
        arguments: &str,
        base_path: &AbsolutePathBuf,
    ) -> Result<ShellCommandToolCallParams, FunctionCallError> {
        parse_arguments_with_base_path(arguments, base_path)
    }

    fn resolve_shell_workdir(
        &self,
        turn: &Arc<TurnContext>,
        workdir: Option<String>,
    ) -> AbsolutePathBuf {
        turn.resolve_shell_workdir(workdir)
    }

    async fn maybe_emit_implicit_skill_invocation(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        command: &str,
        workdir: &AbsolutePathBuf,
    ) {
        maybe_emit_implicit_skill_invocation(session.as_ref(), turn.as_ref(), command, workdir)
            .await;
    }

    fn shell_command_exec_params(
        &self,
        params: &ShellCommandToolCallParams,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
    ) -> Result<ExecParams, FunctionCallError> {
        shell_command_exec_params(
            params,
            session.as_ref(),
            turn.as_ref(),
            session.thread_id(),
            turn.allow_login_shell(),
        )
    }

    fn shell_type(&self, session: &Arc<Session>) -> Option<ToolUserShellType> {
        Some(session.tool_user_shell_type())
    }
}

pub(crate) fn shell_command_exec_params(
    params: &ShellCommandToolCallParams,
    session: &Session,
    turn_context: &TurnContext,
    thread_id: ThreadId,
    allow_login_shell: bool,
) -> Result<ExecParams, FunctionCallError> {
    let use_login_shell = resolve_use_login_shell(params.login, allow_login_shell)?;
    let command = session.derive_shell_exec_args(&params.command, use_login_shell);
    let cwd = turn_context.resolve_shell_workdir(params.workdir.clone());

    Ok(ExecParams {
        command,
        cwd,
        expiration: params.timeout_ms.into(),
        capture_policy: ExecCapturePolicy::ShellTool,
        env: turn_context.shell_exec_env(thread_id),
        network: turn_context.managed_network(),
        sandbox_permissions: params.sandbox_permissions.unwrap_or_default(),
        windows_sandbox_level: turn_context.windows_sandbox_level(),
        windows_sandbox_private_desktop: turn_context.windows_sandbox_private_desktop(),
        justification: params.justification.clone(),
        arg0: None,
    })
}

pub(crate) fn resolve_use_login_shell(
    login: Option<bool>,
    allow_login_shell: bool,
) -> Result<bool, FunctionCallError> {
    if !allow_login_shell && login == Some(true) {
        return Err(FunctionCallError::RespondToModel(
            "login shell is disabled by config; omit `login` or set it to false.".to_string(),
        ));
    }

    Ok(login.unwrap_or(allow_login_shell))
}

#[cfg(test)]
#[path = "shell_tool_host_tests.rs"]
mod tests;
