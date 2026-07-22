use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use codex_agent_roles::AgentRoleConfig;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::LiveAgent;
use codex_agent_runtime::SpawnAgentProvider;
use codex_utils_absolute_path::AbsolutePathBuf;
use config_service::Config;
use futures::StreamExt;
use futures::future::BoxFuture;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::protocol::AgentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use thread_store_api::SharedLiveThread;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStderr;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task::AbortHandle;

const MAX_EXTERNAL_OUTPUT_CHARS: usize = 12_000;
const MAX_EXTERNAL_ERROR_CHARS: usize = 4_000;
const MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS: usize = 8_000;
const CODEX_APP_SERVER_ENV_REMOVALS: &[&str] = &["CODEX_HOME", "CODEX_THREAD_ID"];

#[derive(Clone)]
pub(crate) struct ExternalAgentRun {
    pub(crate) thread_id: ThreadId,
    pub(crate) parent_thread_id: ThreadId,
    pub(crate) agent_path: AgentPath,
    pub(crate) provider: SpawnAgentProvider,
    pub(crate) depth: i32,
    pub(crate) spawn_config: Option<ExternalSpawnConfig>,
    pub(crate) input_sink: Option<ExternalInputSink>,
    pub(crate) live_thread: Option<SharedLiveThread>,
    pub(crate) status: AgentStatus,
    pub(crate) last_task_message: Option<String>,
    pub(crate) abort_handle: Option<AbortHandle>,
}

#[derive(Clone)]
pub(crate) struct ExternalSpawnConfig {
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) agent_max_threads: Option<usize>,
    pub(crate) agent_roles: BTreeMap<String, AgentRoleConfig>,
    pub(crate) model_provider_id: String,
    pub(crate) generate_memories: bool,
}

impl ExternalSpawnConfig {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            cwd: config.cwd.clone(),
            agent_max_threads: config.agent_max_threads,
            agent_roles: config.agent_roles.clone(),
            model_provider_id: config.model_provider_id.clone(),
            generate_memories: config.memories.generate_memories,
        }
    }
}

#[derive(Default)]
pub(crate) struct ExternalAgentRegistry {
    runs: Mutex<HashMap<ThreadId, ExternalAgentRun>>,
}

