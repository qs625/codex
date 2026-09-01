use std::sync::Arc;

use app_server_protocol::ClientRelaunchRequestedNotification;
use app_server_protocol::ServerNotification;
use codex_tool_service::HostLifecycleToolRuntime;
use codex_tool_service::HostRelaunchRequest;
use codex_tool_service::HostRelaunchResult;
use codex_tool_service::HostRelaunchStatus;
use tool_service_api::ToolServiceFuture;

use crate::outgoing_message::OutgoingMessageSender;

const RESUME_STRATEGY: &str = "client_bootstrap_autoresume";

pub(crate) struct AppServerHostLifecycleToolRuntime {
    outgoing: Arc<OutgoingMessageSender>,
}

impl AppServerHostLifecycleToolRuntime {
    pub(crate) fn new(outgoing: Arc<OutgoingMessageSender>) -> Self {
        Self { outgoing }
    }
}

impl HostLifecycleToolRuntime for AppServerHostLifecycleToolRuntime {
    fn request_client_relaunch<'a>(
        &'a self,
        request: HostRelaunchRequest,
    ) -> ToolServiceFuture<'a, HostRelaunchResult> {
        Box::pin(async move {
            self.outgoing
                .send_server_notification(ServerNotification::ClientRelaunchRequested(
                    ClientRelaunchRequestedNotification {
                        reason: request.reason.clone(),
                        requested_by_thread_id: request.requested_by_thread_id.clone(),
                        resume_strategy: RESUME_STRATEGY.to_string(),
                    },
                ))
                .await;

            HostRelaunchResult {
                status: HostRelaunchStatus::Accepted,
                accepted: true,
                relaunching: true,
                message: "Client relaunch request was submitted. Continuation happens after client bootstrap autoresume restores eligible interrupted sessions.".to_string(),
                reason: request.reason,
                resume_strategy: RESUME_STRATEGY.to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use codex_analytics::AnalyticsEventsClient;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn request_client_relaunch_broadcasts_typed_notification() {
        let (tx, mut rx) = mpsc::channel::<OutgoingEnvelope>(1);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            AnalyticsEventsClient::disabled(),
        ));
        let runtime = AppServerHostLifecycleToolRuntime::new(outgoing);

        let result = runtime
            .request_client_relaunch(HostRelaunchRequest {
                reason: Some("runtime update".to_string()),
                requested_by_thread_id: Some("thread-1".to_string()),
            })
            .await;

        assert_eq!(result.status, HostRelaunchStatus::Accepted);
        assert!(result.accepted);
        assert!(result.relaunching);
        assert_eq!(result.reason.as_deref(), Some("runtime update"));
        assert_eq!(result.resume_strategy, RESUME_STRATEGY);

        let envelope = rx.recv().await.expect("notification envelope");
        let OutgoingEnvelope::Broadcast {
            message:
                OutgoingMessage::AppServerNotification(ServerNotification::ClientRelaunchRequested(
                    notification,
                )),
        } = envelope
        else {
            panic!("expected client relaunch broadcast notification");
        };
        assert_eq!(notification.reason.as_deref(), Some("runtime update"));
        assert_eq!(
            notification.requested_by_thread_id.as_deref(),
            Some("thread-1")
        );
        assert_eq!(notification.resume_strategy, RESUME_STRATEGY);
    }
}
