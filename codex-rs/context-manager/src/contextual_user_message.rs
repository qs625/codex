use codex_protocol::items::HookPromptItem;
use codex_protocol::items::parse_hook_prompt_fragment;
use codex_protocol::models::ContentItem;
use codex_protocol::protocol::COLLABORATION_MODE_OPEN_TAG;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_CLOSE_TAG;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_OPEN_TAG;
use codex_protocol::protocol::REALTIME_CONVERSATION_OPEN_TAG;

const CONTEXTUAL_USER_MARKERS: &[(&str, &str)] = &[
    ("# AGENTS.md instructions for ", "</INSTRUCTIONS>"),
    ("<agents_instructions>", "</agents_instructions>"),
    (ENVIRONMENT_CONTEXT_OPEN_TAG, ENVIRONMENT_CONTEXT_CLOSE_TAG),
    ("<multiagent_context>", "</multiagent_context>"),
    ("<skill>", "</skill>"),
    ("<user_shell_command>", "</user_shell_command>"),
    ("<turn_aborted>", "</turn_aborted>"),
    ("<subagent_notification>", "</subagent_notification>"),
    ("<goal_context>", "</goal_context>"),
];

const CONTEXTUAL_DEVELOPER_PREFIXES: &[&str] = &[
    "<permissions instructions>",
    "<model_switch>",
    COLLABORATION_MODE_OPEN_TAG,
    REALTIME_CONVERSATION_OPEN_TAG,
    "<personality_spec>",
];

fn matches_contextual_markers(text: &str, start_marker: &str, end_marker: &str) -> bool {
    if start_marker.is_empty() || end_marker.is_empty() {
        return false;
    }

    let trimmed = text.trim_start();
    let starts_with_marker = trimmed
        .get(..start_marker.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(start_marker));
    let trimmed = trimmed.trim_end();
    let ends_with_marker = trimmed
        .get(trimmed.len().saturating_sub(end_marker.len())..)
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(end_marker));
    starts_with_marker && ends_with_marker
}

fn matches_contextual_prefix(text: &str, prefix: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}

fn is_standard_contextual_user_text(text: &str) -> bool {
    CONTEXTUAL_USER_MARKERS
        .iter()
        .any(|(start_marker, end_marker)| {
            matches_contextual_markers(text, start_marker, end_marker)
        })
}

pub fn is_contextual_user_fragment(content_item: &ContentItem) -> bool {
    let ContentItem::InputText { text } = content_item else {
        return false;
    };
    parse_hook_prompt_fragment(text).is_some() || is_standard_contextual_user_text(text)
}

pub fn is_contextual_user_message_content(message: &[ContentItem]) -> bool {
    message.iter().any(is_contextual_user_fragment)
}

fn is_contextual_dev_fragment(content_item: &ContentItem) -> bool {
    let ContentItem::InputText { text } = content_item else {
        return false;
    };

    CONTEXTUAL_DEVELOPER_PREFIXES
        .iter()
        .any(|prefix| matches_contextual_prefix(text, prefix))
}

/// Returns true when a developer message contains any rollback-trimmable contextual fragment.
///
/// Initial context can bundle these fragments together with persistent developer text in a single
/// developer message, so callers that care about invalidating a stored reference baseline should
/// pair this with `has_non_contextual_dev_message_content`.
pub fn is_contextual_dev_message_content(message: &[ContentItem]) -> bool {
    message.iter().any(is_contextual_dev_fragment)
}

/// Returns true when a developer message contains any fragment that is not part of the
/// rollback-trimmable contextual prefix set.
pub fn has_non_contextual_dev_message_content(message: &[ContentItem]) -> bool {
    message
        .iter()
        .any(|content_item| !is_contextual_dev_fragment(content_item))
}

