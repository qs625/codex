use std::collections::HashSet;

use codex_tool_types::DiscoverablePluginInfo;
use plugin_service_api::PluginHookSource;
use plugin_service_api::PluginRuntime;
use plugin_service_api::PluginsConfigInput;

pub async fn list_tool_suggest_discoverable_plugins(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
    configured_plugin_ids: &HashSet<String>,
    disabled_plugin_ids: &HashSet<String>,
) -> anyhow::Result<Vec<DiscoverablePluginInfo>> {
    if !plugins_input.plugins_enabled {
        return Ok(Vec::new());
    }

    plugin_runtime
        .list_tool_suggest_discoverable_plugins(
            plugins_input,
            configured_plugin_ids,
            disabled_plugin_ids,
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

pub async fn load_plugin_hooks_for_config(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
    plugin_hooks_enabled: bool,
) -> (Vec<PluginHookSource>, Vec<String>) {
    if !plugin_hooks_enabled {
        return (Vec::new(), Vec::new());
    }

    let plugin_outcome = plugin_runtime.plugins_for_config(plugins_input).await;
    (
        plugin_outcome.effective_plugin_hook_sources(),
        plugin_outcome.effective_plugin_hook_warnings(),
    )
}

#[cfg(test)]
#[path = "discoverable_tests.rs"]
mod tests;
