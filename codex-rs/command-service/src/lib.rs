mod exec_env;
mod exec_request;
mod process_exec;
mod runtime_support;
mod shell_escalation;
mod shell_support;
mod spawn;
mod time_utils;
mod unified_exec;
mod user_shell;

use std::sync::Arc;

use codex_command_service_api::CommandServiceApi;
use codex_command_service_api::CommandServiceFuture;
use codex_command_service_api::CommandSessionController;
use codex_command_service_api::CommandSessionError;
use codex_command_service_api::CommandServiceSessionState;
use codex_command_service_api::CommandWaitOperation;
use codex_command_service_api::CommandWaitRequest;
use codex_command_service_api::ExecCommandRunOutput;
use codex_command_service_api::ExecCommandRunRequest;
use codex_command_service_api::UnifiedExecError;
use codex_command_service_api::UserShellRunRequest;
use codex_command_service_api::WriteStdinOutput;
use codex_command_service_api::WriteStdinRequest;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadRuntimeCapability;
use unified_exec::UnifiedExecProcessManager;

pub use unified_exec::UnifiedExecManagerHandle;
#[cfg(unix)]
pub use shell_escalation::run_shell_escalation_execve_wrapper;
pub use exec_env::create_env;
pub use exec_request::ExecRequest;
pub use process_exec::ExecCapturePolicy;
pub use process_exec::ExecExpiration;
pub use process_exec::ExecExpirationOutcome;
pub use process_exec::ExecParams;
pub use process_exec::StdoutStream;
pub use process_exec::build_exec_request;
pub use process_exec::execute_exec_request;
pub use process_exec::process_exec_tool_call;
pub use shell_support::maybe_wrap_shell_lc_with_snapshot;

pub struct CommandService;

impl CommandService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CommandService {
    fn default() -> Self {
        Self::new()
    }
}

pub fn set_deterministic_process_ids_for_tests(enabled: bool) {
    unified_exec::set_deterministic_process_ids_for_tests(enabled);
}

impl CommandServiceApi for CommandService {
    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn ThreadSessionCapability>,
        state: Arc<dyn CommandServiceSessionState>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>> {
        Box::pin(async move {
            state.run_exec_command(session, turn, call_id, request).await
        })
    }

    fn begin_command_wait<'a>(
        &'a self,
        state: Arc<dyn CommandServiceSessionState>,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>> {
        Box::pin(async move { state.begin_command_wait(request).await })
    }

    fn write_command_stdin<'a>(
        &'a self,
        state: Arc<dyn CommandServiceSessionState>,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        Box::pin(async move { state.write_command_stdin(request).await })
    }

    fn run_user_shell_command<'a>(
        &'a self,
        request: UserShellRunRequest,
    ) -> CommandServiceFuture<'a, codex_protocol::error::Result<codex_protocol::exec_output::ExecToolCallOutput>>
    {
        Box::pin(async move { user_shell::run_user_shell_command(request).await })
    }
}

pub struct CommandSessionState {
    unified_exec_manager: Arc<UnifiedExecProcessManager>,
    command_session_controller: Arc<dyn CommandSessionController>,
}

impl CommandSessionState {
    pub fn new(max_write_stdin_yield_time_ms: u64) -> Self {
        let unified_exec_manager = Arc::new(UnifiedExecProcessManager::new(
            max_write_stdin_yield_time_ms,
        ));
        let command_session_controller = Arc::new(
            unified_exec::UnifiedExecCommandSessionController::new(Arc::clone(
                &unified_exec_manager,
            )),
        );
        Self {
            unified_exec_manager,
            command_session_controller,
        }
    }

    pub fn manager_handle(&self) -> UnifiedExecManagerHandle {
        UnifiedExecManagerHandle::new(Arc::downgrade(&self.unified_exec_manager))
    }
}

impl CommandServiceSessionState for CommandSessionState {
    fn allocate_process_id<'a>(&'a self) -> CommandServiceFuture<'a, i32> {
        Box::pin(async move { self.unified_exec_manager.allocate_process_id().await })
    }

    fn release_process_id<'a>(&'a self, process_id: i32) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            self.unified_exec_manager.release_process_id(process_id).await;
        })
    }

    fn has_running_process_for_thread<'a>(
        &'a self,
        thread_id: codex_protocol::ThreadId,
    ) -> CommandServiceFuture<'a, bool> {
        Box::pin(async move {
            self.unified_exec_manager
                .has_running_process_for_thread(thread_id)
                .await
        })
    }

    fn terminate_all_processes<'a>(&'a self) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            self.unified_exec_manager.terminate_all_processes().await;
        })
    }

    fn run_exec_command<'a>(
        &'a self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>> {
        Box::pin(async move {
            let context = unified_exec::UnifiedExecContext::new(session, turn, call_id);
            self.unified_exec_manager
                .exec_command(unified_exec::ExecCommandRequest::from_run_request(
                    request,
                    context.turn.active_network(),
                ), &context)
                .await
        })
    }

    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>> {
        Box::pin(async move {
            self.command_session_controller
                .begin_command_wait(request)
                .await
        })
    }

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        Box::pin(async move {
            self.command_session_controller
                .write_command_stdin(request)
                .await
        })
    }
}
