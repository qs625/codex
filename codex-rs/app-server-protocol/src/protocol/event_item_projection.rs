use crate::protocol::CommandExecutionNotificationKind;
use crate::protocol::CommandWaitNotificationKind;
use crate::protocol::CommandWaitStatus;
use crate::protocol::ContextCompactionReplacementItem;
use crate::protocol::DynamicToolCallStatus;
use crate::protocol::EventCommandEventKind;
use crate::protocol::HookPromptFragment;
use crate::protocol::InjectedContextSection;
use crate::protocol::McpToolCallError;
use crate::protocol::McpToolCallResult;
use crate::protocol::McpToolCallStatus;
use crate::protocol::PatchApplyStatus;
use crate::protocol::ThreadGoalUpdateAction;
use crate::protocol::ThreadGoalUpdateSource;
use crate::protocol::ThreadItem;
use crate::protocol::UserInput;
use crate::protocol::WebSearchAction;
use crate::protocol::assistant_message_thread_item;
use crate::protocol::item_builders::convert_patch_changes;
use crate::protocol::response_item_projection::is_legacy_structured_assistant_message_text;
use crate::protocol::response_item_projection::is_legacy_structured_user_inputs;
use crate::protocol::response_item_projection::thread_goal_from_update_goal;
use crate::protocol::response_item_projection::thread_goal_status_from_update_status;
use crate::protocol::response_item_projection::thread_item_from_inter_agent_communication;
use protocol::items::AgentMessageContent as CoreAgentMessageContent;
use protocol::items::ContextCompactionReplacementItem as CoreContextCompactionReplacementItem;
use protocol::items::TurnItem as CoreTurnItem;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectedEventItem {
    Started {
        turn_id: String,
        item: ThreadItem,
        started_at_ms: i64,
    },
    Completed {
        turn_id: String,
        item: ThreadItem,
        completed_at_ms: i64,
    },
}

