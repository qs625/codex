mod extension_data;
mod extension_tools;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Weak;

use codex_approval_service_api::ApprovalSessionCapability;
use command_service_api::CommandServiceSessionState;
use command_service_api::SessionCommandInteractionCaller;
use mcp_types::ToolInfo;
use protocol::dynamic_tools::DynamicToolSpec;
use protocol::models::ResponseInputItem;
use protocol::protocol::EventMsg;
use thread_service_api::SessionAgentJobCaller;
use thread_service_api::SharedToolTurnDiffTracker;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use tokio_util::sync::CancellationToken;
use tool_config::ToolsConfig;

pub use extension_data::ExtensionData;
pub use extension_tools::ExtensionToolExecutor;
pub use extension_tools::ExtensionToolOutput;
pub use protocol::ToolName;
pub use tool_types::AdditionalProperties;
pub use tool_types::DiscoverablePluginInfo;
pub use tool_types::DiscoverableTool;
pub use tool_types::DiscoverableToolAction;
pub use tool_types::DiscoverableToolType;
pub use tool_types::FreeformTool;
pub use tool_types::FreeformToolFormat;
pub use tool_types::FunctionCallError;
pub use tool_types::JsonSchema;
pub use tool_types::JsonSchemaPrimitiveType;
pub use tool_types::JsonSchemaType;
pub use tool_types::JsonToolOutput;
pub use tool_types::LoadableToolSpec;
pub use tool_types::REQUEST_PLUGIN_INSTALL_APPROVAL_KIND_VALUE;
pub use tool_types::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
pub use tool_types::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
pub use tool_types::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
pub use tool_types::RequestPluginInstallArgs;
pub use tool_types::RequestPluginInstallElicitationForm;
pub use tool_types::RequestPluginInstallElicitationRequest;
pub use tool_types::RequestPluginInstallElicitationSchema;
pub use tool_types::RequestPluginInstallEntry;
pub use tool_types::RequestPluginInstallMeta;
pub use tool_types::RequestPluginInstallResult;
pub use tool_types::ResponsesApiNamespace;
pub use tool_types::ResponsesApiNamespaceTool;
pub use tool_types::ResponsesApiTool;
pub use tool_types::ResponsesApiWebSearchFilters;
pub use tool_types::ResponsesApiWebSearchUserLocation;
pub use tool_types::TOOL_SEARCH_DEFAULT_LIMIT;
pub use tool_types::TOOL_SEARCH_TOOL_NAME;
pub use tool_types::ToolCall;
pub use tool_types::ToolCallSource;
pub use tool_types::ToolExecutor;
pub use tool_types::ToolExecutorFuture;
pub use tool_types::ToolExposure;
pub use tool_types::ToolInvocationMetadata;
pub use tool_types::ToolOutput;
pub use tool_types::ToolPayload;
pub use tool_types::ToolSearchEntry;
pub use tool_types::ToolSearchInfo;
pub use tool_types::ToolSearchOutput;
pub use tool_types::ToolSearchSourceInfo;
pub use tool_types::ToolSpec;
pub use tool_types::UPDATE_GOAL_TOOL_NAME;
pub use tool_types::all_requested_connectors_picked_up;
pub use tool_types::build_request_plugin_install_elicitation_request;
pub use tool_types::coalesce_loadable_tool_specs;
pub use tool_types::collect_request_plugin_install_entries;
pub use tool_types::create_tools_json_for_responses_api;
pub use tool_types::default_namespace_description;
pub use tool_types::filter_request_plugin_install_discoverable_tools_for_client;
pub use tool_types::parse_tool_input_schema;
pub use tool_types::verified_connector_install_completed;

/// Extension contribution that exposes native tools owned by a feature.
pub trait ToolContributor: Send + Sync {
    /// Returns the native tools visible for the supplied extension stores.
    fn tools(
        &self,
        session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn ExtensionToolExecutor>>;
}

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
    pub approval_session: Arc<dyn ApprovalSessionCapability>,
    pub session_command_state: Arc<dyn CommandServiceSessionState>,
    pub session_command_interaction: Arc<dyn SessionCommandInteractionCaller>,
    pub session_agent_jobs: Arc<dyn SessionAgentJobCaller>,
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
    Turn: ThreadTurnCapability + 'static,
{
    fn consume_diff(
        &mut self,
        turn: &dyn ThreadTurnCapability,
        call_id: String,
        diff: &str,
    ) -> Option<protocol::protocol::EventMsg> {
        let turn = turn.as_any().downcast_ref::<Turn>()?;
        self.inner.consume_diff(turn, call_id, diff)
    }

    fn finish(&mut self) -> Result<Option<protocol::protocol::EventMsg>, FunctionCallError> {
        self.inner.finish()
    }
}
