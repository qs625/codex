use super::ContextualUserFragment;
use protocol::protocol::REALTIME_CONVERSATION_CLOSE_TAG;
use protocol::protocol::REALTIME_CONVERSATION_OPEN_TAG;

const REALTIME_END_INSTRUCTIONS: &str = include_str!("prompts/realtime/realtime_end.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeEndInstructions {
    reason: String,
}

impl RealtimeEndInstructions {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl ContextualUserFragment for RealtimeEndInstructions {
    const ROLE: &'static str = "developer";
    const START_MARKER: &'static str = REALTIME_CONVERSATION_OPEN_TAG;
    const END_MARKER: &'static str = REALTIME_CONVERSATION_CLOSE_TAG;

    fn body(&self) -> String {
        format!(
            "\n{}\n\nReason: {}\n",
            REALTIME_END_INSTRUCTIONS.trim(),
            self.reason
        )
    }
}
