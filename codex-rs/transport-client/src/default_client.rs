use crate::BuildCustomCaTransportError;
use crate::build_reqwest_client_with_custom_ca;
use crate::with_chatgpt_cloudflare_cookie_store;
use transport_client_identity::default_headers;
use http::Error as HttpError;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use opentelemetry::global;
use opentelemetry::propagation::Injector;
use reqwest::IntoUrl;
use reqwest::Method;
use reqwest::Response;
use serde::Serialize;
use std::fmt::Display;
use std::time::Duration;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Clone, Debug)]
pub struct CodexHttpClient {
    inner: reqwest::Client,
}

impl CodexHttpClient {
    pub fn new(inner: reqwest::Client) -> Self {
        Self { inner }
    }

    pub fn get<U>(&self, url: U) -> CodexRequestBuilder
    where
        U: IntoUrl,
    {
        self.request(Method::GET, url)
    }

    pub fn post<U>(&self, url: U) -> CodexRequestBuilder
    where
        U: IntoUrl,
    {
        self.request(Method::POST, url)
    }

    pub fn request<U>(&self, method: Method, url: U) -> CodexRequestBuilder
    where
        U: IntoUrl,
    {
        let url_str = url.as_str().to_string();
        CodexRequestBuilder::new(self.inner.request(method.clone(), url), method, url_str)
    }
}

#[must_use = "requests are not sent unless `send` is awaited"]
#[derive(Debug)]
pub struct CodexRequestBuilder {
    builder: reqwest::RequestBuilder,
    method: Method,
    url: String,
}

impl CodexRequestBuilder {
    fn new(builder: reqwest::RequestBuilder, method: Method, url: String) -> Self {
        Self {
            builder,
            method,
            url,
        }
    }

    fn map(self, f: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder) -> Self {
        Self {
            builder: f(self.builder),
            method: self.method,
            url: self.url,
        }
    }

    pub fn headers(self, headers: HeaderMap) -> Self {
        self.map(|builder| builder.headers(headers))
    }

    pub fn header<K, V>(self, key: K, value: V) -> Self
    where
        HeaderName: TryFrom<K>,
        <HeaderName as TryFrom<K>>::Error: Into<HttpError>,
        HeaderValue: TryFrom<V>,
        <HeaderValue as TryFrom<V>>::Error: Into<HttpError>,
    {
        self.map(|builder| builder.header(key, value))
    }

    pub fn bearer_auth<T>(self, token: T) -> Self
    where
        T: Display,
    {
        self.map(|builder| builder.bearer_auth(token))
    }

    pub fn timeout(self, timeout: Duration) -> Self {
        self.map(|builder| builder.timeout(timeout))
    }

    pub fn json<T>(self, value: &T) -> Self
    where
        T: ?Sized + Serialize,
    {
        self.map(|builder| builder.json(value))
    }

    pub fn body<B>(self, body: B) -> Self
    where
        B: Into<reqwest::Body>,
    {
        self.map(|builder| builder.body(body))
    }

    pub async fn send(self) -> Result<Response, reqwest::Error> {
        let headers = trace_headers();

        match self.builder.headers(headers).send().await {
            Ok(response) => {
                tracing::debug!(
                    method = %self.method,
                    url = %self.url,
                    status = %response.status(),
                    headers = ?response.headers(),
                    version = ?response.version(),
                    "Request completed"
                );

                Ok(response)
            }
            Err(error) => {
                let status = error.status();
                tracing::debug!(
                    method = %self.method,
                    url = %self.url,
                    status = status.map(|s| s.as_u16()),
                    error = %error,
                    "Request failed"
                );
                Err(error)
            }
        }
    }
}

pub fn create_client() -> CodexHttpClient {
    let inner = build_reqwest_client();
    CodexHttpClient::new(inner)
}

