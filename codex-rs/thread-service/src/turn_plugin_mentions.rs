use std::collections::HashSet;

use plugin_service_api::PLUGIN_TEXT_MENTION_SIGIL;
use plugin_service_api::PluginCapabilitySummary;
use protocol::user_input::UserInput;
use skill_service_api::collect_tool_mentions_from_messages_with_sigil;
use skill_service_api::injection::ToolMentionKind;
use skill_service_api::injection::plugin_config_name_from_path;
use skill_service_api::injection::tool_kind_for_path;

pub(crate) fn collect_explicit_plugin_mentions(
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
