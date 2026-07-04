use std::sync::Arc;

use model_service_api::ModelProviderAuthFuture;
use model_service_api::ModelProviderAuthManager;
use model_service_api::ModelProviderAuthRecoveryError;
use model_service_api::ModelProviderUnauthorizedRecovery;
use model_service_api::ModelProviderUnauthorizedRecoveryStepResult;
use model_service_api::ProviderAccountError;
use model_service_api::SharedModelProviderAuthManager;
use protocol::account::ProviderAccount;

use crate::auth::AuthManager;
use crate::auth::CodexAuth;
use crate::auth::RefreshTokenError;
use crate::auth::UnauthorizedRecovery;

/// Adapts the login runtime auth manager to the model-provider auth boundary.
///
/// This keeps `CodexAuth` matching and refresh recovery ownership inside
/// `codex-login`; callers should pass the returned trait object to provider
/// factories instead of reimplementing this adapter in session/runtime code.
pub fn model_provider_auth_manager(
    auth_manager: Option<Arc<AuthManager>>,
) -> Option<SharedModelProviderAuthManager> {
    auth_manager.map(|auth_manager| {
        Arc::new(LoginModelProviderAuthManager { auth_manager }) as SharedModelProviderAuthManager
    })
}

#[derive(Debug)]
struct LoginModelProviderAuthManager {
    auth_manager: Arc<AuthManager>,
}

impl ModelProviderAuthManager for LoginModelProviderAuthManager {
    fn auth(&self) -> ModelProviderAuthFuture<'_, Option<codex_auth_types::RequestAuthSnapshot>> {
        Box::pin(async move {
            self.auth_manager
                .auth()
                .await
                .as_ref()
                .map(CodexAuth::request_auth_snapshot)
        })
    }

    fn auth_cached(&self) -> Option<codex_auth_types::RequestAuthSnapshot> {
        self.auth_manager
            .auth_cached()
            .as_ref()
            .map(CodexAuth::request_auth_snapshot)
    }

    fn account(&self) -> Result<Option<ProviderAccount>, ProviderAccountError> {
        let Some(auth) = self.auth_manager.auth_cached() else {
            return Ok(None);
        };
        if self.auth_manager.refresh_failure_for_auth(&auth).is_some() {
            return Ok(None);
        }
        provider_account_from_auth(&auth)
    }

    fn codex_api_key_env_enabled(&self) -> bool {
        self.auth_manager.codex_api_key_env_enabled()
    }

    fn current_auth_uses_codex_backend(&self) -> bool {
        self.auth_manager.current_auth_uses_codex_backend()
    }

    fn unauthorized_recovery(&self) -> Option<Box<dyn ModelProviderUnauthorizedRecovery>> {
        Some(Box::new(LoginModelProviderUnauthorizedRecovery {
            recovery: self.auth_manager.unauthorized_recovery(),
        }))
    }
}

struct LoginModelProviderUnauthorizedRecovery {
    recovery: UnauthorizedRecovery,
}

impl ModelProviderUnauthorizedRecovery for LoginModelProviderUnauthorizedRecovery {
    fn has_next(&self) -> bool {
        self.recovery.has_next()
    }

    fn unavailable_reason(&self) -> &'static str {
        self.recovery.unavailable_reason()
    }

    fn mode_name(&self) -> &'static str {
        self.recovery.mode_name()
    }

    fn step_name(&self) -> &'static str {
        self.recovery.step_name()
    }

    fn next(
        &mut self,
    ) -> ModelProviderAuthFuture<
        '_,
        Result<ModelProviderUnauthorizedRecoveryStepResult, ModelProviderAuthRecoveryError>,
    > {
        Box::pin(async move {
            self.recovery
                .next()
                .await
                .map(|result| {
                    ModelProviderUnauthorizedRecoveryStepResult::new(result.auth_state_changed())
                })
                .map_err(map_recovery_error)
        })
    }
}

fn map_recovery_error(error: RefreshTokenError) -> ModelProviderAuthRecoveryError {
    match error {
        RefreshTokenError::Permanent(error) => ModelProviderAuthRecoveryError::Permanent(error),
        RefreshTokenError::Transient(error) => ModelProviderAuthRecoveryError::Transient(error),
    }
}

fn provider_account_from_auth(
    auth: &CodexAuth,
) -> Result<Option<ProviderAccount>, ProviderAccountError> {
    match auth {
        CodexAuth::ApiKey(_) => Ok(Some(ProviderAccount::ApiKey)),
        CodexAuth::Chatgpt(_) | CodexAuth::ChatgptAuthTokens(_) | CodexAuth::AgentIdentity(_) => {
            let email = auth.get_account_email();
            let plan_type = auth.account_plan_type();

            match (email, plan_type) {
                (Some(email), Some(plan_type)) => {
                    Ok(Some(ProviderAccount::Chatgpt { email, plan_type }))
                }
                _ => Err(ProviderAccountError::MissingChatgptAccountDetails),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use codex_auth_types::AuthMode;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn model_provider_auth_manager_adapts_api_key_auth() {
        let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("sk-test"));
        let provider_auth =
            model_provider_auth_manager(Some(auth_manager)).expect("auth manager should adapt");

        assert_eq!(
            provider_auth.account().unwrap(),
            Some(ProviderAccount::ApiKey)
        );
        assert_eq!(
            provider_auth.auth_cached().unwrap().auth_mode(),
            AuthMode::ApiKey
        );
        assert!(!provider_auth.current_auth_uses_codex_backend());
    }
}
