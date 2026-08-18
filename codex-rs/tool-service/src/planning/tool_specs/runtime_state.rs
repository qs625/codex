use std::collections::BTreeMap;

use serde_json::Value;
use serde_json::json;

use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;

pub fn create_list_commands_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "list_commands".to_string(),
        description: "List currently running exec_command sessions for the current thread only. Returns concise command metadata and never includes recent output.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: Some(list_commands_output_schema()),
    })
}

pub fn create_list_subscriptions_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: "list_subscriptions".to_string(),
        description: "List currently active event subscriptions for the current thread only. This is a read-only snapshot and does not wait for events or change subscriptions.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        output_schema: Some(list_subscriptions_output_schema()),
    })
}

fn list_commands_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "commands": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "command_id": { "type": "integer" },
                        "call_id": { "type": "string" },
                        "label": { "type": "string" },
                        "tty": { "type": "boolean" },
                        "notify_on": { "type": "string", "enum": ["output", "exit"] },
                        "cwd": { "type": "string" },
                        "command_text": { "type": "string" }
                    },
                    "required": [
                        "command_id",
                        "call_id",
                        "label",
                        "tty",
                        "notify_on",
                        "cwd",
                        "command_text"
                    ]
                }
            }
        },
        "required": ["commands"]
    })
}

fn list_subscriptions_output_schema() -> Value {
    let subscription_schemas = vec![
        subscription_schema(
            "fs",
            json!({
                "path": { "type": "string" },
                "recursive": { "type": "boolean" }
            }),
            ["path", "recursive"],
        ),
        subscription_schema(
            "event_command",
            json!({
                "command_text": { "type": "string" },
                "cwd": { "type": ["string", "null"] }
            }),
            ["command_text", "cwd"],
        ),
        subscription_schema(
            "schedule",
            json!({
                "schedule": { "type": "object" },
                "message": { "type": ["string", "null"] }
            }),
            ["schedule", "message"],
        ),
        subscription_schema(
            "process_exit",
            json!({
                "session_id": { "type": "integer" }
            }),
            ["session_id"],
        ),
    ];
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "subscriptions": {
                "type": "array",
                "items": {
                    "oneOf": subscription_schemas
                }
            }
        },
        "required": ["subscriptions"]
    })
}

fn subscription_schema<const N: usize>(
    subscription_type: &str,
    extra_properties: Value,
    extra_required: [&str; N],
) -> Value {
    let mut properties = serde_json::Map::from_iter([
        ("type".to_string(), json!({ "const": subscription_type })),
        ("subscription_id".to_string(), json!({ "type": "string" })),
        ("label".to_string(), json!({ "type": ["string", "null"] })),
        ("status".to_string(), json!({ "const": "active" })),
    ]);
    if let Value::Object(extra_properties) = extra_properties {
        properties.extend(extra_properties);
    }
    let mut required = vec!["type", "subscription_id", "label", "status"];
    required.extend(extra_required);
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function_tool(tool: ToolSpec, name: &str) -> ResponsesApiTool {
        let ToolSpec::Function(tool) = tool else {
            panic!("{name} should be a function tool");
        };
        assert_eq!(tool.name, name);
        tool
    }

    #[test]
    fn list_commands_tool_has_empty_params_and_no_output_tail_schema() {
        let tool = function_tool(create_list_commands_tool(), "list_commands");
        assert_eq!(tool.parameters.required, Some(Vec::new()));
        let schema = tool.output_schema.expect("output schema");
        let serialized = serde_json::to_string(&schema).expect("schema serializes");
        assert!(serialized.contains("command_text"));
        assert!(!serialized.contains("latest_output_tail"));
        assert!(!serialized.contains("output_tail"));
    }

    #[test]
    fn list_subscriptions_tool_has_empty_params_and_active_status_schema() {
        let tool = function_tool(create_list_subscriptions_tool(), "list_subscriptions");
        assert_eq!(tool.parameters.required, Some(Vec::new()));
        let schema = tool.output_schema.expect("output schema");
        let serialized = serde_json::to_string(&schema).expect("schema serializes");
        assert!(serialized.contains("event_command"));
        assert!(serialized.contains("process_exit"));
        assert!(serialized.contains("active"));
    }
}
