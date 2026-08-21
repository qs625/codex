use super::*;
use protocol::AgentPath;
use protocol::models::ContentItem;
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
