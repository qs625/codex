use std::pin::Pin;
use std::sync::Arc;
use std::sync::OnceLock;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use transport_client::ByteStream;
use transport_client::StreamResponse;
use eventsource_stream::Eventsource;
use futures::Stream;
use futures::StreamExt;
use model_service_api::ApiError;
use model_service_api::ResponseEvent;
use model_service_api::ResponseStream;
use model_service_api::SseTelemetry;
use model_service_api::rate_limits::parse_all_rate_limits;
use protocol::models::ResponseItem;
use protocol::protocol::ModelVerification;
use protocol::protocol::TokenUsage;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

use crate::transport_telemetry::summarize_sse_poll;

const X_REASONING_INCLUDED_HEADER: &str = "x-reasoning-included";
const OPENAI_MODEL_HEADER: &str = "openai-model";
const REQUEST_ID_HEADER: &str = "x-request-id";
const TRUSTED_ACCESS_FOR_CYBER_VERIFICATION: &str = "trusted_access_for_cyber";

struct ReceiverResponseStream {
    rx_event: mpsc::Receiver<Result<ResponseEvent, ApiError>>,
}

impl Stream for ReceiverResponseStream {
    type Item = Result<ResponseEvent, ApiError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx_event.poll_recv(cx)
    }
}

pub(crate) fn response_stream_from_receiver(
    rx_event: mpsc::Receiver<Result<ResponseEvent, ApiError>>,
    upstream_request_id: Option<String>,
) -> ResponseStream {
    ResponseStream::new(ReceiverResponseStream { rx_event }, upstream_request_id)
}

pub(crate) fn spawn_response_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    turn_state: Option<Arc<OnceLock<String>>>,
) -> ResponseStream {
    let rate_limit_snapshots = parse_all_rate_limits(&stream_response.headers);
    let models_etag = stream_response
        .headers
        .get("X-Models-Etag")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let server_model = stream_response
        .headers
        .get(OPENAI_MODEL_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let reasoning_included = stream_response
        .headers
        .get(X_REASONING_INCLUDED_HEADER)
        .is_some();
    let upstream_request_id = stream_response
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if let Some(turn_state) = turn_state.as_ref()
        && let Some(header_value) = stream_response
            .headers
            .get("x-codex-turn-state")
            .and_then(|value| value.to_str().ok())
    {
        let _ = turn_state.set(header_value.to_string());
    }

    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        if let Some(model) = server_model {
            let _ = tx_event.send(Ok(ResponseEvent::ServerModel(model))).await;
        }
        for snapshot in rate_limit_snapshots {
            let _ = tx_event.send(Ok(ResponseEvent::RateLimits(snapshot))).await;
        }
        if let Some(etag) = models_etag {
            let _ = tx_event.send(Ok(ResponseEvent::ModelsEtag(etag))).await;
        }
        if reasoning_included {
            let _ = tx_event
                .send(Ok(ResponseEvent::ServerReasoningIncluded(true)))
                .await;
        }
        process_sse(stream_response.bytes, tx_event, idle_timeout, telemetry).await;
    });

    response_stream_from_receiver(rx_event, upstream_request_id)
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Error {
    r#type: Option<String>,
    code: Option<String>,
    message: Option<String>,
    plan_type: Option<String>,
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ResponseCompleted {
    id: String,
    #[serde(default)]
    usage: Option<ResponseCompletedUsage>,
    #[serde(default)]
    end_turn: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ResponseCompletedUsage {
    input_tokens: i64,
    input_tokens_details: Option<ResponseCompletedInputTokensDetails>,
    output_tokens: i64,
    output_tokens_details: Option<ResponseCompletedOutputTokensDetails>,
    total_tokens: i64,
}

impl From<ResponseCompletedUsage> for TokenUsage {
    fn from(value: ResponseCompletedUsage) -> Self {
        TokenUsage {
            input_tokens: value.input_tokens,
            cached_input_tokens: value
                .input_tokens_details
                .map(|details| details.cached_tokens)
                .unwrap_or(0),
            output_tokens: value.output_tokens,
            reasoning_output_tokens: value
                .output_tokens_details
                .map(|details| details.reasoning_tokens)
                .unwrap_or(0),
            total_tokens: value.total_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResponseCompletedInputTokensDetails {
    cached_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ResponseCompletedOutputTokensDetails {
    reasoning_tokens: i64,
}

#[derive(Deserialize, Debug)]
pub(crate) struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    headers: Option<Value>,
    metadata: Option<Value>,
    response: Option<Value>,
    item: Option<Value>,
    item_id: Option<String>,
    call_id: Option<String>,
    delta: Option<String>,
    summary_index: Option<i64>,
    content_index: Option<i64>,
}

impl ResponsesStreamEvent {
    pub(crate) fn response_model(&self) -> Option<String> {
        let response_headers_model = self
            .response
            .as_ref()
            .and_then(|response| response.get("headers"))
            .and_then(header_openai_model_value_from_json);

        match response_headers_model {
            Some(model) => Some(model),
            None => self
                .headers
                .as_ref()
                .and_then(header_openai_model_value_from_json),
        }
    }

    pub(crate) fn model_verifications(&self) -> Option<Vec<ModelVerification>> {
        if self.kind != "response.metadata" {
            return None;
        }

        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.get("openai_verification_recommendation"))
            .and_then(model_verifications_from_json_value)
    }
}

fn header_openai_model_value_from_json(value: &Value) -> Option<String> {
    let headers = value.as_object()?;
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("openai-model") || name.eq_ignore_ascii_case("x-openai-model")
        {
            json_value_as_string(value)
        } else {
            None
        }
    })
}

fn model_verifications_from_json_value(value: &Value) -> Option<Vec<ModelVerification>> {
    let verifications = value
        .as_array()
        .map(|items| {
            let mut verifications = Vec::new();
            for verification in items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_model_verification)
            {
                if !verifications.contains(&verification) {
                    verifications.push(verification);
                }
            }
            verifications
        })
        .unwrap_or_default();

    if verifications.is_empty() {
        None
    } else {
        Some(verifications)
    }
}

fn parse_model_verification(value: &str) -> Option<ModelVerification> {
    match value {
        TRUSTED_ACCESS_FOR_CYBER_VERIFICATION => Some(ModelVerification::TrustedAccessForCyber),
        _ => None,
    }
}

fn json_value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(items) => items.first().and_then(json_value_as_string),
        _ => None,
    }
}

