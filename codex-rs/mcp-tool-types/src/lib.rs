use codex_protocol::ToolName;
use codex_protocol::mcp::CallToolResult;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value as JsonValue;

const META_OPENAI_FILE_PARAMS: &str = "openai/fileParams";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: JsonValue,
    pub output_schema: Option<JsonValue>,
    pub annotations: Option<ToolAnnotations>,
    pub execution: Option<JsonValue>,
    pub icons: Option<Vec<JsonValue>>,
    #[serde(rename = "_meta")]
    pub meta: Option<Map<String, JsonValue>>,
}

impl McpTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: JsonValue,
    ) -> Self {
        Self {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema,
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Raw MCP server name used for routing the tool call.
    pub server_name: String,
    /// Whether calls routed to this server may run in parallel.
    #[serde(default)]
    pub supports_parallel_tool_calls: bool,
    /// MCP server origin used for telemetry and diagnostics, when known.
    #[serde(default)]
    pub server_origin: Option<String>,
    /// Model-visible tool name used in Responses API tool declarations.
    #[serde(rename = "tool_name", alias = "callable_name")]
    pub callable_name: String,
    /// Model-visible namespace used for deferred tool loading.
    #[serde(rename = "tool_namespace", alias = "callable_namespace")]
    pub callable_namespace: String,
    /// Model-visible namespace description.
    // Keep the old serialized field name readable for cached ToolInfo values.
    #[serde(default, alias = "connector_description")]
    pub namespace_description: Option<String>,
    /// Protocol-neutral MCP tool definition; `tool.name` is sent back to the MCP server.
    pub tool: McpTool,
    pub connector_id: Option<String>,
    pub connector_name: Option<String>,
    #[serde(default)]
    pub plugin_display_names: Vec<String>,
}

impl ToolInfo {
    pub fn canonical_tool_name(&self) -> ToolName {
        ToolName::namespaced(self.callable_namespace.clone(), self.callable_name.clone())
    }
}

pub fn declared_openai_file_input_param_names(
    meta: Option<&Map<String, JsonValue>>,
) -> Vec<String> {
    let Some(meta) = meta else {
        return Vec::new();
    };

    meta.get(META_OPENAI_FILE_PARAMS)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn tool_with_model_visible_input_schema(tool: &McpTool) -> McpTool {
    let file_params = declared_openai_file_input_param_names(tool.meta.as_ref());
    if file_params.is_empty() {
        return tool.clone();
    }

    let mut tool = tool.clone();
    let mut input_schema = tool.input_schema.clone();
    mask_input_schema_for_file_path_params(&mut input_schema, &file_params);
    tool.input_schema = input_schema;
    tool
}

fn mask_input_schema_for_file_path_params(input_schema: &mut JsonValue, file_params: &[String]) {
    let Some(properties) = input_schema
        .as_object_mut()
        .and_then(|schema| schema.get_mut("properties"))
        .and_then(JsonValue::as_object_mut)
    else {
        return;
    };

    for field_name in file_params {
        let Some(property_schema) = properties.get_mut(field_name) else {
            continue;
        };
        mask_input_property_schema(property_schema);
    }
}

pub fn sanitize_mcp_tool_result_for_model(
    supports_image_input: bool,
    result: Result<CallToolResult, String>,
) -> Result<CallToolResult, String> {
    if supports_image_input {
        return result;
    }

    result.map(|call_tool_result| CallToolResult {
        content: call_tool_result
            .content
            .iter()
            .map(|block| {
                if let Some(content_type) = block.get("type").and_then(serde_json::Value::as_str)
                    && content_type == "image"
                {
                    return serde_json::json!({
                        "type": "text",
                        "text": "<image content omitted because you do not support image input>",
                    });
                }

                block.clone()
            })
            .collect::<Vec<_>>(),
        structured_content: call_tool_result.structured_content,
        is_error: call_tool_result.is_error,
        meta: call_tool_result.meta,
    })
}

pub fn truncate_mcp_tool_result_for_event(
    result: &Result<CallToolResult, String>,
    max_bytes: usize,
) -> Result<CallToolResult, String> {
    match result {
        Ok(call_tool_result) => {
            // The app-server rebuilds `ThreadItem::McpToolCall` from this item,
            // so avoid persisting multi-megabyte results in rollout storage.
            let Ok(serialized) = serde_json::to_string(call_tool_result) else {
                return Ok(call_tool_result.clone());
            };
            if serialized.len() <= max_bytes {
                return Ok(call_tool_result.clone());
            }

            // A huge MCP result can put bytes in `content`, `structuredContent`,
            // or `_meta`. Collapse the event copy to a text preview of the whole
            // serialized result so the UI still has useful context without
            // preserving a multi-megabyte structured payload.
            //
            // This budget applies to the preview text, not the final event JSON.
            // The preview is itself serialized into a JSON string, so quotes and
            // backslashes can be escaped again and the stored event may end up
            // somewhat larger than this byte budget.
            let truncated = truncate_text(&serialized, TruncationPolicy::Bytes(max_bytes));
            Ok(CallToolResult {
                content: vec![serde_json::json!({
                    "type": "text",
                    "text": truncated,
                })],
                structured_content: None,
                is_error: call_tool_result.is_error,
                meta: None,
            })
        }
        Err(message) => Err(truncate_text(message, TruncationPolicy::Bytes(max_bytes))),
    }
}

fn mask_input_property_schema(schema: &mut JsonValue) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };

    let mut description = object
        .get("description")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let guidance = "This parameter expects an absolute local file path. If you want to upload a file, provide the absolute path to that file here.";
    if description.is_empty() {
        description = guidance.to_string();
    } else if !description.contains(guidance) {
        description = format!("{description} {guidance}");
    }

    let is_array = object.get("type").and_then(JsonValue::as_str) == Some("array")
        || object.get("items").is_some();
    object.clear();
    object.insert("description".to_string(), JsonValue::String(description));
    if is_array {
        object.insert("type".to_string(), JsonValue::String("array".to_string()));
        object.insert("items".to_string(), serde_json::json!({ "type": "string" }));
    } else {
        object.insert("type".to_string(), JsonValue::String("string".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tool(tool_name: &str) -> McpTool {
        McpTool {
            name: tool_name.to_string(),
            title: None,
            description: Some(format!("Test tool: {tool_name}")),
            input_schema: JsonValue::Object(Map::new()),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        }
    }

    #[test]
    fn declared_openai_file_fields_treat_names_literally() {
        let meta = serde_json::json!({
            "openai/fileParams": ["file", "input_file", "attachments"]
        });
        let meta = meta.as_object().expect("meta object");

        assert_eq!(
            declared_openai_file_input_param_names(Some(meta)),
            vec![
                "file".to_string(),
                "input_file".to_string(),
                "attachments".to_string(),
            ]
        );
    }

    #[test]
    fn tool_with_model_visible_input_schema_masks_file_params() {
        let mut tool = create_test_tool("upload");
        tool.input_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "object",
                    "description": "Original file payload."
                },
                "files": {
                    "type": "array",
                    "items": {"type": "object"}
                }
            }
        });
        tool.meta = Some(
            serde_json::json!({
                "openai/fileParams": ["file", "files"]
            })
            .as_object()
            .expect("object")
            .clone(),
        );

        let tool = tool_with_model_visible_input_schema(&tool);

        assert_eq!(
            tool.input_schema,
            serde_json::json!({
                "type": "object",
                "properties": {
                    "file": {
                        "type": "string",
                        "description": "Original file payload. This parameter expects an absolute local file path. If you want to upload a file, provide the absolute path to that file here."
                    },
                    "files": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "This parameter expects an absolute local file path. If you want to upload a file, provide the absolute path to that file here."
                    }
                }
            })
        );
    }

    #[test]
    fn tool_with_model_visible_input_schema_leaves_tools_without_file_params_unchanged() {
        let original_tool = create_test_tool("upload");

        let tool = tool_with_model_visible_input_schema(&original_tool);

        assert_eq!(tool, original_tool);
    }

    #[test]
    fn sanitize_mcp_tool_result_for_model_rewrites_image_content() {
        let result = Ok(CallToolResult {
            content: vec![
                serde_json::json!({
                    "type": "image",
                    "data": "Zm9v",
                    "mimeType": "image/png",
                }),
                serde_json::json!({
                    "type": "text",
                    "text": "hello",
                }),
            ],
            structured_content: None,
            is_error: Some(false),
            meta: None,
        });

        let got = sanitize_mcp_tool_result_for_model(/*supports_image_input*/ false, result)
            .expect("sanitized result");

        assert_eq!(
            got.content,
            vec![
                serde_json::json!({
                    "type": "text",
                    "text": "<image content omitted because you do not support image input>",
                }),
                serde_json::json!({
                    "type": "text",
                    "text": "hello",
                }),
            ]
        );
    }

    #[test]
    fn sanitize_mcp_tool_result_for_model_preserves_image_when_supported() {
        let original = CallToolResult {
            content: vec![serde_json::json!({
                "type": "image",
                "data": "Zm9v",
                "mimeType": "image/png",
            })],
            structured_content: Some(serde_json::json!({"x": 1})),
            is_error: Some(false),
            meta: Some(serde_json::json!({"k": "v"})),
        };

        let got = sanitize_mcp_tool_result_for_model(
            /*supports_image_input*/ true,
            Ok(original.clone()),
        )
        .expect("unsanitized result");

        assert_eq!(got, original);
    }

    #[test]
    fn truncate_mcp_tool_result_for_event_preserves_small_result() {
        let original = CallToolResult {
            content: vec![serde_json::json!({
                "type": "text",
                "text": "hello",
            })],
            structured_content: Some(serde_json::json!({"x": 1})),
            is_error: Some(false),
            meta: Some(serde_json::json!({"k": "v"})),
        };

        let got = truncate_mcp_tool_result_for_event(&Ok(original.clone()), 1024)
            .expect("small result should remain successful");

        assert_eq!(got, original);
    }

    #[test]
    fn truncate_mcp_tool_result_for_event_bounds_large_result() {
        const MAX_BYTES: usize = 1024;
        let original = CallToolResult {
            content: vec![serde_json::json!({
                "type": "text",
                "text": "long-message-with-newlines-\n".repeat(200_000),
            })],
            structured_content: Some(serde_json::json!({
                "structured": "structured-value-".repeat(200_000),
            })),
            is_error: Some(false),
            meta: Some(serde_json::json!({
                "meta": "meta-value-".repeat(200_000),
            })),
        };

        let got = truncate_mcp_tool_result_for_event(&Ok(original), MAX_BYTES)
            .expect("large result should remain successful");
        let serialized = serde_json::to_string(&got).expect("truncated result should serialize");

        // The truncated preview is embedded as a JSON string, so quotes and
        // backslashes can be escaped again. That can roughly double the preview
        // bytes in the worst case. The extra buffer covers the small result
        // wrapper and marker.
        assert!(serialized.len() < MAX_BYTES * 2 + 1024);
        assert_eq!(got.structured_content, None);
        assert_eq!(got.meta, None);
        assert_eq!(got.is_error, Some(false));
        assert!(
            got.content[0]
                .get("text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("truncated")),
            "large event result should contain a truncation marker: {got:?}"
        );
    }

    #[test]
    fn truncate_mcp_tool_result_for_event_bounds_large_error() {
        const MAX_BYTES: usize = 1024;
        let got =
            truncate_mcp_tool_result_for_event(&Err("error-message-".repeat(200_000)), MAX_BYTES)
                .expect_err("large error should remain an error");

        // `truncate_text` includes its own marker, so allow a small amount of
        // overhead beyond the requested byte budget.
        assert!(got.len() < MAX_BYTES + 1024);
        assert!(got.contains("truncated"));
    }
}
