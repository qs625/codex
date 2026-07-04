//! Reduced rollout trace model.
//!
//! These types describe the deterministic replay output. They intentionally
//! separate model-visible conversation from runtime/debug objects.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use crate::payload::RawPayloadId;
use crate::payload::RawPayloadRef;
mod conversation;
mod runtime;
mod session;

pub use conversation::*;
pub use rollout_trace_api::model::AgentPath;
pub use rollout_trace_api::model::AgentThreadId;
pub use rollout_trace_api::model::CodeCellId;
pub use rollout_trace_api::model::CodeCellRuntimeStatus;
pub use rollout_trace_api::model::CodeModeRuntimeToolId;
pub use rollout_trace_api::model::CodexTurnId;
pub use rollout_trace_api::model::CompactionId;
pub use rollout_trace_api::model::CompactionRequestId;
pub use rollout_trace_api::model::ConversationItemId;
pub use rollout_trace_api::model::CorrelationId;
pub use rollout_trace_api::model::EdgeId;
pub use rollout_trace_api::model::ExecutionStatus;
pub use rollout_trace_api::model::InferenceCallId;
pub use rollout_trace_api::model::McpCallId;
pub use rollout_trace_api::model::ModelVisibleCallId;
pub use rollout_trace_api::model::RolloutStatus;
pub use rollout_trace_api::model::TerminalId;
pub use rollout_trace_api::model::TerminalOperationId;
pub use rollout_trace_api::model::ToolCallId;
pub use rollout_trace_api::model::ToolCallKind;
pub use rollout_trace_api::model::ToolCallSummary;
pub use runtime::*;
pub use session::*;

/// Canonical reduced graph for one Codex rollout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutTrace {
    pub schema_version: u32,
    /// Unique identity for this trace capture.
    ///
    /// `rollout_id` names the Codex rollout/session being observed. `trace_id`
    /// names the diagnostic artifact produced for that rollout, which keeps
    /// storage/replay identity separate from the product-level session identity.
    pub trace_id: String,
    /// CLI-visible rollout/run identity. Higher-level experiment/sample IDs wrap this object.
    pub rollout_id: String,
    pub started_at_unix_ms: i64,
    /// Wall-clock timestamp for terminal rollout status. `None` means running or partial trace.
    pub ended_at_unix_ms: Option<i64>,
    pub status: RolloutStatus,
    pub root_thread_id: AgentThreadId,
    pub threads: BTreeMap<AgentThreadId, AgentThread>,
    pub codex_turns: BTreeMap<CodexTurnId, CodexTurn>,
    pub conversation_items: BTreeMap<ConversationItemId, ConversationItem>,
    pub inference_calls: BTreeMap<InferenceCallId, InferenceCall>,
    /// Model-authored `exec` JavaScript cells keyed by reducer-owned cell ID.
    pub code_cells: BTreeMap<CodeCellId, CodeCell>,
    pub tool_calls: BTreeMap<ToolCallId, ToolCall>,
    /// Terminal runtime sessions keyed by process/session ID returned by the runtime.
    pub terminal_sessions: BTreeMap<TerminalId, TerminalSession>,
    /// Commands/writes/polls against terminals keyed by reducer-owned operation ID.
    pub terminal_operations: BTreeMap<TerminalOperationId, TerminalOperation>,
    /// Installed compaction checkpoints keyed by checkpoint ID.
    pub compactions: BTreeMap<CompactionId, Compaction>,
    /// Upstream remote compaction calls keyed by local request ID.
    pub compaction_requests: BTreeMap<CompactionRequestId, CompactionRequest>,
    /// Information-flow edges between threads, cells, tools, and runtime resources.
    pub interaction_edges: BTreeMap<EdgeId, InteractionEdge>,
    /// Raw JSON payloads keyed by raw-payload ID. Most point at files outside this object.
    pub raw_payloads: BTreeMap<RawPayloadId, RawPayloadRef>,
}

impl RolloutTrace {
    /// Builds an empty reduced trace that a reducer can populate.
    pub(crate) fn new(
        schema_version: u32,
        trace_id: String,
        rollout_id: String,
        root_thread_id: AgentThreadId,
        started_at_unix_ms: i64,
    ) -> Self {
        Self {
            schema_version,
            trace_id,
            rollout_id,
            started_at_unix_ms,
            ended_at_unix_ms: None,
            status: RolloutStatus::Running,
            root_thread_id,
            threads: BTreeMap::new(),
            codex_turns: BTreeMap::new(),
            conversation_items: BTreeMap::new(),
            inference_calls: BTreeMap::new(),
            code_cells: BTreeMap::new(),
            tool_calls: BTreeMap::new(),
            terminal_sessions: BTreeMap::new(),
            terminal_operations: BTreeMap::new(),
            compactions: BTreeMap::new(),
            compaction_requests: BTreeMap::new(),
            interaction_edges: BTreeMap::new(),
            raw_payloads: BTreeMap::new(),
        }
    }
}
