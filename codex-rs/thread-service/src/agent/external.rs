use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::LiveAgent;
use codex_agent_runtime::SpawnAgentProvider;
use config_service::Config;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::protocol::AgentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::process::Command;
use tokio::task::AbortHandle;

const MAX_EXTERNAL_OUTPUT_CHARS: usize = 12_000;
const MAX_EXTERNAL_ERROR_CHARS: usize = 4_000;
const MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS: usize = 8_000;
pub(crate) const MAX_EXTERNAL_TRANSCRIPT_CHARS: usize = 24_000;
pub(crate) const MAX_EXTERNAL_TOOL_ITERATIONS: usize = 8;

#[derive(Clone, Debug)]
pub(crate) struct ExternalAgentRun {
    pub(crate) thread_id: ThreadId,
    pub(crate) parent_thread_id: ThreadId,
    pub(crate) agent_path: AgentPath,
    pub(crate) provider: SpawnAgentProvider,
    pub(crate) depth: i32,
    pub(crate) spawn_config: Option<Config>,
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
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExternalCliTurn {
    pub(crate) status: AgentStatus,
    pub(crate) tool_calls: Vec<ExternalToolCall>,
    pub(crate) tool_call_errors: Vec<ExternalToolResult>,
    pub(crate) transcript_lines: Vec<String>,
}

pub(crate) fn external_command_spec(
    provider: SpawnAgentProvider,
    cwd: &Path,
    message: &str,
) -> Result<ExternalCommandSpec, String> {
    match provider {
        SpawnAgentProvider::Native => Err("native is not an external CLI provider".to_string()),
        SpawnAgentProvider::CodexCli => Ok(ExternalCommandSpec {
            program: "codex",
            args: vec![
                "exec".to_string(),
                "--json".to_string(),
                "-C".to_string(),
                cwd.to_string_lossy().into_owned(),
                message.to_string(),
            ],
        }),
        SpawnAgentProvider::ClaudeCli => Ok(ExternalCommandSpec {
            program: "claude",
            args: vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
                "--input-format".to_string(),
                "stream-json".to_string(),
                "--verbose".to_string(),
                message.to_string(),
            ],
        }),
        SpawnAgentProvider::Opencode => Err(
            "opencode external provider is unavailable in this first-stage runtime; install opencode and use a later provider mode"
                .to_string(),
        ),
    }
}

pub(crate) fn external_agent_context_prompt(message: &str, transcript: &str) -> String {
    let transcript_section = if transcript.trim().is_empty() {
        "No prior external tool transcript.".to_string()
    } else {
        format!(
            "External tool transcript so far, newest entries included as JSON lines:\n{}",
            truncate_chars_from_start(transcript.trim(), MAX_EXTERNAL_TRANSCRIPT_CHARS)
        )
    };
    format!(
        r#"You are running as an external code agent connected to the my-codex backend bus.

Use only this external-agent JSON protocol to collaborate with other agents. Do not call internal my-codex tools such as spawn_agent, followup_task, list_agents, poll_event, or close_agent.

Available external tools:
- spawn_external_agent: arguments {{ "task_name": string, "provider": "codex_cli" | "claude_cli" | "opencode", "cwd": string, "message": string }}
- followup_external_task: arguments {{ "target": string, "message": string }}
- list_external_agents: arguments {{ "path_prefix"?: string }}
- poll_external_event: currently unsupported for non-interactive CLI sessions; calling it returns an unsupported error.
- close_external_agent: arguments {{ "target": string }}

Emit one JSON object per line for tool calls:
{{"type":"external_tool_call","id":"call_1","tool":"list_external_agents","arguments":{{}}}}

The backend returns results as JSON objects:
{{"type":"external_tool_result","id":"call_1","ok":true,"result":{{}}}}
{{"type":"external_tool_result","id":"call_1","ok":false,"error":{{"code":"invalid_arguments","message":"..."}}}}

When you receive an external_tool_result in the transcript, continue the task using that result. Emit another external_tool_call only if you need another backend action; otherwise finish with a normal final answer.

{transcript_section}

Original task:
{message}"#
    )
}

