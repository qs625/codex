use super::*;
use codex_context_manager::build_compacted_history_with_limit;
use codex_context_manager::content_items_to_text;
use codex_utils_output_truncation::TruncationPolicy;
use pretty_assertions::assert_eq;
use protocol::AgentPath;
use protocol::config_types::ModeKind;
use protocol::config_types::ReasoningSummary;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::AskForApproval;
use protocol::protocol::CompactedItem;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::ThreadRolledBackEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::UserMessageEvent;
use std::path::PathBuf;

fn options() -> RolloutReconstructionOptions<'static> {
    RolloutReconstructionOptions {
        truncation_policy: TruncationPolicy::Tokens(1024),
        summary_prefix: Some("SUMMARY PREFIX"),
    }
}

fn user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn assistant_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn inter_agent_response_item(text: &str) -> ResponseItem {
    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::root().join("worker").unwrap(),
        Vec::new(),
        text.to_string(),
        InterAgentOperation::SendMessage,
    );
    ResponseItem::InterAgentCommunication {
        id: None,
        communication,
    }
}

fn turn_context_item(turn_id: &str, model: &str) -> TurnContextItem {
    TurnContextItem {
        turn_id: Some(turn_id.to_string()),
        trace_id: Some("trace".to_string()),
        cwd: PathBuf::from("/tmp"),
        current_date: Some("2026-06-23".to_string()),
        timezone: Some("Asia/Shanghai".to_string()),
        approval_policy: AskForApproval::OnRequest,
        sandbox_policy: SandboxPolicy::DangerFullAccess,
        permission_profile: None,
        network: None,
        file_system_sandbox_policy: None,
        model: model.to_string(),
        personality: None,
        collaboration_mode: None,
        realtime_active: Some(true),
        effort: None,
        summary: ReasoningSummary::Auto,
        user_instructions: None,
        developer_instructions: None,
        final_output_json_schema: None,
        truncation_policy: Some(TruncationPolicy::Tokens(1024)),
    }
}

fn turn_started(turn_id: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: turn_id.to_string(),
        started_at: None,
        model_context_window: Some(128_000),
        collaboration_mode_kind: ModeKind::Default,
    }))
}

fn user_event(text: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
        message: text.to_string(),
        images: None,
        local_images: Vec::new(),
        skills: Vec::new(),
        text_elements: Vec::new(),
    }))
}

fn turn_complete(turn_id: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
        turn_id: turn_id.to_string(),
        last_agent_message: None,
        completed_at: None,
        duration_ms: None,
        time_to_first_token_ms: None,
    }))
}

fn rollback(num_turns: u32) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::ThreadRolledBack(ThreadRolledBackEvent {
        num_turns,
    }))
}

