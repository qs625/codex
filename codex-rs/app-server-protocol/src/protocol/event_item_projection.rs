use crate::protocol::response_item_projection::project_structured_response_item;
use crate::protocol::response_item_projection::thread_goal_from_update_goal;
use crate::protocol::response_item_projection::thread_goal_status_from_update_status;
use crate::protocol::response_item_projection::thread_item_from_inter_agent_communication;
use crate::protocol::v2::CommandExecutionNotificationKind;
use crate::protocol::v2::CommandWaitNotificationKind;
use crate::protocol::v2::CommandWaitStatus;
use crate::protocol::v2::EventCommandEventKind;
use crate::protocol::v2::ThreadItem;
use crate::protocol::v2::ThreadGoalUpdateAction;
use crate::protocol::v2::ThreadGoalUpdateSource;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentOperation;

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
        EventMsg::ItemStarted(event) => Some(ProjectedEventItem::Started {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::from(event.item.clone()),
            started_at_ms: event.started_at_ms,
        }),
        EventMsg::ItemCompleted(event) => Some(ProjectedEventItem::Completed {
            turn_id: event.turn_id.clone(),
            item: ThreadItem::from(event.item.clone()),
            completed_at_ms: event.completed_at_ms,
        }),
        EventMsg::ResponseItemStarted(event) => {
            let fallback_id = || {
                format!(
                    "{}-response-item-started-{}",
                    event.turn_id, event.started_at_ms
                )
            };
            project_structured_response_item(&event.item, fallback_id).map(|item| {
                ProjectedEventItem::Started {
                    turn_id: event.turn_id.clone(),
                    item,
                    started_at_ms: event.started_at_ms,
                }
            })
        }
        EventMsg::ResponseItemCompleted(event) => {
            let fallback_id = || {
                format!(
                    "{}-response-item-completed-{}",
                    event.turn_id, event.completed_at_ms
                )
            };
            project_structured_response_item(&event.item, fallback_id).map(|item| {
                ProjectedEventItem::Completed {
                    turn_id: event.turn_id.clone(),
                    item,
                    completed_at_ms: event.completed_at_ms,
                }
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
            if matches!(event.communication.operation, InterAgentOperation::Unknown) {
                return None;
            }
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

fn command_wait_thread_item(
    event: &codex_protocol::protocol::CommandWaitDisplayEvent,
) -> ThreadItem {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v2::CommandWaitNotificationKind;
    use crate::protocol::v2::CommandWaitStatus;
    use codex_protocol::ThreadId;
    use codex_protocol::models::WorkflowRunProgressEvent;
    use codex_protocol::models::WorkflowRunProgressKind;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::CommandExecutionNotificationDisplayEvent;
    use codex_protocol::protocol::CommandWaitDisplayEvent;
    use codex_protocol::protocol::CommandWriteStdinDisplayEvent;
    use codex_protocol::protocol::ResponseItemCompletedEvent;
    use codex_protocol::protocol::ResponseItemStartedEvent;
    use codex_protocol::protocol::WorkflowRunProgressDisplayEvent;
    use pretty_assertions::assert_eq;

    fn command_wait_response_item(
        status: codex_protocol::models::CommandWaitStatus,
    ) -> ResponseItem {
        ResponseItem::CommandWait {
            id: Some("wait-1".to_string()),
            command_id: "cmd-1".to_string(),
            status,
            notification: Some(codex_protocol::models::CommandWaitNotificationKind::Exit),
            exit_code: Some(0),
            wall_time_seconds: 1.25,
            wait_timeout_ms: 250,
            created_at_ms: 1234,
        }
    }

    #[test]
    fn response_item_started_projects_command_wait_to_thread_item() {
        let event = EventMsg::ResponseItemStarted(ResponseItemStartedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            item: command_wait_response_item(codex_protocol::models::CommandWaitStatus::Running),
            started_at_ms: 5678,
        });

        assert_eq!(
            project_event_msg_item(&event),
            Some(ProjectedEventItem::Started {
                turn_id: "turn-1".to_string(),
                item: ThreadItem::CommandWait {
                    id: "wait-1".to_string(),
                    command_id: "cmd-1".to_string(),
                    status: CommandWaitStatus::Running,
                    notification: Some(CommandWaitNotificationKind::Exit),
                    exit_code: Some(0),
                    wall_time_seconds: 1.25,
                    wait_timeout_ms: 250,
                    created_at_ms: 1234,
                },
                started_at_ms: 5678,
            }),
        );
    }

    #[test]
    fn response_item_completed_projects_command_wait_to_thread_item() {
        let event = EventMsg::ResponseItemCompleted(ResponseItemCompletedEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            item: command_wait_response_item(codex_protocol::models::CommandWaitStatus::Completed),
            completed_at_ms: 5678,
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
    fn command_wait_completed_projects_without_response_item() {
        let event = EventMsg::CommandWaitCompleted(CommandWaitDisplayEvent {
            thread_id: ThreadId::new(),
            turn_id: "turn-1".to_string(),
            id: "wait-1".to_string(),
            command_id: "cmd-1".to_string(),
            status: codex_protocol::models::CommandWaitStatus::Completed,
            notification: Some(codex_protocol::models::CommandWaitNotificationKind::Exit),
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
                kind: codex_protocol::models::CommandExecutionNotificationKind::Exit,
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
                    kind: crate::protocol::v2::CommandExecutionNotificationKind::Exit,
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
