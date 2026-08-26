//! Rollout persistence and discovery for Codex session files.

use std::sync::LazyLock;

use protocol::protocol::SessionSource;

pub(crate) mod config;
pub(crate) mod layout;
pub(crate) mod list;
pub(crate) mod metadata;
pub(crate) mod policy;
pub(crate) mod recorder;
pub(crate) mod session_index;
mod sqlite_metrics;
pub mod state_db;

pub(crate) mod default_client {
    pub use transport_client_identity::*;
}

pub(crate) use protocol::protocol as rollout_protocol;

pub use rollout_api::ARCHIVED_SESSIONS_SUBDIR;
pub use rollout_api::SESSIONS_SUBDIR;
pub static INTERACTIVE_SESSION_SOURCES: LazyLock<Vec<SessionSource>> = LazyLock::new(|| {
    vec![
        SessionSource::Cli,
        SessionSource::VSCode,
        SessionSource::Custom("atlas".to_string()),
        SessionSource::Custom("chatgpt".to_string()),
    ]
});

pub use config::Config;
pub use config::RolloutConfig;
pub use config::RolloutConfigView;
pub use layout::rollout_container_path;
pub use layout::rollout_path_after_moving_container;
pub use list::Cursor;
pub use list::SortDirection;
pub use list::ThreadItem;
pub use list::ThreadListConfig;
pub use list::ThreadListLayout;
pub use list::ThreadSortKey;
pub use list::ThreadsPage;
pub use list::find_archived_thread_path_by_id_str;
pub use list::find_thread_path_by_id_str;
#[deprecated(note = "use find_thread_path_by_id_str")]
pub use list::find_thread_path_by_id_str as find_conversation_path_by_id_str;
pub use list::get_threads;
pub use list::get_threads_in_root;
pub use list::parse_cursor;
pub use list::read_head_for_summary;
pub use list::read_session_meta_line;
pub use list::read_thread_item_from_rollout;
pub use list::rollout_date_parts;
pub use metadata::builder_from_items;
pub use policy::EventPersistenceMode;
pub use policy::is_persisted_rollout_item;
pub use policy::persisted_rollout_items;
pub use policy::should_persist_response_item_for_memories;
pub use protocol::protocol::SessionMeta;
pub use recorder::RolloutRecorder;
pub use recorder::RolloutRecorderParams;
pub use recorder::append_rollout_item_to_path;
pub use recorder::resolve_current_segment_path;
pub use recorder::segment_manifest_path_for_rollout;
pub use recorder::segmented_compaction_count_for_rollout_path;
pub use session_index::append_thread_name;
pub use session_index::find_thread_meta_by_name_str;
pub use session_index::find_thread_name_by_id;
pub use session_index::find_thread_names_by_ids;
pub use state_db::StateDbHandle;
pub use state_db::sqlite_telemetry_recorder;

#[cfg(test)]
mod tests;
