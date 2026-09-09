use super::*;

pub(super) async fn handle_realtime_conversation_started(
    conversation_id: &ThreadId,
    outgoing: &ThreadScopedOutgoingMessageSender,
    event: protocol::protocol::RealtimeConversationStartedEvent,
) {
    let notification = ThreadRealtimeStartedNotification {
        thread_id: conversation_id.to_string(),
        realtime_session_id: event.realtime_session_id,
        version: event.version,
    };
    outgoing
        .send_server_notification(ServerNotification::ThreadRealtimeStarted(notification))
        .await;
}

pub(super) async fn handle_realtime_conversation_sdp(
    conversation_id: &ThreadId,
    outgoing: &ThreadScopedOutgoingMessageSender,
    event: protocol::protocol::RealtimeConversationSdpEvent,
) {
    let notification = ThreadRealtimeSdpNotification {
        thread_id: conversation_id.to_string(),
        sdp: event.sdp,
    };
    outgoing
        .send_server_notification(ServerNotification::ThreadRealtimeSdp(notification))
        .await;
}

pub(super) async fn handle_realtime_conversation_event(
    conversation_id: &ThreadId,
    outgoing: &ThreadScopedOutgoingMessageSender,
    event: protocol::protocol::RealtimeConversationRealtimeEvent,
) {
    match event.payload {
        RealtimeEvent::SessionUpdated { .. } => {}
        RealtimeEvent::InputAudioSpeechStarted(event) => {
            let notification = ThreadRealtimeItemAddedNotification {
                thread_id: conversation_id.to_string(),
                item: serde_json::json!({
                    "type": "input_audio_buffer.speech_started",
                    "item_id": event.item_id,
                }),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeItemAdded(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::InputTranscriptDelta(event) => {
            let notification = ThreadRealtimeTranscriptDeltaNotification {
                thread_id: conversation_id.to_string(),
                role: "user".to_string(),
                delta: event.delta,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeTranscriptDelta(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::InputTranscriptDone(event) => {
            let notification = ThreadRealtimeTranscriptDoneNotification {
                thread_id: conversation_id.to_string(),
                role: "user".to_string(),
                text: event.text,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeTranscriptDone(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::OutputTranscriptDelta(event) => {
            let notification = ThreadRealtimeTranscriptDeltaNotification {
                thread_id: conversation_id.to_string(),
                role: "assistant".to_string(),
                delta: event.delta,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeTranscriptDelta(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::OutputTranscriptDone(event) => {
            let notification = ThreadRealtimeTranscriptDoneNotification {
                thread_id: conversation_id.to_string(),
                role: "assistant".to_string(),
                text: event.text,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeTranscriptDone(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::AudioOut(audio) => {
            let notification = ThreadRealtimeOutputAudioDeltaNotification {
                thread_id: conversation_id.to_string(),
                audio: audio.into(),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeOutputAudioDelta(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::ResponseCreated(_) => {}
        RealtimeEvent::ResponseCancelled(event) => {
            let notification = ThreadRealtimeItemAddedNotification {
                thread_id: conversation_id.to_string(),
                item: serde_json::json!({
                    "type": "response.cancelled",
                    "response_id": event.response_id,
                }),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeItemAdded(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::ResponseDone(_) => {}
        RealtimeEvent::ConversationItemAdded(item) => {
            let notification = ThreadRealtimeItemAddedNotification {
                thread_id: conversation_id.to_string(),
                item,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeItemAdded(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::ConversationItemDone { .. } | RealtimeEvent::NoopRequested(_) => {}
        RealtimeEvent::HandoffRequested(handoff) => {
            let notification = ThreadRealtimeItemAddedNotification {
                thread_id: conversation_id.to_string(),
                item: serde_json::json!({
                    "type": "handoff_request",
                    "handoff_id": handoff.handoff_id,
                    "item_id": handoff.item_id,
                    "input_transcript": handoff.input_transcript,
                    "active_transcript": handoff.active_transcript,
                }),
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeItemAdded(
                    notification,
                ))
                .await;
        }
        RealtimeEvent::Error(message) => {
            let notification = ThreadRealtimeErrorNotification {
                thread_id: conversation_id.to_string(),
                message,
            };
            outgoing
                .send_server_notification(ServerNotification::ThreadRealtimeError(notification))
                .await;
        }
    }
}

pub(super) async fn handle_realtime_conversation_closed(
    conversation_id: &ThreadId,
    outgoing: &ThreadScopedOutgoingMessageSender,
    event: protocol::protocol::RealtimeConversationClosedEvent,
) {
    let notification = ThreadRealtimeClosedNotification {
        thread_id: conversation_id.to_string(),
        reason: event.reason,
    };
    outgoing
        .send_server_notification(ServerNotification::ThreadRealtimeClosed(notification))
        .await;
}
