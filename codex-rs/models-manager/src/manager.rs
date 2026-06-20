use super::cache::ModelsCacheManager;
use crate::collaboration_mode_presets::builtin_collaboration_mode_presets;
use crate::model_info;
use async_trait::async_trait;
use codex_auth_types::AuthMode;
use codex_login::AuthManager;
pub use codex_models_manager_api::ModelsManager;
pub use codex_models_manager_api::ModelsManagerConfig;
pub use codex_models_manager_api::ModelsManagerFuture;
pub use codex_models_manager_api::RefreshStrategy;
pub use codex_models_manager_api::SharedModelsManager;
pub use codex_models_manager_api::TryListModelsError;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::error::Result as CoreResult;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::Instrument as _;
use tracing::error;
use tracing::info;

const MODEL_CACHE_FILE: &str = "models_cache.json";
const DEFAULT_MODEL_CACHE_TTL: Duration = Duration::from_secs(300);

/// Remote endpoint used by the OpenAI-compatible model manager.
///
/// Implementations own provider-specific auth and transport details. The model
/// manager owns refresh policy, cache behavior, and catalog merging; it calls
/// this endpoint only when it decides a remote refresh should happen.
#[async_trait]
pub trait ModelsEndpointClient: fmt::Debug + Send + Sync {
    /// Returns whether this provider can authenticate command-scoped requests.
    fn has_command_auth(&self) -> bool;

    /// Returns whether the currently resolved auth can use Codex backend-only models.
    async fn uses_codex_backend(&self) -> bool;

    /// Fetches the latest remote model catalog and optional ETag.
    async fn list_models(
        &self,
        client_version: &str,
    ) -> CoreResult<(Vec<ModelInfo>, Option<String>)>;
}

/// Controls whether a model manager may read and write the shared on-disk model cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCachePolicy {
    /// Use the shared on-disk cache.
    Enabled,
    /// Ignore the shared on-disk cache.
    Disabled,
}

type SharedModelsEndpointClient = Arc<dyn ModelsEndpointClient>;

/// OpenAI-compatible model manager backed by bundled models, cache, and `/models`.
#[derive(Debug)]
pub struct OpenAiModelsManager {
    remote_models: RwLock<Vec<ModelInfo>>,
    fallback_models: Vec<ModelInfo>,
    etag: RwLock<Option<String>>,
    cache_manager: ModelsCacheManager,
    cache_policy: ModelCachePolicy,
    endpoint_client: SharedModelsEndpointClient,
    auth_manager: Option<Arc<AuthManager>>,
}

/// Static model manager backed by an authoritative in-process catalog.
#[derive(Debug)]
pub struct StaticModelsManager {
    remote_models: Vec<ModelInfo>,
    auth_manager: Option<Arc<AuthManager>>,
}

impl OpenAiModelsManager {
    /// Construct an OpenAI-compatible remote model manager.
    pub fn new(
        codex_home: PathBuf,
        endpoint_client: Arc<dyn ModelsEndpointClient>,
        auth_manager: Option<Arc<AuthManager>>,
    ) -> Self {
        Self::new_with_fallback_models(
            codex_home,
            endpoint_client,
            auth_manager,
            load_remote_models_from_file().unwrap_or_default(),
            ModelCachePolicy::Enabled,
        )
    }

    /// Construct an OpenAI-compatible remote model manager with explicit fallback models.
    pub fn new_with_fallback_models(
        codex_home: PathBuf,
        endpoint_client: Arc<dyn ModelsEndpointClient>,
        auth_manager: Option<Arc<AuthManager>>,
        fallback_models: Vec<ModelInfo>,
        cache_policy: ModelCachePolicy,
    ) -> Self {
        let cache_path = codex_home.join(MODEL_CACHE_FILE);
        let cache_manager = ModelsCacheManager::new(cache_path, DEFAULT_MODEL_CACHE_TTL);
        Self {
            remote_models: RwLock::new(fallback_models.clone()),
            fallback_models,
            etag: RwLock::new(None),
            cache_manager,
            cache_policy,
            endpoint_client,
            auth_manager,
        }
    }
}

