//! Trace bundle format, writer, and reducer for Codex rollouts.
//!
//! This crate owns the trace schema. Hot-path Codex code should depend on the
//! small writer API here; semantic replay and viewer projections stay outside
//! `codex-core`.
//!
//! See `README.md` for the system diagram and reducer model.

mod model;
mod reducer;

pub mod bundle {
    pub use rollout_trace_api::bundle::*;
}

pub mod payload {
    pub use rollout_trace_api::payload::*;
}

pub mod raw_event {
    pub use rollout_trace_api::raw_event::*;
}

pub mod writer {
    pub use rollout_trace_api::writer::*;
}

/// Public reduced trace model returned by replay.
pub use model::*;
/// Replay a raw trace bundle and write/read its reduced `RolloutTrace`.
pub use reducer::replay_bundle;
/// Hot-path trace schema and writer API. Re-exported for compatibility; new
/// runtime code should depend on `rollout-trace-api` directly.
pub use rollout_trace_api::AgentResultTracePayload;
pub use rollout_trace_api::CODEX_ROLLOUT_TRACE_ROOT_ENV;
pub use rollout_trace_api::CodeCellTraceContext;
pub use rollout_trace_api::CompactionCheckpointTracePayload;
pub use rollout_trace_api::CompactionTraceAttempt;
pub use rollout_trace_api::CompactionTraceContext;
pub use rollout_trace_api::InferenceTraceAttempt;
pub use rollout_trace_api::InferenceTraceContext;
pub use rollout_trace_api::McpCallTraceContext;
pub use rollout_trace_api::REDUCED_STATE_FILE_NAME;
pub use rollout_trace_api::RawEventSeq;
pub use rollout_trace_api::RawPayloadId;
pub use rollout_trace_api::RawPayloadKind;
pub use rollout_trace_api::RawPayloadRef;
pub use rollout_trace_api::RawToolCallRequester;
pub use rollout_trace_api::RawTraceEvent;
pub use rollout_trace_api::RawTraceEventContext;
pub use rollout_trace_api::RawTraceEventPayload;
pub use rollout_trace_api::ThreadStartedTraceMetadata;
pub use rollout_trace_api::ThreadTraceContext;
pub use rollout_trace_api::ToolDispatchInvocation;
pub use rollout_trace_api::ToolDispatchPayload;
pub use rollout_trace_api::ToolDispatchRequester;
pub use rollout_trace_api::ToolDispatchResult;
pub use rollout_trace_api::ToolDispatchTraceContext;
pub use rollout_trace_api::TraceWriter;
