use crate::HookToolName;
use crate::McpToolOutput;
use crate::PostToolUsePayload;
use crate::PreToolUsePayload;
use crate::flat_tool_name;
use codex_mcp_tool_types::ToolInfo;
use codex_tool_planning::ResponsesApiNamespace;
use codex_tool_planning::ResponsesApiNamespaceTool;
use codex_tool_planning::ToolSearchInfo;
use codex_tool_planning::ToolSearchSourceInfo;
use codex_tool_planning::mcp_tool_to_responses_api_tool;
use codex_tool_runtime::ToolHandler;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime_api::McpToolCallHost;
use codex_tool_runtime_api::ToolTelemetryTags;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolExposure;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_tool_types::ToolSpec;
use serde_json::Map;
use serde_json::Value;
use std::time::Instant;

pub struct McpHandler<Host> {
    host: Host,
    tool_info: ToolInfo,
    exposure: ToolExposure,
}

impl<Host> McpHandler<Host> {
    pub fn new(host: Host, tool_info: ToolInfo) -> Self {
        Self::with_exposure(host, tool_info, ToolExposure::Direct)
    }

    pub fn with_exposure(host: Host, tool_info: ToolInfo, exposure: ToolExposure) -> Self {
        Self {
            host,
            tool_info,
            exposure,
        }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for McpHandler<Host>
where
    Host: McpToolCallHost,
{
    type Output = McpToolOutput;

    fn tool_name(&self) -> ToolName {
        self.tool_info.canonical_tool_name()
    }

    fn spec(&self) -> Option<ToolSpec> {
        let tool_name = self.tool_name();
        let namespace_name = tool_name.namespace.as_ref()?;
        let tool = mcp_tool_to_responses_api_tool(&tool_name, &self.tool_info.tool).ok()?;
        let description = self
            .tool_info
            .namespace_description
            .as_deref()
            .map(str::trim)
            .filter(|description| !description.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.tool_info
                    .connector_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|connector_name| !connector_name.is_empty())
                    .map(|connector_name| format!("Tools for working with {connector_name}."))
            })
            .unwrap_or_default();

        Some(ToolSpec::Namespace(ResponsesApiNamespace {
            name: namespace_name.clone(),
            description,
            tools: vec![ResponsesApiNamespaceTool::Function(tool)],
        }))
    }

    fn exposure(&self) -> ToolExposure {
        self.exposure
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        self.tool_info.supports_parallel_tool_calls
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let payload = match metadata.payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::RespondToModel(
                        "mcp handler received unsupported payload".to_string(),
                    ));
                }
            };

            let started = Instant::now();
            let outcome = self
                .host
                .call_mcp_tool(
                    session,
                    &turn,
                    call_id,
                    self.tool_info.server_name.clone(),
                    self.tool_info.tool.name.to_string(),
                    self.tool_name().to_string(),
                    payload,
                )
                .await;

            Ok(McpToolOutput {
                result: outcome.result,
                tool_input: outcome.tool_input,
                wall_time: started.elapsed(),
                original_image_detail_supported: self
                    .host
                    .mcp_original_image_detail_supported(&turn),
                truncation_policy: self.host.mcp_truncation_policy(&turn),
            })
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for McpHandler<Host>
where
    Host: McpToolCallHost,
{
    fn search_info(&self) -> Option<ToolSearchInfo> {
        let source_name = self
            .tool_info
            .connector_name
            .as_deref()
            .map(str::trim)
            .filter(|connector_name| !connector_name.is_empty())
            .unwrap_or_else(|| self.tool_info.server_name.trim());
        let source_info = (!source_name.is_empty()).then(|| ToolSearchSourceInfo {
            name: source_name.to_string(),
            description: self
                .tool_info
                .namespace_description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map(str::to_string),
        });

        ToolSearchInfo::from_spec(
            build_mcp_search_text(&self.tool_info),
            self.spec()?,
            source_info,
        )
    }

    async fn telemetry_tags(
        &self,
        _invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolTelemetryTags {
        let mut tags = vec![("mcp_server", self.tool_info.server_name.clone())];
        if let Some(origin) = &self.tool_info.server_origin {
            tags.push(("mcp_server_origin", origin.clone()));
        }
        tags
    }

    fn pre_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> Option<PreToolUsePayload> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return None;
        };

        Some(PreToolUsePayload {
            tool_name: HookToolName::new(self.tool_name().to_string()),
            tool_input: mcp_hook_tool_input(arguments),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        updated_input: Value,
    ) -> Result<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, FunctionCallError> {
        if !matches!(invocation.metadata.payload, ToolPayload::Function { .. }) {
            return Err(FunctionCallError::RespondToModel(format!(
                "tool {} does not support hook input rewriting for payload {:?}",
                self.tool_name(),
                invocation.metadata.payload
            )));
        }
        invocation.metadata.payload = ToolPayload::Function {
            arguments: serde_json::to_string(&updated_input).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to serialize rewritten MCP arguments: {err}"
                ))
            })?,
        };
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        let ToolPayload::Function { .. } = &invocation.payload else {
            return None;
        };

        let tool_response =
            result.post_tool_use_response(&invocation.call_id, &invocation.payload)?;
        Some(PostToolUsePayload {
            tool_name: HookToolName::new(self.tool_name().to_string()),
            tool_use_id: invocation.call_id.clone(),
            tool_input: result.tool_input.clone(),
            tool_response,
        })
    }
}