impl StaticModelsManager {
    /// Construct a static model manager from an authoritative catalog.
    pub fn new(auth_manager: Option<Arc<AuthManager>>, model_catalog: ModelsResponse) -> Self {
        Self {
            remote_models: model_catalog.models,
            auth_manager,
        }
    }
}

impl ModelsManager for OpenAiModelsManager {
    fn list_models(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, Vec<ModelPreset>> {
        Box::pin(
            async move {
                let catalog = self.raw_model_catalog(refresh_strategy).await;
                self.build_available_models(catalog.models)
            }
            .instrument(tracing::info_span!(
                "list_models",
                refresh_strategy = %refresh_strategy
            )),
        )
    }

    fn raw_model_catalog(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, ModelsResponse> {
        Box::pin(async move {
            if let Err(err) = self.refresh_available_models(refresh_strategy).await {
                error!("failed to refresh available models: {err}");
            }
            ModelsResponse {
                models: self.get_remote_models().await,
            }
        })
    }

    fn try_list_models(&self) -> Result<Vec<ModelPreset>, TryListModelsError> {
        let remote_models = self
            .remote_models
            .try_read()
            .map_err(|_| TryListModelsError)?
            .clone();
        Ok(self.build_available_models(remote_models))
    }

    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        builtin_collaboration_mode_presets()
    }

    fn get_default_model<'a>(
        &'a self,
        model: &'a Option<String>,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'a, String> {
        Box::pin(
            async move {
                if let Some(model) = model.as_ref() {
                    return model.to_string();
                }
                default_model_from_available(self.list_models(refresh_strategy).await)
            }
            .instrument(tracing::info_span!(
                "get_default_model",
                model.provided = model.is_some(),
                refresh_strategy = %refresh_strategy
            )),
        )
    }

    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
        config: &'a ModelsManagerConfig,
    ) -> ModelsManagerFuture<'a, ModelInfo> {
        Box::pin(
            async move {
                let remote_models = self.get_remote_models().await;
                construct_model_info_from_candidates(model, &remote_models, config)
            }
            .instrument(tracing::info_span!("get_model_info", model = model)),
        )
    }

    fn refresh_if_new_etag(&self, etag: String) -> ModelsManagerFuture<'_, ()> {
        Box::pin(async move {
            let current_etag = self.get_etag().await;
            if current_etag.clone().is_some() && current_etag.as_deref() == Some(etag.as_str()) {
                if let Err(err) = self.cache_manager.renew_cache_ttl().await {
                    error!("failed to renew cache TTL: {err}");
                }
                return;
            }
            if let Err(err) = self.refresh_available_models(RefreshStrategy::Online).await {
                error!("failed to refresh available models: {err}");
            }
        })
    }
}

impl OpenAiModelsManager {
    async fn get_remote_models(&self) -> Vec<ModelInfo> {
        self.remote_models.read().await.clone()
    }

    fn build_available_models(&self, remote_models: Vec<ModelInfo>) -> Vec<ModelPreset> {
        build_available_models(
            remote_models,
            self.auth_manager
                .as_ref()
                .is_some_and(|auth_manager| auth_manager.current_auth_uses_codex_backend()),
        )
    }

    /// Refresh available models according to the specified strategy.
    async fn refresh_available_models(&self, refresh_strategy: RefreshStrategy) -> CoreResult<()> {
        if !self.should_refresh_models().await {
            if matches!(
                refresh_strategy,
                RefreshStrategy::Offline | RefreshStrategy::OnlineIfUncached
            ) {
                self.try_load_cache().await;
            }
            return Ok(());
        }

        match refresh_strategy {
            RefreshStrategy::Offline => {
                // Only try to load from cache, never fetch
                self.try_load_cache().await;
                Ok(())
            }
            RefreshStrategy::OnlineIfUncached => {
                // Try cache first, fall back to online if unavailable
                if self.try_load_cache().await {
                    info!("models cache: using cached models for OnlineIfUncached");
                    return Ok(());
                }
                info!("models cache: cache miss, fetching remote models");
                self.fetch_and_update_models().await
            }
            RefreshStrategy::Online => {
                // Always fetch from network
                self.fetch_and_update_models().await
            }
        }
    }

