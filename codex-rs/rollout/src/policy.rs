use crate::protocol::EventMsg;
use crate::protocol::RolloutItem;
use codex_protocol::models::ResponseItem;
use codex_utils_string::truncate_middle_chars;

const PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES: usize = 10_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EventPersistenceMode {
    #[default]
    Limited,
    Extended,
}

/// Whether a rollout `item` should be persisted in rollout files for the
/// provided persistence `mode`.
pub fn is_persisted_rollout_item(item: &RolloutItem, mode: EventPersistenceMode) -> bool {
    match item {
        RolloutItem::ResponseItem(item) => should_persist_response_item(item),
        RolloutItem::EventMsg(ev) => should_persist_event_msg(ev, mode),
        // Persist Codex executive markers so we can analyze flows (e.g., compaction, API turns).
        RolloutItem::Compacted(_) | RolloutItem::TurnContext(_) | RolloutItem::SessionMeta(_) => {
            true
        }
    }
}

/// Return the canonical rollout items that should be persisted for a live append.
pub fn persisted_rollout_items(
    items: &[RolloutItem],
    mode: EventPersistenceMode,
) -> Vec<RolloutItem> {
    let mut persisted = Vec::new();
    for item in items {
        if is_persisted_rollout_item(item, mode) {
            persisted.push(sanitize_rollout_item_for_persistence(item.clone(), mode));
        }
    }
    persisted
}

fn sanitize_rollout_item_for_persistence(
    item: RolloutItem,
    mode: EventPersistenceMode,
) -> RolloutItem {
    if mode != EventPersistenceMode::Extended {
        return item;
    }

    match item {
        RolloutItem::EventMsg(EventMsg::ExecCommandEnd(mut event)) => {
            event.aggregated_output = truncate_middle_chars(
                &event.aggregated_output,
                PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES,
            );
            event.stdout.clear();
            event.stderr.clear();
            event.formatted_output.clear();
            RolloutItem::EventMsg(EventMsg::ExecCommandEnd(event))
        }
        _ => item,
    }
}

/// Whether a `ResponseItem` should be persisted in rollout files.
#[inline]
pub fn should_persist_response_item(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::Other => false,
    }
}

/// Whether a `ResponseItem` should be persisted for the memories.
#[inline]
pub fn should_persist_response_item_for_memories(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, .. } => role != "developer",
        ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. } => true,
        ResponseItem::Reasoning { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => false,
    }
}

/// Whether an `EventMsg` should be persisted in rollout files for the
/// provided persistence `mode`.
#[inline]
pub fn should_persist_event_msg(ev: &EventMsg, mode: EventPersistenceMode) -> bool {
    match mode {
        EventPersistenceMode::Limited => should_persist_event_msg_limited(ev),
        EventPersistenceMode::Extended => should_persist_event_msg_extended(ev),
    }
}

fn should_persist_event_msg_limited(ev: &EventMsg) -> bool {
    matches!(
        event_msg_persistence_mode(ev),
        Some(EventPersistenceMode::Limited)
    )
}

fn should_persist_event_msg_extended(ev: &EventMsg) -> bool {
    matches!(
        event_msg_persistence_mode(ev),
        Some(EventPersistenceMode::Limited) | Some(EventPersistenceMode::Extended)
    )
}