/// Project display-capable runtime events to app-server thread items.
///
/// `EventMsg` is the runtime/UI event log. This adapter is intentionally at
/// the protocol edge: provider/model context continues to consume
/// `ResponseItem` directly, while live notifications and persisted thread
/// replay share this display projection.
pub fn project_event_msg_item(event: &EventMsg) -> Option<ProjectedEventItem> {
    match event {
        EventMsg::ItemStarted(event) => {
            let item = thread_item_from_turn_item(event.item.clone())?;
            Some(ProjectedEventItem::Started {
                turn_id: event.turn_id.clone(),
                item,
                started_at_ms: event.started_at_ms,
            })
        }
        EventMsg::ItemCompleted(event) => {
            let item = thread_item_from_turn_item(event.item.clone())?;
            Some(ProjectedEventItem::Completed {
                turn_id: event.turn_id.clone(),
                item,
                completed_at_ms: event.completed_at_ms,
            })
        }
        EventMsg::ResponseItemCompleted(event) => {
            let ResponseItem::InterAgentCommunication {
                id: Some(id),
                communication,
            } = &event.item
            else {
                return None;
            };
            Some(ProjectedEventItem::Completed {
                turn_id: event.turn_id.clone(),
                item: thread_item_from_inter_agent_communication(id.clone(), communication.clone()),
                completed_at_ms: event.completed_at_ms,
            })
        }
        EventMsg::CommandWaitStarted(event) => Some(ProjectedEventItem::Started {
            turn_id: event.turn_id.clone(),
            item: command_wait_thread_item(event),
            started_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::CommandWaitCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: command_wait_thread_item(event),
            completed_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::CommandWriteStdinCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::CommandWriteStdin {
                id: event.id.clone(),
                command_id: event.command_id.clone(),
                bytes_written: event.bytes_written,
                contains_newline: event.contains_newline,
                created_at_ms: event.created_at_ms,
            },
            completed_at_ms: event.completed_at_ms,
        }),
        EventMsg::CommandExecutionNotificationCompleted(event) => {
            Some(ProjectedEventItem::Completed {
                turn_id: event.turn_id.clone(),
                item: ThreadItem::CommandExecutionNotification {
                    id: event.id.clone(),
                    command_item_id: event.command_item_id.clone(),
                    kind: CommandExecutionNotificationKind::from(event.kind),
                    message: event.message.clone(),
                    output: event.output.clone(),
                    exit_code: event.exit_code,
                    created_at_ms: event.created_at_ms,
                },
                completed_at_ms: event.completed_at_ms,
            })
        }
        EventMsg::BuiltinToolCallStarted(event) => Some(ProjectedEventItem::Started {
            turn_id: event.turn_id.clone(),
            item: builtin_tool_call_thread_item(event),
            started_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::BuiltinToolCallCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: builtin_tool_call_thread_item(event),
            completed_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::ExternalToolCallStarted(event) => Some(ProjectedEventItem::Started {
            turn_id: event.turn_id.clone(),
            item: external_tool_call_thread_item(event),
            started_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::ExternalToolCallCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: external_tool_call_thread_item(event),
            completed_at_ms: event.lifecycle_at_ms,
        }),
        EventMsg::WorkflowRunProgressCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::WorkflowRunProgress {
                id: event.id.clone(),
                event: event.event.clone().into(),
            },
            completed_at_ms: event.completed_at_ms,
        }),
        EventMsg::EventCommandEventCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::EventCommandEvent {
                id: event.id.clone(),
                subscription_id: event.event.subscription_id.clone(),
                kind: EventCommandEventKind::from(event.event.kind.clone()),
                label: event.event.label.clone(),
                command: event.event.command.clone(),
                cwd: event.event.cwd.clone(),
                line: event.event.line.clone(),
                sequence: event.event.sequence,
                exit_code: event.event.exit_code,
                signal: event.event.signal.clone(),
                message: event.event.message.clone(),
                truncated: event.event.truncated,
                created_at: event.event.created_at,
            },
            completed_at_ms: event.completed_at_ms,
        }),
        EventMsg::EventDrivenToolCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::EventDrivenTool {
                id: event.id.clone(),
                tool: event.trigger.tool.clone(),
                title: event.trigger.title.clone(),
                text: event.trigger.text.clone(),
            },
            completed_at_ms: event.completed_at_ms,
        }),
        EventMsg::InterAgentCommunicationCompleted(event) => {
            Some(ProjectedEventItem::Completed {
                turn_id: event.turn_id.clone(),
                item: thread_item_from_inter_agent_communication(
                    event.id.clone(),
                    event.communication.clone(),
                ),
                completed_at_ms: event.completed_at_ms,
            })
        }
        EventMsg::ThreadGoalUpdateCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::ThreadGoalUpdate {
                id: event.id.clone(),
                goal: thread_goal_from_update_goal(&event.goal),
                action: ThreadGoalUpdateAction::from(event.action),
                source: ThreadGoalUpdateSource::from(event.source),
                previous_status: event
                    .previous_status
                    .map(thread_goal_status_from_update_status),
            },
            completed_at_ms: event.completed_at_ms,
        }),
        _ => None,
    }
}

