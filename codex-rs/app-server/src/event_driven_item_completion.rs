use crate::outgoing_message::ThreadScopedOutgoingMessageSender;
use codex_app_server_protocol::ItemCompletedNotification;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ThreadItem;
use codex_protocol::ThreadId;

pub(crate) async fn maybe_emit_event_driven_tool_trigger_item_completed(
    conversation_id: ThreadId,
    turn_id: &str,
    item: &codex_protocol::models::ResponseItem,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    match item {
        codex_protocol::models::ResponseItem::EventCommandEvent { id, event } => {
            let item_id = id.clone().unwrap_or_else(|| event.stable_item_id());
            emit_event_command_event_completed(
                conversation_id,
                turn_id,
                item_id,
                event.clone(),
                outgoing,
            )
            .await;
            return;
        }
        codex_protocol::models::ResponseItem::EventDrivenTool { id, trigger } => {
            let item_id = id
                .clone()
                .unwrap_or_else(|| event_driven_tool_trigger_item_id(turn_id, trigger));
            emit_event_driven_tool_completed(
                conversation_id,
                turn_id,
                item_id,
                trigger.clone(),
                outgoing,
            )
            .await;
            return;
        }
        _ => {}
    }

    let codex_protocol::models::ResponseItem::Message { content, id, .. } = item else {
        return;
    };

    if let Some(event) =
        codex_protocol::event_command::EventCommandEvent::parse_message_content(content)
    {
        let item_id = id.clone().unwrap_or_else(|| event.stable_item_id());
        emit_event_command_event_completed(conversation_id, turn_id, item_id, event, outgoing)
            .await;
        return;
    }

    let Some(trigger) =
        codex_protocol::event_driven_tool::EventDrivenToolTrigger::parse_message_content(content)
    else {
        return;
    };
    let item_id = id
        .clone()
        .unwrap_or_else(|| event_driven_tool_trigger_item_id(turn_id, &trigger));

    emit_event_driven_tool_completed(conversation_id, turn_id, item_id, trigger, outgoing).await;
}

async fn emit_event_command_event_completed(
    conversation_id: ThreadId,
    turn_id: &str,
    item_id: String,
    event: codex_protocol::event_command::EventCommandEvent,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let notification = ItemCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.to_string(),
        completed_at_ms: now_unix_timestamp_ms(),
        item: ThreadItem::EventCommandEvent {
            id: item_id,
            subscription_id: event.subscription_id,
            kind: event.kind.into(),
            label: event.label,
            command: event.command,
            cwd: event.cwd,
            line: event.line,
            sequence: event.sequence,
            exit_code: event.exit_code,
            signal: event.signal,
            message: event.message,
            truncated: event.truncated,
            created_at: event.created_at,
        },
    };
    outgoing
        .send_server_notification(ServerNotification::ItemCompleted(notification))
        .await;
}

async fn emit_event_driven_tool_completed(
    conversation_id: ThreadId,
    turn_id: &str,
    item_id: String,
    trigger: codex_protocol::event_driven_tool::EventDrivenToolTrigger,
    outgoing: &ThreadScopedOutgoingMessageSender,
) {
    let notification = ItemCompletedNotification {
        thread_id: conversation_id.to_string(),
        turn_id: turn_id.to_string(),
        completed_at_ms: now_unix_timestamp_ms(),
        item: ThreadItem::EventDrivenTool {
            id: item_id,
            tool: trigger.tool,
            title: trigger.title,
            text: trigger.text,
        },
    };
    outgoing
        .send_server_notification(ServerNotification::ItemCompleted(notification))
        .await;
}

fn event_driven_tool_trigger_item_id(
    turn_id: &str,
    trigger: &codex_protocol::event_driven_tool::EventDrivenToolTrigger,
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&trigger.tool, &mut hasher);
    std::hash::Hash::hash(&trigger.title, &mut hasher);
    std::hash::Hash::hash(&trigger.text, &mut hasher);
    let hash = std::hash::Hasher::finish(&hasher);
    format!("{turn_id}:event-driven-tool:{hash:016x}")
}