    async fn fetch_and_update_models(&self) -> CoreResult<()> {
        let client_version = crate::client_version_to_whole();
        let (models, etag) = self.endpoint_client.list_models(&client_version).await?;
        self.apply_remote_models(models.clone()).await;
        *self.etag.write().await = etag.clone();
        if self.cache_policy == ModelCachePolicy::Enabled {
            self.cache_manager
                .persist_cache(&models, etag, client_version)
                .await;
        }
        Ok(())
    }

    async fn should_refresh_models(&self) -> bool {
        self.endpoint_client.uses_codex_backend().await || self.endpoint_client.has_command_auth()
    }

    async fn get_etag(&self) -> Option<String> {
        self.etag.read().await.clone()
    }

    /// Replace the cached remote models and rebuild the derived presets list.
    async fn apply_remote_models(&self, models: Vec<ModelInfo>) {
        // Use the remote models list as the source of truth if it contains at least one
        // non-hidden model and the user is using ChatGPT auth.
        let should_use_remote_models_only = !models.is_empty()
            && models
                .iter()
                .any(|model| model.visibility == ModelVisibility::List)
            && self.auth_manager.as_ref().is_some_and(|auth_manager| {
                matches!(
                    auth_manager.auth_mode(),
                    Some(AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens)
                )
            });
        if should_use_remote_models_only {
            *self.remote_models.write().await = models;
            return;
        }

        let mut existing_models = self.fallback_models.clone();
        for model in models {
            if let Some(existing_index) = existing_models
                .iter()
                .position(|existing| existing.slug == model.slug)
            {
                existing_models[existing_index] = model;
            } else {
                existing_models.push(model);
            }
        }
        *self.remote_models.write().await = existing_models;
    }

    /// Attempt to satisfy the refresh from the cache when it matches the provider and TTL.
    async fn try_load_cache(&self) -> bool {
        if self.cache_policy == ModelCachePolicy::Disabled {
            info!("models cache: disabled for provider");
            return false;
        }

        let _timer =
            codex_otel::start_global_timer("codex.remote_models.load_cache.duration_ms", &[]);
        let client_version = crate::client_version_to_whole();
        info!(client_version, "models cache: evaluating cache eligibility");
        // TODO(celia-oai): Include provider identity in cache eligibility so switching
        // providers does not reuse a fresh models_cache.json entry from another provider.
        let cache = match self.cache_manager.load_fresh(&client_version).await {
            Some(cache) => cache,
            None => {
                info!("models cache: no usable cache entry");
                return false;
            }
        };
        let models = cache.models.clone();
        *self.etag.write().await = cache.etag.clone();
        self.apply_remote_models(models.clone()).await;
        info!(
            models_count = models.len(),
            etag = ?cache.etag,
            "models cache: cache entry applied"
        );
        true
    }
}

impl ModelsManager for StaticModelsManager {
    fn list_models(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, Vec<ModelPreset>> {
        Box::pin(
            async move {
                let catalog = self.raw_model_catalog(refresh_strategy).await;
                self.build_available_models(catalog.models)
            }
            .instrument(tracing::info_span!(
                "list_models",
                refresh_strategy = %refresh_strategy
            )),
        )
    }

    fn raw_model_catalog(
        &self,
        _refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, ModelsResponse> {
        Box::pin(async move {
            ModelsResponse {
                models: self.get_remote_models().await,
            }
        })
    }

    fn try_list_models(&self) -> Result<Vec<ModelPreset>, TryListModelsError> {
        Ok(self.build_available_models(self.remote_models.clone()))
    }

    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        builtin_collaboration_mode_presets()
    }

    fn get_default_model<'a>(
        &'a self,
        model: &'a Option<String>,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'a, String> {
        Box::pin(
            async move {
                if let Some(model) = model.as_ref() {
                    return model.to_string();
                }
                default_model_from_available(self.list_models(refresh_strategy).await)
            }
            .instrument(tracing::info_span!(
                "get_default_model",
                model.provided = model.is_some(),
                refresh_strategy = %refresh_strategy
            )),
        )
    }

    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
        config: &'a ModelsManagerConfig,
    ) -> ModelsManagerFuture<'a, ModelInfo> {
        Box::pin(
            async move {
                let remote_models = self.get_remote_models().await;
                construct_model_info_from_candidates(model, &remote_models, config)
            }
            .instrument(tracing::info_span!("get_model_info", model = model)),
        )
    }

    fn refresh_if_new_etag(&self, _etag: String) -> ModelsManagerFuture<'_, ()> {
        Box::pin(async {})
    }
}

