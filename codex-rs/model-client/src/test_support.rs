use std::path::PathBuf;
use std::sync::Arc;

use codex_auth_types::RequestAuthSnapshot;
use codex_model_provider_api::ModelProvider;
use codex_model_provider_api::ModelProviderFactory;
use codex_model_provider_api::ModelProviderFuture;
use codex_model_provider_api::ProviderAccountResult;
use codex_model_provider_api::ProviderAccountState;
use codex_model_provider_api::SharedModelProvider;
use codex_model_provider_api::SharedModelProviderAuthManager;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager_api::SharedModelsManager;
use codex_protocol::openai_models::ModelsResponse;

#[derive(Debug, Default)]
struct TestModelProviderFactory;

impl ModelProviderFactory for TestModelProviderFactory {
    fn create_model_provider(
        &self,
        provider_info: ModelProviderInfo,
        auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> SharedModelProvider {
        Arc::new(TestModelProvider {
            info: provider_info,
            auth_manager,
        })
    }
}

#[derive(Clone, Debug)]
struct TestModelProvider {
    info: ModelProviderInfo,
    auth_manager: Option<SharedModelProviderAuthManager>,
}

impl ModelProvider for TestModelProvider {
    fn info(&self) -> &ModelProviderInfo {
        &self.info
    }

    fn supports_attestation(&self) -> bool {
        self.auth_manager
            .as_ref()
            .and_then(|auth_manager| auth_manager.auth_cached())
            .is_some_and(|auth| auth.is_chatgpt_auth())
    }

    fn auth_manager(&self) -> Option<SharedModelProviderAuthManager> {
        self.auth_manager.clone()
    }

    fn auth(&self) -> ModelProviderFuture<'_, Option<RequestAuthSnapshot>> {
        Box::pin(async move {
            match self.auth_manager.as_ref() {
                Some(auth_manager) => auth_manager.auth().await,
                None => None,
            }
        })
    }

    fn account_state(&self) -> ProviderAccountResult {
        let account = if self.info.requires_openai_auth {
            self.auth_manager
                .as_ref()
                .map(|auth_manager| auth_manager.account())
                .transpose()?
                .flatten()
        } else {
            None
        };

        Ok(ProviderAccountState {
            account,
            requires_openai_auth: self.info.requires_openai_auth,
        })
    }

    fn models_manager(
        &self,
        _codex_home: PathBuf,
        _config_model_catalog: Option<ModelsResponse>,
    ) -> SharedModelsManager {
        unreachable!("model-client unit tests do not use provider model managers")
    }
}

pub fn model_provider_factory_for_tests() -> SharedModelProviderFactory {
    Arc::new(TestModelProviderFactory)
}
