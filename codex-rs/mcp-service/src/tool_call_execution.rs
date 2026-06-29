use codex_mcp_tool_types::sanitize_mcp_tool_result_for_model;
use codex_mcp_tool_types::truncate_mcp_tool_result_for_event;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp_types::CodexAppsConnectorAuthFailure;
use codex_mcp_types::ElicitationAction;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::MCP_SANDBOX_STATE_META_CAPABILITY;
use codex_mcp_types::McpServerElicitationRequest;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::McpToolApprovalMetadata;
use codex_mcp_types::SandboxState;
use codex_mcp_types::auth_elicitation_completed_result;
use codex_mcp_types::build_auth_elicitation_plan;
use codex_mcp_types::with_mcp_tool_call_thread_id_meta;
use codex_protocol::items::TurnItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::RequestId;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::McpInvocation;
use serde_json::Value as JsonValue;
use std::time::Duration;
use std::time::Instant;
use tracing::Instrument;
use tracing::Span;

use crate::McpToolCallSpanFields;
use crate::build_mcp_tool_call_completed_item;
use crate::build_mcp_tool_call_request_meta;
use crate::mcp_tool_call_span;
use crate::record_mcp_result_span_telemetry;

// Keep the MCP event result cap aligned with codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP
// without making mcp-service depend on the pty crate for a constant.
const MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES: usize = 1024 * 1024;

/// Host capabilities needed by Codex Apps auth elicitation.
///
/// Implementations should dispatch the elicitation through the embedding
/// runtime and refresh Codex Apps connector/tool state after an accepted
/// browser auth flow.
pub trait CodexAppsAuthElicitationHost {
    fn request_codex_apps_auth_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> impl std::future::Future<Output = Option<ElicitationResponse>> + Send;

    fn refresh_codex_apps_after_connector_auth(
        &self,
    ) -> impl std::future::Future<Output = ()> + Send;
}

#[derive(Debug, Clone, Copy)]
pub struct CodexAppsAuthElicitationContext<'a> {
    pub is_host_owned_codex_apps_server: bool,
    pub auth_elicitation_enabled: bool,
    pub approval_policy: AskForApproval,
    pub thread_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub call_id: &'a str,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
}

#[derive(Debug, Clone)]
pub struct CodexAppsAuthElicitationRequest {
    pub request_id: RequestId,
    pub params: McpServerElicitationRequestParams,
    pub auth_failure: CodexAppsConnectorAuthFailure,
}

/// Host capabilities needed for one approved MCP tool execution.
///
/// Implementations own concrete MCP manager access, rollout trace mutation,
/// sandbox metadata lookup, and Codex Apps auth elicitation transport. The MCP
/// runtime owns request metadata sequencing and result shaping.
pub trait McpToolExecutionHost: CodexAppsAuthElicitationHost {
    fn augment_mcp_tool_request_meta_with_sandbox_state(
        &self,
        server: &str,
        meta: Option<JsonValue>,
    ) -> impl std::future::Future<Output = anyhow::Result<Option<JsonValue>>> + Send;

    fn add_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<JsonValue>,
    ) -> Option<JsonValue>;

    fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<JsonValue>,
        meta: Option<JsonValue>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, String>> + Send;
}

