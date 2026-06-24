use crate::ToolInvocationView;
use codex_tool_types::ToolInvocationMetadata;
use codex_tool_types::ToolPayload;
use std::ops::Deref;
use std::ops::DerefMut;
use tokio_util::sync::CancellationToken;

/// Runtime invocation envelope shared by tool router, registry, and handlers.
///
/// `Session`, `Turn`, and `Tracker` stay generic so host crates can provide
/// their own runtime state without making this crate depend on `codex-core`.
#[derive(Clone)]
pub struct ToolInvocation<Session, Turn, Tracker> {
    pub session: Session,
    pub turn: Turn,
    pub cancellation_token: CancellationToken,
    pub tracker: Tracker,
    pub metadata: ToolInvocationMetadata,
}

impl<Session, Turn, Tracker> ToolInvocationView for ToolInvocation<Session, Turn, Tracker> {
    fn call_id(&self) -> &str {
        &self.metadata.call_id
    }

    fn tool_name(&self) -> &codex_tool_types::ToolName {
        &self.metadata.tool_name
    }

    fn payload(&self) -> &ToolPayload {
        &self.metadata.payload
    }
}

impl<Session, Turn, Tracker> Deref for ToolInvocation<Session, Turn, Tracker> {
    type Target = ToolInvocationMetadata;

    fn deref(&self) -> &Self::Target {
        &self.metadata
    }
}

impl<Session, Turn, Tracker> DerefMut for ToolInvocation<Session, Turn, Tracker> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.metadata
    }
}
