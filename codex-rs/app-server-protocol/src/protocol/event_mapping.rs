use crate::protocol::AgentMessageDeltaNotification;
use crate::protocol::CollabAgentState;
use crate::protocol::CollabAgentTool;
use crate::protocol::CollabAgentToolCallStatus;
use crate::protocol::CommandExecutionOutputDeltaNotification;
use crate::protocol::DynamicToolCallOutputContentItem;
use crate::protocol::DynamicToolCallStatus;
use crate::protocol::FileChangePatchUpdatedNotification;
use crate::protocol::ItemCompletedNotification;
use crate::protocol::ItemStartedNotification;
use crate::protocol::PlanDeltaNotification;
use crate::protocol::ReasoningSummaryPartAddedNotification;
use crate::protocol::ReasoningSummaryTextDeltaNotification;
use crate::protocol::ReasoningTextDeltaNotification;
use crate::protocol::TerminalInteractionNotification;
use crate::protocol::ThreadItem;
use crate::protocol::common::ServerNotification;
use crate::protocol::event_item_projection::ProjectedEventItem;
use crate::protocol::event_item_projection::project_event_msg_item;
use crate::protocol::item_builders::build_command_execution_begin_item;
use crate::protocol::item_builders::build_command_execution_end_item;
use crate::protocol::item_builders::convert_patch_changes;
use protocol::dynamic_tools::DynamicToolCallOutputContentItem as CoreDynamicToolCallOutputContentItem;
use protocol::protocol::EventMsg;
use std::collections::HashMap;

