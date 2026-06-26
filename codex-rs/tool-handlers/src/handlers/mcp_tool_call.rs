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
    use std::borrow::Cow;

    let tool_name = info.canonical_tool_name();
    let mut schema_properties = info
        .tool
        .input_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .map(|map| map.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    schema_properties.sort();

    let name = flat_tool_name(&tool_name);
    let title = info
        .tool
        .title
        .as_deref()
        .map(Cow::Borrowed)
        .unwrap_or_else(|| name.clone());
    let description = info.tool.description.as_deref().unwrap_or_default();
    let connector = info.connector_name.as_deref().unwrap_or_default();
    let namespace = tool_name.namespace.as_deref().unwrap_or_default();

    [
        name,
        title,
        Cow::Borrowed(description),
        Cow::Borrowed(connector),
        Cow::Borrowed(namespace),
        Cow::Owned(schema_properties.join(" ")),
    ]
    .iter()
    .map(|part: &Cow<'_, str>| part.trim())
    .filter(|part: &&str| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}
