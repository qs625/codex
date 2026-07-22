use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::LiveAgent;
use codex_agent_runtime::SpawnAgentProvider;
use config_service::Config;
use futures::future::BoxFuture;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::protocol::AgentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::process::Child;
use tokio::process::ChildStderr;
use tokio::process::ChildStdin;
use tokio::process::ChildStdout;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

const MAX_EXTERNAL_OUTPUT_CHARS: usize = 12_000;
const MAX_EXTERNAL_ERROR_CHARS: usize = 4_000;
const MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS: usize = 8_000;

#[derive(Clone, Debug)]
pub(crate) struct ExternalAgentRun {
    pub(crate) thread_id: ThreadId,
    pub(crate) parent_thread_id: ThreadId,
    pub(crate) agent_path: AgentPath,
    pub(crate) provider: SpawnAgentProvider,
    pub(crate) depth: i32,
    pub(crate) spawn_config: Option<Config>,
    pub(crate) input_sink: Option<ExternalInputSink>,
    pub(crate) status: AgentStatus,
    pub(crate) last_task_message: Option<String>,
    pub(crate) abort_handle: Option<AbortHandle>,
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
pub(crate) struct ExternalCommandSpec {
    pub(crate) program: &'static str,
    pub(crate) args: Vec<String>,
    pub(crate) transport: ExternalProviderTransport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExternalProviderTransport {
    ClaudeStreamJson,
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

pub(crate) trait ExternalAgentStream: Send {
    fn input_sink(&self) -> ExternalInputSink;
    fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>>;
}

pub(crate) fn external_command_spec(
    provider: SpawnAgentProvider,
    _cwd: &Path,
) -> Result<ExternalCommandSpec, String> {
    match provider {
        SpawnAgentProvider::Native => Err("native is not an external CLI provider".to_string()),
        SpawnAgentProvider::CodexCli => Err(
            "codex_cli does not expose a persistent stdin/stdout continuation protocol; codex exec is non-interactive and cannot receive external_tool_result in the same running process"
                .to_string(),
        ),
        SpawnAgentProvider::ClaudeCli => Ok(ExternalCommandSpec {
            program: "claude",
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--input-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
            ],
            transport: ExternalProviderTransport::ClaudeStreamJson,
        }),
        SpawnAgentProvider::Opencode => Err(
            "opencode exposes ACP/server transports, not the external JSON stdin/stdout tool-result protocol implemented here; opencode support requires a dedicated ACP adapter"
                .to_string(),
        ),
    }
}

pub(crate) fn external_agent_context_prompt(message: &str) -> String {
    format!(
        r#"You are running as an external code agent connected to the my-codex backend bus.

Use only this external-agent JSON protocol to collaborate with other agents. Do not call internal my-codex tools such as spawn_agent, followup_task, list_agents, poll_event, or close_agent.

Available external tools:
- spawn_external_agent: arguments {{ "task_name": string, "provider": "claude_cli", "cwd": string, "message": string }}. Current persistent streaming support is limited to claude_cli; codex_cli and opencode return unsupported capability errors.
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

pub(crate) struct ExternalStreamingSession {
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

impl ExternalAgentStream for ExternalStreamingSession {
    fn input_sink(&self) -> ExternalInputSink {
        self.input_sink.clone()
    }

    fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>> {
        Box::pin(async move { self.next_event().await })
    }
}

impl ExternalStreamingSession {
    pub(crate) async fn start(
        provider: SpawnAgentProvider,
        cwd: PathBuf,
    ) -> Result<Self, String> {
        let command = external_command_spec(provider, cwd.as_path())?;
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
        Ok(Self {
            provider,
            child,
            stdout: BufReader::new(stdout).lines(),
            stderr: BufReader::new(stderr).lines(),
            stdout_open: true,
            stderr_open: true,
            input_sink: ExternalInputSink::new(input_tx),
            writer_errors,
            writer_errors_open: true,
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

async fn write_external_provider_input(
    transport: ExternalProviderTransport,
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

pub(crate) fn provider_input_line(
    transport: ExternalProviderTransport,
    content: &str,
) -> String {
    match transport {
        ExternalProviderTransport::ClaudeStreamJson => serde_json::json!({
            "type": "user",
            "message": {
                "role": "user",
                "content": content,
            },
            "parent_tool_use_id": null,
        })
        .to_string(),
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
        assert!(
            error
                .message
                .contains("failed to parse external tool call")
        );
        assert!(error.message.len() <= MAX_EXTERNAL_ERROR_CHARS + 3);
    }

    #[test]
    fn external_context_injects_schema_and_forbids_internal_tool_names() {
        let context = external_agent_context_prompt("review this patch");
        assert!(context.contains("spawn_external_agent"));
        assert!(context.contains("external_tool_call"));
        assert!(context.contains("external_tool_result"));
        assert!(context.contains("Do not call internal my-codex tools"));
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
    fn codex_cli_is_unsupported_for_persistent_streaming() {
        let err = external_command_spec(SpawnAgentProvider::CodexCli, Path::new("/tmp/work"))
            .expect_err("codex persistent streaming is unsupported");
        assert!(err.contains("persistent stdin/stdout continuation protocol"));
    }

    #[test]
    fn builds_claude_stream_json_command() {
        let spec = external_command_spec(SpawnAgentProvider::ClaudeCli, Path::new("/tmp/work"))
            .expect("claude command");
        assert_eq!(spec.program, "claude");
        assert_eq!(spec.transport, ExternalProviderTransport::ClaudeStreamJson);
        assert!(spec.args.contains(&"-p".to_string()));
        assert!(spec.args.contains(&"--output-format".to_string()));
        assert!(spec.args.contains(&"stream-json".to_string()));
        assert!(spec.args.contains(&"--input-format".to_string()));
        assert!(!spec.args.contains(&"do it".to_string()));
    }

    #[test]
    fn builds_claude_stream_json_input_line() {
        let line = provider_input_line(ExternalProviderTransport::ClaudeStreamJson, "hello");
        let value: serde_json::Value = serde_json::from_str(&line).expect("json line");
        assert_eq!(value["type"], "user");
        assert_eq!(value["message"]["role"], "user");
        assert_eq!(value["message"]["content"], "hello");
    }

    #[test]
    fn opencode_requires_dedicated_acp_adapter() {
        let err = external_command_spec(SpawnAgentProvider::Opencode, Path::new("/tmp/work"))
            .expect_err("opencode external json transport is unsupported");
        assert!(err.contains("dedicated ACP adapter"));
    }
}
