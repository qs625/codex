use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::turn_diff_tracker::TurnDiffTracker;
use std::sync::Arc;
use tokio::sync::Mutex;

pub use codex_tool_planning::ToolCallSource;
pub use codex_tool_planning::ToolInvocationMetadata;
pub use codex_tool_planning::ToolOutput;
pub use codex_tool_planning::ToolPayload;
pub use codex_tool_runtime::ExecCommandToolOutput;
#[cfg(test)]
pub use codex_tool_runtime::FunctionToolOutput;

pub type SharedTurnDiffTracker = Arc<Mutex<TurnDiffTracker>>;
pub type ToolInvocation =
    codex_tool_runtime::ToolInvocation<Arc<Session>, Arc<TurnContext>, SharedTurnDiffTracker>;
