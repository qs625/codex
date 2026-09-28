use std::sync::Arc;

use app_server_protocol::AttestationGenerateParams;
use app_server_protocol::AttestationGenerateResponse;
use app_server_protocol::ServerRequestPayload;
use axum::http::HeaderValue;
use serde::Serialize;
use thread_service::AttestationContext;
use thread_service::AttestationProvider;
use thread_service::GenerateAttestationFuture;
use tokio::time::Duration;
use tokio::time::timeout;
use tracing::warn;

use crate::outgoing_message::OutgoingMessageSender;
use crate::thread_state::ThreadStateManager;

const ATTESTATION_GENERATE_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) fn app_server_attestation_provider(
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
) -> Arc<dyn AttestationProvider> {
    Arc::new(AppServerAttestationProvider {
        outgoing,
        thread_state_manager,
    })
}

struct AppServerAttestationProvider {
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
}

impl std::fmt::Debug for AppServerAttestationProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppServerAttestationProvider")
            .finish()
    }
}

impl AttestationProvider for AppServerAttestationProvider {
    fn header_for_request(&self, context: AttestationContext) -> GenerateAttestationFuture<'_> {
        let outgoing = self.outgoing.clone();
        let thread_state_manager = self.thread_state_manager.clone();
        Box::pin(async move {
            request_attestation_header_value_with_timeout(
                outgoing,
                thread_state_manager,
                context.thread_id,
                ATTESTATION_GENERATE_TIMEOUT,
            )
            .await
            .and_then(|value| HeaderValue::from_bytes(value.as_bytes()).ok())
        })
    }
}

async fn request_attestation_header_value_with_timeout(
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
    thread_id: protocol::ThreadId,
    timeout_duration: Duration,
) -> Option<String> {
    request_attestation_outcome_with_timeout(
        outgoing,
        thread_state_manager,
        thread_id,
        timeout_duration,
    )
    .await
    .header_value()
}

async fn request_attestation_outcome_with_timeout(
    outgoing: Arc<OutgoingMessageSender>,
    thread_state_manager: ThreadStateManager,
    thread_id: protocol::ThreadId,
    timeout_duration: Duration,
) -> AppServerAttestationOutcome {
    let connection_id = thread_state_manager
        .first_attestation_capable_connection_for_thread(thread_id)
        .await;
    let Some(connection_id) = connection_id else {
        return AppServerAttestationOutcome::Unavailable;
    };

    let connection_ids = [connection_id];
    let (request_id, rx) = outgoing
        .send_request_to_connections(
            Some(&connection_ids),
            ServerRequestPayload::AttestationGenerate(AttestationGenerateParams {}),
            /*thread_id*/ None,
        )
        .await;

    let result = match timeout(timeout_duration, rx).await {
        Ok(Ok(Ok(result))) => result,
        Ok(Ok(Err(err))) => {
            warn!(
                code = err.code,
                message = %err.message,
                "attestation generation request failed"
            );
            return AppServerAttestationOutcome::AppServerFailure(
                AppServerAttestationStatus::RequestFailed,
            );
        }
        Ok(Err(err)) => {
            warn!("attestation generation request canceled: {err}");
            return AppServerAttestationOutcome::AppServerFailure(
                AppServerAttestationStatus::RequestCanceled,
            );
        }
        Err(_) => {
            let _canceled = outgoing.cancel_request(&request_id).await;
            warn!(
                timeout_seconds = timeout_duration.as_secs(),
                "attestation generation request timed out"
            );
            return AppServerAttestationOutcome::AppServerFailure(
                AppServerAttestationStatus::Timeout,
            );
        }
    };

    AppServerAttestationOutcome::from_client_response(result)
}

#[derive(Debug, PartialEq, Eq)]
enum AppServerAttestationOutcome {
    ClientToken(String),
    AppServerFailure(AppServerAttestationStatus),
    Unavailable,
}

impl AppServerAttestationOutcome {
    fn from_client_response(result: serde_json::Value) -> Self {
        match serde_json::from_value::<AttestationGenerateResponse>(result) {
            Ok(response) => Self::ClientToken(response.token),
            Err(err) => {
                warn!("failed to deserialize attestation generation response: {err}");
                Self::AppServerFailure(AppServerAttestationStatus::MalformedResponse)
            }
        }
    }

