use std::sync::Arc;

use codex_extension_api::ExtensionToolExecutor;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::ResponsesApiTool;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolName;
use codex_extension_api::ToolSpec;
use codex_extension_api::parse_tool_input_schema;
use codex_tool_types::ResponsesApiNamespace;
use codex_tool_types::ResponsesApiNamespaceTool;
use codex_tool_types::default_namespace_description;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use super::backend::DEFAULT_LIST_MAX_RESULTS;
use super::backend::DEFAULT_READ_MAX_TOKENS;
use super::backend::DEFAULT_SEARCH_MAX_RESULTS;
use super::backend::ListMemoriesRequest;
use super::backend::ListMemoriesResponse;
use super::backend::MAX_LIST_RESULTS;
use super::backend::MAX_SEARCH_RESULTS;
use super::backend::MemoriesBackend;
use super::backend::MemoriesBackendError;
use super::backend::ReadMemoryRequest;
use super::backend::ReadMemoryResponse;
use super::backend::SearchMatchMode;
use super::backend::SearchMemoriesRequest;
use super::backend::SearchMemoriesResponse;
use super::schema;

pub const MEMORY_TOOLS_NAMESPACE: &str = "memories/";
pub const LIST_TOOL_NAME: &str = "list";
pub const READ_TOOL_NAME: &str = "read";
pub const SEARCH_TOOL_NAME: &str = "search";

pub fn memory_extension_tools<B>(backend: B) -> Vec<Arc<dyn ExtensionToolExecutor>>
where
    B: MemoriesBackend,
{
    vec![
        Arc::new(ListTool {
            backend: backend.clone(),
        }),
        Arc::new(ReadTool {
            backend: backend.clone(),
        }),
        Arc::new(SearchTool { backend }),
    ]
}

pub fn memory_extension_tool_name(name: &str) -> ToolName {
    ToolName::namespaced(MEMORY_TOOLS_NAMESPACE, name)
}

fn memory_function_tool<I: JsonSchema, O: JsonSchema>(name: &str, description: &str) -> ToolSpec {
    let tool = ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters: parse_tool_input_schema(&Value::Object(schema::input_schema_for::<I>()))
            .unwrap_or_else(|err| panic!("generated input schema for {name} should parse: {err}")),
        output_schema: Some(Value::Object(schema::output_schema_for::<O>())),
    };

    ToolSpec::Namespace(ResponsesApiNamespace {
        name: MEMORY_TOOLS_NAMESPACE.to_string(),
        description: default_namespace_description(MEMORY_TOOLS_NAMESPACE),
        tools: vec![ResponsesApiNamespaceTool::Function(tool)],
    })
}

fn parse_args<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> Result<T, FunctionCallError> {
    let arguments = call.function_arguments()?;
    let value = if arguments.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(arguments)
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?
    };
    serde_json::from_value(value).map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
}

fn clamp_max_results(requested: Option<usize>, default: usize, max: usize) -> usize {
    requested.unwrap_or(default).clamp(1, max)
}

fn backend_error_to_function_call(err: MemoriesBackendError) -> FunctionCallError {
    match err {
        MemoriesBackendError::InvalidPath { .. }
        | MemoriesBackendError::InvalidCursor { .. }
        | MemoriesBackendError::NotFound { .. }
        | MemoriesBackendError::InvalidLineOffset
        | MemoriesBackendError::InvalidMaxLines
        | MemoriesBackendError::LineOffsetExceedsFileLength
        | MemoriesBackendError::NotFile { .. }
        | MemoriesBackendError::EmptyQuery
        | MemoriesBackendError::InvalidMatchWindow => {
            FunctionCallError::RespondToModel(err.to_string())
        }
        MemoriesBackendError::Io(_) => FunctionCallError::Fatal(err.to_string()),
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    path: Option<String>,
    cursor: Option<String>,
    #[schemars(range(min = 1))]
    max_results: Option<usize>,
}

#[derive(Clone)]
struct ListTool<B> {
    backend: B,
}

impl<B> ToolExecutor<ToolCall> for ListTool<B>
where
    B: MemoriesBackend,
{
    type Output = JsonToolOutput;

    fn tool_name(&self) -> ToolName {
        memory_extension_tool_name(LIST_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(memory_function_tool::<ListArgs, ListMemoriesResponse>(
            LIST_TOOL_NAME,
            "List immediate files and directories under a path in the Codex memories store.",
        ))
    }

    fn handle<'a>(
        &'a self,
        call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let args: ListArgs = parse_args(&call)?;
            let response = self
                .backend
                .clone()
                .list(ListMemoriesRequest {
                    path: args.path,
                    cursor: args.cursor,
                    max_results: clamp_max_results(
                        args.max_results,
                        DEFAULT_LIST_MAX_RESULTS,
                        MAX_LIST_RESULTS,
                    ),
                })
                .await
                .map_err(backend_error_to_function_call)?;
            Ok(JsonToolOutput::new(json!(response)))
        })
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    path: String,
    #[schemars(range(min = 1))]
    line_offset: Option<usize>,
    #[schemars(range(min = 1))]
    max_lines: Option<usize>,
}

