use crate::rate_limits::RateLimitError;
use codex_client::TransportError;
use codex_response_debug_context::ResponseDebugContext;
use codex_response_debug_context::extract_response_debug_context;
use codex_response_debug_context::telemetry_transport_error_message;
use http::StatusCode;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("api error {status}: {message}")]
    Api { status: StatusCode, message: String },
    #[error("stream error: {0}")]
    Stream(String),
    #[error("context window exceeded")]
    ContextWindowExceeded,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("usage not included")]
    UsageNotIncluded,
    #[error("retryable error: {message}")]
    Retryable {
        message: String,
        delay: Option<Duration>,
    },
    #[error("rate limit: {0}")]
    RateLimit(String),
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("cyber policy: {message}")]
    CyberPolicy { message: String },
    #[error("server overloaded")]
    ServerOverloaded,
}

impl From<RateLimitError> for ApiError {
    fn from(err: RateLimitError) -> Self {
        Self::RateLimit(err.to_string())
    }
}

pub fn extract_response_debug_context_from_api_error(error: &ApiError) -> ResponseDebugContext {
    match error {
        ApiError::Transport(transport) => extract_response_debug_context(transport),
        _ => ResponseDebugContext::default(),
    }
}

pub fn telemetry_api_error_message(error: &ApiError) -> String {
    match error {
        ApiError::Transport(transport) => telemetry_transport_error_message(transport),
        ApiError::Api { status, .. } => format!("api error {}", status.as_u16()),
        ApiError::Stream(err) => err.to_string(),
        ApiError::ContextWindowExceeded => "context window exceeded".to_string(),
        ApiError::QuotaExceeded => "quota exceeded".to_string(),
        ApiError::UsageNotIncluded => "usage not included".to_string(),
        ApiError::Retryable { .. } => "retryable error".to_string(),
        ApiError::RateLimit(_) => "rate limit".to_string(),
        ApiError::InvalidRequest { .. } => "invalid request".to_string(),
        ApiError::CyberPolicy { .. } => "cyber policy".to_string(),
        ApiError::ServerOverloaded => "server overloaded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use http::HeaderValue;

    #[test]
    fn api_error_debug_context_extracts_transport_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("req-api"));
        headers.insert("cf-ray", HeaderValue::from_static("ray-api"));

        let context = extract_response_debug_context_from_api_error(&ApiError::Transport(
            TransportError::Http {
                status: StatusCode::UNAUTHORIZED,
                url: None,
                headers: Some(headers),
                body: None,
            },
        ));

        assert_eq!(
            context,
            ResponseDebugContext {
                request_id: Some("req-api".to_string()),
                cf_ray: Some("ray-api".to_string()),
                auth_error: None,
                auth_error_code: None,
            }
        );
    }

    #[test]
    fn telemetry_api_error_message_omits_http_body() {
        let error = ApiError::Transport(TransportError::Http {
            status: StatusCode::UNAUTHORIZED,
            url: Some("https://example.test".to_string()),
            headers: None,
            body: Some("secret body".to_string()),
        });

        assert_eq!(telemetry_api_error_message(&error), "http 401");
    }

    #[test]
    fn telemetry_api_error_message_preserves_stream_detail() {
        let error = ApiError::Stream("socket closed".to_string());

        assert_eq!(telemetry_api_error_message(&error), "socket closed");
    }
}
