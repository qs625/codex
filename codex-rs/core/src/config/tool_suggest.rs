use super::*;

pub(super) fn resolve_tool_suggest_config(
    config_toml: &ConfigToml,
    config_layer_stack: &ConfigLayerStack,
) -> ToolSuggestConfig {
    resolve_tool_suggest_config_from_config(config_toml.tool_suggest.as_ref(), config_layer_stack)
}

pub(crate) fn resolve_tool_suggest_config_from_layer_stack(
    config_layer_stack: &ConfigLayerStack,
) -> ToolSuggestConfig {
    let tool_suggest = config_layer_stack
        .effective_config()
        .get("tool_suggest")
        .cloned()
        .and_then(|value| value.try_into::<ToolSuggestConfig>().ok());
    resolve_tool_suggest_config_from_config(tool_suggest.as_ref(), config_layer_stack)
}

fn resolve_tool_suggest_config_from_config(
    tool_suggest: Option<&ToolSuggestConfig>,
    config_layer_stack: &ConfigLayerStack,
) -> ToolSuggestConfig {
    let discoverables = tool_suggest
        .into_iter()
        .flat_map(|tool_suggest| tool_suggest.discoverables.iter())
        .filter_map(|discoverable| {
            let trimmed = discoverable.id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(ToolSuggestDiscoverable {
                    kind: discoverable.kind,
                    id: trimmed.to_string(),
                })
            }
        })
        .collect();
    let mut seen_disabled_tools = HashSet::new();
    let mut disabled_tools = Vec::new();
    let mut add_disabled_tool = |disabled_tool: ToolSuggestDisabledTool| {
        if let Some(disabled_tool) = disabled_tool.normalized()
            && seen_disabled_tools.insert(disabled_tool.clone())
        {
            disabled_tools.push(disabled_tool);
        }
    };

    let layers = config_layer_stack.get_layers(
        ConfigLayerStackOrdering::LowestPrecedenceFirst,
        /*include_disabled*/ false,
    );
    if layers.is_empty() {
        for disabled_tool in tool_suggest
            .into_iter()
            .flat_map(|tool_suggest| tool_suggest.disabled_tools.iter().cloned())
        {
            add_disabled_tool(disabled_tool);
        }
    } else {
        for layer in layers {
            let Some(tool_suggest) = layer
                .config
                .get("tool_suggest")
                .cloned()
                .and_then(|value| value.try_into::<ToolSuggestConfig>().ok())
            else {
                continue;
            };
            for disabled_tool in tool_suggest.disabled_tools {
                add_disabled_tool(disabled_tool);
            }
        }
    }

    ToolSuggestConfig {
        discoverables,
        disabled_tools,
    }
}

pub(super) fn thread_store_config(thread_store: Option<ThreadStoreToml>) -> ThreadStoreConfig {
    match thread_store {
        Some(ThreadStoreToml::Local {}) => ThreadStoreConfig::Local,
        Some(ThreadStoreToml::InMemory { id }) => ThreadStoreConfig::InMemory { id },
        None => ThreadStoreConfig::Local,
    }
}

pub(super) fn is_session_layer(source: &ConfigLayerSource) -> bool {
    matches!(source, ConfigLayerSource::SessionFlags)
}