/// Build the app-server notification that directly corresponds to a single core event.
///
/// This only covers the stateless event-to-notification projections that have a one-to-one
/// mapping. Callers remain responsible for any surrounding state checks or side effects before
/// invoking this helper.
pub fn item_event_to_server_notification(
    msg: EventMsg,
    thread_id: &str,
    turn_id: &str,
) -> Option<ServerNotification> {
    let thread_id = thread_id.to_string();
    let turn_id = turn_id.to_string();
    if let Some(projected) = project_event_msg_item(&msg) {
        return Some(match projected {
            ProjectedEventItem::Started {
                item,
                started_at_ms,
                ..
            } => ServerNotification::ItemStarted(ItemStartedNotification {
                thread_id,
                turn_id,
                item,
                started_at_ms,
            }),
            ProjectedEventItem::Completed {
                item,
                completed_at_ms,
                ..
            } => ServerNotification::ItemCompleted(ItemCompletedNotification {
                thread_id,
                turn_id,
                item,
                completed_at_ms,
            }),
        });
    }
    Some(match msg {
        EventMsg::ItemStarted(_) | EventMsg::ItemCompleted(_) => return None,
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
                protocol::protocol::AgentStatus::Errored(_)
                | protocol::protocol::AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
                _ if has_receiver => CollabAgentToolCallStatus::Completed,
                _ => CollabAgentToolCallStatus::Failed,
            };
            let (receiver_thread_ids, agents_states) = match end_event.new_thread_id {
                Some(id) => {
                    let receiver_id = id.to_string();
                    let mut received_status = CollabAgentState::from(end_event.status.clone());
                    received_status.path = end_event.new_agent_path.clone();
                    received_status.agent_nickname = end_event.new_agent_nickname.clone();
                    received_status.agent_role = end_event.new_agent_role.clone();
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
                protocol::protocol::AgentStatus::Errored(_)
                | protocol::protocol::AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut received_status = CollabAgentState::from(end_event.status);
            received_status.path = Some(end_event.receiver_agent_path.clone());
            received_status.agent_nickname = end_event.receiver_agent_nickname.clone();
            received_status.agent_role = end_event.receiver_agent_role.clone();
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
        EventMsg::CollabListAgentsBegin(begin_event) => {
            let item = ThreadItem::CollabAgentToolCall {
                id: begin_event.call_id,
                tool: CollabAgentTool::ListAgents,
                status: CollabAgentToolCallStatus::InProgress,
                sender_thread_id: begin_event.sender_thread_id.to_string(),
                sender_path: begin_event.sender_agent_path,
                receiver_thread_ids: Vec::new(),
                receiver_paths: Vec::new(),
                timeout_ms: None,
                prompt: begin_event.path_prefix,
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
        EventMsg::CollabListAgentsEnd(end_event) => {
            let receiver_paths: Vec<String> = end_event
                .agents
                .iter()
                .map(|agent| agent.agent_path.clone())
                .collect();
            let agents_states = end_event
                .agents
                .into_iter()
                .map(|agent| {
                    let mut state = CollabAgentState::from(agent.lifecycle_status);
                    state.path = Some(agent.agent_path.clone());
                    state.agent_nickname = agent.agent_nickname.clone();
                    state.agent_role = agent.agent_role.clone();
                    if state.message.is_none() {
                        state.message = agent.last_task_message;
                    }
                    (agent.agent_path, state)
                })
                .collect();
            let item = ThreadItem::CollabAgentToolCall {
                id: end_event.call_id,
                tool: CollabAgentTool::ListAgents,
                status: if end_event.success {
                    CollabAgentToolCallStatus::Completed
                } else {
                    CollabAgentToolCallStatus::Failed
                },
                sender_thread_id: end_event.sender_thread_id.to_string(),
                sender_path: end_event.sender_agent_path,
                receiver_thread_ids: Vec::new(),
                receiver_paths,
                timeout_ms: None,
                prompt: end_event.path_prefix,
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
            let status = if end_event.lifecycle_statuses.values().any(|status| {
                matches!(
                    status,
                    protocol::protocol::ThreadLifecycleStatus::Final {
                        result: protocol::protocol::ThreadLifecycleFinalStatus::Errored { .. }
                    } | protocol::protocol::ThreadLifecycleStatus::NotLoaded
                        | protocol::protocol::ThreadLifecycleStatus::SystemError { .. }
                )
            }) {
                CollabAgentToolCallStatus::Failed
            } else {
                CollabAgentToolCallStatus::Completed
            };
            let receiver_thread_ids = end_event
                .lifecycle_statuses
                .keys()
                .map(ToString::to_string)
                .collect();
            let agents_states = end_event
                .lifecycle_statuses
                .iter()
                .map(|(id, status)| {
                    let mut state = CollabAgentState::from(status.clone());
                    state.path = end_event
                        .agent_lifecycles
                        .iter()
                        .find(|entry| entry.thread_id == *id)
                        .and_then(|entry| entry.agent_path.clone());
                    state.agent_nickname = end_event
                        .agent_lifecycles
                        .iter()
                        .find(|entry| entry.thread_id == *id)
                        .and_then(|entry| entry.agent_nickname.clone());
                    state.agent_role = end_event
                        .agent_lifecycles
                        .iter()
                        .find(|entry| entry.thread_id == *id)
                        .and_then(|entry| entry.agent_role.clone());
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
                    .agent_lifecycles
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
                protocol::protocol::AgentStatus::Errored(_)
                | protocol::protocol::AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut receiver_state = CollabAgentState::from(end_event.status);
            receiver_state.path = Some(end_event.receiver_agent_path.clone());
            receiver_state.agent_nickname = end_event.receiver_agent_nickname.clone();
            receiver_state.agent_role = end_event.receiver_agent_role.clone();
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
                protocol::protocol::AgentStatus::Errored(_)
                | protocol::protocol::AgentStatus::NotFound => CollabAgentToolCallStatus::Failed,
                _ => CollabAgentToolCallStatus::Completed,
            };
            let receiver_id = end_event.receiver_thread_id.to_string();
            let mut receiver_state = CollabAgentState::from(end_event.status);
            receiver_state.path = Some(end_event.receiver_agent_path.clone());
            receiver_state.agent_nickname = end_event.receiver_agent_nickname.clone();
            receiver_state.agent_role = end_event.receiver_agent_role.clone();
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
            let protocol::protocol::AgentMessageContentDeltaEvent { item_id, delta, .. } = event;
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextCompactionReplacementItem;
    use pretty_assertions::assert_eq;
    use protocol::AgentPath;
    use protocol::ThreadId;
    use protocol::event_command::EventCommandEvent;
    use protocol::event_command::EventCommandEventKind as CoreEventCommandEventKind;
    use protocol::items::AgentMessageContent;
    use protocol::items::AgentMessageItem;
    use protocol::items::CollabAgentMessageItem;
    use protocol::items::EventCommandEventItem;
    use protocol::items::EventDrivenToolItem;
    use protocol::items::TurnItem;
    use protocol::items::UserMessageItem;
    use protocol::models::ContentItem;
    use protocol::models::ResponseItem;
    use protocol::protocol::AgentStatus;
    use protocol::protocol::CollabResumeBeginEvent;
    use protocol::protocol::CollabResumeEndEvent;
    use protocol::protocol::CollabWaitingBeginEvent;
    use protocol::protocol::ExecCommandOutputDeltaEvent;
    use protocol::protocol::ExecOutputStream;
    use protocol::protocol::InterAgentCommunication;
    use protocol::protocol::InterAgentOperation;
    use protocol::protocol::ItemCompletedEvent;
    use protocol::protocol::ItemStartedEvent;
    use protocol::protocol::ResponseItemCompletedEvent;
    use protocol::protocol::ThreadLifecycleStatus;
    use protocol::user_input::UserInput as CoreUserInput;
    use serde_json::json;

    fn assert_item_started_server_notification(
        notification: Option<ServerNotification>,
        expected: ItemStartedNotification,
    ) {
        match notification.expect("expected notification") {
            ServerNotification::ItemStarted(payload) => assert_eq!(payload, expected),
            other => panic!("expected item started notification, got {other:?}"),
        }
    }

    fn assert_item_completed_server_notification(
        notification: Option<ServerNotification>,
        expected: ItemCompletedNotification,
    ) {
        match notification.expect("expected notification") {
            ServerNotification::ItemCompleted(payload) => assert_eq!(payload, expected),
            other => panic!("expected item completed notification, got {other:?}"),
        }
    }

    fn assert_command_execution_output_delta_server_notification(
        notification: Option<ServerNotification>,
        expected: CommandExecutionOutputDeltaNotification,
    ) {
        match notification.expect("expected notification") {
            ServerNotification::CommandExecutionOutputDelta(payload) => {
                assert_eq!(payload, expected)
            }
            other => panic!("expected command execution output delta, got {other:?}"),
        }
    }

    #[test]
    fn item_event_to_server_notification_skips_raw_subagent_notification_user_item() {
        let message = concat!(
            "<subagent_notification>\n",
            r#"{"agent_path":"/root/worker","status":{"completed":"done"}}"#,
            "\n</subagent_notification>"
        )
        .to_string();
        let notification = item_event_to_server_notification(
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".into(),
                item: TurnItem::UserMessage(UserMessageItem {
                    id: "user-1".into(),
                    content: vec![CoreUserInput::Text {
                        text: message,
                        text_elements: Vec::new(),
                    }],
                }),
                completed_at_ms: 1,
            }),
            "thread-1",
            "turn-1",
        );

        assert!(notification.is_none());
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
            status: protocol::protocol::AgentStatus::NotFound,
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
                            agent_nickname: None,
                            agent_role: None,
                            ..CollabAgentState::from(protocol::protocol::AgentStatus::NotFound)
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
            receiver_agents: vec![protocol::protocol::CollabAgentRef {
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
    fn command_wait_completed_display_event_maps_to_thread_item() {
        let event = protocol::protocol::CommandWaitDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            id: "wait-1".to_string(),
            command_id: "cmd-1".to_string(),
            status: protocol::models::CommandWaitStatus::Completed,
            notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
            exit_code: Some(0),
            wall_time_seconds: 1.25,
            wait_timeout_ms: 250,
            created_at_ms: 1234,
            lifecycle_at_ms: 789,
        };

        let notification = item_event_to_server_notification(
            EventMsg::CommandWaitCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.lifecycle_at_ms,
                item: ThreadItem::CommandWait {
                    id: "wait-1".to_string(),
                    command_id: "cmd-1".to_string(),
                    status: crate::protocol::CommandWaitStatus::Completed,
                    notification: Some(crate::protocol::CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                },
            },
        );
    }

    #[test]
    fn builtin_tool_call_completed_display_event_maps_to_thread_item() {
        let event = protocol::protocol::BuiltinToolCallDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            id: "builtin-1".to_string(),
            tool: "poll_event".to_string(),
            arguments: serde_json::json!({}),
            status: protocol::protocol::BuiltinToolCallStatus::Completed,
            output: Some(serde_json::json!({
                "timedOut": false,
                "sourceHint": "user_input",
                "waitedMs": 12,
                "initialTimeoutMs": 50,
                "currentTimeoutMs": 50,
                "hardCapTimeoutMs": 1000
            })),
            lifecycle_at_ms: 790,
        };

        let notification = item_event_to_server_notification(
            EventMsg::BuiltinToolCallCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.lifecycle_at_ms,
                item: ThreadItem::BuiltinToolCall {
                    id: "builtin-1".to_string(),
                    tool: "poll_event".to_string(),
                    arguments: serde_json::json!({}),
                    status: crate::protocol::DynamicToolCallStatus::Completed,
                    output: event.output,
                },
            },
        );
    }

    #[test]
    fn item_completed_preserves_context_compaction_replacement_history() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::ContextCompaction(
                serde_json::from_value(json!({
                    "id": "compact-1",
                    "replacementHistory": [
                        {
                            "type": "agentMessage",
                            "id": "compact-seed",
                            "content": [
                                {
                                    "type": "Text",
                                    "text": "LOCAL_SUMMARY"
                                }
                            ]
                        }
                    ],
                }))
                .expect("context compaction item"),
            ),
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
                item: ThreadItem::ContextCompaction {
                    id: "compact-1".to_string(),
                    replacement_history: vec![ContextCompactionReplacementItem::AgentMessage {
                        id: "compact-seed".to_string(),
                        text: "LOCAL_SUMMARY".to_string(),
                        phase: None,
                        memory_citation: None,
                    }],
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
    fn item_started_maps_event_driven_tool_turn_item() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::EventDrivenTool(EventDrivenToolItem {
                id: "event-1".to_string(),
                tool: "process_exit_subscribe".to_string(),
                title: "Process exited".to_string(),
                text: "Session 42 exited with code 0".to_string(),
            }),
            started_at_ms: 456,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ItemStarted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_started_server_notification(
            notification,
            ItemStartedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                started_at_ms: event.started_at_ms,
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
    fn item_started_maps_event_command_turn_item() {
        let event = ItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::EventCommandEvent(EventCommandEventItem {
                id: "event-command-1".to_string(),
                event: EventCommandEvent {
                    subscription_id: "sub-1".to_string(),
                    kind: CoreEventCommandEventKind::Exited,
                    label: Some("tests".to_string()),
                    command: "cargo test".to_string(),
                    cwd: Some("/tmp/project".to_string()),
                    line: Some("done".to_string()),
                    sequence: Some(7),
                    exit_code: Some(0),
                    signal: None,
                    message: Some("finished".to_string()),
                    truncated: false,
                    created_at: 1_700_000_000,
                },
            }),
            started_at_ms: 456,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ItemStarted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_started_server_notification(
            notification,
            ItemStartedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                started_at_ms: event.started_at_ms,
                item: ThreadItem::EventCommandEvent {
                    id: "event-command-1".to_string(),
                    subscription_id: "sub-1".to_string(),
                    kind: crate::protocol::EventCommandEventKind::Exited,
                    label: Some("tests".to_string()),
                    command: "cargo test".to_string(),
                    cwd: Some("/tmp/project".to_string()),
                    line: Some("done".to_string()),
                    sequence: Some(7),
                    exit_code: Some(0),
                    signal: None,
                    message: Some("finished".to_string()),
                    truncated: false,
                    created_at: 1_700_000_000,
                },
            },
        );
    }

    #[test]
    fn item_completed_maps_event_command_turn_item() {
        let event = ItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: TurnItem::EventCommandEvent(EventCommandEventItem {
                id: "event-command-1".to_string(),
                event: EventCommandEvent {
                    subscription_id: "sub-1".to_string(),
                    kind: CoreEventCommandEventKind::Exited,
                    label: Some("tests".to_string()),
                    command: "cargo test".to_string(),
                    cwd: Some("/tmp/project".to_string()),
                    line: Some("done".to_string()),
                    sequence: Some(7),
                    exit_code: Some(0),
                    signal: None,
                    message: Some("finished".to_string()),
                    truncated: false,
                    created_at: 1_700_000_000,
                },
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
                item: ThreadItem::EventCommandEvent {
                    id: "event-command-1".to_string(),
                    subscription_id: "sub-1".to_string(),
                    kind: crate::protocol::EventCommandEventKind::Exited,
                    label: Some("tests".to_string()),
                    command: "cargo test".to_string(),
                    cwd: Some("/tmp/project".to_string()),
                    line: Some("done".to_string()),
                    sequence: Some(7),
                    exit_code: Some(0),
                    signal: None,
                    message: Some("finished".to_string()),
                    truncated: false,
                    created_at: 1_700_000_000,
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
                    lifecycle_status: CollabAgentState {
                        path: Some("/root/worker".to_string()),
                        agent_nickname: None,
                        agent_role: None,
                        lifecycle_status: ThreadLifecycleStatus::completed(Some(
                            "done".to_string(),
                        )),
                        message: Some("done".to_string()),
                    },
                },
            },
        );
    }

    #[test]
    fn response_item_completed_maps_inter_agent_communication_to_collab_message() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "status update".to_string(),
            InterAgentOperation::FollowupTask,
        )
        .with_trigger_turn(false);
        let event = ResponseItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: ResponseItem::InterAgentCommunication {
                id: Some("collab-response-1".to_string()),
                communication,
            },
            completed_at_ms: 789,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ResponseItemCompleted(event.clone()),
            "thread-4",
            "turn-4",
        );

        assert_item_completed_server_notification(
            notification,
            ItemCompletedNotification {
                thread_id: "thread-4".to_string(),
                turn_id: "turn-4".to_string(),
                completed_at_ms: event.completed_at_ms,
                item: ThreadItem::CollabAgentMessage {
                    id: "collab-response-1".to_string(),
                    operation: crate::protocol::CollabAgentOperation::FollowupTask,
                    sender_thread_id: None,
                    sender_path: "/root/worker".to_string(),
                    recipient_thread_id: None,
                    recipient_path: "/root".to_string(),
                    other_recipient_paths: Vec::new(),
                    content: "status update".to_string(),
                    trigger_turn: false,
                },
            },
        );
    }

    #[test]
    fn response_item_completed_maps_child_completion_to_collab_status_update() {
        let communication = InterAgentCommunication::new(
            AgentPath::try_from("/root/worker").expect("agent path"),
            AgentPath::root(),
            Vec::new(),
            "completed".to_string(),
            InterAgentOperation::ChildCompletion,
        )
        .with_status(AgentStatus::Completed(Some("done".to_string())));
        let event = ResponseItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-ignored".to_string(),
            item: ResponseItem::InterAgentCommunication {
                id: Some("collab-response-2".to_string()),
                communication,
            },
            completed_at_ms: 790,
        };

        let notification = item_event_to_server_notification(
            EventMsg::ResponseItemCompleted(event.clone()),
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
                    id: "collab-response-2".to_string(),
                    sender_thread_id: None,
                    sender_path: "/root/worker".to_string(),
                    recipient_thread_id: None,
                    recipient_path: "/root".to_string(),
                    lifecycle_status: CollabAgentState {
                        path: Some("/root/worker".to_string()),
                        agent_nickname: None,
                        agent_role: None,
                        lifecycle_status: ThreadLifecycleStatus::completed(Some(
                            "done".to_string(),
                        )),
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
                sequence: None,
                generates_notification: false,
                created_at_ms: 0,
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
