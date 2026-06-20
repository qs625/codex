use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;

/// Boxed future returned by the object-safe model manager API.
pub type ModelsManagerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Default)]
pub struct ModelsManagerConfig {
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub tool_output_token_limit: Option<usize>,
    pub base_instructions: Option<String>,
    pub personality_enabled: bool,
    pub model_supports_reasoning_summaries: Option<bool>,
    pub model_catalog: Option<ModelsResponse>,
    pub model_metadata_overrides: Vec<ModelMetadataOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelMetadataOverride {
    pub model: String,
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    pub auto_compact_token_limit: Option<i64>,
}

/// Strategy for refreshing available models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStrategy {
    /// Always fetch from the network, ignoring cache.
    Online,
    /// Only use cached data, never fetch from the network.
    Offline,
    /// Use cache if available and fresh, otherwise fetch from the network.
    OnlineIfUncached,
}

impl RefreshStrategy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::OnlineIfUncached => "online_if_uncached",
        }
    }
}

impl fmt::Display for RefreshStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TryListModelsError;

impl fmt::Display for TryListModelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("model catalog is currently locked")
    }
}

impl std::error::Error for TryListModelsError {}

/// Runtime model catalog manager used by sessions and UI entrypoints.
///
/// Implementations own cache, auth, remote refresh, and provider-specific
/// behavior. Consumers should depend on this API crate when they only need to
/// list models, select defaults, or fetch model metadata.
pub trait ModelsManager: fmt::Debug + Send + Sync {
    /// List all available models, refreshing according to the specified strategy.
    fn list_models(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, Vec<ModelPreset>>;

    /// Return the active raw model catalog, refreshing according to the specified strategy.
    fn raw_model_catalog(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, ModelsResponse>;

    /// Attempt to list models without blocking, using the current cached state.
    fn try_list_models(&self) -> Result<Vec<ModelPreset>, TryListModelsError>;

    /// List collaboration mode presets.
    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask>;

    /// Get the model identifier to use, refreshing according to the specified strategy.
    fn get_default_model<'a>(
        &'a self,
        model: &'a Option<String>,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'a, String>;

    /// Look up model metadata, applying remote overrides and config adjustments.
    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
        config: &'a ModelsManagerConfig,
    ) -> ModelsManagerFuture<'a, ModelInfo>;

    /// Refresh models if the provided ETag differs from the cached ETag.
    fn refresh_if_new_etag(&self, etag: String) -> ModelsManagerFuture<'_, ()>;
}

/// Shared model manager handle used across runtime services.
pub type SharedModelsManager = Arc<dyn ModelsManager>;
