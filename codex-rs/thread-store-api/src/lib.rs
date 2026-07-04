//! Storage-neutral thread persistence API.
//!
//! This crate owns the thread-store trait, error type, and DTOs. Concrete
//! implementations such as local JSONL/state-db storage live in
//! `thread-store`.

mod error;
mod live;
mod store;
mod types;

pub use error::ThreadStoreError;
pub use error::ThreadStoreResult;
pub use live::LiveThreadFactory;
pub use live::LiveThreadHandle;
pub use live::SharedLiveThread;
pub use live::ThreadStoreFuture;
pub use store::ThreadStore;
pub use types::AppendThreadItemsParams;
pub use types::ArchiveThreadParams;
pub use types::ClearableField;
pub use types::CreateThreadParams;
pub use types::GitInfoPatch;
pub use types::ItemPage;
pub use types::ListItemsParams;
pub use types::ListThreadsParams;
pub use types::ListTurnsParams;
pub use types::LoadThreadHistoryParams;
pub use types::ReadThreadByRolloutPathParams;
pub use types::ReadThreadParams;
pub use types::ResumeThreadParams;
pub use types::SortDirection;
pub use types::StoredThread;
pub use types::StoredThreadHistory;
pub use types::StoredTurn;
pub use types::StoredTurnError;
pub use types::StoredTurnItemsView;
pub use types::StoredTurnStatus;
pub use types::ThreadEventPersistenceMode;
pub use types::ThreadMetadataPatch;
pub use types::ThreadPage;
pub use types::ThreadPersistenceMetadata;
pub use types::ThreadSortKey;
pub use types::TurnPage;
pub use types::UpdateThreadMetadataParams;
