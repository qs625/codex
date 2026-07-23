use crate::rollout_protocol::EventMsg;
use crate::rollout_protocol::RolloutItem;
use codex_utils_string::truncate_middle_chars;
use protocol::models::ResponseItem;

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
    _mode: EventPersistenceMode,
) -> RolloutItem {
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
        | ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::InterAgentCommunication { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
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
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::InterAgentCommunication { .. } => true,
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::Reasoning { .. }
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

fn should_persist_exec_command_begin(
    event: &protocol::protocol::ExecCommandBeginEvent,
) -> Option<EventPersistenceMode> {
    match event.source {
        protocol::protocol::ExecCommandSource::Agent
        | protocol::protocol::ExecCommandSource::UnifiedExecStartup => {
            Some(EventPersistenceMode::Limited)
        }
        protocol::protocol::ExecCommandSource::UserShell
        | protocol::protocol::ExecCommandSource::UnifiedExecInteraction => None,
    }
}

fn should_persist_exec_command_end(
    event: &protocol::protocol::ExecCommandEndEvent,
) -> Option<EventPersistenceMode> {
    match event.source {
        protocol::protocol::ExecCommandSource::Agent
        | protocol::protocol::ExecCommandSource::UnifiedExecStartup => {
            Some(EventPersistenceMode::Limited)
        }
        protocol::protocol::ExecCommandSource::UserShell
        | protocol::protocol::ExecCommandSource::UnifiedExecInteraction => {
            Some(EventPersistenceMode::Extended)
        }
    }
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
        | EventMsg::CommandWaitStarted(_)
        | EventMsg::CommandWaitCompleted(_)
        | EventMsg::CommandWriteStdinCompleted(_)
        | EventMsg::CommandExecutionNotificationCompleted(_)
        | EventMsg::BuiltinToolCallStarted(_)
        | EventMsg::BuiltinToolCallCompleted(_)
        | EventMsg::ExternalToolCallStarted(_)
        | EventMsg::ExternalToolCallCompleted(_)
        | EventMsg::ExternalTerminalStatus(_)
        | EventMsg::WorkflowRunProgressCompleted(_)
        | EventMsg::EventCommandEventCompleted(_)
        | EventMsg::EventDrivenToolCompleted(_)
        | EventMsg::InterAgentCommunicationCompleted(_)
        | EventMsg::ThreadGoalUpdateCompleted(_)
        | EventMsg::EnteredReviewMode(_)
        | EventMsg::ExitedReviewMode(_)
        | EventMsg::McpToolCallEnd(_)
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabListAgentsBegin(_)
        | EventMsg::CollabListAgentsEnd(_)
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
            // Plan and injected context items are display-capable typed items
            // that are not recoverable from raw ResponseItem replay.
            if matches!(
                event.item,
                protocol::items::TurnItem::Plan(_) | protocol::items::TurnItem::InjectedContext(_)
            ) {
                Some(EventPersistenceMode::Limited)
            } else {
                None
            }
        }
        EventMsg::Error(_)
        | EventMsg::GuardianAssessment(_)
        | EventMsg::ViewImageToolCall(_)
        | EventMsg::DynamicToolCallRequest(_)
        | EventMsg::DynamicToolCallResponse(_) => Some(EventPersistenceMode::Extended),
        EventMsg::ExecCommandEnd(event) => should_persist_exec_command_end(event),
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
        | EventMsg::ResponseItemStarted(_)
        | EventMsg::ResponseItemCompleted(_)
        | EventMsg::HookStarted(_)
        | EventMsg::HookCompleted(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::PlanDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::ImageGenerationBegin(_) => None,
        EventMsg::ExecCommandBegin(event) => should_persist_exec_command_begin(event),
    }
}

#[cfg(test)]
mod tests {
    use codex_utils_string::truncate_middle_chars;
    use pretty_assertions::assert_eq;

    use super::EventPersistenceMode;
    use super::PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES;
    use super::persisted_rollout_items;
    use super::should_persist_event_msg;
    use crate::rollout_protocol::RolloutItem;
    use protocol::ThreadId;
    use protocol::items::InjectedContextItem;
    use protocol::items::InjectedContextSection;
    use protocol::items::TurnItem;
    use protocol::openai_models::ReasoningEffort;
    use protocol::protocol::AgentStatus;
    use protocol::protocol::BuiltinToolCallDisplayEvent;
    use protocol::protocol::BuiltinToolCallStatus;
    use protocol::protocol::CollabAgentInteractionBeginEvent;
    use protocol::protocol::CollabAgentInteractionEndEvent;
    use protocol::protocol::CollabAgentSpawnBeginEvent;
    use protocol::protocol::CollabAgentSpawnEndEvent;
    use protocol::protocol::CollabCloseBeginEvent;
    use protocol::protocol::CollabCloseEndEvent;
    use protocol::protocol::CollabResumeBeginEvent;
    use protocol::protocol::CollabResumeEndEvent;
    use protocol::protocol::CollabWaitingBeginEvent;
    use protocol::protocol::CollabWaitingEndEvent;
    use protocol::protocol::EventMsg;
    use protocol::protocol::ExternalTerminalStatus;
    use protocol::protocol::ExternalTerminalStatusEvent;
    use protocol::protocol::ExternalToolCallDisplayEvent;
    use protocol::protocol::ExternalToolCallStatus;
    use protocol::protocol::ItemCompletedEvent;
    use protocol::protocol::ThreadContextUsage;
    use protocol::protocol::ThreadContextUsageCategoryBreakdown;
    use protocol::protocol::ThreadContextUsageLoadedSkills;
    use protocol::protocol::ThreadContextUsageToolBreakdown;
    use protocol::protocol::ThreadContextUsageUpdatedEvent;
    use protocol::protocol::ThreadLifecycleStatus;

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
                tool_breakdown: ThreadContextUsageToolBreakdown::default(),
            },
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            true
        );
    }

    #[test]
    fn limited_mode_persists_agent_exec_command_begin() {
        let event = EventMsg::ExecCommandBegin(protocol::protocol::ExecCommandBeginEvent {
            call_id: "exec-1".into(),
            started_at_ms: 123,
            process_id: Some("pid-1".into()),
            turn_id: "turn-1".into(),
            command: vec!["echo".into(), "hello".into()],
            cwd: std::path::PathBuf::from("/tmp")
                .try_into()
                .expect("absolute path"),
            parsed_cmd: vec![protocol::parse_command::ParsedCommand::Unknown {
                cmd: "echo hello".into(),
            }],
            source: protocol::protocol::ExecCommandSource::Agent,
            interaction_input: None,
            initial_wait_ms: Some(1_000),
            notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            true
        );
    }

    #[test]
    fn limited_mode_skips_user_shell_exec_command_begin() {
        let event = EventMsg::ExecCommandBegin(protocol::protocol::ExecCommandBeginEvent {
            call_id: "exec-1".into(),
            started_at_ms: 123,
            process_id: Some("pid-1".into()),
            turn_id: "turn-1".into(),
            command: vec!["echo".into(), "hello".into()],
            cwd: std::path::PathBuf::from("/tmp")
                .try_into()
                .expect("absolute path"),
            parsed_cmd: vec![protocol::parse_command::ParsedCommand::Unknown {
                cmd: "echo hello".into(),
            }],
            source: protocol::protocol::ExecCommandSource::UserShell,
            interaction_input: None,
            initial_wait_ms: Some(1_000),
            notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            false
        );
    }

    #[test]
    fn limited_mode_persists_unified_exec_command_end() {
        let event = EventMsg::ExecCommandEnd(protocol::protocol::ExecCommandEndEvent {
            call_id: "exec-1".into(),
            process_id: Some("pid-1".into()),
            turn_id: "turn-1".into(),
            completed_at_ms: 456,
            command: vec!["echo".into(), "hello".into()],
            cwd: std::path::PathBuf::from("/tmp")
                .try_into()
                .expect("absolute path"),
            parsed_cmd: vec![protocol::parse_command::ParsedCommand::Unknown {
                cmd: "echo hello".into(),
            }],
            source: protocol::protocol::ExecCommandSource::UnifiedExecStartup,
            interaction_input: None,
            initial_wait_ms: Some(1_000),
            notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
            stdout: "hello\n".into(),
            stderr: String::new(),
            aggregated_output: "hello\n".into(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(12),
            formatted_output: "hello\n".into(),
            status: protocol::protocol::ExecCommandStatus::Completed,
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            true
        );
    }

    #[test]
    fn limited_mode_sanitizes_unified_exec_command_end_output() {
        let large_output = "x".repeat(PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES + 100);
        let expected_truncated =
            truncate_middle_chars(&large_output, PERSISTED_EXEC_AGGREGATED_OUTPUT_MAX_BYTES);
        let persisted = persisted_rollout_items(
            &[RolloutItem::EventMsg(EventMsg::ExecCommandEnd(
                protocol::protocol::ExecCommandEndEvent {
                    call_id: "exec-1".into(),
                    process_id: Some("pid-1".into()),
                    turn_id: "turn-1".into(),
                    completed_at_ms: 456,
                    command: vec!["echo".into(), "hello".into()],
                    cwd: std::path::PathBuf::from("/tmp")
                        .try_into()
                        .expect("absolute path"),
                    parsed_cmd: vec![protocol::parse_command::ParsedCommand::Unknown {
                        cmd: "echo hello".into(),
                    }],
                    source: protocol::protocol::ExecCommandSource::UnifiedExecStartup,
                    interaction_input: None,
                    initial_wait_ms: Some(1_000),
                    notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
                    stdout: large_output.clone(),
                    stderr: large_output.clone(),
                    aggregated_output: large_output,
                    exit_code: 0,
                    duration: std::time::Duration::from_millis(12),
                    formatted_output: "formatted".repeat(2_000),
                    status: protocol::protocol::ExecCommandStatus::Completed,
                },
            ))],
            EventPersistenceMode::Limited,
        );

        let [RolloutItem::EventMsg(EventMsg::ExecCommandEnd(event))] = persisted.as_slice() else {
            panic!("expected persisted exec command end");
        };
        assert_eq!(event.aggregated_output, expected_truncated);
        assert!(event.stdout.is_empty());
        assert!(event.stderr.is_empty());
        assert!(event.formatted_output.is_empty());
    }

    #[test]
    fn limited_mode_skips_user_shell_exec_command_end() {
        let event = EventMsg::ExecCommandEnd(protocol::protocol::ExecCommandEndEvent {
            call_id: "exec-1".into(),
            process_id: Some("pid-1".into()),
            turn_id: "turn-1".into(),
            completed_at_ms: 456,
            command: vec!["echo".into(), "hello".into()],
            cwd: std::path::PathBuf::from("/tmp")
                .try_into()
                .expect("absolute path"),
            parsed_cmd: vec![protocol::parse_command::ParsedCommand::Unknown {
                cmd: "echo hello".into(),
            }],
            source: protocol::protocol::ExecCommandSource::UserShell,
            interaction_input: None,
            initial_wait_ms: Some(1_000),
            notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
            stdout: "hello\n".into(),
            stderr: String::new(),
            aggregated_output: "hello\n".into(),
            exit_code: 0,
            duration: std::time::Duration::from_millis(12),
            formatted_output: "hello\n".into(),
            status: protocol::protocol::ExecCommandStatus::Completed,
        });

        assert_eq!(
            should_persist_event_msg(&event, EventPersistenceMode::Limited),
            false
        );
    }

    #[test]
    fn limited_mode_persists_injected_context_item_completed() {
        let event = EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: ThreadId::from_string("00000000-0000-0000-0000-000000000001")
                .expect("valid thread"),
            turn_id: "turn-init".to_string(),
            completed_at_ms: 1_000,
            item: TurnItem::InjectedContext(InjectedContextItem {
                id: "ctx-1".to_string(),
                title: "Init Context".to_string(),
                preview: "Developer".to_string(),
                sections: vec![InjectedContextSection {
                    label: "Developer".to_string(),
                    text: "Agent type file body: always inspect the active task.".to_string(),
                }],
            }),
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
                status: ThreadLifecycleStatus::completed(None),
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
                agent_lifecycles: Vec::new(),
                lifecycle_statuses: [(receiver_thread_id, ThreadLifecycleStatus::completed(None))]
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
                status: ThreadLifecycleStatus::completed(None),
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
                status: ThreadLifecycleStatus::completed(None),
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

    #[test]
    fn limited_mode_persists_builtin_tool_call_events() {
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let events = [
            EventMsg::BuiltinToolCallStarted(BuiltinToolCallDisplayEvent {
                thread_id,
                turn_id: "turn-1".into(),
                id: "builtin-1".into(),
                tool: "poll_event".into(),
                arguments: serde_json::json!({}),
                status: BuiltinToolCallStatus::InProgress,
                output: None,
                lifecycle_at_ms: 1,
            }),
            EventMsg::BuiltinToolCallCompleted(BuiltinToolCallDisplayEvent {
                thread_id,
                turn_id: "turn-1".into(),
                id: "builtin-1".into(),
                tool: "poll_event".into(),
                arguments: serde_json::json!({}),
                status: BuiltinToolCallStatus::Completed,
                output: Some(serde_json::json!({
                    "timedOut": false,
                    "sourceHint": "user_input",
                })),
                lifecycle_at_ms: 2,
            }),
        ];

        for event in events {
            assert_eq!(
                should_persist_event_msg(&event, EventPersistenceMode::Limited),
                true
            );
        }
    }

    #[test]
    fn limited_mode_persists_external_tool_call_events() {
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let events = [
            EventMsg::ExternalToolCallStarted(ExternalToolCallDisplayEvent {
                thread_id,
                turn_id: "turn-1".into(),
                id: "external-1".into(),
                tool: "list_external_agents".into(),
                arguments: serde_json::json!({ "path_prefix": "/root" }),
                status: ExternalToolCallStatus::InProgress,
                output: None,
                lifecycle_at_ms: 1,
            }),
            EventMsg::ExternalToolCallCompleted(ExternalToolCallDisplayEvent {
                thread_id,
                turn_id: "turn-1".into(),
                id: "external-1".into(),
                tool: "list_external_agents".into(),
                arguments: serde_json::json!({ "path_prefix": "/root" }),
                status: ExternalToolCallStatus::Completed,
                output: Some(serde_json::json!({ "agents": [] })),
                lifecycle_at_ms: 2,
            }),
        ];

        for event in events {
            assert_eq!(
                should_persist_event_msg(&event, EventPersistenceMode::Limited),
                true
            );
        }
    }

    #[test]
    fn limited_mode_persists_external_terminal_status_without_generic_terminal_events() {
        let thread_id =
            ThreadId::from_string("00000000-0000-0000-0000-000000000001").expect("valid thread");
        let external_terminal = EventMsg::ExternalTerminalStatus(ExternalTerminalStatusEvent {
            thread_id,
            turn_id: "turn-1".into(),
            status: ExternalTerminalStatus::Errored,
            message: Some("provider failed".into()),
            terminal_at_ms: 1,
        });

        assert_eq!(
            should_persist_event_msg(&external_terminal, EventPersistenceMode::Limited),
            true
        );
        assert_eq!(
            should_persist_event_msg(
                &EventMsg::Error(protocol::protocol::ErrorEvent {
                    message: "generic error".into(),
                    codex_error_info: None,
                }),
                EventPersistenceMode::Limited,
            ),
            false
        );
        assert_eq!(
            should_persist_event_msg(&EventMsg::ShutdownComplete, EventPersistenceMode::Limited),
            false
        );
    }
}
