use crate::FunctionCallError;
use crate::ToolName;
use crate::ToolOutput;
use crate::ToolSpec;
use std::future::Future;
use std::pin::Pin;

/// Controls where a tool is exposed to the model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    /// Include this tool in the initial model-visible tool list.
    ///
    /// When code mode is enabled, this tool is also available as a nested
    /// code-mode tool.
    Direct,

    /// Register this tool for later discovery, but omit it from the initial
    /// model-visible tool list.
    Deferred,

    /// Include this tool in the initial model-visible tool list only.
    ///
    /// In code-mode-only sessions, this keeps the tool callable as a normal
    /// model tool while excluding it from the nested code-mode tool surface.
    DirectModelOnly,
}

impl ToolExposure {
    pub fn is_direct(self) -> bool {
        matches!(self, Self::Direct | Self::DirectModelOnly)
    }
}

/// Shared runtime contract for model-visible tools.
///
/// Implementations keep the model-visible spec tied to the executable runtime.
/// Host crates can layer routing, hooks, telemetry, or other orchestration on
/// top without reopening the spec/runtime split.
pub trait ToolExecutor<Invocation>: Send + Sync {
    type Output: ToolOutput + 'static;

    /// The concrete tool name handled by this runtime instance.
    fn tool_name(&self) -> ToolName;

    fn spec(&self) -> Option<ToolSpec> {
        None
    }

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }

    /// Executes one tool invocation.
    ///
    /// The boxed future keeps this trait object-safe for extension executors
    /// and host registries without requiring consumers to depend on the
    /// `async-trait` proc macro.
    fn handle<'a>(&'a self, invocation: Invocation) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
        Invocation: 'a;
}

/// Object-safe future returned by [`ToolExecutor::handle`].
pub type ToolExecutorFuture<'a, Output> =
    Pin<Box<dyn Future<Output = Result<Output, FunctionCallError>> + Send + 'a>>;
