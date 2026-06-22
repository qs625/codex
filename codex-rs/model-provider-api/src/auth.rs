use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ProviderAccountError;
use codex_auth_types::RequestAuthSnapshot;
use codex_protocol::account::ProviderAccount;
use codex_protocol::auth::RefreshTokenFailedError;

/// Boxed future returned by object-safe model-provider auth APIs.
pub type ModelProviderAuthFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Provider-facing auth manager boundary used by model-provider orchestration.
///
/// Implementations adapt concrete login or host auth systems into lightweight
/// request snapshots, account visibility, and 401 recovery without exposing the
/// concrete auth runtime to API consumers.
pub trait ModelProviderAuthManager: std::fmt::Debug + Send + Sync {
    /// Returns the current provider-scoped auth snapshot, refreshing if needed.
    fn auth(&self) -> ModelProviderAuthFuture<'_, Option<RequestAuthSnapshot>>;

    /// Returns the cached auth snapshot without attempting refresh.
    fn auth_cached(&self) -> Option<RequestAuthSnapshot>;

    /// Returns the cached auth mode without attempting refresh.
    fn auth_mode(&self) -> Option<codex_auth_types::AuthMode> {
        self.auth_cached()
            .as_ref()
            .map(RequestAuthSnapshot::auth_mode)
    }

    /// Returns account information suitable for app-visible provider state.
    fn account(&self) -> Result<Option<ProviderAccount>, ProviderAccountError>;

    /// Returns whether CODEX_API_KEY environment auth is enabled for telemetry.
    fn codex_api_key_env_enabled(&self) -> bool;

    /// Returns whether the cached auth can access Codex backend-only model data.
    fn current_auth_uses_codex_backend(&self) -> bool {
        self.auth_cached()
            .as_ref()
            .is_some_and(RequestAuthSnapshot::uses_codex_backend)
    }

    /// Creates a fresh 401 recovery state machine when this auth source supports it.
    fn unauthorized_recovery(&self) -> Option<Box<dyn ModelProviderUnauthorizedRecovery>>;
}

/// Shared provider auth manager handle.
pub type SharedModelProviderAuthManager = Arc<dyn ModelProviderAuthManager>;

/// Object-safe state machine for provider auth recovery after HTTP 401.
pub trait ModelProviderUnauthorizedRecovery: Send {
    fn has_next(&self) -> bool;
    fn unavailable_reason(&self) -> &'static str;
    fn mode_name(&self) -> &'static str;
    fn step_name(&self) -> &'static str;
    fn next(
        &mut self,
    ) -> ModelProviderAuthFuture<
        '_,
        Result<ModelProviderUnauthorizedRecoveryStepResult, ModelProviderAuthRecoveryError>,
    >;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelProviderUnauthorizedRecoveryStepResult {
    auth_state_changed: Option<bool>,
}

impl ModelProviderUnauthorizedRecoveryStepResult {
    pub fn new(auth_state_changed: Option<bool>) -> Self {
        Self { auth_state_changed }
    }

    pub fn auth_state_changed(&self) -> Option<bool> {
        self.auth_state_changed
    }
}

#[derive(Debug)]
pub enum ModelProviderAuthRecoveryError {
    Permanent(RefreshTokenFailedError),
    Transient(std::io::Error),
}