pub(crate) enum ResponsesEventError {
    Api(ApiError),
}

impl ResponsesEventError {
    pub(crate) fn into_api_error(self) -> ApiError {
        match self {
            Self::Api(error) => error,
        }
    }
}

pub(crate) fn process_responses_event(
    event: ResponsesStreamEvent,
) -> Result<Option<ResponseEvent>, ResponsesEventError> {
    match event.kind.as_str() {
        "response.output_item.done" => {
            if let Some(item_val) = event.item {
                if let Ok(item) = serde_json::from_value::<ResponseItem>(item_val) {
                    return Ok(Some(ResponseEvent::OutputItemDone(item)));
                }
                debug!("failed to parse ResponseItem from output_item.done");
            }
        }
        "response.output_text.delta" => {
            if let Some(delta) = event.delta {
                return Ok(Some(ResponseEvent::OutputTextDelta(delta)));
            }
        }
        "response.custom_tool_call_input.delta" => {
            if let (Some(delta), Some(item_id)) =
                (event.delta, event.item_id.clone().or(event.call_id.clone()))
            {
                return Ok(Some(ResponseEvent::ToolCallInputDelta {
                    item_id,
                    call_id: event.call_id,
                    delta,
                }));
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let (Some(delta), Some(summary_index)) = (event.delta, event.summary_index) {
                return Ok(Some(ResponseEvent::ReasoningSummaryDelta {
                    delta,
                    summary_index,
                }));
            }
        }
        "response.reasoning_text.delta" => {
            if let (Some(delta), Some(content_index)) = (event.delta, event.content_index) {
                return Ok(Some(ResponseEvent::ReasoningContentDelta {
                    delta,
                    content_index,
                }));
            }
        }
        "response.created" => {
            if event.response.is_some() {
                return Ok(Some(ResponseEvent::Created {}));
            }
        }
        "response.failed" => {
            if let Some(response_val) = event.response {
                let mut response_error = ApiError::Stream("response.failed event received".into());
                if let Some(error) = response_val.get("error")
                    && let Ok(error) = serde_json::from_value::<Error>(error.clone())
                {
                    if is_context_window_error(&error) {
                        response_error = ApiError::ContextWindowExceeded;
                    } else if is_quota_exceeded_error(&error) {
                        response_error = ApiError::QuotaExceeded;
                    } else if is_usage_not_included(&error) {
                        response_error = ApiError::UsageNotIncluded;
                    } else if is_cyber_policy_error(&error) {
                        let message = cyber_policy_message(error.message);
                        response_error = ApiError::CyberPolicy { message };
                    } else if is_invalid_prompt_error(&error) {
                        let message = error
                            .message
                            .unwrap_or_else(|| "Invalid request.".to_string());
                        response_error = ApiError::InvalidRequest { message };
                    } else if is_server_overloaded_error(&error) {
                        response_error = ApiError::ServerOverloaded;
                    } else {
                        let delay = try_parse_retry_after(&error);
                        let message = error.message.unwrap_or_default();
                        response_error = ApiError::Retryable { message, delay };
                    }
                }
                return Err(ResponsesEventError::Api(response_error));
            }
            return Err(ResponsesEventError::Api(ApiError::Stream(
                "response.failed event received".into(),
            )));
        }
        "response.incomplete" => {
            let reason = event.response.as_ref().and_then(|response| {
                response
                    .get("incomplete_details")
                    .and_then(|details| details.get("reason"))
                    .and_then(Value::as_str)
            });
            let reason = reason.unwrap_or("unknown");
            return Err(ResponsesEventError::Api(ApiError::Stream(format!(
                "Incomplete response returned, reason: {reason}"
            ))));
        }
        "response.completed" => {
            if let Some(response_val) = event.response {
                match serde_json::from_value::<ResponseCompleted>(response_val) {
                    Ok(response) => {
                        return Ok(Some(ResponseEvent::Completed {
                            response_id: response.id,
                            token_usage: response.usage.map(Into::into),
                            end_turn: response.end_turn,
                        }));
                    }
                    Err(error) => {
                        let error = format!("failed to parse ResponseCompleted: {error}");
                        debug!("{error}");
                        return Err(ResponsesEventError::Api(ApiError::Stream(error)));
                    }
                }
            }
        }
        "response.output_item.added" => {
            if let Some(item_val) = event.item {
                if let Ok(item) = serde_json::from_value::<ResponseItem>(item_val) {
                    return Ok(Some(ResponseEvent::OutputItemAdded(item)));
                }
                debug!("failed to parse ResponseItem from output_item.added");
            }
        }
        "response.reasoning_summary_part.added" => {
            if let Some(summary_index) = event.summary_index {
                return Ok(Some(ResponseEvent::ReasoningSummaryPartAdded {
                    summary_index,
                }));
            }
        }
        _ => {
            trace!("unhandled responses event: {}", event.kind);
        }
    }

    Ok(None)
}

async fn process_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
) {
    let mut stream = stream.eventsource();
    let mut response_error: Option<ApiError> = None;
    let mut last_server_model: Option<String> = None;

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(telemetry) = telemetry.as_ref() {
            let event = summarize_sse_poll(&response);
            telemetry.on_sse_poll(event.as_ref(), start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(error))) => {
                debug!("SSE Error: {error:#}");
                let _ = tx_event
                    .send(Err(ApiError::Stream(error.to_string())))
                    .await;
                return;
            }
            Ok(None) => {
                let error = response_error.unwrap_or(ApiError::Stream(
                    "stream closed before response.completed".into(),
                ));
                let _ = tx_event.send(Err(error)).await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("SSE event: {}", &sse.data);

        let event: ResponsesStreamEvent = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(error) => {
                debug!("Failed to parse SSE event: {error}, data: {}", &sse.data);
                continue;
            }
        };
        let model_verifications = event.model_verifications();

        if let Some(model) = event.response_model()
            && last_server_model.as_deref() != Some(model.as_str())
        {
            if tx_event
                .send(Ok(ResponseEvent::ServerModel(model.clone())))
                .await
                .is_err()
            {
                return;
            }
            last_server_model = Some(model);
        }
        if let Some(verifications) = model_verifications
            && tx_event
                .send(Ok(ResponseEvent::ModelVerifications(verifications)))
                .await
                .is_err()
        {
            return;
        }

        match process_responses_event(event) {
            Ok(Some(event)) => {
                let is_completed = matches!(event, ResponseEvent::Completed { .. });
                if tx_event.send(Ok(event)).await.is_err() {
                    return;
                }
                if is_completed {
                    return;
                }
            }
            Ok(None) => {}
            Err(error) => {
                response_error = Some(error.into_api_error());
            }
        }
    }
}