fn thread_item_from_turn_item(value: CoreTurnItem) -> Option<ThreadItem> {
    match value {
        CoreTurnItem::UserMessage(user) => {
            let content = user
                .content
                .into_iter()
                .map(UserInput::from)
                .collect::<Vec<_>>();
            if is_legacy_structured_user_inputs(&content) {
                return None;
            }
            Some(ThreadItem::UserMessage {
                id: user.id,
                content,
            })
        }
        CoreTurnItem::HookPrompt(hook_prompt) => Some(ThreadItem::HookPrompt {
            id: hook_prompt.id,
            fragments: hook_prompt
                .fragments
                .into_iter()
                .map(HookPromptFragment::from)
                .collect(),
        }),
        CoreTurnItem::InjectedContext(context) => Some(ThreadItem::InjectedContext {
            id: context.id,
            title: context.title,
            preview: context.preview,
            sections: context
                .sections
                .into_iter()
                .map(|section| InjectedContextSection {
                    label: section.label,
                    text: section.text,
                })
                .collect(),
        }),
        CoreTurnItem::AgentMessage(agent) => {
            let text = agent
                .content
                .into_iter()
                .map(|entry| match entry {
                    CoreAgentMessageContent::Text { text } => text,
                })
                .collect::<String>();
            if is_legacy_structured_assistant_message_text(&text) {
                return None;
            }
            Some(assistant_message_thread_item(
                agent.id,
                text,
                agent.phase,
                agent.memory_citation.map(Into::into),
            ))
        }
        CoreTurnItem::EventDrivenTool(event_driven_tool) => Some(ThreadItem::EventDrivenTool {
            id: event_driven_tool.id,
            tool: event_driven_tool.tool,
            title: event_driven_tool.title,
            text: event_driven_tool.text,
        }),
        CoreTurnItem::EventCommandEvent(event_command) => Some(ThreadItem::EventCommandEvent {
            id: event_command.id,
            subscription_id: event_command.event.subscription_id,
            kind: event_command.event.kind.into(),
            label: event_command.event.label,
            command: event_command.event.command,
            cwd: event_command.event.cwd,
            line: event_command.event.line,
            sequence: event_command.event.sequence,
            exit_code: event_command.event.exit_code,
            signal: event_command.event.signal,
            message: event_command.event.message,
            truncated: event_command.event.truncated,
            created_at: event_command.event.created_at,
        }),
        CoreTurnItem::CollabAgentMessage(collab) => Some(
            thread_item_from_inter_agent_communication(collab.id, collab.communication),
        ),
        CoreTurnItem::Plan(plan) => Some(ThreadItem::Plan {
            id: plan.id,
            text: plan.text,
        }),
        CoreTurnItem::Reasoning(reasoning) => Some(ThreadItem::Reasoning {
            id: reasoning.id,
            summary: reasoning.summary_text,
            content: reasoning.raw_content,
        }),
        CoreTurnItem::WebSearch(search) => Some(ThreadItem::WebSearch {
            id: search.id,
            query: search.query,
            action: Some(WebSearchAction::from(search.action)),
        }),
        CoreTurnItem::ImageView(image) => Some(ThreadItem::ImageView {
            id: image.id,
            path: image.path,
        }),
        CoreTurnItem::ImageGeneration(image) => Some(ThreadItem::ImageGeneration {
            id: image.id,
            status: image.status,
            revised_prompt: image.revised_prompt,
            result: image.result,
            saved_path: image.saved_path,
        }),
        CoreTurnItem::FileChange(file_change) => Some(ThreadItem::FileChange {
            id: file_change.id,
            changes: convert_patch_changes(&file_change.changes),
            status: file_change
                .status
                .as_ref()
                .map(PatchApplyStatus::from)
                .unwrap_or(PatchApplyStatus::InProgress),
        }),
        CoreTurnItem::McpToolCall(mcp) => {
            let duration_ms = mcp
                .duration
                .and_then(|duration| i64::try_from(duration.as_millis()).ok());

            Some(ThreadItem::McpToolCall {
                id: mcp.id,
                server: mcp.server,
                tool: mcp.tool,
                status: McpToolCallStatus::from(mcp.status),
                arguments: mcp.arguments,
                mcp_app_resource_uri: mcp.mcp_app_resource_uri,
                result: mcp.result.map(McpToolCallResult::from).map(Box::new),
                error: mcp.error.map(McpToolCallError::from),
                duration_ms,
            })
        }
        CoreTurnItem::ContextCompaction(compaction) => Some(ThreadItem::ContextCompaction {
            id: compaction.id,
            replacement_history: compaction
                .replacement_history
                .into_iter()
                .map(context_compaction_replacement_item_from_core)
                .collect(),
        }),
    }
}

