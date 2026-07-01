use std::collections::HashMap;
use std::collections::HashSet;

use codex_connectors_api::metadata::connector_mention_slug;
use codex_connectors_api::AppInfo;
use plugin_service_api::TOOL_MENTION_SIGIL;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;

use crate::injection::ToolMentionKind;
use crate::injection::app_id_from_path;
use crate::injection::extract_tool_mentions_with_sigil;
use crate::injection::tool_kind_for_path;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CollectedToolMentions {
    pub plain_names: HashSet<String>,
    pub paths: HashSet<String>,
}

pub fn collect_tool_mentions_from_messages(messages: &[String]) -> CollectedToolMentions {
    collect_tool_mentions_from_messages_with_sigil(messages, TOOL_MENTION_SIGIL)
}

pub fn collect_tool_mentions_from_messages_with_sigil(
    messages: &[String],
    sigil: char,
) -> CollectedToolMentions {
    let mut plain_names = HashSet::new();
    let mut paths = HashSet::new();
    for message in messages {
        let mentions = extract_tool_mentions_with_sigil(message, sigil);
        plain_names.extend(mentions.plain_names().map(str::to_string));
        paths.extend(mentions.paths().map(str::to_string));
    }
    CollectedToolMentions { plain_names, paths }
}

pub fn build_connector_slug_counts(connectors: &[AppInfo]) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for connector in connectors {
        let slug = connector_mention_slug(connector);
        *counts.entry(slug).or_insert(0) += 1;
    }
    counts
}

