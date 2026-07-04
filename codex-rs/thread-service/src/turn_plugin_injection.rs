use std::collections::BTreeSet;

use codex_connectors_api::AppInfo;
use codex_connectors_api::metadata::connector_display_label;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::PluginInstructions;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::ToolInfo;
use plugin_service_api::PluginCapabilitySummary;
use protocol::models::ResponseItem;

pub(crate) fn build_plugin_injections(
    mentioned_plugins: &[PluginCapabilitySummary],
    mcp_tools: &[ToolInfo],
    available_connectors: &[AppInfo],
) -> Vec<ResponseItem> {
    if mentioned_plugins.is_empty() {
        return Vec::new();
    }

    mentioned_plugins
        .iter()
        .filter_map(|plugin| {
            let available_mcp_servers = mcp_tools
                .iter()
                .filter(|tool| {
                    tool.server_name != CODEX_APPS_MCP_SERVER_NAME
                        && tool
                            .plugin_display_names
                            .iter()
                            .any(|plugin_name| plugin_name == &plugin.display_name)
                })
                .map(|tool| tool.server_name.clone())
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect::<Vec<_>>();
            let available_apps = available_connectors
                .iter()
                .filter(|connector| {
                    connector.is_enabled
                        && connector
                            .plugin_display_names
                            .iter()
                            .any(|plugin_name| plugin_name == &plugin.display_name)
                })
                .map(connector_display_label)
                .collect::<BTreeSet<String>>()
                .into_iter()
                .collect::<Vec<_>>();
            render_explicit_plugin_instructions(plugin, &available_mcp_servers, &available_apps)
                .map(PluginInstructions::new)
                .map(ContextualUserFragment::into)
        })
        .collect()
}

fn render_explicit_plugin_instructions(
    plugin: &PluginCapabilitySummary,
    available_mcp_servers: &[String],
    available_apps: &[String],
) -> Option<String> {
    let mut lines = vec![format!(
        "Capabilities from the `{}` plugin:",
        plugin.display_name
    )];

    if plugin.has_skills {
        lines.push(format!(
            "- Skills from this plugin are prefixed with `{}:`.",
            plugin.display_name
        ));
    }

    if !available_mcp_servers.is_empty() {
        lines.push(format!(
            "- MCP servers from this plugin available in this session: {}.",
            available_mcp_servers
                .iter()
                .map(|server| format!("`{server}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if !available_apps.is_empty() {
        lines.push(format!(
            "- Apps from this plugin available in this session: {}.",
            available_apps
                .iter()
                .map(|app| format!("`{app}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if lines.len() == 1 {
        return None;
    }

    lines.push("Use these plugin-associated capabilities to help solve the task.".to_string());
    Some(lines.join("\n"))
}
