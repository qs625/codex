#![allow(clippy::module_inception)]

use std::sync::Arc;
use tokio::sync::Notify;
use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use codex_exec_server_api::ExecProcess;
use codex_exec_server_api::StartedExecProcess;
use codex_exec_server_protocol::ReadResponse as ExecReadResponse;
use codex_exec_server_protocol::WriteStatus;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::protocol::TruncationPolicy;
use codex_sandboxing_api::SandboxType;
use codex_utils_output_truncation::formatted_truncate_text;
use codex_utils_pty::ExecCommandSession;
use codex_utils_pty::SpawnedPty;
use codex_process_exec::is_likely_sandbox_denied;

use super::DEFAULT_COMMAND_OUTPUT_MAX_TOKENS;
use super::ProcessState;
use super::UnifiedExecError;
use super::output::CommandOutputHandles as OutputHandles;
use super::output::CommandOutputRuntime;

const EARLY_EXIT_GRACE_PERIOD: Duration = Duration::from_millis(150);

pub trait SpawnLifecycle: std::fmt::Debug + Send + Sync {
    /// Returns file descriptors that must stay open across the child `exec()`.
    ///
    /// The returned descriptors must already be valid in the parent process and
    /// stay valid until `after_spawn()` runs, which is the first point where
    /// the parent may release its copies.
    fn inherited_fds(&self) -> Vec<i32> {
        Vec::new()
    }

    fn after_spawn(&mut self) {}
}

pub type SpawnLifecycleHandle = Box<dyn SpawnLifecycle>;

#[derive(Debug, Default)]
/// Spawn lifecycle that performs no extra setup around process launch.
pub struct NoopSpawnLifecycle;

impl SpawnLifecycle for NoopSpawnLifecycle {}

/// Transport-specific process handle used by unified exec.
enum ProcessHandle {
    Local(Box<ExecCommandSession>),
    ExecServer(Arc<dyn ExecProcess>),
}

/// Unified wrapper over directly spawned PTY sessions and exec-server-backed
/// processes.
pub struct UnifiedExecProcess {
    process_handle: ProcessHandle,
    output_runtime: CommandOutputRuntime,
    state_tx: watch::Sender<ProcessState>,
    state_rx: watch::Receiver<ProcessState>,
    output_task: Option<JoinHandle<()>>,
    sandbox_type: SandboxType,
    _spawn_lifecycle: Option<SpawnLifecycleHandle>,
}

impl std::fmt::Debug for UnifiedExecProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiedExecProcess")
            .field("has_exited", &self.has_exited())
            .field("exit_code", &self.exit_code())
            .field("sandbox_type", &self.sandbox_type)
            .finish_non_exhaustive()
    }
}

impl UnifiedExecProcess {
    fn new(
        process_handle: ProcessHandle,
        sandbox_type: SandboxType,
        spawn_lifecycle: Option<SpawnLifecycleHandle>,
    ) -> Self {
        let output_runtime = CommandOutputRuntime::new();
        let (state_tx, state_rx) = watch::channel(ProcessState::default());

        Self {
            process_handle,
            output_runtime,
            state_tx,
            state_rx,
            output_task: None,
            sandbox_type,
            _spawn_lifecycle: spawn_lifecycle,
        }
    }

    pub async fn write(&self, data: &[u8]) -> Result<(), UnifiedExecError> {
        match &self.process_handle {
            ProcessHandle::Local(process_handle) => process_handle
                .writer_sender()
                .send(data.to_vec())
                .await
                .map_err(|_| UnifiedExecError::WriteToStdin),
            ProcessHandle::ExecServer(process_handle) => {
                match process_handle.write(data.to_vec()).await {
                    Ok(response) => match response.status {
                        WriteStatus::Accepted => Ok(()),
                        WriteStatus::UnknownProcess | WriteStatus::StdinClosed => {
                            let state = self.state_rx.borrow().clone();
                            let _ = self.state_tx.send_replace(state.exited(state.exit_code));
                            self.output_runtime.cancel();
                            Err(UnifiedExecError::WriteToStdin)
                        }
                        WriteStatus::Starting => Err(UnifiedExecError::WriteToStdin),
                    },
                    Err(err) => Err(UnifiedExecError::process_failed(err.to_string())),
                }
            }
        }
    }

    pub fn output_handles(&self) -> OutputHandles {
        self.output_runtime.handles()
    }