pub fn collect_explicit_app_ids_from_skill_items(
    skill_items: &[ResponseItem],
    connectors: &[AppInfo],
    skill_name_counts_lower: &HashMap<String, usize>,
) -> HashSet<String> {
    if skill_items.is_empty() || connectors.is_empty() {
        return HashSet::new();
    }

    let skill_messages = skill_items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { content, .. } => {
                content.iter().find_map(|content_item| match content_item {
                    ContentItem::InputText { text } => Some(text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .collect::<Vec<String>>();
    if skill_messages.is_empty() {
        return HashSet::new();
    }

    collect_explicit_app_ids_from_messages(&skill_messages, connectors, skill_name_counts_lower)
}

pub fn collect_explicit_app_ids_from_messages(
    messages: &[String],
    connectors: &[AppInfo],
    skill_name_counts_lower: &HashMap<String, usize>,
) -> HashSet<String> {
    let mentions = collect_tool_mentions_from_messages(messages);
    let mention_names_lower = mentions
        .plain_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<String>>();
    let mut connector_ids = mentions
        .paths
        .iter()
        .filter(|path| tool_kind_for_path(path) == ToolMentionKind::App)
        .filter_map(|path| app_id_from_path(path).map(str::to_string))
        .collect::<HashSet<String>>();

    let connector_slug_counts = build_connector_slug_counts(connectors);
    for connector in connectors {
        let slug = connector_mention_slug(connector);
        let connector_count = connector_slug_counts.get(&slug).copied().unwrap_or(0);
        let skill_count = skill_name_counts_lower.get(&slug).copied().unwrap_or(0);
        if connector_count == 1 && skill_count == 0 && mention_names_lower.contains(&slug) {
            connector_ids.insert(connector.id.clone());
        }
    }
    connector_ids
}

pub fn filter_connectors_for_user_messages(
    connectors: &[AppInfo],
    user_messages: &[String],
    explicitly_enabled_connectors: &HashSet<String>,
    skill_name_counts_lower: &HashMap<String, usize>,
) -> Vec<AppInfo> {
    let connectors = connectors
        .iter()
        .filter(|connector| connector.is_enabled)
        .cloned()
        .collect::<Vec<_>>();
    if connectors.is_empty() {
        return Vec::new();
    }

    if user_messages.is_empty() && explicitly_enabled_connectors.is_empty() {
        return Vec::new();
    }

    let mentions = collect_tool_mentions_from_messages(user_messages);
    let mention_names_lower = mentions
        .plain_names
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<String>>();

    let connector_slug_counts = build_connector_slug_counts(&connectors);
    let mut allowed_connector_ids = explicitly_enabled_connectors.clone();
    for path in mentions
        .paths
        .iter()
        .filter(|path| tool_kind_for_path(path) == ToolMentionKind::App)
    {
        if let Some(connector_id) = app_id_from_path(path) {
            allowed_connector_ids.insert(connector_id.to_string());
        }
    }

    connectors
        .into_iter()
        .filter(|connector| {
            connector_inserted_in_messages(
                connector,
                &mention_names_lower,
                &allowed_connector_ids,
                &connector_slug_counts,
                skill_name_counts_lower,
            )
        })
        .collect()
}

fn connector_inserted_in_messages(
    connector: &AppInfo,
    mention_names_lower: &HashSet<String>,
    allowed_connector_ids: &HashSet<String>,
    connector_slug_counts: &HashMap<String, usize>,
    skill_name_counts_lower: &HashMap<String, usize>,
) -> bool {
    if allowed_connector_ids.contains(&connector.id) {
        return true;
    }

    let mention_slug = connector_mention_slug(connector);
    let connector_count = connector_slug_counts
        .get(&mention_slug)
        .copied()
        .unwrap_or(0);
    let skill_count = skill_name_counts_lower
        .get(&mention_slug)
        .copied()
        .unwrap_or(0);
    connector_count == 1 && skill_count == 0 && mention_names_lower.contains(&mention_slug)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn skill_message(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    #[test]
    fn filter_connectors_for_user_messages_skips_duplicate_slug_mentions() {
        let connectors = vec![
            make_connector("one", "Foo Bar"),
            make_connector("two", "Foo-Bar"),
        ];
        let user_messages = vec!["use $foo-bar".to_string()];
        let explicitly_enabled_connectors = HashSet::new();
        let skill_name_counts_lower = HashMap::new();

        let selected = filter_connectors_for_user_messages(
            &connectors,
            &user_messages,
            &explicitly_enabled_connectors,
            &skill_name_counts_lower,
        );

        assert_eq!(selected, Vec::new());
    }

    #[test]
    fn filter_connectors_for_user_messages_skips_when_skill_name_conflicts() {
        let connectors = vec![make_connector("one", "Todoist")];
        let user_messages = vec!["use $todoist".to_string()];
        let explicitly_enabled_connectors = HashSet::new();
        let skill_name_counts_lower = HashMap::from([("todoist".to_string(), 1)]);

        let selected = filter_connectors_for_user_messages(
            &connectors,
            &user_messages,
            &explicitly_enabled_connectors,
            &skill_name_counts_lower,
        );

        assert_eq!(selected, Vec::new());
    }

    #[test]
    fn filter_connectors_for_user_messages_skips_disabled_connectors() {
        let mut connector = make_connector("calendar", "Calendar");
        connector.is_enabled = false;
        let user_messages = vec!["use $calendar".to_string()];
        let explicitly_enabled_connectors = HashSet::new();
        let selected = filter_connectors_for_user_messages(
            &[connector],
            &user_messages,
            &explicitly_enabled_connectors,
            &HashMap::new(),
        );

        assert_eq!(selected, Vec::new());
    }

    #[test]
    fn filter_connectors_for_user_messages_skips_plugin_mentions() {
        let connectors = vec![make_connector("figma", "Figma")];
        let user_messages = vec!["use [@figma](plugin://figma@openai-curated)".to_string()];
        let explicitly_enabled_connectors = HashSet::new();
        let selected = filter_connectors_for_user_messages(
            &connectors,
            &user_messages,
            &explicitly_enabled_connectors,
            &HashMap::new(),
        );

        assert_eq!(selected, Vec::new());
    }

    #[test]
    fn collect_explicit_app_ids_from_skill_items_includes_linked_mentions() {
        let connectors = vec![make_connector("calendar", "Calendar")];
        let skill_items = vec![skill_message(
            "<skill>\n<name>demo</name>\n<path>/tmp/skills/demo/SKILL.md</path>\nuse [$calendar](app://calendar)\n</skill>",
        )];

        let connector_ids =
            collect_explicit_app_ids_from_skill_items(&skill_items, &connectors, &HashMap::new());

        assert_eq!(connector_ids, HashSet::from(["calendar".to_string()]));
    }

    #[test]
    fn collect_explicit_app_ids_from_skill_items_resolves_unambiguous_plain_mentions() {
        let connectors = vec![make_connector("calendar", "Calendar")];
        let skill_items = vec![skill_message(
            "<skill>\n<name>demo</name>\n<path>/tmp/skills/demo/SKILL.md</path>\nuse $calendar\n</skill>",
        )];

        let connector_ids =
            collect_explicit_app_ids_from_skill_items(&skill_items, &connectors, &HashMap::new());

        assert_eq!(connector_ids, HashSet::from(["calendar".to_string()]));
    }

    #[test]
    fn collect_explicit_app_ids_from_skill_items_skips_plain_mentions_with_skill_conflicts() {
        let connectors = vec![make_connector("calendar", "Calendar")];
        let skill_items = vec![skill_message(
            "<skill>\n<name>demo</name>\n<path>/tmp/skills/demo/SKILL.md</path>\nuse $calendar\n</skill>",
        )];
        let skill_name_counts_lower = HashMap::from([("calendar".to_string(), 1)]);

        let connector_ids = collect_explicit_app_ids_from_skill_items(
            &skill_items,
            &connectors,
            &skill_name_counts_lower,
        );

        assert_eq!(connector_ids, HashSet::<String>::new());
    }
}
