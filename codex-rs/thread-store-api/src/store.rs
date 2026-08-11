use protocol::ThreadId;
use protocol::subscriptions::PersistedSubscription;
use std::any::Any;

use crate::AppendThreadItemsParams;
use crate::ArchiveThreadParams;
use crate::CreateThreadParams;
use crate::ItemPage;
use crate::ListItemsParams;
use crate::ListThreadsParams;
use crate::ListTurnsParams;
use crate::LoadThreadHistoryParams;
use crate::ReadThreadByRolloutPathParams;
use crate::ReadThreadParams;
use crate::ResumeThreadParams;
use crate::StoredThread;
use crate::StoredThreadHistory;
use crate::ThreadPage;
use crate::ThreadStoreError;
use crate::ThreadStoreFuture;
use crate::ThreadStoreResult;
use crate::TurnPage;
use crate::UpdateThreadMetadataParams;

/// Storage-neutral thread persistence boundary.
pub trait ThreadStore: Any + Send + Sync {
    /// Return this store as [`Any`] for implementation-owned escape hatches.
    fn as_any(&self) -> &dyn Any;

    /// Creates a new live thread.
    fn create_thread(
        &self,
        params: CreateThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Reopens an existing thread for live appends.
    fn resume_thread(
        &self,
        params: ResumeThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Appends canonical rollout items to a live thread.
    ///
    /// This is the raw history API. It does not infer metadata from item contents. Callers that
    /// need metadata updates should call [`ThreadStore::update_thread_metadata`] with explicit
    /// metadata facts prepared above the store.
    fn append_items(
        &self,
        params: AppendThreadItemsParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Materializes the thread if persistence is lazy, then persists all queued items.
    fn persist_thread(&self, thread_id: ThreadId) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Flushes all queued items and returns once they are durable/readable.
    fn flush_thread(&self, thread_id: ThreadId) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Flushes pending items and closes the live thread writer.
    fn shutdown_thread(&self, thread_id: ThreadId) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Discards the live thread writer without forcing pending in-memory items to become durable.
    ///
    /// Core calls this when session initialization fails after a live writer has been created.
    /// Implementations should release any live writer resources for the thread while preserving
    /// already-durable thread data.
    fn discard_thread(&self, thread_id: ThreadId) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Loads persisted history for resume, fork, rollback, and memory jobs.
    fn load_history(
        &self,
        params: LoadThreadHistoryParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThreadHistory>>;

    /// Reads a thread summary and optionally its persisted history.
    fn read_thread(
        &self,
        params: ReadThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;

    /// Reads a rollout-backed thread by path when the store supports path-addressed lookups.
    ///
    /// Deprecated: new callers should use [`ThreadStore::read_thread`] instead.
    fn read_thread_by_rollout_path(
        &self,
        params: ReadThreadByRolloutPathParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;

    /// Lists stored threads matching the supplied filters.
    fn list_threads(
        &self,
        params: ListThreadsParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<ThreadPage>>;

    /// Lists turns within a stored thread.
    fn list_turns(
        &self,
        _params: ListTurnsParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<TurnPage>> {
        Box::pin(async move {
            Err(ThreadStoreError::Unsupported {
                operation: "list_turns",
            })
        })
    }

    /// Lists persisted items within a stored turn.
    fn list_items(
        &self,
        _params: ListItemsParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<ItemPage>> {
        Box::pin(async move {
            Err(ThreadStoreError::Unsupported {
                operation: "list_items",
            })
        })
    }

    /// Applies a literal metadata patch and returns the updated thread.
    ///
    /// Implementations should apply the supplied fields directly. Policy such as deciding whether
    /// an append-derived preview should be emitted belongs above the store.
    fn update_thread_metadata(
        &self,
        params: UpdateThreadMetadataParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;

    /// Reads the latest persisted subscription snapshot for a thread.
    ///
    /// `Some(vec)` means a current-state snapshot exists, including `Some(Vec::new())`
    /// for an explicitly cleared subscription set. `None` means the store has no
    /// current-state snapshot and callers may choose a legacy fallback.
    fn read_thread_subscriptions(
        &self,
        _thread_id: ThreadId,
        _include_archived: bool,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<Option<Vec<PersistedSubscription>>>> {
        Box::pin(async move {
            Err(ThreadStoreError::Unsupported {
                operation: "read_thread_subscriptions",
            })
        })
    }

    /// Lists active threads whose current-state subscription snapshot is non-empty.
    fn list_thread_ids_with_active_subscriptions(
        &self,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<Vec<ThreadId>>> {
        Box::pin(async move {
            Err(ThreadStoreError::Unsupported {
                operation: "list_thread_ids_with_active_subscriptions",
            })
        })
    }

    /// Archives a thread.
    fn archive_thread(
        &self,
        params: ArchiveThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    /// Unarchives a thread and returns its updated metadata.
    fn unarchive_thread(
        &self,
        params: ArchiveThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;
}
