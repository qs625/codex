use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::ThreadMemoryMode;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::CreateThreadParams;
use crate::ResumeThreadParams;
use crate::StoredThread;
use crate::StoredThreadHistory;
use crate::ThreadMetadataPatch;
use crate::ThreadStore;
use crate::ThreadStoreResult;

pub type ThreadStoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type SharedLiveThread = Arc<dyn LiveThreadHandle>;

/// Session-facing handle for an active persisted thread.
///
/// Implementations own live writer details such as local rollout files, remote
/// stores, and metadata synchronization. Session/runtime code should use this
/// boundary instead of depending on a concrete thread-store implementation.
pub trait LiveThreadHandle: Send + Sync {
    fn append_items<'a>(
        &'a self,
        items: &'a [RolloutItem],
    ) -> ThreadStoreFuture<'a, ThreadStoreResult<()>>;

    fn persist(&self) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    fn flush(&self) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    fn shutdown(&self) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    fn discard(&self) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    fn load_history(
        &self,
        include_archived: bool,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThreadHistory>>;

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;

    fn update_memory_mode(
        &self,
        mode: ThreadMemoryMode,
        include_archived: bool,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<()>>;

    fn update_metadata(
        &self,
        patch: ThreadMetadataPatch,
        include_archived: bool,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<StoredThread>>;

    fn local_rollout_path(&self) -> ThreadStoreFuture<'_, ThreadStoreResult<Option<PathBuf>>>;
}

/// Factory for opening active thread persistence handles.
///
/// Composition roots inject this factory so session/runtime code can create or
/// resume a live thread without depending on the concrete local/in-memory store
/// implementation crate.
pub trait LiveThreadFactory: Send + Sync {
    fn create(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        params: CreateThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<SharedLiveThread>>;

    fn resume(
        &self,
        thread_store: Arc<dyn ThreadStore>,
        params: ResumeThreadParams,
    ) -> ThreadStoreFuture<'_, ThreadStoreResult<SharedLiveThread>>;
}
