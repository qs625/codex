use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use codex_auth_types::AuthEnvTelemetryInput;
use codex_auth_types::AuthEnvTelemetryMetadata;
use codex_auth_types::TelemetryAuthMode;
use codex_auth_types::collect_auth_env_telemetry;
use transport_client::HttpTransport;
use transport_client::Request;
use transport_client::ReqwestTransport;
use transport_client::Response;
use transport_client::build_reqwest_client;
use transport_client::run_with_retry;
use transport_client_types::RequestTelemetry;
use transport_client_types::TransportError;
use codex_feedback_api::FeedbackRequestTags;
use codex_feedback_api::emit_feedback_request_tags_with_auth_env;
use http::HeaderMap;
use http::Method;
use model_service_api::ModelProviderInfo;
use model_service_api::Provider;
use model_service_api::SharedAuthProvider;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::auth_header_telemetry;
use model_service_api::extract_response_debug_context;
use model_service_api::map_api_error;
use model_service_api::telemetry_transport_error_message;
use tokio::time::timeout;

use crate::manager::ModelsEndpointClient;
use crate::provider_auth_support::resolve_provider_auth;
use crate::provider_runtime::model_provider_info_to_api_provider;
use protocol::error::CodexErr;
use protocol::error::Result as CoreResult;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelsResponse;

const MODELS_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const MODELS_ENDPOINT: &str = "/models";

/// Provider-owned OpenAI-compatible `/models` endpoint.
#[derive(Debug)]
pub(crate) struct OpenAiModelsEndpoint {
    provider_info: ModelProviderInfo,
    auth_manager: Option<SharedModelProviderAuthManager>,
}

impl OpenAiModelsEndpoint {
    pub(crate) fn new(
        provider_info: ModelProviderInfo,
        auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> Self {
        Self {
            provider_info,
            auth_manager,
        }
    }

    async fn auth(&self) -> Option<codex_auth_types::RequestAuthSnapshot> {
        match self.auth_manager.as_ref() {
            Some(auth_manager) => auth_manager.auth().await,
            None => None,
        }
    }

    fn auth_env(&self) -> AuthEnvTelemetryMetadata {
        let codex_api_key_env_enabled = self
            .auth_manager
            .as_ref()
            .is_some_and(|auth_manager| auth_manager.codex_api_key_env_enabled());
        collect_auth_env_telemetry(AuthEnvTelemetryInput {
            provider_env_key: self.provider_info.env_key.as_deref(),
            codex_api_key_env_enabled,
        })
        .to_otel_metadata()
    }
}

#[async_trait]
impl ModelsEndpointClient for OpenAiModelsEndpoint {
    fn has_command_auth(&self) -> bool {
        self.provider_info.has_command_auth()
    }

    async fn uses_codex_backend(&self) -> bool {
        self.auth()
            .await
            .as_ref()
            .is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend)
    }

    async fn list_models(
        &self,
        client_version: &str,
    ) -> CoreResult<(Vec<ModelInfo>, Option<String>)> {
        let _timer =
            codex_otel::start_global_timer("codex.remote_models.fetch_update.duration_ms", &[]);
        let auth = self.auth().await;
        let auth_mode = auth
            .as_ref()
            .map(codex_auth_types::RequestAuthSnapshot::auth_mode);
        let api_provider = model_provider_info_to_api_provider(&self.provider_info, auth_mode)?;
        let api_auth = resolve_provider_auth(auth.as_ref(), &self.provider_info)?;
        let transport = ReqwestTransport::new(build_reqwest_client());
        let auth_telemetry = auth_header_telemetry(api_auth.as_ref());
        let request_telemetry: Arc<dyn RequestTelemetry> = Arc::new(ModelsRequestTelemetry {
            auth_mode: auth_mode.map(|mode| TelemetryAuthMode::from(mode).to_string()),
            auth_header_attached: auth_telemetry.attached,
            auth_header_name: auth_telemetry.name,
            auth_env: self.auth_env(),
        });
        timeout(
            MODELS_REFRESH_TIMEOUT,
            list_models_request(
                &transport,
                &api_provider,
                api_auth,
                client_version,
                HeaderMap::new(),
                Some(request_telemetry),
            ),
        )
        .await
        .map_err(|_| CodexErr::Timeout)?
        .map_err(map_api_error)
    }
}

