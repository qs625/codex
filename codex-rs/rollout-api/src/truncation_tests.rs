use super::*;
use codex_protocol::AgentPath;
use codex_protocol::items::HookPromptFragment;
use codex_protocol::items::build_hook_prompt_message;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::ThreadRolledBackEvent;
use pretty_assertions::assert_eq;

fn user_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn contextual_user_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn assistant_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn inter_agent_msg(text: &str, trigger_turn: bool) -> ResponseItem {
    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("agent path"),
        Vec::new(),
        text.to_string(),
        codex_protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(trigger_turn);
    ResponseItem::InterAgentCommunication {
        id: None,
        communication,
    }
}

#[test]
fn truncates_rollout_from_start_before_nth_user_only() {
    let items = [
        user_msg("u1"),
        assistant_msg("a1"),
        assistant_msg("a2"),
        user_msg("u2"),
        assistant_msg("a3"),
        ResponseItem::Reasoning {
            id: "r1".to_string(),
            summary: vec![ReasoningItemReasoningSummary::SummaryText {
                text: "s".to_string(),
            }],
            content: None,
            encrypted_content: None,
        },
        ResponseItem::FunctionCall {
            id: None,
            call_id: "c1".to_string(),
            name: "tool".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
        },
        assistant_msg("a4"),
    ];

    let rollout: Vec<RolloutItem> = items
        .iter()
        .cloned()
        .map(RolloutItem::ResponseItem)
        .collect();

    let truncated =
        truncate_rollout_before_nth_user_message_from_start(&rollout, /*n_from_start*/ 1);
    let expected = vec![
        RolloutItem::ResponseItem(items[0].clone()),
        RolloutItem::ResponseItem(items[1].clone()),
        RolloutItem::ResponseItem(items[2].clone()),
    ];
    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );

    let truncated2 =
        truncate_rollout_before_nth_user_message_from_start(&rollout, /*n_from_start*/ 2);
    assert_eq!(
        serde_json::to_value(&truncated2).unwrap(),
        serde_json::to_value(&rollout).unwrap()
    );
}

#[test]
fn truncation_max_keeps_full_rollout() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(user_msg("u2")),
    ];

    let truncated = truncate_rollout_before_nth_user_message_from_start(&rollout, usize::MAX);

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&rollout).unwrap()
    );
}

#[test]
fn truncates_rollout_from_start_applies_thread_rollback_markers() {
    let rollout_items = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(user_msg("u2")),
        RolloutItem::ResponseItem(assistant_msg("a2")),
        RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 1,
        })),
        RolloutItem::ResponseItem(user_msg("u3")),
        RolloutItem::ResponseItem(assistant_msg("a3")),
        RolloutItem::ResponseItem(user_msg("u4")),
        RolloutItem::ResponseItem(assistant_msg("a4")),
    ];

    let truncated = truncate_rollout_before_nth_user_message_from_start(
        &rollout_items,
        /*n_from_start*/ 2,
    );
    let expected = rollout_items[..7].to_vec();
    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

#[test]
fn ignores_contextual_user_messages_when_truncating_rollout_from_start() {
    let rollout_items = vec![
        RolloutItem::ResponseItem(contextual_user_msg(
            "<goal_context>\nKeep going.\n</goal_context>",
        )),
        RolloutItem::ResponseItem(
            build_hook_prompt_message(&[HookPromptFragment::from_single_hook(
                "Retry",
                "hook-run-1",
            )])
            .expect("hook prompt"),
        ),
        RolloutItem::ResponseItem(user_msg("feature request")),
        RolloutItem::ResponseItem(assistant_msg("ack")),
        RolloutItem::ResponseItem(user_msg("second question")),
        RolloutItem::ResponseItem(assistant_msg("answer")),
    ];

    let truncated = truncate_rollout_before_nth_user_message_from_start(
        &rollout_items,
        /*n_from_start*/ 1,
    );
    let expected = rollout_items[..4].to_vec();

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_counts_trigger_turn_messages() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "queued message",
            /*trigger_turn*/ false,
        )),
        RolloutItem::ResponseItem(assistant_msg("a2")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a3")),
        RolloutItem::ResponseItem(user_msg("u2")),
        RolloutItem::ResponseItem(assistant_msg("a4")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);
    let expected = rollout[4..].to_vec();

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_applies_thread_rollback_markers() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a2")),
        RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 1,
        })),
        RolloutItem::ResponseItem(user_msg("u2")),
        RolloutItem::ResponseItem(assistant_msg("a3")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&rollout).unwrap()
    );
}

#[test]
fn fork_turn_positions_ignore_zero_turn_rollback_markers() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task",
            /*trigger_turn*/ true,
        )),
        RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 0,
        })),
        RolloutItem::ResponseItem(user_msg("u2")),
    ];

    assert_eq!(fork_turn_positions_in_rollout(&rollout), vec![0, 1, 3]);
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_discards_trigger_boundaries_in_rolled_back_suffix() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(user_msg("u2")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 1,
        })),
        RolloutItem::ResponseItem(user_msg("u3")),
        RolloutItem::ResponseItem(assistant_msg("a2")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 2);

    let expected = rollout[1..].to_vec();

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_discards_rolled_back_assistant_instruction_turns() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task 1",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a2")),
        RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
            num_turns: 1,
        })),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task 2",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a3")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 1);
    let expected = rollout[5..].to_vec();

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

#[test]
fn truncates_rollout_to_last_n_fork_turns_keeps_full_rollout_when_n_is_large() {
    let rollout = vec![
        RolloutItem::ResponseItem(user_msg("u1")),
        RolloutItem::ResponseItem(assistant_msg("a1")),
        RolloutItem::ResponseItem(inter_agent_msg(
            "triggered task",
            /*trigger_turn*/ true,
        )),
        RolloutItem::ResponseItem(assistant_msg("a2")),
    ];

    let truncated = truncate_rollout_to_last_n_fork_turns(&rollout, /*n_from_end*/ 10);

    assert_eq!(
        serde_json::to_value(&truncated).unwrap(),
        serde_json::to_value(&rollout).unwrap()
    );
}
