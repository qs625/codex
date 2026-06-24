use super::*;
use codex_connectors_types::AppInfo;
use codex_tool_types::JsonSchema;
use codex_tool_types::ResponsesApiTool;
use codex_tool_types::ToolSpec;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn discoverable_tool_enums_use_expected_wire_names() {
    assert_eq!(
        json!({
            "tool_type": DiscoverableToolType::Connector,
            "action_type": DiscoverableToolAction::Install,
        }),
        json!({
            "tool_type": "connector",
            "action_type": "install",
        })
    );
}

#[test]
fn tool_search_info_from_spec_converts_function_to_loadable_output() {
    let source_info = ToolSearchSourceInfo {
        name: "Calendar".to_string(),
        description: Some("Calendar tools".to_string()),
    };
    let info = ToolSearchInfo::from_spec(
        "create calendar event".to_string(),
        ToolSpec::Function(ResponsesApiTool {
            name: "create_event".to_string(),
            description: "Create an event".to_string(),
            strict: true,
            parameters: JsonSchema::object(
                /*properties*/ Default::default(),
                /*required*/ None,
                /*additional_properties*/ None,
            ),
            output_schema: Some(json!({
                "type": "object",
                "properties": {}
            })),
            defer_loading: None,
        }),
        Some(source_info.clone()),
    )
    .expect("function spec should be searchable");

    assert_eq!(info.entry.search_text, "create calendar event");
    assert_eq!(info.source_info, Some(source_info));
    let codex_tool_types::LoadableToolSpec::Function(tool) = info.entry.output else {
        panic!("expected function output");
    };
    assert_eq!(tool.name, "create_event");
    assert_eq!(tool.defer_loading, Some(true));
    assert_eq!(tool.output_schema, None);
}

#[test]
fn filter_request_plugin_install_discoverable_tools_for_codex_tui_omits_plugins() {
    let discoverable_tools = vec![
        DiscoverableTool::Connector(Box::new(AppInfo {
            id: "connector_google_calendar".to_string(),
            name: "Google Calendar".to_string(),
            description: Some("Plan events and schedules.".to_string()),
            logo_url: None,
            logo_url_dark: None,
            distribution_channel: None,
            branding: None,
            app_metadata: None,
            labels: None,
            install_url: Some("https://example.test/google-calendar".to_string()),
            is_accessible: false,
            is_enabled: true,
            plugin_display_names: Vec::new(),
        })),
        DiscoverableTool::Plugin(Box::new(DiscoverablePluginInfo {
            id: "slack@openai-curated".to_string(),
            name: "Slack".to_string(),
            description: Some("Search Slack messages".to_string()),
            has_skills: true,
            mcp_server_names: vec!["slack".to_string()],
            app_connector_ids: vec!["connector_slack".to_string()],
        })),
    ];

    assert_eq!(
        filter_request_plugin_install_discoverable_tools_for_client(
            discoverable_tools,
            Some("codex-tui"),
        ),
        vec![DiscoverableTool::Connector(Box::new(AppInfo {
            id: "connector_google_calendar".to_string(),
            name: "Google Calendar".to_string(),
            description: Some("Plan events and schedules.".to_string()),
            logo_url: None,
            logo_url_dark: None,
            distribution_channel: None,
            branding: None,
            app_metadata: None,
            labels: None,
            install_url: Some("https://example.test/google-calendar".to_string()),
            is_accessible: false,
            is_enabled: true,
            plugin_display_names: Vec::new(),
        }))]
    );
}
