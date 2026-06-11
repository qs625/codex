use crate::workflows::WorkflowRegistry;
use crate::workflows::render_available_workflows_body;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AvailableWorkflowsInstructions {
    body: String,
}

impl AvailableWorkflowsInstructions {
    pub(crate) fn from_registry(registry: &WorkflowRegistry) -> Option<Self> {
        render_available_workflows_body(registry).map(|body| Self { body })
    }
}

impl ContextualUserFragment for AvailableWorkflowsInstructions {
    const ROLE: &'static str = "developer";
    const START_MARKER: &'static str = "<workflows_instructions>";
    const END_MARKER: &'static str = "</workflows_instructions>";

    fn body(&self) -> String {
        self.body.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::WorkflowInputSpec;
    use crate::workflows::WorkflowSource;
    use crate::workflows::WorkflowSummary;
    use std::collections::BTreeMap;

    #[test]
    fn renders_available_workflows_fragment() {
        let registry = WorkflowRegistry {
            workflows: vec![WorkflowSummary {
                id: "feature-dev".to_string(),
                name: "Feature Development".to_string(),
                description: "Research, implement, review, and verify.".to_string(),
                source: WorkflowSource::Project,
                path: "/repo/.codex/workflows/feature-dev".to_string(),
                entry: "workflow.ts".to_string(),
                version: Some("0.1.0".to_string()),
                when_to_use: vec!["feature work".to_string()],
                inputs: BTreeMap::from([(
                    "objective".to_string(),
                    WorkflowInputSpec {
                        input_type: "string".to_string(),
                        description: Some("Goal".to_string()),
                    },
                )]),
            }],
            diagnostics: Vec::new(),
        };

        let fragment = AvailableWorkflowsInstructions::from_registry(&registry).expect("fragment");
        let rendered = fragment.render();

        assert!(rendered.starts_with("<workflows_instructions>"));
        assert!(rendered.contains("- feature-dev (project)"));
        assert!(rendered.contains("Inputs: objective"));
        assert!(rendered.ends_with("</workflows_instructions>"));
    }
}
