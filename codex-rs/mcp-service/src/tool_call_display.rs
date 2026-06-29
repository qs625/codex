use std::time::Duration;

use codex_mcp_types::MCP_RESULT_TELEMETRY_SERVER_USER_FLOW_SPAN_ATTR;
use codex_mcp_types::MCP_RESULT_TELEMETRY_TARGET_ID_SPAN_ATTR;
use codex_mcp_types::mcp_tool_call_result_span_telemetry;
use codex_mcp_types::mcp_tool_call_server_fields;
use codex_protocol::items::McpToolCallError;
use codex_protocol::items::McpToolCallItem;
use codex_protocol::items::McpToolCallStatus;
use codex_protocol::items::TurnItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::protocol::McpInvocation;
use codex_utils_string::sanitize_metric_tag_value;
use serde_json::Value as JsonValue;
use tracing::Span;
use tracing::field::Empty;

pub const MCP_CALL_COUNT_METRIC: &str = "codex.mcp.call";
pub const MCP_CALL_DURATION_METRIC: &str = "codex.mcp.call.duration_ms";

pub fn mcp_call_metric_tags(
    status: &str,
    tool_name: &str,
    connector_id: Option<&str>,
    connector_name: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut tags = vec![
        ("status", sanitize_metric_tag_value(status)),
        ("tool", sanitize_metric_tag_value(tool_name)),
    ];
    if let Some(connector_id) = connector_id.filter(|connector_id| !connector_id.is_empty()) {
        tags.push(("connector_id", sanitize_metric_tag_value(connector_id)));
    }
    if let Some(connector_name) = connector_name.filter(|connector_name| !connector_name.is_empty())
    {
        tags.push(("connector_name", sanitize_metric_tag_value(connector_name)));
    }
    tags
}

pub struct McpToolCallSpanFields<'a> {
    pub server_name: &'a str,
    pub tool_name: &'a str,
    pub call_id: &'a str,
    pub server_origin: Option<&'a str>,
    pub connector_id: Option<&'a str>,
    pub connector_name: Option<&'a str>,
    pub conversation_id: &'a str,
    pub session_id: &'a str,
    pub turn_id: &'a str,
}

pub fn mcp_tool_call_span(fields: McpToolCallSpanFields<'_>) -> Span {
    let transport = match fields.server_origin {
        Some("stdio") => "stdio",
        Some("in_process") => "in_process",
        Some(_) => "streamable_http",
        None => "",
    };
    let span = tracing::info_span!(
        "mcp.tools.call",
        otel.kind = "client",
        rpc.system = "jsonrpc",
        rpc.method = "tools/call",
        mcp.server.name = fields.server_name,
        mcp.server.origin = fields.server_origin.unwrap_or(""),
        mcp.transport = transport,
        mcp.connector.id = fields.connector_id.unwrap_or(""),
        mcp.connector.name = fields.connector_name.unwrap_or(""),
        tool.name = fields.tool_name,
        tool.call_id = fields.call_id,
        conversation.id = fields.conversation_id,
        session.id = fields.session_id,
        turn.id = fields.turn_id,
        server.address = Empty,
        server.port = Empty,
        codex.mcp.target.id = Empty,
        codex.mcp.server_user_flow.triggered = Empty,
    );
    record_server_fields(&span, fields.server_origin);
    span
}

fn record_server_fields(span: &Span, url: Option<&str>) {
    let Some(url) = url else {
        return;
    };
    let Some(fields) = mcp_tool_call_server_fields(url) else {
        return;
    };
    span.record("server.address", fields.host.as_str());
    if let Some(port) = fields.port {
        span.record("server.port", port as i64);
    }
}

pub fn record_mcp_result_span_telemetry(span: &Span, result: Option<&CallToolResult>) {
    let Some(telemetry) = result.and_then(mcp_tool_call_result_span_telemetry) else {
        return;
    };

    if let Some(target_id) = telemetry.target_id {
        span.record(MCP_RESULT_TELEMETRY_TARGET_ID_SPAN_ATTR, target_id.as_str());
    }

    if let Some(did_trigger_server_user_flow) = telemetry.did_trigger_server_user_flow {
        span.record(
            MCP_RESULT_TELEMETRY_SERVER_USER_FLOW_SPAN_ATTR,
            did_trigger_server_user_flow,
        );
    }
}

pub fn build_mcp_tool_call_started_item(
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
) -> TurnItem {
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    TurnItem::McpToolCall(McpToolCallItem {
        id: call_id.to_string(),
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri,
        status: McpToolCallStatus::InProgress,
        result: None,
        error: None,
        duration: None,
    })
}

