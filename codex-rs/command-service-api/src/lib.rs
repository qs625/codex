mod command_contracts;
mod command_types;
mod process_exec_contracts;
mod session_controller;
mod unified_exec_error;

pub use codex_approval_service_api::PermissionRequestPayload;
pub use codex_approval_service_api::ToolPermissionGrants;
pub use command_contracts::CommandNotifyOnArg;
pub use command_contracts::ExecCommandApprovalMode;
pub use command_contracts::ExecCommandArgs;
pub use command_contracts::ExecCommandRunOutput;
pub use command_contracts::ExecCommandRunRequest;
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
pub use process_exec_contracts::DEFAULT_EXEC_COMMAND_TIMEOUT_MS;
pub use process_exec_contracts::DEFAULT_EXEC_OUTPUT_MAX_BYTES;
pub use process_exec_contracts::ExecCapturePolicy;
pub use process_exec_contracts::ExecExpiration;
pub use process_exec_contracts::ExecExpirationOutcome;
pub use process_exec_contracts::ExecOptions;
pub use process_exec_contracts::ExecParams;
pub use process_exec_contracts::IO_DRAIN_TIMEOUT_MS;
pub use process_exec_contracts::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
pub use process_exec_contracts::bytes_to_string_smart;
pub use process_exec_contracts::cancel_when_either;
pub use process_exec_contracts::is_likely_sandbox_denied;
pub use session_controller::CommandSessionController;
pub use session_controller::CommandSessionError;
pub use session_controller::CommandSessionFuture;
pub use session_controller::CommandWaitOperation;
pub use permissions_service_api::ExecApprovalRequirement;
pub use thread_service_api::NetworkApprovalMode;
pub use thread_service_api::NetworkApprovalSpec;
pub use thread_service_api::ResolvedExecCommand;
pub use thread_service_api::RuntimeShell;
pub use thread_service_api::RuntimeShellSnapshot;
pub use thread_service_api::ToolRuntimeNetworkApprovalError;
pub use thread_service_api::ToolRuntimeNetworkApprovalHandle;
pub use thread_service_api::ToolRuntimeNetworkApprovalTrigger;
pub use thread_service_api::UnifiedExecApprovalKey;
pub use thread_service_api::resolve_exec_command_for_parts;
pub use unified_exec_error::UnifiedExecError;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_channel::Sender;
use codex_approval_service_api::ApprovalSessionCapability;
pub use codex_sandboxing_api::ApplyPatchEnvironment;
pub use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
pub use codex_sandboxing_api::ResolvedExecCommandEnvironment;
pub use codex_sandboxing_api::ToolSandboxContext;
use protocol::ThreadId;
use protocol::config_types::ShellEnvironmentPolicy;
use protocol::config_types::WindowsSandboxLevel;
use protocol::exec_output::ExecToolCallOutput;
use protocol::protocol::Event;
pub use thread_service_api::HookToolName;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;

/// Boxed future returned by object-safe command service APIs.
pub type CommandServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Session-owned command interaction capability consumed by command-wait tools.
pub trait SessionCommandInteractionCaller: Send + Sync + 'static {
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>;

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>>;
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
        approval_session: Arc<dyn ApprovalSessionCapability>,
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

#[derive(Clone)]
pub struct UserShellRunRequest {
    pub command: String,
    pub call_id: String,
    pub turn_id: String,
    pub thread_id: ThreadId,
    pub cwd: codex_utils_absolute_path::AbsolutePathBuf,
    pub session_shell: RuntimeShell,
    pub shell_environment_policy: ShellEnvironmentPolicy,
    pub shell_env_overrides: HashMap<String, String>,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub timeout_ms: u64,
    pub tx_event: Sender<Event>,
}

/// Command execution service API used by tool domains.
pub trait CommandServiceApi: Send + Sync + 'static {
    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn ThreadSessionCapability>,
        approval_session: Arc<dyn ApprovalSessionCapability>,
        state: Arc<dyn CommandServiceSessionState>,
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

    fn run_user_shell_command<'a>(
        &'a self,
        request: UserShellRunRequest,
    ) -> CommandServiceFuture<'a, protocol::error::Result<ExecToolCallOutput>>;
}