pub fn context_compaction_replacement_item_from_core(
    item: CoreContextCompactionReplacementItem,
) -> ContextCompactionReplacementItem {
    match item {
        CoreContextCompactionReplacementItem::InjectedContext(context) => {
            ContextCompactionReplacementItem::InjectedContext {
                id: context.id,
                title: context.title,
                preview: context.preview,
                sections: context
                    .sections
                    .into_iter()
                    .map(|section| InjectedContextSection {
                        label: section.label,
                        text: section.text,
                    })
                    .collect(),
            }
        }
        CoreContextCompactionReplacementItem::UserMessage(user) => {
            ContextCompactionReplacementItem::UserMessage {
                id: user.id,
                content: user.content.into_iter().map(UserInput::from).collect(),
            }
        }
        CoreContextCompactionReplacementItem::AgentMessage(agent) => {
            let text = agent
                .content
                .into_iter()
                .map(|content| match content {
                    CoreAgentMessageContent::Text { text } => text,
                })
                .collect::<Vec<_>>()
                .join("");
            ContextCompactionReplacementItem::AgentMessage {
                id: agent.id,
                text,
                phase: agent.phase,
                memory_citation: agent.memory_citation.map(Into::into),
            }
        }
    }
}

fn command_wait_thread_item(event: &protocol::protocol::CommandWaitDisplayEvent) -> ThreadItem {
    ThreadItem::CommandWait {
        id: event.id.clone(),
        command_id: event.command_id.clone(),
        status: CommandWaitStatus::from(event.status),
        notification: event.notification.map(CommandWaitNotificationKind::from),
        exit_code: event.exit_code,
        wall_time_seconds: event.wall_time_seconds,
        wait_timeout_ms: event.wait_timeout_ms,
        created_at_ms: event.created_at_ms,
    }
}

fn builtin_tool_call_thread_item(
    event: &protocol::protocol::BuiltinToolCallDisplayEvent,
) -> ThreadItem {
    ThreadItem::BuiltinToolCall {
        id: event.id.clone(),
        tool: event.tool.clone(),
        arguments: event.arguments.clone(),
        status: match event.status {
            protocol::protocol::BuiltinToolCallStatus::InProgress => {
                DynamicToolCallStatus::InProgress
            }
            protocol::protocol::BuiltinToolCallStatus::Completed => {
                DynamicToolCallStatus::Completed
            }
            protocol::protocol::BuiltinToolCallStatus::Failed => DynamicToolCallStatus::Failed,
        },
        output: event.output.clone(),
    }
}

