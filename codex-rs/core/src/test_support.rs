//! Test-only helpers exposed for cross-crate integration tests.
//!
//! Production code should not depend on this module.
//! We prefer this to using a crate feature to avoid building multiple
//! permutations of the crate.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use codex_api::ModelsClient;
use codex_api::ReqwestTransport;
use codex_api_types::map_api_error;
use codex_default_client::build_reqwest_client;
use codex_exec_server_api::ExecEnvironmentProvider;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::model_provider_auth_manager;
use codex_model_provider_api::ModelProvider;
use codex_model_provider_api::ModelProviderFactory;
use codex_model_provider_api::ModelProviderFuture;
use codex_model_provider_api::ProviderAccountResult;
use codex_model_provider_api::ProviderAccountState;
use codex_model_provider_api::SharedModelProvider;
use codex_model_provider_api::SharedModelProviderAuthManager;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_model_provider_api::model_provider_info_to_api_provider;
use codex_model_provider_api::resolve_provider_auth;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager::bundled_models_response;
use codex_models_manager::collaboration_mode_presets;
use codex_models_manager::manager::ModelCachePolicy;
use codex_models_manager::manager::ModelsEndpointClient;
use codex_models_manager::manager::OpenAiModelsManager;
use codex_models_manager::manager::StaticModelsManager;
use codex_models_manager::test_support::construct_model_info_offline_for_tests;
use codex_models_manager::test_support::get_model_offline_for_tests;
use codex_models_manager_api::SharedModelsManager;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;
use http::HeaderMap;
use tokio::time::timeout;

use crate::ThreadAuthRuntimes;
use crate::ThreadManager;
use crate::config::Config;
use crate::thread_manager;
use crate::unified_exec;

static TEST_MODEL_PRESETS: LazyLock<Vec<ModelPreset>> = LazyLock::new(|| {
    let mut response = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    response.models.sort_by(|a, b| a.priority.cmp(&b.priority));
    let mut presets: Vec<ModelPreset> = response.models.into_iter().map(Into::into).collect();
    ModelPreset::mark_default_by_picker_visibility(&mut presets);
    presets
});

const TEST_MODELS_REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

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

    fn auth_manager(&self) -> Option<SharedModelProviderAuthManager> {
        self.auth_manager.clone()
    }

    fn supports_attestation(&self) -> bool {
        self.auth_manager
            .as_ref()
            .and_then(|auth_manager| auth_manager.auth_cached())
            .is_some_and(|auth| auth.is_chatgpt_auth())
    }

    fn auth(&self) -> ModelProviderFuture<'_, Option<codex_auth_types::RequestAuthSnapshot>> {
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
        codex_home: PathBuf,
        config_model_catalog: Option<ModelsResponse>,
    ) -> SharedModelsManager {
        match config_model_catalog {
            Some(model_catalog) => Arc::new(StaticModelsManager::new(
                self.auth_manager.clone(),
                model_catalog,
            )),
            None => {
                let endpoint = Arc::new(TestModelsEndpoint {
                    provider_info: self.info.clone(),
                    auth_manager: self.auth_manager.clone(),
                });
                if self.info.is_openai() {
                    Arc::new(OpenAiModelsManager::new(
                        codex_home,
                        endpoint,
                        self.auth_manager.clone(),
                    ))
                } else {
                    Arc::new(OpenAiModelsManager::new_with_fallback_models(
                        codex_home,
                        endpoint,
                        self.auth_manager.clone(),
                        Vec::new(),
                        ModelCachePolicy::Disabled,
                    ))
                }
            }
        }
    }
}

#[derive(Debug)]
struct TestModelsEndpoint {
    provider_info: ModelProviderInfo,
    auth_manager: Option<SharedModelProviderAuthManager>,
}

impl TestModelsEndpoint {
    async fn auth(&self) -> Option<codex_auth_types::RequestAuthSnapshot> {
        match self.auth_manager.as_ref() {
            Some(auth_manager) => auth_manager.auth().await,
            None => None,
        }
    }
}

impl ModelsEndpointClient for TestModelsEndpoint {
    fn has_command_auth(&self) -> bool {
        self.provider_info.has_command_auth()
    }

    fn uses_codex_backend<'life0, 'async_trait>(
        &'life0 self,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            self.auth()
                .await
                .as_ref()
                .is_some_and(codex_auth_types::RequestAuthSnapshot::uses_codex_backend)
        })
    }

    fn list_models<'life0, 'life1, 'async_trait>(
        &'life0 self,
        client_version: &'life1 str,
    ) -> Pin<
        Box<
            dyn Future<Output = CodexResult<(Vec<ModelInfo>, Option<String>)>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            let auth = self.auth().await;
            let auth_mode = auth
                .as_ref()
                .map(codex_auth_types::RequestAuthSnapshot::auth_mode);
            let api_provider = model_provider_info_to_api_provider(&self.provider_info, auth_mode);
            let api_auth = resolve_provider_auth(auth.as_ref(), &self.provider_info)?;
            let transport = ReqwestTransport::new(build_reqwest_client());
            let client = ModelsClient::new(transport, api_provider, api_auth);

            timeout(
                TEST_MODELS_REFRESH_TIMEOUT,
                client.list_models(client_version, HeaderMap::new()),
            )
            .await
            .map_err(|_| CodexErr::Timeout)?
            .map_err(map_api_error)
        })
    }
}

