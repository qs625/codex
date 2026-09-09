use std::sync::Arc;

use app_server_protocol::ClientRelaunchMode;
use app_server_protocol::ClientRelaunchRequestedNotification;
use app_server_protocol::ServerNotification;
use codex_tool_service::HostLifecycleToolRuntime;
use codex_tool_service::HostRelaunchMode;
use codex_tool_service::HostRelaunchRequest;
use codex_tool_service::HostRelaunchResult;
use codex_tool_service::HostRelaunchStatus;
use tool_service_api::ToolServiceFuture;

use crate::outgoing_message::OutgoingMessageSender;
use crate::thread_state::ThreadStateManager;

const RESUME_STRATEGY: &str = "expected_restart_intent";
const MAX_REQUEST_ID_BYTES: usize = 256;

pub(crate) struct AppServerHostLifecycleToolRuntime {
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
}

impl AppServerHostLifecycleToolRuntime {
    pub(crate) fn new(
        outgoing: Arc<OutgoingMessageSender>,
        thread_state_manager: ThreadStateManager,
    ) -> Self {
        Self {
            outgoing,
            thread_state_manager,
        }
    }
}

impl HostLifecycleToolRuntime for AppServerHostLifecycleToolRuntime {
    fn request_client_relaunch<'a>(
        &'a self,
        request: HostRelaunchRequest,
    ) -> ToolServiceFuture<'a, HostRelaunchResult> {
        Box::pin(async move {
            if request.request_id.is_empty() || request.request_id.len() > MAX_REQUEST_ID_BYTES {
                return host_relaunch_result(
                    &request,
                    HostRelaunchStatus::Failed,
                    false,
                    "Invalid runtime refresh request identifier.".to_string(),
                    Some(format!(
                        "requestId must contain between 1 and {MAX_REQUEST_ID_BYTES} UTF-8 bytes"
                    )),
                );
            }
            let Some(connection_id) = self.thread_state_manager.host_lifecycle_connection().await
            else {
                return host_relaunch_result(
                    &request,
                    HostRelaunchStatus::Unsupported,
                    false,
                    "No registered Host lifecycle consumer is connected.".to_string(),
                    request.reason.clone(),
                );
            };

            let notification =
                ServerNotification::ClientRelaunchRequested(ClientRelaunchRequestedNotification {
                    request_id: request.request_id.clone(),
                    mode: client_relaunch_mode(&request.mode),
                    reason: request.reason.clone(),
                    requested_by_thread_id: request.requested_by_thread_id.clone(),
                    resume_strategy: RESUME_STRATEGY.to_string(),
                });
            if let Err(error) = self
                .outgoing
                .send_server_notification_to_connection(connection_id, notification)
                .await
            {
                return host_relaunch_result(
                    &request,
                    HostRelaunchStatus::Failed,
                    false,
                    "Failed to deliver runtime refresh request to the registered Host.".to_string(),
                    Some(error),
                );
            }

            host_relaunch_result(
                &request,
                HostRelaunchStatus::Accepted,
                true,
                format!(
                    "Runtime refresh request ({}) was delivered to the registered Host.",
                    host_relaunch_mode_wire_value(&request.mode),
                ),
                request.reason.clone(),
            )
        })
    }
}

fn host_relaunch_result(
    request: &HostRelaunchRequest,
    status: HostRelaunchStatus,
    accepted: bool,
    message: String,
    reason: Option<String>,
) -> HostRelaunchResult {
    HostRelaunchResult {
        request_id: request.request_id.clone(),
        status,
        accepted,
        relaunching: false,
        requested_mode: request.mode.clone(),
        executed_mode: None,
        message,
        reason,
        resume_strategy: RESUME_STRATEGY.to_string(),
    }
}

fn host_relaunch_mode_wire_value(mode: &HostRelaunchMode) -> &'static str {
    match mode {
        HostRelaunchMode::Hot => "hot",
        HostRelaunchMode::Full => "full",
    }
}

