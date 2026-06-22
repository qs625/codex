//! Raw rollout trace identifiers and lightweight lifecycle enums.

use serde::Deserialize;
use serde::Serialize;

/// Codex conversation/session UUID.
pub type AgentThreadId = String;
/// Stable multi-agent routing path such as `/root` or `/root/search_docs`.
pub type AgentPath = String;
/// Runtime submission/activation UUID. This is not a chat turn.
pub type CodexTurnId = String;
/// Reduced transcript item ID assigned by the trace reducer.
pub type ConversationItemId = String;
/// Local ID for one outbound upstream inference request.
pub type InferenceCallId = String;
/// Globally unique ID for one concrete MCP backend request.
pub type McpCallId = String;
/// Reducer-owned ID for one runtime tool-call object.
pub type ToolCallId = String;
/// Responses `call_id` / custom-tool call ID visible in inference payloads.
pub type ModelVisibleCallId = String;
/// Tool invocation ID assigned inside the code-mode JavaScript runtime.
pub type CodeModeRuntimeToolId = String;
/// Reducer-owned ID for one model-authored `exec` JavaScript cell.
pub type CodeCellId = String;
/// Process/session ID returned by Codex's terminal runtime.
pub type TerminalId = String;
/// Reducer-owned ID for one command/write/poll operation against a terminal.
pub type TerminalOperationId = String;
/// Reducer-owned ID for one installed conversation-history checkpoint.
pub type CompactionId = String;
/// Reducer-owned ID for one upstream request that computes a compaction.
pub type CompactionRequestId = String;
/// Reducer-owned ID for one information-flow edge.
pub type EdgeId = String;
/// Reducer-owned ID for request/log correlation metadata.
pub type CorrelationId = String;

/// Coarse terminal status for the rollout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RolloutStatus {
    /// Writer has not seen a terminal rollout event.
    Running,
    /// Rollout ended normally.
    Completed,
    /// Rollout ended because an operation failed.
    Failed,
    /// Rollout was cancelled or otherwise stopped before normal completion.
    Aborted,
}

/// Coarse lifecycle status for a runtime object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Object is still live or the trace ended before its terminal event.
    Running,
    /// Object completed successfully.
    Completed,
    /// Object reached an error state.
    Failed,
    /// Object was cancelled by user/policy/runtime before completion.
    Cancelled,
    /// Object was aborted when its owner/runtime stopped.
    Aborted,
}

/// Code-mode runtime lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeCellRuntimeStatus {
    /// The `exec` request has been accepted but the runtime has not yet started user code.
    Starting,
    /// Runtime is executing JavaScript and has not yet yielded or terminated.
    Running,
    /// Initial `exec` returned while JavaScript kept running in the background.
    Yielded,
    /// Runtime reached a normal terminal result.
    Completed,
    /// Runtime reached an error terminal result.
    Failed,
    /// Runtime was explicitly terminated.
    Terminated,
}

/// Runtime tool category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ToolCallKind {
    ExecCommand,
    WriteStdin,
    ApplyPatch,
    Mcp {
        server: String,
        tool: String,
    },
    Web,
    ImageGeneration,
    SpawnAgent,
    AssignAgentTask,
    SendMessage,
    /// Multi-agent wait operation. Code-mode wait is modeled separately.
    WaitAgent,
    CloseAgent,
    Other {
        name: String,
    },
}

/// Bounded card/list summary for a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ToolCallSummary {
    /// Tool is summarized by its terminal operation.
    Terminal { operation_id: TerminalOperationId },
    Agent {
        target_agent_path: AgentPath,
        /// Task name/path segment when the operation creates or targets a task.
        task_name: Option<String>,
        message_preview: String,
    },
    WaitAgent {
        /// Wait target, when narrower than "any child".
        target_agent_path: Option<AgentPath>,
        timeout_ms: Option<u64>,
    },
    Generic {
        label: String,
        input_preview: Option<String>,
        output_preview: Option<String>,
    },
}
