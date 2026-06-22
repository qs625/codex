use std::collections::HashSet;

use crate::config::Config;
use codex_config_types::ToolSuggestDiscoverableType;
use codex_core_plugins_api::PluginRuntime;
use codex_features::Feature;
use codex_tool_planning::DiscoverablePluginInfo;

pub(crate) async fn list_tool_suggest_discoverable_plugins(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
) -> anyhow::Result<Vec<DiscoverablePluginInfo>> {
    if !config.features.enabled(Feature::Plugins) {
        return Ok(Vec::new());
    }

    let plugins_input = config.plugins_config_input();
    let configured_plugin_ids = config
        .tool_suggest
        .discoverables
        .iter()
        .filter(|discoverable| discoverable.kind == ToolSuggestDiscoverableType::Plugin)
        .map(|discoverable| discoverable.id.clone())
        .collect::<HashSet<_>>();
    let disabled_plugin_ids = config
        .tool_suggest
        .disabled_tools
        .iter()
        .filter(|disabled_tool| disabled_tool.kind == ToolSuggestDiscoverableType::Plugin)
        .map(|disabled_tool| disabled_tool.id.clone())
        .collect::<HashSet<_>>();
    plugin_runtime
        .list_tool_suggest_discoverable_plugins(
            &plugins_input,
            &configured_plugin_ids,
            &disabled_plugin_ids,
        )
        .await
        .map(|plugins| {
            plugins
                .into_iter()
                .map(|plugin| DiscoverablePluginInfo {
                    id: plugin.id,
                    name: plugin.name,
                    description: plugin.description,
                    has_skills: plugin.has_skills,
                    mcp_server_names: plugin.mcp_server_names,
                    app_connector_ids: plugin.app_connector_ids,
                })
                .collect()
        })
        .map_err(anyhow::Error::msg)
}

#[cfg(test)]
#[path = "discoverable_tests.rs"]
mod tests;
