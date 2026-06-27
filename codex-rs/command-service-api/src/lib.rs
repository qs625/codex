mod command_contracts;
mod command_types;
mod session_controller;
mod unified_exec_error;

pub use command_contracts::ApplyPatchEnvironment;
pub use command_contracts::CommandNotifyOnArg;
pub use command_contracts::ExecApprovalRequirement;
pub use command_contracts::ExecCommandArgs;
pub use command_contracts::ExecCommandApprovalMode;
pub use command_contracts::ExecCommandRunOutput;
pub use command_contracts::ExecCommandRunRequest;
pub use command_contracts::HookToolName;
pub use command_contracts::NetworkApprovalMode;
pub use command_contracts::NetworkApprovalSpec;
pub use command_contracts::PermissionRequestPayload;
pub use command_contracts::ResolvedApplyPatchEnvironment;
pub use command_contracts::ResolvedExecCommand;
pub use command_contracts::ResolvedExecCommandEnvironment;
pub use command_contracts::RuntimeShell;
pub use command_contracts::RuntimeShellSnapshot;
pub use command_contracts::ToolPermissionGrants;
pub use command_contracts::ToolSandboxContext;
pub use command_contracts::UnifiedExecApprovalKey;
pub use command_contracts::resolve_exec_command;
pub use command_contracts::resolve_exec_command_for_parts;
pub use command_types::CommandNotificationFilter;
pub use command_types::CommandNotificationKind;
pub use command_types::CommandWaitOutput;
pub use command_types::CommandWaitRequest;
pub use command_types::CommandWaitStatus;
pub use command_types::DEFAULT_COMMAND_OUTPUT_MAX_BYTES;
pub use command_types::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
pub use command_types::DEFAULT_MAX_OUTPUT_TOKENS;
pub use command_types::MAX_YIELD_TIME_MS;
pub use command_types::MIN_YIELD_TIME_MS;
pub use command_types::WaitBackoffState;
pub use command_types::WriteStdinOutput;
pub use command_types::WriteStdinRequest;
pub use command_types::clamp_yield_time;
pub use command_types::generate_chunk_id;
pub use command_types::resolve_max_tokens;
pub use codex_process_exec::DEFAULT_EXEC_COMMAND_TIMEOUT_MS;
pub use codex_process_exec::DEFAULT_EXEC_OUTPUT_MAX_BYTES;
pub use codex_process_exec::ExecCapturePolicy;
pub use codex_process_exec::ExecExpiration;
pub use codex_process_exec::ExecExpirationOutcome;
pub use codex_process_exec::ExecOptions;
pub use codex_process_exec::IO_DRAIN_TIMEOUT_MS;
pub use codex_process_exec::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
pub use codex_process_exec::bytes_to_string_smart;
pub use codex_process_exec::cancel_when_either;
pub use codex_process_exec::is_likely_sandbox_denied;
pub use session_controller::CommandSessionController;
pub use session_controller::CommandSessionError;
pub use session_controller::CommandSessionFuture;
pub use session_controller::CommandWaitOperation;
pub use unified_exec_error::UnifiedExecError;

use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use codex_hooks_api::PermissionRequestDecision;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::ThreadId;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_thread_api::ThreadCapability;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_tool_config::ToolUserShellType;
use codex_tool_config::UnifiedExecShellMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use tokio::sync::watch;

/// Boxed future returned by object-safe command service APIs.
pub type CommandServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Session-owned command interaction capability consumed by command tools.
pub trait SessionCommandInteractionCaller: Send + Sync + 'static {
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;

    fn emit_model_item_started_display_event<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        item: &'a ResponseItem,
    ) -> CommandServiceFuture<'a, ()>;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        items: &'a [ResponseItem],
    ) -> CommandServiceFuture<'a, ()>;

    fn send_terminal_interaction<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        event: TerminalInteractionEvent,
    ) -> CommandServiceFuture<'a, ()>;
}

impl<Session> SessionCommandInteractionCaller for Arc<Session>
where
    Session: SessionCommandInteractionCaller,
{
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>> {
        self.as_ref().begin_command_wait(request)
    }

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        self.as_ref().write_command_stdin(request)
    }

    fn emit_model_item_started_display_event<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        item: &'a ResponseItem,
    ) -> CommandServiceFuture<'a, ()> {
        self.as_ref().emit_model_item_started_display_event(turn, item)
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        items: &'a [ResponseItem],
    ) -> CommandServiceFuture<'a, ()> {
        self.as_ref()
            .record_model_items_and_emit_display_events(turn, items)
    }

    fn send_terminal_interaction<'a>(
        &'a self,
        turn: &'a dyn ThreadCapability,
        event: TerminalInteractionEvent,
    ) -> CommandServiceFuture<'a, ()> {
        self.as_ref().send_terminal_interaction(turn, event)
    }
}

/// Object-safe turn capability consumed by command service.
pub trait CommandServiceTurnCapability: ThreadCapability + Send + Sync + 'static {
    fn runtime_turn_id_str(&self) -> &str;

    fn runtime_turn_id(&self) -> String;

    fn can_request_original_image_detail(&self) -> bool;

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError>;

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

    fn permission_profile(&self) -> PermissionProfile;

    fn file_system_sandbox_policy(&self) -> codex_protocol::permissions::FileSystemSandboxPolicy;

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, codex_tool_types::FunctionCallError>;

    fn truncation_policy(&self) -> TruncationPolicy;
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

    fn tool_user_shell_type(&self) -> ToolUserShellType;

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool>;

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'a>,
    ) -> CommandServiceFuture<'a, ExecApprovalRequirement>;

    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> CommandServiceFuture<'a, bool>;

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> CommandServiceFuture<'a, String>;

    fn guardian_timeout_message(&self) -> String;

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> CommandServiceFuture<'a, ()>;

    fn exec_permission_approvals_enabled(&self) -> bool;

    fn request_permissions_tool_enabled(&self) -> bool;

    fn tool_permission_grants<'a>(&'a self) -> CommandServiceFuture<'a, ToolPermissionGrants>;

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell;

    fn resolve_exec_command(
        &self,
        turn: &dyn CommandServiceTurnCapability,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String>;

    fn shell_env_overrides(&self) -> HashMap<String, String>;

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf;

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
        cache_keys: Vec<UnifiedExecApprovalKey>,
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
