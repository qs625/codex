use crate::protocol::response_item_projection::project_structured_response_item;
use crate::protocol::v2::ThreadItem;
use codex_protocol::protocol::EventMsg;

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
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::v2::CommandWaitNotificationKind;
    use crate::protocol::v2::CommandWaitStatus;
    use codex_protocol::ThreadId;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::protocol::ResponseItemCompletedEvent;
    use codex_protocol::protocol::ResponseItemStartedEvent;
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
}