pub fn model_provider_factory_for_tests() -> SharedModelProviderFactory {
    Arc::new(TestModelProviderFactory)
}

pub fn create_model_provider_for_tests(
    provider: ModelProviderInfo,
    auth_manager: Option<Arc<AuthManager>>,
) -> SharedModelProvider {
    create_model_provider_for_tests_with_provider_auth(
        provider,
        model_provider_auth_manager(auth_manager),
    )
}

pub fn create_model_provider_for_tests_with_provider_auth(
    provider: ModelProviderInfo,
    provider_auth_manager: Option<SharedModelProviderAuthManager>,
) -> SharedModelProvider {
    TestModelProviderFactory.create_model_provider(provider, provider_auth_manager)
}

pub fn set_thread_manager_test_mode(enabled: bool) {
    thread_manager::set_thread_manager_test_mode_for_tests(enabled);
}

pub fn set_deterministic_process_ids(enabled: bool) {
    unified_exec::set_deterministic_process_ids_for_tests(enabled);
}

pub fn auth_manager_from_auth(auth: CodexAuth) -> Arc<AuthManager> {
    AuthManager::from_auth_for_testing(auth)
}

pub fn auth_manager_from_auth_with_home(auth: CodexAuth, codex_home: PathBuf) -> Arc<AuthManager> {
    AuthManager::from_auth_for_testing_with_home(auth, codex_home)
}

pub fn thread_auth_runtimes_from_auth(auth: CodexAuth) -> ThreadAuthRuntimes {
    thread_auth_runtimes_from_auth_manager(auth_manager_from_auth(auth))
}

pub fn thread_auth_runtimes_from_auth_manager(
    auth_manager: Arc<AuthManager>,
) -> ThreadAuthRuntimes {
    ThreadAuthRuntimes::from_auth_runtime(
        Arc::clone(&auth_manager),
        model_provider_auth_manager(Some(auth_manager)),
    )
}

pub fn thread_manager_with_models_provider(
    auth: CodexAuth,
    provider: ModelProviderInfo,
) -> ThreadManager {
    ThreadManager::with_models_provider_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
    )
}

pub fn thread_manager_with_models_provider_and_home(
    auth: CodexAuth,
    provider: ModelProviderInfo,
    codex_home: PathBuf,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
) -> ThreadManager {
    ThreadManager::with_models_provider_and_home_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
        codex_home,
        environment_manager,
    )
}

pub fn thread_manager_with_models_provider_home_and_state(
    auth: CodexAuth,
    provider: ModelProviderInfo,
    codex_home: PathBuf,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
    state_db: Option<crate::StateDbHandle>,
) -> ThreadManager {
    ThreadManager::with_models_provider_home_and_state_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
        codex_home,
        environment_manager,
        state_db,
    )
}

pub async fn start_thread_with_user_shell_override(
    thread_manager: &ThreadManager,
    config: Config,
    user_shell_override: crate::shell::Shell,
) -> codex_protocol::error::Result<crate::NewThread> {
    thread_manager
        .start_thread_with_user_shell_override_for_tests(config, user_shell_override)
        .await
}

pub async fn resume_thread_from_rollout_with_user_shell_override(
    thread_manager: &ThreadManager,
    config: Config,
    rollout_path: PathBuf,
    user_shell_override: crate::shell::Shell,
) -> codex_protocol::error::Result<crate::NewThread> {
    thread_manager
        .resume_thread_from_rollout_with_user_shell_override_for_tests(
            config,
            rollout_path,
            user_shell_override,
        )
        .await
}

pub fn models_manager_with_provider(
    codex_home: PathBuf,
    auth_manager: Arc<AuthManager>,
    provider: ModelProviderInfo,
) -> SharedModelsManager {
    let provider = create_model_provider_for_tests(provider, Some(auth_manager));
    provider.models_manager(codex_home, /*config_model_catalog*/ None)
}

pub fn models_manager_with_provider_auth(
    codex_home: PathBuf,
    provider_auth_manager: Option<SharedModelProviderAuthManager>,
    provider: ModelProviderInfo,
) -> SharedModelsManager {
    let provider =
        create_model_provider_for_tests_with_provider_auth(provider, provider_auth_manager);
    provider.models_manager(codex_home, /*config_model_catalog*/ None)
}

pub fn get_model_offline(model: Option<&str>) -> String {
    get_model_offline_for_tests(model)
}

pub fn construct_model_info_offline(model: &str, config: &Config) -> ModelInfo {
    construct_model_info_offline_for_tests(model, &config.to_models_manager_config())
}

pub fn all_model_presets() -> &'static Vec<ModelPreset> {
    &TEST_MODEL_PRESETS
}

pub fn builtin_collaboration_mode_presets() -> Vec<CollaborationModeMask> {
    collaboration_mode_presets::builtin_collaboration_mode_presets()
}
