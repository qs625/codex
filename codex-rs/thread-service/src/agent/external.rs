use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use crate::agent::child_completion_content_from_status;
use crate::agent::status::child_lifecycle_status_from_agent_status;
use crate::session::session::ThreadWaitSource;
use crate::session::thread_wait::ThreadWaitOutcome;
use crate::session::thread_wait::ThreadWaitState;
use crate::session::thread_wait::poll_event_result;
use crate::session::thread_wait::poll_event_timeout_result;
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
use protocol::protocol::ExternalProviderSessionIdentity;
use protocol::protocol::ExternalReconnectDescriptor;
use protocol::protocol::ExternalReconnectTransport;
use protocol::protocol::ExternalRestoreDisabledReason;
use protocol::protocol::ExternalRestoreFactState;
use protocol::protocol::ExternalRestorePlan;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use thread_service_api::ThreadPollEvent;
use thread_service_api::ThreadPollEventRequest;
use thread_service_api::ThreadPollEventResult;
use thread_service_api::ThreadPollEventTimeoutMetadata;
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
const MAX_EXTERNAL_SESSION_ID_CHARS: usize = 512;
const MAX_EXTERNAL_TRANSCRIPT_LINE_CHARS: usize = 8_000;
const MAX_EXTERNAL_TOOL_ARGUMENT_CHARS: usize = 8_000;
const CODEX_APP_SERVER_ENV_REMOVALS: &[&str] = &["MORPHEUS_HOME", "CODEX_HOME", "CODEX_THREAD_ID"];
const OPENCODE_RECONNECT_PROOF_BLOCK: &str = "opencode reconnect descriptor only stores the provider session id; the current adapter starts a transient opencode serve HTTP/SSE endpoint on port 0 and has no durable endpoint, input ownership, or wait state facts for cold reattach";

#[derive(Clone)]
pub(crate) struct ExternalAgentRun {
    pub(crate) thread_id: ThreadId,
    pub(crate) parent_thread_id: ThreadId,
    pub(crate) agent_path: AgentPath,
    pub(crate) provider: SpawnAgentProvider,
    pub(crate) depth: i32,
    pub(crate) spawn_config: Option<ExternalSpawnConfig>,
    pub(crate) input_sink: Option<ExternalAgentInputSink>,
    pub(crate) live_thread: Option<SharedLiveThread>,
    pub(crate) status: AgentStatus,
    pub(crate) active_turn_id: Option<String>,
    pub(crate) last_task_message: Option<String>,
    pub(crate) abort_handle: Option<AbortHandle>,
}

impl ExternalAgentRun {
    pub(crate) fn is_root_run(&self) -> bool {
        self.depth == 0 && self.parent_thread_id == self.thread_id
    }
}

#[derive(Clone)]
pub(crate) struct ExternalSpawnConfig {
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) workspace_roots: Vec<AbsolutePathBuf>,
    pub(crate) agent_max_threads: Option<usize>,
    pub(crate) agent_roles: BTreeMap<String, AgentRoleConfig>,
    pub(crate) model: String,
    pub(crate) model_provider_id: String,
    pub(crate) service_tier: Option<String>,
    pub(crate) approval_policy: protocol::protocol::AskForApproval,
    pub(crate) approvals_reviewer: protocol::config_types::ApprovalsReviewer,
    pub(crate) permission_profile: protocol::models::PermissionProfile,
    pub(crate) active_permission_profile: Option<protocol::models::ActivePermissionProfile>,
    pub(crate) reasoning_effort: Option<protocol::openai_models::ReasoningEffort>,
    pub(crate) personality: Option<protocol::config_types::Personality>,
    pub(crate) features: codex_features::Features,
    pub(crate) generate_memories: bool,
    pub(crate) default_wait_timeout_ms: i64,
    pub(crate) max_wait_timeout_ms: i64,
}

impl ExternalSpawnConfig {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            cwd: config.cwd.clone(),
            workspace_roots: config.workspace_roots.clone(),
            agent_max_threads: config.agent_max_threads,
            agent_roles: config.agent_roles.clone(),
            model: config.model.clone().unwrap_or_default(),
            model_provider_id: config.model_provider_id.clone(),
            service_tier: config.service_tier.clone(),
            approval_policy: config.permissions.approval_policy.value(),
            approvals_reviewer: config.approvals_reviewer,
            permission_profile: config.permissions.effective_permission_profile(),
            active_permission_profile: config.permissions.active_permission_profile(),
            reasoning_effort: config.model_reasoning_effort,
            personality: config.personality,
            features: config.features.get().clone(),
            generate_memories: config.memories.generate_memories,
            default_wait_timeout_ms: config.multi_agent_v2.default_wait_timeout_ms,
            max_wait_timeout_ms: config.multi_agent_v2.max_wait_timeout_ms,
        }
    }
}

#[derive(Default)]
pub(crate) struct ExternalAgentRegistry {
    runs: Mutex<HashMap<ThreadId, ExternalAgentRun>>,
    wait_states: Mutex<HashMap<ThreadId, ThreadWaitState>>,
}

