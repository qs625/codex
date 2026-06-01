use std::sync::Arc;

use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::Thread;
use codex_app_server_protocol::ThreadContextUsageUpdatedNotification;
use codex_app_server_protocol::ThreadHistoryBuilder;
use codex_app_server_protocol::ThreadTokenUsage;
use codex_app_server_protocol::Turn;
use codex_app_server_protocol::TurnStatus;
use codex_core::CodexThread;
use codex_protocol::ThreadId;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::ThreadContextUsage;
use codex_protocol::protocol::ThreadContextUsageCategoryBreakdown;
use codex_protocol::protocol::ThreadContextUsageLoadedSkills;
use codex_protocol::protocol::TokenUsageInfo;

use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;

pub(super) async fn send_thread_context_usage_update_to_connection(
    outgoing: &Arc<OutgoingMessageSender>,
    connection_id: ConnectionId,
    thread_id: ThreadId,
    thread: &Thread,
    conversation: &CodexThread,
    rollout_items: &[RolloutItem],
) {
    let Some(token_usage) = conversation
        .token_usage_info()
        .await
        .map(ThreadTokenUsage::from)
    else {
        return;
    };
    let context_usage =
        thread_context_usage_from_rollout_or_conversation(conversation, rollout_items).await;
    let notification = ThreadContextUsageUpdatedNotification {
        thread_id: thread_id.to_string(),
        turn_id: latest_context_usage_turn_id_from_rollout_items(
            rollout_items,
            thread.turns.as_slice(),
        )
        .unwrap_or_else(|| latest_context_usage_turn_id(thread)),
        token_usage,
        context_usage: context_usage.into(),
    };
    outgoing
        .send_server_notification_to_connections(
            &[connection_id],
            ServerNotification::ThreadContextUsageUpdated(notification),
        )
        .await;
}

pub(super) async fn thread_context_usage_from_rollout_or_conversation(
    conversation: &CodexThread,
    rollout_items: &[RolloutItem],
) -> ThreadContextUsage {
    if let Some(usage) = latest_nonzero_thread_context_usage_from_rollout_items(rollout_items) {
        return usage;
    }
    let usage = conversation.thread_context_usage().await;
    if usage.total_bytes > 0 {
        usage
    } else {
        legacy_thread_context_usage_from_rollout_items(rollout_items).unwrap_or(usage)
    }
}

struct ContextUsageTurnOwner {
    id: String,
    position: Option<usize>,
}

pub(super) fn latest_context_usage_turn_id_from_rollout_items(
    rollout_items: &[RolloutItem],
    turns: &[Turn],
) -> Option<String> {
    let mut builder = ThreadHistoryBuilder::new();
    let mut turn_owner = None;

    for item in rollout_items {
        if matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(_))
        ) {
            turn_owner = builder
                .active_turn_snapshot()
                .map(|turn| ContextUsageTurnOwner {
                    id: turn.id,
                    position: builder.active_turn_position(),
                });
        }
        builder.handle_rollout_item(item);
    }

    let owner = turn_owner?;
    if turns.iter().any(|turn| turn.id == owner.id) {
        Some(owner.id)
    } else {
        owner
            .position
            .and_then(|position| turns.get(position))
            .map(|turn| turn.id.clone())
    }
}

pub(super) fn latest_thread_context_usage_from_rollout_items(
    rollout_items: &[RolloutItem],
) -> Option<ThreadContextUsage> {
    rollout_items.iter().rev().find_map(|item| match item {
        RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(event)) => {
            Some(event.usage.clone())
        }
        RolloutItem::ResponseItem(_)
        | RolloutItem::Compacted(_)
        | RolloutItem::TurnContext(_)
        | RolloutItem::SessionMeta(_)
        | RolloutItem::EventMsg(_) => None,
    })
}

pub(super) fn latest_nonzero_thread_context_usage_from_rollout_items(
    rollout_items: &[RolloutItem],
) -> Option<ThreadContextUsage> {
    latest_thread_context_usage_from_rollout_items(rollout_items)
        .filter(|usage| usage.total_bytes > 0)
}