pub fn build_mcp_tool_call_completed_item(
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
    duration: Duration,
    result: Result<CallToolResult, String>,
) -> TurnItem {
    let (status, result, error) = match result {
        Ok(result) if result.is_error.unwrap_or(false) => {
            (McpToolCallStatus::Failed, Some(result), None)
        }
        Ok(result) => (McpToolCallStatus::Completed, Some(result), None),
        Err(message) => (
            McpToolCallStatus::Failed,
            None,
            Some(McpToolCallError { message }),
        ),
    };
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    TurnItem::McpToolCall(McpToolCallItem {
        id: call_id.to_string(),
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri,
        status,
        result,
        error,
        duration: Some(duration),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_mcp_types::MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS;
    use tracing::Instrument;
    use tracing::Level;
    use tracing_subscriber::fmt::format::FmtSpan;
    use tracing_test::internal::MockWriter;

    fn mcp_tool_call_item(item: TurnItem) -> McpToolCallItem {
        match item {
            TurnItem::McpToolCall(item) => item,
            _ => panic!("expected MCP tool call item"),
        }
    }

    #[tokio::test]
    async fn mcp_tool_call_span_records_expected_fields() {
        let buffer: &'static std::sync::Mutex<Vec<u8>> =
            Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_level(true)
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_span_events(FmtSpan::FULL)
            .with_writer(MockWriter::new(buffer))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        async {}
            .instrument(mcp_tool_call_span(McpToolCallSpanFields {
                server_name: "rmcp",
                tool_name: "echo",
                call_id: "call-123",
                server_origin: Some("https://example.com:8443/mcp"),
                connector_id: Some("calendar"),
                connector_name: Some("Calendar"),
                conversation_id: "conversation-123",
                session_id: "session-123",
                turn_id: "turn-123",
            }))
            .await;

        let logs =
            String::from_utf8(buffer.lock().expect("buffer lock").clone()).expect("utf8 logs");
        assert!(
            logs.contains("mcp.tools.call{otel.kind=\"client\"")
                && logs.contains("rpc.system=\"jsonrpc\"")
                && logs.contains("rpc.method=\"tools/call\"")
                && logs.contains("mcp.server.name=\"rmcp\"")
                && logs.contains("mcp.server.origin=\"https://example.com:8443/mcp\"")
                && logs.contains("mcp.transport=\"streamable_http\"")
                && logs.contains("mcp.connector.id=\"calendar\"")
                && logs.contains("mcp.connector.name=\"Calendar\"")
                && logs.contains("tool.name=\"echo\"")
                && logs.contains("tool.call_id=\"call-123\"")
                && logs.contains("server.address=\"example.com\"")
                && logs.contains("server.port=8443")
                && logs.contains("conversation.id=\"conversation-123\"")
                && logs.contains("session.id=\"session-123\"")
                && logs.contains("turn.id=\"turn-123\""),
            "missing MCP tool span fields\nlogs:\n{logs}"
        );
    }

    async fn mcp_result_telemetry_span_logs(meta: Option<serde_json::Value>) -> String {
        let buffer: &'static std::sync::Mutex<Vec<u8>> =
            Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_level(true)
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_span_events(FmtSpan::FULL)
            .with_writer(MockWriter::new(buffer))
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let result = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta,
        };

        {
            let span = mcp_tool_call_span(McpToolCallSpanFields {
                server_name: "rmcp",
                tool_name: "echo",
                call_id: "call-123",
                server_origin: None,
                connector_id: None,
                connector_name: None,
                conversation_id: "conversation-123",
                session_id: "session-123",
                turn_id: "turn-123",
            });

            async {
                record_mcp_result_span_telemetry(&Span::current(), Some(&result));
            }
            .instrument(span)
            .await;
        }

        String::from_utf8(buffer.lock().expect("buffer lock").clone()).expect("utf8 logs")
    }

    #[tokio::test]
    async fn mcp_result_telemetry_records_allowlisted_span_fields() {
        let logs = mcp_result_telemetry_span_logs(Some(serde_json::json!({
            "codex/telemetry": {
                "span": {
                    "target_id": "com.apple.reminders",
                    "did_trigger_server_user_flow": false,
                    "not_promoted_sentinel_key": "not_promoted_sentinel_value",
                },
            },
        })))
        .await;

        assert!(
            logs.contains("codex.mcp.target.id=\"com.apple.reminders\"")
                && logs.contains("codex.mcp.server_user_flow.triggered=false"),
            "missing MCP result telemetry span fields\nlogs:\n{logs}"
        );
        assert!(
            !logs.contains("not_promoted_sentinel_key")
                && !logs.contains("not_promoted_sentinel_value"),
            "unknown MCP result telemetry keys should be ignored\nlogs:\n{logs}"
        );
    }

    #[tokio::test]
    async fn mcp_result_telemetry_ignores_invalid_and_missing_values() {
        let invalid_logs = mcp_result_telemetry_span_logs(Some(serde_json::json!({
            "codex/telemetry": {
                "span": {
                    "target_id": 123,
                    "did_trigger_server_user_flow": "false",
                },
            },
        })))
        .await;
        assert!(
            !invalid_logs.contains("codex.mcp.target.id=")
                && !invalid_logs.contains("codex.mcp.server_user_flow.triggered="),
            "invalid MCP result telemetry values should be ignored\nlogs:\n{invalid_logs}"
        );

        let missing_logs = mcp_result_telemetry_span_logs(Some(serde_json::json!({
            "codex/telemetry": {},
        })))
        .await;
        assert!(
            !missing_logs.contains("codex.mcp.target.id=")
                && !missing_logs.contains("codex.mcp.server_user_flow.triggered="),
            "missing MCP result telemetry span object should be ignored\nlogs:\n{missing_logs}"
        );

        let no_meta_logs = mcp_result_telemetry_span_logs(/*meta*/ None).await;
        assert!(
            !no_meta_logs.contains("codex.mcp.target.id=")
                && !no_meta_logs.contains("codex.mcp.server_user_flow.triggered="),
            "missing MCP result metadata should be ignored\nlogs:\n{no_meta_logs}"
        );
    }

    #[tokio::test]
    async fn mcp_result_telemetry_truncates_long_target_id() {
        let truncated = "x".repeat(MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS);
        let target_id = format!("{truncated}tail");
        let logs = mcp_result_telemetry_span_logs(Some(serde_json::json!({
            "codex/telemetry": {
                "span": {
                    "target_id": target_id,
                },
            },
        })))
        .await;

        assert!(
            logs.contains(&format!("codex.mcp.target.id=\"{truncated}\""))
                && !logs.contains("tail"),
            "long MCP result telemetry target_id should be truncated\nlogs:\n{logs}"
        );
    }

    #[test]
    fn build_mcp_tool_call_started_item_defaults_arguments_to_null() {
        let item = mcp_tool_call_item(build_mcp_tool_call_started_item(
            "call-123",
            McpInvocation {
                server: "server".to_string(),
                tool: "tool".to_string(),
                arguments: None,
            },
            Some("resource://app".to_string()),
        ));

        assert_eq!(item.id, "call-123");
        assert_eq!(item.server, "server");
        assert_eq!(item.tool, "tool");
        assert_eq!(item.arguments, JsonValue::Null);
        assert_eq!(
            item.mcp_app_resource_uri,
            Some("resource://app".to_string())
        );
        assert!(matches!(item.status, McpToolCallStatus::InProgress));
        assert!(item.result.is_none());
        assert!(item.error.is_none());
        assert!(item.duration.is_none());
    }

    #[test]
    fn build_mcp_tool_call_completed_item_marks_success_error_and_transport_error() {
        let invocation = || McpInvocation {
            server: "server".to_string(),
            tool: "tool".to_string(),
            arguments: Some(serde_json::json!({"key": "value"})),
        };
        let duration = Duration::from_millis(12);
        let successful_result = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: Some(false),
            meta: None,
        };
        let tool_error_result = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: Some(true),
            meta: None,
        };

        let item = mcp_tool_call_item(build_mcp_tool_call_completed_item(
            "call-123",
            invocation(),
            /*mcp_app_resource_uri*/ None,
            duration,
            Ok(successful_result.clone()),
        ));
        assert_eq!(item.id, "call-123");
        assert_eq!(item.server, "server");
        assert_eq!(item.tool, "tool");
        assert_eq!(item.arguments, serde_json::json!({"key": "value"}));
        assert!(item.mcp_app_resource_uri.is_none());
        assert!(matches!(item.status, McpToolCallStatus::Completed));
        assert_eq!(item.result, Some(successful_result));
        assert!(item.error.is_none());
        assert_eq!(item.duration, Some(duration));

        let item = mcp_tool_call_item(build_mcp_tool_call_completed_item(
            "call-123",
            invocation(),
            /*mcp_app_resource_uri*/ None,
            duration,
            Ok(tool_error_result.clone()),
        ));
        assert_eq!(item.id, "call-123");
        assert_eq!(item.server, "server");
        assert_eq!(item.tool, "tool");
        assert_eq!(item.arguments, serde_json::json!({"key": "value"}));
        assert!(item.mcp_app_resource_uri.is_none());
        assert!(matches!(item.status, McpToolCallStatus::Failed));
        assert_eq!(item.result, Some(tool_error_result));
        assert!(item.error.is_none());
        assert_eq!(item.duration, Some(duration));

        let item = mcp_tool_call_item(build_mcp_tool_call_completed_item(
            "call-123",
            invocation(),
            /*mcp_app_resource_uri*/ None,
            duration,
            Err("transport failed".to_string()),
        ));
        assert_eq!(item.id, "call-123");
        assert_eq!(item.server, "server");
        assert_eq!(item.tool, "tool");
        assert_eq!(item.arguments, serde_json::json!({"key": "value"}));
        assert!(item.mcp_app_resource_uri.is_none());
        assert!(matches!(item.status, McpToolCallStatus::Failed));
        assert!(item.result.is_none());
        assert_eq!(
            item.error.map(|error| error.message),
            Some("transport failed".to_string())
        );
        assert_eq!(item.duration, Some(duration));
    }
}
