use crate::StateDbHandle;
use crate::agent::AgentControl;
use crate::agent::status::is_final;
use crate::environment_selection::default_thread_environment_selections;
use crate::environment_selection::resolve_environment_selections;
use crate::session::Codex;
use crate::session::CodexSpawnArgs;
use crate::session::CodexSpawnOk;
use crate::session::INITIAL_SUBMIT_ID;
use crate::session::SteerInputError;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::tasks::interrupted_turn_history_marker_from_config;
use crate::thread::CodexThread;
use codex_agent_runtime::LiveAgentShutdownAction;
use codex_agent_runtime::live_agent_shutdown_action;
use codex_analytics_api::AnalyticsEventsClient;
use codex_api_runtime_api::DisabledApiRuntimeFactory;
use codex_api_runtime_api::SharedApiRuntimeFactory;
use codex_auth_types::AuthRuntime;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_config::Config;
use codex_config::skill_config_layer_stack_from_config_layer_stack;
use codex_core_plugins_api::DisabledPluginRuntime;
use codex_core_plugins_api::SharedPluginRuntime;
use codex_core_skills_api::DisabledSkillsRuntime;
use codex_core_skills_api::SharedSkillsRuntime;
use codex_core_skills_api::SkillsLoadInput;
#[cfg(any(test, feature = "test-support"))]
use codex_exec_server::EnvironmentManager;
use codex_exec_server_api::ExecEnvironmentProvider;
use codex_extension_api::ExtensionRegistry;
#[cfg(any(test, feature = "test-support"))]
use codex_extension_api::empty_extension_registry;
use codex_features::Feature;
use codex_hooks_api::DisabledHookRuntimeFactory;
use codex_hooks_api::SharedHookRuntimeFactory;
#[cfg(any(test, feature = "test-support"))]
use codex_login::AuthManager;
#[cfg(any(test, feature = "test-support"))]
use codex_login::CodexAuth;
#[cfg(any(test, feature = "test-support"))]
use codex_login::model_provider_auth_manager;
use codex_mcp_runtime::McpManager;
use codex_mcp_runtime_api::DisabledMcpAuthRuntime;
use codex_mcp_runtime_api::DisabledMcpConnectionRuntimeFactory;
use codex_mcp_runtime_api::McpAuthRuntime;
use codex_mcp_runtime_api::McpConnectionRuntimeFactory;
use codex_memories_read_api::DisabledMemoryToolDeveloperInstructionsProvider;
use codex_memories_read_api::SharedMemoryToolDeveloperInstructionsProvider;
use codex_model_client::AttestationProvider;
use codex_model_provider_api::ModelProviderFactory;
use codex_model_provider_api::SharedModelProviderAuthManager;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_model_provider_info::ModelProviderInfo;
#[cfg(any(test, feature = "test-support"))]
use codex_model_provider_info::OPENAI_PROVIDER_ID;
use codex_models_manager_api::RefreshStrategy;
use codex_models_manager_api::SharedModelsManager;
use codex_network_proxy_api::DisabledNetworkProxyRuntimeFactory;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::DisabledOpenAiFileUploader;
use codex_openai_files_api::SharedOpenAiFileUploader;
use codex_permissions_runtime::EmptyExecPolicyLoader;
use codex_permissions_runtime::ExecPolicyLoader;
use codex_protocol::ThreadId;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ResumedHistory;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::ThreadContextUsage;
use codex_protocol::protocol::ThreadSource;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::user_input::UserInput;
use codex_rollout_api::ForkSnapshot;
use codex_rollout_api::fork_history_from_snapshot;
use codex_sandboxing_api::DisabledSandboxRuntime;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_session_telemetry_api::DisabledSessionTelemetryFactory;
use codex_session_telemetry_api::SharedSessionTelemetryFactory;
use codex_state_api::DirectionalThreadSpawnEdgeStatus;
use codex_state_api::ExternalGoalSet;
use codex_thread_api::ActiveEventSubscriptionTracker;
use codex_thread_api::ThreadCreatedEvent;
use codex_thread_api::ThreadShutdownReport;
use codex_thread_api::ThreadSkillWatchPath;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_store::LocalThreadStore;
#[cfg(any(test, feature = "test-support"))]
use codex_thread_store::LocalThreadStoreConfig;
use codex_thread_store_api::LiveThreadFactory;
use codex_thread_store_api::ReadThreadByRolloutPathParams;
use codex_thread_store_api::ReadThreadParams;
use codex_thread_store_api::StoredThread;
use codex_thread_store_api::StoredThreadHistory;
use codex_thread_store_api::ThreadMetadataPatch;
use codex_thread_store_api::ThreadStore;
use codex_thread_store_api::ThreadStoreError;
use codex_thread_store_api::ThreadStoreResult;
use codex_thread_store_api::UpdateThreadMetadataParams;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_workflow_api::WorkflowRunController;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use tracing::warn;

const THREAD_CREATED_CHANNEL_CAPACITY: usize = 1024;
const DEFAULT_TERMINAL_TYPE: &str = "unknown";
/// Test-only override for enabling thread service behaviors used by integration
/// tests.
///
/// In production builds this value should remain at its default (`false`) and
/// must not be toggled.
static FORCE_TEST_THREAD_SERVICE_BEHAVIOR: AtomicBool = AtomicBool::new(false);

type CapturedOps = Vec<(ThreadId, Op)>;
type SharedCapturedOps = Arc<std::sync::Mutex<CapturedOps>>;

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn set_thread_service_test_mode_for_tests(enabled: bool) {
    FORCE_TEST_THREAD_SERVICE_BEHAVIOR.store(enabled, Ordering::Relaxed);
}

fn should_use_test_thread_service_behavior() -> bool {
    FORCE_TEST_THREAD_SERVICE_BEHAVIOR.load(Ordering::Relaxed)
}

struct TempCodexHomeGuard {
    path: PathBuf,
}

impl Drop for TempCodexHomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Represents a newly created Codex thread (formerly called a conversation), including the first event
/// (which is [`EventMsg::SessionConfigured`]).
pub struct NewThread {
    pub thread_id: ThreadId,
    pub thread: Arc<CodexThread>,
    pub session_configured: SessionConfiguredEvent,
}

enum ShutdownOutcome {
    Complete,
    SubmitFailed,
    TimedOut,
}

/// [`ThreadService`] is responsible for creating threads and maintaining
/// them in memory.
pub struct ThreadService {
    state: Arc<ThreadServiceState>,
    _test_codex_home_guard: Option<TempCodexHomeGuard>,
}

pub struct StartThreadOptions {
    pub config: Config,
    pub initial_history: InitialHistory,
    pub session_source: Option<SessionSource>,
    pub thread_source: Option<ThreadSource>,
    pub dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
    pub persist_extended_history: bool,
    pub metrics_service_name: Option<String>,
    pub parent_trace: Option<W3cTraceContext>,
    pub environments: Vec<TurnEnvironmentSelection>,
}

/// Authentication runtime handles needed by thread/session construction.
///
/// The concrete login runtime owns token storage and refresh behavior. Thread
/// management should only keep these projected traits so `codex-core` does not
/// depend on the full login implementation in its normal dependency graph.
#[derive(Clone)]
pub struct ThreadAuthRuntimes {
    pub auth_runtime: SharedAuthRuntime,
    pub provider_auth_manager: Option<SharedModelProviderAuthManager>,
}

