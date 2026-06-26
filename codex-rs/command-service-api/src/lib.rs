use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_hooks_api::PermissionRequestDecision;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::ThreadId;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_thread_api::ThreadCapability;
use codex_tool_config::UnifiedExecShellMode;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::PermissionRequestPayload;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::ToolSandboxContext;
use tokio::sync::watch;

/// Boxed future returned by object-safe command service APIs.
pub type CommandServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Object-safe turn capability consumed by command service.
pub trait CommandServiceTurnCapability: ThreadCapability + Send + Sync + 'static {
    fn runtime_turn_id_str(&self) -> &str;

    fn runtime_turn_id(&self) -> String;

    fn can_request_original_image_detail(&self) -> bool;

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<codex_tool_runtime_api::ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError>;

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext;

    fn single_local_environment_cwd(
        &self,
    ) -> Result<codex_utils_absolute_path::AbsolutePathBuf, codex_tool_types::FunctionCallError>;

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64>;

    fn routes_approval_to_guardian(&self) -> bool;

    fn tool_sandbox_context(&self) -> ToolSandboxContext;

    fn approval_policy(&self) -> AskForApproval;

    fn shell_environment_policy(&self) -> ShellEnvironmentPolicy;

    fn unified_exec_shell_mode(&self) -> UnifiedExecShellMode;

    fn allow_login_shell(&self) -> bool;

    fn active_network(&self) -> Option<SharedNetworkProxyRuntime>;

    fn emit_unified_exec_tty_metric(&self, tty: bool);
}

/// Per-session command runtime state owned by command-service.
pub trait CommandServiceSessionState: Send + Sync + 'static {
    fn allocate_process_id<'a>(&'a self) -> CommandServiceFuture<'a, i32>;

    fn release_process_id<'a>(&'a self, process_id: i32) -> CommandServiceFuture<'a, ()>;

    fn has_running_process_for_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> CommandServiceFuture<'a, bool>;

    fn terminate_all_processes<'a>(&'a self) -> CommandServiceFuture<'a, ()>;

    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        turn: Arc<dyn CommandServiceTurnCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>>;

    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;
}

/// Object-safe session capability consumed by command service.
pub trait CommandServiceSessionCapability: Send + Sync + 'static {
    fn conversation_id(&self) -> ThreadId;

    fn command_service_state(&self) -> Arc<dyn CommandServiceSessionState>;

    fn sandbox_runtime(&self) -> SharedSandboxRuntime;

    fn runtime_shell(&self) -> RuntimeShell;

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool>;

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'a>,
    ) -> CommandServiceFuture<'a, codex_tool_runtime_api::ExecApprovalRequirement>;

    fn strict_auto_review_enabled_for_turn<'a>(
        &'a self,
    ) -> CommandServiceFuture<'a, bool>;

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> CommandServiceFuture<'a, String>;

    fn guardian_timeout_message(&self) -> String;

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> CommandServiceFuture<'a, Option<PermissionRequestDecision>>;

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> CommandServiceFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>>;

    fn request_command_approval<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<codex_protocol::protocol::NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> CommandServiceFuture<'a, ReviewDecision>;

    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        call_id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        sandbox_permissions: codex_protocol::models::SandboxPermissions,
        tty: bool,
        network_approval_context: Option<codex_protocol::protocol::NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cache_keys: Vec<codex_tool_runtime_api::UnifiedExecApprovalKey>,
    ) -> CommandServiceFuture<'a, ReviewDecision>;

    fn unregister_network_approval<'a>(
        &'a self,
        registration_id: &'a str,
    ) -> CommandServiceFuture<'a, ()>;

    fn send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> CommandServiceFuture<'a, ()>;

    fn send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: ExecCommandEndEvent,
    ) -> CommandServiceFuture<'a, ()>;

    fn send_event<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: EventMsg,
    ) -> CommandServiceFuture<'a, ()>;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        items: &'a [ResponseItem],
    ) -> CommandServiceFuture<'a, ()>;
}

/// Command execution service API used by tool domains.
pub trait CommandServiceApi: Send + Sync + 'static {
    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        turn: Arc<dyn CommandServiceTurnCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>>;

    fn begin_command_wait<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;
}
