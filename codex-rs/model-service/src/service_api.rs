use super::*;
use crate::adapters::LegacyModelClientAdapter;
use model_service_api::CreateModelClientRequest;
use model_service_api::ListModelsRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelFuture;
use model_service_api::ModelServiceApi;
use model_service_api::ModelServiceError;
use model_service_api::ProviderHttpRequest;
use model_service_api::RefreshStrategy;
use model_service_api::ResolveDefaultModelRequest;
use model_service_api::SharedHttpClientApi;
use model_service_api::SharedModelClientApi;
use transport_client_types::Response;

impl ModelServiceApi for ModelService {
    fn list_models(
        &self,
        request: ListModelsRequest,
    ) -> ModelFuture<'_, Result<Vec<ModelPreset>, ModelServiceError>> {
        let manager = self.deps.models_manager.clone();
        Box::pin(async move {
            let models = manager.list_models(request.refresh.into()).await;
            Ok(if request.include_hidden {
                models
            } else {
                models
                    .into_iter()
                    .filter(|preset| preset.show_in_picker)
                    .collect()
            })
        })
    }

    fn raw_model_catalog(
        &self,
        refresh: ModelCatalogRefresh,
    ) -> ModelFuture<'_, Result<ModelsResponse, ModelServiceError>> {
        let manager = self.deps.models_manager.clone();
        Box::pin(async move { Ok(manager.raw_model_catalog(refresh.into()).await) })
    }

    fn list_collaboration_modes(&self) -> Vec<protocol::config_types::CollaborationModeMask> {
        self.deps.models_manager.list_collaboration_modes()
    }

    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
    ) -> ModelFuture<'a, Result<ModelInfo, ModelServiceError>> {
        let manager = self.deps.models_manager.clone();
        let config = self.models_manager_config();
        Box::pin(async move { Ok(manager.get_model_info(model, &config).await) })
    }

    fn resolve_default_model(
        &self,
        request: ResolveDefaultModelRequest,
    ) -> ModelFuture<'_, Result<Option<ModelPreset>, ModelServiceError>> {
        let manager = self.deps.models_manager.clone();
        Box::pin(async move {
            let selected_model = manager
                .get_default_model(
                    &request.selection.requested_model,
                    RefreshStrategy::from(request.selection.refresh),
                )
                .await;
            let presets = manager.list_models(request.selection.refresh.into()).await;
            Ok(presets
                .into_iter()
                .find(|preset| preset.model == selected_model || preset.id == selected_model))
        })
    }

    fn create_client(
        &self,
        request: CreateModelClientRequest,
    ) -> ModelFuture<'_, Result<SharedModelClientApi, ModelServiceError>> {
        let this = self.clone();
        Box::pin(async move {
            let effective_model =
                if let Some(requested_model) = request.selection.requested_model.clone() {
                    requested_model
                } else {
                    this.deps
                        .models_manager
                        .get_default_model(&None, request.selection.refresh.into())
                        .await
                };
            let provider_info =
                this.resolve_provider_for_selection(request.selection.provider_hint.as_deref())?;
            let telemetry_factory = DisabledSessionTelemetryFactory;
            let session_telemetry = telemetry_factory.create(SessionTelemetryCreateParams {
                conversation_id: request.thread_id,
                model: effective_model.clone(),
                slug: effective_model.clone(),
                account_id: None,
                account_email: None,
                auth_mode: None,
                auth_env: Default::default(),
                originator: "model_service".to_string(),
                log_user_prompts: false,
                terminal_type: "unknown".to_string(),
                session_source: request.session_source.clone(),
                metrics_service_name: None,
            });
            let descriptor = ModelClientDescriptor {
                model: effective_model,
                provider: request.selection.provider_hint.clone().or_else(|| {
                    this.deps
                        .default_provider
                        .as_ref()
                        .map(|provider| provider.name.clone())
                }),
                default_reasoning_effort: request.reasoning_effort,
                default_service_tier: request.service_tier,
                default_verbosity: request.verbosity,
            };
            let client = ModelClient::new(
                this.deps.provider_auth_manager.clone(),
                request.session_id,
                request.thread_id,
                request.installation_id,
                Arc::clone(&this.deps.api_runtime_factory),
                Arc::clone(&this.deps.model_provider_factory),
                provider_info,
                request.session_source,
                request.verbosity,
                request.chat_completions_max_tokens_by_model,
                request.enable_request_compression,
                request.include_timing_metrics,
                request.beta_features_header,
                this.deps.attestation_provider.clone(),
            );
            Ok(Arc::new(LegacyModelClientAdapter {
                descriptor,
                client,
                models_manager: this.deps.models_manager.clone(),
                model_metadata_overrides: this.deps.model_metadata_overrides.clone(),
                session_telemetry,
            }) as SharedModelClientApi)
        })
    }

    fn execute_provider_http(
        &self,
        request: ProviderHttpRequest,
    ) -> ModelFuture<'_, Result<Response, ModelRequestError>> {
        let this = self.clone();
        Box::pin(async move { this.execute_provider_http_request(request).await })
    }

    fn http_client(&self) -> SharedHttpClientApi {
        Arc::clone(&self.http_client)
    }

    fn refresh_if_new_etag(&self, etag: String) -> ModelFuture<'_, Result<(), ModelServiceError>> {
        let manager = self.deps.models_manager.clone();
        Box::pin(async move {
            manager.refresh_if_new_etag(etag).await;
            Ok(())
        })
    }
}
