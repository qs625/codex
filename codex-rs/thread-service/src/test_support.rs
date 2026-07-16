//! Test-only helpers exposed for cross-crate integration tests.
//!
//! Production code should not depend on this module.
//! We prefer this to using a crate feature to avoid building multiple
//! permutations of the crate.

use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

#[cfg(any(test, feature = "test-support"))]
use codex_code_mode_api::DisabledCodeModeRuntimeFactory;
#[cfg(any(test, feature = "test-support"))]
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::model_provider_auth_manager;
#[cfg(any(test, feature = "test-support"))]
use config_service::ConfigBuilder;
use exec_server_api::ExecEnvironmentProvider;
use http::HeaderMap;
use model_service::ModelsClient;
use model_service::builtin_collaboration_mode_presets as model_service_builtin_collaboration_mode_presets;
use model_service::bundled_models_response;
use model_service::manager::ModelCachePolicy;
use model_service::manager::ModelsEndpointClient;
use model_service::manager::OpenAiModelsManager;
use model_service::manager::StaticModelsManager;
use model_service::test_support::construct_model_info_offline_for_tests;
use model_service::test_support::get_model_offline_for_tests;
use model_service_api::ModelProvider;
use model_service_api::ModelProviderFactory;
use model_service_api::ModelProviderFuture;
use model_service_api::ModelProviderInfo;
use model_service_api::ProviderAccountResult;
use model_service_api::ProviderAccountState;
use model_service_api::SharedModelProvider;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelProviderFactory;
use model_service_api::SharedModelsManager;
use model_service_api::map_api_error;
use model_service_api::model_provider_info_to_api_provider;
use model_service_api::resolve_provider_auth;
use protocol::config_types::CollaborationModeMask;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelsResponse;
#[cfg(any(test, feature = "test-support"))]
use protocol::protocol::InitialHistory;
#[cfg(any(test, feature = "test-support"))]
use protocol::protocol::SessionSource;
#[cfg(any(test, feature = "test-support"))]
use state::StateRuntime;
#[cfg(any(test, feature = "test-support"))]
use thread_store::DefaultLiveThreadFactory;
#[cfg(any(test, feature = "test-support"))]
use thread_store::LocalThreadStore;
#[cfg(any(test, feature = "test-support"))]
use thread_store::LocalThreadStoreConfig;
use tokio::time::timeout;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::AnyToolResult;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::FunctionCallError;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolDiffConsumerRequest;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolDispatchRequest;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolParallelRequest;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolServiceApi;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolServiceFuture;
#[cfg(any(test, feature = "test-support"))]
use tool_service_api::ToolSpecRequest;
use transport_client::ReqwestTransport;
use transport_client::build_reqwest_client;

use crate::ThreadAuthRuntimes;
use crate::ThreadService;
#[cfg(any(test, feature = "test-support"))]
use crate::ThreadSession;
#[cfg(any(test, feature = "test-support"))]
use crate::ThreadTurnContext;
use crate::config::Config;
use crate::thread;

static TEST_MODEL_PRESETS: LazyLock<Vec<ModelPreset>> = LazyLock::new(|| {
    let mut response = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    response.models.sort_by_key(|a| a.priority);
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

pub fn set_thread_service_test_mode(enabled: bool) {
    thread::set_thread_service_test_mode_for_tests(enabled);
}

pub fn set_deterministic_process_ids(enabled: bool) {
    command_service::set_deterministic_process_ids_for_tests(enabled);
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
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    ThreadAuthRuntimes::from_auth_runtime(
        auth_runtime,
        model_provider_auth_manager(Some(auth_manager)),
    )
}

pub fn thread_service_with_models_provider(
    auth: CodexAuth,
    provider: ModelProviderInfo,
) -> ThreadService {
    ThreadService::with_models_provider_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
    )
}

pub fn thread_service_with_models_provider_and_home(
    auth: CodexAuth,
    provider: ModelProviderInfo,
    codex_home: PathBuf,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
) -> ThreadService {
    ThreadService::with_models_provider_and_home_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
        codex_home,
        environment_manager,
    )
}

pub fn thread_service_with_models_provider_home_and_state(
    auth: CodexAuth,
    provider: ModelProviderInfo,
    codex_home: PathBuf,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
    state_db: Option<crate::StateDbHandle>,
) -> ThreadService {
    ThreadService::with_models_provider_home_and_state_for_tests(
        auth,
        provider,
        model_provider_factory_for_tests(),
        codex_home,
        environment_manager,
        state_db,
    )
}

pub async fn start_thread_with_user_shell_override(
    thread_service: &ThreadService,
    config: Config,
    user_shell_override: crate::runtime_shell_model::Shell,
) -> protocol::error::Result<crate::NewThread> {
    thread_service
        .start_thread_with_user_shell_override_for_tests(config, user_shell_override)
        .await
}

pub async fn resume_thread_from_rollout_with_user_shell_override(
    thread_service: &ThreadService,
    config: Config,
    rollout_path: PathBuf,
    user_shell_override: crate::runtime_shell_model::Shell,
) -> protocol::error::Result<crate::NewThread> {
    thread_service
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
    model_service_builtin_collaboration_mode_presets()
}