impl ExternalAgentRegistry {
    pub(crate) fn insert_running(&self, run: ExternalAgentRun) {
        self.wait_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(run.thread_id)
            .or_default();
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

    pub(crate) fn live_thread_ids(&self) -> Vec<ThreadId> {
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
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

    pub(crate) fn begin_root_turn(
        &self,
        thread_id: ThreadId,
        turn_id: String,
    ) -> Result<ExternalAgentInputSink, String> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = runs
            .get_mut(&thread_id)
            .ok_or_else(|| "external root thread not found".to_string())?;
        if !run.is_root_run() {
            return Err("external thread is not a root thread".to_string());
        }
        if !matches!(run.status, AgentStatus::PendingInit | AgentStatus::Running) {
            return Err("external root thread is no longer accepting input".to_string());
        }
        if run.active_turn_id.is_some() {
            return Err("external root thread already has an active turn".to_string());
        }
        let input_sink = run
            .input_sink
            .clone()
            .ok_or_else(|| "external root thread cannot receive input".to_string())?;
        run.active_turn_id = Some(turn_id);
        Ok(input_sink)
    }

    pub(crate) fn clear_active_turn(&self, thread_id: ThreadId, turn_id: &str) {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(run) = runs.get_mut(&thread_id)
            && run.active_turn_id.as_deref() == Some(turn_id)
        {
            run.active_turn_id = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn shutdown(&self, thread_id: ThreadId) -> Option<ExternalAgentRun> {
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = runs.get_mut(&thread_id)?;
        run.status = AgentStatus::Shutdown;
        run.active_turn_id = None;
        if let Some(abort_handle) = run.abort_handle.take() {
            abort_handle.abort();
        }
        Some(run.clone())
    }

    pub(crate) fn shutdown_and_remove(&self, thread_id: ThreadId) -> Option<ExternalAgentRun> {
        let mut run = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&thread_id)?;
        run.status = AgentStatus::Shutdown;
        run.active_turn_id = None;
        if let Some(abort_handle) = run.abort_handle.take() {
            abort_handle.abort();
        }
        self.wait_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&thread_id);
        Some(run)
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
        run.active_turn_id = None;
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

    pub(crate) fn note_thread_wait_event(&self, thread_id: ThreadId, source: ThreadWaitSource) {
        self.note_thread_wait_event_with_events(thread_id, source, Vec::new());
    }

    pub(crate) fn note_thread_wait_event_with_events(
        &self,
        thread_id: ThreadId,
        source: ThreadWaitSource,
        events: Vec<ThreadPollEvent>,
    ) {
        let Some(state) = self
            .wait_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&thread_id)
            .cloned()
        else {
            return;
        };
        state.note_event_with_events(source, events);
    }

    pub(crate) async fn poll_event(
        &self,
        thread_id: ThreadId,
        request: ThreadPollEventRequest,
    ) -> Result<ThreadPollEventResult, String> {
        let metadata = self
            .poll_event_timeout_metadata(thread_id, request)
            .await
            .ok_or_else(|| "external sender is not registered".to_string())?;
        let Some(wait_state) = self
            .wait_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&thread_id)
            .cloned()
        else {
            return Err("external sender is not registered".to_string());
        };
        let watcher = wait_state.begin_wait();

        match wait_state.wait(watcher, &metadata).await {
            ThreadWaitOutcome::Event {
                snapshot,
                waited_ms,
            } => {
                let events = snapshot.events;
                Ok(poll_event_result(
                    snapshot.source.map(ThreadWaitSource::source_hint),
                    events.first().cloned(),
                    events,
                    waited_ms,
                    metadata,
                ))
            }
            ThreadWaitOutcome::Timeout { waited_ms } => {
                Ok(poll_event_timeout_result(waited_ms, metadata))
            }
        }
    }

