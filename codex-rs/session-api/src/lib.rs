//! Core-independent live session operation API.
//!
//! This crate owns the narrow trait surface that callers need to drive an
//! existing session. Concrete session loop implementations live in runtime
//! crates and implement these traits by adapting their internal state.

use std::future::Future;

use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::Submission;

/// Minimal command surface for an already-created live session.
///
/// Implementations are expected to enqueue operations onto the session's normal
/// turn loop. They should not bypass pending-input hooks, lifecycle events, or
/// status transitions owned by the concrete runtime.
pub trait SessionCommandHandle: Send + Sync {
    /// Submit a high-level operation and let the runtime assign the submission id.
    fn submit_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Submit a prebuilt submission with a caller-provided id.
    fn submit_with_id(
        &self,
        submission: Submission,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Request shutdown through the normal session operation queue.
    fn shutdown(&self) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Append a model-visible conversation item outside the normal user-input path.
    ///
    /// Implementations should record the item through the same history/context
    /// path used by the live session runtime, including any display/event
    /// projection side effects owned by that runtime.
    fn append_conversation_item(
        &self,
        item: ResponseItem,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;
}

/// Read-only live status surface for a session.
pub trait SessionStatusHandle: Send + Sync {
    /// Return the latest lifecycle status observed by the session runtime.
    fn agent_status(&self) -> impl Future<Output = AgentStatus> + Send + '_;
}
