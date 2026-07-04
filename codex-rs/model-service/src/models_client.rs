use std::sync::Arc;

use transport_client::HttpTransport;
use transport_client::Request;
use transport_client_types::RequestTelemetry;
use http::HeaderMap;
use http::Method;
use http::header::ETAG;
use model_service_api::ApiError;
use model_service_api::Provider;
use model_service_api::SharedAuthProvider;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelsResponse;

use crate::endpoint_session::EndpointSession;

/// Lightweight `/models` client owned by `model-service`.
pub struct ModelsClient<T: HttpTransport> {
    session: EndpointSession<T>,
}

impl<T: HttpTransport> ModelsClient<T> {
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
        "models"
    }

    fn append_client_version_query(req: &mut Request, client_version: &str) {
        let separator = if req.url.contains('?') { '&' } else { '?' };
        req.url = format!("{}{}client_version={client_version}", req.url, separator);
    }

    pub async fn list_models(
        &self,
        client_version: &str,
        extra_headers: HeaderMap,
    ) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
        let response = self
            .session
            .execute_with(Method::GET, Self::path(), extra_headers, None, |request| {
                Self::append_client_version_query(request, client_version);
            })
            .await?;
        parse_list_models_response(response)
    }
}

fn parse_list_models_response(
    response: transport_client::Response,
) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
    let header_etag = response
        .headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let ModelsResponse { models } = serde_json::from_slice::<ModelsResponse>(&response.body)
        .map_err(|error| {
            ApiError::Stream(format!(
                "failed to decode models response: {error}; body: {}",
                String::from_utf8_lossy(&response.body)
            ))
        })?;

    Ok((models, header_etag))
}
