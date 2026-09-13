use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use app_server_protocol::CommandExecOutputDeltaNotification;
use app_server_protocol::CommandExecOutputStream;
use app_server_protocol::CommandExecExitedNotification;
use app_server_protocol::CommandExecStartedNotification;
use app_server_protocol::CommandExecResizeParams;
use app_server_protocol::CommandExecResizeResponse;
use app_server_protocol::CommandExecResponse;
use app_server_protocol::CommandExecTerminalSize;
use app_server_protocol::CommandExecTerminateParams;
use app_server_protocol::CommandExecTerminateResponse;
use app_server_protocol::CommandExecWriteParams;
use app_server_protocol::CommandExecWriteResponse;
use app_server_protocol::JSONRPCErrorError;
use app_server_protocol::ServerNotification;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use codex_sandboxing_api::SandboxType;
use codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
use codex_utils_pty::ProcessHandle;
use codex_utils_pty::SpawnedProcess;
use codex_utils_pty::TerminalSize;
use command_service::ExecRequest;
use command_service::execute_exec_request;
use command_service_api::ExecExpiration;
use command_service_api::ExecExpirationOutcome;
use command_service_api::IO_DRAIN_TIMEOUT_MS;
use command_service_api::bytes_to_string_smart;
use thread_service::config::StartedNetworkProxy;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::watch;
use uuid::Uuid;

use crate::error_code::internal_error;
use crate::error_code::invalid_params;
use crate::error_code::invalid_request;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingMessageSender;

const EXEC_TIMEOUT_EXIT_CODE: i32 = 124;
const OUTPUT_CHUNK_SIZE_HINT: usize = 64 * 1024;
const TERMINAL_REPLAY_BYTES_CAP: usize = 1024 * 1024;
const DEFAULT_TTY_TERM: &str = "xterm-256color";

#[derive(Clone)]
pub(crate) struct CommandExecManager {
    sessions: Arc<Mutex<HashMap<ConnectionProcessId, CommandExecSession>>>,
    next_generated_process_id: Arc<AtomicI64>,
}

impl Default for CommandExecManager {
    fn default() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_generated_process_id: Arc::new(AtomicI64::new(1)),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ConnectionProcessId {
    connection_id: ConnectionId,
    process_id: InternalProcessId,
}

#[derive(Clone)]
enum CommandExecSession {
    Active {
        control_tx: mpsc::Sender<CommandControlRequest>,
        info: UserTerminalSessionInfo,
    },
    UnsupportedWindowsSandbox,
}

#[derive(Clone)]
pub(crate) struct UserTerminalSessionInfo {
    pub(crate) process_id: String,
    pub(crate) generation: String,
    resume_token: String,
    pub(crate) command: Vec<String>,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) tty: bool,
    owner_connection_id: ConnectionId,
    runtime: Arc<Mutex<UserTerminalRuntimeState>>,
    delivery_lock: Arc<Mutex<()>>,
}

struct UserTerminalRuntimeState {
    notification_connection_id: ConnectionId,
    authorized_connection_ids: HashSet<ConnectionId>,
    replay: Vec<u8>,
    replay_truncated: bool,
    replay_through_sequence: u64,
    size: Option<TerminalSize>,
}

#[derive(Clone)]
pub(crate) struct UserTerminalSessionSnapshot {
    pub(crate) process_id: String,
    pub(crate) generation: String,
    pub(crate) command: Vec<String>,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) replay_base64: Option<String>,
    pub(crate) replay_truncated: bool,
    pub(crate) replay_through_sequence: u64,
    pub(crate) size: Option<TerminalSize>,
}

enum CommandControl {
    Write { delta: Vec<u8>, close_stdin: bool },
    Resize { size: TerminalSize },
    Terminate,
}

struct CommandControlRequest {
    control: CommandControl,
    response_tx: Option<oneshot::Sender<Result<(), JSONRPCErrorError>>>,
}

pub(crate) struct StartCommandExecParams {
    pub(crate) outgoing: Arc<OutgoingMessageSender>,
    pub(crate) request_id: ConnectionRequestId,
    pub(crate) process_id: Option<String>,
    pub(crate) exec_request: ExecRequest,
    pub(crate) started_network_proxy: Option<StartedNetworkProxy>,
    pub(crate) tty: bool,
    pub(crate) stream_stdin: bool,
    pub(crate) stream_stdout_stderr: bool,
    pub(crate) output_bytes_cap: Option<usize>,
    pub(crate) size: Option<TerminalSize>,
}

struct RunCommandParams {
    outgoing: Arc<OutgoingMessageSender>,
    request_id: ConnectionRequestId,
    process_id: Option<String>,
    generation: Option<String>,
    terminal_runtime: Arc<Mutex<UserTerminalRuntimeState>>,
    terminal_delivery_lock: Arc<Mutex<()>>,
    spawned: SpawnedProcess,
    control_rx: mpsc::Receiver<CommandControlRequest>,
    tty: bool,
    stream_stdin: bool,
    stream_stdout_stderr: bool,
    expiration: ExecExpiration,
    output_bytes_cap: Option<usize>,
}

