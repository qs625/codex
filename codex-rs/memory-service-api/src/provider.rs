use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_utils_absolute_path::AbsolutePathBuf;

pub type MemoryReadFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Provides memory-tool developer instructions to thread/app composition roots.
///
/// Implementations own any filesystem, template, or remote lookup needed to
/// build the prompt without forcing callers to depend on the concrete memory
/// read-path implementation.
pub trait MemoryToolDeveloperInstructionsProvider: Send + Sync {
    fn build_memory_tool_developer_instructions<'a>(
        &'a self,
        codex_home: &'a AbsolutePathBuf,
    ) -> MemoryReadFuture<'a, Option<String>>;
}

pub type SharedMemoryToolDeveloperInstructionsProvider =
    Arc<dyn MemoryToolDeveloperInstructionsProvider>;

#[derive(Debug, Default)]
pub struct DisabledMemoryToolDeveloperInstructionsProvider;

impl MemoryToolDeveloperInstructionsProvider for DisabledMemoryToolDeveloperInstructionsProvider {
    fn build_memory_tool_developer_instructions<'a>(
        &'a self,
        _codex_home: &'a AbsolutePathBuf,
    ) -> MemoryReadFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
}
