use super::*;

use super::tests::make_session_and_context;
use pretty_assertions::assert_eq;
use protocol::ThreadId;
use protocol::protocol::CompactedItem;
use protocol::protocol::InitialHistory;
use protocol::protocol::ResumedHistory;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::UserMessageEvent;
use std::path::PathBuf;

fn turn_context_item(
    turn_context: &TurnContext,
    turn_id: Option<String>,
    model: &str,
) -> TurnContextItem {
    let mut item = turn_context.to_turn_context_item();
    item.turn_id = turn_id;
    item.model = model.to_string();
    item
}

fn turn_started(turn_id: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: turn_id.to_string(),
        started_at: None,
        model_context_window: Some(128_000),
        collaboration_mode_kind: ModeKind::Default,
    }))
}

fn user_event(message: &str) -> RolloutItem {
    RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
        message: message.to_string(),
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

fn resumed_history(history: Vec<RolloutItem>) -> InitialHistory {
    InitialHistory::Resumed(ResumedHistory {
        conversation_id: ThreadId::default(),
        history,
        rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
    })
}

#[tokio::test]
async fn record_initial_history_resumed_bare_turn_context_does_not_hydrate_previous_turn_settings()
{
    let (session, turn_context) = make_session_and_context().await;
    let previous_context_item = turn_context_item(
        &turn_context,
        Some(turn_context.sub_id.clone()),
        "previous-rollout-model",
    );

    session
        .record_initial_history(resumed_history(vec![RolloutItem::TurnContext(
            previous_context_item,
        )]))
        .await;

    assert_eq!(session.previous_turn_settings().await, None);
    assert!(session.reference_context_item().await.is_none());
}

#[tokio::test]
async fn record_initial_history_resumed_hydrates_previous_turn_settings_from_lifecycle_turn_with_missing_turn_context_id()
 {
    let (session, turn_context) = make_session_and_context().await;
    let turn_id = turn_context.sub_id.clone();
    let previous_model = "previous-rollout-model";
    let previous_context_item = turn_context_item(&turn_context, None, previous_model);

    let rollout_items = vec![
        turn_started(&turn_id),
        user_event("seed"),
        RolloutItem::TurnContext(previous_context_item),
        turn_complete(&turn_id),
    ];

    session
        .record_initial_history(resumed_history(rollout_items))
        .await;

    assert_eq!(
        session.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: previous_model.to_string(),
            realtime_active: Some(turn_context.realtime_active),
        })
    );
}

#[tokio::test]
async fn record_initial_history_resumed_turn_context_after_compaction_reestablishes_reference_context_item()
 {
    let (session, turn_context) = make_session_and_context().await;
    let previous_model = "previous-rollout-model";
    let previous_context_item = turn_context_item(
        &turn_context,
        Some(turn_context.sub_id.clone()),
        previous_model,
    );
    let previous_turn_id = previous_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let rollout_items = vec![
        turn_started(&previous_turn_id),
        user_event("seed"),
        RolloutItem::Compacted(CompactedItem {
            message: String::new(),
            replacement_history: Some(Vec::new()),
                visible_replacement_history_len: None,
        }),
        RolloutItem::TurnContext(previous_context_item.clone()),
        turn_complete(&previous_turn_id),
    ];

    session
        .record_initial_history(resumed_history(rollout_items))
        .await;

    assert_eq!(
        session.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: previous_model.to_string(),
            realtime_active: Some(turn_context.realtime_active),
        })
    );
    assert_eq!(
        serde_json::to_value(session.reference_context_item().await)
            .expect("serialize seeded reference context item"),
        serde_json::to_value(Some(previous_context_item))
            .expect("serialize expected reference context item")
    );
}

#[tokio::test]
async fn record_initial_history_resumed_hydrates_materialized_instructions_for_later_initial_context()
 {
    let (session, turn_context) = make_session_and_context().await;
    let mut previous_context_item = turn_context_item(
        &turn_context,
        Some(turn_context.sub_id.clone()),
        "previous-rollout-model",
    );
    previous_context_item.user_instructions =
        Some("# AGENTS.md instructions for /workspace\n\nPersisted agent instructions".to_string());
    previous_context_item.developer_instructions =
        Some("Persisted developer instructions".to_string());
    let previous_turn_id = previous_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let rollout_items = vec![
        turn_started(&previous_turn_id),
        user_event("seed"),
        RolloutItem::Compacted(CompactedItem {
            message: String::new(),
            replacement_history: Some(Vec::new()),
                visible_replacement_history_len: None,
        }),
        RolloutItem::TurnContext(previous_context_item),
        turn_complete(&previous_turn_id),
    ];

    session
        .record_initial_history(resumed_history(rollout_items))
        .await;

    let resumed_turn = session.new_default_turn().await;
    let initial_context = session.build_initial_context(resumed_turn.as_ref()).await;
    let serialized_context =
        serde_json::to_string(&initial_context).expect("serialize initial context");
    assert!(
        serialized_context.contains("Persisted agent instructions"),
        "expected resumed initial context to include persisted AGENTS.md instructions: {serialized_context}"
    );
    assert!(
        serialized_context.contains("Persisted developer instructions"),
        "expected resumed initial context to include persisted developer instructions: {serialized_context}"
    );
}

#[tokio::test]
async fn record_initial_history_resumed_trailing_incomplete_turn_compaction_clears_reference_context_item()
 {
    let (session, turn_context) = make_session_and_context().await;
    let previous_model = "previous-rollout-model";
    let previous_context_item = turn_context_item(
        &turn_context,
        Some(turn_context.sub_id.clone()),
        previous_model,
    );
    let previous_turn_id = previous_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let incomplete_turn_id = "trailing-incomplete-turn";

    let rollout_items = vec![
        turn_started(&previous_turn_id),
        user_event("seed"),
        RolloutItem::TurnContext(previous_context_item),
        turn_complete(&previous_turn_id),
        turn_started(incomplete_turn_id),
        user_event("incomplete"),
        RolloutItem::Compacted(CompactedItem {
            message: String::new(),
            replacement_history: Some(Vec::new()),
                visible_replacement_history_len: None,
        }),
    ];

    session
        .record_initial_history(resumed_history(rollout_items))
        .await;

    assert_eq!(
        session.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: previous_model.to_string(),
            realtime_active: Some(turn_context.realtime_active),
        })
    );
    assert!(session.reference_context_item().await.is_none());
}
