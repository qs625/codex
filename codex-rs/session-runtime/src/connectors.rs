use std::collections::HashSet;

use codex_client_identity::originator;
#[cfg(test)]
pub(crate) use codex_config_types::AppToolApproval;
#[cfg(test)]
pub(crate) use codex_config_types::AppsConfigToml;
use codex_config_types::ToolSuggestDiscoverableType;
use codex_connectors_api::ConnectorDirectoryCacheContext;
use codex_connectors_api::ConnectorDirectoryCacheKey;
pub use codex_connectors_types::AppBranding;
pub use codex_connectors_types::AppInfo;
pub use codex_connectors_types::AppMetadata;
use codex_core_plugins_api::PluginRuntime;
use codex_features::Feature;
pub use codex_mcp_runtime::AccessibleConnectorsStatus;
#[cfg(test)]
pub(crate) use codex_mcp_runtime::AppToolPolicy;
#[cfg(test)]
pub(crate) use codex_mcp_runtime::app_is_enabled;
pub(crate) use codex_mcp_runtime::app_tool_policy;
#[cfg(test)]
pub(crate) use codex_mcp_runtime::app_tool_policy_from_apps_config;
#[cfg(test)]
pub(crate) use codex_mcp_runtime::apply_requirements_apps_constraints;
pub use codex_mcp_runtime::list_accessible_connectors_from_mcp_tools;
pub use codex_mcp_runtime::list_accessible_connectors_from_mcp_tools_with_environment_provider;
pub use codex_mcp_runtime::list_accessible_connectors_from_mcp_tools_with_options;
pub use codex_mcp_runtime::list_accessible_connectors_from_mcp_tools_with_options_and_status;
pub use codex_mcp_runtime::list_cached_accessible_connectors_from_mcp_tools;
#[cfg(test)]
pub(crate) use codex_mcp_runtime::managed_app_tool_approval;
pub(crate) use codex_mcp_runtime::refresh_accessible_connectors_cache_from_mcp_tools;
pub use codex_mcp_runtime::with_app_enabled_state;
pub use codex_mcp_runtime::with_app_plugin_sources;
pub(crate) use codex_mcp_runtime::{
    accessible_connectors_from_mcp_tools, list_accessible_and_enabled_connectors_from_manager,
};
use codex_mcp_types::CodexAppsAuthContext;
use codex_tool_planning::DiscoverableTool;

use crate::config::Config;
use crate::plugins::list_tool_suggest_discoverable_plugins;

pub(crate) async fn list_tool_suggest_discoverable_tools_with_auth(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
    auth_context: Option<&CodexAppsAuthContext>,
    accessible_connectors: &[AppInfo],
) -> anyhow::Result<Vec<DiscoverableTool>> {
    let connector_ids = tool_suggest_connector_ids(config, plugin_runtime).await;
    let directory_connectors = codex_connectors_api::merge::merge_plugin_connectors(
        cached_directory_connectors_for_tool_suggest_with_auth(config, auth_context).await,
        connector_ids.iter().cloned(),
    );
    let discoverable_connectors =
        codex_connectors_api::filter::filter_tool_suggest_discoverable_connectors(
            directory_connectors,
            accessible_connectors,
            &connector_ids,
            originator().value.as_str(),
        )
        .into_iter()
        .map(DiscoverableTool::from);
    let discoverable_plugins = list_tool_suggest_discoverable_plugins(config, plugin_runtime)
        .await?
        .into_iter()
        .map(DiscoverableTool::from);
    Ok(discoverable_connectors
        .chain(discoverable_plugins)
        .collect())
}

async fn tool_suggest_connector_ids(
    config: &Config,
    plugin_runtime: &dyn PluginRuntime,
) -> HashSet<String> {
    let plugins_input = config.plugins_config_input();
    let mut connector_ids = plugin_runtime
        .plugins_for_config(&plugins_input)
        .await
        .capability_summaries()
        .iter()
        .flat_map(|plugin| plugin.app_connector_ids.iter())
        .map(|connector_id| connector_id.0.clone())
        .collect::<HashSet<_>>();
    connector_ids.extend(
        config
            .tool_suggest
            .discoverables
            .iter()
            .filter(|discoverable| discoverable.kind == ToolSuggestDiscoverableType::Connector)
            .map(|discoverable| discoverable.id.clone()),
    );
    let disabled_connector_ids = config
        .tool_suggest
        .disabled_tools
        .iter()
        .filter(|disabled_tool| disabled_tool.kind == ToolSuggestDiscoverableType::Connector)
        .map(|disabled_tool| disabled_tool.id.as_str())
        .collect::<HashSet<_>>();
    connector_ids.retain(|connector_id| !disabled_connector_ids.contains(connector_id.as_str()));
    connector_ids
}

async fn cached_directory_connectors_for_tool_suggest_with_auth(
    config: &Config,
    auth_context: Option<&CodexAppsAuthContext>,
) -> Vec<AppInfo> {
    if !config.features.enabled(Feature::Apps) {
        return Vec::new();
    }

    let Some(auth_context) = auth_context.filter(|auth| auth.uses_codex_backend) else {
        return Vec::new();
    };

    let account_id = match auth_context.account_id.as_deref() {
        Some(account_id) if !account_id.is_empty() => account_id.to_string(),
        _ => return Vec::new(),
    };
    let cache_context = ConnectorDirectoryCacheContext::new(
        config.codex_home.to_path_buf(),
        ConnectorDirectoryCacheKey::new(
            config.chatgpt_base_url.clone(),
            Some(account_id),
            auth_context.chatgpt_user_id.clone(),
            auth_context.is_workspace_account,
        ),
    );

    codex_connectors_api::cached_directory_connectors(&cache_context).unwrap_or_default()
}

#[cfg(test)]
#[path = "connectors_tests.rs"]
mod tests;
