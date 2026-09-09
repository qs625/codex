use model_service_api::ApiError;
use model_service_api::ResponsesWsRequest;
use protocol::error::InvalidModelInputError;
use protocol::error::ModelInputItemReference;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use serde_json::Value;

// Responses accepts model-emitted structured items as output more permissively than it validates
// the same items when they return as the next request's input. Keep this conservative local guard
// bounded to the known recursive object-key failure class; unknown constraints still use the
// provider's structured invalid-input recovery path.
const MAX_STRUCTURED_JSON_PROPERTY_NAME_BYTES: usize = 1_024;
const INVALID_PROPERTY_NAME_CODE: &str = "model_context_invalid_json_property_name";

pub(crate) fn make_responses_input_items_compatible(
    input: &mut Vec<ResponseItem>,
    sources: &mut Vec<Option<ModelInputItemReference>>,
) {
    if sources.len() != input.len() {
        *sources = vec![None; input.len()];
    }
    let prepared = input
        .drain(..)
        .zip(sources.drain(..))
        .filter_map(|(item, source)| {
            responses_compatible_input_item(item).map(|item| (item, source))
        })
        .collect::<Vec<_>>();
    let (compatible_input, compatible_sources) = prepared.into_iter().unzip();
    *input = compatible_input;
    *sources = compatible_sources;
}

pub(crate) fn make_responses_ws_input_items_compatible(request: &mut ResponsesWsRequest) {
    let ResponsesWsRequest::ResponseCreate(payload) = request else {
        return;
    };
    make_responses_input_items_compatible(&mut payload.input, &mut payload.input_sources);
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
        ResponseItem::ModelContextQuarantine {
            target,
            reason,
            error_code,
            error_param,
        } => Some(protocol::models::model_context_quarantine_notice(
            &target,
            &reason,
            error_code.as_deref(),
            error_param.as_deref(),
        )),
        ResponseItem::ContextCompaction { .. } | ResponseItem::Other => None,
        item => Some(item),
    }
}

pub(crate) fn validate_responses_input_items(
    input: &[ResponseItem],
    sources: &[Option<ModelInputItemReference>],
) -> Result<(), ApiError> {
    for (index, item) in input.iter().enumerate() {
        let Some((field, value)) = structured_model_output_value(item) else {
            continue;
        };
        if !contains_oversized_property_name(&value) {
            continue;
        }
        return Err(ApiError::InvalidModelInput(InvalidModelInputError {
            message: format!(
                "Historical structured model output contains a JSON property name longer than \
                 {MAX_STRUCTURED_JSON_PROPERTY_NAME_BYTES} bytes."
            ),
            error_type: Some("invalid_request_error".to_string()),
            code: Some(INVALID_PROPERTY_NAME_CODE.to_string()),
            param: Some(format!("input[{index}].{field}.<oversized_property_name>")),
            input_index: Some(index),
            source: sources.get(index).cloned().flatten(),
        }));
    }
    Ok(())
}

fn structured_model_output_value(item: &ResponseItem) -> Option<(&'static str, Value)> {
    match item {
        ResponseItem::FunctionCall { arguments, .. } => serde_json::from_str(arguments)
            .ok()
            .map(|value| ("arguments", value)),
        ResponseItem::ToolSearchCall { arguments, .. } => Some(("arguments", arguments.clone())),
        ResponseItem::CustomToolCall { input, .. } => serde_json::from_str(input)
            .ok()
            .map(|value| ("input", value)),
        ResponseItem::LocalShellCall { action, .. } => serde_json::to_value(action)
            .ok()
            .map(|value| ("action", value)),
        _ => None,
    }
}

fn contains_oversized_property_name(value: &Value) -> bool {
    fn visit(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                for (name, value) in object {
                    if name.len() > MAX_STRUCTURED_JSON_PROPERTY_NAME_BYTES {
                        return true;
                    }
                    if visit(value) {
                        return true;
                    }
                }
                false
            }
            Value::Array(values) => values.iter().any(visit),
            _ => false,
        }
    }

    visit(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::error::ModelInputItemKind;
    use protocol::models::FunctionCallOutputPayload;

    fn target() -> ModelInputItemReference {
        ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "call-1".to_string(),
        }
    }

    #[test]
    fn compatibility_filter_keeps_sources_aligned() {
        let mut input = vec![
            ResponseItem::ContextCompaction {
                encrypted_content: None,
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "lookup".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload::from_text("done".to_string()),
            },
        ];
        let mut sources = vec![None, Some(target()), Some(target())];

        make_responses_input_items_compatible(&mut input, &mut sources);

        assert_eq!(input.len(), 2);
        assert_eq!(sources, vec![Some(target()), Some(target())]);
    }

    #[test]
    fn preflight_rejects_oversized_recursive_property_name_with_exact_source() {
        let oversized_name = "x".repeat(1_872);
        let input = vec![ResponseItem::FunctionCall {
            id: None,
            name: "lookup".to_string(),
            namespace: None,
            arguments: serde_json::json!({
                "outer": {
                    (oversized_name): true
                }
            })
            .to_string(),
            call_id: "call-1".to_string(),
        }];
        let sources = vec![Some(target())];

        let error = validate_responses_input_items(&input, &sources)
            .expect_err("oversized property should be rejected before transport");
        let ApiError::InvalidModelInput(details) = error else {
            panic!("expected structured invalid model input");
        };
        assert_eq!(details.input_index, Some(0));
        assert_eq!(details.source, Some(target()));
        assert_eq!(
            details.code.as_deref(),
            Some("model_context_invalid_json_property_name")
        );
        assert_eq!(
            details.param.as_deref(),
            Some("input[0].arguments.<oversized_property_name>")
        );
    }

    #[test]
    fn preflight_does_not_parse_json_looking_assistant_text() {
        let oversized_name = "x".repeat(1_872);
        let input = vec![ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: serde_json::json!({(oversized_name): true}).to_string(),
            }],
            phase: None,
        }];

        assert!(validate_responses_input_items(&input, &[None]).is_ok());
    }
}
