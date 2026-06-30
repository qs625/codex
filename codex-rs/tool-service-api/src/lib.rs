use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;

use codex_extension_api::ExtensionData;
use codex_extension_api::ToolContributor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::EventMsg;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::SharedToolTurnDiffTracker;
use thread_service_api::ThreadCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use thread_service_api::ThreadRuntimeCapability;
use codex_tool_config::ToolsConfig;
use codex_tool_types::DiscoverableTool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::RequestPluginInstallElicitationRequest;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_tool_types::ToolSpec;
use tokio_util::sync::CancellationToken;

/// Boxed future returned by object-safe tool service APIs.
pub type ToolServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Borrowed extension state required by the tool owner to discover extension tools.
#[derive(Clone, Copy)]
pub struct ExtensionToolBuildParams<'a> {
    pub tool_contributors: &'a [Arc<dyn ToolContributor>],
    pub session_store: &'a ExtensionData,
    pub thread_store: &'a ExtensionData,
}

/// Borrowed inputs required to build one turn's tool set.
pub struct ToolServiceParams<'a> {
    pub mcp_tools: Option<&'a [ToolInfo]>,
    pub deferred_mcp_tools: Option<&'a [ToolInfo]>,
    pub discoverable_tools: Option<&'a [DiscoverableTool]>,
    pub extension_tools: Option<ExtensionToolBuildParams<'a>>,
    pub dynamic_tools: &'a [DynamicToolSpec],
    pub default_agent_type_description: &'a str,
}

/// Hook-facing tool names and matcher aliases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookToolName {
    name: String,
    matcher_aliases: Vec<String>,
}

impl HookToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            matcher_aliases: Vec::new(),
        }
    }

    pub fn apply_patch() -> Self {
        Self {
            name: "apply_patch".to_string(),
            matcher_aliases: vec!["Write".to_string(), "Edit".to_string()],
        }
    }

    pub fn bash() -> Self {
        Self::new("Bash")
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn matcher_aliases(&self) -> &[String] {
        &self.matcher_aliases
    }
}

pub type ToolTelemetryTags = Vec<(&'static str, String)>;

pub trait ErasedToolArgumentDiffConsumer: Send {
    fn consume_diff(
        &mut self,
        turn: &dyn ThreadTurnCapability,
        call_id: String,
        diff: &str,
    ) -> Option<EventMsg>;

    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        Ok(None)
    }
}

/// Consumes streamed argument diffs for one tool call.
pub trait ToolArgumentDiffConsumer<DiffContext>: Send {
    fn consume_diff(&mut self, turn: &DiffContext, call_id: String, diff: &str)
    -> Option<EventMsg>;

    fn finish(&mut self) -> Result<Option<EventMsg>, FunctionCallError> {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreToolUsePayload {
    pub tool_name: HookToolName,
    pub tool_input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostToolUsePayload {
    pub tool_name: HookToolName,
    pub tool_use_id: String,
    pub tool_input: serde_json::Value,
    pub tool_response: serde_json::Value,
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

pub struct ToolSpecRequest<'a> {
    pub config: &'a ToolsConfig,
    pub session_capability: Weak<dyn ThreadSessionCapability>,
    pub session: Arc<dyn ThreadSessionCapability>,
    pub turn: Arc<dyn ThreadRuntimeCapability>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPluginInstallContext {
    pub server_name: String,
    pub thread_id: String,
    pub turn_id: String,
    pub app_server_client_name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestPluginInstallElicitationOutcome {
    pub user_confirmed: bool,
}

pub struct ToolDispatchRequest<'a> {
    pub tool: ToolSpecRequest<'a>,
    pub cancellation_token: CancellationToken,
    pub tracker: SharedToolTurnDiffTracker,
    pub call: ToolCall,
    pub source: ToolCallSource,
}

/// Plugin/connector install service API consumed by tool discovery handlers.
pub trait RequestPluginInstallApi: Send + Sync + 'static {
    fn request_plugin_install_context(
        &self,
        capability: &dyn ThreadCapability,
    ) -> RequestPluginInstallContext;

    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Vec<DiscoverableTool>, FunctionCallError>>;

    fn request_plugin_install_elicitation<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, RequestPluginInstallElicitationOutcome>;

    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, bool>;
}

impl<Service> RequestPluginInstallApi for Arc<Service>
where
    Service: RequestPluginInstallApi,
{
    fn request_plugin_install_context(
        &self,
        capability: &dyn ThreadCapability,
    ) -> RequestPluginInstallContext {
        self.as_ref().request_plugin_install_context(capability)
    }

    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Vec<DiscoverableTool>, FunctionCallError>> {
        self.as_ref()
            .list_request_plugin_install_discoverable_tools(capability)
    }

    fn request_plugin_install_elicitation<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, RequestPluginInstallElicitationOutcome> {
        self.as_ref()
            .request_plugin_install_elicitation(capability, call_id, request, tool)
    }

    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, bool> {
        self.as_ref()
            .complete_request_plugin_install_if_ready(capability, tool)
    }
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
    Turn: ThreadTurnCapability + 'static,
{
    fn consume_diff(
        &mut self,
        turn: &dyn ThreadTurnCapability,
        call_id: String,
        diff: &str,
    ) -> Option<codex_protocol::protocol::EventMsg> {
        let turn = turn.as_any().downcast_ref::<Turn>()?;
        self.inner.consume_diff(turn, call_id, diff)
    }

    fn finish(&mut self) -> Result<Option<codex_protocol::protocol::EventMsg>, FunctionCallError> {
        self.inner.finish()
    }
}
