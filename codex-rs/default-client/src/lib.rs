//! Default Codex HTTP client: shared identity headers plus reqwest/`CodexHttpClient` construction.

use codex_client::BuildCustomCaTransportError;
use codex_client::CodexHttpClient;
pub use codex_client::CodexRequestBuilder;
use codex_client::build_reqwest_client_with_custom_ca;
use codex_client::with_chatgpt_cloudflare_cookie_store;
pub use codex_default_client_api::*;

/// Create an HTTP client with default `originator` and `User-Agent` headers set.
pub fn create_client() -> CodexHttpClient {
    let inner = build_reqwest_client();
    CodexHttpClient::new(inner)
}

/// Builds the default reqwest client used for ordinary Codex HTTP traffic.
///
/// This starts from the standard Codex user agent, default headers, and sandbox-specific proxy
/// policy, then layers in shared custom CA handling from `CODEX_CA_CERTIFICATE` /
/// `SSL_CERT_FILE`. The function remains infallible for compatibility with existing call sites, so
/// a custom-CA or builder failure is logged and falls back to `reqwest::Client::new()`.
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

/// Tries to build the default reqwest client used for ordinary Codex HTTP traffic.
///
/// Callers that need a structured CA-loading failure instead of the legacy logged fallback can use
/// this method directly.
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

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
