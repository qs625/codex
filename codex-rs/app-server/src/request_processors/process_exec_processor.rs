use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::Duration;

use app_server_protocol::ClientResponsePayload;
use app_server_protocol::JSONRPCErrorError;
use app_server_protocol::ProcessExitedNotification;
use app_server_protocol::ProcessKillParams;
use app_server_protocol::ProcessKillResponse;
use app_server_protocol::ProcessOutputDeltaNotification;
use app_server_protocol::ProcessOutputStream;
use app_server_protocol::ProcessResizePtyParams;
use app_server_protocol::ProcessResizePtyResponse;
use app_server_protocol::ProcessSpawnParams;
use app_server_protocol::ProcessSpawnResponse;
use app_server_protocol::ProcessTerminalSize;
use app_server_protocol::ProcessWriteStdinParams;
use app_server_protocol::ProcessWriteStdinResponse;
use app_server_protocol::ServerNotification;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
use codex_utils_pty::ProcessHandle;
use codex_utils_pty::SpawnedProcess;
use codex_utils_pty::TerminalSize;
use command_service_api::ExecExpiration;
use command_service_api::ExecExpirationOutcome;
use command_service_api::IO_DRAIN_TIMEOUT_MS;
use command_service_api::bytes_to_string_smart;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::error_code::invalid_request;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingMessageSender;

const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;
const OUTPUT_CHUNK_SIZE_HINT: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct ProcessExecRequestProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    process_exec_manager: ProcessExecManager,
}

impl ProcessExecRequestProcessor {
    pub(crate) fn new(outgoing: Arc<OutgoingMessageSender>) -> Self {
        Self {
            outgoing,
            process_exec_manager: ProcessExecManager::default(),
        }
    }

    pub(crate) async fn process_spawn(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessSpawnParams,
    ) -> Result<(), JSONRPCErrorError> {
        let method_name = "process/spawn";
        let request = ProcessSpawnRequest::from_protocol(method_name, params)?;
        tracing::debug!("{method_name} command: {:?}", request.command);

        self.process_exec_manager
            .start(StartProcessParams {
                outgoing: self.outgoing.clone(),
                request_id,
                request,
            })
            .await?;

        Ok(())
    }

