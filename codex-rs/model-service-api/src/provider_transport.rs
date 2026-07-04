use transport_client_types::Request;
use transport_client_types::RequestCompression;
use transport_client_types::RetryOn;
use transport_client_types::RetryPolicy;
use transport_client_types::TransportError;
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

/// Boxed future returned by model request auth providers.
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

/// Applies authentication to model API requests.
pub trait AuthProvider: Send + Sync {
    /// Adds any auth headers that are available without request body access.
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

/// Shared auth handle passed through model clients.
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
            assert!(is_azure_responses_provider("custom", Some(base_url)));
        }
    }
}
