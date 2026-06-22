use crate::error::ApiError;
use codex_api_types::SseEventTelemetry;
pub use codex_api_types::SseTelemetry;
use codex_api_types::WebsocketEventTelemetry;
pub use codex_api_types::WebsocketTelemetry;
use codex_client::Request;
use codex_client::Response;
use codex_client::RetryPolicy;
use codex_client::StreamResponse;
use codex_client::TransportError;
use codex_client::run_with_retry;
use codex_client_types::RequestTelemetry;
use codex_protocol::models::ResponseItem;
use http::StatusCode;
use std::future::Future;
use std::sync::Arc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Error;
use tokio_tungstenite::tungstenite::Message;

pub(crate) fn summarize_sse_poll(
    result: &Result<
        Option<
            Result<eventsource_stream::Event, eventsource_stream::EventStreamError<TransportError>>,
        >,
        tokio::time::error::Elapsed,
    >,
) -> Option<SseEventTelemetry> {
    match result {
        Ok(Some(Ok(sse))) => {
            if sse.data.trim() == "[DONE]" {
                return Some(SseEventTelemetry::succeeded(sse.event.clone()));
            }

            match serde_json::from_str::<serde_json::Value>(&sse.data) {
                Ok(error) if sse.event == "response.failed" => Some(SseEventTelemetry::failed(
                    Some(sse.event.clone()),
                    error.to_string(),
                )),
                Ok(content) if sse.event == "response.output_item.done" => {
                    match serde_json::from_value::<ResponseItem>(content) {
                        Ok(_) => Some(SseEventTelemetry::succeeded(sse.event.clone())),
                        Err(_) => Some(SseEventTelemetry::failed(
                            Some(sse.event.clone()),
                            "failed to parse response.output_item.done",
                        )),
                    }
                }
                Ok(_) => Some(SseEventTelemetry::succeeded(sse.event.clone())),
                Err(error) => Some(SseEventTelemetry::failed(
                    Some(sse.event.clone()),
                    error.to_string(),
                )),
            }
        }
        Ok(Some(Err(error))) => Some(SseEventTelemetry::failed(
            /*kind*/ None,
            error.to_string(),
        )),
        Ok(None) => None,
        Err(_) => Some(SseEventTelemetry::failed(
            /*kind*/ None,
            "idle timeout waiting for SSE",
        )),
    }
}

pub(crate) fn summarize_websocket_poll(
    result: &Result<Option<Result<Message, Error>>, ApiError>,
) -> Option<WebsocketEventTelemetry> {
    match result {
        Ok(Some(Ok(message))) => match message {
            Message::Text(text) => match serde_json::from_str::<serde_json::Value>(text) {
                Ok(value) => {
                    let kind = value
                        .get("type")
                        .and_then(|value| value.as_str())
                        .map(str::to_string);
                    if kind.as_deref() == Some("response.failed") {
                        let error_message = value
                            .get("response")
                            .and_then(|value| value.get("error"))
                            .map(serde_json::Value::to_string)
                            .unwrap_or_else(|| "response.failed event received".to_string());
                        WebsocketEventTelemetry::failed(kind, error_message, Some(value))
                    } else {
                        WebsocketEventTelemetry::succeeded(kind, Some(value))
                    }
                }
                Err(error) => WebsocketEventTelemetry::failed(
                    Some("parse_error".to_string()),
                    error.to_string(),
                    /*payload*/ None,
                ),
            },
            Message::Binary(_) => WebsocketEventTelemetry::failed(
                /*kind*/ None,
                "unexpected binary websocket event",
                /*payload*/ None,
            ),
            Message::Ping(_) | Message::Pong(_) => return None,
            Message::Close(_) => WebsocketEventTelemetry::failed(
                /*kind*/ None,
                "websocket closed by server before response.completed",
                /*payload*/ None,
            ),
            Message::Frame(_) => WebsocketEventTelemetry::failed(
                /*kind*/ None,
                "unexpected websocket frame",
                /*payload*/ None,
            ),
        },
        Ok(Some(Err(error))) => WebsocketEventTelemetry::failed(
            /*kind*/ None,
            error.to_string(),
            /*payload*/ None,
        ),
        Ok(None) => WebsocketEventTelemetry::failed(
            /*kind*/ None,
            "stream closed before response.completed",
            /*payload*/ None,
        ),
        Err(error) => WebsocketEventTelemetry::failed(
            /*kind*/ None,
            error.to_string(),
            /*payload*/ None,
        ),
    }
    .into()
}