    pub(crate) async fn poll_event_timeout_metadata(
        &self,
        thread_id: ThreadId,
        request: ThreadPollEventRequest,
    ) -> Option<ThreadPollEventTimeoutMetadata> {
        let wait_state = self
            .wait_states
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&thread_id)
            .cloned()?;
        let run = self.get(thread_id)?;
        let (default_initial_timeout_ms, default_hard_cap_timeout_ms) = run
            .spawn_config
            .as_ref()
            .map_or((30_000, 120_000), |config| {
                (config.default_wait_timeout_ms, config.max_wait_timeout_ms)
            });
        let initial_timeout_ms = request
            .initial_timeout_ms
            .unwrap_or(default_initial_timeout_ms);
        let hard_cap_timeout_ms = request
            .hard_cap_timeout_ms
            .unwrap_or(default_hard_cap_timeout_ms);
        Some(
            wait_state
                .timeout_metadata(initial_timeout_ms, hard_cap_timeout_ms)
                .await,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExternalCliEvent {
    Status(String),
    Message(String),
    Completion(String),
    ToolCall(ExternalToolCall),
    ToolCallError(ExternalToolResult),
    Display(ExternalProviderDisplayEvent),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ExternalProviderDisplayEvent {
    ReasoningSummary(String),
    ReasoningRawContent(String),
    ToolCall(ExternalProviderToolDisplayEvent),
    FallbackMessage(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ExternalProviderToolDisplayEvent {
    pub(crate) id: String,
    pub(crate) tool: String,
    pub(crate) arguments: JsonValue,
    pub(crate) status: protocol::protocol::ExternalToolCallStatus,
    pub(crate) output: Option<JsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalToolName {
    SpawnExternalAgent,
    FollowupExternalTask,
    ListExternalAgents,
    ReadExternalAgent,
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

#[derive(Clone, Debug)]
pub(crate) struct ExternalAgentInput {
    pub(crate) turn_id: Option<String>,
    pub(crate) content: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ExternalAgentInputSink {
    tx: mpsc::UnboundedSender<ExternalAgentInput>,
}

impl ExternalAgentInputSink {
    pub(crate) fn new(tx: mpsc::UnboundedSender<ExternalAgentInput>) -> Self {
        Self { tx }
    }

    pub(crate) fn send(&self, content: String) -> Result<(), String> {
        self.send_with_turn_id(None, content)
    }

    pub(crate) fn send_with_turn_id(
        &self,
        turn_id: Option<String>,
        content: String,
    ) -> Result<(), String> {
        self.tx
            .send(ExternalAgentInput { turn_id, content })
            .map_err(|_| "external provider stdin is closed".to_string())
    }
}

pub(crate) trait ExternalProviderSession: Send {
    fn input_sink(&self) -> ExternalInputSink;
    fn reconnect_descriptor(&self) -> Option<ExternalReconnectDescriptor> {
        None
    }
    fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>>;
}

pub(crate) fn opencode_reconnect_descriptor(session_id: &str) -> ExternalReconnectDescriptor {
    let session_id = truncate_chars(session_id, MAX_EXTERNAL_SESSION_ID_CHARS);
    let restore_plan = ExternalRestorePlan::opencode_restore_disabled(!session_id.is_empty());
    ExternalReconnectDescriptor {
        provider: "opencode".to_string(),
        transport: ExternalReconnectTransport::OpencodeHttp,
        session_identity: ExternalProviderSessionIdentity { session_id },
        restore_plan: Some(restore_plan),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalReconnectSupport {
    diagnostic: String,
}

impl ExternalReconnectSupport {
    pub(crate) fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

#[cfg(test)]
pub(crate) fn external_reconnect_support(
    descriptor: &ExternalReconnectDescriptor,
) -> ExternalReconnectSupport {
    external_restore_plan_support(&descriptor.provider, &descriptor.restore_plan())
}

pub(crate) fn external_restore_plan_support(
    provider: &str,
    plan: &ExternalRestorePlan,
) -> ExternalReconnectSupport {
    ExternalReconnectSupport {
        diagnostic: external_restore_plan_diagnostic(provider, plan),
    }
}

fn external_restore_plan_diagnostic(provider: &str, plan: &ExternalRestorePlan) -> String {
    let missing = external_restore_plan_missing_facts(plan);
    match plan.disabled_reason {
        ExternalRestoreDisabledReason::MissingProviderSessionIdentity => {
            format!("{provider} reconnect descriptor is missing a provider session id")
        }
        ExternalRestoreDisabledReason::MissingDurableOwnershipFacts if provider == "opencode" => {
            format!(
                "{OPENCODE_RECONNECT_PROOF_BLOCK}; missing ownership facts: {}",
                missing.join(", ")
            )
        }
        ExternalRestoreDisabledReason::MissingDurableOwnershipFacts => {
            format!(
                "external reconnect descriptor is missing durable ownership facts: {}",
                missing.join(", ")
            )
        }
        ExternalRestoreDisabledReason::UnsupportedProviderTransport => {
            "external reconnect descriptor provider or transport is not supported".to_string()
        }
    }
}

fn external_restore_plan_missing_facts(plan: &ExternalRestorePlan) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if plan.provider_session_identity == ExternalRestoreFactState::Missing {
        missing.push("provider session identity");
    }
    if plan.durable_endpoint == ExternalRestoreFactState::Missing {
        missing.push("durable endpoint");
    }
    if plan.input_ownership == ExternalRestoreFactState::Missing {
        missing.push("input ownership");
    }
    if plan.status_watch == ExternalRestoreFactState::Missing {
        missing.push("status/watch ownership");
    }
    if plan.wait_cursor == ExternalRestoreFactState::Missing {
        missing.push("wait cursor");
    }
    if plan.terminal_idempotency == ExternalRestoreFactState::Missing {
        missing.push("terminal idempotency proof");
    }
    missing
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

pub(crate) fn external_agent_initialization_context() -> String {
    r#"You are running as an external code agent connected to the Morpheus backend bus.

Use only this external-agent JSON protocol to collaborate with other agents. Do not call internal Morpheus tools such as spawn_agent, followup_task, list_agents, poll_event, or close_agent.

Available external tools:
- spawn_external_agent: arguments {{ "task_name": string, "provider": "claude_cli" | "opencode" | "codex_cli", "cwd": string, "message": string }}. Current external session transport support includes claude_cli stream-json, opencode HTTP sessions, and codex_cli app-server stdio sessions.
- followup_external_task: arguments {{ "target": string, "message"?: string, "content"?: [{{ "type": "text", "text": string }} | {{ "type": "image_ref", "attachment_id": string }}] }}. Use message for legacy text-only followups or content for structured followups. Use this to send work, corrections, extra context, status requests, or decisions to another agent. If a parent or another existing agent asks you to report status, progress, interim findings, blockers, or decision needs to them, emit a followup_external_task JSON tool call targeting that agent; do not answer only in this external session. A normal final answer completes this external session and does not deliver a typed inter-agent update to the requested target. Examples: report progress to your parent; send a blocker to the PM; ask a reviewer to re-review; pass new requirements to a worker. Image references are currently supported only from native Morpheus agents; external agents receive a typed error instead of a silent downgrade.
- list_external_agents: arguments {{ "path_prefix"?: string }}
- read_external_agent: arguments {{ "target": string }}. Use after list_external_agents to inspect last task and result details for one agent.
- poll_external_event: arguments {{}}. Wait for the next new thread input that reaches the external-agent bus, such as user input, child completion or other inter-agent updates, command output or exit notifications, or other queued model-consumable input. Returns wake or timeout metadata plus a best-effort source hint and typed event payload when available.
- close_external_agent: arguments {{ "target": string }}

Emit one JSON object per line for tool calls:
{{"type":"external_tool_call","id":"call_1","tool":"list_external_agents","arguments":{{}}}}

The backend returns results as JSON objects:
{{"type":"external_tool_result","id":"call_1","ok":true,"result":{{}}}}
{{"type":"external_tool_result","id":"call_1","ok":false,"error":{{"code":"invalid_arguments","message":"..."}}}}

When the backend sends an external_tool_result as input, continue the task using that result. Emit another external_tool_call only if you need another backend action; otherwise finish with a normal final answer.
"#
    .to_string()
}

pub(crate) fn external_agent_context_prompt(message: &str) -> String {
    format!(
        r#"{context}
Original task:
{message}"#,
        context = external_agent_initialization_context(),
    )
}

pub(crate) fn external_agent_initialization_context_for_run(run: &ExternalAgentRun) -> String {
    let context = external_agent_initialization_context();
    external_agent_context_with_run_metadata(&context, run)
}

pub(crate) fn external_agent_context_prompt_for_run(
    message: &str,
    run: &ExternalAgentRun,
) -> String {
    let context = external_agent_context_prompt(message);
    external_agent_context_with_run_metadata(&context, run)
}

fn external_agent_context_with_run_metadata(context: &str, run: &ExternalAgentRun) -> String {
    format!(
        r#"{context}

Current external agent metadata:
- thread_id: {thread_id}
- parent_thread_id: {parent_thread_id}
- agent_path: {agent_path}
- agent_role: {agent_role}
- provider: {provider}
- depth: {depth}"#,
        thread_id = run.thread_id,
        parent_thread_id = run.parent_thread_id,
        agent_path = run.agent_path,
        agent_role = provider_name(run.provider),
        provider = provider_name(run.provider),
        depth = run.depth,
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

    fn reconnect_descriptor(&self) -> Option<ExternalReconnectDescriptor> {
        match self {
            ExternalStreamingSession::OpencodeHttp(session) => {
                Some(session.reconnect_descriptor.clone())
            }
            ExternalStreamingSession::Cli(_) | ExternalStreamingSession::CodexAppServer(_) => None,
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
    seen_reasoning_item_ids: Arc<Mutex<HashSet<String>>>,
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
        let seen_reasoning_item_ids = Arc::new(Mutex::new(HashSet::new()));
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
            seen_reasoning_item_ids,
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
                            if let Some(event) = parse_codex_app_server_jsonrpc_line_with_seen(
                                &line,
                                &self.active_turn_id,
                                &self.pending_completion_event,
                                &self.seen_reasoning_item_ids,
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
    reconnect_descriptor: ExternalReconnectDescriptor,
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
        let reconnect_descriptor = opencode_reconnect_descriptor(&session_id);
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
            reconnect_descriptor,
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

pub(crate) fn external_tool_name(tool: &ExternalToolName) -> String {
    serde_json::to_value(tool)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{tool:?}"))
}

pub(crate) fn bounded_external_tool_arguments(arguments: &JsonValue) -> JsonValue {
    bounded_json_value(arguments, MAX_EXTERNAL_TOOL_ARGUMENT_CHARS)
}

pub(crate) fn bounded_external_tool_result(result: &ExternalToolResult) -> JsonValue {
    if result.ok {
        return result
            .result
            .as_ref()
            .map(|value| bounded_json_value(value, MAX_EXTERNAL_OUTPUT_CHARS))
            .unwrap_or(JsonValue::Null);
    }

    let error = result.error.as_ref();
    serde_json::json!({
        "error": {
            "code": error
                .map(|error| truncate_chars(&error.code, 128))
                .unwrap_or_else(|| "tool_error".to_string()),
            "message": error
                .map(|error| truncate_chars(&error.message, MAX_EXTERNAL_ERROR_CHARS))
                .unwrap_or_else(|| "external tool failed".to_string()),
        }
    })
}

pub(crate) fn bounded_external_output(message: &str) -> String {
    truncate_chars(message.trim(), MAX_EXTERNAL_OUTPUT_CHARS)
}

pub(crate) fn bounded_json_for_external_display_output(value: &JsonValue) -> JsonValue {
    bounded_json_value(value, MAX_EXTERNAL_OUTPUT_CHARS)
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
    let message = child_completion_content_from_status(&run.status);
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
        .with_lifecycle_status(child_lifecycle_status_from_agent_status(&run.status))
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

#[cfg(test)]
fn parse_codex_app_server_jsonrpc_line(
    line: &str,
    active_turn_id: &Arc<Mutex<Option<String>>>,
    pending_completion_event: &Arc<Mutex<Option<ExternalCliEvent>>>,
) -> Option<ExternalProcessEvent> {
    let seen_reasoning_item_ids = Arc::new(Mutex::new(HashSet::new()));
    parse_codex_app_server_jsonrpc_line_with_seen(
        line,
        active_turn_id,
        pending_completion_event,
        &seen_reasoning_item_ids,
    )
}

fn parse_codex_app_server_jsonrpc_line_with_seen(
    line: &str,
    active_turn_id: &Arc<Mutex<Option<String>>>,
    pending_completion_event: &Arc<Mutex<Option<ExternalCliEvent>>>,
    seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>,
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
            clear_seen_codex_reasoning_item_ids(seen_reasoning_item_ids);
            None
        }
        "turn/completed" => {
            set_active_turn_id(active_turn_id, None);
            let event = last_agent_message_text(params)
                .map(codex_completion_event_from_text)
                .or_else(|| take_pending_codex_completion_event(pending_completion_event));
            clear_seen_codex_reasoning_item_ids(seen_reasoning_item_ids);
            if let Some(event) = event {
                return Some(ExternalProcessEvent::Cli(event));
            }
            if let Some(error) = codex_turn_completed_without_message_error(params) {
                return Some(ExternalProcessEvent::StdinError(error));
            }
            None
        }
        "turn/failed" | "turn/interrupted" => {
            set_active_turn_id(active_turn_id, None);
            set_pending_codex_completion_event(pending_completion_event, None);
            clear_seen_codex_reasoning_item_ids(seen_reasoning_item_ids);
            Some(ExternalProcessEvent::StdinError(
                codex_turn_failure_message(params),
            ))
        }
        "error" => {
            let message = codex_app_server_error_message(params);
            if params
                .get("willRetry")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                Some(ExternalProcessEvent::Cli(ExternalCliEvent::Status(message)))
            } else {
                set_active_turn_id(active_turn_id, None);
                set_pending_codex_completion_event(pending_completion_event, None);
                clear_seen_codex_reasoning_item_ids(seen_reasoning_item_ids);
                Some(ExternalProcessEvent::StdinError(message))
            }
        }
        "item/completed" => {
            if let Some(text) = item_agent_message_text(params) {
                set_pending_codex_completion_event(
                    pending_completion_event,
                    Some(codex_completion_event_from_text(text)),
                );
            }
            codex_provider_display_event_from_item_completed(params, seen_reasoning_item_ids)
                .map(|event| ExternalProcessEvent::Cli(ExternalCliEvent::Display(event)))
        }
        "item/agentMessage/delta" => None,
        "item/reasoning/summaryTextDelta" => {
            let delta = delta_field(params)?;
            note_seen_codex_reasoning_item_id(params, seen_reasoning_item_ids);
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary(delta),
            )))
        }
        "item/reasoning/textDelta" => {
            let delta = delta_field(params)?;
            note_seen_codex_reasoning_item_id(params, seen_reasoning_item_ids);
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningRawContent(delta),
            )))
        }
        "item/reasoning/summaryPartAdded" => None,
        "warning" | "guardianWarning" | "configWarning" => {
            text_field(params).map(|text| ExternalProcessEvent::Cli(ExternalCliEvent::Status(text)))
        }
        _ => None,
    }
}

fn codex_provider_display_event_from_item_completed(
    params: &serde_json::Value,
    seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>,
) -> Option<ExternalProviderDisplayEvent> {
    let item = params.get("item")?.clone();
    let item = serde_json::from_value::<app_server_protocol::ThreadItem>(item).ok()?;
    codex_provider_display_event_from_thread_item(item, seen_reasoning_item_ids)
}

fn codex_provider_display_event_from_thread_item(
    item: app_server_protocol::ThreadItem,
    seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>,
) -> Option<ExternalProviderDisplayEvent> {
    match item {
        app_server_protocol::ThreadItem::UserMessage { .. } => None,
        app_server_protocol::ThreadItem::AgentMessage { .. } => None,
        app_server_protocol::ThreadItem::Reasoning {
            id,
            summary,
            content,
        } => {
            if take_seen_codex_reasoning_item_id(&id, seen_reasoning_item_ids) {
                return None;
            }
            let summary = summary.join("\n").trim().to_string();
            if !summary.is_empty() {
                return Some(ExternalProviderDisplayEvent::ReasoningSummary(
                    bounded_external_output(&summary),
                ));
            }
            let content = content.join("\n").trim().to_string();
            if !content.is_empty() {
                return Some(ExternalProviderDisplayEvent::ReasoningRawContent(
                    bounded_external_output(&content),
                ));
            }
            None
        }
        app_server_protocol::ThreadItem::EventDrivenToolCall {
            id,
            tool,
            arguments,
            status,
            output,
        }
        | app_server_protocol::ThreadItem::BuiltinToolCall {
            id,
            tool,
            arguments,
            status,
            output,
        } => Some(ExternalProviderDisplayEvent::ToolCall(
            ExternalProviderToolDisplayEvent {
                id,
                tool,
                arguments: bounded_json_value(&arguments, MAX_EXTERNAL_TOOL_ARGUMENT_CHARS),
                status: external_status_from_dynamic_status(status),
                output: output.map(|value| bounded_json_value(&value, MAX_EXTERNAL_OUTPUT_CHARS)),
            },
        )),
        app_server_protocol::ThreadItem::DynamicToolCall {
            id,
            namespace,
            tool,
            arguments,
            status,
            content_items,
            success,
            duration_ms,
        } => {
            let output = serde_json::json!({
                "contentItems": content_items,
                "success": success,
                "durationMs": duration_ms,
            });
            Some(ExternalProviderDisplayEvent::ToolCall(
                ExternalProviderToolDisplayEvent {
                    id,
                    tool: namespace
                        .filter(|namespace| !namespace.trim().is_empty())
                        .map(|namespace| format!("{namespace}.{tool}"))
                        .unwrap_or(tool),
                    arguments: bounded_json_value(&arguments, MAX_EXTERNAL_TOOL_ARGUMENT_CHARS),
                    status: external_status_from_dynamic_status(status),
                    output: Some(bounded_json_value(&output, MAX_EXTERNAL_OUTPUT_CHARS)),
                },
            ))
        }
        app_server_protocol::ThreadItem::McpToolCall {
            id,
            server,
            tool,
            status,
            arguments,
            result,
            error,
            duration_ms,
            ..
        } => {
            let output = serde_json::json!({
                "result": result,
                "error": error,
                "durationMs": duration_ms,
            });
            Some(ExternalProviderDisplayEvent::ToolCall(
                ExternalProviderToolDisplayEvent {
                    id,
                    tool: format!("{server}.{tool}"),
                    arguments: bounded_json_value(&arguments, MAX_EXTERNAL_TOOL_ARGUMENT_CHARS),
                    status: external_status_from_mcp_status(status),
                    output: Some(bounded_json_value(&output, MAX_EXTERNAL_OUTPUT_CHARS)),
                },
            ))
        }
        app_server_protocol::ThreadItem::CommandExecution {
            id,
            command,
            cwd,
            status,
            aggregated_output,
            exit_code,
            duration_ms,
            ..
        } => {
            let output = serde_json::json!({
                "output": aggregated_output,
                "exitCode": exit_code,
                "durationMs": duration_ms,
            });
            Some(ExternalProviderDisplayEvent::ToolCall(
                ExternalProviderToolDisplayEvent {
                    id,
                    tool: "command_execution".to_string(),
                    arguments: bounded_json_value(
                        &serde_json::json!({
                            "command": command,
                            "cwd": cwd,
                        }),
                        MAX_EXTERNAL_TOOL_ARGUMENT_CHARS,
                    ),
                    status: external_status_from_command_status(status),
                    output: Some(bounded_json_value(&output, MAX_EXTERNAL_OUTPUT_CHARS)),
                },
            ))
        }
        app_server_protocol::ThreadItem::WebSearch { id, query, action } => Some(
            ExternalProviderDisplayEvent::ToolCall(ExternalProviderToolDisplayEvent {
                id,
                tool: "web_search".to_string(),
                arguments: bounded_json_value(
                    &serde_json::json!({
                        "query": query,
                        "action": action,
                    }),
                    MAX_EXTERNAL_TOOL_ARGUMENT_CHARS,
                ),
                status: protocol::protocol::ExternalToolCallStatus::Completed,
                output: None,
            }),
        ),
        other => Some(ExternalProviderDisplayEvent::FallbackMessage(
            bounded_external_output(&format!(
                "codex_cli emitted unsupported display item `{}`",
                codex_thread_item_type_name(&other)
            )),
        )),
    }
}

fn note_seen_codex_reasoning_item_id(
    params: &serde_json::Value,
    seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>,
) {
    let Some(item_id) = params
        .get("itemId")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
    else {
        return;
    };
    seen_reasoning_item_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item_id);
}

fn take_seen_codex_reasoning_item_id(
    item_id: &str,
    seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>,
) -> bool {
    seen_reasoning_item_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(item_id)
}

fn clear_seen_codex_reasoning_item_ids(seen_reasoning_item_ids: &Arc<Mutex<HashSet<String>>>) {
    seen_reasoning_item_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn external_status_from_dynamic_status(
    status: app_server_protocol::DynamicToolCallStatus,
) -> protocol::protocol::ExternalToolCallStatus {
    match status {
        app_server_protocol::DynamicToolCallStatus::InProgress => {
            protocol::protocol::ExternalToolCallStatus::InProgress
        }
        app_server_protocol::DynamicToolCallStatus::Completed => {
            protocol::protocol::ExternalToolCallStatus::Completed
        }
        app_server_protocol::DynamicToolCallStatus::Failed => {
            protocol::protocol::ExternalToolCallStatus::Failed
        }
    }
}

fn external_status_from_mcp_status(
    status: app_server_protocol::McpToolCallStatus,
) -> protocol::protocol::ExternalToolCallStatus {
    match status {
        app_server_protocol::McpToolCallStatus::InProgress => {
            protocol::protocol::ExternalToolCallStatus::InProgress
        }
        app_server_protocol::McpToolCallStatus::Completed => {
            protocol::protocol::ExternalToolCallStatus::Completed
        }
        app_server_protocol::McpToolCallStatus::Failed => {
            protocol::protocol::ExternalToolCallStatus::Failed
        }
    }
}

fn external_status_from_command_status(
    status: app_server_protocol::CommandExecutionStatus,
) -> protocol::protocol::ExternalToolCallStatus {
    match status {
        app_server_protocol::CommandExecutionStatus::InProgress => {
            protocol::protocol::ExternalToolCallStatus::InProgress
        }
        app_server_protocol::CommandExecutionStatus::Completed => {
            protocol::protocol::ExternalToolCallStatus::Completed
        }
        app_server_protocol::CommandExecutionStatus::Failed
        | app_server_protocol::CommandExecutionStatus::Declined => {
            protocol::protocol::ExternalToolCallStatus::Failed
        }
    }
}

fn codex_thread_item_type_name(item: &app_server_protocol::ThreadItem) -> &'static str {
    match item {
        app_server_protocol::ThreadItem::UserMessage { .. } => "userMessage",
        app_server_protocol::ThreadItem::HookPrompt { .. } => "hookPrompt",
        app_server_protocol::ThreadItem::InjectedContext { .. } => "injectedContext",
        app_server_protocol::ThreadItem::AgentMessage { .. } => "agentMessage",
        app_server_protocol::ThreadItem::Plan { .. } => "plan",
        app_server_protocol::ThreadItem::Reasoning { .. } => "reasoning",
        app_server_protocol::ThreadItem::CommandExecution { .. } => "commandExecution",
        app_server_protocol::ThreadItem::CommandExecutionNotification { .. } => {
            "commandExecutionNotification"
        }
        app_server_protocol::ThreadItem::CommandWait { .. } => "commandWait",
        app_server_protocol::ThreadItem::CommandWriteStdin { .. } => "commandWriteStdin",
        app_server_protocol::ThreadItem::FileChange { .. } => "fileChange",
        app_server_protocol::ThreadItem::McpToolCall { .. } => "mcpToolCall",
        app_server_protocol::ThreadItem::BuiltinToolCall { .. } => "builtinToolCall",
        app_server_protocol::ThreadItem::DynamicToolCall { .. } => "dynamicToolCall",
        app_server_protocol::ThreadItem::EventDrivenToolCall { .. } => "eventDrivenToolCall",
        app_server_protocol::ThreadItem::EventDrivenTool { .. } => "eventDrivenTool",
        app_server_protocol::ThreadItem::EventCommandCall { .. } => "eventCommandCall",
        app_server_protocol::ThreadItem::EventCommandEvent { .. } => "eventCommandEvent",
        app_server_protocol::ThreadItem::ThreadGoalUpdate { .. } => "threadGoalUpdate",
        app_server_protocol::ThreadItem::ContextCompaction { .. } => "contextCompaction",
        app_server_protocol::ThreadItem::WorkflowRunProgress { .. } => "workflowRunProgress",
        app_server_protocol::ThreadItem::EnteredReviewMode { .. } => "enteredReviewMode",
        app_server_protocol::ThreadItem::ExitedReviewMode { .. } => "exitedReviewMode",
        app_server_protocol::ThreadItem::CollabAgentToolCall { .. } => "collabAgentToolCall",
        app_server_protocol::ThreadItem::CollabAgentStatusUpdate { .. } => {
            "collabAgentStatusUpdate"
        }
        app_server_protocol::ThreadItem::CollabAgentMessage { .. } => "collabAgentMessage",
        app_server_protocol::ThreadItem::WebSearch { .. } => "webSearch",
        app_server_protocol::ThreadItem::ImageView { .. } => "imageView",
        app_server_protocol::ThreadItem::ImageGeneration { .. } => "imageGeneration",
    }
}

fn codex_app_server_error_message(params: &serde_json::Value) -> String {
    let message = params
        .get("error")
        .and_then(turn_error_text)
        .or_else(|| text_field(params))
        .unwrap_or_else(|| "codex_cli app-server reported an error".to_string());
    let details = params
        .get("additionalDetails")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|details| !details.is_empty());
    match details {
        Some(details) => truncate_chars(
            &format!("codex_cli app-server error: {message}; {details}"),
            MAX_EXTERNAL_ERROR_CHARS,
        ),
        None => truncate_chars(
            &format!("codex_cli app-server error: {message}"),
            MAX_EXTERNAL_ERROR_CHARS,
        ),
    }
}

fn codex_turn_completed_without_message_error(params: &serde_json::Value) -> Option<String> {
    let status = params
        .get("turn")
        .and_then(|turn| turn.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("completed");
    if status == "completed" {
        return None;
    }
    Some(codex_turn_failure_message(params))
}

fn codex_turn_failure_message(params: &serde_json::Value) -> String {
    let status = params
        .get("turn")
        .and_then(|turn| turn.get("status"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("failed");
    let error_text = params
        .get("turn")
        .and_then(|turn| turn.get("error"))
        .and_then(turn_error_text)
        .or_else(|| params.get("error").and_then(turn_error_text));
    match error_text {
        Some(error) if !error.trim().is_empty() => format!(
            "codex_cli app-server turn completed with status {status}: {}",
            truncate_chars(error.trim(), MAX_EXTERNAL_ERROR_CHARS)
        ),
        _ => format!(
            "codex_cli app-server turn completed with status {status} before producing an agent message"
        ),
    }
}

fn turn_error_text(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        ["message", "text", "error"]
            .into_iter()
            .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
            .map(ToOwned::to_owned)
    })
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

fn bounded_json_value(value: &JsonValue, max_chars: usize) -> JsonValue {
    let serialized = serde_json::to_string(value);
    let Ok(serialized) = serialized else {
        return serde_json::json!({
            "truncated": true,
            "message": "failed to serialize external tool payload",
        });
    };
    if serialized.chars().count() <= max_chars {
        return value.clone();
    }
    serde_json::json!({
        "truncated": true,
        "preview": truncate_chars(&serialized, max_chars),
    })
}

fn text_field(value: &serde_json::Value) -> Option<String> {
    ["message", "text", "content", "summary", "error"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
}

fn delta_field(value: &serde_json::Value) -> Option<String> {
    value
        .get("delta")
        .and_then(serde_json::Value::as_str)
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
        assert!(context.contains("followup_external_task"));
        assert!(context.contains("poll_external_event"));
        assert!(context.contains("external_tool_call"));
        assert!(context.contains("external_tool_result"));
        assert!(context.contains("Do not call internal Morpheus tools"));
        assert!(
            context
                .contains("send work, corrections, extra context, status requests, or decisions")
        );
        assert!(context.contains("status, progress, interim findings, blockers"));
        assert!(context.contains("decision needs"));
        assert!(
            context.contains("emit a followup_external_task JSON tool call targeting that agent")
        );
        assert!(context.contains("do not answer only in this external session"));
        assert!(context.contains("typed inter-agent update to the requested target"));
        assert!(context.contains("report progress to your parent"));
        assert!(context.contains("send a blocker to the PM"));
        assert!(context.contains("ask a reviewer to re-review"));
        assert!(context.contains("pass new requirements to a worker"));
        assert!(!context.contains("unsupported"));
        assert!(context.contains("review this patch"));
    }

    #[test]
    fn external_context_for_run_injects_agent_metadata() {
        let run = ExternalAgentRun {
            thread_id: ThreadId::new(),
            parent_thread_id: ThreadId::new(),
            agent_path: AgentPath::try_from("/cp_http_api").expect("agent path"),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: None,
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        };

        let context = external_agent_context_prompt_for_run("explain this project", &run);

        assert!(context.contains("explain this project"));
        assert!(context.contains("spawn_external_agent"));
        assert!(context.contains("Current external agent metadata"));
        assert!(context.contains("agent_path: /cp_http_api"));
        assert!(context.contains("agent_role: claude_cli"));
    }

    #[test]
    fn external_initialization_context_for_run_excludes_original_task() {
        let run = ExternalAgentRun {
            thread_id: ThreadId::new(),
            parent_thread_id: ThreadId::new(),
            agent_path: AgentPath::try_from("/cp_http_api").expect("agent path"),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: None,
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        };

        let context = external_agent_initialization_context_for_run(&run);

        assert!(context.contains("spawn_external_agent"));
        assert!(context.contains("Current external agent metadata"));
        assert!(context.contains("agent_path: /cp_http_api"));
        assert!(context.contains("agent_role: claude_cli"));
        assert!(!context.contains("Original task:"));
    }

    #[test]
    fn completion_communication_uses_plain_typed_status_content() {
        let parent_thread_id = ThreadId::new();
        let child_thread_id = ThreadId::new();
        let run = ExternalAgentRun {
            thread_id: child_thread_id,
            parent_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: None,
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Completed(Some("done".to_string())),
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        };

        let communication = completion_communication(&run).expect("completion communication");

        assert_eq!(communication.content, "done");
        assert!(!communication.content.contains("<subagent_notification>"));
        assert_eq!(communication.status, Some(run.status));
        assert_eq!(
            communication.lifecycle_status,
            Some(protocol::protocol::ThreadLifecycleStatus::completed(Some(
                "done".to_string()
            )))
        );
        assert_eq!(
            communication.operation,
            InterAgentOperation::ChildCompletion
        );
        assert_eq!(communication.sender_thread_id, Some(child_thread_id));
        assert_eq!(communication.recipient_thread_id, Some(parent_thread_id));
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
            active_turn_id: None,
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

        assert!(envs.contains(&("MORPHEUS_HOME".to_string(), false)));
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
    fn parses_codex_app_server_reasoning_item_as_display_event() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","completedAtMs":10,"item":{"type":"reasoning","id":"reasoning_1","summary":["look at project files"],"content":[]}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary("look at project files".to_string())
            )))
        );
    }

    #[test]
    fn parses_codex_app_server_reasoning_delta_as_display_event() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thr_1","turnId":"turn_1","itemId":"reasoning_1","delta":"reading files","summaryIndex":0}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary("reading files".to_string())
            )))
        );
    }

    #[test]
    fn parses_codex_app_server_reasoning_text_delta_as_display_event() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/reasoning/textDelta","params":{"threadId":"thr_1","turnId":"turn_1","itemId":"reasoning_1","delta":"raw thinking","contentIndex":0}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningRawContent("raw thinking".to_string())
            )))
        );
    }

    #[test]
    fn skips_codex_app_server_completed_reasoning_after_seen_delta_item() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let seen_reasoning_item_ids = Arc::new(Mutex::new(HashSet::new()));
        let delta_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thr_1","turnId":"turn_1","itemId":"reasoning_1","delta":"reading files","summaryIndex":0}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );
        let completed_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","completedAtMs":10,"item":{"type":"reasoning","id":"reasoning_1","summary":["reading files"],"content":[]}}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );

        assert!(matches!(
            delta_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary(_)
            )))
        ));
        assert!(completed_event.is_none());
    }

    #[test]
    fn clears_seen_codex_app_server_reasoning_items_when_turn_completes() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let seen_reasoning_item_ids = Arc::new(Mutex::new(HashSet::new()));
        let delta_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thr_1","turnId":"turn_1","itemId":"reasoning_1","delta":"reading files","summaryIndex":0}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );
        let turn_completed_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"completed","error":null,"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );
        let completed_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_2","completedAtMs":10,"item":{"type":"reasoning","id":"reasoning_1","summary":["fresh turn reasoning"],"content":[]}}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );

        assert!(matches!(
            delta_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary(_)
            )))
        ));
        assert!(turn_completed_event.is_none());
        assert_eq!(
            completed_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary("fresh turn reasoning".to_string())
            )))
        );
    }

    #[test]
    fn malformed_codex_app_server_reasoning_delta_does_not_suppress_completed_item() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let seen_reasoning_item_ids = Arc::new(Mutex::new(HashSet::new()));
        let malformed_delta_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/reasoning/summaryTextDelta","params":{"threadId":"thr_1","turnId":"turn_1","itemId":"reasoning_1","summaryIndex":0}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );
        let completed_event = parse_codex_app_server_jsonrpc_line_with_seen(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","completedAtMs":10,"item":{"type":"reasoning","id":"reasoning_1","summary":["completed reasoning"],"content":[]}}}"#,
            &active_turn_id,
            &pending_completion_event,
            &seen_reasoning_item_ids,
        );

        assert!(malformed_delta_event.is_none());
        assert_eq!(
            completed_event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ReasoningSummary("completed reasoning".to_string())
            )))
        );
    }

    #[test]
    fn parses_codex_app_server_tool_item_as_display_event() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","completedAtMs":10,"item":{"type":"eventDrivenToolCall","id":"tool_1","tool":"read_file","arguments":{"path":"Cargo.toml"},"status":"completed","output":{"ok":true}}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ToolCall(ExternalProviderToolDisplayEvent {
                    id: "tool_1".to_string(),
                    tool: "read_file".to_string(),
                    arguments: json!({"path": "Cargo.toml"}),
                    status: protocol::protocol::ExternalToolCallStatus::Completed,
                    output: Some(json!({"ok": true})),
                })
            )))
        );
    }

    #[test]
    fn parses_codex_app_server_command_execution_as_display_event() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"item/completed","params":{"threadId":"thr_1","turnId":"turn_1","completedAtMs":10,"item":{"type":"commandExecution","id":"cmd_1","command":"ls","cwd":"/tmp/project","processId":null,"source":"unifiedExecStartup","status":"completed","initialWaitMs":null,"notifyOn":null,"commandActions":[],"aggregatedOutput":"ok","exitCode":0,"durationMs":25}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Display(
                ExternalProviderDisplayEvent::ToolCall(ExternalProviderToolDisplayEvent {
                    id: "cmd_1".to_string(),
                    tool: "command_execution".to_string(),
                    arguments: json!({
                        "command": "ls",
                        "cwd": "/tmp/project",
                    }),
                    status: protocol::protocol::ExternalToolCallStatus::Completed,
                    output: Some(json!({
                        "durationMs": 25,
                        "exitCode": 0,
                        "output": "ok",
                    })),
                })
            )))
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
    fn parses_codex_app_server_interrupted_turn_completed_as_stdin_error() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"interrupted","error":{"message":"provider stopped"},"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::StdinError(
                "codex_cli app-server turn completed with status interrupted: provider stopped"
                    .to_string()
            ))
        );
        assert!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn parses_codex_app_server_interrupted_turn_without_message_as_stdin_error() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"turn/completed","params":{"threadId":"thr_1","turn":{"id":"turn_1","items":[],"itemsView":"notLoaded","status":"interrupted","error":null,"startedAt":1,"completedAt":2,"durationMs":100}}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::StdinError(
                "codex_cli app-server turn completed with status interrupted before producing an agent message"
                    .to_string()
            ))
        );
        assert!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        );
    }

    #[test]
    fn parses_codex_app_server_retrying_error_as_status_without_clearing_state() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(None));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"error","params":{"threadId":"thr_1","turnId":"turn_1","error":{"message":"Reconnecting... 2/5"},"additionalDetails":"unexpected status 401 Unauthorized","willRetry":true}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::Cli(ExternalCliEvent::Status(
                "codex_cli app-server error: Reconnecting... 2/5; unexpected status 401 Unauthorized"
                    .to_string()
            )))
        );
        assert_eq!(
            active_turn_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_deref(),
            Some("turn_1")
        );
    }

    #[test]
    fn parses_codex_app_server_terminal_error_as_stdin_error_and_clears_state() {
        let active_turn_id = Arc::new(Mutex::new(Some("turn_1".to_string())));
        let pending_completion_event = Arc::new(Mutex::new(Some(ExternalCliEvent::Completion(
            "stale".to_string(),
        ))));
        let event = parse_codex_app_server_jsonrpc_line(
            r#"{"method":"error","params":{"threadId":"thr_1","turnId":"turn_1","error":{"message":"Authentication failed"},"additionalDetails":"unexpected status 401 Unauthorized","willRetry":false}}"#,
            &active_turn_id,
            &pending_completion_event,
        );

        assert_eq!(
            event,
            Some(ExternalProcessEvent::StdinError(
                "codex_cli app-server error: Authentication failed; unexpected status 401 Unauthorized"
                    .to_string()
            ))
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

    #[test]
    fn opencode_reconnect_descriptor_is_bounded_and_provider_scoped() {
        let long_session_id = "s".repeat(MAX_EXTERNAL_SESSION_ID_CHARS + 10);
        let descriptor = opencode_reconnect_descriptor(&long_session_id);

        assert_eq!(descriptor.provider, "opencode");
        assert_eq!(
            descriptor.transport,
            ExternalReconnectTransport::OpencodeHttp
        );
        assert_eq!(
            descriptor.session_identity.session_id.chars().count(),
            MAX_EXTERNAL_SESSION_ID_CHARS + 3
        );
        assert!(descriptor.session_identity.session_id.ends_with("..."));
        let restore_plan = descriptor.restore_plan.expect("opencode restore plan");
        assert_eq!(
            restore_plan.provider_session_identity,
            ExternalRestoreFactState::Present
        );
        assert_eq!(
            restore_plan.durable_endpoint,
            ExternalRestoreFactState::Missing
        );
        assert_eq!(
            restore_plan.input_ownership,
            ExternalRestoreFactState::Missing
        );
        assert_eq!(restore_plan.status_watch, ExternalRestoreFactState::Missing);
        assert_eq!(restore_plan.wait_cursor, ExternalRestoreFactState::Missing);
        assert_eq!(
            restore_plan.terminal_idempotency,
            ExternalRestoreFactState::Missing
        );
        assert!(!restore_plan.restore_enabled);

        struct NoDescriptorSession;
        impl ExternalProviderSession for NoDescriptorSession {
            fn input_sink(&self) -> ExternalInputSink {
                let (tx, _rx) = mpsc::unbounded_channel();
                ExternalInputSink::new(tx)
            }

            fn next_event<'a>(&'a mut self) -> BoxFuture<'a, Result<ExternalProcessEvent, String>> {
                Box::pin(async {
                    std::future::pending::<Result<ExternalProcessEvent, String>>().await
                })
            }
        }

        assert!(NoDescriptorSession.reconnect_descriptor().is_none());
    }

    #[test]
    fn opencode_reconnect_descriptor_is_restore_disabled_without_durable_endpoint() {
        let descriptor = opencode_reconnect_descriptor("opencode-session-123");

        let support = external_reconnect_support(&descriptor);

        assert!(support.diagnostic().contains("provider session id"));
        assert!(support.diagnostic().contains("transient opencode serve"));
        assert!(support.diagnostic().contains("no durable endpoint"));
        assert!(support.diagnostic().contains("input ownership"));
        assert!(support.diagnostic().contains("status/watch ownership"));
        assert!(support.diagnostic().contains("wait cursor"));
        assert!(support.diagnostic().contains("terminal idempotency proof"));
    }

    #[test]
    fn opencode_reconnect_descriptor_without_session_id_is_restore_disabled() {
        let descriptor = ExternalReconnectDescriptor {
            provider: "opencode".to_string(),
            transport: ExternalReconnectTransport::OpencodeHttp,
            session_identity: ExternalProviderSessionIdentity {
                session_id: String::new(),
            },
            restore_plan: Some(ExternalRestorePlan::opencode_restore_disabled(false)),
        };

        let support = external_reconnect_support(&descriptor);

        assert!(
            support
                .diagnostic()
                .contains("missing a provider session id")
        );
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
