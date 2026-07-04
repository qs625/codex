use codex_context_manager::AvailablePluginsInstructions;
use codex_context_manager::ContextualUserFragment;
use plugin_service_api::PluginCapabilitySummary;
use pretty_assertions::assert_eq;
use protocol::user_input::UserInput;

use super::collect_explicit_plugin_mentions;
use super::render_explicit_plugin_instructions;

fn text_input(text: &str) -> UserInput {
    UserInput::Text {
        text: text.to_string(),
        text_elements: Vec::new(),
    }
}

fn plugin(config_name: &str, display_name: &str) -> PluginCapabilitySummary {
    PluginCapabilitySummary {
        config_name: config_name.to_string(),
        display_name: display_name.to_string(),
        description: None,
        has_skills: true,
        mcp_server_names: Vec::new(),
        app_connector_ids: Vec::new(),
    }
}

#[test]
fn collect_explicit_plugin_mentions_from_structured_paths() {
    let plugins = vec![
        plugin("sample@test", "sample"),
        plugin("other@test", "other"),
    ];

    let mentioned = collect_explicit_plugin_mentions(
        &[UserInput::Mention {
            name: "sample".to_string(),
            path: "plugin://sample@test".to_string(),
        }],
        &plugins,
    );

    assert_eq!(mentioned, vec![plugin("sample@test", "sample")]);
}

#[test]
fn collect_explicit_plugin_mentions_from_linked_text_mentions() {
    let plugins = vec![
        plugin("sample@test", "sample"),
        plugin("other@test", "other"),
    ];

    let mentioned = collect_explicit_plugin_mentions(
        &[text_input("use [@sample](plugin://sample@test)")],
        &plugins,
    );

    assert_eq!(mentioned, vec![plugin("sample@test", "sample")]);
}

#[test]
fn collect_explicit_plugin_mentions_dedupes_structured_and_linked_mentions() {
    let plugins = vec![
        plugin("sample@test", "sample"),
        plugin("other@test", "other"),
    ];

    let mentioned = collect_explicit_plugin_mentions(
        &[
            text_input("use [@sample](plugin://sample@test)"),
            UserInput::Mention {
                name: "sample".to_string(),
                path: "plugin://sample@test".to_string(),
            },
        ],
        &plugins,
    );

    assert_eq!(mentioned, vec![plugin("sample@test", "sample")]);
}

#[test]
fn collect_explicit_plugin_mentions_ignores_non_plugin_paths() {
    let plugins = vec![plugin("sample@test", "sample")];

    let mentioned = collect_explicit_plugin_mentions(
        &[text_input(
            "use [$app](app://calendar) and [$skill](skill://team/skill) and [$file](/tmp/file.txt)",
        )],
        &plugins,
    );

    assert_eq!(mentioned, Vec::<PluginCapabilitySummary>::new());
}

#[test]
fn collect_explicit_plugin_mentions_ignores_dollar_linked_plugin_mentions() {
    let plugins = vec![plugin("sample@test", "sample")];

    let mentioned = collect_explicit_plugin_mentions(
        &[text_input("use [$sample](plugin://sample@test)")],
        &plugins,
    );

    assert_eq!(mentioned, Vec::<PluginCapabilitySummary>::new());
}

#[test]
fn render_plugins_section_returns_none_for_empty_plugins() {
    assert_eq!(
        AvailablePluginsInstructions::from_plugins(&[]).map(|v| v.render()),
        None
    );
}

#[test]
fn render_plugins_section_includes_descriptions_and_skill_naming_guidance() {
    let rendered = AvailablePluginsInstructions::from_plugins(&[PluginCapabilitySummary {
        config_name: "sample@test".to_string(),
        display_name: "sample".to_string(),
        description: Some("inspect sample data".to_string()),
        has_skills: true,
        ..PluginCapabilitySummary::default()
    }])
    .map(|instructions| instructions.render())
    .expect("plugin section should render");

    let expected = "<plugins_instructions>\n## Plugins\nA plugin is a local bundle of skills, MCP servers, and apps. Below is the list of plugins that are enabled and available in this session.\n### Available plugins\n- `sample`: inspect sample data\n### How to use plugins\n- Discovery: The list above is the plugins available in this session.\n- Skill naming: If a plugin contributes skills, those skill entries are prefixed with `plugin_name:` in the Skills list.\n- Trigger rules: If the user explicitly names a plugin, prefer capabilities associated with that plugin for that turn.\n- Relationship to capabilities: Plugins are not invoked directly. Use their underlying skills, MCP tools, and app tools to help solve the task.\n- Preference: When a relevant plugin is available, prefer using capabilities associated with that plugin over standalone capabilities that provide similar functionality.\n- Missing/blocked: If the user requests a plugin that is not listed above, or the plugin does not have relevant callable capabilities for that turn, say so briefly and continue with the best fallback.\n</plugins_instructions>";

    assert_eq!(rendered, expected);
}

#[test]
fn render_explicit_plugin_instructions_returns_none_without_visible_capabilities() {
    assert_eq!(
        render_explicit_plugin_instructions(&plugin("sample@test", "sample"), &[], &[]),
        None
    );
}

#[test]
fn render_explicit_plugin_instructions_includes_skills_and_tools() {
    let rendered = render_explicit_plugin_instructions(
        &plugin("sample@test", "sample"),
        &["docs".to_string()],
        &["Calendar".to_string()],
    )
    .expect("rendered");

    assert!(rendered.contains("`sample`"));
    assert!(rendered.contains("`docs`"));
    assert!(rendered.contains("`Calendar`"));
}
