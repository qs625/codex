use std::future::Future;
use std::sync::Arc;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AgentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentContentPart;
use serde::Serialize;
use tool_service_api::FunctionCallError;

use crate::ListedAgent;
use crate::SpawnAgentForkMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnAgentProvider {
    Native,
    CodexCli,
    ClaudeCli,
    Opencode,
}

#[derive(Debug, Clone)]
pub struct SpawnAgentToolRequest {
    pub message: String,
    pub task_name: String,
    pub provider: Option<SpawnAgentProvider>,
    pub agent_type: Option<String>,
    pub cwd: Option<AbsolutePathBuf>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub service_tier: Option<String>,
    pub fork_mode: Option<SpawnAgentForkMode>,
}

#[derive(Debug, Clone)]
pub struct SpawnExternalAgentToolRequest {
    pub message: String,
    pub task_name: String,
    pub provider: SpawnAgentProvider,
    pub cwd: AbsolutePathBuf,
}

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum SpawnAgentToolResult {
    WithNickname {
        task_name: String,
        nickname: Option<String>,
    },
    HiddenMetadata {
        task_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitAgentReason {
    PendingMessage,
    MailboxMessage,
    ThreadInput,
    FinalStatus,
    StatusUpdate,
    Timeout,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaitAgentToolResult {
    pub target: String,
    pub agent_name: String,
    pub reason: WaitAgentReason,
    pub timed_out: bool,
    pub status: AgentStatus,
    pub message_operation: Option<String>,
    pub message_author: Option<String>,
    pub message_excerpt: Option<String>,
    pub waited_ms: i64,
    pub initial_timeout_ms: i64,
    pub current_timeout_ms: i64,
    pub hard_cap_timeout_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct CloseAgentToolResult {
    pub previous_status: AgentStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentsToolResult {
    pub agents: Vec<ListedAgent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadAgentToolResult {
    pub agent: crate::AgentDetails,
}

pub trait MultiAgentToolSession<Turn>: Send + Sync + 'static {
    fn spawn_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        request: SpawnAgentToolRequest,
    ) -> impl Future<Output = Result<SpawnAgentToolResult, FunctionCallError>> + Send + '_;

    fn followup_task_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
        message: String,
        content_parts: Vec<InterAgentContentPart>,
    ) -> impl Future<Output = Result<(), FunctionCallError>> + Send + '_;

    fn wait_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<WaitAgentToolResult, FunctionCallError>> + Send + '_;

    fn close_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<CloseAgentToolResult, FunctionCallError>> + Send + '_;

    fn list_agents_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        path_prefix: Option<String>,
    ) -> impl Future<Output = Result<ListAgentsToolResult, FunctionCallError>> + Send + '_;
}

impl<T, Turn> MultiAgentToolSession<Turn> for Arc<T>
where
    T: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
{
    fn spawn_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        request: SpawnAgentToolRequest,
    ) -> impl Future<Output = Result<SpawnAgentToolResult, FunctionCallError>> + Send + '_ {
        <T as MultiAgentToolSession<Turn>>::spawn_agent_tool(
            Arc::clone(self.as_ref()),
            turn,
            call_id,
            request,
        )
    }

    fn followup_task_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
        message: String,
        content_parts: Vec<InterAgentContentPart>,
    ) -> impl Future<Output = Result<(), FunctionCallError>> + Send + '_ {
        <T as MultiAgentToolSession<Turn>>::followup_task_tool(
            Arc::clone(self.as_ref()),
            turn,
            call_id,
            target,
            message,
            content_parts,
        )
    }

    fn wait_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<WaitAgentToolResult, FunctionCallError>> + Send + '_ {
        <T as MultiAgentToolSession<Turn>>::wait_agent_tool(
            Arc::clone(self.as_ref()),
            turn,
            call_id,
            target,
        )
    }

    fn close_agent_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<CloseAgentToolResult, FunctionCallError>> + Send + '_ {
        <T as MultiAgentToolSession<Turn>>::close_agent_tool(
            Arc::clone(self.as_ref()),
            turn,
            call_id,
            target,
        )
    }

    fn list_agents_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        path_prefix: Option<String>,
    ) -> impl Future<Output = Result<ListAgentsToolResult, FunctionCallError>> + Send + '_ {
        <T as MultiAgentToolSession<Turn>>::list_agents_tool(
            Arc::clone(self.as_ref()),
            turn,
            call_id,
            path_prefix,
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn wait_agent_result_from_message(
    target: String,
    agent_name: String,
    reason: WaitAgentReason,
    status: AgentStatus,
    message: Option<InterAgentCommunication>,
    waited_ms: i64,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult {
    WaitAgentToolResult {
        target,
        agent_name,
        reason,
        timed_out: matches!(reason, WaitAgentReason::Timeout),
        status,
        message_operation: message
            .as_ref()
            .map(|message| operation_name(message.operation).to_string()),
        message_author: message.as_ref().map(|message| message.author.to_string()),
        message_excerpt: message.map(|message| excerpt(&message.content)),
        waited_ms,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    }
}

fn operation_name(operation: protocol::protocol::InterAgentOperation) -> &'static str {
    match operation {
        protocol::protocol::InterAgentOperation::Unknown => "unknown",
        protocol::protocol::InterAgentOperation::SpawnAgent => "spawn_agent",
        protocol::protocol::InterAgentOperation::SendMessage => "send_message",
        protocol::protocol::InterAgentOperation::FollowupTask => "followup_task",
        protocol::protocol::InterAgentOperation::ChildCompletion => "child_completion",
    }
}

fn excerpt(content: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 160;
    let mut excerpt = content.chars().take(MAX_EXCERPT_CHARS).collect::<String>();
    if content.chars().count() > MAX_EXCERPT_CHARS {
        excerpt.push_str("...");
    }
    excerpt
}
