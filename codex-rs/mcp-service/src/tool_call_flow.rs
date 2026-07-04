use std::time::Duration;

use codex_config_types::AppToolApproval;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::McpToolApprovalDecision;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::truncate_mcp_tool_result_for_event;
use protocol::items::TurnItem;
use protocol::mcp::CallToolResult;
use protocol::protocol::AskForApproval;
use protocol::protocol::McpInvocation;
use serde_json::Value as JsonValue;
use tracing::error;

use crate::AppToolPolicy;
use crate::CodexAppsAuthElicitationContext;
use crate::McpApprovedToolCallLifecycleContext;
use crate::McpApprovedToolCallLifecycleHost;
use crate::McpToolMetadataLookupHost;
use crate::build_mcp_tool_call_completed_item;
use crate::build_mcp_tool_call_started_item;
use crate::handle_approved_mcp_tool_call;

// Keep the MCP event result cap aligned with codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP
// without making mcp-service depend on the pty crate for a constant.
const MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES: usize = 1024 * 1024;

/// Host capabilities needed by the top-level MCP tool-call flow.
///
/// Implementations inject concrete metadata lookup, approval review, event
/// emission, metrics, and execution side effects. The MCP runtime owns the
/// state-machine ordering for start/skip/approval/approved execution.
pub trait McpToolCallHost: McpApprovedToolCallLifecycleHost + McpToolMetadataLookupHost {
    fn codex_app_tool_policy(
        &self,
        metadata: Option<&McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> AppToolPolicy;

    fn custom_mcp_tool_approval_mode(
        &self,
        server: &str,
        tool_name: &str,
    ) -> impl std::future::Future<Output = AppToolApproval> + Send;

    fn is_host_owned_codex_apps_server(
        &self,
        server: &str,
    ) -> impl std::future::Future<Output = bool> + Send;

    fn request_mcp_tool_approval(
        &self,
        call_id: &str,
        invocation: &McpInvocation,
        hook_tool_name: &str,
        metadata: Option<&McpToolApprovalMetadata>,
        approval_mode: AppToolApproval,
    ) -> impl std::future::Future<Output = Option<McpToolApprovalDecision>> + Send;

    fn emit_mcp_tool_call_started(
        &self,
        item: TurnItem,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn emit_mcp_call_count_status_only(&self, status: &str);

    fn emit_mcp_call_count_with_tags(
        &self,
        status: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
    );
}

#[derive(Debug, Clone)]
pub struct McpToolCallContext {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub server: String,
    pub tool_name: String,
    pub hook_tool_name: String,
    pub arguments: String,
    pub turn_metadata: Option<JsonValue>,
    pub turn_metadata_header_name: &'static str,
    pub supports_image_input: bool,
    pub auth_elicitation_enabled: bool,
    pub approval_policy: AskForApproval,
}

#[derive(Debug, Clone)]
pub struct McpToolCallOutcome {
    pub result: CallToolResult,
    pub tool_input: JsonValue,
}

pub async fn handle_mcp_tool_call(
    host: &impl McpToolCallHost,
    context: McpToolCallContext,
) -> McpToolCallOutcome {
    let arguments_value = if context.arguments.trim().is_empty() {
        None
    } else {
        match serde_json::from_str::<serde_json::Value>(&context.arguments) {
            Ok(value) => Some(value),
            Err(error) => {
                error!("failed to parse tool call arguments: {error}");
                return McpToolCallOutcome {
                    result: CallToolResult::from_error_text(format!("err: {error}")),
                    tool_input: JsonValue::Object(serde_json::Map::new()),
                };
            }
        }
    };

    let invocation = McpInvocation {
        server: context.server.clone(),
        tool: context.tool_name.clone(),
        arguments: arguments_value.clone(),
    };

    let metadata = crate::lookup_mcp_tool_metadata(host, &context.server, &context.tool_name).await;
    let mcp_app_resource_uri = metadata
        .as_ref()
        .and_then(|metadata| metadata.mcp_app_resource_uri.clone());
    let app_tool_policy = if context.server == CODEX_APPS_MCP_SERVER_NAME {
        host.codex_app_tool_policy(metadata.as_ref(), &context.tool_name)
    } else {
        AppToolPolicy::default()
    };
    let approval_mode = if context.server == CODEX_APPS_MCP_SERVER_NAME {
        app_tool_policy.approval
    } else {
        host.custom_mcp_tool_approval_mode(&context.server, &context.tool_name)
            .await
    };

    if context.server == CODEX_APPS_MCP_SERVER_NAME && !app_tool_policy.enabled {
        let result = notify_mcp_tool_call_skip(
            host,
            &context.call_id,
            invocation,
            mcp_app_resource_uri,
            "MCP tool call blocked by app configuration".to_string(),
            /*already_started*/ false,
        )
        .await;
        let status = if result.is_ok() { "ok" } else { "error" };
        host.emit_mcp_call_count_status_only(status);
        return McpToolCallOutcome {
            result: CallToolResult::from_result(result),
            tool_input: arguments_value
                .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
        };
    }

    let connector_id = metadata
        .as_ref()
        .and_then(|metadata| metadata.connector_id.clone());
    let connector_name = metadata
        .as_ref()
        .and_then(|metadata| metadata.connector_name.clone());

    host.emit_mcp_tool_call_started(build_mcp_tool_call_started_item(
        &context.call_id,
        invocation.clone(),
        mcp_app_resource_uri.clone(),
    ))
    .await;

    if let Some(decision) = host
        .request_mcp_tool_approval(
            &context.call_id,
            &invocation,
            &context.hook_tool_name,
            metadata.as_ref(),
            approval_mode,
        )
        .await
    {
        let result = match decision {
            McpToolApprovalDecision::Accept
            | McpToolApprovalDecision::AcceptForSession
            | McpToolApprovalDecision::AcceptAndRemember => {
                return handle_approved_tool_call(
                    host,
                    context,
                    invocation,
                    metadata.as_ref(),
                    mcp_app_resource_uri,
                )
                .await;
            }
            McpToolApprovalDecision::Decline { message } => {
                let message = message.unwrap_or_else(|| "user rejected MCP tool call".to_string());
                notify_mcp_tool_call_skip(
                    host,
                    &context.call_id,
                    invocation,
                    mcp_app_resource_uri,
                    message,
                    /*already_started*/ true,
                )
                .await
            }
            McpToolApprovalDecision::Cancel => {
                notify_mcp_tool_call_skip(
                    host,
                    &context.call_id,
                    invocation,
                    mcp_app_resource_uri,
                    "user cancelled MCP tool call".to_string(),
                    /*already_started*/ true,
                )
                .await
            }
            McpToolApprovalDecision::BlockedBySafetyMonitor(message) => {
                notify_mcp_tool_call_skip(
                    host,
                    &context.call_id,
                    invocation,
                    mcp_app_resource_uri,
                    message,
                    /*already_started*/ true,
                )
                .await
            }
        };

        let status = if result.is_ok() { "ok" } else { "error" };
        host.emit_mcp_call_count_with_tags(
            status,
            &context.tool_name,
            connector_id.as_deref(),
            connector_name.as_deref(),
        );

        return McpToolCallOutcome {
            result: CallToolResult::from_result(result),
            tool_input: arguments_value
                .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
        };
    }

    handle_approved_tool_call(
        host,
        context,
        invocation,
        metadata.as_ref(),
        mcp_app_resource_uri,
    )
    .await
}

async fn handle_approved_tool_call(
    host: &impl McpToolCallHost,
    context: McpToolCallContext,
    invocation: McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
    mcp_app_resource_uri: Option<String>,
) -> McpToolCallOutcome {
    let is_host_owned_codex_apps_server = host
        .is_host_owned_codex_apps_server(&invocation.server)
        .await;
    let outcome = handle_approved_mcp_tool_call(
        host,
        McpApprovedToolCallLifecycleContext {
            thread_id: &context.thread_id,
            turn_id: &context.turn_id,
            call_id: &context.call_id,
            invocation,
            metadata,
            mcp_app_resource_uri,
            turn_metadata: context.turn_metadata,
            turn_metadata_header_name: context.turn_metadata_header_name,
            supports_image_input: context.supports_image_input,
            auth_elicitation_context: CodexAppsAuthElicitationContext {
                is_host_owned_codex_apps_server,
                auth_elicitation_enabled: context.auth_elicitation_enabled,
                approval_policy: context.approval_policy,
                thread_id: &context.thread_id,
                turn_id: Some(&context.turn_id),
                call_id: &context.call_id,
                metadata,
            },
        },
    )
    .await;
    McpToolCallOutcome {
        result: outcome.result,
        tool_input: outcome.tool_input,
    }
}

async fn notify_mcp_tool_call_skip(
    host: &impl McpToolCallHost,
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
    message: String,
    already_started: bool,
) -> Result<CallToolResult, String> {
    if !already_started {
        host.emit_mcp_tool_call_started(build_mcp_tool_call_started_item(
            call_id,
            invocation.clone(),
            mcp_app_resource_uri.clone(),
        ))
        .await;
    }

    host.emit_mcp_tool_call_completed(build_mcp_tool_call_completed_item(
        call_id,
        invocation,
        mcp_app_resource_uri,
        Duration::ZERO,
        truncate_mcp_tool_result_for_event(
            &Err(message.clone()),
            MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES,
        ),
    ))
    .await;
    Err(message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use codex_config_types::AppToolApproval;
    use protocol::items::McpToolCallStatus;
    use protocol::items::TurnItem;

    use super::*;
    use crate::McpToolExecutionHost;

    #[derive(Default)]
    struct FakeTopLevelHost {
        metadata: Option<McpToolApprovalMetadata>,
        app_tool_policy: AppToolPolicy,
        approval_decision: Option<McpToolApprovalDecision>,
        started_items: Arc<Mutex<Vec<TurnItem>>>,
        completed_items: Arc<Mutex<Vec<TurnItem>>>,
        count_metrics: Arc<Mutex<Vec<String>>>,
        tagged_metrics: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl crate::CodexAppsAuthElicitationHost for FakeTopLevelHost {
        async fn request_codex_apps_auth_elicitation(
            &self,
            _request_id: protocol::mcp::RequestId,
            _params: mcp_types::McpServerElicitationRequestParams,
        ) -> Option<mcp_types::ElicitationResponse> {
            None
        }

        async fn refresh_codex_apps_after_connector_auth(&self) {}
    }

    impl McpToolExecutionHost for FakeTopLevelHost {
        async fn augment_mcp_tool_request_meta_with_sandbox_state(
            &self,
            _server: &str,
            meta: Option<JsonValue>,
        ) -> anyhow::Result<Option<JsonValue>> {
            Ok(meta)
        }

        fn add_mcp_call_trace_request_meta(
            &self,
            _call_id: &str,
            meta: Option<JsonValue>,
        ) -> Option<JsonValue> {
            meta
        }

        async fn call_mcp_tool(
            &self,
            _server: &str,
            _tool: &str,
            _arguments: Option<JsonValue>,
            _meta: Option<JsonValue>,
        ) -> Result<CallToolResult, String> {
            Ok(CallToolResult {
                content: vec![serde_json::json!({"type": "text", "text": "ok"})],
                structured_content: None,
                is_error: None,
                meta: None,
            })
        }
    }

    impl McpApprovedToolCallLifecycleHost for FakeTopLevelHost {
        async fn mark_thread_memory_mode_polluted_if_needed(&self, _server: &str) {}

        async fn server_origin(&self, _server: &str) -> Option<String> {
            None
        }

        async fn rewrite_mcp_tool_arguments_for_openai_files(
            &self,
            arguments: Option<JsonValue>,
            _openai_file_input_params: Option<&[String]>,
        ) -> Result<Option<JsonValue>, String> {
            Ok(arguments)
        }

        async fn emit_mcp_tool_call_completed(&self, item: TurnItem) {
            self.completed_items.lock().unwrap().push(item);
        }

        async fn track_codex_app_used(&self, _server: &str, _tool_name: &str) {}

        fn emit_mcp_call_metrics(
            &self,
            status: &str,
            tool_name: &str,
            _connector_id: Option<&str>,
            _connector_name: Option<&str>,
            _duration: Duration,
        ) {
            self.tagged_metrics
                .lock()
                .unwrap()
                .push((status.to_string(), tool_name.to_string()));
        }
    }

    impl crate::McpToolMetadataLookupHost for FakeTopLevelHost {
        async fn list_all_mcp_tools(&self) -> Vec<mcp_types::ToolInfo> {
            Vec::new()
        }

        async fn codex_apps_auth_snapshot(&self) -> Option<codex_auth_types::RequestAuthSnapshot> {
            None
        }

        async fn cached_accessible_connectors(
            &self,
            _auth_snapshot: Option<&codex_auth_types::RequestAuthSnapshot>,
        ) -> Option<Vec<codex_connectors_api::AppInfo>> {
            None
        }

        async fn fetch_accessible_connectors(
            &self,
            _auth_snapshot: Option<&codex_auth_types::RequestAuthSnapshot>,
        ) -> anyhow::Result<Vec<codex_connectors_api::AppInfo>> {
            Ok(Vec::new())
        }
    }

    impl McpToolCallHost for FakeTopLevelHost {
        fn codex_app_tool_policy(
            &self,
            _metadata: Option<&McpToolApprovalMetadata>,
            _tool_name: &str,
        ) -> AppToolPolicy {
            self.app_tool_policy
        }

        async fn custom_mcp_tool_approval_mode(
            &self,
            _server: &str,
            _tool_name: &str,
        ) -> AppToolApproval {
            AppToolApproval::Auto
        }

        async fn is_host_owned_codex_apps_server(&self, server: &str) -> bool {
            server == CODEX_APPS_MCP_SERVER_NAME
        }

        async fn request_mcp_tool_approval(
            &self,
            _call_id: &str,
            _invocation: &McpInvocation,
            _hook_tool_name: &str,
            _metadata: Option<&McpToolApprovalMetadata>,
            _approval_mode: AppToolApproval,
        ) -> Option<McpToolApprovalDecision> {
            self.approval_decision.clone()
        }

        async fn emit_mcp_tool_call_started(&self, item: TurnItem) {
            self.started_items.lock().unwrap().push(item);
        }

        fn emit_mcp_call_count_status_only(&self, status: &str) {
            self.count_metrics.lock().unwrap().push(status.to_string());
        }

        fn emit_mcp_call_count_with_tags(
            &self,
            status: &str,
            tool_name: &str,
            _connector_id: Option<&str>,
            _connector_name: Option<&str>,
        ) {
            self.tagged_metrics
                .lock()
                .unwrap()
                .push((status.to_string(), tool_name.to_string()));
        }
    }

    fn context(server: &str) -> McpToolCallContext {
        McpToolCallContext {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            call_id: "call-1".to_string(),
            server: server.to_string(),
            tool_name: "create_event".to_string(),
            hook_tool_name: "mcp__calendar__create_event".to_string(),
            arguments: r#"{"title":"Review"}"#.to_string(),
            turn_metadata: None,
            turn_metadata_header_name: "x-codex-turn-metadata",
            supports_image_input: false,
            auth_elicitation_enabled: false,
            approval_policy: AskForApproval::OnRequest,
        }
    }

    #[tokio::test]
    async fn disabled_codex_app_tool_emits_start_and_completed_skip() {
        let host = FakeTopLevelHost {
            app_tool_policy: AppToolPolicy {
                enabled: false,
                approval: AppToolApproval::Prompt,
            },
            ..Default::default()
        };

        let outcome = handle_mcp_tool_call(&host, context(CODEX_APPS_MCP_SERVER_NAME)).await;

        assert!(outcome.result.is_error.unwrap_or(false));
        assert_eq!(host.started_items.lock().unwrap().len(), 1);
        assert_eq!(host.completed_items.lock().unwrap().len(), 1);
        assert_eq!(
            host.count_metrics.lock().unwrap().as_slice(),
            &["error".to_string()]
        );
    }

    #[tokio::test]
    async fn approval_decline_emits_skip_after_started_and_tagged_metric() {
        let host = FakeTopLevelHost {
            approval_decision: Some(McpToolApprovalDecision::Decline {
                message: Some("no".to_string()),
            }),
            ..Default::default()
        };

        let outcome = handle_mcp_tool_call(&host, context("custom")).await;

        assert!(outcome.result.is_error.unwrap_or(false));
        assert_eq!(host.started_items.lock().unwrap().len(), 1);
        let completed_items = host.completed_items.lock().unwrap();
        assert_eq!(completed_items.len(), 1);
        let TurnItem::McpToolCall(item) = &completed_items[0] else {
            panic!("expected MCP tool call item");
        };
        assert_eq!(item.status, McpToolCallStatus::Failed);
        assert_eq!(
            host.tagged_metrics.lock().unwrap().as_slice(),
            &[("error".to_string(), "create_event".to_string())]
        );
    }
}
