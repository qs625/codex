use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::protocol::HookCompletedEvent;
use codex_utils_absolute_path::AbsolutePathBuf;

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
