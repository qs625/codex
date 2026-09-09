use super::*;
use protocol::AgentPath;
use protocol::models::ContentItem;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::MessagePhase;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;

#[test]
fn formats_inter_agent_context_without_raw_json_envelope() {
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("author path"),
        AgentPath::root(),
        Vec::new(),
        "implementation is complete".to_string(),
        InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);
    let prompt = Prompt {
        input: vec![ResponseItem::InterAgentCommunication {
            id: Some("collab-1".to_string()),
            communication,
        }],
        ..Prompt::default()
    };

    let formatted = prompt.get_formatted_input();
    let [
        ResponseItem::Message {
            role,
            content,
            phase,
            ..
        },
    ] = formatted.as_slice()
    else {
        panic!("inter-agent item should become one provider message");
    };
    assert_eq!(role, "assistant");
    assert_eq!(*phase, Some(MessagePhase::Commentary));
    let [ContentItem::OutputText { text }] = content.as_slice() else {
        panic!("inter-agent provider message should be text only");
    };
    assert!(text.contains("Author: /root/worker"));
    assert!(text.contains("Recipient: /"));
    assert!(text.contains("Operation: send_message"));
    assert!(text.contains("Content:\nimplementation is complete"));
    assert!(!text.trim_start().starts_with('{'));
    assert!(!text.contains("\"author\""));
    assert!(!text.contains("\"operation\""));
}

#[test]
fn formatted_responses_input_maps_call_and_output_to_one_transaction() {
    let prompt = Prompt {
        input: vec![
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
        ],
        ..Prompt::default()
    };

    let prepared = prompt.get_formatted_responses_input();

    assert_eq!(prepared.items.len(), 2);
    assert_eq!(prepared.sources.len(), 2);
    assert_eq!(prepared.sources[0], prepared.sources[1]);
    assert_eq!(
        prepared.sources[0],
        Some(ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "call-1".to_string(),
        })
    );
}

#[test]
fn duplicate_call_ids_are_not_mapped_to_a_quarantine_target() {
    let prompt = Prompt {
        input: vec![
            ResponseItem::FunctionCall {
                id: None,
                name: "first".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "duplicate".to_string(),
            },
            ResponseItem::CustomToolCall {
                id: None,
                status: None,
                call_id: "duplicate".to_string(),
                name: "second".to_string(),
                input: "{}".to_string(),
            },
        ],
        ..Prompt::default()
    };

    let prepared = prompt.get_formatted_responses_input();

    assert_eq!(prepared.sources, vec![None, None]);
}

#[test]
fn quarantine_notice_is_bounded_and_does_not_include_raw_arguments() {
    let toxic = "x".repeat(2_000);
    let prompt = Prompt {
        input: vec![ResponseItem::ModelContextQuarantine {
            target: protocol::error::ModelContextQuarantineReference::ToolTransaction {
                kind: ModelInputItemKind::ToolSearchCall,
                call_id: "search-1".to_string(),
            },
            reason: toxic.clone(),
            error_code: Some("invalid_value".to_string()),
            error_param: Some("input[12].arguments.deep".to_string()),
        }],
        ..Prompt::default()
    };

    let formatted = prompt.get_formatted_input();
    let [ResponseItem::Message { content, .. }] = formatted.as_slice() else {
        panic!("quarantine should become one model-visible notice");
    };
    let [ContentItem::InputText { text }] = content.as_slice() else {
        panic!("quarantine notice should be plain input text");
    };
    assert!(text.contains("omitted one untrusted historical tool interaction"));
    assert!(!text.contains(&toxic));
    assert!(text.len() < 1_000);
}
