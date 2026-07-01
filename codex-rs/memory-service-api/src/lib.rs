//! Cross-crate API contracts for the memory service.
//!
//! This crate owns memory citation parsing, memory path helpers, prompt-provider
//! contracts, and startup/consolidation contracts consumed by composition roots.
//! Filesystem, MCP, and pipeline implementations stay in `memory-service`.

pub mod citations;
pub mod provider;
pub mod startup;

use codex_utils_absolute_path::AbsolutePathBuf;

pub use provider::DisabledMemoryToolDeveloperInstructionsProvider;
pub use provider::MemoryReadFuture;
pub use provider::MemoryToolDeveloperInstructionsProvider;
pub use provider::SharedMemoryToolDeveloperInstructionsProvider;
pub use startup::MemoryConsolidationAgent;
pub use startup::MemoryRuntimeFuture;
pub use startup::MemoryStartupRuntime;
pub use startup::MemoryStartupSettings;
pub use startup::StageOnePromptRequest;
pub use startup::StageOneRequestContext;

pub fn memory_root(codex_home: &AbsolutePathBuf) -> AbsolutePathBuf {
    codex_home.join("memories")
}