/// Host capabilities needed for the full approved MCP tool-call lifecycle.
///
/// Implementations own concrete side effects such as local file rewrites,
/// event emission, metrics, app analytics, and memory pollution markers. The
/// MCP runtime owns the lifecycle ordering around those side effects.
pub trait McpApprovedToolCallLifecycleHost: McpToolExecutionHost {
    fn mark_thread_memory_mode_polluted_if_needed(
        &self,
        server: &str,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn server_origin(
        &self,
        server: &str,
    ) -> impl std::future::Future<Output = Option<String>> + Send;

    fn rewrite_mcp_tool_arguments_for_openai_files(
        &self,
        arguments: Option<JsonValue>,
        openai_file_input_params: Option<&[String]>,
    ) -> impl std::future::Future<Output = Result<Option<JsonValue>, String>> + Send;

    fn emit_mcp_tool_call_completed(
        &self,
        item: TurnItem,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn track_codex_app_used(
        &self,
        server: &str,
        tool_name: &str,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn emit_mcp_call_metrics(
        &self,
        status: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
        duration: Duration,
    );
}

#[derive(Debug, Clone)]
pub struct McpToolExecutionContext<'a> {
    pub thread_id: &'a str,
    pub call_id: &'a str,
    pub invocation: &'a McpInvocation,
    pub rewritten_arguments: Option<JsonValue>,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
    pub turn_metadata: Option<JsonValue>,
    pub turn_metadata_header_name: &'a str,
    pub supports_image_input: bool,
    pub auth_elicitation_context: CodexAppsAuthElicitationContext<'a>,
}

#[derive(Debug, Clone)]
pub struct McpApprovedToolCallLifecycleContext<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub call_id: &'a str,
    pub invocation: McpInvocation,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
    pub mcp_app_resource_uri: Option<String>,
    pub turn_metadata: Option<JsonValue>,
    pub turn_metadata_header_name: &'a str,
    pub supports_image_input: bool,
    pub auth_elicitation_context: CodexAppsAuthElicitationContext<'a>,
}

#[derive(Debug, Clone)]
pub struct ApprovedMcpToolCallOutcome {
    pub result: CallToolResult,
    pub tool_input: JsonValue,
}

pub async fn handle_approved_mcp_tool_call(
    host: &impl McpApprovedToolCallLifecycleHost,
    context: McpApprovedToolCallLifecycleContext<'_>,
) -> ApprovedMcpToolCallOutcome {
    let server = context.invocation.server.clone();
    host.mark_thread_memory_mode_polluted_if_needed(&server)
        .await;
    let tool_name = context.invocation.tool.clone();
    let arguments_value = context.invocation.arguments.clone();
    let connector_id = context
        .metadata
        .and_then(|metadata| metadata.connector_id.as_deref());
    let connector_name = context
        .metadata
        .and_then(|metadata| metadata.connector_name.as_deref());
    let server_origin = host.server_origin(&server).await;

    let start = Instant::now();
    let rewrite = host
        .rewrite_mcp_tool_arguments_for_openai_files(
            arguments_value.clone(),
            context
                .metadata
                .and_then(|metadata| metadata.openai_file_input_params.as_deref()),
        )
        .await;
    let tool_input = match &rewrite {
        Ok(Some(rewritten_arguments)) => rewritten_arguments.clone(),
        Ok(None) | Err(_) => {
            arguments_value.unwrap_or_else(|| JsonValue::Object(serde_json::Map::new()))
        }
    };
    let result = async {
        let rewritten_arguments = rewrite?;
        let result = execute_mcp_tool_call(
            host,
            McpToolExecutionContext {
                thread_id: context.thread_id,
                call_id: context.call_id,
                invocation: &context.invocation,
                rewritten_arguments,
                metadata: context.metadata,
                turn_metadata: context.turn_metadata.clone(),
                turn_metadata_header_name: context.turn_metadata_header_name,
                supports_image_input: context.supports_image_input,
                auth_elicitation_context: context.auth_elicitation_context,
            },
        )
        .await;
        record_mcp_result_span_telemetry(&Span::current(), result.as_ref().ok());
        result
    }
    .instrument(mcp_tool_call_span(McpToolCallSpanFields {
        server_name: &server,
        tool_name: &tool_name,
        call_id: context.call_id,
        server_origin: server_origin.as_deref(),
        connector_id,
        connector_name,
        conversation_id: context.thread_id,
        session_id: context.thread_id,
        turn_id: context.turn_id,
    }))
    .await;
    if let Err(error) = &result {
        tracing::warn!("MCP tool call error: {error:?}");
    }
    let duration = start.elapsed();
    let completed_item = build_mcp_tool_call_completed_item(
        context.call_id,
        context.invocation,
        context.mcp_app_resource_uri,
        duration,
        truncate_mcp_tool_result_for_event(&result, MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES),
    );
    host.emit_mcp_tool_call_completed(completed_item).await;
    host.track_codex_app_used(&server, &tool_name).await;

    let status = if result.is_ok() { "ok" } else { "error" };
    host.emit_mcp_call_metrics(status, &tool_name, connector_id, connector_name, duration);

    ApprovedMcpToolCallOutcome {
        result: CallToolResult::from_result(result),
        tool_input,
    }
}

pub async fn execute_mcp_tool_call(
    host: &impl McpToolExecutionHost,
    context: McpToolExecutionContext<'_>,
) -> Result<CallToolResult, String> {
    let request_meta = build_mcp_tool_call_request_meta(
        context.turn_metadata,
        &context.invocation.server,
        context.call_id,
        context.metadata,
        context.turn_metadata_header_name,
    );
    let request_meta = with_mcp_tool_call_thread_id_meta(request_meta, context.thread_id);
    let request_meta = host
        .augment_mcp_tool_request_meta_with_sandbox_state(&context.invocation.server, request_meta)
        .await
        .map_err(|error| format!("failed to build MCP tool request metadata: {error:#}"))?;
    let request_meta = host.add_mcp_call_trace_request_meta(context.call_id, request_meta);
    let result = host
        .call_mcp_tool(
            &context.invocation.server,
            &context.invocation.tool,
            context.rewritten_arguments,
            request_meta,
        )
        .await?;
    let result = sanitize_mcp_tool_result_for_model(context.supports_image_input, Ok(result))?;
    Ok(
        maybe_request_codex_apps_auth_elicitation(host, context.auth_elicitation_context, result)
            .await,
    )
}

pub fn build_codex_apps_auth_elicitation_request(
    thread_id: &str,
    turn_id: Option<&str>,
    call_id: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    result: &CallToolResult,
) -> Option<CodexAppsAuthElicitationRequest> {
    let connector_id = metadata.and_then(|metadata| metadata.connector_id.as_deref());
    let connector_name = metadata.and_then(|metadata| metadata.connector_name.as_deref());
    let install_url = connector_id.map(|connector_id| {
        codex_connectors_api::metadata::connector_install_url(
            connector_name.unwrap_or(connector_id),
            connector_id,
        )
    });
    let plan =
        build_auth_elicitation_plan(call_id, result, connector_id, connector_name, install_url)?;
    let request_id = RequestId::String(plan.elicitation.elicitation_id.clone());
    let params = McpServerElicitationRequestParams {
        thread_id: thread_id.to_string(),
        turn_id: turn_id.map(ToString::to_string),
        server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
        request: McpServerElicitationRequest::Url {
            meta: Some(plan.elicitation.meta),
            message: plan.elicitation.message,
            url: plan.elicitation.url,
            elicitation_id: plan.elicitation.elicitation_id,
        },
    };

    Some(CodexAppsAuthElicitationRequest {
        request_id,
        params,
        auth_failure: plan.auth_failure,
    })
}

pub async fn maybe_request_codex_apps_auth_elicitation(
    host: &impl CodexAppsAuthElicitationHost,
    context: CodexAppsAuthElicitationContext<'_>,
    result: CallToolResult,
) -> CallToolResult {
    if !context.is_host_owned_codex_apps_server {
        return result;
    }

    if !context.auth_elicitation_enabled {
        return result;
    }

    match context.approval_policy {
        AskForApproval::Never => return result,
        AskForApproval::Granular(granular_config) if !granular_config.allows_mcp_elicitations() => {
            return result;
        }
        AskForApproval::OnFailure
        | AskForApproval::OnRequest
        | AskForApproval::UnlessTrusted
        | AskForApproval::Granular(_) => {}
    }

    let Some(request) = build_codex_apps_auth_elicitation_request(
        context.thread_id,
        context.turn_id,
        context.call_id,
        context.metadata,
        &result,
    ) else {
        return result;
    };

    let response = host
        .request_codex_apps_auth_elicitation(request.request_id, request.params)
        .await;
    if !response
        .as_ref()
        .is_some_and(|response| response.action == ElicitationAction::Accept)
    {
        return result;
    }

    host.refresh_codex_apps_after_connector_auth().await;
    auth_elicitation_completed_result(&request.auth_failure, result.meta)
}

pub fn insert_sandbox_state_request_meta(
    mut meta: Option<JsonValue>,
    sandbox_state: SandboxState,
) -> anyhow::Result<Option<JsonValue>> {
    let sandbox_state = serde_json::to_value(sandbox_state)?;

    match meta.as_mut() {
        Some(JsonValue::Object(map)) => {
            map.insert(MCP_SANDBOX_STATE_META_CAPABILITY.to_string(), sandbox_state);
        }
        Some(_) => {}
        None => {
            let mut map = serde_json::Map::new();
            map.insert(MCP_SANDBOX_STATE_META_CAPABILITY.to_string(), sandbox_state);
            meta = Some(JsonValue::Object(map));
        }
    }

    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_mcp_types::CONNECTOR_AUTH_FAILURE_CONNECTOR_ID_KEY;
    use codex_mcp_types::CONNECTOR_AUTH_FAILURE_IS_AUTH_FAILURE_KEY;
    use codex_mcp_types::CONNECTOR_AUTH_FAILURE_META_KEY;
    use codex_mcp_types::MCP_TOOL_CODEX_APPS_META_KEY;
    use codex_mcp_types::MCP_TOOL_THREAD_ID_META_KEY;
    use codex_protocol::items::McpToolCallStatus;
    use codex_protocol::items::TurnItem;
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::protocol::GranularApprovalConfig;
    use codex_protocol::protocol::McpInvocation;
    use codex_protocol::protocol::SandboxPolicy;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    fn auth_failure_result() -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::json!({
                "type": "text",
                "text": "Connector reauthentication required",
            })],
            structured_content: None,
            is_error: Some(true),
            meta: Some(serde_json::json!({
                MCP_TOOL_CODEX_APPS_META_KEY: {
                    CONNECTOR_AUTH_FAILURE_META_KEY: {
                        CONNECTOR_AUTH_FAILURE_IS_AUTH_FAILURE_KEY: true,
                        CONNECTOR_AUTH_FAILURE_CONNECTOR_ID_KEY: "calendar",
                    },
                },
            })),
        }
    }

    #[test]
    fn builds_codex_apps_auth_elicitation_request() {
        let metadata = McpToolApprovalMetadata {
            annotations: None,
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Calendar".to_string()),
            connector_description: None,
            tool_title: None,
            tool_description: None,
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: None,
        };

        let request = build_codex_apps_auth_elicitation_request(
            "thread-1",
            Some("turn-1"),
            "call-1",
            Some(&metadata),
            &auth_failure_result(),
        )
        .expect("auth elicitation request");

        assert_eq!(
            request.request_id,
            RequestId::String("codex_apps_auth_call-1".to_string())
        );
        assert_eq!(request.params.thread_id, "thread-1");
        assert_eq!(request.params.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(request.params.server_name, CODEX_APPS_MCP_SERVER_NAME);
        assert_eq!(request.auth_failure.connector_id, "calendar");
        assert_eq!(request.auth_failure.connector_name, "Calendar");
    }

    #[derive(Clone)]
    struct FakeAuthElicitationHost {
        response: Option<ElicitationResponse>,
        request_count: Arc<AtomicUsize>,
        refresh_count: Arc<AtomicUsize>,
        saw_expected_request: Arc<AtomicBool>,
    }

    impl FakeAuthElicitationHost {
        fn new(response: Option<ElicitationResponse>) -> Self {
            Self {
                response,
                request_count: Arc::new(AtomicUsize::new(0)),
                refresh_count: Arc::new(AtomicUsize::new(0)),
                saw_expected_request: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl CodexAppsAuthElicitationHost for FakeAuthElicitationHost {
        async fn request_codex_apps_auth_elicitation(
            &self,
            request_id: RequestId,
            params: McpServerElicitationRequestParams,
        ) -> Option<ElicitationResponse> {
            self.request_count.fetch_add(1, Ordering::SeqCst);
            self.saw_expected_request.store(
                request_id == RequestId::String("codex_apps_auth_call-1".to_string())
                    && params.thread_id == "thread-1"
                    && params.turn_id.as_deref() == Some("turn-1")
                    && params.server_name == CODEX_APPS_MCP_SERVER_NAME,
                Ordering::SeqCst,
            );
            self.response.clone()
        }

        async fn refresh_codex_apps_after_connector_auth(&self) {
            self.refresh_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct RecordedMcpCall {
        server: String,
        tool: String,
        arguments: Option<JsonValue>,
        meta: Option<JsonValue>,
    }

    #[derive(Clone)]
    struct FakeExecutionHost {
        call_result: CallToolResult,
        recorded_call: Arc<Mutex<Option<RecordedMcpCall>>>,
        completed_items: Arc<Mutex<Vec<TurnItem>>>,
        metric_statuses: Arc<Mutex<Vec<String>>>,
        mark_count: Arc<AtomicUsize>,
        track_count: Arc<AtomicUsize>,
        rewrite_count: Arc<AtomicUsize>,
    }

    impl FakeExecutionHost {
        fn new(call_result: CallToolResult) -> Self {
            Self {
                call_result,
                recorded_call: Arc::new(Mutex::new(None)),
                completed_items: Arc::new(Mutex::new(Vec::new())),
                metric_statuses: Arc::new(Mutex::new(Vec::new())),
                mark_count: Arc::new(AtomicUsize::new(0)),
                track_count: Arc::new(AtomicUsize::new(0)),
                rewrite_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl CodexAppsAuthElicitationHost for FakeExecutionHost {
        async fn request_codex_apps_auth_elicitation(
            &self,
            _request_id: RequestId,
            _params: McpServerElicitationRequestParams,
        ) -> Option<ElicitationResponse> {
            None
        }

        async fn refresh_codex_apps_after_connector_auth(&self) {}
    }

    impl McpToolExecutionHost for FakeExecutionHost {
        async fn augment_mcp_tool_request_meta_with_sandbox_state(
            &self,
            _server: &str,
            mut meta: Option<JsonValue>,
        ) -> anyhow::Result<Option<JsonValue>> {
            if let Some(JsonValue::Object(map)) = meta.as_mut() {
                map.insert("sandbox".to_string(), serde_json::json!(true));
            }
            Ok(meta)
        }

        fn add_mcp_call_trace_request_meta(
            &self,
            call_id: &str,
            mut meta: Option<JsonValue>,
        ) -> Option<JsonValue> {
            if let Some(JsonValue::Object(map)) = meta.as_mut() {
                map.insert("trace".to_string(), serde_json::json!(call_id));
            }
            meta
        }

        async fn call_mcp_tool(
            &self,
            server: &str,
            tool: &str,
            arguments: Option<JsonValue>,
            meta: Option<JsonValue>,
        ) -> Result<CallToolResult, String> {
            *self.recorded_call.lock().unwrap() = Some(RecordedMcpCall {
                server: server.to_string(),
                tool: tool.to_string(),
                arguments,
                meta,
            });
            Ok(self.call_result.clone())
        }
    }

    impl McpApprovedToolCallLifecycleHost for FakeExecutionHost {
        async fn mark_thread_memory_mode_polluted_if_needed(&self, _server: &str) {
            self.mark_count.fetch_add(1, Ordering::SeqCst);
        }

        async fn server_origin(&self, _server: &str) -> Option<String> {
            Some("https://mcp.example".to_string())
        }

        async fn rewrite_mcp_tool_arguments_for_openai_files(
            &self,
            _arguments: Option<JsonValue>,
            _openai_file_input_params: Option<&[String]>,
        ) -> Result<Option<JsonValue>, String> {
            self.rewrite_count.fetch_add(1, Ordering::SeqCst);
            Ok(Some(serde_json::json!({"title": "Rewritten"})))
        }

        async fn emit_mcp_tool_call_completed(&self, item: TurnItem) {
            self.completed_items.lock().unwrap().push(item);
        }

        async fn track_codex_app_used(&self, _server: &str, _tool_name: &str) {
            self.track_count.fetch_add(1, Ordering::SeqCst);
        }

        fn emit_mcp_call_metrics(
            &self,
            status: &str,
            _tool_name: &str,
            _connector_id: Option<&str>,
            _connector_name: Option<&str>,
            _duration: Duration,
        ) {
            self.metric_statuses
                .lock()
                .unwrap()
                .push(status.to_string());
        }
    }

    fn auth_context<'a>(
        metadata: &'a McpToolApprovalMetadata,
    ) -> CodexAppsAuthElicitationContext<'a> {
        CodexAppsAuthElicitationContext {
            is_host_owned_codex_apps_server: true,
            auth_elicitation_enabled: true,
            approval_policy: AskForApproval::OnRequest,
            thread_id: "thread-1",
            turn_id: Some("turn-1"),
            call_id: "call-1",
            metadata: Some(metadata),
        }
    }

    fn auth_failure_metadata() -> McpToolApprovalMetadata {
        McpToolApprovalMetadata {
            annotations: None,
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Google Calendar".to_string()),
            connector_description: None,
            tool_title: None,
            tool_description: None,
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: None,
        }
    }

    fn successful_tool_result() -> CallToolResult {
        CallToolResult {
            content: vec![serde_json::json!({
                "type": "text",
                "text": "done",
            })],
            structured_content: None,
            is_error: None,
            meta: None,
        }
    }

    #[tokio::test]
    async fn execute_mcp_tool_call_builds_meta_and_calls_host() {
        let metadata = McpToolApprovalMetadata {
            annotations: None,
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Calendar".to_string()),
            connector_description: None,
            tool_title: Some("Create Event".to_string()),
            tool_description: None,
            mcp_app_resource_uri: None,
            codex_apps_meta: Some(
                serde_json::json!({
                    "resource_uri": "connector://calendar/tools/create_event"
                })
                .as_object()
                .cloned()
                .expect("codex apps meta object"),
            ),
            openai_file_input_params: None,
        };
        let invocation = McpInvocation {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            tool: "calendar_create_event".to_string(),
            arguments: Some(serde_json::json!({"title": "Review"})),
        };
        let host = FakeExecutionHost::new(successful_tool_result());

        let result = execute_mcp_tool_call(
            &host,
            McpToolExecutionContext {
                thread_id: "thread-1",
                call_id: "call-1",
                invocation: &invocation,
                rewritten_arguments: Some(serde_json::json!({"title": "Rewritten"})),
                metadata: Some(&metadata),
                turn_metadata: Some(serde_json::json!({"model": "gpt-test"})),
                turn_metadata_header_name: "x-codex-turn-metadata",
                supports_image_input: false,
                auth_elicitation_context: auth_context(&metadata),
            },
        )
        .await
        .expect("tool execution");

        assert_eq!(result, successful_tool_result());
        let recorded = host
            .recorded_call
            .lock()
            .unwrap()
            .clone()
            .expect("recorded call");
        assert_eq!(recorded.server, CODEX_APPS_MCP_SERVER_NAME);
        assert_eq!(recorded.tool, "calendar_create_event");
        assert_eq!(
            recorded.arguments,
            Some(serde_json::json!({"title": "Rewritten"}))
        );
        let meta = recorded.meta.expect("request meta");
        assert_eq!(
            meta["x-codex-turn-metadata"],
            serde_json::json!({"model": "gpt-test"})
        );
        assert_eq!(
            meta[MCP_TOOL_THREAD_ID_META_KEY],
            serde_json::json!("thread-1")
        );
        assert_eq!(meta["sandbox"], serde_json::json!(true));
        assert_eq!(meta["trace"], serde_json::json!("call-1"));
        assert_eq!(
            meta[MCP_TOOL_CODEX_APPS_META_KEY]["call_id"],
            serde_json::json!("call-1")
        );
        assert_eq!(
            meta[MCP_TOOL_CODEX_APPS_META_KEY]["resource_uri"],
            serde_json::json!("connector://calendar/tools/create_event")
        );
    }

    #[tokio::test]
    async fn handle_approved_mcp_tool_call_runs_lifecycle_and_emits_completion() {
        let metadata = McpToolApprovalMetadata {
            annotations: None,
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Calendar".to_string()),
            connector_description: None,
            tool_title: Some("Create Event".to_string()),
            tool_description: None,
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: Some(vec!["attachment".to_string()]),
        };
        let invocation = McpInvocation {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            tool: "calendar_create_event".to_string(),
            arguments: Some(serde_json::json!({
                "title": "Review",
                "attachment": "/tmp/agenda.pdf",
            })),
        };
        let host = FakeExecutionHost::new(successful_tool_result());

        let outcome = handle_approved_mcp_tool_call(
            &host,
            McpApprovedToolCallLifecycleContext {
                thread_id: "thread-1",
                turn_id: "turn-1",
                call_id: "call-1",
                invocation,
                metadata: Some(&metadata),
                mcp_app_resource_uri: Some("connector://calendar".to_string()),
                turn_metadata: Some(serde_json::json!({"model": "gpt-test"})),
                turn_metadata_header_name: "x-codex-turn-metadata",
                supports_image_input: false,
                auth_elicitation_context: auth_context(&metadata),
            },
        )
        .await;

        assert_eq!(outcome.result, successful_tool_result());
        assert_eq!(
            outcome.tool_input,
            serde_json::json!({"title": "Rewritten"})
        );
        assert_eq!(host.mark_count.load(Ordering::SeqCst), 1);
        assert_eq!(host.rewrite_count.load(Ordering::SeqCst), 1);
        assert_eq!(host.track_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            host.metric_statuses.lock().unwrap().as_slice(),
            &["ok".to_string()]
        );
        let recorded = host
            .recorded_call
            .lock()
            .unwrap()
            .clone()
            .expect("recorded call");
        assert_eq!(
            recorded.arguments,
            Some(serde_json::json!({"title": "Rewritten"}))
        );
        let completed_items = host.completed_items.lock().unwrap();
        assert_eq!(completed_items.len(), 1);
        let TurnItem::McpToolCall(item) = &completed_items[0] else {
            panic!("expected MCP tool call item");
        };
        assert_eq!(item.id, "call-1");
        assert_eq!(item.status, McpToolCallStatus::Completed);
        assert_eq!(
            item.mcp_app_resource_uri.as_deref(),
            Some("connector://calendar")
        );
    }

    #[tokio::test]
    async fn auth_elicitation_returns_original_result_when_disabled_or_disallowed() {
        let metadata = auth_failure_metadata();
        let result = auth_failure_result();
        let host = FakeAuthElicitationHost::new(Some(ElicitationResponse {
            action: ElicitationAction::Accept,
            content: None,
            meta: None,
        }));

        let disabled_context = CodexAppsAuthElicitationContext {
            auth_elicitation_enabled: false,
            ..auth_context(&metadata)
        };
        assert_eq!(
            maybe_request_codex_apps_auth_elicitation(&host, disabled_context, result.clone())
                .await,
            result
        );

        let non_host_owned_context = CodexAppsAuthElicitationContext {
            is_host_owned_codex_apps_server: false,
            ..auth_context(&metadata)
        };
        assert_eq!(
            maybe_request_codex_apps_auth_elicitation(
                &host,
                non_host_owned_context,
                result.clone()
            )
            .await,
            result
        );

        let approval_never_context = CodexAppsAuthElicitationContext {
            approval_policy: AskForApproval::Never,
            ..auth_context(&metadata)
        };
        assert_eq!(
            maybe_request_codex_apps_auth_elicitation(
                &host,
                approval_never_context,
                result.clone()
            )
            .await,
            result
        );

        let granular_denied_context = CodexAppsAuthElicitationContext {
            approval_policy: AskForApproval::Granular(GranularApprovalConfig {
                sandbox_approval: true,
                rules: true,
                skill_approval: true,
                request_permissions: true,
                mcp_elicitations: false,
            }),
            ..auth_context(&metadata)
        };
        assert_eq!(
            maybe_request_codex_apps_auth_elicitation(
                &host,
                granular_denied_context,
                result.clone()
            )
            .await,
            result
        );

        assert_eq!(host.request_count.load(Ordering::SeqCst), 0);
        assert_eq!(host.refresh_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn auth_elicitation_acceptance_returns_completed_result_and_refreshes_apps() {
        let metadata = auth_failure_metadata();
        let result = auth_failure_result();
        let host = FakeAuthElicitationHost::new(Some(ElicitationResponse {
            action: ElicitationAction::Accept,
            content: None,
            meta: None,
        }));

        let returned =
            maybe_request_codex_apps_auth_elicitation(&host, auth_context(&metadata), result).await;

        assert_eq!(
            returned.content,
            vec![serde_json::json!({
                "type": "text",
                "text": "Authentication for Google Calendar was requested and accepted. Retry this tool call now.",
            })]
        );
        assert_eq!(host.request_count.load(Ordering::SeqCst), 1);
        assert_eq!(host.refresh_count.load(Ordering::SeqCst), 1);
        assert!(host.saw_expected_request.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn auth_elicitation_decline_returns_original_result_without_refresh() {
        let metadata = auth_failure_metadata();
        let result = auth_failure_result();
        let host = FakeAuthElicitationHost::new(Some(ElicitationResponse {
            action: ElicitationAction::Decline,
            content: None,
            meta: None,
        }));

        assert_eq!(
            maybe_request_codex_apps_auth_elicitation(
                &host,
                auth_context(&metadata),
                result.clone()
            )
            .await,
            result
        );
        assert_eq!(host.request_count.load(Ordering::SeqCst), 1);
        assert_eq!(host.refresh_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn inserts_sandbox_state_request_meta() -> anyhow::Result<()> {
        let meta = insert_sandbox_state_request_meta(
            Some(serde_json::json!({ "existing": true })),
            SandboxState {
                permission_profile: Some(PermissionProfile::Disabled),
                sandbox_policy: SandboxPolicy::ReadOnly {
                    network_access: false,
                },
                codex_linux_sandbox_exe: None,
                sandbox_cwd: std::path::PathBuf::from("/tmp/workspace"),
                use_legacy_landlock: false,
            },
        )?
        .expect("meta");

        assert_eq!(meta["existing"], serde_json::json!(true));
        assert_eq!(
            meta[MCP_SANDBOX_STATE_META_CAPABILITY]["sandboxCwd"],
            serde_json::json!("/tmp/workspace")
        );

        Ok(())
    }
}