struct SpawnProcessOutputParams {
    process_id: Option<String>,
    generation: Option<String>,
    terminal_runtime: Arc<Mutex<UserTerminalRuntimeState>>,
    terminal_delivery_lock: Arc<Mutex<()>>,
    output_rx: mpsc::Receiver<Vec<u8>>,
    stdio_timeout_rx: watch::Receiver<bool>,
    outgoing: Arc<OutgoingMessageSender>,
    stream: CommandExecOutputStream,
    stream_output: bool,
    output_bytes_cap: Option<usize>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum InternalProcessId {
    Generated(i64),
    Client(String),
}

trait InternalProcessIdExt {
    fn error_repr(&self) -> String;
}

impl InternalProcessIdExt for InternalProcessId {
    fn error_repr(&self) -> String {
        match self {
            Self::Generated(id) => id.to_string(),
            Self::Client(id) => serde_json::to_string(id).unwrap_or_else(|_| format!("{id:?}")),
        }
    }
}

impl CommandExecManager {
    pub(crate) async fn start(
        &self,
        params: StartCommandExecParams,
    ) -> Result<(), JSONRPCErrorError> {
        let StartCommandExecParams {
            outgoing,
            request_id,
            process_id,
            exec_request,
            started_network_proxy,
            tty,
            stream_stdin,
            stream_stdout_stderr,
            output_bytes_cap,
            size,
        } = params;
        if process_id.is_none() && (tty || stream_stdin || stream_stdout_stderr) {
            return Err(invalid_request(
                "command/exec tty or streaming requires a client-supplied processId",
            ));
        }
        let process_id = process_id.map_or_else(
            || {
                InternalProcessId::Generated(
                    self.next_generated_process_id
                        .fetch_add(1, Ordering::Relaxed),
                )
            },
            InternalProcessId::Client,
        );
        let process_key = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: process_id.clone(),
        };

        if matches!(exec_request.sandbox, SandboxType::WindowsRestrictedToken) {
            if tty || stream_stdin || stream_stdout_stderr {
                return Err(invalid_request(
                    "streaming command/exec is not supported with windows sandbox",
                ));
            }
            if output_bytes_cap != Some(DEFAULT_OUTPUT_BYTES_CAP) {
                return Err(invalid_request(
                    "custom outputBytesCap is not supported with windows sandbox",
                ));
            }
            if let InternalProcessId::Client(_) = &process_id {
                let mut sessions = self.sessions.lock().await;
                if sessions.contains_key(&process_key) {
                    return Err(invalid_request(format!(
                        "duplicate active command/exec process id: {}",
                        process_key.process_id.error_repr(),
                    )));
                }
                sessions.insert(
                    process_key.clone(),
                    CommandExecSession::UnsupportedWindowsSandbox,
                );
            }
            let sessions = Arc::clone(&self.sessions);
            tokio::spawn(async move {
                let _started_network_proxy = started_network_proxy;
                match execute_exec_request(
                    exec_request,
                    /*stdout_stream*/ None,
                    /*after_spawn*/ None,
                )
                .await
                {
                    Ok(output) => {
                        outgoing
                            .send_response(
                                request_id,
                                CommandExecResponse {
                                    exit_code: output.exit_code,
                                    stdout: output.stdout.text,
                                    stderr: output.stderr.text,
                                },
                            )
                            .await;
                    }
                    Err(err) => {
                        outgoing
                            .send_error(request_id, internal_error(format!("exec failed: {err}")))
                            .await;
                    }
                }
                sessions.lock().await.remove(&process_key);
            });
            return Ok(());
        }

        let ExecRequest {
            command,
            cwd,
            env,
            expiration,
            sandbox: _sandbox,
            arg0,
            ..
        } = exec_request;
        let env = effective_spawn_env(env, tty);

        let stream_stdin = tty || stream_stdin;
        let stream_stdout_stderr = tty || stream_stdout_stderr;
        let (control_tx, control_rx) = mpsc::channel(32);
        let notification_process_id = match &process_id {
            InternalProcessId::Generated(_) => None,
            InternalProcessId::Client(process_id) => Some(process_id.clone()),
        };
        let session_info = UserTerminalSessionInfo {
            process_id: notification_process_id.clone().unwrap_or_default(),
            generation: notification_process_id
                .as_ref()
                .map(|_| new_terminal_generation())
                .unwrap_or_default(),
            resume_token: notification_process_id
                .as_ref()
                .map(|_| new_terminal_resume_token())
                .unwrap_or_default(),
            command: command.clone(),
            cwd: cwd.as_path().to_path_buf(),
            tty,
            owner_connection_id: request_id.connection_id,
            runtime: Arc::new(Mutex::new(UserTerminalRuntimeState {
                notification_connection_id: request_id.connection_id,
                authorized_connection_ids: [request_id.connection_id].into_iter().collect(),
                replay: Vec::new(),
                replay_truncated: false,
                replay_through_sequence: 0,
                size: tty.then(|| size.unwrap_or_default()),
            })),
            delivery_lock: Arc::new(Mutex::new(())),
        };
        let generation = notification_process_id
            .as_ref()
            .map(|_| session_info.generation.clone());
        let terminal_runtime = Arc::clone(&session_info.runtime);
        let terminal_delivery_lock = Arc::clone(&session_info.delivery_lock);
        let terminal_generation = session_info.generation.clone();
        let terminal_resume_token = session_info.resume_token.clone();

