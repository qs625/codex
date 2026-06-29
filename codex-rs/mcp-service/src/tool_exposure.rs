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

#[cfg(test)]
mod tests {
    use super::*;
    use codex_config::ConfigBuilder;
    use codex_features::Features;
    use codex_mcp_tool_types::McpTool;
    use codex_models_manager::test_support::construct_model_info_offline_for_tests;
    use codex_protocol::config_types::WebSearchMode;
    use codex_protocol::config_types::WindowsSandboxLevel;
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::protocol::SessionSource;
    use codex_tool_config::ToolsConfigParams;
    use pretty_assertions::assert_eq;
    use std::collections::HashSet;
    use tempfile::tempdir;

    fn make_connector(id: &str, name: &str) -> AppInfo {
        AppInfo {
            id: id.to_string(),
            name: name.to_string(),
            description: None,
            logo_url: None,
            logo_url_dark: None,
            distribution_channel: None,
            branding: None,
            app_metadata: None,
            labels: None,
            install_url: None,
            is_accessible: true,
            is_enabled: true,
            plugin_display_names: Vec::new(),
        }
    }

    fn make_mcp_tool(
        server_name: &str,
        tool_name: &str,
        callable_namespace: &str,
        callable_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
    ) -> McpToolInfo {
        McpToolInfo {
            server_name: server_name.to_string(),
            supports_parallel_tool_calls: false,
            server_origin: None,
            callable_name: callable_name.to_string(),
            callable_namespace: callable_namespace.to_string(),
            namespace_description: None,
            tool: McpTool::new(
                tool_name,
                format!("Test tool: {tool_name}"),
                serde_json::Value::Object(serde_json::Map::new()),
            ),
            connector_id: connector_id.map(str::to_string),
            connector_name: connector_name.map(str::to_string),
            plugin_display_names: Vec::new(),
        }
    }

    fn numbered_mcp_tools(count: usize) -> Vec<McpToolInfo> {
        (0..count)
            .map(|index| {
                let tool_name = format!("tool_{index}");
                make_mcp_tool(
                    "rmcp",
                    &tool_name,
                    "mcp__rmcp__",
                    &tool_name,
                    /*connector_id*/ None,
                    /*connector_name*/ None,
                )
            })
            .collect()
    }

    fn tool_names(tools: &[McpToolInfo]) -> HashSet<String> {
        tools
            .iter()
            .map(|tool| tool.canonical_tool_name().to_string())
            .collect()
    }

    async fn test_config() -> Config {
        let codex_home = tempdir().expect("temp dir");
        ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .build()
            .await
            .expect("load default test config")
    }

