use std::sync::Arc;

use codex_protocol::ToolName;
use rmcp::model::Tool;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value as JsonValue;

const META_OPENAI_FILE_PARAMS: &str = "openai/fileParams";

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
    /// Raw MCP tool definition; `tool.name` is sent back to the MCP server.
    pub tool: Tool,
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

pub fn tool_with_model_visible_input_schema(tool: &Tool) -> Tool {
    let file_params = declared_openai_file_input_param_names(tool.meta.as_deref());
    if file_params.is_empty() {
        return tool.clone();
    }

    let mut tool = tool.clone();
    let mut input_schema = JsonValue::Object(tool.input_schema.as_ref().clone());
    mask_input_schema_for_file_path_params(&mut input_schema, &file_params);
    if let JsonValue::Object(input_schema) = input_schema {
        tool.input_schema = Arc::new(input_schema);
    }
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
    use std::sync::Arc;

    use rmcp::model::JsonObject;
    use rmcp::model::Meta;

    use super::*;

    fn create_test_tool(tool_name: &str) -> Tool {
        Tool {
            name: tool_name.to_string().into(),
            title: None,
            description: Some(format!("Test tool: {tool_name}").into()),
            input_schema: Arc::new(JsonObject::default()),
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
        tool.input_schema = Arc::new(
            serde_json::json!({
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
            })
            .as_object()
            .expect("object")
            .clone(),
        );
        tool.meta = Some(Meta(
            serde_json::json!({
                "openai/fileParams": ["file", "files"]
            })
            .as_object()
            .expect("object")
            .clone(),
        ));

        let tool = tool_with_model_visible_input_schema(&tool);

        assert_eq!(
            *tool.input_schema,
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
            .as_object()
            .expect("object")
            .clone()
        );
    }

    #[test]
    fn tool_with_model_visible_input_schema_leaves_tools_without_file_params_unchanged() {
        let original_tool = create_test_tool("upload");

        let tool = tool_with_model_visible_input_schema(&original_tool);

        assert_eq!(tool, original_tool);
    }
}