impl ExternalAgentRegistry {
    pub(crate) fn insert_running(&self, run: ExternalAgentRun) {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run.thread_id, run);
    }

    pub(crate) fn get(&self, thread_id: ThreadId) -> Option<ExternalAgentRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&thread_id)
            .cloned()
    }

    pub(crate) fn get_by_path(&self, agent_path: &AgentPath) -> Option<ExternalAgentRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .find(|run| &run.agent_path == agent_path)
            .cloned()
    }

    pub(crate) fn attach_abort_handle(&self, thread_id: ThreadId, abort_handle: AbortHandle) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = runs.get_mut(&thread_id) {
            run.abort_handle = Some(abort_handle);
        }
    }

    pub(crate) fn update_last_task_message(&self, thread_id: ThreadId, message: String) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = runs.get_mut(&thread_id) {
            run.last_task_message = Some(message);
        }
    }

    pub(crate) fn shutdown(&self, thread_id: ThreadId) -> Option<ExternalAgentRun> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = runs.get_mut(&thread_id)?;
        run.status = AgentStatus::Shutdown;
        if let Some(abort_handle) = run.abort_handle.take() {
            abort_handle.abort();
        }
        Some(run.clone())
    }

    pub(crate) fn set_terminal_status_if_active(
        &self,
        thread_id: ThreadId,
        status: AgentStatus,
    ) -> Option<ExternalAgentRun> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = runs.get_mut(&thread_id)?;
        if !matches!(
            run.status,
            AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
        ) {
            return None;
        }
        run.status = status;
        run.abort_handle = None;
        Some(run.clone())
    }

    pub(crate) fn list(&self) -> Vec<ExternalAgentRun> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn direct_children_are_active(&self, parent_thread_id: ThreadId) -> bool {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .any(|run| {
                run.parent_thread_id == parent_thread_id
                    && matches!(run.status, AgentStatus::PendingInit | AgentStatus::Running)
            })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExternalCliEvent {
    Status(String),
    Message(String),
    Completion(String),
    ToolCall(ExternalToolCall),
    ToolCallError(ExternalToolResult),
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalToolName {
    SpawnExternalAgent,
    FollowupExternalTask,
    ListExternalAgents,
    PollExternalEvent,
    CloseExternalAgent,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct ExternalToolCall {
    pub(crate) id: String,
    pub(crate) tool: ExternalToolName,
    #[serde(default)]
    pub(crate) arguments: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct ExternalToolResult {
    #[serde(rename = "type")]
    pub(crate) result_type: String,
    pub(crate) id: String,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ExternalToolError>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct ExternalToolError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl ExternalToolResult {
    pub(crate) fn ok(id: impl Into<String>, result: JsonValue) -> Self {
        Self {
            result_type: "external_tool_result".to_string(),
            id: id.into(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn error(
        id: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            result_type: "external_tool_result".to_string(),
            id: id.into(),
            ok: false,
            result: None,
            error: Some(ExternalToolError {
                code: code.into(),
                message: truncate_chars(&message.into(), MAX_EXTERNAL_ERROR_CHARS),
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalSessionSpec {
    pub(crate) program: &'static str,
    pub(crate) args: Vec<String>,
    pub(crate) transport: ExternalProviderSessionTransport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExternalProviderSessionTransport {
    /// Claude's stream-json CLI keeps the provider session open on stdin/stdout.
    ClaudeStreamJson,
    /// Codex CLI app-server exposes a persistent JSON-RPC session over stdio.
    CodexAppServerStdio,
    /// OpenCode's headless server exposes sessions over HTTP plus SSE events.
    OpencodeHttp,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExternalProcessEvent {
    Cli(ExternalCliEvent),
    StdinError(String),
    ProcessExited { success: bool, status: String },
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalInputSink {
    tx: mpsc::UnboundedSender<String>,
}

impl ExternalInputSink {
    pub(crate) fn new(tx: mpsc::UnboundedSender<String>) -> Self {
        Self { tx }
    }

    pub(crate) fn send(&self, content: String) -> Result<(), String> {
        self.tx
            .send(content)
            .map_err(|_| "external provider stdin is closed".to_string())
    }
}

pub(crate) trait ExternalProviderSession: Send {
    fn input_sink(&self) -> ExternalInputSink;
    fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>>;
}

pub(crate) fn external_session_spec(
    provider: SpawnAgentProvider,
    _cwd: &Path,
) -> Result<ExternalSessionSpec, String> {
    match provider {
        SpawnAgentProvider::Native => Err("native is not an external CLI provider".to_string()),
        SpawnAgentProvider::CodexCli => Ok(ExternalSessionSpec {
            program: "codex",
            args: vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string(),
            ],
            transport: ExternalProviderSessionTransport::CodexAppServerStdio,
        }),
        SpawnAgentProvider::ClaudeCli => Ok(ExternalSessionSpec {
            program: "claude",
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--input-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
            ],
            transport: ExternalProviderSessionTransport::ClaudeStreamJson,
        }),
        SpawnAgentProvider::Opencode => Ok(ExternalSessionSpec {
            program: "opencode",
            args: vec![
                "serve".to_string(),
                "--port".to_string(),
                "0".to_string(),
                "--hostname".to_string(),
                "127.0.0.1".to_string(),
                "--print-logs".to_string(),
            ],
            transport: ExternalProviderSessionTransport::OpencodeHttp,
        }),
    }
}

pub(crate) fn external_agent_context_prompt(message: &str) -> String {
    format!(
        r#"You are running as an external code agent connected to the Morpheus backend bus.

Use only this external-agent JSON protocol to collaborate with other agents. Do not call internal Morpheus tools such as spawn_agent, followup_task, list_agents, poll_event, or close_agent.

Available external tools:
- spawn_external_agent: arguments {{ "task_name": string, "provider": "claude_cli" | "opencode" | "codex_cli", "cwd": string, "message": string }}. Current external session transport support includes claude_cli stream-json, opencode HTTP sessions, and codex_cli app-server stdio sessions.
- followup_external_task: arguments {{ "target": string, "message": string }}
- list_external_agents: arguments {{ "path_prefix"?: string }}
- poll_external_event: currently unsupported for non-interactive CLI sessions; calling it returns an unsupported error.
- close_external_agent: arguments {{ "target": string }}

Emit one JSON object per line for tool calls:
{{"type":"external_tool_call","id":"call_1","tool":"list_external_agents","arguments":{{}}}}

The backend returns results as JSON objects:
{{"type":"external_tool_result","id":"call_1","ok":true,"result":{{}}}}
{{"type":"external_tool_result","id":"call_1","ok":false,"error":{{"code":"invalid_arguments","message":"..."}}}}

When the backend sends an external_tool_result as input, continue the task using that result. Emit another external_tool_call only if you need another backend action; otherwise finish with a normal final answer.

Original task:
{message}"#
    )
}

pub(crate) enum ExternalStreamingSession {
    Cli(ExternalCliSession),
    CodexAppServer(CodexAppServerSession),
    OpencodeHttp(OpencodeHttpSession),
}

pub(crate) struct ExternalCliSession {
    provider: SpawnAgentProvider,
    child: Child,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    stderr: tokio::io::Lines<BufReader<ChildStderr>>,
    stdout_open: bool,
    stderr_open: bool,
    input_sink: ExternalInputSink,
    writer_errors: mpsc::UnboundedReceiver<String>,
    writer_errors_open: bool,
}

impl ExternalProviderSession for ExternalStreamingSession {
    fn input_sink(&self) -> ExternalInputSink {
        match self {
            ExternalStreamingSession::Cli(session) => session.input_sink.clone(),
            ExternalStreamingSession::CodexAppServer(session) => session.input_sink.clone(),
            ExternalStreamingSession::OpencodeHttp(session) => session.input_sink.clone(),
        }
    }

    fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>> {
        Box::pin(async move {
            match self {
                ExternalStreamingSession::Cli(session) => session.next_event().await,
                ExternalStreamingSession::CodexAppServer(session) => session.next_event().await,
                ExternalStreamingSession::OpencodeHttp(session) => session.next_event().await,
            }
        })
    }
}

impl ExternalStreamingSession {
    pub(crate) async fn start(provider: SpawnAgentProvider, cwd: PathBuf) -> Result<Self, String> {
        let command = external_session_spec(provider, cwd.as_path())?;
        if command.transport == ExternalProviderSessionTransport::CodexAppServerStdio {
            return CodexAppServerSession::start(command, cwd)
                .await
                .map(ExternalStreamingSession::CodexAppServer);
        }
        if command.transport == ExternalProviderSessionTransport::OpencodeHttp {
            return OpencodeHttpSession::start(command, cwd)
                .await
                .map(ExternalStreamingSession::OpencodeHttp);
        }
        let transport = command.transport;
        let mut child = Command::new(command.program)
            .args(command.args)
            .current_dir(cwd)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| {
                format!(
                    "{} external provider unavailable: {err}",
                    provider_name(provider)
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            format!(
                "{} external provider did not expose stdin",
                provider_name(provider)
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            format!(
                "{} external provider did not expose stdout",
                provider_name(provider)
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            format!(
                "{} external provider did not expose stderr",
                provider_name(provider)
            )
        })?;
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (writer_error_tx, writer_errors) = mpsc::unbounded_channel();
        tokio::spawn(write_external_provider_input(
            transport,
            stdin,
            input_rx,
            writer_error_tx,
        ));
        Ok(ExternalStreamingSession::Cli(ExternalCliSession {
            provider,
            child,
            stdout: BufReader::new(stdout).lines(),
            stderr: BufReader::new(stderr).lines(),
            stdout_open: true,
            stderr_open: true,
            input_sink: ExternalInputSink::new(input_tx),
            writer_errors,
            writer_errors_open: true,
        }))
    }
}

pub(crate) struct CodexAppServerSession {
    child: Child,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    stderr: tokio::io::Lines<BufReader<ChildStderr>>,
    stdout_open: bool,
    stderr_open: bool,
    input_sink: ExternalInputSink,
    writer_errors: mpsc::UnboundedReceiver<String>,
    writer_errors_open: bool,
    active_turn_id: Arc<Mutex<Option<String>>>,
    pending_completion_event: Arc<Mutex<Option<ExternalCliEvent>>>,
}

impl CodexAppServerSession {
    async fn start(command: ExternalSessionSpec, cwd: PathBuf) -> Result<Self, String> {
        let mut command = codex_app_server_command(command, &cwd);
        let mut child = command
            .spawn()
            .map_err(|err| format!("codex_cli external provider unavailable: {err}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "codex_cli app-server did not expose stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "codex_cli app-server did not expose stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "codex_cli app-server did not expose stderr".to_string())?;
        let mut stdout = BufReader::new(stdout).lines();
        let mut stderr = BufReader::new(stderr).lines();

        send_codex_jsonrpc_request(
            &mut stdin,
            1,
            "initialize",
            serde_json::json!({
                "clientInfo": {
                    "name": "external_codex_cli",
                    "title": "External Codex CLI Agent",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            }),
        )
        .await?;
        read_codex_jsonrpc_response(&mut stdout, &mut stderr, 1).await?;
        write_codex_jsonrpc_line(
            &mut stdin,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "method": "initialized",
                "params": {},
            }),
        )
        .await?;

        send_codex_jsonrpc_request(
            &mut stdin,
            2,
            "thread/start",
            serde_json::json!({
                "cwd": cwd.to_string_lossy(),
                "threadSource": "subagent",
            }),
        )
        .await?;
        let start_response = read_codex_jsonrpc_response(&mut stdout, &mut stderr, 2).await?;
        let thread_id = start_response
            .get("result")
            .and_then(|result| result.get("thread"))
            .and_then(|thread| thread.get("id"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "codex_cli thread/start response did not include thread.id".to_string())?
            .to_string();

        let active_turn_id = Arc::new(Mutex::new(None));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (writer_error_tx, writer_errors) = mpsc::unbounded_channel();
        tokio::spawn(write_codex_app_server_input(
            stdin,
            thread_id,
            Arc::clone(&active_turn_id),
            input_rx,
            writer_error_tx,
        ));

        Ok(Self {
            child,
            stdout,
            stderr,
            stdout_open: true,
            stderr_open: true,
            input_sink: ExternalInputSink::new(input_tx),
            writer_errors,
            writer_errors_open: true,
            active_turn_id,
            pending_completion_event,
        })
    }

    async fn next_event(&mut self) -> Result<ExternalProcessEvent, String> {
        loop {
            if !self.stdout_open && !self.stderr_open {
                return self.next_process_event().await;
            }
            tokio::select! {
                error = self.writer_errors.recv(), if self.writer_errors_open => {
                    match error {
                        Some(error) => return Ok(ExternalProcessEvent::StdinError(error)),
                        None => {
                            self.writer_errors_open = false;
                        }
                    }
                }
                stdout = self.stdout.next_line(), if self.stdout_open => {
                    match stdout {
                        Ok(Some(line)) => {
                            if let Some(event) = parse_codex_app_server_jsonrpc_line(
                                &line,
                                &self.active_turn_id,
                                &self.pending_completion_event,
                            ) {
                                return Ok(event);
                            }
                        }
                        Ok(None) => {
                            self.stdout_open = false;
                        }
                        Err(err) => return Err(format!("failed to read codex_cli app-server stdout: {err}")),
                    }
                }
                stderr = self.stderr.next_line(), if self.stderr_open => {
                    match stderr {
                        Ok(Some(line)) => {
                            let line = line.trim();
                            if !line.is_empty() {
                                return Ok(ExternalProcessEvent::Cli(ExternalCliEvent::Status(
                                    truncate_chars(line, MAX_EXTERNAL_ERROR_CHARS),
                                )));
                            }
                        }
                        Ok(None) => {
                            self.stderr_open = false;
                        }
                        Err(err) => return Err(format!("failed to read codex_cli app-server stderr: {err}")),
                    }
                }
            }
        }
    }

    async fn next_process_event(&mut self) -> Result<ExternalProcessEvent, String> {
        let status = self.child.wait().await.map_err(|err| {
            format!("codex_cli app-server failed while waiting for process exit: {err}")
        })?;
        Ok(ExternalProcessEvent::ProcessExited {
            success: status.success(),
            status: status.to_string(),
        })
    }
}

fn codex_app_server_command(spec: ExternalSessionSpec, cwd: &Path) -> Command {
    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for env_name in CODEX_APP_SERVER_ENV_REMOVALS {
        command.env_remove(env_name);
    }
    command
}

impl ExternalCliSession {
    async fn next_event(&mut self) -> Result<ExternalProcessEvent, String> {
        loop {
            if !self.stdout_open && !self.stderr_open {
                return self.next_process_event().await;
            }
            tokio::select! {
                error = self.writer_errors.recv(), if self.writer_errors_open => {
                    match error {
                        Some(error) => return Ok(ExternalProcessEvent::StdinError(error)),
                        None => {
                            self.writer_errors_open = false;
                        }
                    }
                }
                stdout = self.stdout.next_line(), if self.stdout_open => {
                    match stdout {
                        Ok(Some(line)) => {
                            if let Some(event) = parse_external_line(self.provider, &line) {
                                return Ok(ExternalProcessEvent::Cli(event));
                            }
                        }
                        Ok(None) => {
                            self.stdout_open = false;
                        }
                        Err(err) => return Err(format!("failed to read external provider stdout: {err}")),
                    }
                }
                stderr = self.stderr.next_line(), if self.stderr_open => {
                    match stderr {
                        Ok(Some(line)) => {
                            let line = line.trim();
                            if !line.is_empty() {
                                return Ok(ExternalProcessEvent::Cli(ExternalCliEvent::Status(
                                    truncate_chars(line, MAX_EXTERNAL_ERROR_CHARS)
                                )));
                            }
                        }
                        Ok(None) => {
                            self.stderr_open = false;
                        }
                        Err(err) => return Err(format!("failed to read external provider stderr: {err}")),
                    }
                }
            }
        }
    }

    async fn next_process_event(&mut self) -> Result<ExternalProcessEvent, String> {
        let status = self.child.wait().await.map_err(|err| {
            format!(
                "{} external provider failed while waiting for process exit: {err}",
                provider_name(self.provider)
            )
        })?;
        Ok(ExternalProcessEvent::ProcessExited {
            success: status.success(),
            status: status.to_string(),
        })
    }
}

pub(crate) struct OpencodeHttpSession {
    child: Child,
    input_sink: ExternalInputSink,
    events: mpsc::UnboundedReceiver<Result<ExternalProcessEvent, String>>,
}

impl OpencodeHttpSession {
    async fn start(command: ExternalSessionSpec, cwd: PathBuf) -> Result<Self, String> {
        let mut child = Command::new(command.program)
            .args(command.args)
            .current_dir(&cwd)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| format!("opencode external provider unavailable: {err}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "opencode external provider did not expose stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "opencode external provider did not expose stderr".to_string())?;
        let (base_url, stdout, stderr) = wait_for_opencode_server_url(
            BufReader::new(stdout).lines(),
            BufReader::new(stderr).lines(),
        )
        .await?;
        tokio::spawn(drain_opencode_logs(stdout));
        tokio::spawn(drain_opencode_logs(stderr));

        let client = reqwest::Client::new();
        let session_id = create_opencode_session(&client, &base_url, &cwd).await?;
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        let (subscription_tx, mut subscription_rx) = watch::channel(None);
        tokio::spawn(write_opencode_provider_input(
            client.clone(),
            base_url.clone(),
            session_id.clone(),
            cwd.clone(),
            input_rx,
            event_tx.clone(),
            subscription_rx.clone(),
        ));
        tokio::spawn(read_opencode_events(
            client,
            base_url,
            session_id,
            cwd,
            event_tx,
            subscription_tx,
        ));
        wait_for_opencode_event_subscription(&mut subscription_rx).await?;
        Ok(Self {
            child,
            input_sink: ExternalInputSink::new(input_tx),
            events,
        })
    }

    async fn next_event(&mut self) -> Result<ExternalProcessEvent, String> {
        tokio::select! {
            event = self.events.recv() => {
                event.unwrap_or_else(|| Err("opencode external provider event stream closed".to_string()))
            }
            status = self.child.wait() => {
                let status = status.map_err(|err| {
                    format!("opencode external provider failed while waiting for process exit: {err}")
                })?;
                Ok(ExternalProcessEvent::ProcessExited {
                    success: status.success(),
                    status: status.to_string(),
                })
            }
        }
    }
}

async fn wait_for_opencode_server_url(
    mut stdout: tokio::io::Lines<BufReader<ChildStdout>>,
    mut stderr: tokio::io::Lines<BufReader<ChildStderr>>,
) -> Result<
    (
        String,
        tokio::io::Lines<BufReader<ChildStdout>>,
        tokio::io::Lines<BufReader<ChildStderr>>,
    ),
    String,
> {
    loop {
        tokio::select! {
            line = stdout.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(url) = opencode_server_url_from_line(&line) {
                            return Ok((url, stdout, stderr));
                        }
                    }
                    Ok(None) => return Err("opencode server exited before printing listen URL".to_string()),
                    Err(err) => return Err(format!("failed to read opencode stdout: {err}")),
                }
            }
            line = stderr.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(url) = opencode_server_url_from_line(&line) {
                            return Ok((url, stdout, stderr));
                        }
                    }
                    Ok(None) => return Err("opencode server exited before printing listen URL".to_string()),
                    Err(err) => return Err(format!("failed to read opencode stderr: {err}")),
                }
            }
        }
    }
}

async fn drain_opencode_logs<R>(mut lines: tokio::io::Lines<BufReader<R>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    while matches!(lines.next_line().await, Ok(Some(_))) {}
}

fn opencode_server_url_from_line(line: &str) -> Option<String> {
    let start = line.find("http://")?;
    Some(line[start..].trim().to_string())
}

async fn create_opencode_session(
    client: &reqwest::Client,
    base_url: &str,
    cwd: &Path,
) -> Result<String, String> {
    let response = client
        .post(format!("{base_url}/session"))
        .query(&[("directory", cwd.to_string_lossy().to_string())])
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|err| format!("failed to create opencode session: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to create opencode session: HTTP {}",
            response.status()
        ));
    }
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|err| format!("failed to decode opencode session response: {err}"))?;
    value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "opencode session response did not include id".to_string())
}

async fn write_opencode_provider_input(
    client: reqwest::Client,
    base_url: String,
    session_id: String,
    cwd: PathBuf,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    event_tx: mpsc::UnboundedSender<Result<ExternalProcessEvent, String>>,
    mut subscription_rx: watch::Receiver<Option<Result<(), String>>>,
) {
    if let Err(err) = wait_for_opencode_event_subscription(&mut subscription_rx).await {
        let _ = event_tx.send(Ok(ExternalProcessEvent::StdinError(err)));
        return;
    }
    while let Some(content) = input_rx.recv().await {
        let result = client
            .post(format!("{base_url}/session/{session_id}/prompt_async"))
            .query(&[("directory", cwd.to_string_lossy().to_string())])
            .json(&serde_json::json!({
                "parts": [{
                    "type": "text",
                    "text": content,
                }],
            }))
            .send()
            .await
            .map_err(|err| format!("failed to send opencode prompt: {err}"))
            .and_then(|response| {
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "failed to send opencode prompt: HTTP {}",
                        response.status()
                    ))
                }
            });
        if let Err(err) = result {
            let _ = event_tx.send(Ok(ExternalProcessEvent::StdinError(err)));
            return;
        }
    }
}

async fn read_opencode_events(
    client: reqwest::Client,
    base_url: String,
    session_id: String,
    cwd: PathBuf,
    event_tx: mpsc::UnboundedSender<Result<ExternalProcessEvent, String>>,
    subscription_tx: watch::Sender<Option<Result<(), String>>>,
) {
    let response = match client
        .get(format!("{base_url}/event"))
        .query(&[("directory", cwd.to_string_lossy().to_string())])
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            let message = format!("failed to subscribe to opencode events: {err}");
            let _ = subscription_tx.send(Some(Err(message.clone())));
            let _ = event_tx.send(Err(message));
            return;
        }
    };
    if !response.status().is_success() {
        let message = format!(
            "failed to subscribe to opencode events: HTTP {}",
            response.status()
        );
        let _ = subscription_tx.send(Some(Err(message.clone())));
        let _ = event_tx.send(Err(message));
        return;
    }
    let _ = subscription_tx.send(Some(Ok(())));
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut text_buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                let _ = event_tx.send(Err(format!("failed to read opencode events: {err}")));
                return;
            }
        };
        pending.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = pending.find('\n') {
            let line = pending[..newline].trim_end_matches('\r').to_string();
            pending.drain(..=newline);
            if let Some(event) = opencode_event_from_sse_line(&line, &session_id, &mut text_buffer)
            {
                let _ = event_tx.send(Ok(event));
            }
        }
    }
    let _ = event_tx.send(Err("opencode event stream closed".to_string()));
}