fn now_unix_timestamp_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing_message::ConnectionId;
    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use crate::outgoing_message::OutgoingMessageSender;
    use crate::transport::CHANNEL_CAPACITY;
    use anyhow::Result;
    use anyhow::anyhow;
    use anyhow::bail;
    use codex_app_server_protocol::ServerNotification;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    async fn recv_broadcast_message(
        rx: &mut mpsc::Receiver<OutgoingEnvelope>,
    ) -> Result<OutgoingMessage> {
        let envelope = rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("channel closed before broadcast message"))?;
        match envelope {
            OutgoingEnvelope::Broadcast { message } => Ok(message),
            OutgoingEnvelope::ToConnection { message, .. } => Ok(message),
        }
    }

    #[tokio::test]
    async fn event_driven_tool_trigger_message_emits_item_completed() -> Result<()> {
        let conversation_id = ThreadId::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let trigger = codex_protocol::event_driven_tool::EventDrivenToolTrigger {
            tool: "fs_subscribe".to_string(),
            title: "File watch triggered".to_string(),
            text: "build.log changed".to_string(),
        };
        let expected_item_id = event_driven_tool_trigger_item_id("turn-1", &trigger);

        maybe_emit_event_driven_tool_trigger_item_completed(
            conversation_id,
            "turn-1",
            &trigger.to_response_item(),
            &outgoing,
        )
        .await;

        let completed = recv_broadcast_message(&mut rx).await?;
        match completed {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(
                    payload.item,
                    ThreadItem::EventDrivenTool {
                        id: expected_item_id,
                        tool: "fs_subscribe".to_string(),
                        title: "File watch triggered".to_string(),
                        text: "build.log changed".to_string(),
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(
            rx.try_recv().is_err(),
            "event-driven trigger message should emit exactly once"
        );
        Ok(())
    }

    #[tokio::test]
    async fn typed_event_driven_tool_trigger_emits_item_completed() -> Result<()> {
        let conversation_id = ThreadId::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let trigger = codex_protocol::event_driven_tool::EventDrivenToolTrigger {
            tool: "fs_subscribe".to_string(),
            title: "File watch triggered".to_string(),
            text: "build.log changed".to_string(),
        };

        maybe_emit_event_driven_tool_trigger_item_completed(
            conversation_id,
            "turn-1",
            &codex_protocol::models::ResponseItem::EventDrivenTool {
                id: Some("typed-trigger-item".to_string()),
                trigger,
            },
            &outgoing,
        )
        .await;

        let completed = recv_broadcast_message(&mut rx).await?;
        match completed {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(
                    payload.item,
                    ThreadItem::EventDrivenTool {
                        id: "typed-trigger-item".to_string(),
                        tool: "fs_subscribe".to_string(),
                        title: "File watch triggered".to_string(),
                        text: "build.log changed".to_string(),
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(
            rx.try_recv().is_err(),
            "typed event-driven trigger should emit exactly once"
        );
        Ok(())
    }

    #[tokio::test]
    async fn typed_event_command_event_emits_item_completed() -> Result<()> {
        let conversation_id = ThreadId::new();
        let (tx, mut rx) = mpsc::channel(CHANNEL_CAPACITY);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        let outgoing = ThreadScopedOutgoingMessageSender::new(
            outgoing,
            vec![ConnectionId(1)],
            ThreadId::new(),
        );
        let event = codex_protocol::event_command::EventCommandEvent {
            subscription_id: "sub-command".to_string(),
            kind: codex_protocol::event_command::EventCommandEventKind::Output,
            label: Some("build log".to_string()),
            command: "tail -f /tmp/build.log".to_string(),
            cwd: Some("/repo".to_string()),
            line: Some("changed:/tmp/build.log".to_string()),
            sequence: Some(1),
            exit_code: None,
            signal: None,
            message: None,
            truncated: false,
            created_at: 1,
        };

        maybe_emit_event_driven_tool_trigger_item_completed(
            conversation_id,
            "turn-1",
            &codex_protocol::models::ResponseItem::EventCommandEvent {
                id: Some("typed-event-command".to_string()),
                event,
            },
            &outgoing,
        )
        .await;

        let completed = recv_broadcast_message(&mut rx).await?;
        match completed {
            OutgoingMessage::AppServerNotification(ServerNotification::ItemCompleted(payload)) => {
                assert_eq!(
                    payload.item,
                    ThreadItem::EventCommandEvent {
                        id: "typed-event-command".to_string(),
                        subscription_id: "sub-command".to_string(),
                        kind: codex_app_server_protocol::EventCommandEventKind::Output,
                        label: Some("build log".to_string()),
                        command: "tail -f /tmp/build.log".to_string(),
                        cwd: Some("/repo".to_string()),
                        line: Some("changed:/tmp/build.log".to_string()),
                        sequence: Some(1),
                        exit_code: None,
                        signal: None,
                        message: None,
                        truncated: false,
                        created_at: 1,
                    }
                );
            }
            other => bail!("unexpected message: {other:?}"),
        }

        assert!(
            rx.try_recv().is_err(),
            "typed event command event should emit exactly once"
        );
        Ok(())
    }
}
