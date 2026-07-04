use super::*;
use std::path::PathBuf;
use transport_client::HttpTransport;
use transport_client::ReqwestTransport;
use transport_client::build_reqwest_client;
use transport_client_types::Request;
use transport_client_types::Response;
use model_service_api::HttpClientApi;
use model_service_api::ModelFuture;
use model_service_api::ModelServiceError;
use model_service_api::ProviderHttpRequest;
use model_service_api::SharedHttpClientApi;

/// Construction parameters for `ModelService`.
#[derive(Clone)]
pub struct ModelServiceDeps {
    pub models_manager: SharedModelsManager,
    pub api_runtime_factory: SharedApiRuntimeFactory,
    pub provider_auth_manager: Option<SharedModelProviderAuthManager>,
    pub model_provider_factory: SharedModelProviderFactory,
    pub default_provider: Option<ModelProviderInfo>,
    pub providers_by_id: std::collections::HashMap<String, ModelProviderInfo>,
    pub model_metadata_overrides: Vec<ModelMetadataOverride>,
    pub attestation_provider: Option<Arc<dyn AttestationProvider>>,
}

/// Construction parameters for building a `ModelService` from configured
/// provider/runtime inputs instead of a pre-built models manager.
#[derive(Clone)]
pub struct ModelServiceRuntimeDeps {
    pub codex_home: PathBuf,
    pub config_model_catalog: Option<ModelsResponse>,
    pub api_runtime_factory: SharedApiRuntimeFactory,
    pub provider_auth_manager: Option<SharedModelProviderAuthManager>,
    pub model_provider_factory: SharedModelProviderFactory,
    pub default_provider: Option<ModelProviderInfo>,
    pub providers_by_id: std::collections::HashMap<String, ModelProviderInfo>,
    pub model_metadata_overrides: Vec<ModelMetadataOverride>,
    pub attestation_provider: Option<Arc<dyn AttestationProvider>>,
}

impl fmt::Debug for ModelServiceDeps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelServiceDeps")
            .field("models_manager", &"<models manager>")
            .field("api_runtime_factory", &"<api runtime factory>")
            .field(
                "provider_auth_manager",
                &self.provider_auth_manager.as_ref().map(|_| "<auth manager>"),
            )
            .field("model_provider_factory", &"<provider factory>")
            .field("default_provider", &self.default_provider)
            .field(
                "providers_by_id",
                &self.providers_by_id.keys().collect::<Vec<_>>(),
            )
            .field("model_metadata_overrides", &self.model_metadata_overrides)
            .field(
                "attestation_provider",
                &self.attestation_provider.as_ref().map(|_| "<attestation provider>"),
            )
            .finish()
    }
}

/// Concrete owner for model catalog, provider resolution, and model-client construction.
#[derive(Clone)]
pub struct ModelService {
    pub(crate) deps: Arc<ModelServiceDeps>,
    pub(crate) http_client: SharedHttpClientApi,
}

impl fmt::Debug for ModelService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelService")
            .field("deps", &self.deps)
            .finish()
    }
}

impl ModelService {
    pub fn new(deps: ModelServiceDeps) -> Self {
        Self {
            deps: Arc::new(deps),
            http_client: Arc::new(DefaultHttpClient::new()),
        }
    }

    pub fn from_runtime_deps(deps: ModelServiceRuntimeDeps) -> Self {
        let provider = deps
            .default_provider
            .clone()
            .or_else(|| deps.providers_by_id.values().next().cloned())
            .expect("model service requires at least one model provider");
        let models_manager = deps
            .model_provider_factory
            .create_model_provider(provider, deps.provider_auth_manager.clone())
            .models_manager(deps.codex_home, deps.config_model_catalog);
        Self::new(ModelServiceDeps {
            models_manager,
            api_runtime_factory: deps.api_runtime_factory,
            provider_auth_manager: deps.provider_auth_manager,
            model_provider_factory: deps.model_provider_factory,
            default_provider: deps.default_provider,
            providers_by_id: deps.providers_by_id,
            model_metadata_overrides: deps.model_metadata_overrides,
            attestation_provider: deps.attestation_provider,
        })
    }

    pub(crate) fn models_manager_config(&self) -> ModelsManagerConfig {
        ModelsManagerConfig {
            model_metadata_overrides: self.deps.model_metadata_overrides.clone(),
            ..Default::default()
        }
    }

    pub(crate) fn resolve_provider_for_selection(
        &self,
        requested_provider: Option<&str>,
    ) -> Result<ModelProviderInfo, ModelServiceError> {
        if let Some(provider_id) = requested_provider {
            if let Some(provider) = self.deps.providers_by_id.get(provider_id) {
                return Ok(provider.clone());
            }
            return Err(ModelServiceError::new(format!(
                "unknown model provider `{provider_id}`"
            )));
        }

        self.deps
            .default_provider
            .clone()
            .ok_or_else(|| ModelServiceError::new("no default model provider configured"))
    }

    pub(crate) async fn execute_provider_http_request(
        &self,
        request: ProviderHttpRequest,
    ) -> Result<Response, ModelRequestError> {
        let ProviderHttpRequest {
            selection,
            auth,
            request,
        } = request;
        let provider_info = self
            .resolve_provider_for_selection(selection.provider_hint.as_deref())
            .map_err(|error| ModelRequestError::new(error.message))?;
        let provider = self
            .deps
            .model_provider_factory
            .create_model_provider(provider_info, self.deps.provider_auth_manager.clone());
        let api_provider = provider
            .api_provider()
            .await
            .map_err(|error| ModelRequestError::new(error.to_string()))?;
        let auth = if let Some(auth) = auth {
            model_service_api::auth_provider_from_auth_snapshot(&auth)
        } else {
            provider
                .api_auth()
                .await
                .map_err(|error| ModelRequestError::new(error.to_string()))?
        };
        let request = request_with_provider_defaults(request, &api_provider);
        let request = auth
            .apply_auth(request)
            .await
            .map_err(|error| ModelRequestError::new(error.to_string()))?;
        let transport = ReqwestTransport::new(build_reqwest_client());
        transport
            .execute(request)
            .await
            .map_err(|error| ModelRequestError::new(error.to_string()))
    }
}

#[derive(Debug)]
struct DefaultHttpClient {
    transport: ReqwestTransport,
}

impl DefaultHttpClient {
    fn new() -> Self {
        Self {
            transport: ReqwestTransport::new(build_reqwest_client()),
        }
    }
}

impl HttpClientApi for DefaultHttpClient {
    fn execute(&self, request: Request) -> ModelFuture<'_, Result<Response, ModelRequestError>> {
        Box::pin(async move {
            self.transport
                .execute(request)
                .await
                .map_err(|error| ModelRequestError::new(error.to_string()))
        })
    }
}

pub fn default_http_client() -> SharedHttpClientApi {
    Arc::new(DefaultHttpClient::new())
}

fn request_with_provider_defaults(
    mut request: Request,
    provider: &model_service_api::Provider,
) -> Request {
    if !request.url.contains("://") {
        request.url = provider.url_for_path(&request.url);
    }
    for (header_name, header_value) in &provider.headers {
        if !request.headers.contains_key(header_name) {
            request
                .headers
                .insert(header_name.clone(), header_value.clone());
        }
    }
    request
}