async fn wait_for_opencode_event_subscription(
    subscription_rx: &mut watch::Receiver<Option<Result<(), String>>>,
) -> Result<(), String> {
    loop {
        if let Some(result) = subscription_rx.borrow().clone() {
            return result;
        }
        subscription_rx.changed().await.map_err(|_| {
            "opencode event subscription closed before reporting readiness".to_string()
        })?;
    }
}

fn opencode_event_from_sse_line(
    line: &str,
    session_id: &str,
    text_buffer: &mut String,
) -> Option<ExternalProcessEvent> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    let properties = value.get("properties").unwrap_or(&serde_json::Value::Null);
    let event_session_id = properties
        .get("sessionID")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            properties
                .get("part")
                .and_then(|part| part.get("sessionID"))
                .and_then(serde_json::Value::as_str)
        });
    if event_session_id != Some(session_id) {
        return None;
    }
    let event_type = value.get("type").and_then(serde_json::Value::as_str)?;
    match event_type {
        "message.part.updated" => {
            let part = properties.get("part")?;
            if part.get("type").and_then(serde_json::Value::as_str) != Some("text") {
                return None;
            }
            let text = part
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let is_complete = part.get("time").and_then(|time| time.get("end")).is_some();
            if !is_complete {
                if let Some(delta) = properties.get("delta").and_then(serde_json::Value::as_str) {
                    text_buffer.push_str(delta);
                } else {
                    *text_buffer = text;
                }
                return None;
            }
            let text = if text.is_empty() {
                std::mem::take(text_buffer)
            } else {
                text
            };
            Some(ExternalProcessEvent::Cli(
                external_tool_call_from_text(&text).unwrap_or(ExternalCliEvent::Completion(text)),
            ))
        }
        "session.next.text.delta" => {
            if let Some(delta) = properties.get("delta").and_then(serde_json::Value::as_str) {
                text_buffer.push_str(delta);
            }
            None
        }
        "message.part.delta" => {
            if properties.get("field").and_then(serde_json::Value::as_str) == Some("text")
                && let Some(delta) = properties.get("delta").and_then(serde_json::Value::as_str)
            {
                text_buffer.push_str(delta);
            }
            None
        }
        "session.next.text.ended" => {
            let text = properties
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| std::mem::take(text_buffer));
            Some(ExternalProcessEvent::Cli(
                external_tool_call_from_text(&text).unwrap_or(ExternalCliEvent::Completion(text)),
            ))
        }
        "session.error" => Some(ExternalProcessEvent::Cli(ExternalCliEvent::Status(
            truncate_chars(data, MAX_EXTERNAL_ERROR_CHARS),
        ))),
        _ => None,
    }
}