impl ThreadAuthRuntimes {
    pub fn new(
        auth_runtime: SharedAuthRuntime,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> Self {
        Self {
            auth_runtime,
            provider_auth_manager,
        }
    }

    pub fn from_auth_runtime<T>(
        auth_runtime: Arc<T>,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> Self
    where
        T: AuthRuntime + 'static,
    {
        let auth_runtime: SharedAuthRuntime = auth_runtime;
        Self::new(auth_runtime, provider_auth_manager)
    }
}

pub(crate) struct ResumeThreadWithHistoryOptions {
    pub(crate) config: Config,
    pub(crate) initial_history: InitialHistory,
    pub(crate) agent_control: AgentControl,
    pub(crate) session_source: SessionSource,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<codex_permissions_runtime::ExecPolicyManager>>,
}

/// Shared, `Arc`-owned state for [`ThreadService`]. This `Arc` is required to have a single
/// `Arc` reference that can be downgraded to by `AgentControl` while preventing every single
/// function to require an `Arc<&Self>`.
pub(crate) struct ThreadServiceState {
    threads: Arc<RwLock<HashMap<ThreadId, Arc<CodexThread>>>>,
    thread_created_tx: broadcast::Sender<ThreadCreatedEvent>,
    auth_runtime: SharedAuthRuntime,
    provider_auth_manager: Option<SharedModelProviderAuthManager>,
    models_manager: SharedModelsManager,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
    skills_manager: SharedSkillsRuntime,
    plugin_runtime: SharedPluginRuntime,
    mcp_manager: Arc<McpManager>,
    mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    api_runtime_factory: SharedApiRuntimeFactory,
    network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    sandbox_runtime: SharedSandboxRuntime,
    session_telemetry_factory: SharedSessionTelemetryFactory,
    hook_runtime_factory: SharedHookRuntimeFactory,
    memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
    extensions: Arc<ExtensionRegistry<Config>>,
    thread_store: Arc<dyn ThreadStore>,
    live_thread_factory: Arc<dyn LiveThreadFactory>,
    attestation_provider: Option<Arc<dyn AttestationProvider>>,
    session_source: SessionSource,
    terminal_type: StdRwLock<String>,
    installation_id: String,
    analytics_events_client: Option<AnalyticsEventsClient>,
    state_db: Option<StateDbHandle>,
    active_event_subscriptions: Arc<ActiveEventSubscriptionTracker>,
    model_provider_factory: SharedModelProviderFactory,
    code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    openai_file_uploader: SharedOpenAiFileUploader,
    exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    tool_service: Arc<crate::CoreToolServiceApi>,
    workflow_runs: Arc<dyn WorkflowRunController>,
    // Captures submitted ops for testing purpose when test mode is enabled.
    ops_log: Option<SharedCapturedOps>,
}

#[allow(dead_code)]

pub fn build_models_manager(
    config: &Config,
    provider_auth_manager: Option<SharedModelProviderAuthManager>,
    model_provider_factory: &dyn ModelProviderFactory,
) -> SharedModelsManager {
    let provider = model_provider_factory
        .create_model_provider(config.model_provider.clone(), provider_auth_manager);
    provider.models_manager(
        config.codex_home.to_path_buf(),
        config.model_catalog.clone(),
    )
}

impl ThreadService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &Config,
        auth_runtimes: ThreadAuthRuntimes,
        session_source: SessionSource,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        extensions: Arc<ExtensionRegistry<Config>>,
        analytics_events_client: Option<AnalyticsEventsClient>,
        thread_store: Arc<dyn ThreadStore>,
        state_db: Option<StateDbHandle>,
        live_thread_factory: Arc<dyn LiveThreadFactory>,
        installation_id: String,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
        model_provider_factory: SharedModelProviderFactory,
        code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
        tool_service: Arc<crate::CoreToolServiceApi>,
    ) -> Self {
        Self::new_with_mcp_auth_runtime(
            config,
            auth_runtimes,
            session_source,
            environment_manager,
            extensions,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            installation_id,
            attestation_provider,
            model_provider_factory,
            code_mode_runtime_factory,
            tool_service,
            Arc::new(DisabledMcpAuthRuntime),
            Arc::new(DisabledMcpConnectionRuntimeFactory),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_mcp_auth_runtime(
        config: &Config,
        auth_runtimes: ThreadAuthRuntimes,
        session_source: SessionSource,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        extensions: Arc<ExtensionRegistry<Config>>,
        analytics_events_client: Option<AnalyticsEventsClient>,
        thread_store: Arc<dyn ThreadStore>,
        state_db: Option<StateDbHandle>,
        live_thread_factory: Arc<dyn LiveThreadFactory>,
        installation_id: String,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
        model_provider_factory: SharedModelProviderFactory,
        code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
        tool_service: Arc<crate::CoreToolServiceApi>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    ) -> Self {
        Self::new_with_workflow_runs(
            config,
            auth_runtimes,
            session_source,
            environment_manager,
            extensions,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            installation_id,
            attestation_provider,
            model_provider_factory,
            code_mode_runtime_factory,
            tool_service,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            Arc::new(codex_workflow_api::DisabledWorkflowRunController),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_workflow_runs(
        config: &Config,
        auth_runtimes: ThreadAuthRuntimes,
        session_source: SessionSource,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        extensions: Arc<ExtensionRegistry<Config>>,
        analytics_events_client: Option<AnalyticsEventsClient>,
        thread_store: Arc<dyn ThreadStore>,
        state_db: Option<StateDbHandle>,
        live_thread_factory: Arc<dyn LiveThreadFactory>,
        installation_id: String,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
        model_provider_factory: SharedModelProviderFactory,
        code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
        tool_service: Arc<crate::CoreToolServiceApi>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
        workflow_runs: Arc<dyn WorkflowRunController>,
    ) -> Self {
        Self::new_with_workflow_runs_and_openai_file_uploader(
            config,
            auth_runtimes,
            session_source,
            environment_manager,
            extensions,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            installation_id,
            attestation_provider,
            model_provider_factory,
            code_mode_runtime_factory,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            workflow_runs,
            Arc::new(DisabledOpenAiFileUploader),
            Arc::new(EmptyExecPolicyLoader),
            Arc::new(DisabledApiRuntimeFactory),
            Arc::new(DisabledNetworkProxyRuntimeFactory),
            Arc::new(DisabledSandboxRuntime),
            Arc::new(DisabledSessionTelemetryFactory),
            Arc::new(DisabledHookRuntimeFactory),
            Arc::new(DisabledMemoryToolDeveloperInstructionsProvider),
            Arc::new(DisabledSkillsRuntime),
            Arc::new(DisabledPluginRuntime),
            tool_service,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_workflow_runs_and_openai_file_uploader(
        config: &Config,
        auth_runtimes: ThreadAuthRuntimes,
        session_source: SessionSource,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        extensions: Arc<ExtensionRegistry<Config>>,
        analytics_events_client: Option<AnalyticsEventsClient>,
        thread_store: Arc<dyn ThreadStore>,
        state_db: Option<StateDbHandle>,
        live_thread_factory: Arc<dyn LiveThreadFactory>,
        installation_id: String,
        attestation_provider: Option<Arc<dyn AttestationProvider>>,
        model_provider_factory: SharedModelProviderFactory,
        code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
        workflow_runs: Arc<dyn WorkflowRunController>,
        openai_file_uploader: SharedOpenAiFileUploader,
        exec_policy_loader: Arc<dyn ExecPolicyLoader>,
        api_runtime_factory: SharedApiRuntimeFactory,
        network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
        sandbox_runtime: SharedSandboxRuntime,
        session_telemetry_factory: SharedSessionTelemetryFactory,
        hook_runtime_factory: SharedHookRuntimeFactory,
        memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
        skills_runtime: SharedSkillsRuntime,
        plugin_runtime: SharedPluginRuntime,
        tool_service: Arc<crate::CoreToolServiceApi>,
    ) -> Self {
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let mcp_manager = Arc::new(McpManager::new(plugin_runtime.clone()));
        let ThreadAuthRuntimes {
            auth_runtime,
            provider_auth_manager,
        } = auth_runtimes;
        Self {
            state: Arc::new(ThreadServiceState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: build_models_manager(
                    config,
                    provider_auth_manager.clone(),
                    model_provider_factory.as_ref(),
                ),
                provider_auth_manager,
                environment_manager,
                skills_manager: skills_runtime,
                plugin_runtime,
                mcp_manager,
                mcp_auth_runtime,
                mcp_connection_runtime_factory,
                api_runtime_factory,
                network_proxy_runtime_factory,
                sandbox_runtime,
                session_telemetry_factory,
                hook_runtime_factory,
                memory_tool_developer_instructions_provider,
                extensions,
                thread_store,
                live_thread_factory,
                attestation_provider,
                auth_runtime,
                session_source,
                terminal_type: StdRwLock::new(DEFAULT_TERMINAL_TYPE.to_string()),
                installation_id,
                analytics_events_client,
                state_db,
                active_event_subscriptions: Arc::new(ActiveEventSubscriptionTracker::default()),
                model_provider_factory,
                code_mode_runtime_factory,
                openai_file_uploader,
                exec_policy_loader,
                tool_service,
                workflow_runs,
                ops_log: should_use_test_thread_service_behavior()
                    .then(|| Arc::new(std::sync::Mutex::new(Vec::new()))),
            }),
            _test_codex_home_guard: None,
        }
    }

    /// Construct with a dummy AuthManager containing the provided CodexAuth.
    /// Used for integration tests: should not be used by ordinary business logic.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_models_provider_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        model_provider_factory: SharedModelProviderFactory,
    ) -> Self {
        set_thread_service_test_mode_for_tests(/*enabled*/ true);
        let codex_home = std::env::temp_dir().join(format!(
            "codex-thread-service-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&codex_home)
            .unwrap_or_else(|err| panic!("temp codex home dir create failed: {err}"));
        let mut manager = Self::with_models_provider_and_home_for_tests(
            auth,
            provider,
            model_provider_factory,
            codex_home.clone(),
            Arc::new(EnvironmentManager::default_for_tests()),
        );
        manager._test_codex_home_guard = Some(TempCodexHomeGuard { path: codex_home });
        manager
    }

    /// Construct with a dummy AuthManager containing the provided CodexAuth and codex home.
    /// Used for integration tests: should not be used by ordinary business logic.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_models_provider_and_home_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        model_provider_factory: SharedModelProviderFactory,
        codex_home: PathBuf,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
    ) -> Self {
        Self::with_models_provider_home_and_state_for_tests(
            auth,
            provider,
            model_provider_factory,
            codex_home,
            environment_manager,
            /*state_db*/ None,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_models_provider_home_and_state_for_tests(
        auth: CodexAuth,
        provider: ModelProviderInfo,
        model_provider_factory: SharedModelProviderFactory,
        codex_home: PathBuf,
        environment_manager: Arc<dyn ExecEnvironmentProvider>,
        state_db: Option<StateDbHandle>,
    ) -> Self {
        set_thread_service_test_mode_for_tests(/*enabled*/ true);
        let auth_manager = AuthManager::from_auth_for_testing(auth);
        let installation_id = uuid::Uuid::new_v4().to_string();
        let skills_codex_home = match AbsolutePathBuf::from_absolute_path_checked(&codex_home) {
            Ok(codex_home) => codex_home,
            Err(err) => panic!("test codex_home should be absolute: {err}"),
        };
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let restriction_product = SessionSource::Exec.restriction_product();
        let plugin_runtime: SharedPluginRuntime = Arc::new(DisabledPluginRuntime);
        let mcp_manager = Arc::new(McpManager::new(plugin_runtime.clone()));
        let skills_manager: SharedSkillsRuntime = Arc::new(
            codex_core_skills::SkillsManager::new_with_restriction_product(
                skills_codex_home,
                /*bundled_skills_enabled*/ true,
                restriction_product,
            ),
        );
        // This test constructor has no Config input. Tests that need a non-local
        // process store should construct ThreadService::new with an explicit store.
        let thread_store: Arc<dyn ThreadStore> = Arc::new(LocalThreadStore::new(
            LocalThreadStoreConfig {
                codex_home: codex_home.clone(),
                sqlite_home: codex_home.clone(),
                default_model_provider_id: OPENAI_PROVIDER_ID.to_string(),
            },
            state_db.clone(),
        ));
        let provider_auth_manager = model_provider_auth_manager(Some(Arc::clone(&auth_manager)));
        let auth_runtime: SharedAuthRuntime = auth_manager;
        Self {
            state: Arc::new(ThreadServiceState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                thread_created_tx,
                models_manager: model_provider_factory
                    .create_model_provider(provider, provider_auth_manager.clone())
                    .models_manager(codex_home, /*config_model_catalog*/ None),
                provider_auth_manager,
                environment_manager,
                skills_manager,
                plugin_runtime,
                mcp_manager,
                mcp_auth_runtime: Arc::new(codex_mcp::DefaultMcpAuthRuntime),
                mcp_connection_runtime_factory: Arc::new(
                    codex_mcp::DefaultMcpConnectionRuntimeFactory,
                ),
                api_runtime_factory: Arc::new(DisabledApiRuntimeFactory),
                network_proxy_runtime_factory: Arc::new(
                    codex_network_proxy::DefaultNetworkProxyRuntimeFactory,
                ),
                sandbox_runtime: Arc::new(DisabledSandboxRuntime),
                session_telemetry_factory: Arc::new(DisabledSessionTelemetryFactory),
                hook_runtime_factory: Arc::new(DisabledHookRuntimeFactory),
                memory_tool_developer_instructions_provider: Arc::new(
                    DisabledMemoryToolDeveloperInstructionsProvider,
                ),
                extensions: empty_extension_registry(),
                thread_store,
                live_thread_factory: Arc::new(codex_thread_store::DefaultLiveThreadFactory),
                attestation_provider: None,
                auth_runtime,
                session_source: SessionSource::Exec,
                terminal_type: StdRwLock::new(DEFAULT_TERMINAL_TYPE.to_string()),
                installation_id,
                analytics_events_client: None,
                state_db,
                active_event_subscriptions: Arc::new(ActiveEventSubscriptionTracker::default()),
                model_provider_factory,
                code_mode_runtime_factory: Arc::new(
                    codex_code_mode_api::DisabledCodeModeRuntimeFactory,
                ),
                openai_file_uploader: Arc::new(DisabledOpenAiFileUploader),
                exec_policy_loader: Arc::new(EmptyExecPolicyLoader),
                tool_service: Arc::new(crate::test_support::TestToolService),
                workflow_runs: Arc::new(codex_workflow_api::DisabledWorkflowRunController),
                ops_log: should_use_test_thread_service_behavior()
                    .then(|| Arc::new(std::sync::Mutex::new(Vec::new()))),
            }),
            _test_codex_home_guard: None,
        }
    }

    pub fn session_source(&self) -> SessionSource {
        self.state.session_source.clone()
    }

    pub fn with_terminal_type(self, terminal_type: impl Into<String>) -> Self {
        self.state.set_terminal_type(terminal_type.into());
        self
    }

    pub fn active_event_subscriptions(&self) -> Arc<ActiveEventSubscriptionTracker> {
        Arc::clone(&self.state.active_event_subscriptions)
    }

    pub async fn maybe_notify_parent_of_final_status(&self, thread_id: ThreadId) {
        let Ok(thread) = self.state.get_thread(thread_id).await else {
            return;
        };
        Box::pin(
            thread
                .codex
                .session
                .maybe_notify_parent_of_final_status_for_current_source(),
        )
        .await;
    }

    pub fn auth_runtime(&self) -> SharedAuthRuntime {
        self.state.auth_runtime.clone()
    }

    pub fn skills_manager(&self) -> SharedSkillsRuntime {
        self.state.skills_manager.clone()
    }

    pub fn plugin_runtime(&self) -> SharedPluginRuntime {
        self.state.plugin_runtime.clone()
    }

    pub fn mcp_manager(&self) -> Arc<McpManager> {
        self.state.mcp_manager.clone()
    }

    pub fn environment_provider(&self) -> Arc<dyn ExecEnvironmentProvider> {
        self.state.environment_manager.clone()
    }

    pub fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection> {
        default_thread_environment_selections(self.state.environment_manager.as_ref(), cwd)
    }

    pub fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()> {
        resolve_environment_selections(self.state.environment_manager.as_ref(), environments)
            .map(|_| ())
    }

    pub fn get_models_manager(&self) -> SharedModelsManager {
        self.state.models_manager.clone()
    }

    pub async fn list_models(&self, refresh_strategy: RefreshStrategy) -> Vec<ModelPreset> {
        self.state
            .models_manager
            .list_models(refresh_strategy)
            .await
    }

    pub async fn list_models_for_provider(
        &self,
        config: &Config,
        provider_info: ModelProviderInfo,
        model_catalog: Option<ModelsResponse>,
        refresh_strategy: RefreshStrategy,
    ) -> Vec<ModelPreset> {
        let mut config = config.clone();
        config.model_provider = provider_info;
        config.model_catalog = model_catalog;
        build_models_manager(
            &config,
            self.state.provider_auth_manager.clone(),
            self.state.model_provider_factory.as_ref(),
        )
        .list_models(refresh_strategy)
        .await
    }

    pub fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        self.state.models_manager.list_collaboration_modes()
    }

    pub async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.state.list_thread_ids().await
    }

    pub fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadCreatedEvent> {
        self.state.thread_created_tx.subscribe()
    }

    pub async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        self.state.get_thread(thread_id).await
    }

    pub async fn live_thread_config(&self, thread_id: ThreadId) -> CodexResult<Arc<Config>> {
        let thread = self.state.get_thread(thread_id).await?;
        Ok(thread.config().await)
    }

    pub async fn refresh_live_threads_runtime_config(&self, next_config: Config) {
        for thread_id in self.state.list_thread_ids().await {
            let Ok(thread) = self.state.get_thread(thread_id).await else {
                continue;
            };
            thread.refresh_runtime_config(next_config.clone()).await;
        }
    }

    pub async fn read_thread_mcp_resource(
        &self,
        thread_id: ThreadId,
        server: &str,
        uri: &str,
    ) -> anyhow::Result<serde_json::Value> {
        let thread = self.state.get_thread(thread_id).await?;
        thread.read_mcp_resource(server, uri).await
    }

    pub async fn call_thread_mcp_tool(
        &self,
        thread_id: ThreadId,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> anyhow::Result<CallToolResult> {
        let thread = self.state.get_thread(thread_id).await?;
        thread.call_mcp_tool(server, tool, arguments, meta).await
    }

    pub async fn inject_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> CodexResult<()> {
        let thread = self.state.get_thread(thread_id).await?;
        thread.inject_conversation_items(items).await
    }

    pub async fn steer_thread_input(
        &self,
        thread_id: ThreadId,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> CodexResult<Result<String, SteerInputError>> {
        let thread = self.state.get_thread(thread_id).await?;
        Ok(thread
            .steer_input(input, expected_turn_id, responsesapi_client_metadata)
            .await)
    }

    pub async fn read_thread(&self, params: ReadThreadParams) -> CodexResult<StoredThread> {
        self.state.read_stored_thread(params).await
    }

    /// Updates metadata for loaded and cold threads through one entrypoint.
    ///
    /// Loaded threads route through `CodexThread`/`LiveThread`, so metadata changes stay ordered
    /// with live rollout writes. Cold threads go directly to the store, which owns unloaded JSONL
    /// compatibility and SQLite metadata updates.
    pub async fn update_thread_metadata(
        &self,
        thread_id: ThreadId,
        patch: ThreadMetadataPatch,
        include_archived: bool,
    ) -> CodexResult<StoredThread> {
        if let Ok(thread) = self.get_thread(thread_id).await {
            if thread.config_snapshot().await.ephemeral {
                return Err(CodexErr::InvalidRequest(format!(
                    "ephemeral thread does not support metadata updates: {thread_id}"
                )));
            }
            return thread
                .update_thread_metadata(patch, include_archived)
                .await
                .map_err(|err| thread_store_metadata_update_error(thread_id, err));
        }
        self.state
            .thread_store
            .update_thread_metadata(UpdateThreadMetadataParams {
                thread_id,
                patch,
                include_archived,
            })
            .await
            .map_err(|err| match err {
                ThreadStoreError::ThreadNotFound { thread_id } => {
                    CodexErr::ThreadNotFound(thread_id)
                }
                err => thread_store_metadata_update_error(thread_id, err),
            })
    }

    /// List `thread_id` plus all known descendants in its spawn subtree.
    pub async fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        let thread = self.state.get_thread(thread_id).await?;

        let mut subtree_thread_ids = Vec::new();
        let mut seen_thread_ids = HashSet::new();
        subtree_thread_ids.push(thread_id);
        seen_thread_ids.insert(thread_id);

        if let Some(state_db_ctx) = thread.state_db() {
            for status in [
                DirectionalThreadSpawnEdgeStatus::Open,
                DirectionalThreadSpawnEdgeStatus::Closed,
            ] {
                for descendant_id in state_db_ctx
                    .list_thread_spawn_descendants_with_status(thread_id, status)
                    .await
                    .map_err(|err| {
                        CodexErr::Fatal(format!("failed to load thread-spawn descendants: {err}"))
                    })?
                {
                    if seen_thread_ids.insert(descendant_id) {
                        subtree_thread_ids.push(descendant_id);
                    }
                }
            }
        }

        for descendant_id in thread
            .codex
            .session
            .services
            .agent_control
            .list_live_agent_subtree_thread_ids(thread_id)
            .await?
        {
            if seen_thread_ids.insert(descendant_id) {
                subtree_thread_ids.push(descendant_id);
            }
        }

        Ok(subtree_thread_ids)
    }

    pub async fn start_thread(&self, config: Config) -> CodexResult<NewThread> {
        // Box delegated thread-spawn futures so these convenience wrappers do
        // not inline the full spawn path into every caller's async state.
        Box::pin(self.start_thread_with_tools(
            config,
            Vec::new(),
            /*persist_extended_history*/ false,
        ))
        .await
    }

    pub async fn start_thread_with_tools(
        &self,
        config: Config,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        Box::pin(self.start_thread_with_options(StartThreadOptions {
            config,
            initial_history: InitialHistory::New,
            session_source: None,
            thread_source: None,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name: None,
            parent_trace: None,
            environments,
        }))
        .await
    }

    pub async fn start_thread_with_options(
        &self,
        options: StartThreadOptions,
    ) -> CodexResult<NewThread> {
        let session_source = options
            .session_source
            .unwrap_or_else(|| self.state.session_source.clone());
        let thread_source = options
            .thread_source
            .or_else(|| options.initial_history.get_resumed_thread_source());
        Box::pin(self.state.spawn_thread_with_source(
            options.config,
            options.initial_history,
            self.agent_control(),
            session_source,
            thread_source,
            options.dynamic_tools,
            options.persist_extended_history,
            options.metrics_service_name,
            /*inherited_shell_snapshot*/ None,
            /*inherited_exec_policy*/ None,
            options.parent_trace,
            options.environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    // TODO(jif) merge with fork_agent
    /// Spawn a subagent by forking persisted history from `forked_from_thread_id`.
    pub async fn spawn_subagent(
        &self,
        forked_from_thread_id: ThreadId,
        mut options: StartThreadOptions,
    ) -> CodexResult<NewThread> {
        let fork_source = self.get_thread(forked_from_thread_id).await?;
        // Persist queued rollout updates before reading the fork snapshot.
        fork_source.ensure_rollout_materialized().await;
        fork_source.flush_rollout().await?;
        let stored_thread = fork_source
            .read_thread(
                /*include_archived*/ true, /*include_history*/ true,
            )
            .await
            .map_err(|err| {
                CodexErr::Fatal(format!(
                    "failed to read subagent fork source {forked_from_thread_id}: {err}"
                ))
            })?;
        let history = stored_thread_to_initial_history(stored_thread, fork_source.rollout_path())?;
        options.initial_history = fork_history_from_snapshot(
            ForkSnapshot::Interrupted,
            history,
            interrupted_turn_history_marker_from_config(&options.config),
        );
        self.start_thread_with_options(options).await
    }

    /// Fork a loaded live thread after first materializing and flushing its latest rollout.
    pub async fn fork_live_thread_from_current_history<S>(
        &self,
        forked_from_thread_id: ThreadId,
        snapshot: S,
        config: Config,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        let fork_source = self.get_thread(forked_from_thread_id).await?;
        fork_source.ensure_rollout_materialized().await;
        fork_source.flush_rollout().await?;
        let stored_thread = fork_source
            .read_thread(
                /*include_archived*/ true, /*include_history*/ true,
            )
            .await
            .map_err(|err| {
                CodexErr::Fatal(format!(
                    "failed to read fork source {forked_from_thread_id}: {err}"
                ))
            })?;
        let history = stored_thread_to_initial_history(stored_thread, fork_source.rollout_path())?;
        self.fork_thread_with_initial_history(
            snapshot.into(),
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    pub async fn resume_thread_from_rollout(
        &self,
        config: Config,
        rollout_path: PathBuf,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let initial_history = self.initial_history_from_rollout_path(rollout_path).await?;
        Box::pin(self.resume_thread_with_history(
            config,
            initial_history,
            /*persist_extended_history*/ false,
            parent_trace,
        ))
        .await
    }

    pub async fn resume_thread_with_history(
        &self,
        config: Config,
        initial_history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.state.spawn_thread(
            config,
            initial_history,
            self.agent_control(),
            thread_source,
            Vec::new(),
            persist_extended_history,
            /*metrics_service_name*/ None,
            parent_trace,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    pub async fn resume_thread_with_history_and_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        session_source: SessionSource,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.state.spawn_thread_with_source(
            config,
            initial_history,
            self.agent_control(),
            session_source,
            thread_source,
            Vec::new(),
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            /*inherited_shell_snapshot*/ None,
            /*inherited_exec_policy*/ None,
            parent_trace,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    pub(crate) async fn start_thread_with_user_shell_override_for_tests(
        &self,
        config: Config,
        user_shell_override: crate::runtime_shell_model::Shell,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        Box::pin(self.state.spawn_thread(
            config,
            InitialHistory::New,
            self.agent_control(),
            /*thread_source*/ None,
            Vec::new(),
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            /*parent_trace*/ None,
            environments,
            /*user_shell_override*/ Some(user_shell_override),
        ))
        .await
    }

    pub(crate) async fn resume_thread_from_rollout_with_user_shell_override_for_tests(
        &self,
        config: Config,
        rollout_path: PathBuf,
        user_shell_override: crate::runtime_shell_model::Shell,
    ) -> CodexResult<NewThread> {
        let initial_history = self.initial_history_from_rollout_path(rollout_path).await?;
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.state.spawn_thread(
            config,
            initial_history,
            self.agent_control(),
            thread_source,
            Vec::new(),
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            /*parent_trace*/ None,
            environments,
            /*user_shell_override*/ Some(user_shell_override),
        ))
        .await
    }

    /// Removes the thread from the manager's internal map, though the thread is stored
    /// as `Arc<CodexThread>`, it is possible that other references to it exist elsewhere.
    /// Returns the thread if the thread was found and removed.
    pub async fn remove_thread(&self, thread_id: &ThreadId) -> Option<Arc<CodexThread>> {
        self.state.threads.write().await.remove(thread_id)
    }

    /// Tries to shut down all tracked threads concurrently within the provided timeout.
    /// Threads that complete shutdown are removed from the manager; incomplete shutdowns
    /// remain tracked so callers can retry or inspect them later.
    pub async fn shutdown_all_threads_bounded(&self, timeout: Duration) -> ThreadShutdownReport {
        let threads = {
            let threads = self.state.threads.read().await;
            threads
                .iter()
                .map(|(thread_id, thread)| (*thread_id, Arc::clone(thread)))
                .collect::<Vec<_>>()
        };

        let mut shutdowns = threads
            .into_iter()
            .map(|(thread_id, thread)| async move {
                let outcome = match tokio::time::timeout(timeout, thread.shutdown_and_wait()).await
                {
                    Ok(Ok(())) => ShutdownOutcome::Complete,
                    Ok(Err(_)) => ShutdownOutcome::SubmitFailed,
                    Err(_) => ShutdownOutcome::TimedOut,
                };
                (thread_id, outcome)
            })
            .collect::<FuturesUnordered<_>>();
        let mut report = ThreadShutdownReport::default();

        while let Some((thread_id, outcome)) = shutdowns.next().await {
            match outcome {
                ShutdownOutcome::Complete => report.completed.push(thread_id),
                ShutdownOutcome::SubmitFailed => report.submit_failed.push(thread_id),
                ShutdownOutcome::TimedOut => report.timed_out.push(thread_id),
            }
        }

        let mut tracked_threads = self.state.threads.write().await;
        for thread_id in &report.completed {
            tracked_threads.remove(thread_id);
        }

        report
            .completed
            .sort_by_key(std::string::ToString::to_string);
        report
            .submit_failed
            .sort_by_key(std::string::ToString::to_string);
        report
            .timed_out
            .sort_by_key(std::string::ToString::to_string);
        report
    }

    /// Fork an existing thread by snapshotting rollout history according to
    /// `snapshot` and starting a new thread with identical configuration
    /// (unless overridden by the caller's `config`). The new thread will have
    /// a fresh id.
    pub async fn fork_thread<S>(
        &self,
        snapshot: S,
        config: Config,
        path: PathBuf,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        let snapshot = snapshot.into();
        let history = self.initial_history_from_rollout_path(path).await?;
        self.fork_thread_from_history(
            snapshot,
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    async fn initial_history_from_rollout_path(
        &self,
        rollout_path: PathBuf,
    ) -> CodexResult<InitialHistory> {
        let requested_rollout_path = rollout_path.clone();
        let stored_thread = self
            .state
            .thread_store
            .read_thread_by_rollout_path(ReadThreadByRolloutPathParams {
                rollout_path,
                include_archived: true,
                include_history: true,
            })
            .await
            .map_err(thread_store_rollout_read_error)?;
        stored_thread_to_initial_history(stored_thread, Some(requested_rollout_path))
    }

    /// Fork an existing thread from already-loaded store history.
    pub async fn fork_thread_from_history<S>(
        &self,
        snapshot: S,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread>
    where
        S: Into<ForkSnapshot>,
    {
        self.fork_thread_with_initial_history(
            snapshot.into(),
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        )
        .await
    }

    async fn fork_thread_with_initial_history(
        &self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let interrupted_marker = interrupted_turn_history_marker_from_config(&config);
        let history = fork_history_from_snapshot(snapshot, history, interrupted_marker);
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        Box::pin(self.state.spawn_thread(
            config,
            history,
            self.agent_control(),
            thread_source,
            Vec::new(),
            persist_extended_history,
            /*metrics_service_name*/ None,
            parent_trace,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    pub(crate) fn agent_control(&self) -> AgentControl {
        AgentControl::new(Arc::downgrade(&self.state))
    }

    #[cfg(test)]
    pub(crate) fn captured_ops(&self) -> Vec<(ThreadId, Op)> {
        self.state
            .ops_log
            .as_ref()
            .and_then(|ops_log| ops_log.lock().ok().map(|log| log.clone()))
            .unwrap_or_default()
    }
}

impl ThreadServiceState {
    fn terminal_type(&self) -> String {
        self.terminal_type
            .read()
            .map(|terminal_type| terminal_type.clone())
            .unwrap_or_else(|_| DEFAULT_TERMINAL_TYPE.to_string())
    }

    fn set_terminal_type(&self, terminal_type: String) {
        let terminal_type = terminal_type.trim();
        let terminal_type = if terminal_type.is_empty() {
            DEFAULT_TERMINAL_TYPE
        } else {
            terminal_type
        };
        match self.terminal_type.write() {
            Ok(mut stored) => *stored = terminal_type.to_string(),
            Err(err) => warn!("failed to store terminal type: {err}"),
        }
    }

    pub(crate) fn state_db(&self) -> Option<StateDbHandle> {
        self.state_db.clone()
    }

    pub(crate) async fn list_thread_ids(&self) -> Vec<ThreadId> {
        self.threads
            .read()
            .await
            .iter()
            .filter_map(|(thread_id, thread)| {
                (!thread.session_source.is_internal()).then_some(*thread_id)
            })
            .collect()
    }

    /// Fetch a thread by ID or return ThreadNotFound.
    pub(crate) async fn get_thread(&self, thread_id: ThreadId) -> CodexResult<Arc<CodexThread>> {
        let threads = self.threads.read().await;
        match threads.get(&thread_id) {
            Some(thread) if !thread.session_source.is_internal() => Ok(thread.clone()),
            Some(_) | None => Err(CodexErr::ThreadNotFound(thread_id)),
        }
    }

    pub(crate) async fn read_stored_thread(
        &self,
        params: ReadThreadParams,
    ) -> CodexResult<StoredThread> {
        let thread_id = params.thread_id;
        self.thread_store
            .read_thread(params)
            .await
            .map_err(|err| match err {
                ThreadStoreError::ThreadNotFound { thread_id } => {
                    CodexErr::ThreadNotFound(thread_id)
                }
                ThreadStoreError::InvalidRequest { message } => {
                    if message.starts_with("no rollout found for thread id ") {
                        CodexErr::ThreadNotFound(thread_id)
                    } else {
                        CodexErr::Fatal(format!(
                            "failed to read stored thread {thread_id}: invalid thread-store request: {message}"
                        ))
                    }
                }
                err => CodexErr::Fatal(format!("failed to read stored thread {thread_id}: {err}")),
            })
    }

    /// Send an operation to a thread by ID.
    pub(crate) async fn send_op(&self, thread_id: ThreadId, op: Op) -> CodexResult<String> {
        let thread = self.get_thread(thread_id).await?;
        if let Some(ops_log) = &self.ops_log
            && let Ok(mut log) = ops_log.lock()
        {
            log.push((thread_id, op.clone()));
        }
        thread.submit(op).await
    }

    #[cfg(test)]
    /// Append a prebuilt message to a thread by ID outside the normal user-input path.
    pub(crate) async fn append_message(
        &self,
        thread_id: ThreadId,
        message: ResponseItem,
    ) -> CodexResult<String> {
        let thread = self.get_thread(thread_id).await?;
        thread.append_message(message).await
    }

    /// Remove a thread from the manager by ID, returning it when present.
    pub(crate) async fn remove_thread(&self, thread_id: &ThreadId) -> Option<Arc<CodexThread>> {
        self.threads.write().await.remove(thread_id)
    }

    /// Spawn a new thread with no history using a provided config.
    pub(crate) async fn spawn_new_thread(
        &self,
        config: Config,
        agent_control: AgentControl,
    ) -> CodexResult<NewThread> {
        Box::pin(self.spawn_new_thread_with_source(
            config,
            agent_control,
            self.session_source.clone(),
            /*thread_source*/ None,
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            /*inherited_shell_snapshot*/ None,
            /*inherited_exec_policy*/ None,
            /*environments*/ None,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_new_thread_with_source(
        &self,
        config: Config,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<codex_permissions_runtime::ExecPolicyManager>>,
        environments: Option<Vec<TurnEnvironmentSelection>>,
    ) -> CodexResult<NewThread> {
        let environments = environments.unwrap_or_else(|| {
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd)
        });
        Box::pin(self.spawn_thread_with_source(
            config,
            InitialHistory::New,
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            inherited_exec_policy,
            /*parent_trace*/ None,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    pub(crate) async fn resume_thread_with_history_with_source(
        &self,
        options: ResumeThreadWithHistoryOptions,
    ) -> CodexResult<NewThread> {
        let ResumeThreadWithHistoryOptions {
            config,
            initial_history,
            agent_control,
            session_source,
            inherited_shell_snapshot,
            inherited_exec_policy,
        } = options;
        let environments =
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd);
        let thread_source = initial_history.get_resumed_thread_source();
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            /*persist_extended_history*/ false,
            /*metrics_service_name*/ None,
            inherited_shell_snapshot,
            inherited_exec_policy,
            /*parent_trace*/ None,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fork_thread_with_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<codex_permissions_runtime::ExecPolicyManager>>,
        environments: Option<Vec<TurnEnvironmentSelection>>,
    ) -> CodexResult<NewThread> {
        let environments = environments.unwrap_or_else(|| {
            default_thread_environment_selections(self.environment_manager.as_ref(), &config.cwd)
        });
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            agent_control,
            session_source,
            thread_source,
            Vec::new(),
            persist_extended_history,
            /*metrics_service_name*/ None,
            inherited_shell_snapshot,
            inherited_exec_policy,
            /*parent_trace*/ None,
            environments,
            /*user_shell_override*/ None,
        ))
        .await
    }

    /// Spawn a new thread with optional history and register it with the manager.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread(
        &self,
        config: Config,
        initial_history: InitialHistory,
        agent_control: AgentControl,
        thread_source: Option<ThreadSource>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        parent_trace: Option<W3cTraceContext>,
        environments: Vec<TurnEnvironmentSelection>,
        user_shell_override: Option<crate::runtime_shell_model::Shell>,
    ) -> CodexResult<NewThread> {
        Box::pin(self.spawn_thread_with_source(
            config,
            initial_history,
            agent_control,
            self.session_source.clone(),
            thread_source,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            /*inherited_shell_snapshot*/ None,
            /*inherited_exec_policy*/ None,
            parent_trace,
            environments,
            user_shell_override,
        ))
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_thread_with_source(
        &self,
        config: Config,
        initial_history: InitialHistory,
        agent_control: AgentControl,
        session_source: SessionSource,
        thread_source: Option<ThreadSource>,
        dynamic_tools: Vec<codex_protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<codex_permissions_runtime::ExecPolicyManager>>,
        parent_trace: Option<W3cTraceContext>,
        environments: Vec<TurnEnvironmentSelection>,
        user_shell_override: Option<crate::runtime_shell_model::Shell>,
    ) -> CodexResult<NewThread> {
        let is_resumed_thread = matches!(&initial_history, InitialHistory::Resumed(_));
        if let InitialHistory::Resumed(resumed) = &initial_history {
            let mut threads = self.threads.write().await;
            if let Some(thread) = threads.get(&resumed.conversation_id).cloned() {
                if thread.is_running() {
                    if let Some(requested_rollout_path) = resumed.rollout_path.as_deref()
                        && thread.rollout_path().as_deref() != Some(requested_rollout_path)
                    {
                        return Err(CodexErr::InvalidRequest(format!(
                            "thread {} is already running with a different rollout path",
                            resumed.conversation_id
                        )));
                    }
                    return Ok(NewThread {
                        thread_id: resumed.conversation_id,
                        session_configured: thread.session_configured(),
                        thread,
                    });
                }
                threads.remove(&resumed.conversation_id);
            }
        }
        let environment_selections =
            resolve_environment_selections(self.environment_manager.as_ref(), &environments)?;
        let parent_rollout_thread_trace = self
            .parent_rollout_thread_trace_for_source(&session_source, &initial_history)
            .await;
        let tracked_session_source = session_source.clone();
        let environment_manager: Arc<dyn ExecEnvironmentProvider> =
            self.environment_manager.clone();
        let CodexSpawnOk {
            codex, thread_id, ..
        } = Codex::spawn(CodexSpawnArgs {
            config,
            installation_id: self.installation_id.clone(),
            terminal_type: self.terminal_type(),
            auth_runtime: Arc::clone(&self.auth_runtime),
            provider_auth_manager: self.provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&self.model_provider_factory),
            api_runtime_factory: Arc::clone(&self.api_runtime_factory),
            session_telemetry_factory: Arc::clone(&self.session_telemetry_factory),
            memory_tool_developer_instructions_provider: Arc::clone(
                &self.memory_tool_developer_instructions_provider,
            ),
            hook_runtime_factory: Arc::clone(&self.hook_runtime_factory),
            models_manager: Arc::clone(&self.models_manager),
            environment_manager,
            skills_manager: Arc::clone(&self.skills_manager),
            plugins_manager: self.plugin_runtime.clone(),
            mcp_manager: Arc::clone(&self.mcp_manager),
            mcp_auth_runtime: Arc::clone(&self.mcp_auth_runtime),
            mcp_connection_runtime_factory: Arc::clone(&self.mcp_connection_runtime_factory),
            network_proxy_runtime_factory: Arc::clone(&self.network_proxy_runtime_factory),
            sandbox_runtime: Arc::clone(&self.sandbox_runtime),
            extensions: Arc::clone(&self.extensions),
            conversation_history: initial_history,
            session_source,
            thread_source,
            agent_control,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            inherited_exec_policy,
            exec_policy_loader: Arc::clone(&self.exec_policy_loader),
            parent_rollout_thread_trace,
            user_shell_override,
            parent_trace,
            environment_selections,
            analytics_events_client: self.analytics_events_client.clone(),
            thread_store: Arc::clone(&self.thread_store),
            state_db: self.state_db.clone(),
            live_thread_factory: Arc::clone(&self.live_thread_factory),
            attestation_provider: self.attestation_provider.clone(),
            active_event_subscriptions: Arc::clone(&self.active_event_subscriptions),
            openai_file_uploader: Arc::clone(&self.openai_file_uploader),
            code_mode_service: self.code_mode_runtime_factory.create_service(),
            code_mode_runtime_factory: Arc::clone(&self.code_mode_runtime_factory),
            tool_service: Arc::clone(&self.tool_service),
            workflow_runs: Arc::clone(&self.workflow_runs),
        })
        .await?;
        let new_thread = self
            .finalize_thread_spawn(codex, thread_id, tracked_session_source)
            .await?;
        if is_resumed_thread {
            new_thread.thread.emit_thread_resume_lifecycle();
            if let Err(err) = new_thread.thread.apply_goal_resume_runtime_effects().await {
                warn!("failed to apply goal resume runtime effects: {err}");
            }
        }
        Ok(new_thread)
    }

    async fn finalize_thread_spawn(
        &self,
        codex: Codex,
        thread_id: ThreadId,
        session_source: SessionSource,
    ) -> CodexResult<NewThread> {
        let event = codex.next_event().await?;
        let session_configured = match event {
            Event {
                id,
                msg: EventMsg::SessionConfigured(session_configured),
            } if id == INITIAL_SUBMIT_ID => session_configured,
            _ => {
                return Err(CodexErr::SessionConfiguredNotFirstEvent);
            }
        };

        {
            let mut threads = self.threads.write().await;
            if let std::collections::hash_map::Entry::Vacant(e) = threads.entry(thread_id) {
                let thread = Arc::new(CodexThread::new(
                    codex,
                    session_configured.clone(),
                    session_configured.rollout_path.clone(),
                    session_source,
                ));
                e.insert(thread.clone());
                return Ok(NewThread {
                    thread_id,
                    thread,
                    session_configured,
                });
            }
        }

        if let Err(err) = codex.shutdown_and_wait().await {
            warn!("failed to shut down duplicate thread {thread_id}: {err}");
        }
        Err(CodexErr::InvalidRequest(format!(
            "thread {thread_id} is already running"
        )))
    }

    pub(crate) fn notify_thread_started(&self, thread_id: ThreadId) {
        let _ = self
            .thread_created_tx
            .send(ThreadCreatedEvent::Started(thread_id));
    }

    pub(crate) fn notify_thread_resumed(&self, thread_id: ThreadId) {
        let _ = self
            .thread_created_tx
            .send(ThreadCreatedEvent::Resumed(thread_id));
    }

    async fn parent_rollout_thread_trace_for_source(
        &self,
        session_source: &SessionSource,
        initial_history: &InitialHistory,
    ) -> codex_rollout_trace_api::ThreadTraceContext {
        // A fresh v2 child belongs to the same rollout tree as its parent, so
        // session startup derives its child trace from the parent's thread
        // context. Resumed children already have a prior `ThreadStarted` event
        // for this thread id; deriving a child trace during resume would write
        // that start event again and make the bundle unreplayable.
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) = session_source
        else {
            return codex_rollout_trace_api::ThreadTraceContext::disabled();
        };
        if matches!(initial_history, InitialHistory::Resumed(_)) {
            return codex_rollout_trace_api::ThreadTraceContext::disabled();
        }
        // Parent lookup can fail if the parent was closed or released between
        // spawn preparation and session construction. Tracing is diagnostic, so
        // that race should not block child creation; the child simply starts
        // without a parent rollout trace.
        self.get_thread(*parent_thread_id)
            .await
            .ok()
            .map(|thread| thread.codex.session.services.rollout_thread_trace.clone())
            .unwrap_or_else(codex_rollout_trace_api::ThreadTraceContext::disabled)
    }
}

impl codex_thread_api::LiveThreadActivitySource for ThreadServiceState {
    fn live_thread_activity_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = codex_thread_api::LiveThreadActivitySnapshot> + Send + '_
    {
        async move {
            let active_event_subscription_count =
                self.active_event_subscriptions.active_count(thread_id);
            let Ok(thread) = self.get_thread(thread_id).await else {
                return codex_thread_api::LiveThreadActivitySnapshot {
                    manager_available: true,
                    active_event_subscription_count,
                    thread_found: false,
                    has_active_turn: false,
                    status: None,
                };
            };
            codex_thread_api::LiveThreadActivitySnapshot {
                manager_available: true,
                active_event_subscription_count,
                thread_found: true,
                has_active_turn: thread.codex.session.active_turn.lock().await.is_some(),
                status: Some(thread.agent_status().await),
            }
        }
    }
}

impl codex_thread_api::LiveThreadCommandRuntime for ThreadServiceState {
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.send_op(thread_id, op)
    }

    fn remove_live_thread(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move { self.remove_thread(&thread_id).await.is_some() }
    }
}

impl codex_thread_api::LiveThreadShutdownRuntime for ThreadServiceState {
    fn shutdown_live_thread(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            if let Ok(thread) = self.get_thread(thread_id).await {
                thread.codex.session.ensure_rollout_materialized().await;
                thread.codex.session.flush_rollout().await?;
                let status = thread.agent_status().await;
                let result =
                    match live_agent_shutdown_action(/*thread_found*/ true, Some(&status)) {
                        LiveAgentShutdownAction::SubmitWithoutWait
                        | LiveAgentShutdownAction::SubmitAndWait => {
                            self.send_op(thread_id, Op::Shutdown {}).await
                        }
                        LiveAgentShutdownAction::AlreadyShutdownWait => Ok(String::new()),
                    };
                thread.wait_until_terminated().await;
                return result;
            }

            match live_agent_shutdown_action(/*thread_found*/ false, None) {
                LiveAgentShutdownAction::SubmitWithoutWait => {
                    self.send_op(thread_id, Op::Shutdown {}).await
                }
                LiveAgentShutdownAction::SubmitAndWait
                | LiveAgentShutdownAction::AlreadyShutdownWait => Ok(String::new()),
            }
        }
    }
}

impl codex_thread_api::LiveThreadChildCompletionRuntime for ThreadServiceState {
    fn mark_direct_child_completion_pending_if_enabled(
        &self,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move {
            let Ok(parent_thread) = self.get_thread(parent_thread_id).await else {
                return false;
            };
            if !parent_thread.enabled(Feature::MultiAgentV2) {
                return false;
            }
            parent_thread
                .codex
                .session
                .mark_direct_child_completion_pending(child_thread_id)
                .await;
            true
        }
    }

    fn mark_direct_child_completion_received_and_notify(
        &self,
        parent_thread_id: ThreadId,
        child_thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move {
            let Ok(parent_thread) = self.get_thread(parent_thread_id).await else {
                return false;
            };
            if !parent_thread
                .codex
                .session
                .mark_direct_child_completion_received(child_thread_id)
                .await
            {
                return false;
            }
            parent_thread
                .codex
                .session
                .maybe_notify_parent_of_final_status_for_current_source()
                .await;
            true
        }
    }
}

impl codex_thread_api::LiveThreadStatusRuntime for ThreadServiceState {
    fn live_thread_agent_status(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<AgentStatus>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            let status = thread.agent_status().await;
            if is_final(&status) {
                thread
                    .codex
                    .session
                    .maybe_notify_parent_of_final_status_for_current_source()
                    .await;
            }
            Ok(status)
        }
    }

    fn subscribe_live_thread_status(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<tokio::sync::watch::Receiver<AgentStatus>>>
    + Send
    + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.subscribe_status())
        }
    }
}

impl codex_thread_api::LiveThreadInspectionRuntime for ThreadServiceState {
    fn list_live_thread_ids(&self) -> impl std::future::Future<Output = Vec<ThreadId>> + Send + '_ {
        self.list_thread_ids()
    }

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<codex_thread_api::ThreadConfigSnapshot>> + Send + '_
    {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.config_snapshot().await)
        }
    }

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> impl std::future::Future<Output = CodexResult<bool>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.enabled(feature))
        }
    }
}

impl codex_thread_api::LiveThreadStateRuntimeSource for ThreadServiceState {
    fn thread_state_runtime(&self) -> Option<codex_state_api::SharedStateDbRuntime> {
        self.state_db
            .as_ref()
            .map(|state_db| Arc::clone(state_db) as codex_state_api::SharedStateDbRuntime)
    }
}

fn stored_thread_to_initial_history(
    stored_thread: StoredThread,
    rollout_path: Option<PathBuf>,
) -> CodexResult<InitialHistory> {
    let thread_id = stored_thread.thread_id;
    let history = stored_thread.history.ok_or_else(|| {
        CodexErr::Fatal(format!(
            "thread {thread_id} did not include persisted history"
        ))
    })?;
    Ok(InitialHistory::Resumed(ResumedHistory {
        conversation_id: thread_id,
        history: history.items,
        rollout_path: rollout_path.or(stored_thread.rollout_path),
    }))
}

impl codex_thread_api::LiveThreadRegistry for ThreadService {
    type Thread = CodexThread;

    fn list_thread_ids(&self) -> impl std::future::Future<Output = Vec<ThreadId>> + Send + '_ {
        ThreadService::list_thread_ids(self)
    }

    fn session_source(&self) -> SessionSource {
        ThreadService::session_source(self)
    }

    fn is_thread_loaded(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move { self.get_thread(thread_id).await.is_ok() }
    }

    fn live_thread_handle(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Arc<Self::Thread>>> + Send + '_ {
        self.get_thread(thread_id)
    }

    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<codex_thread_api::LiveThreadInfo>> + Send + '_
    {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(codex_thread_api::LiveThreadInfo {
                session_id: thread.session_configured().session_id,
                rollout_path: thread.rollout_path(),
            })
        }
    }

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<codex_thread_api::LiveThreadSnapshot>> + Send + '_
    {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(codex_thread_api::LiveThreadSnapshot {
                info: codex_thread_api::LiveThreadInfo {
                    session_id: thread.session_configured().session_id,
                    rollout_path: thread.rollout_path(),
                },
                config_snapshot: thread.config_snapshot().await,
            })
        }
    }

    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<ThreadId>>> + Send + '_ {
        ThreadService::list_agent_subtree_thread_ids(self, thread_id)
    }

    fn send_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.submit(op).await
        }
    }