fn external_tool_call_thread_item(
    event: &protocol::protocol::ExternalToolCallDisplayEvent,
) -> ThreadItem {
    ThreadItem::EventDrivenToolCall {
        id: event.id.clone(),
        tool: event.tool.clone(),
        arguments: event.arguments.clone(),
        status: match event.status {
            protocol::protocol::ExternalToolCallStatus::InProgress => {
                DynamicToolCallStatus::InProgress
            }
            protocol::protocol::ExternalToolCallStatus::Completed => {
                DynamicToolCallStatus::Completed
            }
            protocol::protocol::ExternalToolCallStatus::Failed => DynamicToolCallStatus::Failed,
        },
        output: event.output.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CommandWaitNotificationKind;
    use crate::protocol::CommandWaitStatus;
    use pretty_assertions::assert_eq;
    use protocol::ThreadId;
    use protocol::models::WorkflowRunProgressEvent;
    use protocol::models::WorkflowRunProgressKind;
    use protocol::protocol::CommandExecutionNotificationDisplayEvent;
    use protocol::protocol::CommandWaitDisplayEvent;
    use protocol::protocol::CommandWriteStdinDisplayEvent;
    use protocol::protocol::WorkflowRunProgressDisplayEvent;

    #[test]
    fn command_wait_completed_projects_without_response_item() {
        let event = EventMsg::CommandWaitCompleted(CommandWaitDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            id: "wait-1".to_string(),
            command_id: "cmd-1".to_string(),
            status: protocol::models::CommandWaitStatus::Completed,
            notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
            exit_code: Some(0),
            wall_time_seconds: 1.25,
            wait_timeout_ms: 250,
            created_at_ms: 1234,
            lifecycle_at_ms: 5678,
        });

        assert_eq!(
            project_event_msg_item(&event),
            Some(ProjectedEventItem::Completed {
                turn_id: "turn-1".to_string(),
                item: ThreadItem::CommandWait {
                    id: "wait-1".to_string(),
                    command_id: "cmd-1".to_string(),
                    status: CommandWaitStatus::Completed,
                    notification: Some(CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                },
                completed_at_ms: 5678,
            }),
        );
    }

    #[test]
    fn command_write_stdin_completed_projects_without_response_item() {
        let event = EventMsg::CommandWriteStdinCompleted(CommandWriteStdinDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            id: "stdin-1".to_string(),
            command_id: "cmd-1".to_string(),
            bytes_written: 6,
            contains_newline: true,
            created_at_ms: 1234,
            completed_at_ms: 5678,
        });

        assert_eq!(
            project_event_msg_item(&event),
            Some(ProjectedEventItem::Completed {
                turn_id: "turn-1".to_string(),
                item: ThreadItem::CommandWriteStdin {
                    id: "stdin-1".to_string(),
                    command_id: "cmd-1".to_string(),
                    bytes_written: 6,
                    contains_newline: true,
                    created_at_ms: 1234,
                },
                completed_at_ms: 5678,
            }),
        );
    }

    #[test]
    fn command_notification_completed_projects_without_response_item() {
        let event = EventMsg::CommandExecutionNotificationCompleted(
            CommandExecutionNotificationDisplayEvent {
                thread_id: ThreadId::new(),
                turn_id: "turn-1".to_string(),
                id: "notify-1".to_string(),
                command_item_id: "cmd-item-1".to_string(),
                kind: protocol::models::CommandExecutionNotificationKind::Exit,
                message: "Command exited".to_string(),
                output: Some("done".to_string()),
                exit_code: Some(0),
                created_at_ms: 1234,
                completed_at_ms: 5678,
            },
        );

        assert_eq!(
            project_event_msg_item(&event),
            Some(ProjectedEventItem::Completed {
                turn_id: "turn-1".to_string(),
                item: ThreadItem::CommandExecutionNotification {
                    id: "notify-1".to_string(),
                    command_item_id: "cmd-item-1".to_string(),
                    kind: crate::protocol::CommandExecutionNotificationKind::Exit,
                    message: "Command exited".to_string(),
                    output: Some("done".to_string()),
                    exit_code: Some(0),
                    created_at_ms: 1234,
                },
                completed_at_ms: 5678,
            }),
        );
    }

    #[test]
    fn workflow_progress_completed_projects_without_response_item() {
        let progress = WorkflowRunProgressEvent {
            run_id: "run-1".to_string(),
            workflow_id: "feature-dev".to_string(),
            status: serde_json::json!({"state": "completed"}),
            runner_status: "completed".to_string(),
            kind: WorkflowRunProgressKind::Completed,
            message: "done".to_string(),
            updated_at: 1234,
        };
        let event = EventMsg::WorkflowRunProgressCompleted(WorkflowRunProgressDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            id: "workflow-1".to_string(),
            event: progress.clone(),
            completed_at_ms: 5678,
        });

        assert_eq!(
            project_event_msg_item(&event),
            Some(ProjectedEventItem::Completed {
                turn_id: "turn-1".to_string(),
                item: ThreadItem::WorkflowRunProgress {
                    id: "workflow-1".to_string(),
                    event: progress.into(),
                },
                completed_at_ms: 5678,
            }),
        );
    }
}
