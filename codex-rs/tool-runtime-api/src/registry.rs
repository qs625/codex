use crate::HookToolName;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::EventMsg;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolRegistryEntry;
use codex_tool_planning::ToolSearchInfo;
use codex_tool_planning::ToolSpec;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExposure;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use serde_json::Value;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolTelemetryTags = Vec<(&'static str, String)>;
pub type ToolRuntimeApiBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Minimal view a host invocation must expose to the generic tool registry.
pub trait ToolInvocationView {
    fn call_id(&self) -> &str;
    fn tool_name(&self) -> &ToolName;
    fn payload(&self) -> &ToolPayload;
}

pub trait ToolHandler<Invocation, DiffContext>: ToolExecutor<Invocation> {
    fn search_info(&self) -> Option<ToolSearchInfo> {
        None
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(
            payload,
            ToolPayload::Function { .. } | ToolPayload::ToolSearch { .. }
        )
    }

    fn telemetry_tags(
        &self,
        _invocation: &Invocation,
    ) -> impl Future<Output = ToolTelemetryTags> + Send {
        async { Vec::new() }
    }

    fn post_tool_use_payload(
        &self,
        _invocation: &Invocation,
        _result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        None
    }

    fn pre_tool_use_payload(&self, _invocation: &Invocation) -> Option<PreToolUsePayload> {
        None
    }

    /// Rebuilds a tool invocation from hook-facing `tool_input`.
    ///
    /// Tools that opt into input-rewriting hooks should invert the same stable
    /// hook contract they expose from `pre_tool_use_payload`.
    fn with_updated_hook_input(
        &self,
        _invocation: Invocation,
        _updated_input: Value,
    ) -> Result<Invocation, FunctionCallError> {
        Err(FunctionCallError::RespondToModel(
            "tool does not support hook input rewriting".to_string(),
        ))
    }

    /// Creates an optional consumer for streamed tool argument diffs.
    fn create_diff_consumer(&self) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        None
    }
}

/// Consumes streamed argument diffs for a tool call and emits protocol events
/// derived from partial tool input.
pub trait ToolArgumentDiffConsumer<DiffContext>: Send {
    /// Consume the next argument diff for a tool call.
    fn consume_diff(&mut self, turn: &DiffContext, call_id: String, diff: &str)
    -> Option<EventMsg>;

    /// Finish consuming argument diffs before the tool call completes.
    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        Ok(None)
    }
}

/// Read-only capabilities a router needs from a concrete tool registry.
pub trait ToolRegistryView<DiffContext> {
    fn tool_exposure(&self, name: &ToolName) -> Option<ToolExposure>;

    fn create_diff_consumer(
        &self,
        name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>>;

    fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool>;
}

pub struct AnyToolResult {
    pub call_id: String,
    pub payload: ToolPayload,
    pub result: Box<dyn ToolOutput>,
    pub post_tool_use_payload: Option<PostToolUsePayload>,
}

impl AnyToolResult {
    pub fn into_response(self) -> ResponseInputItem {
        let Self {
            call_id,
            payload,
            result,
            ..
        } = self;
        result.to_response_item(&call_id, &payload)
    }

