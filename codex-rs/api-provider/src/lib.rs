use codex_client_types::Request;
use codex_client_types::RequestCompression;
use codex_client_types::RetryOn;
use codex_client_types::RetryPolicy;
use codex_client_types::TransportError;
use http::HeaderMap;
use http::HeaderValue;
use http::Method;
use http::header::HeaderMap as ApiHeaderMap;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Boxed future returned by API auth providers.
pub type AuthProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Error returned while applying authentication to an outbound request.
#[derive(Debug)]
pub enum AuthError {
    Build(String),
    Transient(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(message) => write!(f, "request auth build error: {message}"),
            Self::Transient(message) => write!(f, "transient auth error: {message}"),
        }
    }
}

impl std::error::Error for AuthError {}

impl From<AuthError> for TransportError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Build(message) => TransportError::Build(message),
            AuthError::Transient(message) => TransportError::Network(message),
        }
    }
}

/// Applies authentication to API requests.
///
/// Header-only providers should implement `add_auth_headers`. Providers that
/// need to sign complete request bodies may override `apply_auth`; the returned
/// request is authoritative and must be sent instead of the input request.
pub trait AuthProvider: Send + Sync {
    /// Adds any auth headers that are available without request body access.
    ///
    /// Implementations should be cheap and non-blocking. This method is also
    /// used by telemetry and non-HTTP request paths.
    fn add_auth_headers(&self, headers: &mut HeaderMap);

    /// Returns any auth headers that are available without request body access.
    fn to_auth_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        self.add_auth_headers(&mut headers);
        headers
    }

    /// Applies auth to a complete outbound request and returns the request to send.
    fn apply_auth(&self, request: Request) -> AuthProviderFuture<'_, Result<Request, AuthError>> {
        Box::pin(async move {
            let mut request = request;
            self.add_auth_headers(&mut request.headers);
            Ok(request)
        })
    }
}

/// Shared auth handle passed through API clients.
pub type SharedAuthProvider = Arc<dyn AuthProvider>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthHeaderTelemetry {
    pub attached: bool,
    pub name: Option<&'static str>,
}

pub fn auth_header_telemetry(auth: &dyn AuthProvider) -> AuthHeaderTelemetry {
    let mut headers = HeaderMap::new();
    auth.add_auth_headers(&mut headers);
    let name = headers
        .contains_key(http::header::AUTHORIZATION)
        .then_some("authorization");
    AuthHeaderTelemetry {
        attached: name.is_some(),
        name,
    }
}

/// High-level retry configuration for a provider.
///
/// This is converted into a `RetryPolicy` used by `codex-client` to drive
/// transport-level retries for both unary and streaming calls.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_attempts: u64,
    pub base_delay: Duration,
    pub retry_429: bool,
    pub retry_5xx: bool,
    pub retry_transport: bool,
}

impl RetryConfig {
    pub fn to_policy(&self) -> RetryPolicy {
        RetryPolicy {
            max_attempts: self.max_attempts,
            base_delay: self.base_delay,
            retry_on: RetryOn {
                retry_429: self.retry_429,
                retry_5xx: self.retry_5xx,
                retry_transport: self.retry_transport,
            },
        }
    }
}

/// HTTP endpoint configuration used to talk to a concrete API deployment.
///
/// Encapsulates base URL, default headers, query params, retry policy, stream
/// idle timeout, and helper methods for building requests.
#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub query_params: Option<HashMap<String, String>>,
    pub headers: ApiHeaderMap,
    pub retry: RetryConfig,
    pub stream_idle_timeout: Duration,
}

impl Provider {
    pub fn url_for_path(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        let mut url = if path.is_empty() {
            base.to_string()
        } else {
            format!("{base}/{path}")
        };

        if let Some(params) = &self.query_params
            && !params.is_empty()
        {
            let qs = params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&qs);
        }

        url
    }

    pub fn build_request(&self, method: Method, path: &str) -> Request {
        Request {
            method,
            url: self.url_for_path(path),
            headers: self.headers.clone(),
            body: None,
            compression: RequestCompression::None,
            timeout: None,
        }
    }

    pub fn is_azure_responses_endpoint(&self) -> bool {
        is_azure_responses_provider(&self.name, Some(&self.base_url))
    }
}

pub fn is_azure_responses_provider(name: &str, base_url: Option<&str>) -> bool {
    if name.eq_ignore_ascii_case("azure") {
        true
    } else if let Some(base_url) = base_url {
        matches_azure_responses_base_url(base_url)
    } else {
        false
    }
}

fn matches_azure_responses_base_url(base_url: &str) -> bool {
    let base_url = base_url.to_ascii_lowercase();
    const AZURE_MARKERS: [&str; 6] = [
        "openai.azure.",
        "cognitiveservices.azure.",
        "aoai.azure.",
        "azure-api.",
        "azurefd.",
        "windows.net/openai",
    ];
    AZURE_MARKERS.iter().any(|marker| base_url.contains(marker))
}

pub fn build_session_headers(session_id: Option<String>, thread_id: Option<String>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(id) = session_id {
        insert_header(&mut headers, "session-id", &id);
    }
    if let Some(id) = thread_id {
        insert_header(&mut headers, "thread-id", &id);
    }
    headers
}

pub fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(header_name), Ok(header_value)) = (
        name.parse::<http::HeaderName>(),
        HeaderValue::from_str(value),
    ) {
        headers.insert(header_name, header_value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestAuth;

    impl AuthProvider for TestAuth {
        fn add_auth_headers(&self, headers: &mut HeaderMap) {
            headers.insert(
                http::header::AUTHORIZATION,
                HeaderValue::from_static("Bearer token"),
            );
        }
    }

    #[test]
    fn auth_header_telemetry_reports_authorization_header() {
        assert_eq!(
            auth_header_telemetry(&TestAuth),
            AuthHeaderTelemetry {
                attached: true,
                name: Some("authorization"),
            }
        );
    }

    #[test]
    fn detects_azure_responses_base_urls() {
        let positive_cases = [
            "https://foo.openai.azure.com/openai",
            "https://foo.openai.azure.us/openai/deployments/bar",
            "https://foo.cognitiveservices.azure.cn/openai",
            "https://foo.aoai.azure.com/openai",
            "https://foo.openai.azure-api.net/openai",
            "https://foo.z01.azurefd.net/",
        ];

        for base_url in positive_cases {
            assert!(
                is_azure_responses_provider("test", Some(base_url)),
                "expected {base_url} to be detected as Azure"
            );
        }

        assert!(is_azure_responses_provider(
            "Azure",
            Some("https://example.com")
        ));

        let negative_cases = [
            "https://api.openai.com/v1",
            "https://example.com/openai",
            "https://myproxy.azurewebsites.net/openai",
        ];

        for base_url in negative_cases {
            assert!(
                !is_azure_responses_provider("test", Some(base_url)),
                "expected {base_url} not to be detected as Azure"
            );
        }
    }
}