fn client_relaunch_mode(mode: &HostRelaunchMode) -> ClientRelaunchMode {
    match mode {
        HostRelaunchMode::Hot => ClientRelaunchMode::Hot,
        HostRelaunchMode::Full => ClientRelaunchMode::Full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use crate::thread_state::ConnectionCapabilities;
    use codex_analytics::AnalyticsEventsClient;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn request_client_relaunch_is_unsupported_without_registered_host() {
        let (tx, _rx) = mpsc::channel::<OutgoingEnvelope>(1);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            AnalyticsEventsClient::disabled(),
        ));
        let runtime = AppServerHostLifecycleToolRuntime::new(outgoing, ThreadStateManager::new());

        let result = runtime
            .request_client_relaunch(host_request(HostRelaunchMode::Hot))
            .await;

        assert_eq!(result.status, HostRelaunchStatus::Unsupported);
        assert!(!result.accepted);
    }

    #[tokio::test]
    async fn request_client_relaunch_targets_registered_host_without_waiting_for_response() {
        let (tx, mut rx) = mpsc::channel::<OutgoingEnvelope>(1);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            AnalyticsEventsClient::disabled(),
        ));
        let manager = ThreadStateManager::new();
        let connection_id = crate::outgoing_message::ConnectionId(7);
        manager
            .connection_initialized(
                connection_id,
                ConnectionCapabilities {
                    host_lifecycle_eligible: true,
                    ..Default::default()
                },
            )
            .await;
        manager
            .register_host_lifecycle_connection(connection_id)
            .await
            .expect("register Host");
        let runtime = AppServerHostLifecycleToolRuntime::new(outgoing, manager);

        let result = runtime
            .request_client_relaunch(host_request(HostRelaunchMode::Full))
            .await;

        assert_eq!(result.status, HostRelaunchStatus::Accepted);
        assert!(result.accepted);
        assert_eq!(result.requested_mode, HostRelaunchMode::Full);

        let envelope = rx.recv().await.expect("targeted notification");
        let OutgoingEnvelope::ToConnection {
            connection_id: actual_connection_id,
            message:
                OutgoingMessage::AppServerNotification(ServerNotification::ClientRelaunchRequested(
                    notification,
                )),
            write_complete_tx: None,
        } = envelope
        else {
            panic!("expected targeted client relaunch notification");
        };
        assert_eq!(actual_connection_id, connection_id);
        assert_eq!(notification.request_id, "restart-call");
        assert_eq!(notification.mode, ClientRelaunchMode::Full);
        assert_eq!(
            notification.requested_by_thread_id.as_deref(),
            Some("thread-1")
        );
        assert_eq!(notification.resume_strategy, RESUME_STRATEGY);
    }

    #[tokio::test]
    async fn request_client_relaunch_reports_targeted_enqueue_failure() {
        let (tx, rx) = mpsc::channel::<OutgoingEnvelope>(1);
        drop(rx);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            AnalyticsEventsClient::disabled(),
        ));
        let manager = ThreadStateManager::new();
        let connection_id = crate::outgoing_message::ConnectionId(7);
        manager
            .connection_initialized(
                connection_id,
                ConnectionCapabilities {
                    host_lifecycle_eligible: true,
                    ..Default::default()
                },
            )
            .await;
        manager
            .register_host_lifecycle_connection(connection_id)
            .await
            .expect("register Host");
        let runtime = AppServerHostLifecycleToolRuntime::new(outgoing, manager);

        let result = runtime
            .request_client_relaunch(host_request(HostRelaunchMode::Hot))
            .await;

        assert_eq!(result.status, HostRelaunchStatus::Failed);
        assert!(!result.accepted);
    }

    #[tokio::test]
    async fn request_client_relaunch_enforces_utf8_byte_request_id_limit() {
        let (tx, mut rx) = mpsc::channel::<OutgoingEnvelope>(1);
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            AnalyticsEventsClient::disabled(),
        ));
        let manager = ThreadStateManager::new();
        let connection_id = crate::outgoing_message::ConnectionId(7);
        manager
            .connection_initialized(
                connection_id,
                ConnectionCapabilities {
                    host_lifecycle_eligible: true,
                    ..Default::default()
                },
            )
            .await;
        manager
            .register_host_lifecycle_connection(connection_id)
            .await
            .expect("register Host");
        let runtime = AppServerHostLifecycleToolRuntime::new(outgoing, manager);
        let mut boundary_request = host_request(HostRelaunchMode::Hot);
        boundary_request.request_id = "é".repeat(MAX_REQUEST_ID_BYTES / 2);
        let boundary_result = runtime.request_client_relaunch(boundary_request).await;
        assert_eq!(boundary_result.status, HostRelaunchStatus::Accepted);
        assert!(boundary_result.accepted);
        rx.recv().await.expect("boundary request notification");

        let mut oversized_request = host_request(HostRelaunchMode::Hot);
        oversized_request.request_id = "é".repeat(MAX_REQUEST_ID_BYTES / 2 + 1);
        let result = runtime.request_client_relaunch(oversized_request).await;

        assert_eq!(result.status, HostRelaunchStatus::Failed);
        assert!(!result.accepted);
        assert!(rx.try_recv().is_err());
    }

    fn host_request(mode: HostRelaunchMode) -> HostRelaunchRequest {
        HostRelaunchRequest {
            request_id: "restart-call".to_string(),
            mode,
            reason: Some("runtime update".to_string()),
            requested_by_thread_id: Some("thread-1".to_string()),
        }
    }
}
