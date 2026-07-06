use super::HookPromptFragment;
use super::HookPromptItem;
use crate::models::ContentItem;
use crate::models::ResponseItem;

impl HookPromptItem {
    pub fn from_fragments(id: Option<&String>, fragments: Vec<HookPromptFragment>) -> Self {
        Self {
            id: id
                .cloned()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            fragments,
        }
    }
}

impl HookPromptFragment {
    pub fn from_single_hook(text: impl Into<String>, hook_run_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            hook_run_id: hook_run_id.into(),
        }
    }
}

pub fn build_hook_prompt_message(fragments: &[HookPromptFragment]) -> Option<ResponseItem> {
    let content = fragments
        .iter()
        .filter(|fragment| !fragment.hook_run_id.trim().is_empty())
        .filter_map(|fragment| {
            serialize_hook_prompt_fragment(&fragment.text, &fragment.hook_run_id)
                .map(|text| ContentItem::InputText { text })
        })
        .collect::<Vec<_>>();

    if content.is_empty() {
        return None;
    }

    Some(ResponseItem::Message {
        id: Some(uuid::Uuid::new_v4().to_string()),
        role: "user".to_string(),
        content,
        phase: None,
    })
}

pub fn parse_hook_prompt_message(
    id: Option<&String>,
    content: &[ContentItem],
) -> Option<HookPromptItem> {
    let fragments = content
        .iter()
        .map(|content_item| {
            let ContentItem::InputText { text } = content_item else {
                return None;
            };
            parse_hook_prompt_fragment(text)
        })
        .collect::<Option<Vec<_>>>()?;

    if fragments.is_empty() {
        return None;
    }

    Some(HookPromptItem::from_fragments(id, fragments))
}

pub fn parse_hook_prompt_fragment(text: &str) -> Option<HookPromptFragment> {
    let trimmed = text.trim();
    let (text, hook_run_id) = parse_hook_prompt_xml(trimmed)?;
    if hook_run_id.trim().is_empty() {
        return None;
    }

    Some(HookPromptFragment { text, hook_run_id })
}

fn serialize_hook_prompt_fragment(text: &str, hook_run_id: &str) -> Option<String> {
    if hook_run_id.trim().is_empty() {
        return None;
    }
    Some(format!(
        "<hook_prompt hook_run_id=\"{}\">{}</hook_prompt>",
        escape_xml_attr(hook_run_id),
        escape_xml_text(text)
    ))
}

fn parse_hook_prompt_xml(input: &str) -> Option<(String, String)> {
    let without_open = input.strip_prefix("<hook_prompt")?;
    let tag_end = without_open.find('>')?;
    let start_tag = &without_open[..tag_end];
    if !start_tag.is_empty() && !start_tag.starts_with(char::is_whitespace) {
        return None;
    }
    let content_and_close = &without_open[tag_end + 1..];
    let content = content_and_close.strip_suffix("</hook_prompt>")?;
    let hook_run_id = parse_xml_attr(start_tag, "hook_run_id")?;
    Some((unescape_xml(content)?, hook_run_id))
}

fn parse_xml_attr(tag: &str, attr_name: &str) -> Option<String> {
    let mut search_start = 0;
    let attr_start = loop {
        let relative_start = tag[search_start..].find(attr_name)?;
        let attr_start = search_start + relative_start;
        let before_ok = attr_start == 0
            || tag[..attr_start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let after_start = attr_start + attr_name.len();
        let after_ok = tag[after_start..]
            .chars()
            .next()
            .is_some_and(|ch| ch == '=' || ch.is_whitespace());
        if before_ok && after_ok {
            break attr_start;
        }
        search_start = after_start;
    };
    let rest = &tag[attr_start + attr_name.len()..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value_start = quote.len_utf8();
    let value_end = rest[value_start..].find(quote)?;
    unescape_xml(&rest[value_start..value_start + value_end])
}

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attr(value: &str) -> String {
    escape_xml_text(value)
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unescape_xml(value: &str) -> Option<String> {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(entity_start) = rest.find('&') {
        output.push_str(&rest[..entity_start]);
        rest = &rest[entity_start + 1..];
        let entity_end = rest.find(';')?;
        let entity = &rest[..entity_end];
        let decoded = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ if entity.starts_with("#x") => {
                let code = u32::from_str_radix(&entity[2..], 16).ok()?;
                char::from_u32(code)?
            }
            _ if entity.starts_with('#') => {
                let code = entity[1..].parse::<u32>().ok()?;
                char::from_u32(code)?
            }
            _ => return None,
        };
        output.push(decoded);
        rest = &rest[entity_end + 1..];
    }
    output.push_str(rest);
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn hook_prompt_roundtrips_multiple_fragments() {
        let original = vec![
            HookPromptFragment::from_single_hook("Retry with care & joy.", "hook-run-1"),
            HookPromptFragment::from_single_hook("Then summarize cleanly.", "hook-run-2"),
        ];
        let message = build_hook_prompt_message(&original).expect("hook prompt");

        let ResponseItem::Message { content, .. } = message else {
            panic!("expected hook prompt message");
        };

        let parsed = parse_hook_prompt_message(/*id*/ None, &content).expect("parsed hook prompt");
        assert_eq!(parsed.fragments, original);
    }

    #[test]
    fn hook_prompt_parses_legacy_single_hook_run_id() {
        let parsed = parse_hook_prompt_fragment(
            r#"<hook_prompt hook_run_id="hook-run-1">Retry with tests.</hook_prompt>"#,
        )
        .expect("legacy hook prompt");

        assert_eq!(
            parsed,
            HookPromptFragment {
                text: "Retry with tests.".to_string(),
                hook_run_id: "hook-run-1".to_string(),
            }
        );
    }
}
