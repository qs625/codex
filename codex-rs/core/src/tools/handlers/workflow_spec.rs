use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use std::collections::BTreeMap;

pub(crate) const WORKFLOW_LIST_TOOL_NAME: &str = "workflow_list";
pub(crate) const WORKFLOW_DESCRIBE_TOOL_NAME: &str = "workflow_describe";
pub(crate) const WORKFLOW_START_TOOL_NAME: &str = "workflow_start";
pub(crate) const WORKFLOW_STATUS_TOOL_NAME: &str = "workflow_status";
pub(crate) const WORKFLOW_RESUME_TOOL_NAME: &str = "workflow_resume";
pub(crate) const WORKFLOW_ABORT_TOOL_NAME: &str = "workflow_abort";

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
        description: "Read WORKFLOW.md frontmatter metadata and instructions for one available Codex Dynamic Workflow. Use this before deciding how to run or emulate a workflow.".to_string(),
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

pub(crate) fn create_workflow_start_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "workflow".to_string(),
            JsonSchema::string(Some(
                "Workflow id returned by workflow_list, for example `feature-dev`.".to_string(),
            )),
        ),
        (
            "inputs".to_string(),
            JsonSchema::object(BTreeMap::new(), /*required*/ None, Some(true.into())),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_START_TOOL_NAME.to_string(),
        description: "Start a Codex Dynamic Workflow run and return its run id and current status."
            .to_string(),
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

pub(crate) fn create_workflow_status_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "run_id".to_string(),
        JsonSchema::string(Some(
            "Workflow run id returned by workflow_start.".to_string(),
        )),
    )]);
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_STATUS_TOOL_NAME.to_string(),
        description: "Read the current status for a Codex Dynamic Workflow run.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["run_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub(crate) fn create_workflow_resume_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "run_id".to_string(),
            JsonSchema::string(Some(
                "Workflow run id returned by workflow_start.".to_string(),
            )),
        ),
        (
            "inputs".to_string(),
            JsonSchema::object(BTreeMap::new(), /*required*/ None, Some(true.into())),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_RESUME_TOOL_NAME.to_string(),
        description:
            "Resume or continue an existing Codex Dynamic Workflow run and return its status."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["run_id".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

pub(crate) fn create_workflow_abort_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "run_id".to_string(),
            JsonSchema::string(Some(
                "Workflow run id returned by workflow_start.".to_string(),
            )),
        ),
        (
            "reason".to_string(),
            JsonSchema::string(Some("Optional human-readable abort reason.".to_string())),
        ),
    ]);
    ToolSpec::Function(ResponsesApiTool {
        name: WORKFLOW_ABORT_TOOL_NAME.to_string(),
        description: "Abort an existing Codex Dynamic Workflow run.".to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["run_id".to_string()]),
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

    #[test]
    fn workflow_control_tools_require_primary_ids() {
        let ToolSpec::Function(start) = create_workflow_start_tool() else {
            panic!("expected function tool");
        };
        let ToolSpec::Function(status) = create_workflow_status_tool() else {
            panic!("expected function tool");
        };
        let ToolSpec::Function(resume) = create_workflow_resume_tool() else {
            panic!("expected function tool");
        };
        let ToolSpec::Function(abort) = create_workflow_abort_tool() else {
            panic!("expected function tool");
        };

        assert_eq!(
            start.parameters.required,
            Some(vec!["workflow".to_string()])
        );
        assert_eq!(status.parameters.required, Some(vec!["run_id".to_string()]));
        assert_eq!(resume.parameters.required, Some(vec!["run_id".to_string()]));
        assert_eq!(abort.parameters.required, Some(vec!["run_id".to_string()]));
    }
}
