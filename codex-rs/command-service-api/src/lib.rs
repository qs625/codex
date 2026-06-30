mod command_contracts;
mod command_types;
mod session_controller;
mod unified_exec_error;

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
pub use command_contracts::CommandNotifyOnArg;
pub use command_contracts::ExecApprovalRequirement;
pub use command_contracts::ExecCommandApprovalMode;
pub use command_contracts::ExecCommandArgs;
pub use command_contracts::ExecCommandRunOutput;
pub use command_contracts::ExecCommandRunRequest;
pub use command_contracts::ResolvedExecCommand;
pub use command_contracts::RuntimeShell;
pub use command_contracts::RuntimeShellSnapshot;
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

pub use codex_sandboxing_api::ApplyPatchEnvironment;
pub use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
pub use codex_sandboxing_api::ResolvedExecCommandEnvironment;
pub use codex_sandboxing_api::ToolSandboxContext;
use codex_protocol::ThreadId;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::error::CodexErr;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::TerminalInteractionEvent;
use thread_service_api::ThreadCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadRuntimeCapability;
pub use thread_service_api::HookToolName;
pub use thread_service_api::PermissionRequestPayload;
pub use thread_service_api::ToolPermissionGrants;
use codex_tool_config::ToolUserShellType;
use codex_utils_absolute_path::AbsolutePathBuf;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// Boxed future returned by object-safe command service APIs.
pub type CommandServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRuntimeNetworkApprovalTrigger {
    pub call_id: String,
    pub tool_name: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_permissions: codex_protocol::models::SandboxPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<codex_network_proxy_api::SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}

#[derive(Debug)]
pub enum ToolRuntimeNetworkApprovalError {
    Rejected(String),
    Codex(CodexErr),
}

pub trait ToolRuntimeNetworkApprovalHandle: Send + Sync + 'static {
    fn mode(&self) -> NetworkApprovalMode;

    fn registration_id(&self) -> Option<String>;

    fn cancellation_token(&self) -> CancellationToken;

    fn finish<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<(), ToolRuntimeNetworkApprovalError>> + Send + 'a>>;
}

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
        self.as_ref()
            .emit_model_item_started_display_event(turn, item)
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
        session: Arc<dyn ThreadSessionCapability>,
        session_api: Arc<dyn CommandServiceSessionApi>,
        turn: Arc<dyn ThreadRuntimeCapability>,
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

/// Session-owned command runtime inputs that remain command-domain specific.
pub trait CommandServiceSessionApi: Send + Sync + 'static {
    fn command_service_state(&self) -> Arc<dyn CommandServiceSessionState>;

    fn runtime_shell(&self) -> RuntimeShell;

    fn tool_user_shell_type(&self) -> ToolUserShellType;

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool>;

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'a>,
    ) -> CommandServiceFuture<'a, ExecApprovalRequirement>;

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        turn: &'a dyn ThreadRuntimeCapability,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> CommandServiceFuture<'a, ()>;

    fn exec_permission_approvals_enabled(&self) -> bool;

    fn request_permissions_tool_enabled(&self) -> bool;

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell;

    fn resolve_exec_command(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String>;

    fn shell_env_overrides(&self) -> HashMap<String, String>;

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf;

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> CommandServiceFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>>;

    #[allow(clippy::too_many_arguments)]
    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadRuntimeCapability,
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
}

/// Command execution service API used by tool domains.
pub trait CommandServiceApi: Send + Sync + 'static {
    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn ThreadSessionCapability>,
        session_api: Arc<dyn CommandServiceSessionApi>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>>;

    fn begin_command_wait<'a>(
        &'a self,
        state: Arc<dyn CommandServiceSessionState>,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        state: Arc<dyn CommandServiceSessionState>,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;
}
