//! Storage-neutral thread persistence interfaces.
//!
//! Application code should treat [`codex_protocol::ThreadId`] as the only durable thread handle.
//! Implementations are responsible for resolving that id to local rollout files, RPC requests, or
//! any other backing store.

mod in_memory;
mod live_thread;
mod local;
mod thread_metadata_sync;

pub use codex_thread_store_api::*;
pub use in_memory::InMemoryThreadStore;
pub use in_memory::InMemoryThreadStoreCalls;
pub use live_thread::DefaultLiveThreadFactory;
pub use live_thread::LiveThread;
pub use live_thread::LiveThreadInitGuard;
pub use local::LocalThreadStore;
pub use local::LocalThreadStoreConfig;
