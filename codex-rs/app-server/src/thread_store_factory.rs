use std::sync::Arc;

use codex_core::config::Config;
use codex_core::config::ThreadStoreConfig;
use codex_rollout::StateDbHandle;
use codex_thread_store::InMemoryThreadStore;
use codex_thread_store::LocalThreadStore;
use codex_thread_store::LocalThreadStoreConfig;
use codex_thread_store::ThreadStore;

pub(crate) fn thread_store_from_config(
    config: &Config,
    state_db: Option<StateDbHandle>,
) -> Arc<dyn ThreadStore> {
    match &config.experimental_thread_store {
        ThreadStoreConfig::Local => Arc::new(LocalThreadStore::new(
            LocalThreadStoreConfig::from_config(config),
            state_db,
        )),
        ThreadStoreConfig::InMemory { id } => InMemoryThreadStore::for_id(id),
    }
}
