use crate::ResponsesApiNamespaceTool;
use crate::ToolExposure;
use crate::ToolRegistryEntry;
use crate::ToolSearchInfo;
use crate::ToolSpec;
use crate::ToolsConfig;
use crate::WebSearchToolOptions;
use crate::augment_tool_spec_for_code_mode;
use crate::collect_code_mode_exec_prompt_tool_definitions;
use crate::create_image_generation_tool;
use crate::create_web_search_tool;
use crate::default_namespace_description;
use std::collections::BTreeMap;

/// Pure code-mode planning result derived from model-visible nested tool specs.
pub struct CodeModeExecPlan {
    pub enabled_tools: Vec<codex_code_mode_api::ToolDefinition>,
    pub namespace_descriptions: BTreeMap<String, codex_code_mode_api::ToolNamespaceDescription>,
}

/// Host-neutral registry planning output for a concrete host's tool entries.
///
/// This contains only model-visible spec decisions and deferred discovery
/// metadata. The host remains responsible for constructing runtime handlers,
/// code-mode handler adapters, and the executable registry.
pub struct PlannedToolRegistry<E> {
    pub entries: Vec<E>,
    pub model_visible_specs: Vec<ToolSpec>,
    pub code_mode_nested_tool_specs: Vec<ToolSpec>,
    pub deferred_search_infos: Vec<ToolSearchInfo>,
    pub deferred_tools_available: bool,
}

pub fn plan_tool_registry_entries<E>(
    config: &ToolsConfig,
    entries: Vec<E>,
    hosted_specs: Vec<ToolSpec>,
) -> PlannedToolRegistry<E>
where
    E: ToolRegistryEntry,
{
    let entries = filter_tool_registry_entries_for_agent(config, entries);
    let hosted_specs = filter_tool_specs_for_agent(config, hosted_specs);
    let deferred_tools_available = entries
        .iter()
        .any(|entry| entry.exposure() == ToolExposure::Deferred);
    let code_mode_nested_tool_specs = if config.code_mode_enabled {
        entries
            .iter()
            .filter_map(|entry| {
                if entry.exposure() == ToolExposure::DirectModelOnly {
                    return None;
                }

                entry.spec()
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut non_deferred_specs = Vec::new();
    let mut deferred_search_infos = Vec::new();
    for entry in &entries {
        match entry.exposure() {
            ToolExposure::Direct | ToolExposure::DirectModelOnly => {
                if let Some(spec) = entry.spec() {
                    non_deferred_specs.push((spec, entry.exposure()));
                }
            }
            ToolExposure::Deferred => {
                if let Some(search_info) = entry.search_info() {
                    deferred_search_infos.push(search_info);
                }
            }
        }
    }

    non_deferred_specs.extend(
        hosted_specs
            .into_iter()
            .map(|spec| (spec, ToolExposure::Direct)),
    );

    let non_deferred_specs = non_deferred_specs
        .into_iter()
        .map(|(spec, exposure)| {
            if config.code_mode_enabled && exposure != ToolExposure::DirectModelOnly {
                augment_tool_spec_for_code_mode(spec)
            } else {
                spec
            }
        })
        .collect();

    let model_visible_specs = merge_tool_specs_into_namespaces(non_deferred_specs)
        .into_iter()
        .filter(|spec| config.namespace_tools || !matches!(spec, ToolSpec::Namespace(_)))
        .collect();

    PlannedToolRegistry {
        entries,
        model_visible_specs,
        code_mode_nested_tool_specs,
        deferred_search_infos,
        deferred_tools_available,
    }
}

fn filter_tool_registry_entries_for_agent<E>(config: &ToolsConfig, entries: Vec<E>) -> Vec<E>
where
    E: ToolRegistryEntry,
{
    let Some(patterns) = config.agent_tool_patterns.as_deref() else {
        return entries;
    };
    entries
        .into_iter()
        .filter(|entry| tool_name_matches_patterns(&entry.tool_name().to_string(), patterns))
        .collect()
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