    pub fn code_mode_result(self) -> serde_json::Value {
        let Self {
            payload, result, ..
        } = self;
        result.code_mode_result(&payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreToolUsePayload {
    /// Hook-facing tool name model.
    ///
    /// The canonical name is serialized to hook stdin, while aliases are used
    /// only for matcher compatibility.
    pub tool_name: HookToolName,
    /// Tool-specific input exposed at `tool_input`.
    ///
    /// Shell-like tools use `{ "command": ... }`; MCP tools use their resolved
    /// JSON arguments.
    pub tool_input: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostToolUsePayload {
    /// Hook-facing tool name model.
    ///
    /// The canonical name is serialized to hook stdin, while aliases are used
    /// only for matcher compatibility.
    pub tool_name: HookToolName,
    /// The originating tool-use id exposed at `tool_use_id`.
    pub tool_use_id: String,
    /// Tool-specific input exposed at `tool_input`.
    pub tool_input: Value,
    /// Tool result exposed at `tool_response`.
    pub tool_response: Value,
}

/// Object-safe registry entry for heterogeneous tool handlers.
///
/// Concrete handlers keep their typed `ToolExecutor::Output`; the registry
/// boxes that output only after typed hooks have run.
pub trait RegisteredTool<Invocation, DiffContext>: Send + Sync {
    fn tool_name(&self) -> ToolName;

    fn spec(&self) -> Option<ToolSpec>;

    fn exposure(&self) -> ToolExposure;

    fn search_info(&self) -> Option<ToolSearchInfo>;

    fn supports_parallel_tool_calls(&self) -> bool;

    fn matches_kind(&self, payload: &ToolPayload) -> bool;

    fn pre_tool_use_payload(&self, invocation: &Invocation) -> Option<PreToolUsePayload>;

    fn with_updated_hook_input(
        &self,
        invocation: Invocation,
        updated_input: Value,
    ) -> Result<Invocation, FunctionCallError>;

    fn telemetry_tags<'a>(
        &'a self,
        invocation: &'a Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, ToolTelemetryTags>;

    fn create_diff_consumer(&self) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>>;

    fn handle_any<'a>(
        &'a self,
        invocation: Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, Result<AnyToolResult, FunctionCallError>>;
}

impl<Invocation, DiffContext> ToolRegistryEntry for dyn RegisteredTool<Invocation, DiffContext> {
    fn tool_name(&self) -> ToolName {
        RegisteredTool::tool_name(self)
    }

    fn spec(&self) -> Option<ToolSpec> {
        RegisteredTool::spec(self)
    }

    fn exposure(&self) -> ToolExposure {
        RegisteredTool::exposure(self)
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        RegisteredTool::search_info(self)
    }
}

pub fn registered_tool<T, Invocation, DiffContext>(
    handler: Arc<T>,
) -> Arc<dyn RegisteredTool<Invocation, DiffContext>>
where
    T: ToolHandler<Invocation, DiffContext> + 'static,
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    Arc::new(RegisteredToolAdapter {
        handler,
        _marker: PhantomData,
    })
}

struct RegisteredToolAdapter<T, Invocation, DiffContext> {
    handler: Arc<T>,
    _marker: PhantomData<fn(Invocation, DiffContext)>,
}

impl<T, Invocation, DiffContext> RegisteredTool<Invocation, DiffContext>
    for RegisteredToolAdapter<T, Invocation, DiffContext>
where
    T: ToolHandler<Invocation, DiffContext>,
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    fn tool_name(&self) -> ToolName {
        ToolExecutor::tool_name(self.handler.as_ref())
    }

    fn spec(&self) -> Option<ToolSpec> {
        ToolExecutor::spec(self.handler.as_ref())
    }

    fn exposure(&self) -> ToolExposure {
        ToolExecutor::exposure(self.handler.as_ref())
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        ToolHandler::search_info(self.handler.as_ref())
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        ToolExecutor::supports_parallel_tool_calls(self.handler.as_ref())
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        ToolHandler::matches_kind(self.handler.as_ref(), payload)
    }

    fn pre_tool_use_payload(&self, invocation: &Invocation) -> Option<PreToolUsePayload> {
        ToolHandler::pre_tool_use_payload(self.handler.as_ref(), invocation)
    }

    fn with_updated_hook_input(
        &self,
        invocation: Invocation,
        updated_input: Value,
    ) -> Result<Invocation, FunctionCallError> {
        ToolHandler::with_updated_hook_input(self.handler.as_ref(), invocation, updated_input)
    }

    fn telemetry_tags<'a>(
        &'a self,
        invocation: &'a Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, ToolTelemetryTags> {
        Box::pin(ToolHandler::telemetry_tags(
            self.handler.as_ref(),
            invocation,
        ))
    }

    fn create_diff_consumer(&self) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        ToolHandler::create_diff_consumer(self.handler.as_ref())
    }

    fn handle_any<'a>(
        &'a self,
        invocation: Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, Result<AnyToolResult, FunctionCallError>> {
        Box::pin(async move {
            let call_id = invocation.call_id().to_string();
            let payload = invocation.payload().clone();
            let output = ToolExecutor::handle(self.handler.as_ref(), invocation.clone()).await?;
            let post_tool_use_payload =
                ToolHandler::post_tool_use_payload(self.handler.as_ref(), &invocation, &output);
            Ok(AnyToolResult {
                call_id,
                payload,
                result: Box::new(output),
                post_tool_use_payload,
            })
        })
    }
}

pub fn override_tool_exposure<Invocation, DiffContext>(
    handler: Arc<dyn RegisteredTool<Invocation, DiffContext>>,
    exposure: ToolExposure,
) -> Arc<dyn RegisteredTool<Invocation, DiffContext>>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    if handler.exposure() == exposure {
        return handler;
    }

    Arc::new(ExposureOverride {
        handler,
        exposure,
        _marker: PhantomData,
    })
}

struct ExposureOverride<Invocation, DiffContext> {
    handler: Arc<dyn RegisteredTool<Invocation, DiffContext>>,
    exposure: ToolExposure,
    _marker: PhantomData<fn(Invocation, DiffContext)>,
}

impl<Invocation, DiffContext> RegisteredTool<Invocation, DiffContext>
    for ExposureOverride<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    fn tool_name(&self) -> ToolName {
        self.handler.tool_name()
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.handler.spec()
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn search_info(&self) -> Option<ToolSearchInfo> {
        self.handler.search_info()
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        self.handler.supports_parallel_tool_calls()
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        self.handler.matches_kind(payload)
    }

    fn pre_tool_use_payload(&self, invocation: &Invocation) -> Option<PreToolUsePayload> {
        self.handler.pre_tool_use_payload(invocation)
    }

    fn with_updated_hook_input(
        &self,
        invocation: Invocation,
        updated_input: Value,
    ) -> Result<Invocation, FunctionCallError> {
        self.handler
            .with_updated_hook_input(invocation, updated_input)
    }

    fn telemetry_tags<'a>(
        &'a self,
        invocation: &'a Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, ToolTelemetryTags> {
        self.handler.telemetry_tags(invocation)
    }

    fn create_diff_consumer(&self) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.handler.create_diff_consumer()
    }

    fn handle_any<'a>(
        &'a self,
        invocation: Invocation,
    ) -> ToolRuntimeApiBoxFuture<'a, Result<AnyToolResult, FunctionCallError>> {
        self.handler.handle_any(invocation)
    }
}
