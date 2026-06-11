use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub(crate) const WORKFLOW_LIST_TOOL_NAME: &str = "workflow_list";
pub(crate) const WORKFLOW_DESCRIBE_TOOL_NAME: &str = "workflow_describe";

pub(crate) fn create_workflow_list_tool() -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_LIST_TOOL_NAME.to_string(),
        description: "List Codex Dynamic Workflows available from CODEX_HOME and the current project's .codex/workflows directories.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(BTreeMap::new(), /*required*/ None, Some(false.into())),
        output_schema: None,
    })
}

pub(crate) fn create_workflow_describe_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "workflow".to_string(),
        JsonSchema::string(Some(
            "Workflow id returned by workflow_list, for example `feature-dev`.".to_string(),
        )),
    )]);
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_DESCRIBE_TOOL_NAME.to_string(),
        description: "Read metadata and README context for one available Codex Dynamic Workflow. Use this before deciding how to run or emulate a workflow.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["workflow".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_tools::ToolSpec;
    use pretty_assertions::assert_eq;

    #[test]
    fn workflow_describe_requires_workflow_id() {
        let ToolSpec::Function(tool) = create_workflow_describe_tool() else {
            panic!("expected function tool");
        };

        assert_eq!(tool.name, WORKFLOW_DESCRIBE_TOOL_NAME);
        assert_eq!(tool.parameters.required, Some(vec!["workflow".to_string()]));
    }
}
