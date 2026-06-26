use std::future::Future;
use std::pin::Pin;
use std::sync::Weak;

use codex_thread_api::SharedToolTurnDiffTracker;
use codex_thread_api::ToolServiceSessionRef;
use codex_thread_api::ToolServiceTurnRef;
use codex_thread_api::ToolSessionCapability;
use codex_tool_config::ToolsConfig;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_runtime_api::ToolArgumentDiffConsumer;
use codex_tool_runtime_api::ToolServiceParams;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolSpec;
use tokio_util::sync::CancellationToken;

/// Boxed future returned by object-safe tool service APIs.
pub type ToolServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ErasedToolArgumentDiffConsumer: Send {
    fn consume_diff(
        &mut self,
        turn: &dyn ToolServiceTurnRef,
        call_id: String,
        diff: &str,
    ) -> Option<codex_protocol::protocol::EventMsg>;

    fn finish(&mut self) -> Result<Option<codex_protocol::protocol::EventMsg>, FunctionCallError> {
        Ok(None)
    }
}

pub struct ToolSpecRequest<'a> {
    pub config: &'a ToolsConfig,
    pub session_capability: Weak<dyn ToolSessionCapability>,
    pub session: std::sync::Arc<dyn ToolServiceSessionRef>,
    pub turn: std::sync::Arc<dyn ToolServiceTurnRef>,
    pub params: ToolServiceParams<'a>,
}

pub struct ToolDiffConsumerRequest<'a> {
    pub tool: ToolSpecRequest<'a>,
    pub tool_name: &'a ToolName,
}

pub struct ToolParallelRequest<'a> {
    pub tool: ToolSpecRequest<'a>,
    pub call: &'a ToolCall,
}

pub struct ToolDispatchRequest<'a> {
    pub tool: ToolSpecRequest<'a>,
    pub cancellation_token: CancellationToken,
    pub tracker: SharedToolTurnDiffTracker,
    pub call: ToolCall,
    pub source: ToolCallSource,
}

/// Tool domain service API exposed to thread/session runtimes.
pub trait ToolServiceApi: Send + Sync + 'static {
    fn model_visible_specs(&self, request: ToolSpecRequest<'_>) -> Vec<ToolSpec>;

    fn create_diff_consumer(
        &self,
        request: ToolDiffConsumerRequest<'_>,
    ) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>>;

    fn tool_supports_parallel(&self, request: ToolParallelRequest<'_>) -> bool;

    fn dispatch_tool(
        &self,
        request: ToolDispatchRequest<'_>,
    ) -> ToolServiceFuture<'_, Result<AnyToolResult, FunctionCallError>>;
}

pub struct TypedDiffConsumer<Turn> {
    inner: Box<dyn ToolArgumentDiffConsumer<Turn>>,
}

impl<Turn> TypedDiffConsumer<Turn> {
    pub fn new(inner: Box<dyn ToolArgumentDiffConsumer<Turn>>) -> Self {
        Self { inner }
    }
}

impl<Turn> ErasedToolArgumentDiffConsumer for TypedDiffConsumer<Turn>
where
    Turn: ToolServiceTurnRef + 'static,
{
    fn consume_diff(
        &mut self,
        turn: &dyn ToolServiceTurnRef,
        call_id: String,
        diff: &str,
    ) -> Option<codex_protocol::protocol::EventMsg> {
        let Some(turn) = turn.as_any().downcast_ref::<Turn>() else {
            return None;
        };
        self.inner.consume_diff(turn, call_id, diff)
    }

    fn finish(&mut self) -> Result<Option<codex_protocol::protocol::EventMsg>, FunctionCallError> {
        self.inner.finish()
    }
}
