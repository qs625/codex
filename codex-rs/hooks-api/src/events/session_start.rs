use std::path::PathBuf;

use codex_protocol::ThreadId;
use codex_protocol::protocol::HookCompletedEvent;
use codex_utils_absolute_path::AbsolutePathBuf;

#[derive(Debug, Clone, Copy)]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
}

impl SessionStartSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Resume => "resume",
            Self::Clear => "clear",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionStartRequest {
    pub session_id: ThreadId,
    pub cwd: AbsolutePathBuf,
    pub transcript_path: Option<PathBuf>,
    pub model: String,
    pub permission_mode: String,
    pub source: SessionStartSource,
}

#[derive(Debug)]
pub struct SessionStartOutcome {
    pub hook_events: Vec<HookCompletedEvent>,
    pub should_stop: bool,
    pub stop_reason: Option<String>,
    pub additional_contexts: Vec<String>,
}
