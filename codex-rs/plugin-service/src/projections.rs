use codex_context_manager::AvailablePluginsInstructions;
use codex_context_manager::ContextualUserFragment;
use plugin_service_api::AppConnectorId;
use plugin_service_api::PluginCapabilitySummary;
use plugin_service_api::PluginRuntime;
use plugin_service_api::PluginSkillRoot;
use plugin_service_api::PluginsConfigInput;
use std::collections::HashSet;

pub async fn load_effective_plugin_skill_roots_for_config(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
) -> Vec<PluginSkillRoot> {
    plugin_runtime
        .plugins_for_config(plugins_input)
        .await
        .effective_plugin_skill_roots()
}

pub async fn render_available_plugins_instructions(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
) -> Option<String> {
    let loaded_plugins = plugin_runtime.plugins_for_config(plugins_input).await;
    AvailablePluginsInstructions::from_plugins(loaded_plugins.capability_summaries())
        .map(|instructions| instructions.render())
}

pub async fn load_plugin_capability_summaries_for_config(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
) -> Vec<PluginCapabilitySummary> {
    plugin_runtime
        .plugins_for_config(plugins_input)
        .await
        .capability_summaries()
        .to_vec()
}

pub async fn load_plugin_effective_apps_for_config(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
) -> Vec<AppConnectorId> {
    plugin_runtime
        .plugins_for_config(plugins_input)
        .await
        .effective_apps()
}

pub async fn load_plugin_connector_ids_for_config(
    plugin_runtime: &dyn PluginRuntime,
    plugins_input: &PluginsConfigInput,
) -> HashSet<String> {
    load_plugin_capability_summaries_for_config(plugin_runtime, plugins_input)
        .await
        .into_iter()
        .flat_map(|plugin| plugin.app_connector_ids.into_iter())
        .map(|connector_id| connector_id.0)
        .collect()
}
