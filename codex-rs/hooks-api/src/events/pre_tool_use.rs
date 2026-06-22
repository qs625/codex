use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::protocol::HookCompletedEvent;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct PreToolUseRequest {
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
}

#[derive(Debug)]
pub struct PreToolUseOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_block: bool,
    pub block_reason: Option<String>,
    pub additional_contexts: Vec<String>,
    pub updated_input: Option<Value>,
}