pub(super) fn legacy_thread_context_usage_from_rollout_items(
    rollout_items: &[RolloutItem],
) -> Option<ThreadContextUsage> {
    let mut categories = ThreadContextUsageCategoryBreakdown {
        compact: 0,
        skills_metadata: 0,
        concrete_skills: 0,
        tools_metadata: 0,
        tool_calls: 0,
        user_messages: 0,
        llm_messages: 0,
        reasoning: 0,
    };
    let mut token_info = None;

    for item in rollout_items {
        let RolloutItem::EventMsg(event) = item else {
            continue;
        };
        match event {
            EventMsg::UserMessage(event) => {
                categories.user_messages = categories
                    .user_messages
                    .saturating_add(estimated_text_bytes(event.message.as_str()));
                categories.skills_metadata = categories
                    .skills_metadata
                    .saturating_add(i64::try_from(event.skills.len()).unwrap_or(i64::MAX) * 36);
                categories.concrete_skills = categories
                    .concrete_skills
                    .saturating_add(i64::try_from(event.skills.len()).unwrap_or(i64::MAX) * 120);
            }
            EventMsg::AgentMessage(event) => {
                categories.llm_messages = categories
                    .llm_messages
                    .saturating_add(estimated_text_bytes(event.message.as_str()));
            }
            EventMsg::AgentReasoning(event) => {
                categories.reasoning = categories
                    .reasoning
                    .saturating_add(estimated_text_bytes(event.text.as_str()));
            }
            EventMsg::AgentReasoningRawContent(event) => {
                categories.reasoning = categories
                    .reasoning
                    .saturating_add(estimated_text_bytes(event.text.as_str()));
            }
            EventMsg::TokenCount(event) => {
                token_info = event.info.clone();
            }
            _ => {}
        }
    }

    let total_bytes = [
        categories.compact,
        categories.skills_metadata,
        categories.concrete_skills,
        categories.tools_metadata,
        categories.tool_calls,
        categories.user_messages,
        categories.llm_messages,
        categories.reasoning,
    ]
    .into_iter()
    .fold(0i64, i64::saturating_add);
    if total_bytes <= 0 {
        return None;
    }

    Some(ThreadContextUsage {
        total_bytes,
        budget_used_percent: context_budget_percent(token_info.as_ref()),
        categories,
        loaded_skills: ThreadContextUsageLoadedSkills {
            loaded_count: 0,
            total_count: None,
            skills: Vec::new(),
        },
    })
}

fn estimated_text_bytes(text: &str) -> i64 {
    i64::try_from(text.trim().len()).unwrap_or(i64::MAX)
}

fn context_budget_percent(token_info: Option<&TokenUsageInfo>) -> Option<i64> {
    token_info.and_then(|info| {
        info.model_context_window.and_then(|window| {
            if window <= 0 {
                None
            } else {
                Some(
                    info.total_token_usage
                        .total_tokens
                        .saturating_mul(100)
                        .saturating_div(window)
                        .clamp(0, 100),
                )
            }
        })
    })
}

fn latest_context_usage_turn_id(thread: &Thread) -> String {
    thread
        .turns
        .iter()
        .rev()
        .find(|turn| matches!(turn.status, TurnStatus::Completed | TurnStatus::Failed))
        .or_else(|| thread.turns.last())
        .map(|turn| turn.id.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::latest_context_usage_turn_id_from_rollout_items;
    use super::latest_thread_context_usage_from_rollout_items;
    use codex_app_server_protocol::build_turns_from_rollout_items;
    use codex_protocol::protocol::AgentMessageEvent;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::RolloutItem;
    use codex_protocol::protocol::ThreadContextUsage;
    use codex_protocol::protocol::ThreadContextUsageCategoryBreakdown;
    use codex_protocol::protocol::ThreadContextUsageLoadedSkills;
    use codex_protocol::protocol::ThreadContextUsageUpdatedEvent;
    use codex_protocol::protocol::UserMessageEvent;
    use pretty_assertions::assert_eq;

    #[test]
    fn replay_extracts_latest_context_usage() {
        let rollout_items = context_usage_history();

        let usage = latest_thread_context_usage_from_rollout_items(rollout_items.as_slice())
            .expect("usage");

        assert_eq!(usage.total_bytes, 123);
        assert_eq!(usage.budget_used_percent, Some(64));
    }

    #[test]
    fn replay_attribution_uses_loaded_history() {
        let rollout_items = context_usage_history();
        let turns = build_turns_from_rollout_items(rollout_items.as_slice());

        assert_eq!(
            latest_context_usage_turn_id_from_rollout_items(
                rollout_items.as_slice(),
                turns.as_slice()
            ),
            Some(turns[0].id.clone())
        );
    }

    fn context_usage_history() -> Vec<RolloutItem> {
        vec![
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "first turn".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "first answer".to_string(),
                phase: None,
                memory_citation: None,
            })),
            RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(
                ThreadContextUsageUpdatedEvent {
                    usage: ThreadContextUsage {
                        total_bytes: 123,
                        budget_used_percent: Some(64),
                        categories: ThreadContextUsageCategoryBreakdown {
                            compact: 10,
                            skills_metadata: 11,
                            concrete_skills: 12,
                            tools_metadata: 13,
                            tool_calls: 14,
                            user_messages: 15,
                            llm_messages: 16,
                            reasoning: 17,
                        },
                        loaded_skills: ThreadContextUsageLoadedSkills {
                            loaded_count: 0,
                            total_count: Some(0),
                            skills: Vec::new(),
                        },
                    },
                },
            )),
        ]
    }
}