pub fn build_reqwest_client() -> reqwest::Client {
    try_build_reqwest_client().unwrap_or_else(|error| {
        tracing::warn!(error = %error, "failed to build default reqwest client");
        with_chatgpt_cloudflare_cookie_store(reqwest::Client::builder())
            .build()
            .unwrap_or_else(|fallback_error| {
                tracing::warn!(
                    error = %fallback_error,
                    "failed to build fallback reqwest client with ChatGPT Cloudflare cookie store"
                );
                reqwest::Client::new()
            })
    })
}

pub fn try_build_reqwest_client() -> Result<reqwest::Client, BuildCustomCaTransportError> {
    let mut builder = reqwest::Client::builder().default_headers(default_headers());
    if is_sandboxed() {
        builder = builder.no_proxy();
    }
    builder = with_chatgpt_cloudflare_cookie_store(builder);

    build_reqwest_client_with_custom_ca(builder)
}

fn is_sandboxed() -> bool {
    std::env::var("CODEX_SANDBOX").as_deref() == Ok("seatbelt")
}

struct HeaderMapInjector<'a>(&'a mut HeaderMap);

impl<'a> Injector for HeaderMapInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(val)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, val);
        }
    }
}

fn trace_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    global::get_text_map_propagator(|prop| {
        prop.inject_context(
            &Span::current().context(),
            &mut HeaderMapInjector(&mut headers),
        );
    });
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport_client_identity::RESIDENCY_HEADER_NAME;
    use transport_client_identity::ResidencyRequirement;
    use transport_client_identity::get_codex_user_agent;
    use transport_client_identity::originator;
    use transport_client_identity::set_default_client_residency_requirement;
    use opentelemetry::propagation::Extractor;
    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing::trace_span;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[test]
    fn inject_trace_headers_uses_current_span_context() {
        global::set_text_map_propagator(TraceContextPropagator::new());

        let provider = SdkTracerProvider::builder().build();
        let tracer = provider.tracer("test-tracer");
        let subscriber =
            tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
        let _guard = subscriber.set_default();

        let span = trace_span!("client_request");
        let _entered = span.enter();
        let span_context = span.context().span().span_context().clone();

        let headers = trace_headers();

        let extractor = HeaderMapExtractor(&headers);
        let extracted = TraceContextPropagator::new().extract(&extractor);
        let extracted_span = extracted.span();
        let extracted_context = extracted_span.span_context();

        assert!(extracted_context.is_valid());
        assert_eq!(extracted_context.trace_id(), span_context.trace_id());
        assert_eq!(extracted_context.span_id(), span_context.span_id());
    }

    struct HeaderMapExtractor<'a>(&'a HeaderMap);

    impl<'a> Extractor for HeaderMapExtractor<'a> {
        fn get(&self, key: &str) -> Option<&str> {
            self.0.get(key).and_then(|value| value.to_str().ok())
        }

        fn keys(&self) -> Vec<&str> {
            self.0.keys().map(HeaderName::as_str).collect()
        }
    }

    #[tokio::test]
    async fn test_create_client_sets_default_headers() {
        set_default_client_residency_requirement(Some(ResidencyRequirement::Us));

        use wiremock::Mock;
        use wiremock::MockServer;
        use wiremock::ResponseTemplate;
        use wiremock::matchers::method;
        use wiremock::matchers::path;

        let client = create_client();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let resp = client
            .get(server.uri())
            .send()
            .await
            .expect("failed to send request");
        assert!(resp.status().is_success());

        let requests = server
            .received_requests()
            .await
            .expect("failed to fetch received requests");
        assert!(!requests.is_empty());
        let headers = &requests[0].headers;

        let originator_header = headers
            .get("originator")
            .expect("originator header missing");
        assert_eq!(originator_header.to_str().unwrap(), originator().value);

        let expected_ua = get_codex_user_agent();
        let ua_header = headers
            .get("user-agent")
            .expect("user-agent header missing");
        assert_eq!(ua_header.to_str().unwrap(), expected_ua);

        let residency_header = headers
            .get(RESIDENCY_HEADER_NAME)
            .expect("residency header missing");
        assert_eq!(residency_header.to_str().unwrap(), "us");

        set_default_client_residency_requirement(/*enforce_residency*/ None);
    }
}