        let sessions = Arc::clone(&self.sessions);
        let (program, args) = command
            .split_first()
            .ok_or_else(|| invalid_request("command must not be empty"))?;
        {
            let mut sessions = self.sessions.lock().await;
            let duplicate_terminal_id = tty
                && sessions.iter().any(|(key, session)| {
                    matches!(
                        (&key.process_id, session),
                        (
                            InternalProcessId::Client(existing),
                            CommandExecSession::Active { info, .. }
                        ) if info.tty && existing == &session_info.process_id
                    )
                });
            if sessions.contains_key(&process_key) || duplicate_terminal_id {
                return Err(invalid_request(format!(
                    "duplicate active command/exec process id: {}",
                    process_key.process_id.error_repr(),
                )));
            }
            sessions.insert(
                process_key.clone(),
                CommandExecSession::Active {
                    control_tx,
                    info: session_info,
                },
            );
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
                return Err(internal_error(format!("failed to spawn command: {err}")));
            }
        };
        if tty
            && let Some(process_id) = notification_process_id.as_ref()
        {
            outgoing
                .send_server_notification_to_connection_and_wait(
                    request_id.connection_id,
                    ServerNotification::CommandExecStarted(CommandExecStartedNotification {
                        process_id: process_id.clone(),
                        generation: terminal_generation,
                        resume_token: terminal_resume_token,
                    }),
                )
                .await;
        }
        tokio::spawn(async move {
            let _started_network_proxy = started_network_proxy;
            run_command(RunCommandParams {
                outgoing,
                request_id: request_id.clone(),
                process_id: notification_process_id,
                generation,
                terminal_runtime,
                terminal_delivery_lock,
                spawned,
                control_rx,
                tty,
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

    pub(crate) async fn write(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecWriteParams,
    ) -> Result<CommandExecWriteResponse, JSONRPCErrorError> {
        if params.delta_base64.is_none() && !params.close_stdin {
            return Err(invalid_params(
                "command/exec/write requires deltaBase64 or closeStdin",
            ));
        }

        let delta = match params.delta_base64 {
            Some(delta_base64) => STANDARD
                .decode(delta_base64)
                .map_err(|err| invalid_params(format!("invalid deltaBase64: {err}")))?,
            None => Vec::new(),
        };

        let target_process_id = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: InternalProcessId::Client(params.process_id),
        };
        self.send_control(
            target_process_id,
            None,
            CommandControl::Write {
                delta,
                close_stdin: params.close_stdin,
            },
        )
        .await?;

        Ok(CommandExecWriteResponse {})
    }

    pub(crate) async fn terminate(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecTerminateParams,
    ) -> Result<CommandExecTerminateResponse, JSONRPCErrorError> {
        let target_process_id = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: InternalProcessId::Client(params.process_id),
        };
        self.send_control(
            target_process_id,
            None,
            CommandControl::Terminate,
        )
            .await?;
        Ok(CommandExecTerminateResponse {})
    }

    pub(crate) async fn resize(
        &self,
        request_id: ConnectionRequestId,
        params: CommandExecResizeParams,
    ) -> Result<CommandExecResizeResponse, JSONRPCErrorError> {
        let target_process_id = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: InternalProcessId::Client(params.process_id),
        };
        self.send_control(
            target_process_id,
            None,
            CommandControl::Resize {
                size: terminal_size_from_protocol(params.size)?,
            },
        )
        .await?;
        Ok(CommandExecResizeResponse {})
    }

    pub(crate) async fn connection_closed(&self, connection_id: ConnectionId) {
        let controls = {
            let mut sessions = self.sessions.lock().await;
            let process_ids = sessions
                .keys()
                .filter(|process_id| process_id.connection_id == connection_id)
                .cloned()
                .collect::<Vec<_>>();
            let mut controls = Vec::with_capacity(process_ids.len());
            for process_id in process_ids {
                let keep_terminal = sessions.get(&process_id).is_some_and(|session| {
                    matches!(
                        session,
                        CommandExecSession::Active { info, .. } if info.tty
                    )
                });
                if !keep_terminal
                    && let Some(control) = sessions.remove(&process_id)
                {
                    controls.push(control);
                }
            }
            controls
        };

        for control in controls {
            if let CommandExecSession::Active { control_tx, .. } = control {
                let _ = control_tx
                    .send(CommandControlRequest {
                        control: CommandControl::Terminate,
                        response_tx: None,
                    })
                    .await;
            }
        }
    }

    async fn send_control(
        &self,
        process_id: ConnectionProcessId,
        expected_generation: Option<&str>,
        control: CommandControl,
    ) -> Result<(), JSONRPCErrorError> {
        let session = {
            self.sessions
                .lock()
                .await
                .get(&process_id)
                .cloned()
                .ok_or_else(|| {
                    invalid_request(format!(
                        "no active command/exec for process id {}",
                        process_id.process_id.error_repr(),
                    ))
                })?
        };
        let CommandExecSession::Active { control_tx, info } = session else {
            return Err(invalid_request(
                "command/exec/write, command/exec/terminate, and command/exec/resize are not supported for windows sandbox processes",
            ));
        };
        if let Some(expected_generation) = expected_generation
            && info.generation != expected_generation
        {
            return Err(invalid_request(format!(
                "stale command/exec generation for process id {}",
                process_id.process_id.error_repr(),
            )));
        }
        let (response_tx, response_rx) = oneshot::channel();
        let request = CommandControlRequest {
            control,
            response_tx: Some(response_tx),
        };
        control_tx
            .send(request)
            .await
            .map_err(|_| command_no_longer_running_error(&process_id.process_id))?;
        response_rx
            .await
            .map_err(|_| command_no_longer_running_error(&process_id.process_id))?
    }

    pub(crate) async fn list(
        &self,
        connection_id: ConnectionId,
        reattach_tokens: &[String],
    ) -> Vec<UserTerminalSessionSnapshot> {
        let infos = {
            let sessions = self.sessions.lock().await;
            sessions
            .values()
            .filter_map(|session| {
                match session {
                    CommandExecSession::Active { info, .. }
                        if info.tty
                            && (info.owner_connection_id == connection_id
                                || reattach_tokens.contains(&info.resume_token)) =>
                    {
                        Some(info.clone())
                    }
                    CommandExecSession::Active { .. }
                    | CommandExecSession::UnsupportedWindowsSandbox => None,
                }
            })
            .collect::<Vec<_>>()
        };
        let mut data = Vec::with_capacity(infos.len());
        for info in infos {
            let mut runtime = info.runtime.lock().await;
            runtime.notification_connection_id = connection_id;
            runtime.authorized_connection_ids.insert(connection_id);
            data.push(UserTerminalSessionSnapshot {
                process_id: info.process_id,
                generation: info.generation,
                command: info.command,
                cwd: info.cwd,
                replay_base64: (!runtime.replay.is_empty()).then(|| STANDARD.encode(&runtime.replay)),
                replay_truncated: runtime.replay_truncated,
                replay_through_sequence: runtime.replay_through_sequence,
                size: runtime.size,
            });
        }
        data.sort_by(|left, right| left.process_id.cmp(&right.process_id));
        data
    }

    pub(crate) async fn write_terminal(
        &self,
        connection_id: ConnectionId,
        process_id: String,
        generation: &str,
        resume_token: Option<&str>,
        delta: Vec<u8>,
    ) -> Result<(), JSONRPCErrorError> {
        self.send_terminal_control(
            connection_id,
            process_id,
            generation,
            resume_token,
            CommandControl::Write {
                delta,
                close_stdin: false,
            },
        )
        .await
    }

    pub(crate) async fn resize_terminal(
        &self,
        connection_id: ConnectionId,
        process_id: String,
        generation: &str,
        resume_token: Option<&str>,
        size: TerminalSize,
    ) -> Result<(), JSONRPCErrorError> {
        self.send_terminal_control(
            connection_id,
            process_id,
            generation,
            resume_token,
            CommandControl::Resize { size },
        )
        .await
    }

    pub(crate) async fn terminate_terminal(
        &self,
        connection_id: ConnectionId,
        process_id: String,
        generation: &str,
        resume_token: Option<&str>,
    ) -> Result<(), JSONRPCErrorError> {
        self.send_terminal_control(
            connection_id,
            process_id,
            generation,
            resume_token,
            CommandControl::Terminate,
        )
        .await
    }

    async fn send_terminal_control(
        &self,
        connection_id: ConnectionId,
        process_id: String,
        generation: &str,
        resume_token: Option<&str>,
        control: CommandControl,
    ) -> Result<(), JSONRPCErrorError> {
        let target = {
            let sessions = self.sessions.lock().await;
            sessions.iter().find_map(|(key, session)| match session {
                CommandExecSession::Active { info, .. }
                    if info.tty
                        && info.process_id == process_id
                        && info.generation == generation =>
                {
                    Some((key.clone(), info.clone()))
                }
                CommandExecSession::Active { .. }
                | CommandExecSession::UnsupportedWindowsSandbox => None,
            })
        }
        .ok_or_else(|| invalid_request("terminal session is no longer running"))?;
        let (target, info) = target;
        if info.owner_connection_id != connection_id
            && (resume_token != Some(info.resume_token.as_str())
                || !info
                    .runtime
                    .lock()
                    .await
                    .authorized_connection_ids
                    .contains(&connection_id))
        {
            return Err(invalid_request("terminal session is not attached to this connection"));
        }
        self.send_control(target, Some(generation), control).await
    }
}

fn effective_spawn_env(
    mut env: HashMap<String, String>,
    tty: bool,
) -> HashMap<String, String> {
    if tty && !cfg!(windows) {
        env.entry("TERM".to_string())
            .or_insert_with(|| DEFAULT_TTY_TERM.to_string());
    }
    env
}

async fn run_command(params: RunCommandParams) {
    let RunCommandParams {
        outgoing,
        request_id,
        process_id,
        generation,
        terminal_runtime,
        terminal_delivery_lock,
        spawned,
        control_rx,
        tty,
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

    let stdout_handle = spawn_process_output(SpawnProcessOutputParams {
        process_id: process_id.clone(),
        generation: generation.clone(),
        terminal_runtime: Arc::clone(&terminal_runtime),
        terminal_delivery_lock: Arc::clone(&terminal_delivery_lock),
        output_rx: stdout_rx,
        stdio_timeout_rx: stdio_timeout_rx.clone(),
        outgoing: Arc::clone(&outgoing),
        stream: CommandExecOutputStream::Stdout,
        stream_output: stream_stdout_stderr,
        output_bytes_cap,
    });
    let stderr_handle = spawn_process_output(SpawnProcessOutputParams {
        process_id: process_id.clone(),
        generation: generation.clone(),
        terminal_runtime: Arc::clone(&terminal_runtime),
        terminal_delivery_lock,
        output_rx: stderr_rx,
        stdio_timeout_rx,
        outgoing: Arc::clone(&outgoing),
        stream: CommandExecOutputStream::Stderr,
        stream_output: stream_stdout_stderr,
        output_bytes_cap,
    });

    let exit_code = loop {
        tokio::select! {
            control = control_rx.recv(), if control_open => {
                match control {
                    Some(CommandControlRequest { control, response_tx }) => {
                        let result = match control {
                            CommandControl::Write { delta, close_stdin } => {
                                handle_process_write(
                                    &session,
                                    stream_stdin,
                                    delta,
                                    close_stdin,
                                ).await
                            }
                            CommandControl::Resize { size } => {
                                let result = handle_process_resize(&session, size);
                                if result.is_ok() {
                                    terminal_runtime.lock().await.size = Some(size);
                                }
                                result
                            }
                            CommandControl::Terminate => {
                                session.request_terminate();
                                Ok(())
                            }
                        };
                        if let Some(response_tx) = response_tx {
                            let _ = response_tx.send(result);
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

    let timeout_handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(IO_DRAIN_TIMEOUT_MS)).await;
        let _ = stdio_timeout_tx.send(true);
    });

    let stdout = stdout_handle.await.unwrap_or_default();
    let stderr = stderr_handle.await.unwrap_or_default();
    timeout_handle.abort();

    if tty
        && let Some(process_id) = process_id.as_ref()
    {
        outgoing
            .send_server_notification_to_connection_and_wait(
                terminal_notification_connection_id(&terminal_runtime).await,
                ServerNotification::CommandExecExited(CommandExecExitedNotification {
                    process_id: process_id.clone(),
                    generation: generation.clone().unwrap_or_default(),
                    exit_code,
                }),
            )
            .await;
    }

    outgoing
        .send_response(
            request_id,
            CommandExecResponse {
                exit_code,
                stdout,
                stderr,
            },
        )
        .await;
}

fn spawn_process_output(params: SpawnProcessOutputParams) -> tokio::task::JoinHandle<String> {
    let SpawnProcessOutputParams {
        process_id,
        generation,
        terminal_runtime,
        terminal_delivery_lock,
        mut output_rx,
        mut stdio_timeout_rx,
        outgoing,
        stream,
        stream_output,
        output_bytes_cap,
    } = params;
    tokio::spawn(async move {
        let mut buffer: Vec<u8> = Vec::new();
        let mut observed_num_bytes = 0usize;
        loop {
            let mut chunk = tokio::select! {
                chunk = output_rx.recv() => match chunk {
                    Some(chunk) => chunk,
                    None => break,
                },
                _ = stdio_timeout_rx.wait_for(|&v| v) => break,
            };
            // Individual chunks are at most 8KiB, so overshooting a bit is acceptable.
            while chunk.len() < OUTPUT_CHUNK_SIZE_HINT
                && let Ok(next_chunk) = output_rx.try_recv()
            {
                chunk.extend_from_slice(&next_chunk);
            }
            let capped_chunk = match output_bytes_cap {
                Some(output_bytes_cap) => {
                    let capped_chunk_len = output_bytes_cap
                        .saturating_sub(observed_num_bytes)
                        .min(chunk.len());
                    observed_num_bytes += capped_chunk_len;
                    &chunk[0..capped_chunk_len]
                }
                None => chunk.as_slice(),
            };
            let cap_reached = Some(observed_num_bytes) == output_bytes_cap;
            if let (true, Some(process_id)) = (stream_output, process_id.as_ref()) {
                // stdout and stderr readers run independently. Keep replay
                // sequence allocation and delivery serialized so the list
                // snapshot watermark always describes a prefix of deltas.
                let _delivery_guard = terminal_delivery_lock.lock().await;
                let sequence = append_user_terminal_replay(&terminal_runtime, capped_chunk).await;
                outgoing
                    .send_server_notification_to_connection_and_wait(
                        terminal_notification_connection_id(&terminal_runtime).await,
                        ServerNotification::CommandExecOutputDelta(
                            CommandExecOutputDeltaNotification {
                                process_id: process_id.clone(),
                                generation: generation.clone().unwrap_or_default(),
                                sequence,
                                stream,
                                delta_base64: STANDARD.encode(capped_chunk),
                                cap_reached,
                            },
                        ),
                    )
                    .await;
            } else if !stream_output {
                buffer.extend_from_slice(capped_chunk);
            }
            if cap_reached {
                break;
            }
        }
        bytes_to_string_smart(&buffer)
    })
}

async fn append_user_terminal_replay(
    runtime: &Mutex<UserTerminalRuntimeState>,
    chunk: &[u8],
) -> u64 {
    let mut runtime = runtime.lock().await;
    runtime.replay_through_sequence = runtime.replay_through_sequence.saturating_add(1);
    if !chunk.is_empty() {
        runtime.replay.extend_from_slice(chunk);
        if runtime.replay.len() > TERMINAL_REPLAY_BYTES_CAP {
            let excess = runtime.replay.len() - TERMINAL_REPLAY_BYTES_CAP;
            runtime.replay.drain(..excess);
            runtime.replay_truncated = true;
        }
    }
    runtime.replay_through_sequence
}

async fn terminal_notification_connection_id(
    runtime: &Mutex<UserTerminalRuntimeState>,
) -> ConnectionId {
    runtime.lock().await.notification_connection_id
}

async fn handle_process_write(
    session: &ProcessHandle,
    stream_stdin: bool,
    delta: Vec<u8>,
    close_stdin: bool,
) -> Result<(), JSONRPCErrorError> {
    if !stream_stdin {
        return Err(invalid_request(
            "stdin streaming is not enabled for this command/exec",
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

pub(crate) fn terminal_size_from_protocol(
    size: CommandExecTerminalSize,
) -> Result<TerminalSize, JSONRPCErrorError> {
    if size.rows == 0 || size.cols == 0 {
        return Err(invalid_params(
            "command/exec size rows and cols must be greater than 0",
        ));
    }
    Ok(TerminalSize {
        rows: size.rows,
        cols: size.cols,
    })
}

fn command_no_longer_running_error(process_id: &InternalProcessId) -> JSONRPCErrorError {
    invalid_request(format!(
        "command/exec {} is no longer running",
        process_id.error_repr(),
    ))
}

fn new_terminal_generation() -> String {
    Uuid::now_v7().to_string()
}

fn new_terminal_resume_token() -> String {
    Uuid::now_v7().to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::error_code::INVALID_REQUEST_ERROR_CODE;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use command_service_api::ExecCapturePolicy;
    use pretty_assertions::assert_eq;
    use protocol::config_types::WindowsSandboxLevel;
    use protocol::models::PermissionProfile;
    #[cfg(not(target_os = "windows"))]
    use tokio::time::Duration;
    #[cfg(not(target_os = "windows"))]
    use tokio::time::timeout;
    #[cfg(not(target_os = "windows"))]
    use tokio_util::sync::CancellationToken;

    use super::*;
    #[cfg(not(target_os = "windows"))]
    use crate::outgoing_message::OutgoingEnvelope;
    #[cfg(not(target_os = "windows"))]
    use crate::outgoing_message::OutgoingMessage;

    fn windows_sandbox_exec_request() -> ExecRequest {
        let cwd = AbsolutePathBuf::current_dir().expect("current dir");
        ExecRequest::new(
            vec!["cmd".to_string()],
            cwd,
            HashMap::new(),
            /*network*/ None,
            ExecExpiration::DefaultTimeout,
            ExecCapturePolicy::ShellTool,
            SandboxType::WindowsRestrictedToken,
            WindowsSandboxLevel::Disabled,
            /*windows_sandbox_private_desktop*/ false,
            PermissionProfile::read_only(),
            /*arg0*/ None,
        )
    }

    #[test]
    fn tty_spawn_environment_defaults_missing_term() {
        let env = effective_spawn_env(HashMap::new(), true);

        #[cfg(not(windows))]
        assert_eq!(env.get("TERM"), Some(&DEFAULT_TTY_TERM.to_string()));
        #[cfg(windows)]
        assert!(!env.contains_key("TERM"));
    }

    #[test]
    fn tty_spawn_environment_preserves_explicit_term() {
        let env = effective_spawn_env(
            HashMap::from([("TERM".to_string(), "screen-256color".to_string())]),
            true,
        );

        assert_eq!(env.get("TERM"), Some(&"screen-256color".to_string()));
    }

    #[test]
    fn pipe_spawn_environment_does_not_add_term() {
        let env = effective_spawn_env(HashMap::new(), false);

        assert!(!env.contains_key("TERM"));
    }

    #[tokio::test]
    async fn windows_sandbox_streaming_exec_is_rejected() {
        let (tx, _rx) = mpsc::channel(1);
        let manager = CommandExecManager::default();
        let err = manager
            .start(StartCommandExecParams {
                outgoing: Arc::new(OutgoingMessageSender::new(
                    tx,
                    codex_analytics::AnalyticsEventsClient::disabled(),
                )),
                request_id: ConnectionRequestId {
                    connection_id: ConnectionId(1),
                    request_id: app_server_protocol::RequestId::Integer(42),
                },
                process_id: Some("proc-42".to_string()),
                exec_request: windows_sandbox_exec_request(),
                started_network_proxy: None,
                tty: false,
                stream_stdin: false,
                stream_stdout_stderr: true,
                output_bytes_cap: None,
                size: None,
            })
            .await
            .expect_err("streaming windows sandbox exec should be rejected");

        assert_eq!(err.code, INVALID_REQUEST_ERROR_CODE);
        assert_eq!(
            err.message,
            "streaming command/exec is not supported with windows sandbox"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn windows_sandbox_non_streaming_exec_uses_execution_path() {
        let (tx, mut rx) = mpsc::channel(1);
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(7),
            request_id: app_server_protocol::RequestId::Integer(99),
        };

        manager
            .start(StartCommandExecParams {
                outgoing: Arc::new(OutgoingMessageSender::new(
                    tx,
                    codex_analytics::AnalyticsEventsClient::disabled(),
                )),
                request_id: request_id.clone(),
                process_id: Some("proc-99".to_string()),
                exec_request: windows_sandbox_exec_request(),
                started_network_proxy: None,
                tty: false,
                stream_stdin: false,
                stream_stdout_stderr: false,
                output_bytes_cap: Some(DEFAULT_OUTPUT_BYTES_CAP),
                size: None,
            })
            .await
            .expect("non-streaming windows sandbox exec should start");

        let envelope = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for outgoing message")
            .expect("channel closed before outgoing message");
        let OutgoingEnvelope::ToConnection {
            connection_id,
            message,
            ..
        } = envelope
        else {
            panic!("expected connection-scoped outgoing message");
        };
        assert_eq!(connection_id, request_id.connection_id);
        let OutgoingMessage::Error(error) = message else {
            panic!("expected execution failure to be reported as an error");
        };
        assert_eq!(error.id, request_id.request_id);
        assert!(error.error.message.starts_with("exec failed:"));
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn cancellation_expiration_keeps_process_alive_until_terminated() {
        let (tx, mut rx) = mpsc::channel(4);
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(8),
            request_id: app_server_protocol::RequestId::Integer(100),
        };
        let cwd = AbsolutePathBuf::current_dir().expect("current dir");

        manager
            .start(StartCommandExecParams {
                outgoing: Arc::new(OutgoingMessageSender::new(
                    tx,
                    codex_analytics::AnalyticsEventsClient::disabled(),
                )),
                request_id: request_id.clone(),
                process_id: Some("proc-100".to_string()),
                exec_request: ExecRequest::new(
                    vec!["sh".to_string(), "-lc".to_string(), "sleep 30".to_string()],
                    cwd.clone(),
                    HashMap::new(),
                    /*network*/ None,
                    ExecExpiration::Cancellation(CancellationToken::new()),
                    ExecCapturePolicy::ShellTool,
                    SandboxType::None,
                    WindowsSandboxLevel::Disabled,
                    /*windows_sandbox_private_desktop*/ false,
                    PermissionProfile::read_only(),
                    /*arg0*/ None,
                ),
                started_network_proxy: None,
                tty: false,
                stream_stdin: false,
                stream_stdout_stderr: false,
                output_bytes_cap: Some(DEFAULT_OUTPUT_BYTES_CAP),
                size: None,
            })
            .await
            .expect("cancellation-based exec should start");

        assert!(
            timeout(Duration::from_millis(250), rx.recv())
                .await
                .is_err(),
            "command/exec should remain active until explicit termination",
        );

        manager
            .terminate(
                request_id.clone(),
                CommandExecTerminateParams {
                    process_id: "proc-100".to_string(),
                },
            )
            .await
            .expect("terminate should succeed");

        let envelope = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for outgoing message")
            .expect("channel closed before outgoing message");
        let OutgoingEnvelope::ToConnection {
            connection_id,
            message,
            ..
        } = envelope
        else {
            panic!("expected connection-scoped outgoing message");
        };
        assert_eq!(connection_id, request_id.connection_id);
        let OutgoingMessage::Response(response) = message else {
            panic!("expected execution response after termination");
        };
        assert_eq!(response.id, request_id.request_id);
        let response: CommandExecResponse =
            serde_json::from_value(response.result).expect("deserialize command/exec response");
        assert_ne!(response.exit_code, 0);
        assert_eq!(response.stdout, "");
        // The deferred response now drains any already-emitted stderr before
        // replying, so shell startup noise is allowed here.
    }

    #[cfg(not(target_os = "windows"))]
    #[tokio::test]
    async fn timeout_or_cancellation_reports_cancellation_without_timeout_exit_code() {
        let (tx, mut rx) = mpsc::channel(4);
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(9),
            request_id: app_server_protocol::RequestId::Integer(101),
        };
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();

        manager
            .start(StartCommandExecParams {
                outgoing: Arc::new(OutgoingMessageSender::new(
                    tx,
                    codex_analytics::AnalyticsEventsClient::disabled(),
                )),
                request_id: request_id.clone(),
                process_id: Some("proc-101".to_string()),
                exec_request: ExecRequest::new(
                    vec!["sh".to_string(), "-lc".to_string(), "sleep 30".to_string()],
                    AbsolutePathBuf::current_dir().expect("current dir"),
                    HashMap::new(),
                    /*network*/ None,
                    ExecExpiration::TimeoutOrCancellation {
                        timeout: Duration::from_secs(30),
                        cancellation,
                    },
                    ExecCapturePolicy::ShellTool,
                    SandboxType::None,
                    WindowsSandboxLevel::Disabled,
                    /*windows_sandbox_private_desktop*/ false,
                    PermissionProfile::read_only(),
                    /*arg0*/ None,
                ),
                started_network_proxy: None,
                tty: false,
                stream_stdin: false,
                stream_stdout_stderr: false,
                output_bytes_cap: Some(DEFAULT_OUTPUT_BYTES_CAP),
                size: None,
            })
            .await
            .expect("timeout-or-cancellation exec should start");

        cancel.cancel();

        let envelope = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for outgoing message")
            .expect("channel closed before outgoing message");
        let OutgoingEnvelope::ToConnection {
            connection_id,
            message,
            ..
        } = envelope
        else {
            panic!("expected connection-scoped outgoing message");
        };
        assert_eq!(connection_id, request_id.connection_id);
        let OutgoingMessage::Response(response) = message else {
            panic!("expected execution response after cancellation");
        };
        assert_eq!(response.id, request_id.request_id);
        let response: CommandExecResponse =
            serde_json::from_value(response.result).expect("deserialize command/exec response");
        assert_ne!(response.exit_code, EXEC_TIMEOUT_EXIT_CODE);
    }

    #[tokio::test]
    async fn windows_sandbox_process_ids_reject_write_requests() {
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(11),
            request_id: app_server_protocol::RequestId::Integer(1),
        };
        let process_id = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: InternalProcessId::Client("proc-11".to_string()),
        };
        manager
            .sessions
            .lock()
            .await
            .insert(process_id, CommandExecSession::UnsupportedWindowsSandbox);

        let err = manager
            .write(
                request_id,
                CommandExecWriteParams {
                    process_id: "proc-11".to_string(),
                    delta_base64: Some(STANDARD.encode("hello")),
                    close_stdin: false,
                },
            )
            .await
            .expect_err("windows sandbox process ids should reject command/exec/write");

        assert_eq!(err.code, INVALID_REQUEST_ERROR_CODE);
        assert_eq!(
            err.message,
            "command/exec/write, command/exec/terminate, and command/exec/resize are not supported for windows sandbox processes"
        );
    }

    #[tokio::test]
    async fn windows_sandbox_process_ids_reject_terminate_requests() {
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(12),
            request_id: app_server_protocol::RequestId::Integer(2),
        };
        let process_id = ConnectionProcessId {
            connection_id: request_id.connection_id,
            process_id: InternalProcessId::Client("proc-12".to_string()),
        };
        manager
            .sessions
            .lock()
            .await
            .insert(process_id, CommandExecSession::UnsupportedWindowsSandbox);

        let err = manager
            .terminate(
                request_id,
                CommandExecTerminateParams {
                    process_id: "proc-12".to_string(),
                },
            )
            .await
            .expect_err("windows sandbox process ids should reject command/exec/terminate");

        assert_eq!(err.code, INVALID_REQUEST_ERROR_CODE);
        assert_eq!(
            err.message,
            "command/exec/write, command/exec/terminate, and command/exec/resize are not supported for windows sandbox processes"
        );
    }

    #[tokio::test]
    async fn dropped_control_request_is_reported_as_not_running() {
        let manager = CommandExecManager::default();
        let request_id = ConnectionRequestId {
            connection_id: ConnectionId(13),
            request_id: app_server_protocol::RequestId::Integer(3),
        };
        let process_id = InternalProcessId::Client("proc-13".to_string());
        let (control_tx, mut control_rx) = mpsc::channel(1);
        manager.sessions.lock().await.insert(
            ConnectionProcessId {
                connection_id: request_id.connection_id,
                process_id: process_id.clone(),
            },
            CommandExecSession::Active {
                control_tx,
                info: UserTerminalSessionInfo {
                    process_id: "proc-13".to_string(),
                    generation: "proc-13".to_string(),
                    resume_token: "resume-13".to_string(),
                    command: vec!["sh".to_string()],
                    cwd: std::path::PathBuf::from("/tmp"),
                    tty: true,
                    owner_connection_id: request_id.connection_id,
                    runtime: Arc::new(Mutex::new(UserTerminalRuntimeState {
                        notification_connection_id: request_id.connection_id,
                        authorized_connection_ids: [request_id.connection_id].into_iter().collect(),
                        replay: Vec::new(),
                        replay_truncated: false,
                        replay_through_sequence: 0,
                        size: Some(TerminalSize::default()),
                    })),
                    delivery_lock: Arc::new(Mutex::new(())),
                },
            },
        );

        tokio::spawn(async move {
            let _request = control_rx
                .recv()
                .await
                .expect("expected queued control request");
        });

        let err = manager
            .terminate(
                request_id,
                CommandExecTerminateParams {
                    process_id: "proc-13".to_string(),
                },
            )
            .await
            .expect_err("dropped control request should be treated as not running");

        assert_eq!(err.code, INVALID_REQUEST_ERROR_CODE);
        assert_eq!(err.message, "command/exec \"proc-13\" is no longer running");
    }

    #[tokio::test]
    async fn terminal_session_survives_disconnect_and_rebinds_notifications() {
        let manager = CommandExecManager::default();
        let original_connection = ConnectionId(14);
        let reattached_connection = ConnectionId(15);
        let runtime = Arc::new(Mutex::new(UserTerminalRuntimeState {
            notification_connection_id: original_connection,
            authorized_connection_ids: [original_connection].into_iter().collect(),
            replay: b"ready".to_vec(),
            replay_truncated: false,
            replay_through_sequence: 1,
            size: Some(TerminalSize { rows: 33, cols: 120 }),
        }));
        let (control_tx, mut control_rx) = mpsc::channel(1);
        manager.sessions.lock().await.insert(
            ConnectionProcessId {
                connection_id: original_connection,
                process_id: InternalProcessId::Client("proc-14".to_string()),
            },
            CommandExecSession::Active {
                control_tx,
                info: UserTerminalSessionInfo {
                    process_id: "proc-14".to_string(),
                    generation: "generation-14".to_string(),
                    resume_token: "resume-14".to_string(),
                    command: vec!["sh".to_string()],
                    cwd: std::path::PathBuf::from("/tmp"),
                    tty: true,
                    owner_connection_id: original_connection,
                    runtime: Arc::clone(&runtime),
                    delivery_lock: Arc::new(Mutex::new(())),
                },
            },
        );

        manager.connection_closed(original_connection).await;
        assert!(
            manager.list(reattached_connection, &[]).await.is_empty(),
            "a different connection must not enumerate or claim a user PTY without its resume token",
        );
        let err = manager
            .write_terminal(
                reattached_connection,
                "proc-14".to_string(),
                "generation-14",
                Some("resume-14"),
                b"before-reattach".to_vec(),
            )
            .await
            .expect_err("resume token must not authorize control before explicit reattach");
        assert_eq!(err.message, "terminal session is not attached to this connection");
        assert!(control_rx.try_recv().is_err());
        let sessions = manager
            .list(reattached_connection, &["resume-14".to_string()])
            .await;

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].replay_base64.as_deref(), Some("cmVhZHk="));
        assert_eq!(sessions[0].replay_through_sequence, 1);
        assert_eq!(
            runtime.lock().await.notification_connection_id,
            reattached_connection,
        );
        tokio::spawn(async move {
            let request = control_rx.recv().await.expect("expected terminal control");
            let Some(response_tx) = request.response_tx else {
                panic!("control request should expect a response");
            };
            response_tx.send(Ok(())).expect("response receiver should be open");
        });
        manager
            .write_terminal(
                reattached_connection,
                "proc-14".to_string(),
                "generation-14",
                Some("resume-14"),
                b"after-reattach".to_vec(),
            )
            .await
            .expect("reattached connection should control its terminal");
    }

    #[tokio::test]
    async fn terminal_replay_is_bounded_and_sequence_is_monotonic() {
        let runtime = Mutex::new(UserTerminalRuntimeState {
            notification_connection_id: ConnectionId(16),
            authorized_connection_ids: [ConnectionId(16)].into_iter().collect(),
            replay: Vec::new(),
            replay_truncated: false,
            replay_through_sequence: 0,
            size: Some(TerminalSize::default()),
        });

        assert_eq!(append_user_terminal_replay(&runtime, b"first").await, 1);
        assert_eq!(
            append_user_terminal_replay(
                &runtime,
                &vec![b'x'; TERMINAL_REPLAY_BYTES_CAP + 32],
            )
            .await,
            2,
        );

        let runtime = runtime.lock().await;
        assert_eq!(runtime.replay.len(), TERMINAL_REPLAY_BYTES_CAP);
        assert!(runtime.replay_truncated);
        assert_eq!(runtime.replay_through_sequence, 2);
    }

    #[test]
    fn terminal_generation_is_fresh_for_each_process_lifetime() {
        let process_id = "reused-client-process-id";
        let first = new_terminal_generation();
        let second = new_terminal_generation();

        assert_ne!(first, process_id);
        assert_ne!(second, process_id);
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn stale_terminal_generation_is_rejected_before_control_delivery() {
        let manager = CommandExecManager::default();
        let connection_id = ConnectionId(17);
        let (control_tx, mut control_rx) = mpsc::channel(1);
        manager.sessions.lock().await.insert(
            ConnectionProcessId {
                connection_id,
                process_id: InternalProcessId::Client("proc-17".to_string()),
            },
            CommandExecSession::Active {
                control_tx,
                info: UserTerminalSessionInfo {
                    process_id: "proc-17".to_string(),
                    generation: "generation-17".to_string(),
                    resume_token: "resume-17".to_string(),
                    command: vec!["sh".to_string()],
                    cwd: std::path::PathBuf::from("/tmp"),
                    tty: true,
                    owner_connection_id: connection_id,
                    runtime: Arc::new(Mutex::new(UserTerminalRuntimeState {
                        notification_connection_id: connection_id,
                        authorized_connection_ids: [connection_id].into_iter().collect(),
                        replay: Vec::new(),
                        replay_truncated: false,
                        replay_through_sequence: 0,
                        size: Some(TerminalSize::default()),
                    })),
                    delivery_lock: Arc::new(Mutex::new(())),
                },
            },
        );

        let err = manager
            .write_terminal(
                connection_id,
                "proc-17".to_string(),
                "stale-generation",
                Some("resume-17"),
                b"input".to_vec(),
            )
            .await
            .expect_err("stale generation should fail");

        assert_eq!(err.code, INVALID_REQUEST_ERROR_CODE);
        assert_eq!(err.message, "terminal session is no longer running");
        assert!(control_rx.try_recv().is_err());
    }
}
