use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::items::HookPromptFragment;
use codex_protocol::protocol::HookCompletedEvent;
use codex_utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone)]
pub struct StopRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
    pub stop_hook_active: bool,
    pub last_assistant_message: Option<String>,
}

#[derive(Debug)]
pub struct StopOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
    pub should_block: bool,
    pub block_reason: Option<String>,
    pub continuation_fragments: Vec<HookPromptFragment>,
}
