use compact_service_api::ReplacementHistoryInput;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;

const MAX_RECENT_USER_MESSAGES: usize = 2;

pub(super) fn build_replacement_history(input: ReplacementHistoryInput) -> Vec<ResponseItem> {
    let mut history = input.initial_context;

    for message in input
        .recent_real_user_messages
        .iter()
        .rev()
        .take(MAX_RECENT_USER_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        history.push(user_message(message.clone()));
    }

    if let Some(final_output) = input
        .final_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        history.push(assistant_message(final_output.to_string()));
    }

    history
}

fn user_message(text: String) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text }],
        phase: None,
    }
}

fn assistant_message(text: String) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText { text }],
        phase: None,
    }
}
