use crate::protocol::common::ServerNotification;
use crate::protocol::item_builders::build_command_execution_begin_item;
use crate::protocol::item_builders::build_command_execution_end_item;
use crate::protocol::item_builders::convert_patch_changes;
use crate::protocol::v2::AgentMessageDeltaNotification;
use crate::protocol::v2::CollabAgentState;
use crate::protocol::v2::CollabAgentTool;
use crate::protocol::v2::CollabAgentToolCallStatus;
use crate::protocol::v2::CommandExecutionOutputDeltaNotification;
use crate::protocol::v2::DynamicToolCallOutputContentItem;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::FileChangePatchUpdatedNotification;
use crate::protocol::v2::ItemCompletedNotification;
use crate::protocol::v2::ItemStartedNotification;
use crate::protocol::v2::PlanDeltaNotification;
use crate::protocol::v2::ReasoningSummaryPartAddedNotification;
use crate::protocol::v2::ReasoningSummaryTextDeltaNotification;
use crate::protocol::v2::ReasoningTextDeltaNotification;
use crate::protocol::v2::TerminalInteractionNotification;
use crate::protocol::v2::ThreadItem;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem as CoreDynamicToolCallOutputContentItem;
use codex_protocol::protocol::EventMsg;
use std::collections::HashMap;