async fn write_external_provider_input(
    transport: ExternalProviderSessionTransport,
    mut stdin: ChildStdin,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    writer_error_tx: mpsc::UnboundedSender<String>,
) {
    while let Some(content) = input_rx.recv().await {
        let line = provider_input_line(transport, &content);
        if let Err(err) = stdin.write_all(line.as_bytes()).await {
            let _ = writer_error_tx.send(format!("failed to write external provider stdin: {err}"));
            return;
        }
        if let Err(err) = stdin.write_all(b"\n").await {
            let _ = writer_error_tx.send(format!("failed to write external provider stdin: {err}"));
            return;
        }
        if let Err(err) = stdin.flush().await {
            let _ = writer_error_tx.send(format!("failed to flush external provider stdin: {err}"));
            return;
        }
    }
}

async fn write_codex_app_server_input(
    mut stdin: ChildStdin,
    thread_id: String,
    active_turn_id: Arc<Mutex<Option<String>>>,
    mut input_rx: mpsc::UnboundedReceiver<String>,
    writer_error_tx: mpsc::UnboundedSender<String>,
) {
    let mut request_id = 3_u64;
    while let Some(content) = input_rx.recv().await {
        let active_turn = active_turn_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let (method, params) = codex_app_server_input_request(&thread_id, active_turn, &content);
        let result = send_codex_jsonrpc_request(&mut stdin, request_id, method, params).await;
        request_id = request_id.saturating_add(1);
        if let Err(err) = result {
            let _ = writer_error_tx.send(err);
            return;
        }
    }
}

fn codex_app_server_input_request(
    thread_id: &str,
    active_turn: Option<String>,
    content: &str,
) -> (&'static str, serde_json::Value) {
    if let Some(expected_turn_id) = active_turn {
        (
            "turn/steer",
            serde_json::json!({
                "threadId": thread_id,
                "expectedTurnId": expected_turn_id,
                "input": [codex_text_input(content)],
            }),
        )
    } else {
        (
            "turn/start",
            serde_json::json!({
                "threadId": thread_id,
                "input": [codex_text_input(content)],
            }),
        )
    }
}

fn codex_text_input(content: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": content,
        "textElements": [],
    })
}

async fn send_codex_jsonrpc_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<(), String> {
    write_codex_jsonrpc_line(stdin, &codex_jsonrpc_request_value(id, method, params)).await
}

