use super::rate_limits::RateLimitError;
use super::response_debug_context::ResponseDebugContext;
use super::response_debug_context::extract_response_debug_context;
use super::response_debug_context::telemetry_transport_error_message;
use http::StatusCode;
use protocol::error::InvalidModelInputError;
use protocol::error::ModelInputItemReference;
use serde_json::Value;
use std::fmt;
use std::time::Duration;
use transport_client_types::TransportError;

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
    InvalidModelInput(InvalidModelInputError),
    CyberPolicy {
        message: String,
    },
    ServerOverloaded,
}

pub fn attach_invalid_model_input_source(
    details: &mut InvalidModelInputError,
    sources: &[Option<ModelInputItemReference>],
) {
    // The first attachment is performed at the layer that owns the actual
    // outbound input after compatibility filtering or websocket suffixing.
    // Higher adapters may still see the error, but must not overwrite that
    // authoritative index/source with an earlier prompt-side mapping.
    if details.input_index.is_some() {
        return;
    }
    let Some(index) = details
        .param
        .as_deref()
        .and_then(input_index_from_error_param)
    else {
        return;
    };
    details.input_index = Some(index);
    details.source = sources.get(index).cloned().flatten();
}

pub fn parse_invalid_model_input_error(body: &str) -> Option<InvalidModelInputError> {
    let parsed = serde_json::from_str::<Value>(body).ok()?;
    let error = parsed.get("error").unwrap_or(&parsed);
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    if error_type.as_deref() != Some("invalid_request_error") {
        return None;
    }
    Some(InvalidModelInputError {
        message: error
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or("Invalid request.")
            .to_string(),
        error_type,
        code: error
            .get("code")
            .and_then(Value::as_str)
            .map(str::to_string),
        param: error
            .get("param")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_index: None,
        source: None,
    })
}

fn input_index_from_error_param(param: &str) -> Option<usize> {
    let remainder = param.strip_prefix("input[")?;
    let closing = remainder.find(']')?;
    let index_text = &remainder[..closing];
    if index_text.is_empty() || !index_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let suffix = &remainder[closing + 1..];
    if suffix.is_empty() || (!suffix.starts_with('.') && !suffix.starts_with('[')) {
        return None;
    }
    index_text.parse().ok()
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
            Self::InvalidModelInput(error) => write!(f, "invalid model input: {error}"),
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
            | Self::InvalidModelInput(_)
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
        ApiError::InvalidModelInput(_) => "invalid model input".to_string(),
        ApiError::CyberPolicy { .. } => "cyber policy".to_string(),
        ApiError::ServerOverloaded => "server overloaded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use http::HeaderValue;
    use protocol::error::ModelInputItemKind;

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
    fn attaches_source_to_structured_bad_request_using_actual_input_index() {
        let mut error = ApiError::InvalidModelInput(InvalidModelInputError {
            message: "invalid historical input".to_string(),
            error_type: Some("invalid_request_error".to_string()),
            code: Some("invalid_value".to_string()),
            param: Some("input[1].arguments.outer".to_string()),
            input_index: None,
            source: None,
        });
        let target = ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "call-1".to_string(),
        };

        let ApiError::InvalidModelInput(details) = &mut error else {
            panic!("expected structured invalid model input");
        };
        attach_invalid_model_input_source(details, &[None, Some(target.clone())]);

        let ApiError::InvalidModelInput(details) = error else {
            panic!("expected structured invalid model input");
        };
        assert_eq!(details.input_index, Some(1));
        assert_eq!(details.source, Some(target));
    }

    #[test]
    fn malformed_input_param_does_not_guess_a_source() {
        let mut error = ApiError::InvalidModelInput(InvalidModelInputError {
            message: "invalid".to_string(),
            error_type: Some("invalid_request_error".to_string()),
            code: Some("invalid_value".to_string()),
            param: Some("input[-1].arguments".to_string()),
            input_index: None,
            source: None,
        });
        let target = ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "call-1".to_string(),
        };

        let ApiError::InvalidModelInput(details) = &mut error else {
            panic!("expected structured invalid model input");
        };
        attach_invalid_model_input_source(details, &[Some(target)]);

        let ApiError::InvalidModelInput(details) = error else {
            panic!("expected structured invalid model input");
        };
        assert_eq!(details.input_index, None);
        assert_eq!(details.source, None);
    }

    #[test]
    fn existing_actual_source_is_not_overwritten_by_an_earlier_mapping() {
        let actual = ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "actual-call".to_string(),
        };
        let stale = ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: "stale-call".to_string(),
        };
        let mut details = InvalidModelInputError {
            message: "invalid".to_string(),
            error_type: Some("invalid_request_error".to_string()),
            code: Some("invalid_value".to_string()),
            param: Some("input[0].arguments".to_string()),
            input_index: Some(0),
            source: Some(actual.clone()),
        };

        attach_invalid_model_input_source(&mut details, &[Some(stale)]);

        assert_eq!(details.input_index, Some(0));
        assert_eq!(details.source, Some(actual));
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
