//! Trace bundle manifest and local layout constants.

use serde::Deserialize;
use serde::Serialize;

use crate::model::AgentThreadId;

pub const MANIFEST_FILE_NAME: &str = "manifest.json";
pub const RAW_EVENT_LOG_FILE_NAME: &str = "trace.jsonl";
pub const PAYLOADS_DIR_NAME: &str = "payloads";
/// Conventional file name for a reducer-written `RolloutTrace` cache.
pub const REDUCED_STATE_FILE_NAME: &str = "state.json";
pub const TRACE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const REDUCED_TRACE_SCHEMA_VERSION: u32 = 1;

/// Manifest stored at the root of a trace bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBundleManifest {
    pub schema_version: u32,
    pub trace_id: String,
    pub rollout_id: String,
    /// Root thread for the recorded rollout. Replay should fail rather than
    /// inventing a placeholder, because every reduced object is scoped back to
    /// this thread tree.
    pub root_thread_id: AgentThreadId,
    pub started_at_unix_ms: i64,
    pub raw_event_log: String,
    pub payloads_dir: String,
}

impl TraceBundleManifest {
    /// Builds a manifest that uses the standard local bundle layout.
    pub fn new(
        trace_id: String,
        rollout_id: String,
        root_thread_id: AgentThreadId,
        started_at_unix_ms: i64,
    ) -> Self {
        Self {
            schema_version: TRACE_MANIFEST_SCHEMA_VERSION,
            trace_id,
            rollout_id,
            root_thread_id,
            started_at_unix_ms,
            raw_event_log: RAW_EVENT_LOG_FILE_NAME.to_string(),
            payloads_dir: PAYLOADS_DIR_NAME.to_string(),
        }
    }
}
