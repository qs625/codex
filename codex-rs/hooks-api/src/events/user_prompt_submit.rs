use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::ThreadId;
use protocol::protocol::HookCompletedEvent;

#[derive(Debug, Clone)]
pub struct UserPromptSubmitRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
    pub prompt: String,
}

#[derive(Debug)]
pub struct UserPromptSubmitOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
    pub additional_contexts: Vec<String>,
}