fn codex_jsonrpc_request_value(
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

async fn write_codex_jsonrpc_line(
    stdin: &mut ChildStdin,
    value: &serde_json::Value,
) -> Result<(), String> {
    let line = serde_json::to_string(value)
        .map_err(|err| format!("failed to encode codex_cli JSON-RPC request: {err}"))?;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|err| format!("failed to write codex_cli app-server stdin: {err}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|err| format!("failed to write codex_cli app-server stdin: {err}"))?;
    stdin
        .flush()
        .await
        .map_err(|err| format!("failed to flush codex_cli app-server stdin: {err}"))
}

async fn read_codex_jsonrpc_response(
    stdout: &mut tokio::io::Lines<BufReader<ChildStdout>>,
    stderr: &mut tokio::io::Lines<BufReader<ChildStderr>>,
    expected_id: u64,
) -> Result<serde_json::Value, String> {
    let mut stderr_preview = String::new();
    let mut stderr_open = true;
    loop {
        tokio::select! {
            stdout_line = stdout.next_line() => {
                let line = stdout_line
                    .map_err(|err| format!("failed to read codex_cli app-server stdout: {err}"))?
                    .ok_or_else(|| codex_closed_stdout_before_response_error(&stderr_preview))?;
                let value = serde_json::from_str::<serde_json::Value>(&line).map_err(|err| {
                    format!("failed to decode codex_cli app-server JSON-RPC line `{line}`: {err}")
                })?;
                if value.get("id").and_then(serde_json::Value::as_u64) != Some(expected_id) {
                    continue;
                }
                if let Some(error) = value.get("error") {
                    return Err(format!("codex_cli app-server request failed: {error}"));
                }
                return Ok(value);
            }
            stderr_line = stderr.next_line(), if stderr_open => {
                match stderr_line {
                    Ok(Some(line)) => append_codex_startup_stderr(&mut stderr_preview, &line),
                    Ok(None) => stderr_open = false,
                    Err(err) => {
                        append_codex_startup_stderr(
                            &mut stderr_preview,
                            &format!("failed to read codex_cli app-server stderr: {err}"),
                        );
                        stderr_open = false;
                    }
                }
            }
        }
    }
}

fn append_codex_startup_stderr(preview: &mut String, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    if !preview.is_empty() {
        preview.push('\n');
    }
    preview.push_str(line);
    *preview = truncate_chars(preview, MAX_EXTERNAL_ERROR_CHARS);
}

fn codex_closed_stdout_before_response_error(stderr_preview: &str) -> String {
    if stderr_preview.trim().is_empty() {
        return "codex_cli app-server closed stdout before response".to_string();
    }
    format!(
        "codex_cli app-server closed stdout before response; stderr: {}",
        truncate_chars(stderr_preview.trim(), MAX_EXTERNAL_ERROR_CHARS)
    )
}

pub(crate) fn provider_input_line(
    transport: ExternalProviderSessionTransport,
    content: &str,
) -> String {
    match transport {
        ExternalProviderSessionTransport::ClaudeStreamJson => serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            },
            "parent_tool_use_id": null,
        })
        .to_string(),
        ExternalProviderSessionTransport::CodexAppServerStdio => content.to_string(),
        ExternalProviderSessionTransport::OpencodeHttp => content.to_string(),
    }
}

pub(crate) fn external_tool_result_input(result: &ExternalToolResult) -> String {
    external_tool_result_json_line(result)
}

pub(crate) fn bounded_external_output(message: &str) -> String {
    truncate_chars(message.trim(), MAX_EXTERNAL_OUTPUT_CHARS)
}

pub(crate) fn external_metadata(run: &ExternalAgentRun) -> AgentMetadata {
    AgentMetadata {
        agent_id: Some(run.thread_id),
        agent_path: Some(run.agent_path.clone()),
        agent_nickname: Some(provider_name(run.provider).to_string()),
        agent_role: Some(provider_name(run.provider).to_string()),
        last_task_message: run.last_task_message.clone(),
        counted: true,
        ..Default::default()
    }
}

pub(crate) fn external_live_agent(run: &ExternalAgentRun) -> LiveAgent {
    LiveAgent {
        thread_id: run.thread_id,
        metadata: external_metadata(run),
        status: run.status.clone(),
    }
}

pub(crate) fn completion_communication(run: &ExternalAgentRun) -> Option<InterAgentCommunication> {
    let parent_agent_path = parent_path(&run.agent_path)?;
    let message = crate::session_prefix::format_subagent_notification_message(
        run.agent_path.as_str(),
        &run.status,
    );
    Some(
        InterAgentCommunication::new(
            run.agent_path.clone(),
            parent_agent_path,
            Vec::new(),
            message,
            InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(true)
        .with_thread_ids(run.thread_id, run.parent_thread_id)
        .with_status(run.status.clone())
        .with_agent_metadata(
            Some(provider_name(run.provider).to_string()),
            Some(provider_name(run.provider).to_string()),
        ),
    )
}

#[cfg(test)]
pub(crate) fn parse_external_stream(
    provider: SpawnAgentProvider,
    stream: &str,
) -> Vec<ExternalCliEvent> {
    stream
        .lines()
        .filter_map(|line| parse_external_line(provider, line))
        .collect()
}

fn parse_external_line(provider: SpawnAgentProvider, line: &str) -> Option<ExternalCliEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Some(ExternalCliEvent::Message(line.to_string()));
    };
    if let Some(tool_event) = parse_external_tool_call_event(&value) {
        return Some(tool_event);
    }
    match provider {
        SpawnAgentProvider::CodexCli => parse_codex_json_event(&value),
        SpawnAgentProvider::ClaudeCli => parse_claude_stream_json_event(&value),
        SpawnAgentProvider::Opencode => parse_opencode_json_event(&value),
        SpawnAgentProvider::Native => None,
    }
    .or_else(|| {
        Some(ExternalCliEvent::Status(truncate_chars(
            line,
            MAX_EXTERNAL_ERROR_CHARS,
        )))
    })
}

fn parse_external_tool_call_event(value: &serde_json::Value) -> Option<ExternalCliEvent> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("external_tool_call") {
        return None;
    }
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("invalid_external_tool_call")
        .to_string();
    match serde_json::from_value(value.clone()) {
        Ok(call) => Some(ExternalCliEvent::ToolCall(call)),
        Err(err) => Some(ExternalCliEvent::ToolCallError(ExternalToolResult::error(
            id,
            "invalid_tool_call",
            format!("failed to parse external tool call: {err}"),
        ))),
    }
}

fn external_tool_call_from_text(text: &str) -> Option<ExternalCliEvent> {
    text.lines().find_map(|line| {
        let value = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
        parse_external_tool_call_event(&value)
    })
}

fn parse_codex_app_server_jsonrpc_line(
    line: &str,
    active_turn_id: &Arc<Mutex<Option<String>>>,
    pending_completion_event: &Arc<Mutex<Option<ExternalCliEvent>>>,
) -> Option<ExternalProcessEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let value = match serde_json::from_str::<serde_json::Value>(line) {
        Ok(value) => value,
        Err(_) => {
            return Some(ExternalProcessEvent::Cli(ExternalCliEvent::Status(
                truncate_chars(line, MAX_EXTERNAL_ERROR_CHARS),
            )));
        }
    };
    if let Some(error) = value.get("error") {
        return Some(ExternalProcessEvent::StdinError(format!(
            "codex_cli app-server request failed: {error}"
        )));
    }
    let method = value.get("method").and_then(serde_json::Value::as_str)?;
    let params = value.get("params").unwrap_or(&serde_json::Value::Null);
    match method {
        "turn/started" => {
            if let Some(turn_id) = turn_id_from_params(params) {
                set_active_turn_id(active_turn_id, Some(turn_id));
            }
            set_pending_codex_completion_event(pending_completion_event, None);
            None
        }
        "turn/completed" => {
            set_active_turn_id(active_turn_id, None);
            let event = last_agent_message_text(params)
                .map(codex_completion_event_from_text)
                .or_else(|| take_pending_codex_completion_event(pending_completion_event))?;
            Some(ExternalProcessEvent::Cli(event))
        }
        "item/completed" => {
            if let Some(text) = item_agent_message_text(params) {
                set_pending_codex_completion_event(
                    pending_completion_event,
                    Some(codex_completion_event_from_text(text)),
                );
            }
            None
        }
        "item/agentMessage/delta" => None,
        "warning" | "guardianWarning" | "configWarning" => {
            text_field(params).map(|text| ExternalProcessEvent::Cli(ExternalCliEvent::Status(text)))
        }
        _ => None,
    }
}

