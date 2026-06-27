//! Shared tool invocation, schema, spec, and discovery contracts.
//!
//! This crate intentionally owns only lightweight, host-facing tool contracts.
//! Concrete tool planning and spec builder logic should stay in owner service
//! crates such as `codex-tool-service`.

mod function_call_error;
mod json_schema;
mod request_plugin_install;
mod responses_api;
mod tool_discovery;
mod tool_call;
mod tool_executor;
mod tool_output;
mod tool_payload;
mod tool_spec;
mod tool_names;

pub use codex_protocol::ToolName;
pub use function_call_error::FunctionCallError;
pub use json_schema::AdditionalProperties;
pub use json_schema::JsonSchema;
pub use json_schema::JsonSchemaPrimitiveType;
pub use json_schema::JsonSchemaType;
pub use json_schema::parse_tool_input_schema;
pub use responses_api::FreeformTool;
pub use responses_api::FreeformToolFormat;
pub use responses_api::LoadableToolSpec;
pub use responses_api::ResponsesApiNamespace;
pub use responses_api::ResponsesApiNamespaceTool;
pub use responses_api::ResponsesApiTool;
pub use responses_api::coalesce_loadable_tool_specs;
pub use responses_api::default_namespace_description;
pub use request_plugin_install::REQUEST_PLUGIN_INSTALL_APPROVAL_KIND_VALUE;
pub use request_plugin_install::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
pub use request_plugin_install::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
pub use request_plugin_install::RequestPluginInstallArgs;
pub use request_plugin_install::RequestPluginInstallElicitationForm;
pub use request_plugin_install::RequestPluginInstallElicitationRequest;
pub use request_plugin_install::RequestPluginInstallElicitationSchema;
pub use request_plugin_install::RequestPluginInstallMeta;
pub use request_plugin_install::RequestPluginInstallResult;
pub use request_plugin_install::all_requested_connectors_picked_up;
pub use request_plugin_install::build_request_plugin_install_elicitation_request;
pub use request_plugin_install::verified_connector_install_completed;
pub use tool_call::ToolCall;
pub use tool_call::ToolCallSource;
pub use tool_call::ToolInvocationMetadata;
pub use tool_discovery::DiscoverablePluginInfo;
pub use tool_discovery::DiscoverableTool;
pub use tool_discovery::DiscoverableToolAction;
pub use tool_discovery::DiscoverableToolType;
pub use tool_discovery::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
pub use tool_discovery::TOOL_SEARCH_DEFAULT_LIMIT;
pub use tool_discovery::TOOL_SEARCH_TOOL_NAME;
pub use tool_discovery::RequestPluginInstallEntry;
pub use tool_discovery::ToolSearchEntry;
pub use tool_discovery::ToolSearchInfo;
pub use tool_discovery::ToolSearchSourceInfo;
pub use tool_discovery::collect_request_plugin_install_entries;
pub use tool_discovery::filter_request_plugin_install_discoverable_tools_for_client;
pub use tool_executor::ToolExecutor;
pub use tool_executor::ToolExecutorFuture;
pub use tool_executor::ToolExposure;
pub use tool_output::JsonToolOutput;
pub use tool_output::ToolOutput;
pub use tool_output::ToolSearchOutput;
pub use tool_payload::ToolPayload;
pub use tool_spec::ResponsesApiWebSearchFilters;
pub use tool_spec::ResponsesApiWebSearchUserLocation;
pub use tool_spec::ToolSpec;
pub use tool_spec::create_tools_json_for_responses_api;
pub use tool_names::UPDATE_GOAL_TOOL_NAME;
