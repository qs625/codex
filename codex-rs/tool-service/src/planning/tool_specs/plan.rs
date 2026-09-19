use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub fn create_update_plan_tool() -> ToolSpec {
    let plan_item_properties = BTreeMap::from([
        ("step".to_string(), JsonSchema::string(/*description*/ None)),
        (
            "status".to_string(),
            JsonSchema::string_enum(
                vec![
                    json!("pending"),
                    json!("in_progress"),
                    json!("blocked"),
                    json!("completed"),
                ],
                Some("One of: pending, in_progress, blocked, completed".to_string()),
            ),
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "explanation".to_string(),
            JsonSchema::string(/*description*/ None),
        ),
        (
            "plan".to_string(),
            JsonSchema::array(
                JsonSchema::object(
                    plan_item_properties,
                    Some(vec!["step".to_string(), "status".to_string()]),
                    Some(false.into()),
                ),
                Some("The list of steps".to_string()),
            ),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "update_plan".to_string(),
        description: r#"Updates the task plan.
Provide an optional explanation and a list of plan items, each with a step and status.
Use statuses pending, in_progress, blocked, and completed.
Use blocked when work is stuck on user input, external conditions, or failed recovery.
For linear single-agent tasks, prefer one in_progress step at a time. For coordination or parallel work, multiple steps may be in_progress.
The plan is the current work board, not a history archive: completed items may remain briefly as delivery proof, but remove them on the next update when no follow-up work remains.
"#
        .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["plan".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_plan_tool_schema_lists_blocked_and_allows_parallel_in_progress() {
        let ToolSpec::Function(tool) = create_update_plan_tool() else {
            panic!("update_plan should be a function tool");
        };

        assert!(
            tool.description
                .contains("multiple steps may be in_progress")
        );
        assert!(
            tool.description
                .contains("current work board, not a history archive")
        );
        assert!(!tool.description.contains("At most one step"));

        let status_schema = tool
            .parameters
            .properties
            .as_ref()
            .and_then(|properties| properties.get("plan"))
            .and_then(|plan| plan.items.as_ref())
            .and_then(|items| items.properties.as_ref())
            .and_then(|properties| properties.get("status"))
            .expect("status schema");

        assert_eq!(
            status_schema.enum_values.as_deref(),
            Some(
                [
                    json!("pending"),
                    json!("in_progress"),
                    json!("blocked"),
                    json!("completed"),
                ]
                .as_slice()
            )
        );
    }
}