    fn header_value(&self) -> Option<String> {
        match self {
            Self::ClientToken(token) => app_server_attestation_header_value(
                AppServerAttestationStatus::Ok,
                Some(token.as_str()),
            ),
            Self::AppServerFailure(status) => {
                app_server_attestation_header_value(*status, /*token*/ None)
            }
            Self::Unavailable => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppServerAttestationStatus {
    Ok,
    Timeout,
    RequestFailed,
    RequestCanceled,
    MalformedResponse,
}

impl AppServerAttestationStatus {
    const fn code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Timeout => 1,
            Self::RequestFailed => 2,
            Self::RequestCanceled => 3,
            Self::MalformedResponse => 4,
        }
    }
}

#[derive(Serialize)]
struct AppServerAttestationEnvelope<'a> {
    v: u8,
    s: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    t: Option<&'a str>,
}

fn app_server_attestation_header_value(
    status: AppServerAttestationStatus,
    token: Option<&str>,
) -> Option<String> {
    serde_json::to_string(&AppServerAttestationEnvelope {
        v: 1,
        s: status.code(),
        t: token,
    })
    .map_err(|err| warn!("failed to serialize app-server attestation envelope: {err}"))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::AppServerAttestationOutcome;
    use super::AppServerAttestationStatus;
    use super::app_server_attestation_header_value;
    use app_server_protocol::AttestationGenerateResponse;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn app_server_attestation_header_value_wraps_opaque_client_payloads() {
        assert_eq!(
            app_server_attestation_header_value(
                AppServerAttestationStatus::Ok,
                Some("v1.opaque-client-payload"),
            ),
            Some(r#"{"v":1,"s":0,"t":"v1.opaque-client-payload"}"#.to_string())
        );
    }

    #[test]
    fn app_server_attestation_header_value_reports_app_server_failures() {
        assert_eq!(
            app_server_attestation_header_value(
                AppServerAttestationStatus::Timeout,
                /*token*/ None,
            ),
            Some(r#"{"v":1,"s":1}"#.to_string())
        );
        assert_eq!(
            app_server_attestation_header_value(
                AppServerAttestationStatus::RequestFailed,
                /*token*/ None,
            ),
            Some(r#"{"v":1,"s":2}"#.to_string())
        );
        assert_eq!(
            app_server_attestation_header_value(
                AppServerAttestationStatus::RequestCanceled,
                /*token*/ None,
            ),
            Some(r#"{"v":1,"s":3}"#.to_string())
        );
        assert_eq!(
            app_server_attestation_header_value(
                AppServerAttestationStatus::MalformedResponse,
                /*token*/ None
            ),
            Some(r#"{"v":1,"s":4}"#.to_string())
        );
    }

    #[test]
    fn app_server_attestation_outcome_projects_client_token() {
        let result = serde_json::to_value(AttestationGenerateResponse {
            token: "v1.client-token".to_string(),
        })
        .expect("serialize response");

        let outcome = AppServerAttestationOutcome::from_client_response(result);

        assert_eq!(
            outcome,
            AppServerAttestationOutcome::ClientToken("v1.client-token".to_string())
        );
        assert_eq!(
            outcome.header_value(),
            Some(r#"{"v":1,"s":0,"t":"v1.client-token"}"#.to_string())
        );
    }

    #[test]
    fn app_server_attestation_outcome_projects_malformed_response() {
        let outcome = AppServerAttestationOutcome::from_client_response(json!({
            "unexpected": true
        }));

        assert_eq!(
            outcome,
            AppServerAttestationOutcome::AppServerFailure(
                AppServerAttestationStatus::MalformedResponse
            )
        );
        assert_eq!(outcome.header_value(), Some(r#"{"v":1,"s":4}"#.to_string()));
    }

    #[test]
    fn app_server_attestation_outcome_omits_header_when_unavailable() {
        assert_eq!(
            AppServerAttestationOutcome::Unavailable.header_value(),
            None
        );
    }
}
