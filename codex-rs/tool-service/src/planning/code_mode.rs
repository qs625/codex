use crate::ResponsesApiNamespaceTool;
use crate::ToolName;
use crate::ToolSpec;
use codex_code_mode_api::CodeModeToolKind;
use codex_code_mode_api::ToolDefinition as CodeModeToolDefinition;

pub fn collect_code_mode_tool_definitions<'a>(
    specs: impl IntoIterator<Item = &'a ToolSpec>,
) -> Vec<CodeModeToolDefinition> {
    let mut tool_definitions = specs
        .into_iter()
        .flat_map(code_mode_tool_definitions_for_spec)
        .filter(|definition| codex_code_mode_api::is_code_mode_nested_tool(&definition.name))
        .map(codex_code_mode_api::augment_tool_definition)
        .collect::<Vec<_>>();
    tool_definitions.sort_by(|left, right| left.name.cmp(&right.name));
    tool_definitions.dedup_by(|left, right| left.name == right.name);
    tool_definitions
}

pub fn collect_code_mode_exec_prompt_tool_definitions<'a>(
    specs: impl IntoIterator<Item = &'a ToolSpec>,
) -> Vec<CodeModeToolDefinition> {
    let mut tool_definitions = specs
        .into_iter()
        .flat_map(code_mode_tool_definitions_for_spec)
        .filter(|definition| codex_code_mode_api::is_code_mode_nested_tool(&definition.name))
        .collect::<Vec<_>>();
    tool_definitions.sort_by(|left, right| left.name.cmp(&right.name));
    tool_definitions.dedup_by(|left, right| left.name == right.name);
    tool_definitions
}

fn code_mode_tool_definitions_for_spec(spec: &ToolSpec) -> Vec<CodeModeToolDefinition> {
    match spec {
        ToolSpec::Function(tool) => {
            let name = tool.name.clone();
            vec![CodeModeToolDefinition {
                tool_name: ToolName::plain(name.clone()),
                name,
                description: tool.description.clone(),
                kind: CodeModeToolKind::Function,
                input_schema: serde_json::to_value(&tool.parameters).ok(),
                output_schema: tool.output_schema.clone(),
            }]
        }
        ToolSpec::Freeform(tool) => {
            let name = tool.name.clone();
            vec![CodeModeToolDefinition {
                tool_name: ToolName::plain(name.clone()),
                name,
                description: tool.description.clone(),
                kind: CodeModeToolKind::Freeform,
                input_schema: None,
                output_schema: None,
            }]
        }
        ToolSpec::Namespace(namespace) => namespace
            .tools
            .iter()
            .map(|tool| match tool {
                ResponsesApiNamespaceTool::Function(tool) => {
                    let tool_name = ToolName::namespaced(namespace.name.clone(), tool.name.clone());
                    CodeModeToolDefinition {
                        name: code_mode_name_for_tool_name(&tool_name),
                        tool_name,
                        description: tool.description.clone(),
                        kind: CodeModeToolKind::Function,
                        input_schema: serde_json::to_value(&tool.parameters).ok(),
                        output_schema: tool.output_schema.clone(),
                    }
                }
            })
            .collect(),
        ToolSpec::ImageGeneration { .. }
        | ToolSpec::ToolSearch { .. }
        | ToolSpec::WebSearch { .. } => Vec::new(),
    }
}

pub fn code_mode_name_for_tool_name(tool_name: &ToolName) -> String {
    match tool_name.namespace.as_deref() {
        Some(namespace) if namespace.ends_with('_') || tool_name.name.starts_with('_') => {
            format!("{namespace}{}", tool_name.name)
        }
        Some(namespace) => format!("{namespace}_{}", tool_name.name),
        None => tool_name.name.clone(),
    }
}