fn set_active_turn_id(active_turn_id: &Arc<Mutex<Option<String>>>, value: Option<String>) {
    *active_turn_id
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

fn set_pending_codex_completion_event(
    pending_completion_event: &Arc<Mutex<Option<ExternalCliEvent>>>,
    value: Option<ExternalCliEvent>,
) {
    *pending_completion_event
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

fn take_pending_codex_completion_event(
    pending_completion_event: &Arc<Mutex<Option<ExternalCliEvent>>>,
) -> Option<ExternalCliEvent> {
    pending_completion_event
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

fn turn_id_from_params(params: &serde_json::Value) -> Option<String> {
    params
        .get("turn")
        .and_then(|turn| turn.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            params
                .get("turnId")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn item_agent_message_text(params: &serde_json::Value) -> Option<String> {
    let item = params.get("item")?;
    if item.get("type").and_then(serde_json::Value::as_str) != Some("agentMessage") {
        return None;
    }
    item.get("text")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn last_agent_message_text(params: &serde_json::Value) -> Option<String> {
    let items = params
        .get("turn")
        .and_then(|turn| turn.get("items"))
        .and_then(serde_json::Value::as_array)?;
    items.iter().rev().find_map(|item| {
        if item.get("type").and_then(serde_json::Value::as_str) != Some("agentMessage") {
            return None;
        }
        item.get("text")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

fn codex_completion_event_from_text(text: String) -> ExternalCliEvent {
    external_tool_call_from_text(&text).unwrap_or(ExternalCliEvent::Completion(text))
}

fn parse_codex_json_event(value: &serde_json::Value) -> Option<ExternalCliEvent> {
    let event_type = value.get("type").and_then(serde_json::Value::as_str)?;
    match event_type {
        "assistant_message" | "agent_message" | "message" => text_field(value).map(|text| {
            external_tool_call_from_text(&text).unwrap_or(ExternalCliEvent::Message(text))
        }),
        "task_complete" | "turn_complete" | "completed" => text_field(value)
            .or_else(|| nested_text(value, &["result", "message"]))
            .map(ExternalCliEvent::Completion),
        "error" | "failed" => text_field(value).map(ExternalCliEvent::Status),
        other => Some(ExternalCliEvent::Status(other.to_string())),
    }
}

fn parse_claude_stream_json_event(value: &serde_json::Value) -> Option<ExternalCliEvent> {
    let event_type = value.get("type").and_then(serde_json::Value::as_str)?;
    match event_type {
        "assistant" => claude_message_text(value).map(|text| {
            external_tool_call_from_text(&text).unwrap_or(ExternalCliEvent::Message(text))
        }),
        "result" => text_field(value)
            .or_else(|| {
                value
                    .get("result")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .map(ExternalCliEvent::Completion),
        "system" => text_field(value).map(ExternalCliEvent::Status),
        "error" => text_field(value).map(ExternalCliEvent::Status),
        other => Some(ExternalCliEvent::Status(other.to_string())),
    }
}

fn parse_opencode_json_event(value: &serde_json::Value) -> Option<ExternalCliEvent> {
    let event_type = value
        .get("type")
        .or_else(|| value.get("event"))
        .and_then(serde_json::Value::as_str)?;
    match event_type {
        "message" | "assistant" | "part" => text_field(value).map(ExternalCliEvent::Message),
        "complete" | "completed" | "result" => text_field(value).map(ExternalCliEvent::Completion),
        "error" | "status" => text_field(value).map(ExternalCliEvent::Status),
        other => Some(ExternalCliEvent::Status(other.to_string())),
    }
}

pub(crate) fn external_tool_result_json_line(result: &ExternalToolResult) -> String {
    let serialized = serde_json::to_string(result);
    let Ok(line) = serialized else {
        return fallback_external_tool_result_line(
            "serialization_error",
            "serialization_error",
            "failed to serialize external tool result",
        );
    };
    if line.chars().count() <= MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS {
        return line;
    }

    let id = truncate_chars(&result.id, 256);
    if result.ok {
        let bounded = ExternalToolResult::ok(
            id,
            serde_json::json!({
                "truncated": true,
                "message": "external tool result exceeded transcript budget",
                "preview": truncate_chars(&line, MAX_EXTERNAL_ERROR_CHARS),
            }),
        );
        serde_json::to_string(&bounded).unwrap_or_else(|_| {
            fallback_external_tool_result_line(
                "serialization_error",
                "serialization_error",
                "failed to serialize bounded external tool result",
            )
        })
    } else {
        let error = result.error.as_ref();
        let bounded = ExternalToolResult::error(
            id,
            error
                .map(|error| truncate_chars(&error.code, 128))
                .unwrap_or_else(|| "tool_error".to_string()),
            error
                .map(|error| truncate_chars(&error.message, MAX_EXTERNAL_ERROR_CHARS))
                .unwrap_or_else(|| "external tool failed".to_string()),
        );
        serde_json::to_string(&bounded).unwrap_or_else(|_| {
            fallback_external_tool_result_line(
                "serialization_error",
                "serialization_error",
                "failed to serialize bounded external tool error",
            )
        })
    }
}

fn fallback_external_tool_result_line(id: &str, code: &str, message: &str) -> String {
    serde_json::json!({
        "type": "external_tool_result",
        "id": id,
        "ok": false,
        "error": {
            "code": code,
            "message": message,
        },
    })
    .to_string()
}

fn text_field(value: &serde_json::Value) -> Option<String> {
    ["message", "text", "content", "summary", "error"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
}

fn nested_text(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str().map(ToOwned::to_owned)
}

fn claude_message_text(value: &serde_json::Value) -> Option<String> {
    let message = value.get("message")?;
    let content = message.get("content")?.as_array()?;
    let mut parts = Vec::new();
    for part in content {
        if part.get("type").and_then(serde_json::Value::as_str) == Some("text")
            && let Some(text) = part.get("text").and_then(serde_json::Value::as_str)
        {
            parts.push(text);
        }
    }
    (!parts.is_empty()).then(|| parts.join(""))
}

fn parent_path(agent_path: &AgentPath) -> Option<AgentPath> {
    agent_path
        .as_str()
        .rsplit_once('/')
        .and_then(|(parent, _)| AgentPath::try_from(parent).ok())
}

fn provider_name(provider: SpawnAgentProvider) -> &'static str {
    match provider {
        SpawnAgentProvider::Native => "native",
        SpawnAgentProvider::CodexCli => "codex_cli",
        SpawnAgentProvider::ClaudeCli => "claude_cli",
        SpawnAgentProvider::Opencode => "opencode",
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("...");
    output
}

pub(crate) fn provider_is_external(provider: Option<SpawnAgentProvider>) -> bool {
    !matches!(
        provider.unwrap_or(SpawnAgentProvider::Native),
        SpawnAgentProvider::Native
    )
}

pub(crate) type SharedExternalAgentRegistry = Arc<ExternalAgentRegistry>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio::time::sleep;
    use tokio::time::timeout;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    #[test]
    fn parses_codex_jsonl_completion_and_unknown_status() {
        let events = parse_external_stream(
            SpawnAgentProvider::CodexCli,
            "{\"type\":\"agent_message\",\"message\":\"hello\"}\n{\"type\":\"mystery\",\"value\":1}\n{\"type\":\"completed\",\"message\":\"done\"}",
        );
        assert_eq!(
            events,
            vec![
                ExternalCliEvent::Message("hello".to_string()),
                ExternalCliEvent::Status("mystery".to_string()),
                ExternalCliEvent::Completion("done".to_string()),
            ]
        );
    }

    #[test]
    fn parses_claude_stream_json_result() {
        let events = parse_external_stream(
            SpawnAgentProvider::ClaudeCli,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n{\"type\":\"result\",\"result\":\"done\"}",
        );
        assert_eq!(
            events,
            vec![
                ExternalCliEvent::Message("hi".to_string()),
                ExternalCliEvent::Completion("done".to_string()),
            ]
        );
    }

    #[test]
    fn parses_opencode_skeleton_ndjson() {
        let events = parse_external_stream(
            SpawnAgentProvider::Opencode,
            "{\"event\":\"message\",\"message\":\"working\"}\n{\"event\":\"complete\",\"message\":\"ok\"}",
        );
        assert_eq!(
            events,
            vec![
                ExternalCliEvent::Message("working".to_string()),
                ExternalCliEvent::Completion("ok".to_string()),
            ]
        );
    }

    #[test]
    fn parses_external_tool_call_before_provider_events() {
        let events = parse_external_stream(
            SpawnAgentProvider::CodexCli,
            "{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"list_external_agents\",\"arguments\":{\"path_prefix\":\"reviewer\"}}\n{\"type\":\"completed\",\"message\":\"done\"}",
        );
        assert_eq!(
            events,
            vec![
                ExternalCliEvent::ToolCall(ExternalToolCall {
                    id: "call_1".to_string(),
                    tool: ExternalToolName::ListExternalAgents,
                    arguments: json!({ "path_prefix": "reviewer" }),
                }),
                ExternalCliEvent::Completion("done".to_string()),
            ]
        );
    }

    #[test]
    fn parses_codex_wrapped_external_tool_call() {
        let events = parse_external_stream(
            SpawnAgentProvider::CodexCli,
            "{\"type\":\"agent_message\",\"message\":\"{\\\"type\\\":\\\"external_tool_call\\\",\\\"id\\\":\\\"call_1\\\",\\\"tool\\\":\\\"list_external_agents\\\",\\\"arguments\\\":{}}\"}",
        );
        assert_eq!(
            events,
            vec![ExternalCliEvent::ToolCall(ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: json!({}),
            })]
        );
    }

    #[test]
    fn parses_claude_wrapped_external_tool_call() {
        let events = parse_external_stream(
            SpawnAgentProvider::ClaudeCli,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"{\\\"type\\\":\\\"external_tool_call\\\",\\\"id\\\":\\\"call_1\\\",\\\"tool\\\":\\\"followup_external_task\\\",\\\"arguments\\\":{\\\"target\\\":\\\"/root/native\\\",\\\"message\\\":\\\"hi\\\"}}\"}]}}",
        );
        assert_eq!(
            events,
            vec![ExternalCliEvent::ToolCall(ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: json!({
                    "target": "/root/native",
                    "message": "hi",
                }),
            })]
        );
    }

    #[test]
    fn unknown_external_tool_call_becomes_bounded_tool_result() {
        let events = parse_external_stream(
            SpawnAgentProvider::CodexCli,
            "{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"unknown_external_tool\",\"arguments\":{}}",
        );
        let [ExternalCliEvent::ToolCallError(result)] = events.as_slice() else {
            panic!("expected tool call error");
        };
        assert_eq!(result.id, "call_1");
        assert!(!result.ok);
        let error = result.error.as_ref().expect("error");
        assert_eq!(error.code, "invalid_tool_call");
        assert!(error.message.contains("failed to parse external tool call"));
        assert!(error.message.len() <= MAX_EXTERNAL_ERROR_CHARS + 3);
    }

    #[test]
    fn external_context_injects_schema_and_forbids_internal_tool_names() {
        let context = external_agent_context_prompt("review this patch");
        assert!(context.contains("spawn_external_agent"));
        assert!(context.contains("external_tool_call"));
        assert!(context.contains("external_tool_result"));
        assert!(context.contains("Do not call internal Morpheus tools"));
        assert!(context.contains("review this patch"));
    }

    #[test]
    fn external_tool_result_line_is_bounded_but_keeps_json_envelope() {
        let result = ExternalToolResult::ok(
            "call_1",
            json!({ "payload": "x".repeat(MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS + 100) }),
        );
        let line = external_tool_result_json_line(&result);
        let value: serde_json::Value = serde_json::from_str(&line).expect("valid json");

        assert!(line.len() <= MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS + 512);
        assert_eq!(value["type"], "external_tool_result");
        assert_eq!(value["id"], "call_1");
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"]["truncated"], true);
    }

    #[test]
    fn external_tool_error_result_is_bounded_json() {
        let result = ExternalToolResult::error("call_1", "invalid_arguments", "x".repeat(5000));
        let value = serde_json::to_value(result).expect("serialize result");
        assert_eq!(value["type"], "external_tool_result");
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "invalid_arguments");
        assert!(
            value["error"]["message"].as_str().expect("message").len()
                <= MAX_EXTERNAL_ERROR_CHARS + 3
        );
    }

    #[test]
    fn shutdown_prevents_late_terminal_status_override() {
        let registry = ExternalAgentRegistry::default();
        let thread_id = ThreadId::new();
        registry.insert_running(ExternalAgentRun {
            thread_id,
            parent_thread_id: ThreadId::new(),
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: None,
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });

        registry.shutdown(thread_id).expect("shutdown external run");
        let late = registry.set_terminal_status_if_active(
            thread_id,
            AgentStatus::Completed(Some("late".to_string())),
        );

        assert!(late.is_none());
        assert_eq!(
            registry.get(thread_id).expect("external run").status,
            AgentStatus::Shutdown
        );
    }

    #[test]
    fn builds_codex_app_server_session_command() {
        let spec = external_session_spec(SpawnAgentProvider::CodexCli, Path::new("/tmp/work"))
            .expect("codex app-server command");
        assert_eq!(spec.program, "codex");
        assert_eq!(
            spec.transport,
            ExternalProviderSessionTransport::CodexAppServerStdio
        );
        assert_eq!(
            spec.args,
            vec![
                "app-server".to_string(),
                "--listen".to_string(),
                "stdio://".to_string()
            ]
        );
    }

    #[test]
    fn codex_app_server_command_clears_morpheus_runtime_environment() {
        let spec = external_session_spec(SpawnAgentProvider::CodexCli, Path::new("/tmp/work"))
            .expect("codex app-server command");
        let command = codex_app_server_command(spec, Path::new("/tmp/work"));
        let envs = command
            .as_std()
            .get_envs()
            .map(|(name, value)| (name.to_string_lossy().to_string(), value.is_some()))
            .collect::<Vec<_>>();

        assert!(envs.contains(&("CODEX_HOME".to_string(), false)));
        assert!(envs.contains(&("CODEX_THREAD_ID".to_string(), false)));
    }

    #[test]
    fn codex_jsonrpc_request_includes_standard_version_field() {
        let value = codex_jsonrpc_request_value(3, "turn/start", json!({ "threadId": "thr_1" }));

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 3);
        assert_eq!(value["method"], "turn/start");
        assert_eq!(value["params"]["threadId"], "thr_1");
    }

    #[test]
    fn codex_closed_stdout_error_includes_startup_stderr() {
        let error = codex_closed_stdout_before_response_error(
            "Error: failed to initialize sqlite state runtime",
        );

        assert!(error.contains("closed stdout before response"));
        assert!(error.contains("failed to initialize sqlite state runtime"));
    }

    #[test]
    fn builds_claude_stream_json_command() {
        let spec = external_session_spec(SpawnAgentProvider::ClaudeCli, Path::new("/tmp/work"))
            .expect("claude command");
        assert_eq!(spec.program, "claude");
        assert_eq!(
            spec.transport,
            ExternalProviderSessionTransport::ClaudeStreamJson
        );
        assert!(spec.args.contains(&"-p".to_string()));
        assert!(spec.args.contains(&"--output-format".to_string()));
        assert!(spec.args.contains(&"stream-json".to_string()));
        assert!(spec.args.contains(&"--input-format".to_string()));
        assert!(!spec.args.contains(&"do it".to_string()));
    }

    #[test]
    fn builds_claude_stream_json_input_line() {
        let line = provider_input_line(ExternalProviderSessionTransport::ClaudeStreamJson, "hello");
        let value: serde_json::Value = serde_json::from_str(&line).expect("json line");
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "hello");
    }

    #[test]
    fn parses_codex_app_server_turn_started_as_state_only() {
        let active_turn_id = Arc::new(Mutex::new(None));
        let pending_completion_event = Arc::new(Mutex::new(Some(ExternalCliEvent::Completion(
            "stale".to_string(),
        ))));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/started","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"status":"inProgress","error":null,"startedAt":1,"completedAt":null,"durationMs":null}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert!(event.is_none());
        assert_eq!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref(),
            Some("turn_1")
        );
        assert!(
            pending_completion_event
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn ignores_codex_app_server_item_completed_external_tool_call_until_turn_completion() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"agentMessage","id":"item_1","text":"{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"list_external_agents\",\"arguments\":{}}"}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert!(event.is_none());
        assert_eq!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref(),
            Some("turn_1")
        );
        assert!(matches!(
            pending_completion_event
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref(),
            Some(ExternalCliEvent::ToolCall(_))
        ));
    }

    #[test]
    fn parses_codex_app_server_turn_completed_external_tool_call_and_clears_state() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let item_event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"agentMessage","id":"item_1","text":"{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"list_external_agents\",\"arguments\":{}}"}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );
        let completed_event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert!(item_event.is_none());
        assert_eq!(
            completed_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::ToolCall(
                ExternalToolCall {
                    id: "call_1".to_string(),
                    tool: ExternalToolName::ListExternalAgents,
                    arguments: json!({}),
                }
            )))
        );
        assert!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
        assert!(
            pending_completion_event
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn ignores_codex_app_server_plain_item_completed_until_turn_completion() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"agentMessage","id":"item_1","text":"intermediate text"}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert!(event.is_none());
        assert_eq!(
            pending_completion_event
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref(),
            Some(&ExternalCliEvent::Completion(
                "intermediate text".to_string()
            ))
        );
    }

    #[test]
    fn parses_codex_app_server_turn_completed_as_completion_and_clears_state() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(Some(ExternalCliEvent::Completion(
            "done".to_string(),
        ))));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Completion(
                "done".to_string()
            )))
        );
        assert!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn parses_codex_app_server_response_error_as_stdin_error() {
        let active_turn_id = Arc::new(Mutex::new(None));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"id":3,"error":{"code":-32602,"message":"bad expectedTurnId"}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::StdinError(
                r#"codex_cli app-server request failed: {"code":-32602,"message":"bad expectedTurnId"}"#.to_string()
            ))
        );
    }

    #[test]
    fn codex_app_server_tool_result_starts_next_turn_after_completion() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let (active_method, active_params) = codex_app_server_input_request(
            "thr_1",
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            "while active",
        );
        assert_eq!(active_method, "turn/steer");
        assert_eq!(active_params["expectedTurnId"], "turn_1");

        assert!(
            parse_codex_app_server_jsonrpc_line(
                r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","item":{"type":"agentMessage","id":"item_1","text":"{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"list_external_agents\",\"arguments\":{}}"}}}"#,
                &active_turn_id,
                &pending_completion_event,
            )
            .is_none()
        );
        let completed_event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );
        assert!(matches!(
            completed_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::ToolCall(_)))
        ));

        let result = ExternalToolResult::ok("call_1", json!({ "agents": [] }));
        let (result_method, result_params) = codex_app_server_input_request(
            "thr_1",
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
            &external_tool_result_input(&result),
        );
        assert_eq!(result_method, "turn/start");
        assert!(result_params.get("expectedTurnId").is_none());
        assert_eq!(
            result_params["input"][0]["text"],
            external_tool_result_input(&result)
        );
    }

    #[test]
    fn builds_opencode_http_session_command() {
        let spec = external_session_spec(SpawnAgentProvider::Opencode, Path::new("/tmp/work"))
            .expect("opencode command");
        assert_eq!(spec.program, "opencode");
        assert_eq!(
            spec.transport,
            ExternalProviderSessionTransport::OpencodeHttp
        );
        assert!(spec.args.contains(&"serve".to_string()));
        assert!(spec.args.contains(&"--port".to_string()));
        assert!(spec.args.contains(&"0".to_string()));
    }

    #[tokio::test]
    async fn opencode_writer_waits_for_event_subscription_before_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/ses_1/prompt_async"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (subscription_tx, subscription_rx) = watch::channel(None);
        let writer = tokio::spawn(write_opencode_provider_input(
            reqwest::Client::new(),
            server.uri(),
            "ses_1".to_string(),
            PathBuf::from("/tmp/work"),
            input_rx,
            event_tx,
            subscription_rx,
        ));

        input_tx.send("hello".to_string()).expect("queue prompt");
        sleep(Duration::from_millis(50)).await;
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );

        subscription_tx
            .send(Some(Ok(())))
            .expect("publish event subscription readiness");
        timeout(Duration::from_secs(1), async {
            loop {
                let requests = server.received_requests().await.expect("requests");
                if requests.len() == 1 {
                    return;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("prompt sent after event subscription readiness");

        drop(input_tx);
        writer.await.expect("writer task");
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn opencode_writer_reports_event_subscription_failure_without_prompt() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/ses_1/prompt_async"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let (subscription_tx, subscription_rx) = watch::channel(None);
        let writer = tokio::spawn(write_opencode_provider_input(
            reqwest::Client::new(),
            server.uri(),
            "ses_1".to_string(),
            PathBuf::from("/tmp/work"),
            input_rx,
            event_tx,
            subscription_rx,
        ));

        input_tx.send("hello".to_string()).expect("queue prompt");
        subscription_tx
            .send(Some(Err(
                "failed to subscribe to opencode events: HTTP 500".to_string(),
            )))
            .expect("publish event subscription failure");
        writer.await.expect("writer task");

        assert_eq!(
            event_rx.recv().await,
            Some(Ok(ExternalProcessEvent::StdinError(
                "failed to subscribe to opencode events: HTTP 500".to_string()
            )))
        );
        assert!(
            server
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    }

    #[test]
    fn parses_opencode_message_part_updated_text_end_as_completion() {
        let mut buffer = String::new();
        let event = opencode_event_from_sse_line(
            r#"data: {"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_1","messageID":"msg_1","type":"text","text":"done","time":{"start":1,"end":2}}}}"#,
            "ses_1",
            &mut buffer,
        )
        .expect("event");
        assert_eq!(
            event,
            ExternalProcessEvent::Cli(ExternalCliEvent::Completion("done".to_string()))
        );
    }

    #[test]
    fn parses_opencode_message_part_updated_external_tool_call() {
        let mut buffer = String::new();
        let event = opencode_event_from_sse_line(
            r#"data: {"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_1","messageID":"msg_1","type":"text","text":"{\"type\":\"external_tool_call\",\"id\":\"call_1\",\"tool\":\"list_external_agents\",\"arguments\":{}}","time":{"start":1,"end":2}}}}"#,
            "ses_1",
            &mut buffer,
        )
        .expect("event");
        assert_eq!(
            event,
            ExternalProcessEvent::Cli(ExternalCliEvent::ToolCall(ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: json!({}),
            }))
        );
    }

    #[test]
    fn ignores_opencode_events_for_other_sessions() {
        let mut buffer = String::new();
        let event = opencode_event_from_sse_line(
            r#"data: {"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_other","messageID":"msg_1","type":"text","text":"done","time":{"start":1,"end":2}}}}"#,
            "ses_1",
            &mut buffer,
        );
        assert!(event.is_none());
    }

    #[test]
    fn accumulates_opencode_message_part_updated_text_until_end() {
        let mut buffer = String::new();
        assert!(
            opencode_event_from_sse_line(
                r#"data: {"type":"message.part.updated","properties":{"delta":"hel","part":{"id":"prt_1","sessionID":"ses_1","messageID":"msg_1","type":"text","text":"hel","time":{"start":1}}}}"#,
                "ses_1",
                &mut buffer,
            )
            .is_none()
        );
        let event = opencode_event_from_sse_line(
            r#"data: {"type":"message.part.updated","properties":{"part":{"id":"prt_1","sessionID":"ses_1","messageID":"msg_1","type":"text","text":"","time":{"start":1,"end":2}}}}"#,
            "ses_1",
            &mut buffer,
        )
        .expect("event");
        assert_eq!(
            event,
            ExternalProcessEvent::Cli(ExternalCliEvent::Completion("hel".to_string()))
        );
    }
}
