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

    history.push(user_message(input.compact_marker_text));

    for snapshot in input.memory_bundle.snapshots {
        history.push(user_message(format!(
            "Memory checkpoint: {}\n{}",
            snapshot.label, snapshot.content
        )));
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
