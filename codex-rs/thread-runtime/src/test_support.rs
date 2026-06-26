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
#[cfg(any(test, feature = "test-support"))]
use codex_thread_api::SessionMcpToolCallHost;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_api::SessionToolRouter;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime_api::AnyToolResult;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime_api::registered_tool;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime::ToolRegistryBuilder;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_runtime::ToolRouter;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolDispatchRequest;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolDiffConsumerRequest;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolServiceApi;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolParallelRequest;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolServiceFuture;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_service_api::ToolSpecRequest;
#[cfg(any(test, feature = "test-support"))]
use codex_tool_types::FunctionCallError;
use http::HeaderMap;
use tokio::time::timeout;

use crate::ThreadAuthRuntimes;
use crate::ThreadService;
#[cfg(any(test, feature = "test-support"))]
use crate::SharedTurnDiffTracker;
use crate::config::Config;
#[cfg(any(test, feature = "test-support"))]
use crate::session::session::Session;
#[cfg(any(test, feature = "test-support"))]
use crate::session::turn_context::TurnContext;
use crate::thread;

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

pub fn set_thread_service_test_mode(enabled: bool) {
    thread::set_thread_service_test_mode_for_tests(enabled);
}

pub fn set_deterministic_process_ids(enabled: bool) {
    codex_command_service::set_deterministic_process_ids_for_tests(enabled);
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
) -> codex_protocol::error::Result<crate::NewThread> {
    thread_service
        .start_thread_with_user_shell_override_for_tests(config, user_shell_override)
        .await
}

