use model_service_api::ResponsesWsRequest;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use serde_json::Value;

pub(crate) fn make_responses_input_items_compatible(input: &mut Vec<ResponseItem>) {
    *input = input
        .drain(..)
        .filter_map(responses_compatible_input_item)
        .collect();
}

pub(crate) fn make_responses_ws_input_items_compatible(request: &mut ResponsesWsRequest) {
    let ResponsesWsRequest::ResponseCreate(payload) = request else {
        return;
    };
    make_responses_input_items_compatible(&mut payload.input);
}

fn responses_compatible_input_item(item: ResponseItem) -> Option<ResponseItem> {
    match item {
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::InterAgentCommunication { .. }
        | ResponseItem::ThreadGoalUpdate { .. } => Some(internal_event_message(item)),
        ResponseItem::ContextCompaction { .. } | ResponseItem::Other => None,
        item => Some(item),
    }
}

fn internal_event_message(item: ResponseItem) -> ResponseItem {
    let item_json = serde_json::to_string(&item).unwrap_or_else(|_| format!("{item:?}"));
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!(
                "Codex recorded this internal event in conversation history:\n{item_json}"
            ),
        }],
        phase: None,
    }
}

pub(crate) fn attach_item_ids(payload_json: &mut Value, original_items: &[ResponseItem]) {
    let Some(input_value) = payload_json.get_mut("input") else {
        return;
    };
    let Value::Array(items) = input_value else {
        return;
    };

    for (value, item) in items.iter_mut().zip(original_items.iter()) {
        if let ResponseItem::Reasoning { id, .. }
        | ResponseItem::Message { id: Some(id), .. }
        | ResponseItem::WebSearchCall { id: Some(id), .. }
        | ResponseItem::FunctionCall { id: Some(id), .. }
        | ResponseItem::ToolSearchCall { id: Some(id), .. }
        | ResponseItem::LocalShellCall { id: Some(id), .. }
        | ResponseItem::CustomToolCall { id: Some(id), .. } = item
        {
            if id.is_empty() {
                continue;
            }

            if let Some(obj) = value.as_object_mut() {
                obj.insert("id".to_string(), Value::String(id.clone()));
            }
        }
    }
}
