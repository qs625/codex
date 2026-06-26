use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use crate::CapabilityToolOrchestratorHost;
use codex_file_system::FileSystemSandboxContext;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_thread_api::SessionToolEventHost;
use codex_thread_api::SharedToolTurnDiffTracker;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::ExecCommandHandlerHost;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ResolvedExecCommand;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::ShellExecutionHost;
use codex_tool_runtime_api::ShellRuntimeHost as ShellRuntimeHostTrait;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_types::FunctionCallError;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;

pub struct CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost> {
    runtime_host: RuntimeHost,
    _marker: PhantomData<fn() -> (Session, Turn, DiffContext)>,
}

impl<Session, Turn, DiffContext, RuntimeHost> CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost> {
    pub fn new(runtime_host: RuntimeHost) -> Self {
        Self {
            runtime_host,
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, DiffContext, RuntimeHost> Clone
    for CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost>
where
    RuntimeHost: Clone,
{
    fn clone(&self) -> Self {
        Self::new(self.runtime_host.clone())
    }
}

impl<Session, Turn, DiffContext, RuntimeHost> ApplyPatchHandlerHost
    for CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost>
where
    Session: ToolRuntimeSessionCapability,
    Turn: Clone + ToolRuntimeTurnCapability,
    DiffContext: ApplyPatchDiffContext + 'static,
    RuntimeHost: Clone
        + codex_tool_runtime_api::ApplyPatchRuntimeHost<
            Session = Arc<Session>,
            Turn = Turn,
            NetworkApprovalTrigger = codex_thread_api::ToolRuntimeNetworkApprovalTrigger,
        >
        + Send
        + Sync
        + 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = SharedToolTurnDiffTracker;
    type DiffContext = DiffContext;
    type RuntimeHost = RuntimeHost;
    type OrchestratorHost = CapabilityToolOrchestratorHost;
    type EventHost<'a>
        = SessionToolEventHost<'a, Session, Turn>
    where
        Self: 'a,
        Session: 'a,
        Turn: 'a;

    fn runtime_host(&self) -> Self::RuntimeHost {
        self.runtime_host.clone()
    }

    fn orchestrator_host(&self) -> Self::OrchestratorHost {
        CapabilityToolOrchestratorHost
    }

    fn sandbox_runtime(&self, session: &Self::Session) -> codex_sandboxing_api::SharedSandboxRuntime {
        session.sandbox_runtime()
    }

    fn tool_sandbox_context(&self, turn: &Self::Turn) -> codex_tool_runtime_api::ToolSandboxContext {
        turn.tool_sandbox_context()
    }

    fn approval_policy(&self, turn: &Self::Turn) -> AskForApproval {
        turn.approval_policy()
    }

    fn permission_profile(&self, turn: &Self::Turn) -> PermissionProfile {
        turn.permission_profile()
    }

    fn file_system_sandbox_policy(&self, turn: &Self::Turn) -> FileSystemSandboxPolicy {
        turn.file_system_sandbox_policy()
    }

    fn windows_sandbox_level(&self, turn: &Self::Turn) -> WindowsSandboxLevel {
        turn.windows_sandbox_level()
    }

    fn file_system_sandbox_context(
        &self,
        turn: &Self::Turn,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        codex_thread_api::ToolRuntimeTurnCapability::file_system_sandbox_context(
            turn,
            additional_permissions,
            cwd,
        )
    }

    fn resolve_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        turn.resolve_apply_patch_environment(environment_id)
    }

    fn permission_grants<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl std::future::Future<Output = ToolPermissionGrants> + Send + 'a {
        session.tool_permission_grants()
    }

    fn event_host<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        tracker: Option<&'a Self::Tracker>,
    ) -> Self::EventHost<'a> {
        SessionToolEventHost::new(session.as_ref(), turn, tracker)
    }
}

impl<Session, Turn, DiffContext, RuntimeHost> ShellExecutionHost
    for CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost>
