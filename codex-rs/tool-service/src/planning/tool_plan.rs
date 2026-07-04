use crate::ResponsesApiNamespaceTool;
use crate::ToolSpec;
use crate::WebSearchToolOptions;
use crate::collect_code_mode_exec_prompt_tool_definitions;
use crate::create_image_generation_tool;
use crate::create_web_search_tool;
use crate::default_namespace_description;
use std::collections::BTreeMap;
use tool_config::ToolsConfig;

/// Pure code-mode planning result derived from model-visible nested tool specs.
pub struct CodeModeExecPlan {
    pub enabled_tools: Vec<codex_code_mode_api::ToolDefinition>,
    pub namespace_descriptions: BTreeMap<String, codex_code_mode_api::ToolNamespaceDescription>,
}

pub fn hosted_model_tool_specs(config: &ToolsConfig) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    if let Some(web_search_tool) = create_web_search_tool(WebSearchToolOptions {
        web_search_mode: config.web_search_mode,
        web_search_config: config.web_search_config.as_ref(),
        web_search_tool_type: config.web_search_tool_type,
    }) {
        specs.push(web_search_tool);
    }
    if config.image_gen_tool {
        specs.push(create_image_generation_tool("png"));
    }
    specs
}

pub fn merge_tool_specs_into_namespaces(specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut merged_specs = Vec::with_capacity(specs.len());
    let mut namespace_indices = BTreeMap::<String, usize>::new();
    for spec in specs {
        match spec {
            ToolSpec::Namespace(mut namespace) => {
                if let Some(index) = namespace_indices.get(&namespace.name).copied() {
                    let ToolSpec::Namespace(existing_namespace) = &mut merged_specs[index] else {
                        unreachable!("namespace index must point to a namespace spec");
                    };
                    if existing_namespace.description.trim().is_empty()
                        && !namespace.description.trim().is_empty()
                    {
                        existing_namespace.description = namespace.description;
                    }
                    existing_namespace.tools.append(&mut namespace.tools);
                    continue;
                }

                namespace_indices.insert(namespace.name.clone(), merged_specs.len());
                merged_specs.push(ToolSpec::Namespace(namespace));
            }
            spec => merged_specs.push(spec),
        }
    }

    for spec in &mut merged_specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        namespace.tools.sort_by(|left, right| match (left, right) {
            (
                ResponsesApiNamespaceTool::Function(left),
                ResponsesApiNamespaceTool::Function(right),
            ) => left.name.cmp(&right.name),
        });

        if namespace.description.trim().is_empty() {
            namespace.description = default_namespace_description(&namespace.name);
        }
    }

    merged_specs
}

pub fn code_mode_exec_plan_for_specs(specs: &[ToolSpec]) -> CodeModeExecPlan {
    let namespace_descriptions = code_mode_namespace_descriptions(specs);
    let mut enabled_tools = collect_code_mode_exec_prompt_tool_definitions(specs.iter());
    enabled_tools
        .sort_by(|left, right| compare_code_mode_tools(left, right, &namespace_descriptions));

    CodeModeExecPlan {
        enabled_tools,
        namespace_descriptions,
    }
}

fn code_mode_namespace_descriptions(
    specs: &[ToolSpec],
) -> BTreeMap<String, codex_code_mode_api::ToolNamespaceDescription> {
    let mut namespace_descriptions = BTreeMap::new();
    for spec in specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        let entry = namespace_descriptions
            .entry(namespace.name.clone())
            .or_insert_with(|| codex_code_mode_api::ToolNamespaceDescription {
                name: namespace.name.clone(),
                description: namespace.description.clone(),
            });
        if entry.description.trim().is_empty() && !namespace.description.trim().is_empty() {
            entry.description = namespace.description.clone();
        }
    }
    namespace_descriptions
}

fn compare_code_mode_tools(
    left: &codex_code_mode_api::ToolDefinition,
    right: &codex_code_mode_api::ToolDefinition,
    namespace_descriptions: &BTreeMap<String, codex_code_mode_api::ToolNamespaceDescription>,
) -> std::cmp::Ordering {
    let left_namespace = code_mode_namespace_name(left, namespace_descriptions);
    let right_namespace = code_mode_namespace_name(right, namespace_descriptions);

    left_namespace
        .cmp(&right_namespace)
        .then_with(|| left.tool_name.name.cmp(&right.tool_name.name))
        .then_with(|| left.name.cmp(&right.name))
}

fn code_mode_namespace_name<'a>(
    tool: &codex_code_mode_api::ToolDefinition,
    namespace_descriptions: &'a BTreeMap<String, codex_code_mode_api::ToolNamespaceDescription>,
) -> Option<&'a str> {
    tool.tool_name
        .namespace
        .as_ref()
        .and_then(|namespace| namespace_descriptions.get(namespace))
        .map(|namespace_description| namespace_description.name.as_str())
}

#[cfg(test)]
#[path = "tool_plan_tests.rs"]
mod tests;
