use std::collections::HashSet;

use codex_config::Config;
use codex_connectors_types::AppInfo;
use codex_features::Feature;
use codex_mcp_tool_types::ToolInfo as McpToolInfo;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_tool_config::ToolsConfig;

use crate::codex_app_tool_is_enabled;

pub const DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD: usize = 100;

pub struct McpToolExposure {
    pub direct_tools: Vec<McpToolInfo>,
    pub deferred_tools: Option<Vec<McpToolInfo>>,
}

pub fn build_mcp_tool_exposure(
    all_mcp_tools: &[McpToolInfo],
    connectors: Option<&[AppInfo]>,
    explicitly_enabled_connectors: &[AppInfo],
    config: &Config,
    tools_config: &ToolsConfig,
) -> McpToolExposure {
    let mut deferred_tools = filter_non_codex_apps_mcp_tools_only(all_mcp_tools);
    if let Some(connectors) = connectors {
        deferred_tools.extend(filter_codex_apps_mcp_tools(
            all_mcp_tools,
            connectors,
            config,
        ));
    }

    let should_defer = tools_config.search_tool
        && (config
            .features
            .enabled(Feature::ToolSearchAlwaysDeferMcpTools)
            || deferred_tools.len() >= DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD);

    if !should_defer {
        return McpToolExposure {
            direct_tools: deferred_tools,
            deferred_tools: None,
        };
    }

    let direct_tools =
        filter_codex_apps_mcp_tools(all_mcp_tools, explicitly_enabled_connectors, config);
    let direct_tool_names = direct_tools
        .iter()
        .map(McpToolInfo::canonical_tool_name)
        .collect::<HashSet<_>>();
    deferred_tools.retain(|tool| !direct_tool_names.contains(&tool.canonical_tool_name()));

    McpToolExposure {
        direct_tools,
        deferred_tools: (!deferred_tools.is_empty()).then_some(deferred_tools),
    }
}

fn filter_non_codex_apps_mcp_tools_only(mcp_tools: &[McpToolInfo]) -> Vec<McpToolInfo> {
    mcp_tools
        .iter()
        .filter(|tool| tool.server_name != CODEX_APPS_MCP_SERVER_NAME)
        .cloned()
        .collect()
}

fn filter_codex_apps_mcp_tools(
    mcp_tools: &[McpToolInfo],
    connectors: &[AppInfo],
    config: &Config,
) -> Vec<McpToolInfo> {
    let allowed: HashSet<&str> = connectors
        .iter()
        .map(|connector| connector.id.as_str())
        .collect();

    mcp_tools
        .iter()
        .filter(|tool| {
            if tool.server_name != CODEX_APPS_MCP_SERVER_NAME {
                return false;
            }
            let Some(connector_id) = tool.connector_id.as_deref() else {
                return false;
            };
            allowed.contains(connector_id) && codex_app_tool_is_enabled(config, tool)
        })
        .cloned()
        .collect()
}
