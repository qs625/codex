use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_api_provider::Provider;
use codex_api_provider::SharedAuthProvider;
use codex_auth_types::RequestAuthSnapshot;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager_api::SharedModelsManager;
use codex_protocol::account::ProviderAccount;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::openai_models::ModelsResponse;

use crate::BearerAuthProvider;
use crate::SharedModelProviderAuthManager;
use crate::auth_provider_from_auth_snapshot;
use crate::model_provider_info_to_api_provider;
use crate::unauthenticated_auth_provider;

/// Boxed future returned by object-safe model provider APIs.
pub type ModelProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Optional provider-backed features that Codex may expose at runtime.
///
/// These capabilities are a provider-owned upper bound. Callers can disable
/// more functionality through normal config, but should not expose a feature
/// that the active provider marks unsupported here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub namespace_tools: bool,
    pub image_generation: bool,
    pub web_search: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            namespace_tools: true,
            image_generation: true,
            web_search: true,
        }
    }
}

/// Current app-visible account state for a model provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAccountState {
    pub account: Option<ProviderAccount>,
    pub requires_openai_auth: bool,
}

/// Error returned when a provider cannot construct its app-visible account state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAccountError {
    MissingChatgptAccountDetails,
}

impl fmt::Display for ProviderAccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingChatgptAccountDetails => {
                write!(
                    f,
                    "email and plan type are required for chatgpt authentication"
                )
            }
        }
    }
}

impl std::error::Error for ProviderAccountError {}

pub type ProviderAccountResult = std::result::Result<ProviderAccountState, ProviderAccountError>;

/// Default model used for automatic approval review when a provider does not
/// require a backend-specific model ID.
pub const DEFAULT_APPROVAL_REVIEW_PREFERRED_MODEL: &str = "codex-auto-review";

/// Runtime provider abstraction used by model execution.
///
/// Implementations own provider-specific behavior for a model backend. The
/// `ModelProviderInfo` returned by `info` is the serialized/configured provider
/// metadata used by the default OpenAI-compatible implementation.
pub trait ModelProvider: fmt::Debug + Send + Sync {
    /// Returns the configured provider metadata.
    fn info(&self) -> &ModelProviderInfo;

    /// Returns the provider-owned capability upper bounds.
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Returns the preferred model used for automatic approval review.
    ///
    /// Providers that require backend-specific model IDs should override this.
    fn approval_review_preferred_model(&self) -> &'static str {
        DEFAULT_APPROVAL_REVIEW_PREFERRED_MODEL
    }

    /// Returns whether requests made through this provider should include attestation.
    fn supports_attestation(&self) -> bool {
        false
    }

    /// Returns the provider-scoped auth manager, when this provider uses one.
    fn auth_manager(&self) -> Option<SharedModelProviderAuthManager>;

    /// Returns the current provider-scoped auth value, if one is configured.
    fn auth(&self) -> ModelProviderFuture<'_, Option<RequestAuthSnapshot>>;

    /// Returns the current app-visible account state for this provider.
    fn account_state(&self) -> ProviderAccountResult;

    /// Returns provider configuration adapted for the API client.
    fn api_provider(&self) -> ModelProviderFuture<'_, CodexResult<Provider>> {
        Box::pin(async move {
            let auth = self.auth().await;
            Ok(model_provider_info_to_api_provider(
                self.info(),
                auth.as_ref().map(RequestAuthSnapshot::auth_mode),
            ))
        })
    }

    /// Returns the provider base URL that will be used at request time.
    fn runtime_base_url(&self) -> ModelProviderFuture<'_, CodexResult<Option<String>>> {
        Box::pin(async move { Ok(self.info().base_url.clone()) })
    }

    /// Returns the auth provider used to attach request credentials.
    fn api_auth(&self) -> ModelProviderFuture<'_, CodexResult<SharedAuthProvider>> {
        Box::pin(async move {
            let auth = self.auth().await;
            resolve_provider_auth(auth.as_ref(), self.info())
        })
    }

    /// Creates the model manager implementation appropriate for this provider.
    fn models_manager(
        &self,
        codex_home: PathBuf,
        config_model_catalog: Option<ModelsResponse>,
    ) -> SharedModelsManager;
}

/// Shared runtime model provider handle.
pub type SharedModelProvider = Arc<dyn ModelProvider>;

/// Factory for creating runtime model provider implementations.
///
/// Runtime orchestration crates depend on this trait so they can request a
/// provider for a configured backend without depending on the concrete
/// provider implementation crate.
pub trait ModelProviderFactory: fmt::Debug + Send + Sync {
    /// Create a runtime provider for the supplied provider metadata and optional auth manager.
    fn create_model_provider(
        &self,
        provider_info: ModelProviderInfo,
        auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> SharedModelProvider;
}

/// Shared runtime model provider factory handle.
pub type SharedModelProviderFactory = Arc<dyn ModelProviderFactory>;

/// Resolves the auth provider used for runtime API requests.
pub fn resolve_provider_auth(
    auth: Option<&RequestAuthSnapshot>,
    provider: &ModelProviderInfo,
) -> CodexResult<SharedAuthProvider> {
    if let Some(auth) = bearer_auth_for_provider(provider)? {
        return Ok(Arc::new(auth));
    }

    Ok(match auth {
        Some(auth) => auth_provider_from_auth_snapshot(auth),
        None => unauthenticated_auth_provider(),
    })
}

fn bearer_auth_for_provider(
    provider: &ModelProviderInfo,
) -> CodexResult<Option<BearerAuthProvider>> {
    if let Some(api_key) = provider.api_key()? {
        return Ok(Some(BearerAuthProvider::new(api_key)));
    }

    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(Some(BearerAuthProvider::new(token)));
    }

    Ok(None)
}
