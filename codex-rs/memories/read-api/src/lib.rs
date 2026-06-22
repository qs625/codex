//! Lightweight API for read-path Codex memories.
//!
//! This crate owns memory citation parsing, the memory root path helper, and the
//! prompt-provider trait consumed by `codex-core`. Filesystem/template prompt
//! rendering stays in `codex-memories-read`.

pub mod citations;
pub mod provider;

use codex_utils_absolute_path::AbsolutePathBuf;

pub use provider::DisabledMemoryToolDeveloperInstructionsProvider;
pub use provider::MemoryReadFuture;
pub use provider::MemoryToolDeveloperInstructionsProvider;
pub use provider::SharedMemoryToolDeveloperInstructionsProvider;

pub fn memory_root(codex_home: &AbsolutePathBuf) -> AbsolutePathBuf {
    codex_home.join("memories")
}
