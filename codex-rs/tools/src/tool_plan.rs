use crate::ResponsesApiNamespaceTool;
use crate::ToolSpec;
use crate::ToolsConfig;
use crate::WebSearchToolOptions;
use crate::collect_code_mode_exec_prompt_tool_definitions;
use crate::create_image_generation_tool;
use crate::create_web_search_tool;
use crate::default_namespace_description;
use std::collections::BTreeMap;

/// Pure code-mode planning result derived from model-visible nested tool specs.
pub struct CodeModeExecPlan {
    pub enabled_tools: Vec<codex_code_mode::ToolDefinition>,
    pub namespace_descriptions: BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
}

pub fn filter_tool_specs_for_agent(config: &ToolsConfig, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let Some(patterns) = config.agent_tool_patterns.as_deref() else {
        return specs;
    };
    specs
        .into_iter()
        .filter_map(|spec| filter_tool_spec_for_agent(spec, patterns))
        .collect()
}

fn filter_tool_spec_for_agent(mut spec: ToolSpec, patterns: &[String]) -> Option<ToolSpec> {
    match &mut spec {
        ToolSpec::Namespace(namespace) => {
            if tool_name_matches_patterns(&namespace.name, patterns) {
                return Some(spec);
            }
            let namespace_name = namespace.name.clone();
            namespace.tools.retain(|tool| match tool {
                ResponsesApiNamespaceTool::Function(tool) => {
                    let qualified = format!("{namespace_name}{}", tool.name);
                    tool_name_matches_patterns(&tool.name, patterns)
                        || tool_name_matches_patterns(&qualified, patterns)
                }
            });
            (!namespace.tools.is_empty()).then_some(spec)
        }
        _ => tool_name_matches_patterns(spec.name(), patterns).then_some(spec),
    }
}

pub fn tool_name_matches_patterns(tool_name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim();
        if pattern == "*" {
            true
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            tool_name.starts_with(prefix)
        } else {
            tool_name == pattern
        }
    })
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
    enabled_tools.sort_by(|left, right| {
        compare_code_mode_tools(left, right, &namespace_descriptions)
    });

    CodeModeExecPlan {
        enabled_tools,
        namespace_descriptions,
    }
}

fn code_mode_namespace_descriptions(
    specs: &[ToolSpec],
) -> BTreeMap<String, codex_code_mode::ToolNamespaceDescription> {
    let mut namespace_descriptions = BTreeMap::new();
    for spec in specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        let entry = namespace_descriptions
            .entry(namespace.name.clone())
            .or_insert_with(|| codex_code_mode::ToolNamespaceDescription {
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
    left: &codex_code_mode::ToolDefinition,
    right: &codex_code_mode::ToolDefinition,
    namespace_descriptions: &BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
) -> std::cmp::Ordering {
    let left_namespace = code_mode_namespace_name(left, namespace_descriptions);
    let right_namespace = code_mode_namespace_name(right, namespace_descriptions);

    left_namespace
        .cmp(&right_namespace)
        .then_with(|| left.tool_name.name.cmp(&right.tool_name.name))
        .then_with(|| left.name.cmp(&right.name))
}

fn code_mode_namespace_name<'a>(
    tool: &codex_code_mode::ToolDefinition,
    namespace_descriptions: &'a BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
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