fn mcp_hook_tool_input(raw_arguments: &str) -> Value {
    if raw_arguments.trim().is_empty() {
        return Value::Object(Map::new());
    }

    serde_json::from_str(raw_arguments).unwrap_or_else(|_| Value::String(raw_arguments.to_string()))
}

fn build_mcp_search_text(info: &ToolInfo) -> String {
    let tool_name = info.canonical_tool_name();
    let mut schema_properties = info
        .tool
        .input_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    schema_properties.sort();
    let mut parts = vec![
        flat_tool_name(&tool_name).into_owned(),
        info.callable_name.clone(),
        info.tool.name.to_string(),
        info.server_name.clone(),
    ];
    if let Some(title) = info.tool.title.as_deref().map(str::trim)
        && !title.is_empty()
    {
        parts.push(title.to_string());
    }
    if let Some(description) = info.tool.description.as_deref().map(str::trim)
        && !description.is_empty()
    {
        parts.push(description.to_string());
    }
    if let Some(connector_name) = info.connector_name.as_deref().map(str::trim)
        && !connector_name.is_empty()
    {
        parts.push(connector_name.to_string());
    }
    if let Some(namespace_description) = info.namespace_description.as_deref().map(str::trim)
        && !namespace_description.is_empty()
    {
        parts.push(namespace_description.to_string());
    }
    parts.extend(
        info.plugin_display_names
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|display_name| !display_name.is_empty())
            .map(str::to_string),
    );
    parts.extend(schema_properties);
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::mcp::CallToolResult;
    use codex_tool_planning::LoadableToolSpec;
    use codex_tool_runtime_api::McpToolCallOutcome;
    use codex_tool_types::ToolCallSource;
    use codex_tool_types::ToolInvocationMetadata;
    use codex_utils_output_truncation::TruncationPolicy;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Copy)]
    struct StubMcpToolCallHost;

    impl McpToolCallHost for StubMcpToolCallHost {
        type Session = ();
        type Turn = ();
        type Tracker = ();
        type DiffContext = ();

        async fn call_mcp_tool(
            &self,
            _session: Self::Session,
            _turn: &Self::Turn,
            _call_id: String,
            _server: String,
            _tool_name: String,
            _hook_tool_name: String,
            _arguments: String,
        ) -> McpToolCallOutcome {
            McpToolCallOutcome {
                result: CallToolResult::from_error_text("not used".to_string()),
                tool_input: json!({}),
            }
        }

        fn mcp_original_image_detail_supported(&self, _turn: &Self::Turn) -> bool {
            true
        }

        fn mcp_truncation_policy(&self, _turn: &Self::Turn) -> TruncationPolicy {
            TruncationPolicy::Bytes(1024)
        }
    }

    #[test]
    fn search_info_uses_mcp_tool_metadata_and_parameter_names() {
        let handler = McpHandler::new(StubMcpToolCallHost, search_tool_info());
        let search_info = handler.search_info().expect("MCP search info");

        assert_eq!(
            search_info.entry.search_text,
            "mcp__calendar___create_event _create_event createEvent codex-apps Create event Create a calendar event. Calendar Plan events. Calendar plugin attendees start_time"
        );
        assert_eq!(
            search_info.source_info,
            Some(ToolSearchSourceInfo {
                name: "Calendar".to_string(),
                description: Some("Plan events.".to_string()),
            })
        );
    }

    #[test]
    fn search_info_uses_connector_name_for_output_namespace_description() {
        let mut tool_info = search_tool_info();
        tool_info.namespace_description = None;
        let handler = McpHandler::new(StubMcpToolCallHost, tool_info);
        let search_info = handler.search_info().expect("MCP search info");

        let LoadableToolSpec::Namespace(namespace) = search_info.entry.output else {
            panic!("expected namespace search output");
        };
        assert_eq!(namespace.description, "Tools for working with Calendar.");
        assert_eq!(
            search_info.source_info,
            Some(ToolSearchSourceInfo {
                name: "Calendar".to_string(),
                description: None,
            })
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_use_payload_uses_model_tool_name_and_raw_args() {
        let payload = ToolPayload::Function {
            arguments: json!({
                "entities": [{
                    "name": "Ada",
                    "entityType": "person"
                }]
            })
            .to_string(),
        };
        let handler = McpHandler::new(
            StubMcpToolCallHost,
            tool_info("memory", "mcp__memory__", "create_entities"),
        );
        assert_eq!(
            handler.pre_tool_use_payload(&invocation(
                "call-mcp-pre",
                ToolName::namespaced("mcp__memory__", "create_entities"),
                payload,
            )),
            Some(PreToolUsePayload {
                tool_name: HookToolName::new("mcp__memory__create_entities"),
                tool_input: json!({
                    "entities": [{
                        "name": "Ada",
                        "entityType": "person"
                    }]
                }),
            })
        );
    }

    #[tokio::test]
    async fn mcp_pre_tool_use_payload_keeps_builtin_like_tool_names_namespaced() {
        let payload = ToolPayload::Function {
            arguments: json!({ "message": "hello" }).to_string(),
        };
        let handler = McpHandler::new(
            StubMcpToolCallHost,
            tool_info("foo", "mcp__foo__", "exec_command"),
        );

        assert_eq!(
            handler.pre_tool_use_payload(&invocation(
                "call-mcp-pre-builtin-like",
                ToolName::namespaced("mcp__foo__", "exec_command"),
                payload,
            )),
            Some(PreToolUsePayload {
                tool_name: HookToolName::new("mcp__foo__exec_command"),
                tool_input: json!({ "message": "hello" }),
            })
        );
    }

    #[tokio::test]
    async fn mcp_updated_input_rewrites_builtin_like_tool_names_as_mcp() {
        let payload = ToolPayload::Function {
            arguments: json!({ "message": "hello" }).to_string(),
        };
        let handler = McpHandler::new(
            StubMcpToolCallHost,
            tool_info("foo", "mcp__foo__", "exec_command"),
        );

        let invocation = handler
            .with_updated_hook_input(
                invocation(
                    "call-mcp-rewrite-builtin-like",
                    ToolName::namespaced("mcp__foo__", "exec_command"),
                    payload,
                ),
                json!({ "message": "rewritten" }),
            )
            .expect("MCP rewrite should succeed");

        let ToolPayload::Function { arguments } = invocation.metadata.payload else {
            panic!("builtin-like MCP tool should stay function-shaped");
        };
        assert_eq!(arguments, json!({ "message": "rewritten" }).to_string());
    }

    #[tokio::test]
    async fn mcp_post_tool_use_payload_is_absent_without_hook_response() {
        let payload = ToolPayload::Function {
            arguments: json!({ "path": "/tmp/notes.txt" }).to_string(),
        };
        let output = McpToolOutput {
            result: CallToolResult {
                content: vec![json!({
                    "type": "text",
                    "text": "notes"
                })],
                structured_content: Some(json!({ "bytes": 5 })),
                is_error: None,
                meta: None,
            },
            tool_input: json!({
                "path": {
                    "file_id": "file_123"
                }
            }),
            wall_time: Duration::from_millis(42),
            original_image_detail_supported: true,
            truncation_policy: TruncationPolicy::Bytes(1024),
        };
        let handler = McpHandler::new(
            StubMcpToolCallHost,
            tool_info("filesystem", "mcp__filesystem__", "read_file"),
        );
        let invocation = invocation(
            "call-mcp-post",
            ToolName::namespaced("mcp__filesystem__", "read_file"),
            payload,
        );
        assert_eq!(handler.post_tool_use_payload(&invocation, &output), None);
    }

    #[test]
    fn mcp_hook_tool_input_defaults_empty_args_to_object() {
        assert_eq!(mcp_hook_tool_input("  "), json!({}));
    }

    fn invocation(
        call_id: &str,
        tool_name: ToolName,
        payload: ToolPayload,
    ) -> ToolInvocation<(), (), ()> {
        ToolInvocation {
            session: (),
            turn: (),
            cancellation_token: CancellationToken::new(),
            tracker: (),
            metadata: ToolInvocationMetadata {
                call_id: call_id.to_string(),
                tool_name,
                source: ToolCallSource::Direct,
                payload,
            },
        }
    }

    fn tool_info(server_name: &str, callable_namespace: &str, tool_name: &str) -> ToolInfo {
        ToolInfo {
            server_name: server_name.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: tool_name.to_string(),
            callable_namespace: callable_namespace.to_string(),
            namespace_description: None,
            tool: codex_mcp_tool_types::McpTool::new(
                tool_name,
                "",
                serde_json::json!({
                    "type": "object",
                }),
            ),
            connector_id: None,
            connector_name: None,
            plugin_display_names: Vec::new(),
        }
    }

    fn search_tool_info() -> ToolInfo {
        ToolInfo {
            server_name: "codex-apps".to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: "_create_event".to_string(),
            callable_namespace: "mcp__calendar__".to_string(),
            namespace_description: Some("Plan events.".to_string()),
            tool: codex_mcp_tool_types::McpTool {
                name: "createEvent".to_string(),
                title: Some("Create event".to_string()),
                description: Some("Create a calendar event.".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "start_time": { "type": "string" },
                        "attendees": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            connector_id: None,
            connector_name: Some("Calendar".to_string()),
            plugin_display_names: vec![" Calendar plugin ".to_string(), " ".to_string()],
        }
    }
}