where
    Session: ToolRuntimeSessionCapability,
    Turn: Clone + ToolRuntimeTurnCapability,
    DiffContext: ApplyPatchDiffContext + 'static,
    RuntimeHost: Clone
        + codex_tool_runtime_api::ApplyPatchRuntimeHost<
            Session = Arc<Session>,
            Turn = Turn,
            NetworkApprovalTrigger = codex_thread_api::ToolRuntimeNetworkApprovalTrigger,
        >
        + ShellRuntimeHostTrait<
            Session = Arc<Session>,
            Turn = Turn,
            NetworkApprovalTrigger = codex_thread_api::ToolRuntimeNetworkApprovalTrigger,
        >
        + Send
        + Sync
        + 'static,
{
    type ShellHost = RuntimeHost;
    type ShellOrchestratorHost = CapabilityToolOrchestratorHost;

    fn shell_runtime_host(&self) -> Self::ShellHost {
        self.runtime_host.clone()
    }

    fn shell_orchestrator_host(&self) -> Self::ShellOrchestratorHost {
        CapabilityToolOrchestratorHost
    }

    fn primary_environment(
        &self,
        turn: &Self::Turn,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        Ok(turn.primary_apply_patch_environment())
    }

    fn dependency_env<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl std::future::Future<Output = HashMap<String, String>> + Send + 'a {
        session.dependency_env()
    }

    fn explicit_env_overrides(&self, turn: &Self::Turn) -> HashMap<String, String> {
        turn.explicit_shell_env_overrides()
    }

    fn exec_permission_approvals_enabled(&self, session: &Self::Session) -> bool {
        session.exec_permission_approvals_enabled()
    }

    fn request_permissions_tool_enabled(&self, session: &Self::Session) -> bool {
        session.request_permissions_tool_enabled()
    }

    fn create_exec_approval_requirement<'a>(
        &'a self,
        session: &'a Self::Session,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> impl std::future::Future<Output = codex_tool_runtime_api::ExecApprovalRequirement> + Send + 'a
    {
        session.create_exec_approval_requirement(request)
    }

    fn truncation_policy(&self, turn: &Self::Turn) -> TruncationPolicy {
        turn.truncation_policy()
    }
}

impl<Session, Turn, DiffContext, RuntimeHost> ExecCommandHandlerHost
    for CapabilityToolHost<Session, Turn, DiffContext, RuntimeHost>
where
    Session: ToolRuntimeSessionCapability,
    Turn: Clone + ToolRuntimeTurnCapability,
    DiffContext: ApplyPatchDiffContext + 'static,
    RuntimeHost: Clone
        + codex_tool_runtime_api::ApplyPatchRuntimeHost<
            Session = Arc<Session>,
            Turn = Turn,
            NetworkApprovalTrigger = codex_thread_api::ToolRuntimeNetworkApprovalTrigger,
        >
        + ShellRuntimeHostTrait<
            Session = Arc<Session>,
            Turn = Turn,
            NetworkApprovalTrigger = codex_thread_api::ToolRuntimeNetworkApprovalTrigger,
        >
        + Send
        + Sync
        + 'static,
{
    fn resolve_exec_command_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError> {
        turn.resolve_exec_command_environment(environment_id, workdir)
    }

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell {
        RuntimeShell {
            shell_type: infer_shell_type(shell),
            shell_path: shell.to_path_buf(),
            shell_snapshot: None,
        }
    }

    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> Result<ResolvedExecCommand, String> {
        session.resolve_exec_command(turn, command, login, model_shell)
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        session.maybe_emit_implicit_skill_invocation(turn, command, workdir)
    }

    fn allocate_exec_process_id<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl std::future::Future<Output = i32> + Send + 'a {
        session.allocate_exec_process_id()
    }

    fn release_exec_process_id<'a>(
        &'a self,
        session: &'a Self::Session,
        process_id: i32,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        session.release_exec_process_id(process_id)
    }

    fn run_exec_command<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        request: ExecCommandRunRequest,
    ) -> impl std::future::Future<Output = Result<ExecCommandRunOutput, codex_command_runtime::UnifiedExecError>> + Send + 'a {
        session.run_exec_command(turn, call_id, request)
    }

    fn emit_unified_exec_tty_metric(&self, turn: &Self::Turn, tty: bool) {
        turn.emit_unified_exec_tty_metric(tty);
    }
}

fn infer_shell_type(shell: &Path) -> codex_tool_config::ToolUserShellType {
    let Some(name) = shell.file_stem().and_then(|value| value.to_str()) else {
        return codex_tool_config::ToolUserShellType::Sh;
    };
    match name {
        "zsh" => codex_tool_config::ToolUserShellType::Zsh,
        "bash" => codex_tool_config::ToolUserShellType::Bash,
        "pwsh" | "powershell" => codex_tool_config::ToolUserShellType::PowerShell,
        "cmd" => codex_tool_config::ToolUserShellType::Cmd,
        "sh" => codex_tool_config::ToolUserShellType::Sh,
        _ => codex_tool_config::ToolUserShellType::Sh,
    }
}