#[derive(Clone)]
struct ReadTool<B> {
    backend: B,
}

impl<B> ToolExecutor<ToolCall> for ReadTool<B>
where
    B: MemoriesBackend,
{
    type Output = JsonToolOutput;

    fn tool_name(&self) -> ToolName {
        memory_extension_tool_name(READ_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(memory_function_tool::<ReadArgs, ReadMemoryResponse>(
            READ_TOOL_NAME,
            "Read a Codex memory file by relative path, optionally starting at a 1-indexed line offset and limiting the number of lines returned.",
        ))
    }

    fn handle<'a>(
        &'a self,
        call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let args: ReadArgs = parse_args(&call)?;
            let response = self
                .backend
                .clone()
                .read(ReadMemoryRequest {
                    path: args.path,
                    line_offset: args.line_offset.unwrap_or(1),
                    max_lines: args.max_lines,
                    max_tokens: DEFAULT_READ_MAX_TOKENS,
                })
                .await
                .map_err(backend_error_to_function_call)?;
            Ok(JsonToolOutput::new(json!(response)))
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    #[schemars(length(min = 1))]
    queries: Vec<String>,
    match_mode: Option<SearchMatchMode>,
    path: Option<String>,
    cursor: Option<String>,
    #[schemars(range(min = 0))]
    context_lines: Option<usize>,
    case_sensitive: Option<bool>,
    normalized: Option<bool>,
    #[schemars(range(min = 1))]
    max_results: Option<usize>,
}

impl SearchArgs {
    fn into_request(self) -> SearchMemoriesRequest {
        SearchMemoriesRequest {
            queries: self.queries,
            match_mode: self.match_mode.unwrap_or(SearchMatchMode::Any),
            path: self.path,
            cursor: self.cursor,
            context_lines: self.context_lines.unwrap_or(0),
            case_sensitive: self.case_sensitive.unwrap_or(true),
            normalized: self.normalized.unwrap_or(false),
            max_results: clamp_max_results(
                self.max_results,
                DEFAULT_SEARCH_MAX_RESULTS,
                MAX_SEARCH_RESULTS,
            ),
        }
    }
}

#[derive(Clone)]
struct SearchTool<B> {
    backend: B,
}

impl<B> ToolExecutor<ToolCall> for SearchTool<B>
where
    B: MemoriesBackend,
{
    type Output = JsonToolOutput;

    fn tool_name(&self) -> ToolName {
        memory_extension_tool_name(SEARCH_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(memory_function_tool::<SearchArgs, SearchMemoriesResponse>(
            SEARCH_TOOL_NAME,
            "Search Codex memory files for substring matches, optionally normalizing separators or requiring all query substrings on the same line or within a line window.",
        ))
    }

    fn handle<'a>(
        &'a self,
        call: ToolCall,
    ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let response = self
                .backend
                .clone()
                .search(parse_args::<SearchArgs>(&call)?.into_request())
                .await
                .map_err(backend_error_to_function_call)?;
            Ok(JsonToolOutput::new(json!(response)))
        })
    }
}
