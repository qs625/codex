use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::ThreadId;
use protocol::protocol::HookCompletedEvent;

#[derive(Debug, Clone)]
pub struct PreCompactRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub trigger: String,
}

#[derive(Debug, Clone)]
pub struct PostCompactRequest {
    pub session_id: ThreadId,
    pub turn_id: String,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub trigger: String,
}

#[derive(Debug)]
pub struct StatelessHookOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
}

#[derive(Debug)]
pub struct PreCompactOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
}
