//! Shared tool invocation, schema, spec, and output types.
//!
//! This crate intentionally owns only lightweight, host-facing tool contracts.
//! Tool planning, tool discovery, MCP conversion, and concrete tool factory
//! helpers continue to live in `codex-tools`.

mod function_call_error;
mod json_schema;
mod responses_api;
mod tool_call;
mod tool_executor;
mod tool_output;
mod tool_payload;
mod tool_spec;

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
pub use tool_call::ToolCall;
pub use tool_call::ToolCallSource;
pub use tool_call::ToolInvocationMetadata;
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
