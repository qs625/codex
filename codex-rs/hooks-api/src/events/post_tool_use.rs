use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::ThreadId;
use protocol::protocol::HookCompletedEvent;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PostToolUseRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
    pub tool_name: String,
    pub matcher_aliases: Vec<String>,
    pub tool_use_id: String,
    pub tool_input: Value,
    pub tool_response: Value,
}

#[derive(Debug)]
pub struct PostToolUseOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
    pub additional_contexts: Vec<String>,
    pub feedback_message: Option<String>,
}
