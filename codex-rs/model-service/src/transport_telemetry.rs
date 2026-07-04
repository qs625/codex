use std::future::Future;
use std::sync::Arc;

use transport_client::Request;
use transport_client::Response;
use transport_client::RetryPolicy;
use transport_client::StreamResponse;
use transport_client::TransportError;
use transport_client::run_with_retry;
use transport_client_types::RequestTelemetry;
use eventsource_stream::Event;
use http::StatusCode;
use model_service_api::ApiError;
use model_service_api::SseEventTelemetry;
use model_service_api::WebsocketEventTelemetry;
use protocol::models::ResponseItem;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;

pub(crate) fn summarize_sse_poll(
    result: &Result<
        Option<Result<Event, eventsource_stream::EventStreamError<TransportError>>>,
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
        Ok(Some(Err(error))) => Some(SseEventTelemetry::failed(None, error.to_string())),
        Ok(None) => None,
        Err(_) => Some(SseEventTelemetry::failed(
            None,
            "idle timeout waiting for SSE",
        )),
    }
}

pub(crate) fn summarize_websocket_poll(
    result: &Result<Option<Result<Message, WsError>>, ApiError>,
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

fn http_status(error: &TransportError) -> Option<StatusCode> {
    match error {
        TransportError::Http { status, .. } => Some(*status),
        _ => None,
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
    run_with_retry(policy, make_request, move |request, attempt| {
        let telemetry = telemetry.clone();
        let send = send.clone();
        async move {
            let start = Instant::now();
            let result = send(request).await;
            if let Some(telemetry) = telemetry.as_ref() {
                let (status, error) = match &result {
                    Ok(response) => (Some(response.status()), None),
                    Err(error) => (http_status(error), Some(error)),
                };
                telemetry.on_request(attempt, status, error, start.elapsed());
            }
            result
        }
    })
    .await
}
