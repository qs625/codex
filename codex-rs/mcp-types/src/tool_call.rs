use protocol::mcp::CallToolResult;
use serde_json::Map;
use serde_json::Value;

use crate::CODEX_APPS_MCP_SERVER_NAME;
use crate::declared_openai_file_input_param_names;

pub const MCP_RESULT_TELEMETRY_TARGET_ID_SPAN_ATTR: &str = "codex.mcp.target.id";
pub const MCP_RESULT_TELEMETRY_SERVER_USER_FLOW_SPAN_ATTR: &str =
    "codex.mcp.server_user_flow.triggered";
pub const MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS: usize = 256;
pub const MCP_TOOL_OPENAI_OUTPUT_TEMPLATE_META_KEY: &str = "openai/outputTemplate";
pub const MCP_TOOL_UI_RESOURCE_URI_META_KEY: &str = "ui/resourceUri";
pub const MCP_TOOL_THREAD_ID_META_KEY: &str = "threadId";

const MCP_RESULT_TELEMETRY_META_KEY: &str = "codex/telemetry";
const MCP_RESULT_TELEMETRY_SPAN_KEY: &str = "span";
const MCP_RESULT_TELEMETRY_TARGET_ID_KEY: &str = "target_id";
const MCP_RESULT_TELEMETRY_DID_TRIGGER_SERVER_USER_FLOW_KEY: &str = "did_trigger_server_user_flow";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolCallServerFields {
    pub host: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolCallResultSpanTelemetry {
    pub target_id: Option<String>,
    pub did_trigger_server_user_flow: Option<bool>,
}

pub fn mcp_tool_call_server_fields(url: &str) -> Option<McpToolCallServerFields> {
    let uri = url.parse::<http::Uri>().ok()?;
    let authority = uri.authority()?;
    let host = normalize_uri_host(authority.host()).to_string();
    if host.is_empty() {
        return None;
    }
    let port = authority.port_u16().or_else(|| {
        let scheme = uri.scheme_str()?;
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("ws") {
            Some(80)
        } else if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("wss") {
            Some(443)
        } else {
            None
        }
    });
    Some(McpToolCallServerFields { host, port })
}

fn normalize_uri_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
}

pub fn mcp_tool_call_result_span_telemetry(
    result: &CallToolResult,
) -> Option<McpToolCallResultSpanTelemetry> {
    let span_telemetry = result
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get(MCP_RESULT_TELEMETRY_META_KEY))
        .and_then(Value::as_object)
        .and_then(|telemetry| telemetry.get(MCP_RESULT_TELEMETRY_SPAN_KEY))
        .and_then(Value::as_object)?;

    let target_id = span_telemetry
        .get(MCP_RESULT_TELEMETRY_TARGET_ID_KEY)
        .and_then(Value::as_str)
        .filter(|target_id| !target_id.is_empty())
        .map(|target_id| {
            truncate_str_to_char_boundary(target_id, MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS)
                .to_string()
        });
    let did_trigger_server_user_flow = span_telemetry
        .get(MCP_RESULT_TELEMETRY_DID_TRIGGER_SERVER_USER_FLOW_KEY)
        .and_then(Value::as_bool);

    (target_id.is_some() || did_trigger_server_user_flow.is_some()).then_some(
        McpToolCallResultSpanTelemetry {
            target_id,
            did_trigger_server_user_flow,
        },
    )
}

fn truncate_str_to_char_boundary(value: &str, max_chars: usize) -> &str {
    match value.char_indices().nth(max_chars) {
        Some((index, _)) => &value[..index],
        None => value,
    }
}

pub fn mcp_app_resource_uri_from_tool_meta(meta: Option<&Map<String, Value>>) -> Option<String> {
    meta.and_then(|meta| {
        meta.get("ui")
            .and_then(Value::as_object)
            .and_then(|ui| ui.get("resourceUri"))
            .and_then(Value::as_str)
            .or_else(|| {
                meta.get(MCP_TOOL_UI_RESOURCE_URI_META_KEY)
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                meta.get(MCP_TOOL_OPENAI_OUTPUT_TEMPLATE_META_KEY)
                    .and_then(Value::as_str)
            })
            .map(str::to_string)
    })
}

pub fn openai_file_input_params_for_server(
    server: &str,
    meta: Option<&Map<String, Value>>,
) -> Option<Vec<String>> {
    (server == CODEX_APPS_MCP_SERVER_NAME)
        .then_some(declared_openai_file_input_param_names(meta))
        .filter(|params| !params.is_empty())
}