pub async fn resume_thread_from_rollout_with_user_shell_override(
    thread_service: &ThreadService,
    config: Config,
    rollout_path: PathBuf,
    user_shell_override: crate::runtime_shell_model::Shell,
) -> codex_protocol::error::Result<crate::NewThread> {
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
    collaboration_mode_presets::builtin_collaboration_mode_presets()
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct TestToolService;

#[cfg(any(test, feature = "test-support"))]
impl TestToolService {
    fn build_inner(
        &self,
        request: ToolSpecRequest<'_>,
    ) -> Arc<dyn SessionToolRouter<SharedTurnDiffTracker, TurnContext>> {
        let turn = Arc::clone(&request.turn)
            .into_any_arc()
            .downcast::<TurnContext>()
            .expect("test tool service requires TurnContext");
        let session = Arc::clone(&request.session)
            .into_any_arc()
            .downcast::<Session>()
            .unwrap_or_else(|_| turn.session_arc());

        let mut builder =
            ToolRegistryBuilder::<
                codex_tool_runtime::ToolInvocation<
                    Arc<Session>,
                    Arc<TurnContext>,
                    SharedTurnDiffTracker,
                >,
                TurnContext,
            >::new();

        let multi_environment =
            request.config.environment_mode == codex_tool_planning::ToolEnvironmentMode::Multiple;
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::ApplyPatchHandler::with_host(
                multi_environment,
                crate::thread_capability_tool_host(),
            ),
        )))
        .expect("register apply_patch");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::ExecCommandHandler::new(
                crate::thread_capability_tool_host(),
                codex_tool_handlers::ExecCommandHandlerOptions {
                    allow_login_shell: request.config.allow_login_shell,
                    exec_permission_approvals_enabled: request
                        .config
                        .exec_permission_approvals_enabled,
                    include_environment_id: multi_environment,
                },
            ),
        )))
        .expect("register exec_command");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::CommandWaitHandler::new(),
        )))
        .expect("register command_wait");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::WriteStdinHandler::new(),
        )))
        .expect("register command_write_stdin");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::PlanHandler::new(),
        )))
        .expect("register update_plan");
        if request.config.request_permissions_tool_enabled {
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::RequestPermissionsHandler::new(),
            )))
            .expect("register request_permissions");
        }
        if !request.config.request_user_input_available_modes.is_empty() {
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::RequestUserInputHandler::new(
                    request.config.request_user_input_available_modes.clone(),
                ),
            )))
            .expect("register request_user_input");
        }
        if request.config.goal_tools {
            let goal_service = Arc::new(crate::GoalService);
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::GetGoalHandler::new(goal_service.clone()),
            )))
            .expect("register get_goal");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::CreateGoalHandler::new(goal_service.clone()),
            )))
            .expect("register create_goal");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::UpdateGoalHandler::new(goal_service),
            )))
            .expect("register update_goal");
        }

        if request.config.multi_agent_v2 {
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::SpawnAgentHandler::new(
                    codex_tool_planning::SpawnAgentToolOptions {
                        available_models: request.config.available_models.clone(),
                        agent_type_description: request.config.agent_type_description.clone(),
                        hide_agent_type_model_reasoning: request
                            .config
                            .hide_spawn_agent_metadata,
                        include_usage_hint: request.config.spawn_agent_usage_hint,
                        usage_hint_text: request.config.spawn_agent_usage_hint_text.clone(),
                        max_concurrent_threads_per_session: request
                            .config
                            .max_concurrent_threads_per_session,
                    },
                ),
            )))
            .expect("register spawn_agent");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::FollowupTaskHandler::new(),
            )))
            .expect("register followup_task");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::WaitAgentHandler::new(),
            )))
            .expect("register wait_agent");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::CloseAgentHandler::new(),
            )))
            .expect("register close_agent");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::ListAgentsHandler::new(),
            )))
            .expect("register list_agents");
        }

        if request.config.agent_jobs_tools {
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::SpawnAgentsOnCsvHandler::new(),
            )))
            .expect("register spawn_agents_on_csv");
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::ReportAgentJobResultHandler::new(),
            )))
            .expect("register report_agent_job_result");
        }

        let mcp_host = SessionMcpToolCallHost::<
            Session,
            Arc<TurnContext>,
            SharedTurnDiffTracker,
            TurnContext,
        >::default();
        if let Some(mcp_tools) = request.params.mcp_tools {
            for tool in mcp_tools {
                builder
                    .register_tool(registered_tool(Arc::new(
                        codex_tool_handlers::McpHandler::new(mcp_host, tool.clone()),
                    )))
                    .expect("register mcp tool");
            }
        }
        if let Some(deferred_mcp_tools) = request.params.deferred_mcp_tools {
            for tool in deferred_mcp_tools {
                builder
                    .register_tool(registered_tool(Arc::new(
                        codex_tool_handlers::McpHandler::with_exposure(
                            mcp_host,
                            tool.clone(),
                            codex_tool_types::ToolExposure::Deferred,
                        ),
                    )))
                    .expect("register deferred mcp tool");
            }
        }

        let mcp_resource_service = Arc::new(crate::McpResourceService);
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::ListMcpResourcesHandler::new(mcp_resource_service.clone()),
        )))
        .expect("register list_mcp_resources");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::ListMcpResourceTemplatesHandler::new(
                mcp_resource_service.clone(),
            ),
        )))
        .expect("register list_mcp_resource_templates");
        builder.register_tool(registered_tool(Arc::new(
            codex_tool_handlers::ReadMcpResourceHandler::new(mcp_resource_service),
        )))
        .expect("register read_mcp_resource");

        if let Some(discoverable_tools) = request.params.discoverable_tools {
            builder.register_tool(registered_tool(Arc::new(
                codex_tool_handlers::RequestPluginInstallHandler::new(
                    Arc::new(crate::RequestPluginInstallService),
                    discoverable_tools,
                ),
            )))
            .expect("register request_plugin_install");
        }

        for dynamic_tool in request.params.dynamic_tools {
            if let Some(handler) = codex_tool_handlers::DynamicToolHandler::new(dynamic_tool) {
                builder
                    .register_tool(registered_tool(Arc::new(handler)))
                    .expect("register dynamic tool");
            }
        }

        let (specs, registry) = builder.build();
        let router = ToolRouter::new(request.config.code_mode_only_enabled, specs, registry);
        Arc::new(codex_tool_handlers::SessionToolRouterAdapter::new(
            router,
            codex_tool_handlers::SessionToolDispatchHost::new(request.session_capability),
            session,
            turn,
        ))
    }

    fn incompatible_payload_error(tool_name: &codex_tool_types::ToolName) -> FunctionCallError {
        FunctionCallError::Fatal(format!("tool {tool_name} invoked with incompatible payload"))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ToolServiceApi for TestToolService {
    fn model_visible_specs(&self, request: ToolSpecRequest<'_>) -> Vec<codex_tool_types::ToolSpec> {
        self.build_inner(request).model_visible_specs()
    }

    fn create_diff_consumer(
        &self,
        request: ToolDiffConsumerRequest<'_>,
    ) -> Option<Box<dyn codex_tool_service_api::ErasedToolArgumentDiffConsumer>> {
        self.build_inner(request.tool)
            .create_diff_consumer(request.tool_name)
            .map(|consumer| {
                Box::new(codex_tool_service_api::TypedDiffConsumer::<TurnContext>::new(consumer))
                    as Box<dyn codex_tool_service_api::ErasedToolArgumentDiffConsumer>
            })
    }

    fn tool_supports_parallel(&self, request: ToolParallelRequest<'_>) -> bool {
        self.build_inner(request.tool)
            .tool_supports_parallel(request.call)
    }

    fn dispatch_tool(
        &self,
        request: ToolDispatchRequest<'_>,
    ) -> ToolServiceFuture<'_, Result<AnyToolResult, FunctionCallError>> {
        let inner = self.build_inner(request.tool);
        let cancellation_token = request.cancellation_token;
        let tracker = request.tracker;
        let call = request.call;
        let source = request.source;
        Box::pin(async move {
            inner
                .dispatch_tool_call_with_code_mode_result(
                    cancellation_token,
                    tracker,
                    call,
                    source,
                )
                .await
        })
    }
}
