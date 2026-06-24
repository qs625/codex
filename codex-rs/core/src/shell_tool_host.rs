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

use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::function_tool::FunctionCallError;
use crate::maybe_emit_implicit_skill_invocation;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::handlers::CoreToolDomainHost;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::handlers::resolve_workdir_base_path;
use crate::tools::orchestrator::CoreToolOrchestratorHost;
use crate::tools::runtimes::CoreApplyPatchEnvironment;
use crate::tools::runtimes::CoreToolRuntimeHost;

impl ShellExecutionHost for CoreToolDomainHost {
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
        Ok(turn
            .environments
            .primary()
            .map(|turn_environment| ResolvedApplyPatchEnvironment {
                cwd: turn_environment.cwd.clone(),
                environment: CoreApplyPatchEnvironment::new(turn_environment.clone()),
            }))
    }

    async fn dependency_env(&self, session: &Arc<Session>) -> HashMap<String, String> {
        session.dependency_env().await
    }

    fn explicit_env_overrides(&self, turn: &Arc<TurnContext>) -> HashMap<String, String> {
        turn.shell_environment_policy.r#set.clone()
    }

    fn exec_permission_approvals_enabled(&self, session: &Arc<Session>) -> bool {
        session.features().enabled(Feature::ExecPermissionApprovals)
    }

    fn request_permissions_tool_enabled(&self, session: &Arc<Session>) -> bool {
        session.features().enabled(Feature::RequestPermissionsTool)
    }

    async fn create_exec_approval_requirement(
        &self,
        session: &Arc<Session>,
        request: ExecPolicyApprovalRequest<'_>,
    ) -> codex_tool_runtime_api::ExecApprovalRequirement {
        session
            .services
            .exec_policy
            .create_exec_approval_requirement_for_command(request)
            .await
    }

    fn truncation_policy(&self, turn: &Arc<TurnContext>) -> TruncationPolicy {
        turn.truncation_policy
    }
}

impl ShellCommandHandlerHost for CoreToolDomainHost {
    fn resolve_workdir_base_path(
        &self,
        turn: &Arc<TurnContext>,
        arguments: &str,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        resolve_workdir_base_path(arguments, &turn.cwd)
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
        #[allow(deprecated)]
        turn.resolve_path(workdir)
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
            session.conversation_id,
            turn.tools_config.allow_login_shell,
        )
    }

    fn shell_type(&self, session: &Arc<Session>) -> Option<ToolUserShellType> {
        Some(crate::tools::runtimes::runtime_shell_type(
            &session.user_shell().shell_type,
        ))
    }
}

pub(crate) fn shell_command_exec_params(
    params: &ShellCommandToolCallParams,
    session: &Session,
    turn_context: &TurnContext,
    thread_id: ThreadId,
    allow_login_shell: bool,
) -> Result<ExecParams, FunctionCallError> {
    let shell = session.user_shell();
    let use_login_shell = resolve_use_login_shell(params.login, allow_login_shell)?;
    let command = shell.derive_exec_args(&params.command, use_login_shell);
    #[allow(deprecated)]
    let cwd = turn_context.resolve_path(params.workdir.clone());

    Ok(ExecParams {
        command,
        cwd,
        expiration: params.timeout_ms.into(),
        capture_policy: ExecCapturePolicy::ShellTool,
        env: create_env(&turn_context.shell_environment_policy, Some(thread_id)),
        network: turn_context.network.clone(),
        sandbox_permissions: params.sandbox_permissions.unwrap_or_default(),
        windows_sandbox_level: turn_context.windows_sandbox_level,
        windows_sandbox_private_desktop: turn_context
            .config
            .permissions
            .windows_sandbox_private_desktop,
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