    fn send_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.submit_with_trace(op, trace).await
        }
    }

    fn append_thread_conversation_item(
        &self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.append_message(item).await
        }
    }

    fn thread_agent_status(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<AgentStatus>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.agent_status().await)
        }
    }

    fn thread_runtime_status(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<codex_thread_api::ThreadRuntimeStatus>> + Send + '_
    {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.runtime_thread_status().await)
        }
    }

    fn thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> impl std::future::Future<Output = CodexResult<bool>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.enabled(feature))
        }
    }

    fn set_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: codex_thread_api::AppServerClientInfo,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread
                .set_app_server_client_info(
                    info.app_server_client_name,
                    info.app_server_client_version,
                    info.mcp_elicitations_auto_deny,
                )
                .await
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!(
                        "failed to set app server client info for thread {thread_id}: {err}"
                    ))
                })
        }
    }

    fn validate_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: codex_thread_api::CodexThreadTurnContextOverrides,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread
                .validate_turn_context_overrides(overrides)
                .await
                .map_err(|err| {
                    CodexErr::InvalidRequest(format!("invalid turn context override: {err}"))
                })
        }
    }

    fn thread_guardian_trunk_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Option<PathBuf>>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.guardian_trunk_rollout_path().await)
        }
    }

    fn thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThreadHistory>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await.map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    ThreadStoreError::ThreadNotFound { thread_id }
                }
                err => ThreadStoreError::Internal {
                    message: err.to_string(),
                },
            })?;
            thread.load_history(include_archived).await
        }
    }

    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<ThreadSkillWatchPath>>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            let environments = thread.environment_selections().await;
            let Some(environment_selection) = environments.first() else {
                return Ok(Vec::new());
            };
            let Some(environment) = self
                .environment_provider()
                .get_environment(&environment_selection.environment_id)
            else {
                warn!(
                    "failed to register skills watcher for unknown environment `{}`",
                    environment_selection.environment_id
                );
                return Ok(Vec::new());
            };
            if environment.is_remote() {
                return Ok(Vec::new());
            }

            let config = thread.config().await;
            let plugins_input = config.plugins_config_input();
            let plugin_outcome = self
                .plugin_runtime()
                .plugins_for_config(&plugins_input)
                .await;
            let skills_input = SkillsLoadInput::new(
                config.cwd.clone(),
                plugin_outcome.effective_plugin_skill_roots(),
                skill_config_layer_stack_from_config_layer_stack(&config.config_layer_stack),
                config.bundled_skills_enabled(),
            );
            let paths = self
                .skills_manager()
                .skill_root_paths_for_config(&skills_input, Some(environment.get_filesystem()))
                .await
                .into_iter()
                .map(|root| ThreadSkillWatchPath {
                    path: root.into_path_buf(),
                    recursive: true,
                })
                .collect();
            Ok(paths)
        }
    }

    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Option<TokenUsageInfo>>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.token_usage_info().await)
        }
    }

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<ThreadContextUsage>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.thread_context_usage().await)
        }
    }

    fn shutdown_thread_and_wait(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.shutdown_and_wait().await
        }
    }

    fn remove_loaded_thread(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move { self.remove_thread(&thread_id).await.is_some() }
    }

    fn wait_thread_until_terminated(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.wait_until_terminated().await;
            Ok(())
        }
    }

    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.prepare_external_goal_mutation().await;
            Ok(())
        }
    }

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.apply_external_goal_set(external_set).await;
            Ok(())
        }
    }

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.apply_external_goal_clear().await;
            Ok(())
        }
    }

    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.increment_out_of_band_elicitation_count().await
        }
    }

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.decrement_out_of_band_elicitation_count().await
        }
    }
}

fn thread_store_rollout_read_error(err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        err => CodexErr::Fatal(format!("failed to read thread by rollout path: {err}")),
    }
}

fn thread_store_metadata_update_error(thread_id: ThreadId, err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        ThreadStoreError::Unsupported { operation } => CodexErr::UnsupportedOperation(format!(
            "thread metadata update is not supported by this store: {operation}"
        )),
        err => CodexErr::Fatal(format!(
            "failed to update thread metadata {thread_id}: {err}"
        )),
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