#[test]
fn rollback_keeps_history_and_metadata_in_sync_for_completed_turns() {
    let first_context_item = turn_context_item("turn-1", "model-1");
    let rolled_back_context_item = turn_context_item("turn-2", "rolled-back-model");
    let turn_one_user = user_message("turn 1 user");
    let turn_one_assistant = assistant_message("turn 1 assistant");
    let turn_two_user = user_message("turn 2 user");
    let turn_two_assistant = assistant_message("turn 2 assistant");

    let rollout_items = vec![
        turn_started("turn-1"),
        user_event("turn 1 user"),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(turn_one_user.clone()),
        RolloutItem::ResponseItem(turn_one_assistant.clone()),
        turn_complete("turn-1"),
        turn_started("turn-2"),
        user_event("turn 2 user"),
        RolloutItem::TurnContext(rolled_back_context_item),
        RolloutItem::ResponseItem(turn_two_user),
        RolloutItem::ResponseItem(turn_two_assistant),
        turn_complete("turn-2"),
        rollback(1),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(
        reconstructed.history,
        vec![turn_one_user, turn_one_assistant]
    );
    assert_eq!(
        reconstructed.previous_turn_settings,
        Some(PreviousTurnSettings {
            model: "model-1".to_string(),
            realtime_active: Some(true),
        })
    );
    assert_eq!(
        serde_json::to_value(reconstructed.reference_context_item).unwrap(),
        serde_json::to_value(Some(first_context_item)).unwrap()
    );
}

#[test]
fn rollback_skips_non_user_turns_for_history_and_metadata() {
    let first_context_item = turn_context_item("turn-1", "model-1");
    let turn_one_user = user_message("turn 1 user");
    let turn_one_assistant = assistant_message("turn 1 assistant");

    let rollout_items = vec![
        turn_started("turn-1"),
        user_event("turn 1 user"),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(turn_one_user.clone()),
        RolloutItem::ResponseItem(turn_one_assistant.clone()),
        turn_complete("turn-1"),
        turn_started("turn-2"),
        user_event("turn 2 user"),
        RolloutItem::ResponseItem(user_message("turn 2 user")),
        RolloutItem::ResponseItem(assistant_message("turn 2 assistant")),
        turn_complete("turn-2"),
        turn_started("standalone-turn"),
        RolloutItem::ResponseItem(assistant_message("standalone assistant")),
        turn_complete("standalone-turn"),
        rollback(1),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(
        reconstructed.history,
        vec![turn_one_user, turn_one_assistant]
    );
    assert_eq!(
        reconstructed.previous_turn_settings,
        Some(PreviousTurnSettings {
            model: "model-1".to_string(),
            realtime_active: Some(true),
        })
    );
    assert_eq!(
        serde_json::to_value(reconstructed.reference_context_item).unwrap(),
        serde_json::to_value(Some(first_context_item)).unwrap()
    );
}

#[test]
fn rollback_counts_inter_agent_response_item_turns() {
    let first_context_item = turn_context_item("turn-1", "model-1");
    let assistant_turn_context = turn_context_item("assistant-turn", "model-1");

    let rollout_items = vec![
        turn_started("turn-1"),
        user_event("turn 1 user"),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 1 user")),
        RolloutItem::ResponseItem(assistant_message("turn 1 assistant")),
        turn_complete("turn-1"),
        turn_started("assistant-turn"),
        RolloutItem::TurnContext(assistant_turn_context),
        RolloutItem::ResponseItem(inter_agent_response_item("continue")),
        RolloutItem::ResponseItem(assistant_message("worker reply")),
        turn_complete("assistant-turn"),
        rollback(1),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(
        reconstructed.history,
        vec![
            user_message("turn 1 user"),
            assistant_message("turn 1 assistant"),
        ]
    );
    assert_eq!(
        reconstructed.previous_turn_settings,
        Some(PreviousTurnSettings {
            model: "model-1".to_string(),
            realtime_active: Some(true),
        })
    );
    assert_eq!(
        serde_json::to_value(reconstructed.reference_context_item).unwrap(),
        serde_json::to_value(Some(first_context_item)).unwrap()
    );
}

#[test]
fn rollback_clears_history_and_metadata_when_exceeding_user_turns() {
    let only_context_item = turn_context_item("only-turn", "model-1");
    let rollout_items = vec![
        turn_started("only-turn"),
        user_event("only user"),
        RolloutItem::TurnContext(only_context_item),
        RolloutItem::ResponseItem(user_message("only user")),
        RolloutItem::ResponseItem(assistant_message("only assistant")),
        turn_complete("only-turn"),
        rollback(99),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(reconstructed.history, Vec::new());
    assert_eq!(reconstructed.previous_turn_settings, None);
    assert!(reconstructed.reference_context_item.is_none());
}

#[test]
fn replacement_history_checkpoint_seeds_replay_suffix() {
    let first_context_item = turn_context_item("turn-1", "model-1");
    let replacement_user = user_message("replacement user");
    let suffix_assistant = assistant_message("suffix assistant");
    let rollout_items = vec![
        turn_started("turn-1"),
        user_event("turn 1 user"),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::Compacted(CompactedItem {
            message: "summary".to_string(),
            replacement_history: Some(vec![replacement_user.clone()]),
        }),
        RolloutItem::ResponseItem(suffix_assistant.clone()),
        turn_complete("turn-1"),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(
        reconstructed.history,
        vec![replacement_user, suffix_assistant]
    );
    assert_eq!(
        reconstructed.previous_turn_settings,
        Some(PreviousTurnSettings {
            model: "model-1".to_string(),
            realtime_active: Some(true),
        })
    );
    assert!(reconstructed.reference_context_item.is_none());
}

#[test]
fn legacy_compaction_without_replacement_history_filters_context_and_warning_messages() {
    let rollout_items = vec![
        RolloutItem::ResponseItem(user_message("real user message")),
        RolloutItem::ResponseItem(user_message(
            "# AGENTS.md instructions for project\n\n<INSTRUCTIONS>\ndo things\n</INSTRUCTIONS>",
        )),
        RolloutItem::ResponseItem(user_message(
            "<ENVIRONMENT_CONTEXT>cwd=/tmp</ENVIRONMENT_CONTEXT>",
        )),
        RolloutItem::ResponseItem(user_message(
            "Warning: The maximum number of unified exec processes you can keep open is 60 and you currently have 61 processes open. Reuse older processes or close them to prevent automatic pruning of old processes",
        )),
        RolloutItem::ResponseItem(user_message(
            "Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.",
        )),
        RolloutItem::ResponseItem(user_message(
            "Warning: Your account was flagged for potentially high-risk cyber activity and this request was routed to gpt-5.2 as a fallback.",
        )),
        RolloutItem::ResponseItem(user_message("SUMMARY PREFIX\nprior compact summary")),
        RolloutItem::Compacted(CompactedItem {
            message: "legacy summary".to_string(),
            replacement_history: None,
        }),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert_eq!(
        reconstructed.history,
        vec![
            user_message("real user message"),
            user_message("legacy summary")
        ]
    );
    assert!(reconstructed.reference_context_item.is_none());
}

#[test]
fn legacy_compaction_without_replacement_history_clears_later_reference_context_item() {
    let current_context_item = turn_context_item("current-turn", "model-1");
    let rollout_items = vec![
        RolloutItem::ResponseItem(user_message("before compact")),
        RolloutItem::Compacted(CompactedItem {
            message: "legacy summary".to_string(),
            replacement_history: None,
        }),
        turn_started("current-turn"),
        user_event("after legacy compact"),
        RolloutItem::TurnContext(current_context_item),
        turn_complete("current-turn"),
    ];

    let reconstructed = reconstruct_history_from_rollout(&rollout_items, options());

    assert!(reconstructed.reference_context_item.is_none());
}

#[test]
fn build_token_limited_compacted_history_truncates_overlong_user_messages() {
    let max_tokens = 16;
    let big = "word ".repeat(200);
    let history = build_compacted_history_with_limit(
        Vec::new(),
        std::slice::from_ref(&big),
        "SUMMARY",
        max_tokens,
    );
    assert_eq!(history.len(), 2);

    let truncated_text = match &history[0] {
        ResponseItem::Message { role, content, .. } if role == "user" => {
            content_items_to_text(content).unwrap_or_default()
        }
        other => panic!("unexpected item in history: {other:?}"),
    };

    assert!(truncated_text.contains("tokens truncated"));
    assert!(!truncated_text.contains(&big));

    let summary_text = match &history[1] {
        ResponseItem::Message { role, content, .. } if role == "user" => {
            content_items_to_text(content).unwrap_or_default()
        }
        other => panic!("unexpected item in history: {other:?}"),
    };
    assert_eq!(summary_text, "SUMMARY");
}