/// Build the v2 app-server notification that directly corresponds to a single core event.
///
/// This only covers the stateless event-to-notification projections that have a one-to-one
/// mapping. Callers remain responsible for any surrounding state checks or side effects before
/// invoking this helper.
pub fn item_event_to_server_notification(
    msg: EventMsg,
    thread_id: &str,
    turn_id: &str,
) -> ServerNotification {
    let thread_id = thread_id.to_string();
    let turn_id = turn_id.to_string();
    match msg {
        EventMsg::DynamicToolCallResponse(response) => {
            let status = if response.success {
                DynamicToolCallStatus::Completed
            } else {
                DynamicToolCallStatus::Failed
            };
            let duration_ms = i64::try_from(response.duration.as_millis()).ok();
            let item = ThreadItem::DynamicToolCall {
                id: response.call_id,
                namespace: response.namespace,
                tool: response.tool,
                arguments: response.arguments,
                status,
                content_items: Some(
                    response
                        .content_items
                        .into_iter()
                        .map(|item| match item {
                            CoreDynamicToolCallOutputContentItem::InputText { text } => {
                                DynamicToolCallOutputContentItem::InputText { text }
                            }
                            CoreDynamicToolCallOutputContentItem::InputImage { image_url } => {
                                DynamicToolCallOutputContentItem::InputImage { image_url }
                            }
                        })
                        .collect(),
                ),
                success: Some(response.success),
                duration_ms,
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id: response.turn_id,
                item,
                completed_at_ms: response.completed_at_ms,
            })
        }
        EventMsg::CollabAgentSpawnBegin(begin_event) => {
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids: Vec::new(),
                receiver_paths: Vec::new(),
                timeout_ms: None,
                prompt: Some(begin_event.prompt),
                model: Some(begin_event.model),
                reasoning_effort: Some(begin_event.reasoning_effort),
                agents_states: HashMap::new(),
            };
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms: begin_event.started_at_ms,
            })
        }
        EventMsg::CollabAgentSpawnEnd(end_event) => {
            let has_receiver = end_event.new_thread_id.is_some();
            let status = match &end_event.status {
                codex_protocol::protocol::AgentStatus::Errored(_)
                | codex_protocol::protocol::AgentStatus::NotFound => {
                    CollabAgentToolCallStatus::Failed
                }
                _ if has_receiver => CollabAgentToolCallStatus::Completed,
                _ => CollabAgentToolCallStatus::Failed,
            };
            let (receiver_thread_ids, agents_states) = match end_event.new_thread_id {
                Some(id) => {
                    let receiver_id = id.to_string();
                    let mut received_status = CollabAgentState::from(end_event.status.clone());
                    received_status.path = end_event.new_agent_path.clone();
                    (
                        vec![receiver_id.clone()],
                        [(receiver_id, received_status)].into_iter().collect(),
                    )
                }
                None => (Vec::new(), HashMap::new()),
            };
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::SpawnAgent,
                status,
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids,
                receiver_paths: end_event.new_agent_path.into_iter().collect(),
                timeout_ms: None,
                prompt: Some(end_event.prompt),
                model: Some(end_event.model),
                reasoning_effort: Some(end_event.reasoning_effort),
                agents_states,
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms: end_event.completed_at_ms,
            })
        }
        EventMsg::CollabAgentInteractionBegin(begin_event) => {
            let receiver_thread_ids = vec![begin_event.receiver_thread_id.to_string()];
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::SendInput,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids,
                receiver_paths: vec![begin_event.receiver_agent_path],
                timeout_ms: None,
                prompt: Some(begin_event.prompt),
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::new(),
            };
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms: begin_event.started_at_ms,
            })
        }
        EventMsg::CollabAgentInteractionEnd(end_event) => {
            let status = match &end_event.status {
                codex_protocol::protocol::AgentStatus::Errored(_)
                | codex_protocol::protocol::AgentStatus::NotFound => {
                    CollabAgentToolCallStatus::Failed
                }
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut received_status = CollabAgentState::from(end_event.status);
            received_status.path = Some(end_event.receiver_agent_path.clone());
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::SendInput,
                status,
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids: vec![receiver_id.clone()],
                receiver_paths: vec![end_event.receiver_agent_path],
                timeout_ms: None,
                prompt: Some(end_event.prompt),
                model: None,
                reasoning_effort: None,
                agents_states: [(receiver_id, received_status)].into_iter().collect(),
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms: end_event.completed_at_ms,
            })
        }
        EventMsg::CollabWaitingBegin(begin_event) => {
            let receiver_thread_ids = begin_event
                .receiver_thread_ids
                .iter()
                .map(ToString::to_string)
                .collect();
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::Wait,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids,
                receiver_paths: begin_event
                    .receiver_agents
                    .into_iter()
                    .filter_map(|agent| agent.agent_path)
                    .collect(),
                timeout_ms: Some(begin_event.timeout_ms),
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::new(),
            };
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms: begin_event.started_at_ms,
            })
        }
        EventMsg::CollabWaitingEnd(end_event) => {
            let status = if end_event.statuses.values().any(|status| {
                matches!(
                    status,
                    codex_protocol::protocol::AgentStatus::Errored(_)
                        | codex_protocol::protocol::AgentStatus::NotFound
                )
            }) {
                CollabAgentToolCallStatus::Failed
            } else {
                CollabAgentToolCallStatus::Completed
            };
            let receiver_thread_ids = end_event.statuses.keys().map(ToString::to_string).collect();
            let agents_states = end_event
                .statuses
                .iter()
                .map(|(id, status)| {
                    let mut state = CollabAgentState::from(status.clone());
                    state.path = end_event
                        .agent_statuses
                        .iter()
                        .find(|entry| entry.thread_id == *id)
                        .and_then(|entry| entry.agent_path.clone());
                    (id.to_string(), state)
                })
                .collect();
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::Wait,
                status,
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids,
                receiver_paths: end_event
                    .agent_statuses
                    .iter()
                    .filter_map(|entry| entry.agent_path.clone())
                    .collect(),
                timeout_ms: Some(end_event.timeout_ms),
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states,
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms: end_event.completed_at_ms,
            })
        }
        EventMsg::CollabCloseBegin(begin_event) => {
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::CloseAgent,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids: vec![begin_event.receiver_thread_id.to_string()],
                receiver_paths: vec![begin_event.receiver_agent_path],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::new(),
            };
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms: begin_event.started_at_ms,
            })
        }
        EventMsg::CollabCloseEnd(end_event) => {
            let status = match &end_event.status {
                codex_protocol::protocol::AgentStatus::Errored(_)
                | codex_protocol::protocol::AgentStatus::NotFound => {
                    CollabAgentToolCallStatus::Failed
                }
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut receiver_state = CollabAgentState::from(end_event.status);
            receiver_state.path = Some(end_event.receiver_agent_path.clone());
            let agents_states = [(receiver_id.clone(), receiver_state)]
                .into_iter()
                .collect();
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::CloseAgent,
                status,
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids: vec![receiver_id],
                receiver_paths: vec![end_event.receiver_agent_path],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states,
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms: end_event.completed_at_ms,
            })
        }
        EventMsg::CollabResumeBegin(begin_event) => {
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::ResumeAgent,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids: vec![begin_event.receiver_thread_id.to_string()],
                receiver_paths: vec![begin_event.receiver_agent_path],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::new(),
            };
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms: begin_event.started_at_ms,
            })
        }
        EventMsg::CollabResumeEnd(end_event) => {
            let status = match &end_event.status {
                codex_protocol::protocol::AgentStatus::Errored(_)
                | codex_protocol::protocol::AgentStatus::NotFound => {
                    CollabAgentToolCallStatus::Failed
                }
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut receiver_state = CollabAgentState::from(end_event.status);
            receiver_state.path = Some(end_event.receiver_agent_path.clone());
            let agents_states = [(receiver_id.clone(), receiver_state)]
                .into_iter()
                .collect();
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::ResumeAgent,
                status,
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids: vec![receiver_id],
                receiver_paths: vec![end_event.receiver_agent_path],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states,
            };
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms: end_event.completed_at_ms,
            })
        }
        EventMsg::AgentMessageContentDelta(event) => {
            let codex_protocol::protocol::AgentMessageContentDeltaEvent { item_id, delta, .. } =
                event;
            ServerNotification::AgentMessageDelta(AgentMessageDeltaNotification {
                thread_id,
                turn_id,
                item_id,
                delta,
            })
        }
        EventMsg::PlanDelta(event) => ServerNotification::PlanDelta(PlanDeltaNotification {
            thread_id,
            turn_id,
            item_id: event.item_id,
            delta: event.delta,
        }),
        EventMsg::ReasoningContentDelta(event) => {
            ServerNotification::ReasoningSummaryTextDelta(ReasoningSummaryTextDeltaNotification {
                thread_id,
                turn_id,
                item_id: event.item_id,
                delta: event.delta,
                summary_index: event.summary_index,
            })
        }
        EventMsg::ReasoningRawContentDelta(event) => {
            ServerNotification::ReasoningTextDelta(ReasoningTextDeltaNotification {
                thread_id,
                turn_id,
                item_id: event.item_id,
                delta: event.delta,
                content_index: event.content_index,
            })
        }
        EventMsg::AgentReasoningSectionBreak(event) => {
            ServerNotification::ReasoningSummaryPartAdded(ReasoningSummaryPartAddedNotification {
                thread_id,
                turn_id,
                item_id: event.item_id,
                summary_index: event.summary_index,
            })
        }
        EventMsg::ItemStarted(item_started_event) => {
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item: item_started_event.item.into(),
                started_at_ms: item_started_event.started_at_ms,
            })
        }
        EventMsg::ItemCompleted(item_completed_event) => {
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item: item_completed_event.item.into(),
                completed_at_ms: item_completed_event.completed_at_ms,
            })
        }
        EventMsg::PatchApplyUpdated(event) => {
            ServerNotification::FileChangePatchUpdated(FileChangePatchUpdatedNotification {
                thread_id,
                turn_id,
                item_id: event.call_id,
                changes: convert_patch_changes(&event.changes),
            })
        }
        EventMsg::ExecCommandBegin(exec_command_begin_event) => {
            ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item: build_command_execution_begin_item(&exec_command_begin_event),
                started_at_ms: exec_command_begin_event.started_at_ms,
            })
        }
        EventMsg::ExecCommandOutputDelta(exec_command_output_delta_event) => {
            let item_id = exec_command_output_delta_event.call_id;
            let delta = String::from_utf8_lossy(&exec_command_output_delta_event.chunk).to_string();
            ServerNotification::CommandExecutionOutputDelta(
                CommandExecutionOutputDeltaNotification {
                    thread_id,
                    turn_id,
                    item_id,
                    delta,
                },
            )
        }
        EventMsg::TerminalInteraction(terminal_event) => {
            ServerNotification::TerminalInteraction(TerminalInteractionNotification {
                thread_id,
                turn_id,
                item_id: terminal_event.call_id,
                process_id: terminal_event.process_id,
                stdin: terminal_event.stdin,
            })
        }
        EventMsg::ExecCommandEnd(exec_command_end_event) => {
            ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item: build_command_execution_end_item(&exec_command_end_event),
                completed_at_ms: exec_command_end_event.completed_at_ms,
            })
        }
        _ => unreachable!("unsupported item event"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v2::CollabAgentStatus;
    use codex_protocol::AgentPath;
    use codex_protocol::ThreadId;
    use codex_protocol::items::AgentMessageContent;
    use codex_protocol::items::AgentMessageItem;
    use codex_protocol::items::CollabAgentMessageItem;
    use codex_protocol::items::EventDrivenToolItem;
    use codex_protocol::items::TurnItem;
    use codex_protocol::protocol::AgentStatus;
    use codex_protocol::protocol::CollabResumeBeginEvent;
    use codex_protocol::protocol::CollabResumeEndEvent;
    use codex_protocol::protocol::CollabWaitingBeginEvent;
    use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
    use codex_protocol::protocol::ExecOutputStream;
    use codex_protocol::protocol::InterAgentCommunication;
    use codex_protocol::protocol::InterAgentOperation;
    use codex_protocol::protocol::ItemCompletedEvent;
    use pretty_assertions::assert_eq;

    fn assert_item_started_server_notification(
        notification: ServerNotification,
        expected: ItemStartedNotification,
    ) {
        match notification {
            ServerNotification::ItemStarted(payload) => assert_eq!(payload, expected),
            other => panic!("expected item started notification, got {other:?}"),
        }
    }

    fn assert_item_completed_server_notification(
        notification: ServerNotification,
        expected: ItemCompletedNotification,
    ) {
        match notification {
            ServerNotification::ItemCompleted(payload) => assert_eq!(payload, expected),
            other => panic!("expected item completed notification, got {other:?}"),
        }
    }

    fn assert_command_execution_output_delta_server_notification(
        notification: ServerNotification,
        expected: CommandExecutionOutputDeltaNotification,
    ) {
        match notification {
            ServerNotification::CommandExecutionOutputDelta(payload) => {
                assert_eq!(payload, expected)
            }
            other => panic!("expected command execution output delta, got {other:?}"),
        }
    }

    #[test]
    fn collab_resume_begin_maps_to_item_started_resume_agent() {
        let event = CollabResumeBeginEvent {
            call_id: "call-1".to_string(),
            started_at_ms: 123,
            sender_thread_id: ThreadId::new(),
            sender_agent_path: "/root".to_string(),
            receiver_thread_id: ThreadId::new(),
            receiver_agent_path: "/root/scout".to_string(),
            receiver_agent_nickname: None,
            receiver_agent_role: None,
        };

        let notification = item_event_to_server_notification(
            EventMsg::CollabResumeBegin(event.clone()),
            "thread-1",
            "turn-1",
        );
        assert_item_started_server_notification(
            notification,
            ItemStartedNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                started_at_ms: event.started_at_ms,
                item: ThreadItem::CollabAgentToolCall {
                    id: event.call_id,
                    tool: CollabAgentTool::ResumeAgent,
                    status: CollabAgentToolCallStatus::InProgress,
                    sender_thread_id: event.sender_thread_id.to_string(),
                    sender_path: event.sender_agent_path,
                    receiver_thread_ids: vec![event.receiver_thread_id.to_string()],
                    receiver_paths: vec![event.receiver_agent_path],
                    timeout_ms: None,
                    prompt: None,
                    model: None,
                    reasoning_effort: None,
                    agents_states: HashMap::new(),
                },
            },
        );
    }

    #[test]
    fn collab_resume_end_maps_to_item_completed_resume_agent() {
        let event = CollabResumeEndEvent {
            call_id: "call-2".to_string(),
            completed_at_ms: 456,
            sender_thread_id: ThreadId::new(),
            sender_agent_path: "/root".to_string(),
            receiver_thread_id: ThreadId::new(),
            receiver_agent_path: "/root/scout".to_string(),
            receiver_agent_nickname: None,
            receiver_agent_role: None,
            status: codex_protocol::protocol::AgentStatus::NotFound,
        };

        let receiver_id = event.receiver_thread_id.to_string();
        let notification = item_event_to_server_notification(
            EventMsg::CollabResumeEnd(event.clone()),
            "thread-2",
            "turn-2",
        );
        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-2".to_string(),
                turn_id: "turn-2".to_string(),
                completed_at_ms: event.completed_at_ms,
                item: ThreadItem::CollabAgentToolCall {
                    id: event.call_id,
                    tool: CollabAgentTool::ResumeAgent,
                    status: CollabAgentToolCallStatus::Failed,
                    sender_thread_id: event.sender_thread_id.to_string(),
                    sender_path: event.sender_agent_path,
                    receiver_thread_ids: vec![receiver_id.clone()],
                    receiver_paths: vec![event.receiver_agent_path.clone()],
                    timeout_ms: None,
                    prompt: None,
                    model: None,
                    reasoning_effort: None,
                    agents_states: [(
                        receiver_id,
                        CollabAgentState {
                            path: Some(event.receiver_agent_path),
                            ..CollabAgentState::from(
                                codex_protocol::protocol::AgentStatus::NotFound,
                            )
                        },
                    )]
                    .into_iter()
                    .collect(),
                },
            },
        );
    }

    #[test]
    fn collab_wait_begin_maps_timeout_and_receiver_path() {
        let sender_thread_id = ThreadId::new();
        let receiver_thread_id = ThreadId::new();
        let event = CollabWaitingBeginEvent {
            started_at_ms: 123,
            sender_thread_id,
            sender_agent_path: "/root".to_string(),
            receiver_thread_ids: vec![receiver_thread_id],
            receiver_agents: vec![codex_protocol::protocol::CollabAgentRef {
                thread_id: receiver_thread_id,
                agent_path: Some("/root/scout".to_string()),
                agent_nickname: None,
                agent_role: None,
            }],
            timeout_ms: 30_000,
            call_id: "wait-1".to_string(),
        };

        let notification = item_event_to_server_notification(
            EventMsg::CollabWaitingBegin(event.clone()),
            "thread-3",
            "turn-3",
        );
        assert_item_started_server_notification(
            notification,
            ItemStartedNotification {
                thread_id: "thread-3".to_string(),
                turn_id: "turn-3".to_string(),
                started_at_ms: event.started_at_ms,
                item: ThreadItem::CollabAgentToolCall {
                    id: event.call_id,
                    tool: CollabAgentTool::Wait,
                    status: CollabAgentToolCallStatus::InProgress,
                    sender_thread_id: sender_thread_id.to_string(),
                    sender_path: event.sender_agent_path,
                    receiver_thread_ids: vec![receiver_thread_id.to_string()],
                    receiver_paths: vec!["/root/scout".to_string()],
                    timeout_ms: Some(30_000),
                    prompt: None,
                    model: None,
                    reasoning_effort: None,
                    agents_states: HashMap::new(),
                },
            },
        );
    }

    #[test]
    fn item_completed_normalizes_agent_message_payloads() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::AgentMessage(AgentMessageItem {
                id: "agent-1".to_string(),
                content: vec![AgentMessageContent::Text {
                    text: "[Process exit subscription] Session 42 exited with code 0".to_string(),
                }],
                phase: None,
                memory_citation: None,
            }),
            completed_at_ms: 789,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ItemCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.completed_at_ms,
                item: ThreadItem::AgentMessage {
                    id: "agent-1".to_string(),
                    text: "[Process exit subscription] Session 42 exited with code 0".to_string(),
                    phase: None,
                    memory_citation: None,
                },
            },
        );
    }

    #[test]
    fn item_completed_maps_event_driven_tool_turn_item() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::EventDrivenTool(EventDrivenToolItem {
                id: "event-1".to_string(),
                tool: "process_exit_subscribe".to_string(),
                title: "Process exited".to_string(),
                text: "Session 42 exited with code 0".to_string(),
            }),
            completed_at_ms: 789,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ItemCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.completed_at_ms,
                item: ThreadItem::EventDrivenTool {
                    id: "event-1".to_string(),
                    tool: "process_exit_subscribe".to_string(),
                    title: "Process exited".to_string(),
                    text: "Session 42 exited with code 0".to_string(),
                },
            },
        );
    }

    #[test]
    fn item_completed_maps_collab_status_turn_item() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".to_string(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(AgentStatus::Completed(Some("done".to_string())));
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::CollabAgentMessage(CollabAgentMessageItem {
                id: "collab-1".to_string(),
                communication,
            }),
            completed_at_ms: 789,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ItemCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.completed_at_ms,
                item: ThreadItem::CollabAgentStatusUpdate {
                    id: "collab-1".to_string(),
                    sender_thread_id: None,
                    sender_path: "/root/worker".to_string(),
                    recipient_thread_id: None,
                    recipient_path: "/root".to_string(),
                    status: CollabAgentState {
                        path: Some("/root/worker".to_string()),
                        status: CollabAgentStatus::Completed,
                        message: Some("done".to_string()),
                    },
                },
            },
        );
    }

    #[test]
    fn exec_command_output_delta_maps_to_command_execution_output_delta() {
        let notification = item_event_to_server_notification(
            EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                call_id: "call-1".to_string(),
                stream: ExecOutputStream::Stdout,
                chunk: b"hello".to_vec(),
            }),
            "thread-1",
            "turn-1",
        );

        assert_command_execution_output_delta_server_notification(
            notification,
            CommandExecutionOutputDeltaNotification {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "call-1".to_string(),
                delta: "hello".to_string(),
            },
        );
    }
}
