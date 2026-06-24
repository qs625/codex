//! Core-independent live thread operation API.
//!
//! `codex-thread-store-api` owns persisted thread storage. This crate owns the
//! live runtime handle traits used to drive, inspect, and shut down active
//! threads without depending on `codex-core`.

use std::future::Future;
use std::path::PathBuf;

use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::Op;
use codex_session_api::SessionCommandHandle;

/// Canonical coarse runtime state for a live thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadRuntimeStatus {
    Active,
    IdleWaitCommand,
    IdleWaitChild,
    Complete,
}

/// Lightweight identity metadata for a loaded live thread.
///
/// This intentionally excludes config snapshots, stores, and runtime services so
/// app/server consumers can avoid depending on concrete core thread types for
/// basic identity checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveThreadInfo {
    pub session_id: SessionId,
    pub rollout_path: Option<PathBuf>,
}

/// Minimal command/status surface for a live thread.
///
/// Implementations should delegate to the underlying session runtime instead of
/// reimplementing turn or pending-input behavior at the thread layer.
pub trait LiveThreadHandle: SessionCommandHandle + Send + Sync {
    /// Submit an operation to this thread's session.
    fn submit_thread_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Return the latest agent lifecycle status for this thread.
    fn agent_status(&self) -> impl Future<Output = AgentStatus> + Send + '_;

    /// Return the coarse runtime status used by thread tree clients.
    fn runtime_thread_status(&self) -> impl Future<Output = ThreadRuntimeStatus> + Send + '_;

    /// Request shutdown and wait until the runtime has terminated.
    fn shutdown_and_wait(&self) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Wait until the runtime terminates without issuing a shutdown request.
    fn wait_until_terminated(&self) -> impl Future<Output = ()> + Send + '_;
}

/// Lookup and command surface for a collection of live threads.
pub trait LiveThreadRegistry: Send + Sync {
    /// List all live thread ids known to this registry.
    fn list_thread_ids(&self) -> impl Future<Output = Vec<ThreadId>> + Send + '_;

    /// Return whether a live thread is currently loaded in this registry.
    fn is_thread_loaded(&self, thread_id: ThreadId) -> impl Future<Output = bool> + Send + '_;

    /// Return lightweight identity metadata for a loaded live thread.
    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<LiveThreadInfo>> + Send + '_;

    /// Submit an operation to a specific live thread.
    fn send_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Append a model-visible conversation item to a specific live thread.
    fn append_thread_conversation_item(
        &self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Return the latest agent lifecycle status for a specific live thread.
    fn thread_agent_status(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<AgentStatus>> + Send + '_;

    /// Return the coarse runtime status for a specific live thread.
    fn thread_runtime_status(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<ThreadRuntimeStatus>> + Send + '_;

    /// Request shutdown for a specific live thread and wait until it terminates.
    fn shutdown_thread_and_wait(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Wait until a specific live thread terminates without issuing shutdown.
    fn wait_thread_until_terminated(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;
}
