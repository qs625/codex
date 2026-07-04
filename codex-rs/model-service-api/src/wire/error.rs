use super::rate_limits::RateLimitError;
use super::response_debug_context::ResponseDebugContext;
use super::response_debug_context::extract_response_debug_context;
use super::response_debug_context::telemetry_transport_error_message;
use transport_client_types::TransportError;
use http::StatusCode;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum ApiError {
    Transport(TransportError),
    Api {
        status: StatusCode,
        message: String,
    },
    Stream(String),
    ContextWindowExceeded,
    QuotaExceeded,
    UsageNotIncluded,
    Retryable {
        message: String,
        delay: Option<Duration>,
    },
    RateLimit(String),
    InvalidRequest {
        message: String,
    },
    CyberPolicy {
        message: String,
    },
    ServerOverloaded,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(err) => err.fmt(f),
            Self::Api { status, message } => write!(f, "api error {status}: {message}"),
            Self::Stream(message) => write!(f, "stream error: {message}"),
            Self::ContextWindowExceeded => write!(f, "context window exceeded"),
            Self::QuotaExceeded => write!(f, "quota exceeded"),
            Self::UsageNotIncluded => write!(f, "usage not included"),
            Self::Retryable { message, .. } => write!(f, "retryable error: {message}"),
            Self::RateLimit(message) => write!(f, "rate limit: {message}"),
            Self::InvalidRequest { message } => write!(f, "invalid request: {message}"),
            Self::CyberPolicy { message } => write!(f, "cyber policy: {message}"),
            Self::ServerOverloaded => write!(f, "server overloaded"),
        }
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(err) => Some(err),
            Self::Api { .. }
            | Self::Stream(_)
            | Self::ContextWindowExceeded
            | Self::QuotaExceeded
            | Self::UsageNotIncluded
            | Self::Retryable { .. }
            | Self::RateLimit(_)
            | Self::InvalidRequest { .. }
            | Self::CyberPolicy { .. }
            | Self::ServerOverloaded => None,
        }
    }
}

impl From<TransportError> for ApiError {
    fn from(err: TransportError) -> Self {
        Self::Transport(err)
    }
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