    pub fn output_receiver(&self) -> tokio::sync::broadcast::Receiver<Vec<u8>> {
        self.output_runtime.receiver()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.output_runtime.cancellation_token()
    }

    pub fn output_drained_notify(&self) -> Arc<Notify> {
        self.output_runtime.output_drained_notify()
    }

    pub fn has_exited(&self) -> bool {
        let state = self.state_rx.borrow().clone();
        match &self.process_handle {
            ProcessHandle::Local(process_handle) => state.has_exited || process_handle.has_exited(),
            ProcessHandle::ExecServer(_) => state.has_exited,
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        let state = self.state_rx.borrow().clone();
        match &self.process_handle {
            ProcessHandle::Local(process_handle) => {
                state.exit_code.or_else(|| process_handle.exit_code())
            }
            ProcessHandle::ExecServer(_) => state.exit_code,
        }
    }

    pub fn terminate(&self) {
        self.output_runtime.close_output();
        match &self.process_handle {
            ProcessHandle::Local(process_handle) => process_handle.terminate(),
            ProcessHandle::ExecServer(process_handle) => {
                let process_handle = Arc::clone(process_handle);
                tokio::spawn(async move {
                    let _ = process_handle.terminate().await;
                });
            }
        }
        self.output_runtime.cancel();
        if let Some(output_task) = &self.output_task {
            output_task.abort();
        }
    }

    pub fn fail_and_terminate(&self, message: String) {
        let state = self.state_rx.borrow().clone();
        if state.failure_message.is_none() {
            let _ = self.state_tx.send_replace(state.failed(message));
        }
        self.terminate();
    }

    async fn snapshot_output(&self) -> Vec<Vec<u8>> {
        self.output_runtime.snapshot_chunks().await
    }

    pub fn sandbox_type(&self) -> SandboxType {
        self.sandbox_type
    }

    pub fn failure_message(&self) -> Option<String> {
        self.state_rx.borrow().failure_message.clone()
    }

    async fn check_for_sandbox_denial(&self) -> Result<(), UnifiedExecError> {
        let output_handles = self.output_handles();
        let _ = tokio::time::timeout(
            Duration::from_millis(20),
            output_handles.output_notify.notified(),
        )
        .await;

        let collected_chunks = self.snapshot_output().await;
        let mut aggregated: Vec<u8> = Vec::new();
        for chunk in collected_chunks {
            aggregated.extend_from_slice(&chunk);
        }
        let aggregated_text = String::from_utf8_lossy(&aggregated).to_string();
        self.check_for_sandbox_denial_with_text(&aggregated_text)
            .await?;

        Ok(())
    }

    pub async fn check_for_sandbox_denial_with_text(
        &self,
        text: &str,
    ) -> Result<(), UnifiedExecError> {
        let sandbox_type = self.sandbox_type();
        if sandbox_type == SandboxType::None || !self.has_exited() {
            return Ok(());
        }

        let exit_code = self.exit_code().unwrap_or(-1);
        let exec_output = ExecToolCallOutput {
            exit_code,
            stderr: StreamOutput::new(text.to_string()),
            aggregated_output: StreamOutput::new(text.to_string()),
            ..Default::default()
        };
        if is_likely_sandbox_denied(sandbox_type, &exec_output) {
            let snippet = formatted_truncate_text(
                text,
                TruncationPolicy::Tokens(DEFAULT_COMMAND_OUTPUT_MAX_TOKENS),
            );
            let message = if snippet.is_empty() {
                format!("Process exited with code {exit_code}")
            } else {
                snippet
            };
            return Err(UnifiedExecError::sandbox_denied(message, exec_output));
        }
        Ok(())
    }

    pub async fn from_spawned(
        spawned: SpawnedPty,
        sandbox_type: SandboxType,
        spawn_lifecycle: SpawnLifecycleHandle,
    ) -> Result<Self, UnifiedExecError> {
        let SpawnedPty {
            session: process_handle,
            stdout_rx,
            stderr_rx,
            mut exit_rx,
        } = spawned;
        let output_rx = codex_utils_pty::combine_output_receivers(stdout_rx, stderr_rx);
        let mut managed = Self::new(
            ProcessHandle::Local(Box::new(process_handle)),
            sandbox_type,
            Some(spawn_lifecycle),
        );
        managed.output_task = Some(tokio::spawn(
            managed
                .output_runtime
                .clone()
                .pump_broadcast_receiver(output_rx),
        ));

        match exit_rx.try_recv() {
            Ok(exit_code) => {
                managed.signal_exit(Some(exit_code));
                managed.check_for_sandbox_denial().await?;
                return Ok(managed);
            }
            Err(TryRecvError::Closed) => {
                managed.signal_exit(/*exit_code*/ None);
                managed.check_for_sandbox_denial().await?;
                return Ok(managed);
            }
            Err(TryRecvError::Empty) => {}
        }

        if let Ok(exit_result) = tokio::time::timeout(EARLY_EXIT_GRACE_PERIOD, &mut exit_rx).await {
            managed.signal_exit(exit_result.ok());
            managed.check_for_sandbox_denial().await?;
            return Ok(managed);
        }

        tokio::spawn({
            let state_tx = managed.state_tx.clone();
            let output_runtime = managed.output_runtime.clone();
            async move {
                let exit_code = exit_rx.await.ok();
                let state = state_tx.borrow().clone();
                let _ = state_tx.send_replace(state.exited(exit_code));
                output_runtime.cancel();
            }
        });

        Ok(managed)
    }

    pub async fn from_exec_server_started(
        started: StartedExecProcess,
        sandbox_type: SandboxType,
    ) -> Result<Self, UnifiedExecError> {
        let process_handle = ProcessHandle::ExecServer(Arc::clone(&started.process));
        let mut managed = Self::new(process_handle, sandbox_type, /*spawn_lifecycle*/ None);
        let output_runtime = managed.output_runtime.clone();
        managed.output_task = Some(Self::spawn_exec_server_output_task(
            started,
            output_runtime,
            managed.state_tx.clone(),
        ));

        let mut state_rx = managed.state_rx.clone();
        if tokio::time::timeout(EARLY_EXIT_GRACE_PERIOD, async {
            loop {
                let state = state_rx.borrow().clone();
                if state.has_exited || state.failure_message.is_some() {
                    break;
                }
                if state_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .is_ok()
        {
            managed.check_for_sandbox_denial().await?;
        }

        Ok(managed)
    }

    fn spawn_exec_server_output_task(
        started: StartedExecProcess,
        output_runtime: CommandOutputRuntime,
        state_tx: watch::Sender<ProcessState>,
    ) -> JoinHandle<()> {
        let process = started.process;
        let mut wake_rx = process.subscribe_wake();
        tokio::spawn(async move {
            let mut after_seq = None;
            loop {
                match process
                    .read(after_seq, /*max_bytes*/ None, /*wait_ms*/ Some(0))
                    .await
                {
                    Ok(response) => {
                        let ExecReadResponse {
                            chunks,
                            next_seq,
                            exited,
                            exit_code,
                            closed,
                            failure,
                        } = response;

                        for chunk in chunks {
                            let bytes = chunk.chunk.into_inner();
                            output_runtime.push_chunk(bytes).await;
                        }

                        if let Some(message) = failure {
                            let state = state_tx.borrow().clone();
                            let _ = state_tx.send_replace(state.failed(message));
                            output_runtime.close_output();
                            output_runtime.cancel();
                            break;
                        }

                        if exited {
                            let state = state_tx.borrow().clone();
                            let _ = state_tx.send_replace(state.exited(exit_code));
                        }

                        if closed {
                            output_runtime.close_output();
                            output_runtime.cancel();
                        }

                        after_seq = next_seq.checked_sub(1);
                        if closed {
                            break;
                        }
                    }
                    Err(err) => {
                        let state = state_tx.borrow().clone();
                        let _ = state_tx.send_replace(state.failed(err.to_string()));
                        output_runtime.close_output();
                        output_runtime.cancel();
                        break;
                    }
                }

                if wake_rx.changed().await.is_err() {
                    let state = state_tx.borrow().clone();
                    let _ = state_tx
                        .send_replace(state.failed("exec-server wake channel closed".to_string()));
                    output_runtime.close_output();
                    output_runtime.cancel();
                    break;
                }
            }
        })
    }

    fn signal_exit(&self, exit_code: Option<i32>) {
        let state = self.state_rx.borrow().clone();
        let _ = self.state_tx.send_replace(state.exited(exit_code));
        self.output_runtime.cancel();
    }
}

impl Drop for UnifiedExecProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}
