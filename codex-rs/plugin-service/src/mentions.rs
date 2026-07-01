use std::collections::BTreeSet;
use std::collections::HashSet;

use codex_config::AgentRoleConfig;
use codex_config::agent_roles::merge_missing_agent_roles_from_plugin_dirs;
use codex_connectors_api::metadata::connector_display_label;
use codex_connectors_api::AppInfo;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::PluginInstructions;
use codex_core_skills_api::collect_tool_mentions_from_messages_with_sigil;
use codex_core_skills_api::injection::ToolMentionKind;
use codex_core_skills_api::injection::plugin_config_name_from_path;
use codex_core_skills_api::injection::tool_kind_for_path;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_protocol::models::ResponseItem;
use codex_protocol::user_input::UserInput;
use codex_file_system::LOCAL_FS;
use plugin_service_api::PLUGIN_TEXT_MENTION_SIGIL;
use plugin_service_api::PluginCapabilitySummary;
use plugin_service_api::PluginLoadOutcome;

pub fn collect_explicit_plugin_mentions(
    input: &[UserInput],
    plugins: &[PluginCapabilitySummary],
) -> Vec<PluginCapabilitySummary> {
    if plugins.is_empty() {
        return Vec::new();
    }

    let messages = input
        .iter()
        .filter_map(|item| match item {
            UserInput::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<String>>();

    let mentioned_config_names: HashSet<String> = input
        .iter()
        .filter_map(|item| match item {
            UserInput::Mention { path, .. } => Some(path.clone()),
            _ => None,
        })
        .chain(
            collect_tool_mentions_from_messages_with_sigil(&messages, PLUGIN_TEXT_MENTION_SIGIL)
                .paths,
        )
        .filter(|path| tool_kind_for_path(path.as_str()) == ToolMentionKind::Plugin)
        .filter_map(|path| plugin_config_name_from_path(path.as_str()).map(str::to_string))
        .collect();

    if mentioned_config_names.is_empty() {
        return Vec::new();
    }

    plugins
        .iter()
        .filter(|plugin| mentioned_config_names.contains(plugin.config_name.as_str()))
        .cloned()
        .collect()
}

pub fn build_plugin_injections(
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

pub async fn merge_plugin_agent_roles(
    agent_roles: &mut std::collections::BTreeMap<String, AgentRoleConfig>,
    startup_warnings: &mut Vec<String>,
    plugin_outcome: &PluginLoadOutcome,
) {
    let plugin_agent_dirs = plugin_outcome
        .effective_plugin_agent_dirs()
        .into_iter()
        .map(|agent_dir| (agent_dir.plugin_id, agent_dir.path))
        .collect::<Vec<_>>();
    if plugin_agent_dirs.is_empty() {
        return;
    }

    let mut warnings = Vec::new();
    if let Err(err) = merge_missing_agent_roles_from_plugin_dirs(
        LOCAL_FS.as_ref(),
        agent_roles,
        &plugin_agent_dirs,
        &mut warnings,
    )
    .await
    {
        tracing::warn!("failed to load plugin agent definitions: {err}");
    }
    startup_warnings.extend(warnings);
}

pub async fn load_and_merge_plugin_agent_roles_for_config(
    plugin_runtime: &dyn plugin_service_api::PluginRuntime,
    plugins_input: &plugin_service_api::PluginsConfigInput,
    agent_roles: &mut std::collections::BTreeMap<String, AgentRoleConfig>,
    startup_warnings: &mut Vec<String>,
) {
    let plugin_outcome = plugin_runtime.plugins_for_config(plugins_input).await;
    merge_plugin_agent_roles(agent_roles, startup_warnings, &plugin_outcome).await;
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

#[cfg(test)]
#[path = "mentions_tests.rs"]
mod tests;