#[cfg(any(test, feature = "test-support"))]
async fn build_test_config(codex_home: &Path) -> Config {
    ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .fallback_cwd(Some(codex_home.to_path_buf()))
        .build()
        .await
        .unwrap_or_else(|err| panic!("load default test config: {err}"))
}

#[cfg(any(test, feature = "test-support"))]
pub async fn make_session_and_context() -> (Arc<ThreadSession>, Arc<ThreadTurnContext>) {
    make_session_and_context_with(None, |_config| {}).await
}

#[cfg(any(test, feature = "test-support"))]
pub async fn make_session_and_context_with<F>(
    session_source: Option<SessionSource>,
    configure_config: F,
) -> (Arc<ThreadSession>, Arc<ThreadTurnContext>)
where
    F: FnOnce(&mut Config),
{
    make_session_and_context_with_auth(
        CodexAuth::from_api_key("Test API Key"),
        session_source,
        configure_config,
    )
    .await
}

#[cfg(any(test, feature = "test-support"))]
async fn make_session_and_context_with_auth<F>(
    auth: CodexAuth,
    session_source: Option<SessionSource>,
    configure_config: F,
) -> (Arc<ThreadSession>, Arc<ThreadTurnContext>)
where
    F: FnOnce(&mut Config),
{
    set_thread_service_test_mode(/*enabled*/ true);
    let codex_home = std::env::temp_dir().join(format!(
        "thread-service-test-support-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&codex_home)
        .unwrap_or_else(|err| panic!("create temp dir {codex_home:?}: {err}"));
    let mut config = build_test_config(&codex_home).await;
    configure_config(&mut config);
    let state_db: Option<state_api::SharedStateDbRuntime> =
        if config.features.enabled(Feature::Goals) {
            Some(
                StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                    .await
                    .unwrap_or_else(|err| {
                        panic!("goal tests should initialize sqlite state db: {err}")
                    }),
            )
        } else {
            None
        };
    let auth_manager = auth_manager_from_auth_with_home(auth, config.codex_home.to_path_buf());
    let environment_manager = Arc::new(codex_exec_server::EnvironmentManager::default_for_tests());
    let thread_store = Arc::new(LocalThreadStore::new(
        LocalThreadStoreConfig::from_config(&config),
        state_db.clone(),
    ));
    let thread_service = ThreadService::new(
        &config,
        thread_auth_runtimes_from_auth_manager(auth_manager),
        session_source.clone().unwrap_or(SessionSource::Exec),
        environment_manager.clone(),
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        /*analytics_events_client*/ None,
        thread_store,
        state_db,
        Arc::new(DefaultLiveThreadFactory),
        uuid::Uuid::new_v4().to_string(),
        /*attestation_provider*/ None,
        model_provider_factory_for_tests(),
        Arc::new(DisabledCodeModeRuntimeFactory),
        Arc::new(command_service::CommandService::new()),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
        Arc::new(mcp_service::McpService::new(Arc::new(
            approval_service::ApprovalService,
        ))),
    );
    let thread = thread_service
        .start_thread_with_options(crate::StartThreadOptions {
            config: config.clone(),
            initial_history: InitialHistory::New,
            session_source,
            agent_metadata: None,
            thread_source: None,
            dynamic_tools: Vec::new(),
            persist_extended_history: false,
            metrics_service_name: None,
            parent_trace: None,
            environments: thread_service.default_environment_selections(&config.cwd),
        })
        .await
        .unwrap_or_else(|err| panic!("start test thread: {err}"));
    let session = Arc::clone(&thread.thread.codex.session);
    let turn = session.new_default_turn().await;
    (session, turn)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
/// 仅用于不触发真实 tool dispatch 的测试场景。
///
/// 需要验证 tool 行为的测试不应注入这个类型，而应直接走真实 owner
/// service 的 dispatch 路径。
pub struct DisabledToolServiceForTests;

#[cfg(any(test, feature = "test-support"))]
impl ToolServiceApi for DisabledToolServiceForTests {
    fn model_visible_specs(&self, request: ToolSpecRequest<'_>) -> Vec<tool_service_api::ToolSpec> {
        let _ = request;
        Vec::new()
    }

    fn create_diff_consumer(
        &self,
        request: ToolDiffConsumerRequest<'_>,
    ) -> Option<Box<dyn tool_service_api::ErasedToolArgumentDiffConsumer>> {
        let _ = request;
        None
    }

    fn tool_supports_parallel(&self, request: ToolParallelRequest<'_>) -> bool {
        let _ = request;
        false
    }

    fn dispatch_tool(
        &self,
        request: ToolDispatchRequest<'_>,
    ) -> ToolServiceFuture<'_, Result<AnyToolResult, FunctionCallError>> {
        Box::pin(async move {
            Err(FunctionCallError::Fatal(format!(
                "DisabledToolServiceForTests does not dispatch {}",
                request.call.tool_name
            )))
        })
    }
}