    async fn tools_config_for_mcp_tool_exposure(search_tool: bool) -> ToolsConfig {
        let config = test_config().await;
        let model_info =
            construct_model_info_offline_for_tests("gpt-5.4", &config.to_models_manager_config());
        let features = Features::with_defaults();
        let available_models = Vec::new();
        let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            available_models: &available_models,
            features: &features,
            image_generation_tool_auth_allowed: true,
            web_search_mode: Some(WebSearchMode::Cached),
            session_source: SessionSource::Cli,
            permission_profile: &PermissionProfile::Disabled,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
        });
        tools_config.search_tool = search_tool;
        tools_config
    }

    #[tokio::test]
    async fn directly_exposes_small_effective_tool_sets() {
        let config = test_config().await;
        let tools_config = tools_config_for_mcp_tool_exposure(/*search_tool*/ true).await;
        let mcp_tools = numbered_mcp_tools(DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD - 1);

        let exposure = build_mcp_tool_exposure(
            &mcp_tools,
            /*connectors*/ None,
            &[],
            &config,
            &tools_config,
        );

        assert_eq!(tool_names(&exposure.direct_tools), tool_names(&mcp_tools));
        assert!(exposure.deferred_tools.is_none());
    }

    #[tokio::test]
    async fn searches_large_effective_tool_sets() {
        let config = test_config().await;
        let tools_config = tools_config_for_mcp_tool_exposure(/*search_tool*/ true).await;
        let mcp_tools = numbered_mcp_tools(DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD);

        let exposure = build_mcp_tool_exposure(
            &mcp_tools,
            /*connectors*/ None,
            &[],
            &config,
            &tools_config,
        );

        assert!(exposure.direct_tools.is_empty());
        let deferred_tools = exposure
            .deferred_tools
            .as_ref()
            .expect("large tool sets should be discoverable through tool_search");
        assert_eq!(tool_names(deferred_tools), tool_names(&mcp_tools));
    }

    #[tokio::test]
    async fn directly_exposes_explicit_apps_without_deferred_overlap() {
        let config = test_config().await;
        let tools_config = tools_config_for_mcp_tool_exposure(/*search_tool*/ true).await;
        let mut mcp_tools = numbered_mcp_tools(DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD - 1);
        mcp_tools.push(make_mcp_tool(
            CODEX_APPS_MCP_SERVER_NAME,
            "calendar_create_event",
            "mcp__codex_apps__calendar",
            "_create_event",
            Some("calendar"),
            Some("Calendar"),
        ));
        let connectors = vec![make_connector("calendar", "Calendar")];

        let exposure = build_mcp_tool_exposure(
            &mcp_tools,
            Some(connectors.as_slice()),
            connectors.as_slice(),
            &config,
            &tools_config,
        );

        let direct_tool_names = tool_names(&exposure.direct_tools);
        assert_eq!(
            direct_tool_names,
            HashSet::from(["mcp__codex_apps__calendar_create_event".to_string()])
        );
        assert_eq!(
            exposure.deferred_tools.as_ref().map(Vec::len),
            Some(DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD - 1)
        );
        let deferred_tools = exposure
            .deferred_tools
            .as_ref()
            .expect("large tool sets should be discoverable through tool_search");
        let deferred_tool_names = tool_names(deferred_tools);
        assert!(
            direct_tool_names.is_disjoint(&deferred_tool_names),
            "direct tools should not also be deferred: {direct_tool_names:?}"
        );
        assert!(!deferred_tool_names.contains("mcp__codex_apps__calendar_create_event"));
        assert!(deferred_tool_names.contains("mcp__rmcp__tool_0"));
    }

    #[tokio::test]
    async fn always_defer_feature_preserves_explicit_apps() {
        let mut config = test_config().await;
        config
            .features
            .enable(Feature::ToolSearchAlwaysDeferMcpTools)
            .expect("test config should allow feature update");
        let tools_config = tools_config_for_mcp_tool_exposure(/*search_tool*/ true).await;
        let mcp_tools = vec![
            make_mcp_tool(
                "rmcp",
                "tool",
                "mcp__rmcp__",
                "tool",
                /*connector_id*/ None,
                /*connector_name*/ None,
            ),
            make_mcp_tool(
                CODEX_APPS_MCP_SERVER_NAME,
                "calendar_create_event",
                "mcp__codex_apps__calendar",
                "_create_event",
                Some("calendar"),
                Some("Calendar"),
            ),
        ];
        let connectors = vec![make_connector("calendar", "Calendar")];

        let exposure = build_mcp_tool_exposure(
            &mcp_tools,
            Some(connectors.as_slice()),
            connectors.as_slice(),
            &config,
            &tools_config,
        );

        let direct_tool_names = tool_names(&exposure.direct_tools);
        assert_eq!(
            direct_tool_names,
            HashSet::from(["mcp__codex_apps__calendar_create_event".to_string()])
        );
        let deferred_tools = exposure
            .deferred_tools
            .as_ref()
            .expect("MCP tools should be discoverable through tool_search");
        let deferred_tool_names = tool_names(deferred_tools);
        assert!(deferred_tool_names.contains("mcp__rmcp__tool"));
        assert!(!deferred_tool_names.contains("mcp__codex_apps__calendar_create_event"));
    }
}