    pub(crate) async fn process_write_stdin(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessWriteStdinParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.process_exec_manager
            .write_stdin(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn process_resize_pty(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessResizePtyParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.process_exec_manager
            .resize_pty(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn process_kill(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessKillParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.process_exec_manager
            .kill(request_id, params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn connection_closed(&self, connection_id: ConnectionId) {
        self.process_exec_manager
            .connection_closed(connection_id)
            .await;
    }
}

#[derive(Clone, Default)]
struct ProcessExecManager {
    sessions: Arc<Mutex<HashMap<ConnectionProcessHandle, ProcessSession>>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ConnectionProcessHandle {
    connection_id: ConnectionId,
    process_handle: String,
}

#[derive(Clone)]
struct ProcessSession {
    control_tx: mpsc::Sender<ProcessControlRequest>,
}

enum ProcessControl {
    Write { delta: Vec<u8>, close_stdin: bool },
    Resize { size: TerminalSize },
    Kill,
}

struct ProcessControlRequest {
    control: ProcessControl,
    response_tx: Option<oneshot::Sender<Result<(), JSONRPCErrorError>>>,
}

struct StartProcessParams {
    outgoing: Arc<OutgoingMessageSender>,
    request_id: ConnectionRequestId,
    request: ProcessSpawnRequest,
}

struct ProcessSpawnRequest {
    process_handle: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    env: HashMap<String, String>,
    expiration: ExecExpiration,
    tty: bool,
    stream_stdin: bool,
    stream_stdout_stderr: bool,
    output_bytes_cap: Option<usize>,
    size: Option<TerminalSize>,
}

impl ProcessSpawnRequest {
    fn from_protocol(
        method_name: &str,
        params: ProcessSpawnParams,
    ) -> Result<Self, JSONRPCErrorError> {
        let ProcessSpawnParams {
            command,
            process_handle,
            cwd,
            tty,
            stream_stdin,
            stream_stdout_stderr,
            output_bytes_cap,
            timeout_ms,
            env: env_overrides,
            size,
        } = params;

        if command.is_empty() {
            return Err(invalid_request("command must not be empty"));
        }
        if process_handle.is_empty() {
            return Err(invalid_request("processHandle must not be empty"));
        }
        if size.is_some() && !tty {
            return Err(invalid_params("process/spawn size requires tty: true"));
        }

        let mut env = std::env::vars().collect::<HashMap<_, _>>();
        apply_env_overrides(&mut env, env_overrides);

        let expiration = match timeout_ms {
            Some(Some(timeout_ms)) => match u64::try_from(timeout_ms) {
                Ok(timeout_ms) => timeout_ms.into(),
                Err(_) => {
                    return Err(invalid_params(format!(
                        "{method_name} timeoutMs must be non-negative, got {timeout_ms}"
                    )));
                }
            },
            Some(None) => ExecExpiration::Cancellation(CancellationToken::new()),
            None => ExecExpiration::DefaultTimeout,
        };

        Ok(Self {
            process_handle,
            command,
            cwd,
            env,
            expiration,
            tty,
            stream_stdin,
            stream_stdout_stderr,
            output_bytes_cap: output_bytes_cap.unwrap_or(Some(DEFAULT_OUTPUT_BYTES_CAP)),
            size: size.map(terminal_size_from_protocol).transpose()?,
        })
    }
}

fn apply_env_overrides(
    env: &mut HashMap<String, String>,
    env_overrides: Option<HashMap<String, Option<String>>>,
) {
    if let Some(env_overrides) = env_overrides {
        for (key, value) in env_overrides {
            match value {
                Some(value) => {
                    env.insert(key, value);
                }
                None => {
                    env.remove(&key);
                }
            }
        }
    }
}

struct RunProcessParams {
    outgoing: Arc<OutgoingMessageSender>,
    request_id: ConnectionRequestId,
    process_handle: String,
    spawned: SpawnedProcess,
    control_rx: mpsc::Receiver<ProcessControlRequest>,
    stream_stdin: bool,
    stream_stdout_stderr: bool,
    expiration: ExecExpiration,
    output_bytes_cap: Option<usize>,
}

struct SpawnProcessOutputParams {
    connection_id: ConnectionId,
    process_handle: String,
    output_rx: mpsc::Receiver<Vec<u8>>,
    stdio_timeout_rx: watch::Receiver<bool>,
    outgoing: Arc<OutgoingMessageSender>,
    stream: ProcessOutputStream,
    stream_output: bool,
    output_bytes_cap: Option<usize>,
}

#[derive(Default)]
struct ProcessOutputCapture {
    text: String,
    cap_reached: bool,
}

struct ProcessOutputAccumulator {
    buffer: Vec<u8>,
    observed_num_bytes: usize,
    cap_reached: bool,
    stream_output: bool,
    output_bytes_cap: Option<usize>,
}

struct ProcessOutputChunk<'a> {
    bytes: &'a [u8],
    cap_reached: bool,
}

impl ProcessOutputAccumulator {
    fn new(stream_output: bool, output_bytes_cap: Option<usize>) -> Self {
        Self {
            buffer: Vec::new(),
            observed_num_bytes: 0,
            cap_reached: false,
            stream_output,
            output_bytes_cap,
        }
    }

    fn accept_chunk<'a>(&mut self, chunk: &'a [u8]) -> ProcessOutputChunk<'a> {
        let capped_chunk = match self.output_bytes_cap {
            Some(output_bytes_cap) => {
                let capped_chunk_len = output_bytes_cap
                    .saturating_sub(self.observed_num_bytes)
                    .min(chunk.len());
                self.observed_num_bytes += capped_chunk_len;
                &chunk[0..capped_chunk_len]
            }
            None => chunk,
        };
        self.cap_reached = Some(self.observed_num_bytes) == self.output_bytes_cap;
        if !self.stream_output {
            self.buffer.extend_from_slice(capped_chunk);
        }
        ProcessOutputChunk {
            bytes: capped_chunk,
            cap_reached: self.cap_reached,
        }
    }

    fn finish(self) -> ProcessOutputCapture {
        ProcessOutputCapture {
            text: bytes_to_string_smart(&self.buffer),
            cap_reached: self.cap_reached,
        }
    }
}

impl ProcessExecManager {
    async fn start(&self, params: StartProcessParams) -> Result<(), JSONRPCErrorError> {
        let StartProcessParams {
            outgoing,
            request_id,
            request,
        } = params;
        let ProcessSpawnRequest {
            process_handle,
            command,
            cwd,
            env,
            expiration,
            tty,
            stream_stdin,
            stream_stdout_stderr,
            output_bytes_cap,
            size,
        } = request;

        let (program, args) = command
            .split_first()
            .ok_or_else(|| invalid_request("command must not be empty"))?;
        let stream_stdin = tty || stream_stdin;
        let stream_stdout_stderr = tty || stream_stdout_stderr;
        let arg0 = None;
        let (control_tx, control_rx) = mpsc::channel(32);
        let process_key = ConnectionProcessHandle {
            connection_id: request_id.connection_id,
            process_handle: process_handle.clone(),
        };

        {
            let mut sessions = self.sessions.lock().await;
            match sessions.entry(process_key.clone()) {
                Entry::Occupied(_) => {
                    return Err(invalid_request(format!(
                        "duplicate active process handle: {process_handle:?}",
                    )));
                }
                Entry::Vacant(entry) => {
                    entry.insert(ProcessSession { control_tx });
                }
            }
        }

        let spawned = if tty {
            codex_utils_pty::spawn_pty_process(
                program,
                args,
                cwd.as_path(),
                &env,
                &arg0,
                size.unwrap_or_default(),
            )
            .await
        } else if stream_stdin {
            codex_utils_pty::spawn_pipe_process(program, args, cwd.as_path(), &env, &arg0).await
        } else {
            codex_utils_pty::spawn_pipe_process_no_stdin(program, args, cwd.as_path(), &env, &arg0)
                .await
        };
        let spawned = match spawned {
            Ok(spawned) => spawned,
            Err(err) => {
                self.sessions.lock().await.remove(&process_key);
                return Err(internal_error(format!("failed to spawn process: {err}")));
            }
        };

        outgoing
            .send_response(request_id.clone(), ProcessSpawnResponse {})
            .await;

        let sessions = Arc::clone(&self.sessions);
        tokio::spawn(async move {
            run_process(RunProcessParams {
                outgoing,
                request_id,
                process_handle,
                spawned,
                control_rx,
                stream_stdin,
                stream_stdout_stderr,
                expiration,
                output_bytes_cap,
            })
            .await;
            sessions.lock().await.remove(&process_key);
        });

        Ok(())
    }

    async fn write_stdin(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessWriteStdinParams,
    ) -> Result<ProcessWriteStdinResponse, JSONRPCErrorError> {
        if params.delta_base64.is_none() && !params.close_stdin {
            return Err(invalid_params(
                "process/writeStdin requires deltaBase64 or closeStdin",
            ));
        }

        let delta = match params.delta_base64 {
            Some(delta_base64) => STANDARD
                .decode(delta_base64)
                .map_err(|err| invalid_params(format!("invalid deltaBase64: {err}")))?,
            None => Vec::new(),
        };

        self.send_control(
            request_id.connection_id,
            params.process_handle,
            ProcessControl::Write {
                delta,
                close_stdin: params.close_stdin,
            },
        )
        .await?;

        Ok(ProcessWriteStdinResponse {})
    }

    async fn kill(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessKillParams,
    ) -> Result<ProcessKillResponse, JSONRPCErrorError> {
        self.send_control(
            request_id.connection_id,
            params.process_handle,
            ProcessControl::Kill,
        )
        .await?;
        Ok(ProcessKillResponse {})
    }

    async fn resize_pty(
        &self,
        request_id: ConnectionRequestId,
        params: ProcessResizePtyParams,
    ) -> Result<ProcessResizePtyResponse, JSONRPCErrorError> {
        self.send_control(
            request_id.connection_id,
            params.process_handle,
            ProcessControl::Resize {
                size: terminal_size_from_protocol(params.size)?,
            },
        )
        .await?;
        Ok(ProcessResizePtyResponse {})
    }

    async fn connection_closed(&self, connection_id: ConnectionId) {
        let controls = {
            let mut sessions = self.sessions.lock().await;
            let process_handles = sessions
                .keys()
                .filter(|process_handle| process_handle.connection_id == connection_id)
                .cloned()
                .collect::<Vec<_>>();
            let mut controls = Vec::with_capacity(process_handles.len());
            for process_handle in process_handles {
                if let Some(control) = sessions.remove(&process_handle) {
                    controls.push(control);
                }
            }
            controls
        };

        for control in controls {
            let _ = control
                .control_tx
                .send(ProcessControlRequest {
                    control: ProcessControl::Kill,
                    response_tx: None,
                })
                .await;
        }
    }

    async fn send_control(
        &self,
        connection_id: ConnectionId,
        process_handle: String,
        control: ProcessControl,
    ) -> Result<(), JSONRPCErrorError> {
        let process_key = ConnectionProcessHandle {
            connection_id,
            process_handle,
        };
        let session = self
            .sessions
            .lock()
            .await
            .get(&process_key)
            .cloned()
            .ok_or_else(|| no_active_process_error(&process_key.process_handle))?;
        let (response_tx, response_rx) = oneshot::channel();
        session
            .control_tx
            .send(ProcessControlRequest {
                control,
                response_tx: Some(response_tx),
            })
            .await
            .map_err(|_| process_no_longer_running_error(&process_key.process_handle))?;
        response_rx
            .await
            .map_err(|_| process_no_longer_running_error(&process_key.process_handle))?
    }
}

async fn run_process(params: RunProcessParams) {
    let RunProcessParams {
        outgoing,
        request_id,
        process_handle,
        spawned,
        control_rx,
        stream_stdin,
        stream_stdout_stderr,
        expiration,
        output_bytes_cap,
    } = params;
    let mut control_rx = control_rx;
    let mut control_open = true;
    let expiration = expiration.wait_with_outcome();
    tokio::pin!(expiration);
    let SpawnedProcess {
        session,
        stdout_rx,
        stderr_rx,
        exit_rx,
    } = spawned;
    tokio::pin!(exit_rx);
    let mut expiration_outcome = None;
    let (stdio_timeout_tx, stdio_timeout_rx) = watch::channel(false);

    let stdout_handle = collect_spawn_process_output(SpawnProcessOutputParams {
        connection_id: request_id.connection_id,
        process_handle: process_handle.clone(),
        output_rx: stdout_rx,
        stdio_timeout_rx: stdio_timeout_rx.clone(),
        outgoing: Arc::clone(&outgoing),
        stream: ProcessOutputStream::Stdout,
        stream_output: stream_stdout_stderr,
        output_bytes_cap,
    });
    let stderr_handle = collect_spawn_process_output(SpawnProcessOutputParams {
        connection_id: request_id.connection_id,
        process_handle: process_handle.clone(),
        output_rx: stderr_rx,
        stdio_timeout_rx,
        outgoing: Arc::clone(&outgoing),
        stream: ProcessOutputStream::Stderr,
        stream_output: stream_stdout_stderr,
        output_bytes_cap,
    });

    let exit_code = loop {
        tokio::select! {
            control = control_rx.recv(), if control_open => {
                match control {
                    Some(ProcessControlRequest { control, response_tx }) => {
                        let result = match control {
                            ProcessControl::Write { delta, close_stdin } => {
                                handle_process_write(
                                    &session,
                                    stream_stdin,
                                    delta,
                                    close_stdin,
                                ).await
                            }
                            ProcessControl::Resize { size } => {
                                handle_process_resize(&session, size)
                            }
                            ProcessControl::Kill => {
                                session.request_terminate();
                                Ok(())
                            }
                        };
                        if let Some(response_tx) = response_tx
                            && response_tx.send(result).is_err()
                        {
                            tracing::debug!(
                                process_handle = %process_handle,
                                "process control response receiver dropped"
                            );
                        }
                    },
                    None => {
                        control_open = false;
                        session.request_terminate();
                    }
                }
            }
            outcome = &mut expiration, if expiration_outcome.is_none() => {
                expiration_outcome = Some(outcome);
                session.request_terminate();
            }
            exit = &mut exit_rx => {
                if matches!(expiration_outcome, Some(ExecExpirationOutcome::TimedOut)) {
                    break EXEC_TIMEOUT_EXIT_CODE;
                } else {
                    break exit.unwrap_or(-1);
                }
            }
        }
    };

    // Give stdout/stderr readers a bounded grace period to drain after process exit.
    let timeout_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(IO_DRAIN_TIMEOUT_MS)).await;
        let _ = stdio_timeout_tx.send(true);
    });

    let stdout = stdout_handle.await.unwrap_or_default();
    let stderr = stderr_handle.await.unwrap_or_default();
    timeout_handle.abort();

    outgoing
        .send_server_notification_to_connection_and_wait(
            request_id.connection_id,
            ServerNotification::ProcessExited(ProcessExitedNotification {
                process_handle,
                exit_code,
                stdout: stdout.text,
                stdout_cap_reached: stdout.cap_reached,
                stderr: stderr.text,
                stderr_cap_reached: stderr.cap_reached,
            }),
        )
        .await;
}

fn collect_spawn_process_output(
    params: SpawnProcessOutputParams,
) -> tokio::task::JoinHandle<ProcessOutputCapture> {
    let SpawnProcessOutputParams {
        connection_id,
        process_handle,
        mut output_rx,
        mut stdio_timeout_rx,
        outgoing,
        stream,
        stream_output,
        output_bytes_cap,
    } = params;
    tokio::spawn(async move {
        let mut output = ProcessOutputAccumulator::new(stream_output, output_bytes_cap);
        loop {
            let mut chunk = tokio::select! {
                chunk = output_rx.recv() => match chunk {
                    Some(chunk) => chunk,
                    None => break,
                },
                _ = stdio_timeout_rx.wait_for(|&v| v) => break,
            };
            while chunk.len() < OUTPUT_CHUNK_SIZE_HINT
                && let Ok(next_chunk) = output_rx.try_recv()
            {
                chunk.extend_from_slice(&next_chunk);
            }
            let capped_chunk = output.accept_chunk(&chunk);
            if stream_output {
                outgoing
                    .send_server_notification_to_connection_and_wait(
                        connection_id,
                        ServerNotification::ProcessOutputDelta(ProcessOutputDeltaNotification {
                            process_handle: process_handle.clone(),
                            stream,
                            delta_base64: STANDARD.encode(capped_chunk.bytes),
                            cap_reached: capped_chunk.cap_reached,
                        }),
                    )
                    .await;
            }
            if capped_chunk.cap_reached {
                break;
            }
        }
        output.finish()
    })
}

async fn handle_process_write(
    session: &ProcessHandle,
    stream_stdin: bool,
    delta: Vec<u8>,
    close_stdin: bool,
) -> Result<(), JSONRPCErrorError> {
    if !stream_stdin {
        return Err(invalid_request(
            "stdin streaming is not enabled for this process",
        ));
    }
    if !delta.is_empty() {
        session
            .writer_sender()
            .send(delta)
            .await
            .map_err(|_| invalid_request("stdin is already closed"))?;
    }
    if close_stdin {
        // Closing drops our sender; the writer task still drains any bytes
        // accepted above before its receiver observes EOF and closes stdin.
        session.close_stdin();
    }
    Ok(())
}

fn handle_process_resize(
    session: &ProcessHandle,
    size: TerminalSize,
) -> Result<(), JSONRPCErrorError> {
    session
        .resize(size)
        .map_err(|err| invalid_request(format!("failed to resize PTY: {err}")))
}

fn terminal_size_from_protocol(
    size: ProcessTerminalSize,
) -> Result<TerminalSize, JSONRPCErrorError> {
    if size.rows == 0 || size.cols == 0 {
        return Err(invalid_params(
            "process size rows and cols must be greater than 0",
        ));
    }
    Ok(TerminalSize {
        rows: size.rows,
        cols: size.cols,
    })
}

fn no_active_process_error(process_handle: &str) -> JSONRPCErrorError {
    invalid_request(format!(
        "no active process for process handle {process_handle:?}"
    ))
}

fn process_no_longer_running_error(process_handle: &str) -> JSONRPCErrorError {
    invalid_request(format!("process {process_handle:?} is no longer running"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn spawn_params() -> ProcessSpawnParams {
        ProcessSpawnParams {
            command: vec!["echo".to_string()],
            process_handle: "process-1".to_string(),
            cwd: AbsolutePathBuf::try_from(std::env::current_dir().unwrap()).unwrap(),
            tty: false,
            stream_stdin: false,
            stream_stdout_stderr: false,
            output_bytes_cap: None,
            timeout_ms: None,
            env: None,
            size: None,
        }
    }

    #[test]
    fn process_spawn_request_applies_defaults_and_env_overrides() {
        let mut params = spawn_params();
        let existing_key = std::env::vars()
            .next()
            .map(|(key, _)| key)
            .unwrap_or_else(|| "PATH".to_string());
        params.env = Some(HashMap::from([
            (
                "PROCESS_EXEC_TEST_ENV".to_string(),
                Some("value".to_string()),
            ),
            (existing_key.clone(), None),
        ]));

        let request = ProcessSpawnRequest::from_protocol("process/spawn", params).unwrap();

        assert_eq!(request.output_bytes_cap, Some(DEFAULT_OUTPUT_BYTES_CAP));
        assert_eq!(
            request.env.get("PROCESS_EXEC_TEST_ENV"),
            Some(&"value".to_string())
        );
        assert!(!request.env.contains_key(&existing_key));
    }

    #[test]
    fn process_spawn_request_preserves_explicit_unbounded_output_and_timeout_cancellation() {
        let mut params = spawn_params();
        params.output_bytes_cap = Some(None);
        params.timeout_ms = Some(None);

        let request = ProcessSpawnRequest::from_protocol("process/spawn", params).unwrap();

        assert_eq!(request.output_bytes_cap, None);
        assert!(matches!(
            request.expiration,
            ExecExpiration::Cancellation(_)
        ));
    }

    #[test]
    fn process_spawn_request_rejects_size_without_tty_before_size_validation() {
        let mut params = spawn_params();
        params.size = Some(ProcessTerminalSize { rows: 0, cols: 0 });

        let err = match ProcessSpawnRequest::from_protocol("process/spawn", params) {
            Ok(_) => panic!("size without tty should fail"),
            Err(err) => err,
        };

        assert_eq!(err.message, "process/spawn size requires tty: true");
    }

    #[test]
    fn process_output_accumulator_buffers_capped_output() {
        let mut output = ProcessOutputAccumulator::new(false, Some(3));

        let first = output.accept_chunk(b"ab");
        assert_eq!(first.bytes, b"ab");
        assert!(!first.cap_reached);
        let second = output.accept_chunk(b"cde");
        assert_eq!(second.bytes, b"c");
        assert!(second.cap_reached);

        let captured = output.finish();
        assert_eq!(captured.text, "abc");
        assert!(captured.cap_reached);
    }

    #[test]
    fn process_output_accumulator_streams_without_buffering() {
        let mut output = ProcessOutputAccumulator::new(true, None);

        let chunk = output.accept_chunk(b"streamed");

        assert_eq!(chunk.bytes, b"streamed");
        assert!(!chunk.cap_reached);
        let captured = output.finish();
        assert_eq!(captured.text, "");
        assert!(!captured.cap_reached);
    }
}