/// Returns the minimum persistence mode that includes this event.
/// `None` means the event should never be persisted.
fn event_msg_persistence_mode(ev: &EventMsg) -> Option<EventPersistenceMode> {
    match ev {
        EventMsg::UserMessage(_)
        | EventMsg::AgentMessage(_)
        | EventMsg::AgentReasoning(_)
        | EventMsg::AgentReasoningRawContent(_)
        | EventMsg::PatchApplyEnd(_)
        | EventMsg::TokenCount(_)
        | EventMsg::ThreadContextUsageUpdated(_)
        | EventMsg::ThreadGoalUpdated(_)
        | EventMsg::ThreadSkillsUpdated(_)
        | EventMsg::ContextCompacted(_)
        | EventMsg::EnteredReviewMode(_)
        | EventMsg::ExitedReviewMode(_)
        | EventMsg::McpToolCallEnd(_)
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabWaitingBegin(_)
        | EventMsg::CollabWaitingEnd(_)
        | EventMsg::CollabCloseBegin(_)
        | EventMsg::CollabCloseEnd(_)
        | EventMsg::CollabResumeBegin(_)
        | EventMsg::CollabResumeEnd(_)
        | EventMsg::ThreadRolledBack(_)
        | EventMsg::TurnAborted(_)
        | EventMsg::TurnStarted(_)
        | EventMsg::TurnComplete(_)
        | EventMsg::WebSearchEnd(_)
        | EventMsg::ImageGenerationEnd(_) => Some(EventPersistenceMode::Limited),
        EventMsg::ItemCompleted(event) => {
            // Plan items are derived from streaming tags and are not part of the
            // raw ResponseItem history, so we persist their completion to replay
            // them on resume without bloating rollouts with every item lifecycle.
            if matches!(event.item, codex_protocol::items::TurnItem::Plan(_)) {
                Some(EventPersistenceMode::Limited)
            } else {
                None
            }
        }
        EventMsg::Error(_)
        | EventMsg::GuardianAssessment(_)
        | EventMsg::ExecCommandEnd(_)
        | EventMsg::ViewImageToolCall(_)
        | EventMsg::DynamicToolCallRequest(_)
        | EventMsg::DynamicToolCallResponse(_) => Some(EventPersistenceMode::Extended),
        EventMsg::Warning(_)
        | EventMsg::GuardianWarning(_)
        | EventMsg::RealtimeConversationStarted(_)
        | EventMsg::RealtimeConversationSdp(_)
        | EventMsg::RealtimeConversationRealtime(_)
        | EventMsg::RealtimeConversationClosed(_)
        | EventMsg::ModelReroute(_)
        | EventMsg::ModelVerification(_)
        | EventMsg::AgentReasoningSectionBreak(_)
        | EventMsg::RawResponseItem(_)
        | EventMsg::SessionConfigured(_)
        | EventMsg::McpToolCallBegin(_)
        | EventMsg::ExecCommandBegin(_)
        | EventMsg::TerminalInteraction(_)
        | EventMsg::ExecCommandOutputDelta(_)
        | EventMsg::ExecApprovalRequest(_)
        | EventMsg::RequestPermissions(_)
        | EventMsg::RequestUserInput(_)
        | EventMsg::ElicitationRequest(_)
        | EventMsg::ApplyPatchApprovalRequest(_)
        | EventMsg::StreamError(_)
        | EventMsg::PatchApplyBegin(_)
        | EventMsg::PatchApplyUpdated(_)
        | EventMsg::TurnDiff(_)
        | EventMsg::RealtimeConversationListVoicesResponse(_)
        | EventMsg::McpStartupUpdate(_)
        | EventMsg::McpStartupComplete(_)
        | EventMsg::WebSearchBegin(_)
        | EventMsg::PlanUpdate(_)
        | EventMsg::ShutdownComplete
        | EventMsg::DeprecationNotice(_)
        | EventMsg::ItemStarted(_)
        | EventMsg::HookStarted(_)
        | EventMsg::HookCompleted(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::PlanDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::ImageGenerationBegin(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::EventPersistenceMode;
    use super::should_persist_event_msg;
    use codex_protocol::ThreadId;
    use codex_protocol::openai_models::ReasoningEffort;
    use codex_protocol::protocol::AgentStatus;
    use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
    use codex_protocol::protocol::CollabAgentInteractionEndEvent;
    use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
    use codex_protocol::protocol::CollabAgentSpawnEndEvent;
    use codex_protocol::protocol::CollabCloseBeginEvent;
    use codex_protocol::protocol::CollabCloseEndEvent;
    use codex_protocol::protocol::CollabResumeBeginEvent;
    use codex_protocol::protocol::CollabResumeEndEvent;
    use codex_protocol::protocol::CollabWaitingBeginEvent;
    use codex_protocol::protocol::CollabWaitingEndEvent;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::ThreadContextUsage;
    use codex_protocol::protocol::ThreadContextUsageCategoryBreakdown;
    use codex_protocol::protocol::ThreadContextUsageLoadedSkills;
    use codex_protocol::protocol::ThreadContextUsageUpdatedEvent;

    #[test]
    fn limited_mode_persists_thread_context_usage() {
        let event = EventMsg::ThreadContextUsageUpdated(ThreadContextUsageUpdatedEvent {
            usage: ThreadContextUsage {
                total_bytes: 256,
                budget_used_percent: Some(3),
                categories: ThreadContextUsageCategoryBreakdown {
                    compact: 0,
                    skills_metadata: 0,
                    concrete_skills: 0,
                    tools_metadata: 0,
                    tool_calls: 0,
                    user_messages: 256,
                    llm_messages: 0,
                    reasoning: 0,
                },
                loaded_skills: ThreadContextUsageLoadedSkills {
                    loaded_count: 0,
                    total_count: Some(0),
                    skills: Vec::new(),
                },
            },
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            true
        );
    }

    #[test]
    fn limited_mode_persists_collab_agent_events() {
        let sender_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid sender");
        let receiver_thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000002").expect("valid receiver");
        let sender_agent_path = "/root".to_string();
        let receiver_agent_path = "/root/scout".to_string();

        let events = [
            EventMsg::CollabAgentSpawnBegin(CollabAgentSpawnBeginEvent {
                call_id: "spawn-begin".into(),
                started_at_ms: 1,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                prompt: "inspect".into(),
                model: "gpt-5.4".into(),
                reasoning_effort: ReasoningEffort::Medium,
            }),
            EventMsg::CollabAgentSpawnEnd(CollabAgentSpawnEndEvent {
                call_id: "spawn-end".into(),
                completed_at_ms: 2,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                new_thread_id: Some(receiver_thread_id),
                new_agent_path: Some(receiver_agent_path.clone()),
                new_agent_nickname: Some("Scout".into()),
                new_agent_role: Some("worker".into()),
                prompt: "inspect".into(),
                model: "gpt-5.4".into(),
                reasoning_effort: ReasoningEffort::Medium,
                status: AgentStatus::Running,
            }),
            EventMsg::CollabAgentInteractionBegin(CollabAgentInteractionBeginEvent {
                call_id: "send-begin".into(),
                started_at_ms: 3,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path.clone(),
                prompt: "continue".into(),
            }),
            EventMsg::CollabAgentInteractionEnd(CollabAgentInteractionEndEvent {
                call_id: "send-end".into(),
                completed_at_ms: 4,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path.clone(),
                receiver_agent_nickname: None,
                receiver_agent_role: None,
                prompt: "continue".into(),
                status: AgentStatus::Completed(None),
            }),
            EventMsg::CollabWaitingBegin(CollabWaitingBeginEvent {
                call_id: "wait-begin".into(),
                started_at_ms: 5,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_ids: vec![receiver_thread_id],
                receiver_agents: Vec::new(),
                timeout_ms: 30_000,
            }),
            EventMsg::CollabWaitingEnd(CollabWaitingEndEvent {
                call_id: "wait-end".into(),
                completed_at_ms: 6,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                timeout_ms: 30_000,
                agent_statuses: Vec::new(),
                statuses: [(receiver_thread_id, AgentStatus::Completed(None))]
                    .into_iter()
                    .collect(),
            }),
            EventMsg::CollabCloseBegin(CollabCloseBeginEvent {
                call_id: "close-begin".into(),
                started_at_ms: 7,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path.clone(),
            }),
            EventMsg::CollabCloseEnd(CollabCloseEndEvent {
                call_id: "close-end".into(),
                completed_at_ms: 8,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path.clone(),
                receiver_agent_nickname: None,
                receiver_agent_role: None,
                status: AgentStatus::Completed(None),
            }),
            EventMsg::CollabResumeBegin(CollabResumeBeginEvent {
                call_id: "resume-begin".into(),
                started_at_ms: 9,
                sender_thread_id,
                sender_agent_path: sender_agent_path.clone(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path.clone(),
                receiver_agent_nickname: None,
                receiver_agent_role: None,
            }),
            EventMsg::CollabResumeEnd(CollabResumeEndEvent {
                call_id: "resume-end".into(),
                completed_at_ms: 10,
                sender_thread_id,
                sender_agent_path,
                receiver_thread_id,
                receiver_agent_path,
                receiver_agent_nickname: None,
                receiver_agent_role: None,
                status: AgentStatus::Completed(None),
            }),
        ];

        for event in events {
            assert_eq!(
                should_persist_event_msg(&event, EventPersistenceMode::Limited),
                true,
                "expected {event:?} to persist in limited mode",
            );
        }
    }
}
