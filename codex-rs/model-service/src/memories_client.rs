use std::sync::Arc;

use transport_client::HttpTransport;
use transport_client_types::RequestTelemetry;
use http::HeaderMap;
use http::Method;
use model_service_api::ApiError;
use model_service_api::MemorySummarizeInput;
use model_service_api::MemorySummarizeOutput;
use model_service_api::Provider;
use model_service_api::SharedAuthProvider;
use serde::Deserialize;
use serde_json::Value;
use serde_json::to_value;

use crate::endpoint_session::EndpointSession;

/// Lightweight `/memories/trace_summarize` client owned by `model-service`.
pub struct MemoriesClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

impl<T: HttpTransport> MemoriesClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
        }
    }

    fn path() -> &'static str {
        "memories/trace_summarize"
    }

    pub async fn summarize(
        &self,
        body: Value,
        extra_headers: HeaderMap,
    ) -> Result<Vec<MemorySummarizeOutput>, ApiError> {
        let response = self
            .session
            .execute(Method::POST, Self::path(), extra_headers, Some(body))
            .await?;
        let parsed: SummarizeResponse = serde_json::from_slice(&response.body)
            .map_err(|error| ApiError::Stream(error.to_string()))?;
        Ok(parsed.output)
    }

    pub async fn summarize_input(
        &self,
        input: &MemorySummarizeInput,
        extra_headers: HeaderMap,
    ) -> Result<Vec<MemorySummarizeOutput>, ApiError> {
        let body = to_value(input).map_err(|error| {
            ApiError::Stream(format!("failed to encode memory summarize input: {error}"))
        })?;
        self.summarize(body, extra_headers).await
    }
}

#[derive(Debug, Deserialize)]
struct SummarizeResponse {
    output: Vec<MemorySummarizeOutput>,
}
