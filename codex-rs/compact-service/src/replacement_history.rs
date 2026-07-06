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

    if let Some(user_preferences) = input.memory_bundle.user_preferences {
        history.push(user_message(format!(
            "Memory checkpoint: user preferences\n{user_preferences}"
        )));
    }
    if let Some(project_understanding) = input.memory_bundle.project_understanding {
        history.push(user_message(format!(
            "Memory checkpoint: project understanding\n{project_understanding}"
        )));
    }
    if let Some(current_work) = input.memory_bundle.current_work {
        history.push(user_message(format!(
            "Memory checkpoint: current work\n{current_work}"
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
