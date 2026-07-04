use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::models::is_image_close_tag_text;
use protocol::models::is_image_open_tag_text;
use protocol::models::is_local_image_close_tag_text;
use protocol::models::is_local_image_open_tag_text;

use crate::is_contextual_user_message_content;

pub const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;

pub fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

pub fn collect_compaction_user_messages(
    items: &[ResponseItem],
    summary_prefix: Option<&str>,
) -> Vec<String> {
    items
        .iter()
        .filter_map(user_message_text)
        .filter(|message| !is_compaction_summary_message(message, summary_prefix))
        .filter(|message| !is_legacy_compaction_warning_message(message))
        .collect()
}

fn user_message_text(item: &ResponseItem) -> Option<String> {
    let ResponseItem::Message { role, content, .. } = item else {
        return None;
    };
    if role != "user" || is_contextual_user_message_content(content) {
        return None;
    }

    let mut pieces = Vec::new();
    for (idx, content_item) in content.iter().enumerate() {
        match content_item {
            ContentItem::InputText { text } => {
                if (is_local_image_open_tag_text(text) || is_image_open_tag_text(text))
                    && matches!(content.get(idx + 1), Some(ContentItem::InputImage { .. }))
                    || (idx > 0
                        && (is_local_image_close_tag_text(text) || is_image_close_tag_text(text))
                        && matches!(content.get(idx - 1), Some(ContentItem::InputImage { .. })))
                {
                    continue;
                }
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } | ContentItem::OutputText { .. } => {}
        }
    }

    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join(""))
    }
}

pub fn is_compaction_summary_message(message: &str, summary_prefix: Option<&str>) -> bool {
    summary_prefix.is_some_and(|prefix| message.starts_with(format!("{prefix}\n").as_str()))
}

pub fn is_legacy_compaction_warning_message(message: &str) -> bool {
    message.starts_with(
        "Warning: The maximum number of unified exec processes you can keep open is ",
    ) || message.starts_with(
        "Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.",
    ) || message.starts_with(
        "Warning: Your account was flagged for potentially high-risk cyber activity",
    )
}

/// Inserts canonical initial context into compacted replacement history at the
/// model-expected boundary.
///
/// Placement rules:
/// - Prefer immediately before the last real user message.
/// - If no real user messages remain, insert before the compaction summary so
///   the summary stays last.
/// - If there are no user messages, insert before the last compaction item so
///   that item remains last.
/// - If there are no user messages or compaction items, append the context.
pub fn insert_initial_context_before_last_real_user_or_summary(
    mut compacted_history: Vec<ResponseItem>,
    initial_context: Vec<ResponseItem>,
    summary_prefix: Option<&str>,
) -> Vec<ResponseItem> {
    let mut last_user_or_summary_index = None;
    let mut last_real_user_index = None;
    for (i, item) in compacted_history.iter().enumerate().rev() {
        let Some(message) = user_message_text(item) else {
            continue;
        };
        // Compaction summaries are encoded as user messages, so track both:
        // the last real user message (preferred insertion point) and the last
        // user-message-like item (fallback summary insertion point).
        last_user_or_summary_index.get_or_insert(i);
        if !is_compaction_summary_message(&message, summary_prefix) {
            last_real_user_index = Some(i);
            break;
        }
    }
    let last_compaction_index = compacted_history
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, item)| {
            matches!(
                item,
                ResponseItem::Compaction { .. } | ResponseItem::ContextCompaction { .. }
            )
            .then_some(i)
        });
    let insertion_index = last_real_user_index
        .or(last_user_or_summary_index)
        .or(last_compaction_index);

    if let Some(insertion_index) = insertion_index {
        compacted_history.splice(insertion_index..insertion_index, initial_context);
    } else {
        compacted_history.extend(initial_context);
    }

    compacted_history
}

pub fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
) -> Vec<ResponseItem> {
    build_compacted_history_with_limit(
        initial_context,
        user_messages,
        summary_text,
        COMPACT_USER_MESSAGE_MAX_TOKENS,
    )
}

pub fn build_compacted_history_with_limit(
    mut history: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
    max_tokens: usize,
) -> Vec<ResponseItem> {
    let mut selected_messages: Vec<String> = Vec::new();
    if max_tokens > 0 {
        let mut remaining = max_tokens;
        for message in user_messages.iter().rev() {
            if remaining == 0 {
                break;
            }
            let tokens = approx_token_count(message);
            if tokens <= remaining {
                selected_messages.push(message.clone());
                remaining = remaining.saturating_sub(tokens);
            } else {
                let truncated = truncate_text(message, TruncationPolicy::Tokens(remaining));
                selected_messages.push(truncated);
                break;
            }
        }
        selected_messages.reverse();
    }

    for message in &selected_messages {
        history.push(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: message.clone(),
            }],
            phase: None,
        });
    }

    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };

    history.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: summary_text }],
        phase: None,
    });

    history
}

#[cfg(test)]
#[path = "compact_history_tests.rs"]
mod tests;