fn try_parse_retry_after(error: &Error) -> Option<Duration> {
    if error.code.as_deref() != Some("rate_limit_exceeded") {
        return None;
    }

    let re =
        regex_lite::Regex::new(r"try again in ([0-9]+(?:\.[0-9]+)?)\s*(ms|s|seconds?)").ok()?;
    if let Some(message) = &error.message
        && let Some(captures) = re.captures(message)
    {
        let seconds = captures.get(1);
        let unit = captures.get(2);

        if let (Some(value), Some(unit)) = (seconds, unit) {
            let value = value.as_str().parse::<f64>().ok()?;
            let unit = unit.as_str().to_ascii_lowercase();

            if unit == "s" || unit.starts_with("second") {
                return Some(Duration::from_secs_f64(value));
            } else if unit == "ms" {
                return Some(Duration::from_millis(value as u64));
            }
        }
    }
    None
}

fn is_context_window_error(error: &Error) -> bool {
    error.code.as_deref() == Some("context_length_exceeded")
}

fn is_quota_exceeded_error(error: &Error) -> bool {
    error.code.as_deref() == Some("insufficient_quota")
}

fn is_usage_not_included(error: &Error) -> bool {
    error.code.as_deref() == Some("usage_not_included")
}

fn is_invalid_prompt_error(error: &Error) -> bool {
    error.code.as_deref() == Some("invalid_prompt")
}

fn is_server_overloaded_error(error: &Error) -> bool {
    matches!(
        error.code.as_deref(),
        Some("server_is_overloaded" | "slow_down")
    )
}

fn is_cyber_policy_error(error: &Error) -> bool {
    error.code.as_deref() == Some("cyber_policy")
}

fn cyber_policy_message(message: Option<String>) -> String {
    message.unwrap_or_else(|| {
        "This request has been flagged for possible cybersecurity risk.".to_string()
    })
}
