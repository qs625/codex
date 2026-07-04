use std::path::PathBuf;

use protocol::ThreadId;
use protocol::protocol::HookCompletedEvent;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PermissionRequestRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: PathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
    pub tool_name: String,
    pub matcher_aliases: Vec<String>,
    pub run_id_suffix: String,
    pub tool_input: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRequestDecision {
    Allow,
    Deny { message: String },
}

#[derive(Debug)]
pub struct PermissionRequestOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub decision: Option<PermissionRequestDecision>,
}