pub fn with_mcp_tool_call_thread_id_meta(meta: Option<Value>, thread_id: &str) -> Option<Value> {
    match meta {
        Some(Value::Object(mut map)) => {
            map.insert(
                MCP_TOOL_THREAD_ID_META_KEY.to_string(),
                Value::String(thread_id.to_string()),
            );
            Some(Value::Object(map))
        }
        None => {
            let mut map = Map::new();
            map.insert(
                MCP_TOOL_THREAD_ID_META_KEY.to_string(),
                Value::String(thread_id.to_string()),
            );
            Some(Value::Object(map))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_fields_extracts_host_and_default_port() {
        assert_eq!(
            mcp_tool_call_server_fields("https://example.com/mcp"),
            Some(McpToolCallServerFields {
                host: "example.com".to_string(),
                port: Some(443),
            })
        );
        assert_eq!(
            mcp_tool_call_server_fields("http://example.com/mcp"),
            Some(McpToolCallServerFields {
                host: "example.com".to_string(),
                port: Some(80),
            })
        );
    }

    #[test]
    fn server_fields_prefers_explicit_port() {
        assert_eq!(
            mcp_tool_call_server_fields("https://example.com:8443/mcp"),
            Some(McpToolCallServerFields {
                host: "example.com".to_string(),
                port: Some(8443),
            })
        );
    }

    #[test]
    fn server_fields_normalizes_ipv6_host() {
        assert_eq!(
            mcp_tool_call_server_fields("https://[::1]:8443/mcp"),
            Some(McpToolCallServerFields {
                host: "::1".to_string(),
                port: Some(8443),
            })
        );
    }

    #[test]
    fn server_fields_ignores_invalid_origin() {
        assert_eq!(mcp_tool_call_server_fields("/relative/path"), None);
        assert_eq!(mcp_tool_call_server_fields("not a url"), None);
    }

    #[test]
    fn mcp_app_resource_uri_reads_known_tool_meta_keys() {
        let nested = serde_json::json!({
            "ui": {
                "resourceUri": "ui://widget/nested.html",
            },
        });
        assert_eq!(
            mcp_app_resource_uri_from_tool_meta(nested.as_object()),
            Some("ui://widget/nested.html".to_string())
        );

        let flat = serde_json::json!({
            "ui/resourceUri": "ui://widget/flat.html",
        });
        assert_eq!(
            mcp_app_resource_uri_from_tool_meta(flat.as_object()),
            Some("ui://widget/flat.html".to_string())
        );

        let output_template = serde_json::json!({
            "openai/outputTemplate": "ui://widget/output-template.html",
        });
        assert_eq!(
            mcp_app_resource_uri_from_tool_meta(output_template.as_object()),
            Some("ui://widget/output-template.html".to_string())
        );
    }

    #[test]
    fn openai_file_params_are_only_honored_for_codex_apps() {
        let meta = serde_json::json!({
            "openai/fileParams": ["file"],
        });
        let meta = meta.as_object();

        assert_eq!(
            openai_file_input_params_for_server(CODEX_APPS_MCP_SERVER_NAME, meta),
            Some(vec!["file".to_string()])
        );
        assert_eq!(
            openai_file_input_params_for_server("minimaltest", meta),
            None
        );
    }

    #[test]
    fn result_span_telemetry_extracts_allowlisted_fields() {
        let result = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta: Some(serde_json::json!({
                "codex/telemetry": {
                    "span": {
                        "target_id": "com.apple.reminders",
                        "did_trigger_server_user_flow": false,
                        "not_promoted_sentinel_key": "not_promoted_sentinel_value",
                    },
                },
            })),
        };

        assert_eq!(
            mcp_tool_call_result_span_telemetry(&result),
            Some(McpToolCallResultSpanTelemetry {
                target_id: Some("com.apple.reminders".to_string()),
                did_trigger_server_user_flow: Some(false),
            })
        );
    }

    #[test]
    fn result_span_telemetry_ignores_invalid_and_missing_values() {
        let invalid = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta: Some(serde_json::json!({
                "codex/telemetry": {
                    "span": {
                        "target_id": 123,
                        "did_trigger_server_user_flow": "false",
                    },
                },
            })),
        };
        assert_eq!(mcp_tool_call_result_span_telemetry(&invalid), None);

        let missing = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta: Some(serde_json::json!({
                "codex/telemetry": {},
            })),
        };
        assert_eq!(mcp_tool_call_result_span_telemetry(&missing), None);

        let no_meta = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta: None,
        };
        assert_eq!(mcp_tool_call_result_span_telemetry(&no_meta), None);
    }

    #[test]
    fn result_span_telemetry_truncates_long_target_id() {
        let truncated = "x".repeat(MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS);
        let target_id = format!("{truncated}tail");
        let result = CallToolResult {
            content: Vec::new(),
            structured_content: None,
            is_error: None,
            meta: Some(serde_json::json!({
                "codex/telemetry": {
                    "span": {
                        "target_id": target_id,
                    },
                },
            })),
        };

        assert_eq!(
            mcp_tool_call_result_span_telemetry(&result),
            Some(McpToolCallResultSpanTelemetry {
                target_id: Some(truncated),
                did_trigger_server_user_flow: None,
            })
        );
    }

    #[test]
    fn truncates_strings_on_char_boundaries() {
        let prefix = "á".repeat(MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS);
        let value = format!("{prefix}tail");
        let truncated =
            truncate_str_to_char_boundary(&value, MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS);

        assert_eq!(truncated, prefix);
        assert_eq!(
            truncate_str_to_char_boundary("short", MCP_RESULT_TELEMETRY_TARGET_ID_MAX_CHARS),
            "short"
        );
    }

    #[test]
    fn thread_id_meta_is_added_to_request_meta() {
        assert_eq!(
            with_mcp_tool_call_thread_id_meta(
                Some(serde_json::json!({
                    "source": "test-client",
                    "threadId": "stale-thread",
                })),
                "thread-live",
            ),
            Some(serde_json::json!({
                "source": "test-client",
                "threadId": "thread-live",
            }))
        );

        assert_eq!(
            with_mcp_tool_call_thread_id_meta(/*meta*/ None, "thread-live"),
            Some(serde_json::json!({
                "threadId": "thread-live",
            }))
        );

        assert_eq!(
            with_mcp_tool_call_thread_id_meta(
                Some(serde_json::json!("invalid-meta")),
                "thread-live"
            ),
            Some(serde_json::json!("invalid-meta"))
        );
    }
}
