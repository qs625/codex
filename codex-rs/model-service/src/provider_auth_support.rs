use std::sync::Arc;

use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::RefreshTokenError;
use codex_login::UnauthorizedRecovery;
use model_service_api::ModelProviderAuthFuture;
use model_service_api::ModelProviderAuthManager;
use model_service_api::ModelProviderAuthRecoveryError;
use model_service_api::ModelProviderInfo;
use model_service_api::ModelProviderUnauthorizedRecovery;
use model_service_api::ModelProviderUnauthorizedRecoveryStepResult;
use model_service_api::ProviderAccountError;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::auth_provider_from_auth_snapshot;
pub(crate) use model_service_api::resolve_provider_auth;
use protocol::account::ProviderAccount;

/// Returns the provider-scoped auth manager when this provider uses command-backed auth.
///
/// Providers without custom auth continue using the caller-supplied base manager, when present.
pub(crate) fn auth_manager_for_provider(
    auth_manager: Option<SharedModelProviderAuthManager>,
    provider: &ModelProviderInfo,
) -> Option<SharedModelProviderAuthManager> {
    match provider.auth.clone() {
        Some(config) => Some(login_auth_manager(AuthManager::external_bearer_only(
            config,
        ))),
        None => auth_manager,
    }
}

pub(crate) fn login_auth_manager(auth_manager: Arc<AuthManager>) -> SharedModelProviderAuthManager {
    Arc::new(LoginModelProviderAuthManager { auth_manager })
}

pub fn auth_provider_from_auth(auth: &CodexAuth) -> model_service_api::SharedAuthProvider {
    auth_provider_from_auth_snapshot(&auth.request_auth_snapshot())
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
    use model_service_api::WireApi;
    use model_service_api::create_oss_provider_with_base_url;

    use super::*;

    #[test]
    fn unauthenticated_auth_provider_adds_no_headers() {
        let provider =
            create_oss_provider_with_base_url("http://localhost:11434/v1", WireApi::Responses);
        let auth = resolve_provider_auth(/*auth*/ None, &provider).expect("auth should resolve");

        assert!(auth.to_auth_headers().is_empty());
    }
}