pub fn parse_visible_hook_prompt_message(
    id: Option<&String>,
    content: &[ContentItem],
) -> Option<HookPromptItem> {
    let mut fragments = Vec::new();

    for content_item in content {
        let ContentItem::InputText { text } = content_item else {
            return None;
        };
        if let Some(fragment) = parse_hook_prompt_fragment(text) {
            fragments.push(fragment);
            continue;
        }
        if is_standard_contextual_user_text(text) {
            continue;
        }
        return None;
    }

    if fragments.is_empty() {
        return None;
    }

    Some(HookPromptItem::from_fragments(id, fragments))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextualUserFragment;
    use codex_protocol::items::HookPromptFragment;
    use codex_protocol::items::build_hook_prompt_message;
    use codex_protocol::models::ResponseItem;

    struct GoalContext {
        prompt: String,
    }

    impl ContextualUserFragment for GoalContext {
        const ROLE: &'static str = "user";
        const START_MARKER: &'static str = "<goal_context>";
        const END_MARKER: &'static str = "</goal_context>";

        fn body(&self) -> String {
            self.prompt.clone()
        }
    }

    #[test]
    fn detects_environment_context_fragment() {
        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text: "<environment_context>\n<cwd>/tmp</cwd>\n</environment_context>".to_string(),
        }));
    }

    #[test]
    fn detects_multiagent_context_fragment() {
        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text: "<multiagent_context>\n<current_thread_canonical_path>/root</current_thread_canonical_path>\n</multiagent_context>".to_string(),
        }));
    }

    #[test]
    fn detects_agents_instructions_fragment() {
        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text: "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nbody\n</INSTRUCTIONS>"
                .to_string(),
        }));
    }

    #[test]
    fn detects_available_agents_instructions_fragment() {
        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text: "<agents_instructions>\n## Agents\n</agents_instructions>".to_string(),
        }));
    }

    #[test]
    fn detects_subagent_notification_fragment_case_insensitively() {
        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text: "<SUBAGENT_NOTIFICATION>{}</subagent_notification>".to_string(),
        }));
    }

    #[test]
    fn detects_goal_context_fragment() {
        let text = GoalContext {
            prompt: "Continue working toward the active thread goal.".to_string(),
        }
        .render();

        assert!(is_contextual_user_fragment(&ContentItem::InputText {
            text
        }));
    }

    #[test]
    fn ignores_regular_user_text() {
        assert!(!is_contextual_user_fragment(&ContentItem::InputText {
            text: "hello".to_string(),
        }));
    }

    #[test]
    fn detects_contextual_developer_prefixes() {
        let message = [ContentItem::InputText {
            text: " \n<permissions instructions>body</permissions instructions>".to_string(),
        }];

        assert!(is_contextual_dev_message_content(&message));
        assert!(!has_non_contextual_dev_message_content(&message));
    }

    #[test]
    fn detects_mixed_contextual_developer_content() {
        let message = [
            ContentItem::InputText {
                text: "<permissions instructions>body</permissions instructions>".to_string(),
            },
            ContentItem::InputText {
                text: "persistent plugin instructions".to_string(),
            },
        ];

        assert!(is_contextual_dev_message_content(&message));
        assert!(has_non_contextual_dev_message_content(&message));
    }

    #[test]
    fn detects_hook_prompt_fragment_and_roundtrips_escaping() {
        let message = build_hook_prompt_message(&[HookPromptFragment::from_single_hook(
            r#"Retry with "waves" & <tides>"#,
            "hook-run-1",
        )])
        .expect("hook prompt message");

        let ResponseItem::Message { content, .. } = message else {
            panic!("expected hook prompt response item");
        };

        let [content_item] = content.as_slice() else {
            panic!("expected a single content item");
        };

        assert!(is_contextual_user_fragment(content_item));

        let ContentItem::InputText { text } = content_item else {
            panic!("expected input text content item");
        };
        let parsed = parse_visible_hook_prompt_message(/*id*/ None, content.as_slice())
            .expect("visible hook prompt");
        assert_eq!(
            parsed.fragments,
            vec![HookPromptFragment {
                text: r#"Retry with "waves" & <tides>"#.to_string(),
                hook_run_id: "hook-run-1".to_string(),
            }],
        );
        assert!(!text.contains("&quot;waves&quot; & <tides>"));
    }
}