pub(crate) async fn run_external_cli_with_events(
    provider: SpawnAgentProvider,
    cwd: std::path::PathBuf,
    message: String,
    transcript: String,
) -> ExternalCliTurn {
    let injected_message = external_agent_context_prompt(&message, &transcript);
    let command = match external_command_spec(provider, cwd.as_path(), &injected_message) {
        Ok(command) => command,
        Err(message) => return ExternalCliTurn::errored(message),
    };
    let child = Command::new(command.program)
        .args(command.args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let child = match child {
        Ok(child) => child,
        Err(err) => {
            return ExternalCliTurn::errored(format!(
                "{} external provider unavailable: {err}",
                provider_name(provider)
            ));
        }
    };
    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(err) => {
            return ExternalCliTurn::errored(format!(
                "{} external provider failed: {err}",
                provider_name(provider)
            ));
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let parsed = parse_external_stream(provider, &stdout);
    let tool_calls = parsed
        .iter()
        .filter_map(|event| match event {
            ExternalCliEvent::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect();
    let tool_call_errors = parsed
        .iter()
        .filter_map(|event| match event {
            ExternalCliEvent::ToolCallError(result) => Some(result.clone()),
            _ => None,
        })
        .collect();
    let transcript_lines = transcript_lines_from_events(&parsed);
    let final_message = summarize_external_output(&parsed, &stdout);
    let status = if output.status.success() {
        AgentStatus::Completed(final_message)
    } else {
        let summary = first_non_empty([
            final_message,
            Some(truncate_chars(stderr.trim(), MAX_EXTERNAL_ERROR_CHARS)),
        ])
        .unwrap_or_else(|| {
            format!(
                "{} exited with status {}",
                provider_name(provider),
                output.status
            )
        });
        AgentStatus::Errored(summary)
    };
    ExternalCliTurn {
        status,
        tool_calls,
        tool_call_errors,
        transcript_lines,
    }
}

impl ExternalCliTurn {
    pub(crate) fn errored(message: String) -> Self {
        Self {
            status: AgentStatus::Errored(truncate_chars(&message, MAX_EXTERNAL_ERROR_CHARS)),
            tool_calls: Vec::new(),
            tool_call_errors: Vec::new(),
            transcript_lines: Vec::new(),
        }
    }
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

fn summarize_external_output(events: &[ExternalCliEvent], raw_stdout: &str) -> Option<String> {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            ExternalCliEvent::Completion(text) | ExternalCliEvent::Message(text)
                if !text.trim().is_empty() =>
            {
                Some(truncate_chars(text.trim(), MAX_EXTERNAL_OUTPUT_CHARS))
            }
            ExternalCliEvent::ToolCall(_) | ExternalCliEvent::ToolCallError(_) => None,
            _ => None,
        })
        .or_else(|| {
            let raw = raw_stdout.trim();
            (!raw.is_empty()).then(|| truncate_chars(raw, MAX_EXTERNAL_OUTPUT_CHARS))
        })
}

pub(crate) fn append_external_transcript_line(transcript: &mut String, line: String) {
    let line = bounded_external_transcript_line(&line);
    if line.trim().is_empty() {
        return;
    }
    if !transcript.is_empty() {
        transcript.push('\n');
    }
    transcript.push_str(&line);
    trim_external_transcript_to_budget(transcript);
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

fn transcript_lines_from_events(events: &[ExternalCliEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            ExternalCliEvent::ToolCall(call) => {
                serde_json::to_string(&json_external_tool_call(call)).ok()
            }
            ExternalCliEvent::ToolCallError(_) => None,
            ExternalCliEvent::Message(text) if !text.trim().is_empty() => Some(json_line(
                "external_agent_message",
                truncate_chars(text.trim(), MAX_EXTERNAL_OUTPUT_CHARS),
            )),
            ExternalCliEvent::Completion(text) if !text.trim().is_empty() => Some(json_line(
                "external_agent_completion",
                truncate_chars(text.trim(), MAX_EXTERNAL_OUTPUT_CHARS),
            )),
            ExternalCliEvent::Status(text) if !text.trim().is_empty() => Some(json_line(
                "external_agent_status",
                truncate_chars(text.trim(), MAX_EXTERNAL_ERROR_CHARS),
            )),
            _ => None,
        })
        .collect()
}

fn bounded_external_transcript_line(line: &str) -> String {
    if line.chars().count() <= MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS {
        return line.to_string();
    }
    if let Ok(result) = serde_json::from_str::<ExternalToolResult>(line) {
        if result.result_type == "external_tool_result" {
            return external_tool_result_json_line(&result);
        }
    }
    json_line(
        "external_agent_status",
        format!(
            "transcript line exceeded {MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS} characters and was summarized: {}",
            truncate_chars(line, MAX_EXTERNAL_ERROR_CHARS)
        ),
    )
}

fn trim_external_transcript_to_budget(transcript: &mut String) {
    while transcript.chars().count() > MAX_EXTERNAL_TRANSCRIPT_CHARS {
        let Some(newline_index) = transcript.find('\n') else {
            *transcript = bounded_external_transcript_line(transcript);
            return;
        };
        transcript.drain(..=newline_index);
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

fn json_external_tool_call(call: &ExternalToolCall) -> serde_json::Value {
    serde_json::json!({
        "type": "external_tool_call",
        "id": &call.id,
        "tool": &call.tool,
        "arguments": &call.arguments,
    })
}

fn json_line(line_type: &str, text: String) -> String {
    serde_json::json!({
        "type": line_type,
        "text": text,
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

fn first_non_empty(values: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("...");
    output
}

fn truncate_chars_from_start(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let tail = value
        .chars()
        .rev()
        .take(max_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("...{tail}")
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
        let context = external_agent_context_prompt("review this patch", "");
        assert!(context.contains("spawn_external_agent"));
        assert!(context.contains("external_tool_call"));
        assert!(context.contains("external_tool_result"));
        assert!(context.contains("Do not call internal my-codex tools"));
        assert!(context.contains("review this patch"));
    }

    #[test]
    fn external_context_includes_bounded_tool_result_transcript() {
        let result = ExternalToolResult::ok(
            "call_1",
            json!({ "payload": "x".repeat(MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS + 100) }),
        );
        let mut transcript = "x".repeat(MAX_EXTERNAL_TRANSCRIPT_CHARS + 100);
        append_external_transcript_line(&mut transcript, external_tool_result_json_line(&result));
        let context = external_agent_context_prompt("continue work", &transcript);

        assert!(context.contains("External tool transcript so far"));
        assert!(context.contains("external_tool_result"));
        assert!(context.contains("continue work"));
        assert!(context.len() < MAX_EXTERNAL_TRANSCRIPT_CHARS + 3000);
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
    fn append_external_transcript_line_preserves_latest_result_json_line() {
        let result = ExternalToolResult::ok(
            "call_1",
            json!({ "payload": "x".repeat(MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS + 100) }),
        );
        let mut transcript = String::new();
        for index in 0..10 {
            append_external_transcript_line(
                &mut transcript,
                json_line(
                    "external_agent_message",
                    format!("{index}:{}", "x".repeat(MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS / 2)),
                ),
            );
        }
        append_external_transcript_line(&mut transcript, external_tool_result_json_line(&result));

        let latest = transcript.lines().last().expect("latest transcript line");
        let value: serde_json::Value = serde_json::from_str(latest).expect("valid latest json");
        assert!(transcript.chars().count() <= MAX_EXTERNAL_TRANSCRIPT_CHARS);
        assert_eq!(value["type"], "external_tool_result");
        assert_eq!(value["id"], "call_1");
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
    fn builds_codex_exec_json_command() {
        let spec = external_command_spec(
            SpawnAgentProvider::CodexCli,
            Path::new("/tmp/work"),
            "do it",
        )
        .expect("codex command");
        assert_eq!(spec.program, "codex");
        assert_eq!(spec.args[0], "exec");
        assert!(spec.args.contains(&"--json".to_string()));
        assert!(spec.args.contains(&"-C".to_string()));
        assert!(spec.args.contains(&"/tmp/work".to_string()));
    }
}
