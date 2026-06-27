mod exec_env;
mod exec_request;
mod runtime_support;
mod shell_support;
mod time_utils;
mod unified_exec;

use std::sync::Arc;

use codex_command_service_api::CommandServiceApi;
use codex_command_service_api::CommandServiceFuture;
use codex_command_service_api::CommandSessionController;
use codex_command_service_api::CommandSessionError;
use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_service_api::CommandServiceSessionState;
use codex_command_service_api::CommandServiceTurnCapability;
use codex_command_service_api::CommandWaitOperation;
use codex_command_service_api::CommandWaitRequest;
use codex_command_service_api::ExecCommandRunOutput;
use codex_command_service_api::ExecCommandRunRequest;
use codex_command_service_api::UnifiedExecError;
use codex_command_service_api::WriteStdinOutput;
use codex_command_service_api::WriteStdinRequest;
use unified_exec::UnifiedExecProcessManager;

pub use unified_exec::UnifiedExecManagerHandle;
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
        session: Arc<dyn CommandServiceSessionCapability>,
        turn: Arc<dyn CommandServiceTurnCapability>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>> {
        let state = session.command_service_state();
        Box::pin(async move { state.run_exec_command(session, turn, call_id, request).await })
    }

    fn begin_command_wait<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        request: CommandWaitRequest,
    ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>> {
        let state = session.command_service_state();
        Box::pin(async move { state.begin_command_wait(request).await })
    }

    fn write_command_stdin<'a>(
        &'a self,
        session: Arc<dyn CommandServiceSessionCapability>,
        request: WriteStdinRequest<'a>,
    ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        let state = session.command_service_state();
        Box::pin(async move { state.write_command_stdin(request).await })
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
        session: Arc<dyn CommandServiceSessionCapability>,
        turn: Arc<dyn CommandServiceTurnCapability>,
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