impl StaticModelsManager {
    async fn get_remote_models(&self) -> Vec<ModelInfo> {
        self.remote_models.clone()
    }

    fn build_available_models(&self, remote_models: Vec<ModelInfo>) -> Vec<ModelPreset> {
        build_available_models(
            remote_models,
            self.auth_manager
                .as_ref()
                .is_some_and(|auth_manager| auth_manager.current_auth_uses_codex_backend()),
        )
    }
}

fn load_remote_models_from_file() -> Result<Vec<ModelInfo>, std::io::Error> {
    Ok(crate::bundled_models_response()?.models)
}

fn default_model_from_available(available: Vec<ModelPreset>) -> String {
    available
        .iter()
        .find(|model| model.is_default)
        .or_else(|| available.first())
        .map(|model| model.model.clone())
        .unwrap_or_default()
}

fn build_available_models(
    mut remote_models: Vec<ModelInfo>,
    uses_codex_backend: bool,
) -> Vec<ModelPreset> {
    remote_models.sort_by(|a, b| a.priority.cmp(&b.priority));

    let mut presets: Vec<ModelPreset> = remote_models.into_iter().map(Into::into).collect();
    presets = ModelPreset::filter_by_auth(presets, uses_codex_backend);

    ModelPreset::mark_default_by_picker_visibility(&mut presets);

    presets
}

fn find_model_by_longest_prefix(model: &str, candidates: &[ModelInfo]) -> Option<ModelInfo> {
    let mut best: Option<ModelInfo> = None;
    for candidate in candidates {
        if !model.starts_with(&candidate.slug) {
            continue;
        }
        let is_better_match = if let Some(current) = best.as_ref() {
            candidate.slug.len() > current.slug.len()
        } else {
            true
        };
        if is_better_match {
            best = Some(candidate.clone());
        }
    }
    best
}

fn find_model_by_namespaced_suffix(model: &str, candidates: &[ModelInfo]) -> Option<ModelInfo> {
    // Retry metadata lookup for a single namespaced slug like `namespace/model-name`.
    //
    // This only strips one leading namespace segment and only when the namespace looks
    // like a simple provider id to avoid broadly matching arbitrary aliases.
    let (namespace, suffix) = model.split_once('/')?;
    if suffix.contains('/') {
        return None;
    }
    if namespace.is_empty()
        || !namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    find_model_by_longest_prefix(suffix, candidates)
}

pub(crate) fn construct_model_info_from_candidates(
    model: &str,
    candidates: &[ModelInfo],
    config: &ModelsManagerConfig,
) -> ModelInfo {
    // First use the normal longest-prefix match. If that misses, allow a narrowly scoped
    // retry for namespaced slugs like `custom/gpt-5.3-codex`.
    let remote = find_model_by_longest_prefix(model, candidates)
        .or_else(|| find_model_by_namespaced_suffix(model, candidates));
    let model_info = if let Some(remote) = remote {
        ModelInfo {
            slug: model.to_string(),
            used_fallback_model_metadata: false,
            ..remote
        }
    } else {
        model_info::model_info_from_slug(model)
    };
    model_info::with_config_overrides(model_info, config)
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