pub(crate) trait WithStatus {
    fn status(&self) -> StatusCode;
}

fn http_status(err: &TransportError) -> Option<StatusCode> {
    match err {
        TransportError::Http { status, .. } => Some(*status),
        _ => None,
    }
}

impl WithStatus for Response {
    fn status(&self) -> StatusCode {
        self.status
    }
}

impl WithStatus for StreamResponse {
    fn status(&self) -> StatusCode {
        self.status
    }
}

pub(crate) async fn run_with_request_telemetry<T, F, Fut>(
    policy: RetryPolicy,
    telemetry: Option<Arc<dyn RequestTelemetry>>,
    make_request: impl FnMut() -> Request,
    send: F,
) -> Result<T, TransportError>
where
    T: WithStatus,
    F: Clone + Fn(Request) -> Fut,
    Fut: Future<Output = Result<T, TransportError>>,
{
    // Wraps `run_with_retry` to attach per-attempt request telemetry for both
    // unary and streaming HTTP calls.
    run_with_retry(policy, make_request, move |req, attempt| {
        let telemetry = telemetry.clone();
        let send = send.clone();
        async move {
            let start = Instant::now();
            let result = send(req).await;
            if let Some(t) = telemetry.as_ref() {
                let (status, err) = match &result {
                    Ok(resp) => (Some(resp.status()), None),
                    Err(err) => (http_status(err), Some(err)),
                };
                t.on_request(attempt, status, err, start.elapsed());
            }
            result
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use eventsource_stream::Event;

    #[test]
    fn summarizes_sse_poll_success() {
        let result = Ok(Some(Ok(Event {
            event: "response.created".to_string(),
            data: "{}".to_string(),
            id: String::new(),
            retry: None,
        })));

        assert_eq!(
            summarize_sse_poll(&result),
            Some(SseEventTelemetry::succeeded("response.created"))
        );
    }

    #[test]
    fn summarizes_sse_poll_response_failed() {
        let result = Ok(Some(Ok(Event {
            event: "response.failed".to_string(),
            data: r#"{"error":{"message":"bad request"}}"#.to_string(),
            id: String::new(),
            retry: None,
        })));

        assert_eq!(
            summarize_sse_poll(&result),
            Some(SseEventTelemetry::failed(
                Some("response.failed".to_string()),
                r#"{"error":{"message":"bad request"}}"#,
            ))
        );
    }

    #[test]
    fn summarizes_websocket_poll_timing_payload() {
        let result = Ok(Some(Ok(Message::Text(
            r#"{"type":"responsesapi.websocket_timing","timing_metrics":{"engine_service_total_ms":12}}"#
                .into(),
        ))));

        assert_eq!(
            summarize_websocket_poll(&result),
            Some(WebsocketEventTelemetry::succeeded(
                Some("responsesapi.websocket_timing".to_string()),
                Some(serde_json::json!({
                    "type": "responsesapi.websocket_timing",
                    "timing_metrics": {
                        "engine_service_total_ms": 12,
                    },
                })),
            ))
        );
    }

    #[test]
    fn summarizes_websocket_ping_as_ignored() {
        let result = Ok(Some(Ok(Message::Ping(Vec::new().into()))));

        assert_eq!(summarize_websocket_poll(&result), None);
    }
}