async fn list_models_request(
    transport: &ReqwestTransport,
    provider: &Provider,
    auth: SharedAuthProvider,
    client_version: &str,
    extra_headers: HeaderMap,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
) -> Result<(Vec<ModelInfo>, Option<String>), model_service_api::ApiError> {
    let response = run_with_retry(
        provider.retry.to_policy(),
        || {
            let mut request = provider.build_request(Method::GET, "models");
            request.headers.extend(extra_headers.clone());
            append_client_version_query(&mut request, client_version);
            request
        },
        |request, attempt| {
            let auth = auth.clone();
            let request_telemetry = request_telemetry.clone();
            async move {
                let start = tokio::time::Instant::now();
                let result = execute_authorized_request(transport, auth, request).await;
                if let Some(telemetry) = request_telemetry.as_ref() {
                    let (status, error) = match &result {
                        Ok(response) => (Some(response.status), None),
                        Err(error) => (http_status(error), Some(error)),
                    };
                    telemetry.on_request(attempt, status, error, start.elapsed());
                }
                result
            }
        },
    )
    .await
    .map_err(model_service_api::ApiError::from)?;

    let header_etag = response
        .headers
        .get(http::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let models_response =
        serde_json::from_slice::<ModelsResponse>(&response.body).map_err(|error| {
            model_service_api::ApiError::Stream(format!(
                "failed to decode models response: {error}; body: {}",
                String::from_utf8_lossy(&response.body)
            ))
        })?;

    Ok((models_response.models, header_etag))
}

fn append_client_version_query(request: &mut Request, client_version: &str) {
    let separator = if request.url.contains('?') { '&' } else { '?' };
    request.url = format!(
        "{}{}client_version={client_version}",
        request.url, separator
    );
}

async fn execute_authorized_request(
    transport: &ReqwestTransport,
    auth: SharedAuthProvider,
    request: Request,
) -> Result<Response, TransportError> {
    let request = auth
        .apply_auth(request)
        .await
        .map_err(TransportError::from)?;
    transport.execute(request).await
}

fn http_status(error: &TransportError) -> Option<http::StatusCode> {
    match error {
        TransportError::Http { status, .. } => Some(*status),
        _ => None,
    }
}

#[derive(Clone)]
struct ModelsRequestTelemetry {
    auth_mode: Option<String>,
    auth_header_attached: bool,
    auth_header_name: Option<&'static str>,
    auth_env: AuthEnvTelemetryMetadata,
}

impl RequestTelemetry for ModelsRequestTelemetry {
    fn on_request(
        &self,
        attempt: u64,
        status: Option<http::StatusCode>,
        error: Option<&TransportError>,
        duration: Duration,
    ) {
        let success = status.is_some_and(|code| code.is_success()) && error.is_none();
        let error_message = error.map(telemetry_transport_error_message);
        let response_debug = error
            .map(extract_response_debug_context)
            .unwrap_or_default();
        let status = status.map(|status| status.as_u16());
        tracing::event!(
            target: "codex_otel.log_only",
            tracing::Level::INFO,
            event.name = "codex.api_request",
            duration_ms = %duration.as_millis(),
            http.response.status_code = status,
            success = success,
            error.message = error_message.as_deref(),
            attempt = attempt,
            endpoint = MODELS_ENDPOINT,
            auth.header_attached = self.auth_header_attached,
            auth.header_name = self.auth_header_name,
            auth.env_openai_api_key_present = self.auth_env.openai_api_key_env_present,
            auth.env_codex_api_key_present = self.auth_env.codex_api_key_env_present,
            auth.env_codex_api_key_enabled = self.auth_env.codex_api_key_env_enabled,
            auth.env_provider_key_name = self.auth_env.provider_env_key_name.as_deref(),
            auth.env_provider_key_present = self.auth_env.provider_env_key_present,
            auth.env_refresh_token_url_override_present = self.auth_env.refresh_token_url_override_present,
            auth.request_id = response_debug.request_id.as_deref(),
            auth.cf_ray = response_debug.cf_ray.as_deref(),
            auth.error = response_debug.auth_error.as_deref(),
            auth.error_code = response_debug.auth_error_code.as_deref(),
            auth.mode = self.auth_mode.as_deref(),
        );
        tracing::event!(
            target: "codex_otel.trace_safe",
            tracing::Level::INFO,
            event.name = "codex.api_request",
            duration_ms = %duration.as_millis(),
            http.response.status_code = status,
            success = success,
            error.message = error_message.as_deref(),
            attempt = attempt,
            endpoint = MODELS_ENDPOINT,
            auth.header_attached = self.auth_header_attached,
            auth.header_name = self.auth_header_name,
            auth.env_openai_api_key_present = self.auth_env.openai_api_key_env_present,
            auth.env_codex_api_key_present = self.auth_env.codex_api_key_env_present,
            auth.env_codex_api_key_enabled = self.auth_env.codex_api_key_env_enabled,
            auth.env_provider_key_name = self.auth_env.provider_env_key_name.as_deref(),
            auth.env_provider_key_present = self.auth_env.provider_env_key_present,
            auth.env_refresh_token_url_override_present = self.auth_env.refresh_token_url_override_present,
            auth.request_id = response_debug.request_id.as_deref(),
            auth.cf_ray = response_debug.cf_ray.as_deref(),
            auth.error = response_debug.auth_error.as_deref(),
            auth.error_code = response_debug.auth_error_code.as_deref(),
            auth.mode = self.auth_mode.as_deref(),
        );
        emit_feedback_request_tags_with_auth_env(
            &FeedbackRequestTags {
                endpoint: MODELS_ENDPOINT,
                auth_header_attached: self.auth_header_attached,
                auth_header_name: self.auth_header_name,
                auth_mode: self.auth_mode.as_deref(),
                auth_retry_after_unauthorized: None,
                auth_recovery_mode: None,
                auth_recovery_phase: None,
                auth_connection_reused: None,
                auth_request_id: response_debug.request_id.as_deref(),
                auth_cf_ray: response_debug.cf_ray.as_deref(),
                auth_error: response_debug.auth_error.as_deref(),
                auth_error_code: response_debug.auth_error_code.as_deref(),
                auth_recovery_followup_success: None,
                auth_recovery_followup_status: None,
            },
            &self.auth_env,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use protocol::config_types::ModelProviderAuthInfo;

    fn provider_info_with_command_auth() -> ModelProviderInfo {
        ModelProviderInfo {
            auth: Some(ModelProviderAuthInfo {
                command: "print-token".to_string(),
                args: Vec::new(),
                timeout_ms: NonZeroU64::new(5_000).expect("timeout should be non-zero"),
                refresh_interval_ms: 300_000,
                cwd: std::env::current_dir()
                    .expect("current dir should be available")
                    .try_into()
                    .expect("current dir should be absolute"),
            }),
            requires_openai_auth: false,
            ..ModelProviderInfo::create_openai_provider(/*base_url*/ None)
        }
    }

    #[test]
    fn command_auth_provider_reports_command_auth_without_cached_auth() {
        let endpoint = OpenAiModelsEndpoint::new(
            provider_info_with_command_auth(),
            /*auth_manager*/ None,
        );

        assert!(endpoint.has_command_auth());
    }

    #[test]
    fn provider_without_command_auth_reports_no_command_auth() {
        let endpoint = OpenAiModelsEndpoint::new(
            ModelProviderInfo::create_openai_provider(/*base_url*/ None),
            /*auth_manager*/ None,
        );

        assert!(!endpoint.has_command_auth());
    }
}
