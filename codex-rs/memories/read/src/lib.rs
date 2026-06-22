//! Read-path helpers for Codex memories.
//!
//! This crate owns memory injection and memory citation parsing for read access
//! to the memory folder. It intentionally does not depend on the memory write
//! pipeline.

mod prompts;

pub use codex_memories_read_api::DisabledMemoryToolDeveloperInstructionsProvider;
pub use codex_memories_read_api::MemoryReadFuture;
pub use codex_memories_read_api::MemoryToolDeveloperInstructionsProvider;
pub use codex_memories_read_api::SharedMemoryToolDeveloperInstructionsProvider;
pub use codex_memories_read_api::citations;
pub use codex_memories_read_api::memory_root;
pub use prompts::FsMemoryToolDeveloperInstructionsProvider;
pub use prompts::build_memory_tool_developer_instructions;

const MEMORY_TOOL_DEVELOPER_INSTRUCTIONS_SUMMARY_TOKEN_LIMIT: usize = 5_000;
