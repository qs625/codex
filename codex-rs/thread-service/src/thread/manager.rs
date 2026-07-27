use crate::StateDbHandle;
use crate::agent::AgentControl;
use crate::agent::external::ExternalSpawnConfig;
use crate::agent::external::SharedExternalAgentRegistry;
use crate::environment_selection::default_thread_environment_selections;
use crate::environment_selection::resolve_environment_selections;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::session::Codex;
use crate::session::CodexSpawnArgs;
use crate::session::CodexSpawnOk;
use crate::session::INITIAL_SUBMIT_ID;
use crate::session::SteerInputError;
use crate::tasks::interrupted_turn_history_marker_from_config;
use crate::thread::CodexThread;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::AgentRegistry;
use codex_agent_runtime::LiveAgentShutdownAction;
use codex_agent_runtime::SpawnAgentProvider;
use codex_agent_runtime::live_agent_shutdown_action;
use codex_analytics_api::AnalyticsEventsClient;
use codex_approval_service_api::ApprovalServiceApi;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
#[cfg(any(test, feature = "test-support"))]
use codex_exec_server::EnvironmentManager;
use codex_extension_api::ExtensionRegistry;
#[cfg(any(test, feature = "test-support"))]
use codex_extension_api::empty_extension_registry;
use codex_features::Feature;
#[cfg(any(test, feature = "test-support"))]
use codex_login::AuthManager;
#[cfg(any(test, feature = "test-support"))]
use codex_login::CodexAuth;
#[cfg(any(test, feature = "test-support"))]
use codex_login::model_provider_auth_manager;
use codex_network_proxy_api::DisabledNetworkProxyRuntimeFactory;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::DisabledOpenAiFileUploader;
use codex_openai_files_api::SharedOpenAiFileUploader;
use codex_sandboxing_api::DisabledSandboxRuntime;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_utils_absolute_path::AbsolutePathBuf;
use command_service_api::CommandServiceApi;
use config_service::Config;
use config_service::ConfigLayerStackOrdering;
use config_service::skill_config_layer_stack_from_config_layer_stack;
use exec_server_api::ExecEnvironmentProvider;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use goal_service_api::GoalServiceApi;
use hooks_api::DisabledHookRuntimeFactory;
use hooks_api::SharedHookRuntimeFactory;
use mcp_service_api::DisabledMcpAuthRuntime;
use mcp_service_api::DisabledMcpConnectionRuntimeFactory;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpServiceApi;
use memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider;
use memory_service_api::SharedMemoryToolDeveloperInstructionsProvider;
use model_service::AttestationProvider;
use model_service::ModelService;
use model_service::ModelServiceRuntimeDeps;
use model_service_api::DisabledApiRuntimeFactory;
use model_service_api::ListModelsRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelProviderInfo;
use model_service_api::ModelServiceApi;
#[cfg(any(test, feature = "test-support"))]
use model_service_api::OPENAI_PROVIDER_ID;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelProviderFactory;
use model_service_api::SharedModelServiceApi;
use permissions_service::EmptyExecPolicyLoader;
use permissions_service::ExecPolicyLoader;
use plugin_service_api::DisabledPluginRuntime;
use plugin_service_api::SharedPluginRuntime;
use protocol::ThreadId;
use protocol::config_types::CollaborationModeMask;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
use protocol::mcp::CallToolResult;
use protocol::models::BaseInstructions;
use protocol::models::ResponseItem;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelsResponse;
use protocol::protocol::AgentStatus;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::InitialHistory;
use protocol::protocol::Op;
use protocol::protocol::ResumedHistory;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadMemoryMode;
use protocol::protocol::ThreadSource;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::TurnEnvironmentSelection;
use protocol::protocol::W3cTraceContext;
use protocol::user_input::UserInput;
use rollout_api::ForkSnapshot;
use rollout_api::fork_history_from_snapshot;
use session_telemetry_api::DisabledSessionTelemetryFactory;
use session_telemetry_api::SharedSessionTelemetryFactory;
use skill_service_api::DisabledSkillService;
use skill_service_api::SharedSkillServiceApi;
use skill_service_api::SkillWatchPath;
use skill_service_api::SkillsLoadInput;
use state_api::ExternalGoalSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use thread_service_api::ActiveEventSubscriptionTracker;
use thread_service_api::ExternalRootThreadInputRoute;
use thread_service_api::ExternalRootThreadRuntime;
use thread_service_api::LiveThreadSnapshot;
use thread_service_api::PersistedExternalRootThreadFacts;
use thread_service_api::PersistedThreadProviderFactsSelector;
use thread_service_api::ThreadCreatedEvent;
use thread_service_api::ThreadShutdownReport;
#[cfg(any(test, feature = "test-support"))]
use thread_store::LocalThreadStore;
#[cfg(any(test, feature = "test-support"))]
use thread_store::LocalThreadStoreConfig;
use thread_store_api::CreateThreadParams;
use thread_store_api::LiveThreadFactory;
use thread_store_api::ReadThreadByRolloutPathParams;
use thread_store_api::ReadThreadParams;
use thread_store_api::SharedLiveThread;
use thread_store_api::StoredThread;
use thread_store_api::StoredThreadHistory;
use thread_store_api::ThreadEventPersistenceMode;
use thread_store_api::ThreadMetadataPatch;
use thread_store_api::ThreadPersistenceMetadata;
use thread_store_api::ThreadStore;
use thread_store_api::ThreadStoreError;
use thread_store_api::ThreadStoreResult;
use thread_store_api::UpdateThreadMetadataParams;
use thread_store_api::external_live_restore_eligibility;
use thread_store_api::persisted_external_root_provider_id;
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

#[derive(Clone, Debug)]
struct ExternalLiveThreadRecord {
    snapshot: LiveThreadSnapshot,
    features: codex_features::Features,
    status: AgentStatus,
    status_tx: tokio::sync::watch::Sender<AgentStatus>,
}

fn external_agent_status_to_thread_runtime_status(
    status: &AgentStatus,
) -> thread_service_api::ThreadRuntimeStatus {
    match status {
        AgentStatus::PendingInit | AgentStatus::Running => {
            thread_service_api::ThreadRuntimeStatus::Active
        }
        AgentStatus::Interrupted
        | AgentStatus::Completed(_)
        | AgentStatus::Errored(_)
        | AgentStatus::Shutdown
        | AgentStatus::NotFound => thread_service_api::ThreadRuntimeStatus::Complete,
    }
}

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

pub struct NewExternalRootThread {
    pub thread_id: ThreadId,
    pub session_configured: SessionConfiguredEvent,
}

enum ShutdownOutcome {
    Complete,
    SubmitFailed,
    TimedOut,
}

fn merge_thread_shutdown_report(
    report: &mut ThreadShutdownReport,
    mut other: ThreadShutdownReport,
) {
    report.completed.append(&mut other.completed);
    report.submit_failed.append(&mut other.submit_failed);
    report.timed_out.append(&mut other.timed_out);
}

fn sort_thread_shutdown_report(report: &mut ThreadShutdownReport) {
    report
        .completed
        .sort_by_key(std::string::ToString::to_string);
    report
        .submit_failed
        .sort_by_key(std::string::ToString::to_string);
    report
        .timed_out
        .sort_by_key(std::string::ToString::to_string);
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
    pub agent_metadata: Option<AgentMetadata>,
    pub thread_source: Option<ThreadSource>,
    pub dynamic_tools: Vec<protocol::dynamic_tools::DynamicToolSpec>,
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

    pub fn from_auth_runtime(
        auth_runtime: SharedAuthRuntime,
        provider_auth_manager: Option<SharedModelProviderAuthManager>,
    ) -> Self {
        Self::new(auth_runtime, provider_auth_manager)
    }
}

pub(crate) struct ResumeThreadWithHistoryOptions {
    pub(crate) config: Config,
    pub(crate) initial_history: InitialHistory,
    pub(crate) agent_control: AgentControl,
    pub(crate) session_source: SessionSource,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<permissions_service::ExecPolicyManager>>,
}

/// Shared, `Arc`-owned state for [`ThreadService`]. This `Arc` is required to have a single
/// `Arc` reference that can be downgraded to by `AgentControl` while preventing every single
/// function to require an `Arc<&Self>`.
pub(crate) struct ThreadServiceState {
    threads: Arc<RwLock<HashMap<ThreadId, Arc<CodexThread>>>>,
    external_live_threads: Arc<RwLock<HashMap<ThreadId, ExternalLiveThreadRecord>>>,
    external_root_agents: SharedExternalAgentRegistry,
    thread_created_tx: broadcast::Sender<ThreadCreatedEvent>,
    auth_runtime: SharedAuthRuntime,
    provider_auth_manager: Option<SharedModelProviderAuthManager>,
    model_service: SharedModelServiceApi,
    environment_manager: Arc<dyn ExecEnvironmentProvider>,
    skill_service: SharedSkillServiceApi,
    plugin_runtime: SharedPluginRuntime,
    mcp_service: Arc<dyn McpServiceApi>,
    mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    api_runtime_factory: SharedApiRuntimeFactory,
    network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    sandbox_runtime: SharedSandboxRuntime,
    command_service_api: Arc<dyn CommandServiceApi>,
    session_telemetry_factory: SharedSessionTelemetryFactory,
    hook_runtime_factory: SharedHookRuntimeFactory,
    memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
    extensions: Arc<ExtensionRegistry<Config>>,
    thread_store: Arc<dyn ThreadStore>,
    live_thread_factory: Arc<dyn LiveThreadFactory>,
    root_agent_registry: Arc<AgentRegistry>,
    attestation_provider: Option<Arc<dyn AttestationProvider>>,
    session_source: SessionSource,
    terminal_type: StdRwLock<String>,
    installation_id: String,
    analytics_events_client: Option<AnalyticsEventsClient>,
    state_db: Option<StateDbHandle>,
    active_event_subscriptions: Arc<ActiveEventSubscriptionTracker>,
    model_provider_factory: SharedModelProviderFactory,
    code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    approval_service: Arc<dyn ApprovalServiceApi>,
    goal_service: Arc<dyn GoalServiceApi>,
    openai_file_uploader: SharedOpenAiFileUploader,
    exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    tool_service: Arc<crate::ToolServiceApi>,
    // Captures submitted ops for testing purpose when test mode is enabled.
    ops_log: Option<SharedCapturedOps>,
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
        command_service_api: Arc<dyn CommandServiceApi>,
        approval_service: Arc<dyn ApprovalServiceApi>,
        goal_service: Arc<dyn GoalServiceApi>,
        tool_service: Arc<crate::ToolServiceApi>,
        mcp_service: Arc<dyn McpServiceApi>,
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
            command_service_api,
            approval_service,
            goal_service,
            tool_service,
            mcp_service,
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
        command_service_api: Arc<dyn CommandServiceApi>,
        approval_service: Arc<dyn ApprovalServiceApi>,
        goal_service: Arc<dyn GoalServiceApi>,
        tool_service: Arc<crate::ToolServiceApi>,
        mcp_service: Arc<dyn McpServiceApi>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    ) -> Self {
        Self::new_with_openai_file_uploader(
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
            command_service_api,
            approval_service,
            goal_service,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            Arc::new(DisabledOpenAiFileUploader),
            Arc::new(EmptyExecPolicyLoader),
            Arc::new(DisabledApiRuntimeFactory),
            Arc::new(DisabledNetworkProxyRuntimeFactory),
            Arc::new(DisabledSandboxRuntime),
            Arc::new(DisabledSessionTelemetryFactory),
            Arc::new(DisabledHookRuntimeFactory),
            Arc::new(DisabledMemoryToolDeveloperInstructionsProvider),
            Arc::new(DisabledSkillService),
            Arc::new(DisabledPluginRuntime),
            tool_service,
            mcp_service,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_openai_file_uploader(
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
        command_service_api: Arc<dyn CommandServiceApi>,
        approval_service: Arc<dyn ApprovalServiceApi>,
        goal_service: Arc<dyn GoalServiceApi>,
        mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
        mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
        openai_file_uploader: SharedOpenAiFileUploader,
        exec_policy_loader: Arc<dyn ExecPolicyLoader>,
        api_runtime_factory: SharedApiRuntimeFactory,
        network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
        sandbox_runtime: SharedSandboxRuntime,
        session_telemetry_factory: SharedSessionTelemetryFactory,
        hook_runtime_factory: SharedHookRuntimeFactory,
        memory_tool_developer_instructions_provider: SharedMemoryToolDeveloperInstructionsProvider,
        skills_runtime: SharedSkillServiceApi,
        plugin_runtime: SharedPluginRuntime,
        tool_service: Arc<crate::ToolServiceApi>,
        mcp_service: Arc<dyn McpServiceApi>,
    ) -> Self {
        let (thread_created_tx, _) = broadcast::channel(THREAD_CREATED_CHANNEL_CAPACITY);
        let ThreadAuthRuntimes {
            auth_runtime,
            provider_auth_manager,
        } = auth_runtimes;
        let model_service: SharedModelServiceApi =
            Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
                codex_home: config.codex_home.to_path_buf(),
                config_model_catalog: config.model_catalog.clone(),
                api_runtime_factory: Arc::clone(&api_runtime_factory),
                provider_auth_manager: provider_auth_manager.clone(),
                model_provider_factory: Arc::clone(&model_provider_factory),
                default_provider: Some(config.model_provider.clone()),
                providers_by_id: config.model_providers.clone(),
                model_metadata_overrides: config
                    .to_models_manager_config()
                    .model_metadata_overrides,
                attestation_provider: attestation_provider.clone(),
            }));
        Self {
            state: Arc::new(ThreadServiceState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                external_live_threads: Arc::new(RwLock::new(HashMap::new())),
                external_root_agents: SharedExternalAgentRegistry::default(),
                thread_created_tx,
                model_service,
                provider_auth_manager,
                environment_manager,
                skill_service: skills_runtime,
                plugin_runtime,
                mcp_service,
                mcp_auth_runtime,
                mcp_connection_runtime_factory,
                api_runtime_factory,
                network_proxy_runtime_factory,
                sandbox_runtime,
                command_service_api,
                session_telemetry_factory,
                hook_runtime_factory,
                memory_tool_developer_instructions_provider,
                extensions,
                thread_store,
                live_thread_factory,
                root_agent_registry: Arc::new(AgentRegistry::default()),
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
                approval_service,
                goal_service,
                openai_file_uploader,
                exec_policy_loader,
                tool_service,
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
        let skill_service: SharedSkillServiceApi =
            Arc::new(skill_service::SkillService::new_with_restriction_product(
                skills_codex_home,
                /*bundled_skills_enabled*/ true,
                restriction_product,
            ));
        // This test constructor has no Config input. Tests that need a non-local
        // process store should construct ThreadService::new with an explicit store.
        let thread_store: Arc<dyn ThreadStore> = Arc::new(LocalThreadStore::new(
            LocalThreadStoreConfig {
                codex_home: codex_home.clone(),
                sqlite_home: codex_home.clone(),
                default_model_provider_id: OPENAI_PROVIDER_ID.to_string(),
            },
            state_db
                .clone()
                .map(|state_db| state_db as state_api::SharedStateDbRuntime),
        ));
        let provider_auth_manager = model_provider_auth_manager(Some(Arc::clone(&auth_manager)));
        let model_service: SharedModelServiceApi =
            Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
                codex_home,
                config_model_catalog: None,
                api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
                provider_auth_manager: provider_auth_manager.clone(),
                model_provider_factory: Arc::clone(&model_provider_factory),
                default_provider: Some(provider),
                providers_by_id: std::collections::HashMap::new(),
                model_metadata_overrides: Vec::new(),
                attestation_provider: None,
            }));
        let auth_runtime: SharedAuthRuntime = auth_manager;
        Self {
            state: Arc::new(ThreadServiceState {
                threads: Arc::new(RwLock::new(HashMap::new())),
                external_live_threads: Arc::new(RwLock::new(HashMap::new())),
                external_root_agents: SharedExternalAgentRegistry::default(),
                thread_created_tx,
                model_service,
                provider_auth_manager,
                environment_manager,
                skill_service,
                plugin_runtime,
                mcp_service: Arc::new(mcp_service::McpService::new(Arc::new(
                    approval_service::ApprovalService,
                ))),
                mcp_auth_runtime: Arc::new(mcp_service::DefaultMcpAuthRuntime),
                mcp_connection_runtime_factory: Arc::new(
                    mcp_service::DefaultMcpConnectionRuntimeFactory,
                ),
                api_runtime_factory: Arc::new(DisabledApiRuntimeFactory),
                network_proxy_runtime_factory: Arc::new(
                    codex_network_proxy::DefaultNetworkProxyRuntimeFactory,
                ),
                sandbox_runtime: Arc::new(DisabledSandboxRuntime),
                command_service_api: Arc::new(command_service::CommandService::new()),
                session_telemetry_factory: Arc::new(DisabledSessionTelemetryFactory),
                hook_runtime_factory: Arc::new(DisabledHookRuntimeFactory),
                memory_tool_developer_instructions_provider: Arc::new(
                    DisabledMemoryToolDeveloperInstructionsProvider,
                ),
                extensions: empty_extension_registry(),
                thread_store,
                live_thread_factory: Arc::new(thread_store::DefaultLiveThreadFactory),
                root_agent_registry: Arc::new(AgentRegistry::default()),
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
                approval_service: Arc::new(approval_service::ApprovalService),
                goal_service: Arc::new(goal_service::GoalService),
                openai_file_uploader: Arc::new(DisabledOpenAiFileUploader),
                exec_policy_loader: Arc::new(EmptyExecPolicyLoader),
                tool_service: Arc::new(crate::test_support::DisabledToolServiceForTests),
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

    pub fn skill_service(&self) -> SharedSkillServiceApi {
        self.state.skill_service.clone()
    }

    pub fn plugin_runtime(&self) -> SharedPluginRuntime {
        self.state.plugin_runtime.clone()
    }

    pub fn mcp_service(&self) -> Arc<dyn McpServiceApi> {
        self.state.mcp_service.clone()
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

    pub fn model_service(&self) -> SharedModelServiceApi {
        Arc::clone(&self.state.model_service)
    }

    pub async fn list_models(&self, refresh: ModelCatalogRefresh) -> Vec<ModelPreset> {
        self.state
            .model_service
            .list_models(ListModelsRequest {
                include_hidden: true,
                refresh,
            })
            .await
            .unwrap_or_default()
    }

    pub async fn list_models_for_provider(
        &self,
        config: &Config,
        provider_info: ModelProviderInfo,
        model_catalog: Option<ModelsResponse>,
        refresh: ModelCatalogRefresh,
    ) -> Vec<ModelPreset> {
        let mut config = config.clone();
        config.model_provider = provider_info;
        config.model_catalog = model_catalog;
        let model_service = ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::clone(&self.state.api_runtime_factory),
            provider_auth_manager: self.state.provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&self.state.model_provider_factory),
            default_provider: Some(config.model_provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: self.state.attestation_provider.clone(),
        });
        model_service
            .list_models(ListModelsRequest {
                include_hidden: true,
                refresh,
            })
            .await
            .unwrap_or_default()
    }

    pub fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask> {
        self.state.model_service.list_collaboration_modes()
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

    pub async fn persisted_external_root_thread_facts(
        &self,
        selector: PersistedThreadProviderFactsSelector,
    ) -> CodexResult<Option<PersistedExternalRootThreadFacts>> {
        self.state
            .persisted_external_root_thread_facts(selector)
            .await
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
        self.agent_control()
            .list_agent_subtree_thread_ids(thread_id)
            .await
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
        dynamic_tools: Vec<protocol::dynamic_tools::DynamicToolSpec>,
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
            agent_metadata: None,
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
        let agent_metadata = options.agent_metadata;
        let root_agent_metadata = agent_metadata.clone();
        let mut agent_path_reservation = agent_metadata
            .as_ref()
            .and_then(|metadata| metadata.agent_path.as_ref())
            .map(|agent_path| {
                self.agent_control()
                    .reserve_root_scope_agent_path(agent_path)
            })
            .transpose()?;
        let new_thread = Box::pin(self.state.spawn_thread_with_source(
            options.config,
            options.initial_history,
            self.agent_control(),
            session_source,
            root_agent_metadata,
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
        .await?;
        if let Some(mut agent_metadata) = agent_metadata {
            agent_metadata.agent_id = Some(new_thread.thread_id);
            if let Some(reservation) = agent_path_reservation.take() {
                reservation.commit(agent_metadata.clone());
            } else {
                self.agent_control()
                    .register_root_scope_agent_metadata(agent_metadata.clone());
            }
            new_thread
                .thread
                .update_thread_metadata(
                    ThreadMetadataPatch {
                        agent_role: Some(agent_metadata.agent_role),
                        agent_path: Some(agent_metadata.agent_path.map(Into::into)),
                        ..Default::default()
                    },
                    /*include_archived*/ false,
                )
                .await
                .map_err(|err| {
                    CodexErr::Fatal(format!("failed to persist root agent metadata: {err}"))
                })?;
        }
        Ok(new_thread)
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
        self.resume_thread_with_history_source_and_agent_metadata(
            config,
            initial_history,
            session_source,
            None,
            parent_trace,
        )
        .await
    }

    pub async fn resume_thread_with_history_source_and_agent_metadata(
        &self,
        config: Config,
        initial_history: InitialHistory,
        session_source: SessionSource,
        agent_metadata: Option<AgentMetadata>,
        parent_trace: Option<W3cTraceContext>,
    ) -> CodexResult<NewThread> {
        let environments = default_thread_environment_selections(
            self.state.environment_manager.as_ref(),
            &config.cwd,
        );
        let mut agent_path_reservation = agent_metadata
            .as_ref()
            .and_then(|metadata| metadata.agent_path.as_ref())
            .map(|agent_path| {
                self.agent_control()
                    .reserve_root_scope_agent_path(agent_path)
            })
            .transpose()?;
        let thread_source = initial_history.get_resumed_thread_source();
        let new_thread = Box::pin(self.state.spawn_thread_with_source(
            config,
            initial_history,
            self.agent_control(),
            session_source,
            agent_metadata.clone(),
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
        .await?;
        if let Some(mut agent_metadata) = agent_metadata {
            agent_metadata.agent_id = Some(new_thread.thread_id);
            if let Some(reservation) = agent_path_reservation.take() {
                reservation.commit(agent_metadata);
            } else {
                self.agent_control()
                    .register_root_scope_agent_metadata(agent_metadata);
            }
        }
        Ok(new_thread)
    }

    #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
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
        let removed = self.state.threads.write().await.remove(thread_id);
        if removed.is_some() {
            self.state
                .root_agent_registry
                .release_uncounted_thread_metadata(*thread_id);
        }
        removed
    }

    pub async fn remove_live_thread(&self, thread_id: ThreadId) -> bool {
        self.state.remove_live_thread(thread_id).await
    }

    pub async fn shutdown_live_thread(&self, thread_id: ThreadId) -> CodexResult<String> {
        if self.has_external_root_thread(thread_id) {
            return ExternalRootThreadRuntime::close_external_root_thread(self, thread_id).await;
        }
        self.state.shutdown_live_thread(thread_id).await
    }

    pub async fn start_external_root_thread(
        &self,
        config: Config,
        provider: SpawnAgentProvider,
    ) -> CodexResult<NewExternalRootThread> {
        let config = ExternalSpawnConfig::from_config(&config);
        self.start_external_root_thread_with_spawn_config(config, provider, None)
            .await
    }

    pub(crate) async fn start_external_root_thread_with_spawn_config(
        &self,
        config: ExternalSpawnConfig,
        provider: SpawnAgentProvider,
        agent_metadata: Option<AgentMetadata>,
    ) -> CodexResult<NewExternalRootThread> {
        self.root_external_agent_control()
            .start_external_root_thread(
                config,
                provider,
                self.state.session_source.clone(),
                agent_metadata,
            )
            .await
    }

    pub async fn submit_external_root_input(
        &self,
        thread_id: ThreadId,
        message: String,
    ) -> CodexResult<String> {
        self.root_external_agent_control()
            .send_external_root_input(thread_id, message)
            .await
    }

    pub fn has_external_root_thread(&self, thread_id: ThreadId) -> bool {
        self.root_external_agent_control()
            .has_external_root_thread(thread_id)
    }

    pub fn live_external_root_thread_facts(
        &self,
        thread_id: ThreadId,
    ) -> Option<thread_service_api::LiveExternalRootThreadFacts> {
        self.root_external_agent_control()
            .live_external_root_thread_facts(thread_id)
    }

    pub async fn external_root_thread_input_route(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<ExternalRootThreadInputRoute> {
        if let Some(facts) = self.live_external_root_thread_facts(thread_id) {
            return Ok(ExternalRootThreadInputRoute::LiveExternalRoot {
                thread_id: facts.thread_id,
                provider: facts.provider,
            });
        }
        if thread_service_api::LiveThreadInspectionRuntime::is_live_thread_loaded(self, thread_id)
            .await
        {
            return Ok(ExternalRootThreadInputRoute::NativeRequired);
        }
        if let Some(facts) = self
            .persisted_external_root_thread_facts(PersistedThreadProviderFactsSelector::ThreadId(
                thread_id,
            ))
            .await?
        {
            return Ok(
                ExternalRootThreadInputRoute::UnsupportedPersistedExternalRoot {
                    thread_id: facts.thread_id,
                    provider_id: facts.provider_id,
                },
            );
        }
        Ok(ExternalRootThreadInputRoute::NativeRequired)
    }

    pub async fn close_external_root_thread(&self, thread_id: ThreadId) -> CodexResult<String> {
        self.root_external_agent_control()
            .close_external_root_thread(thread_id)
            .await
    }

    pub async fn live_thread_agent_status(&self, thread_id: ThreadId) -> CodexResult<AgentStatus> {
        self.state.live_thread_agent_status(thread_id).await
    }

    pub async fn live_thread_runtime_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<thread_service_api::ThreadRuntimeStatus> {
        self.state.live_thread_runtime_status(thread_id).await
    }

    pub async fn subscribe_live_thread_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<tokio::sync::watch::Receiver<AgentStatus>> {
        self.state.subscribe_live_thread_status(thread_id).await
    }

    /// Tries to shut down all tracked threads concurrently within the provided timeout.
    /// Threads that complete shutdown are removed from the manager; incomplete shutdowns
    /// remain tracked so callers can retry or inspect them later.
    pub async fn shutdown_all_threads_bounded(&self, timeout: Duration) -> ThreadShutdownReport {
        let mut report = self.state.shutdown_native_threads_bounded(timeout).await;
        let external_report = self
            .root_external_agent_control()
            .shutdown_all_live_external_threads_bounded(timeout)
            .await;
        merge_thread_shutdown_report(&mut report, external_report);
        sort_thread_shutdown_report(&mut report);
        report
    }

    pub async fn shutdown_all_threads_for_runtime_teardown_bounded(
        &self,
        timeout: Duration,
    ) -> ThreadShutdownReport {
        let mut report = self.state.shutdown_native_threads_bounded(timeout).await;
        let external_report = self
            .root_external_agent_control()
            .shutdown_all_live_external_threads_for_runtime_teardown_bounded(timeout)
            .await;
        merge_thread_shutdown_report(&mut report, external_report);
        sort_thread_shutdown_report(&mut report);
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
        AgentControl::new_with_registry(
            Arc::downgrade(&self.state),
            Arc::clone(&self.state.root_agent_registry),
        )
    }

    fn root_external_agent_control(&self) -> AgentControl {
        AgentControl::new_with_external_registry(
            Arc::downgrade(&self.state),
            Arc::clone(&self.state.root_agent_registry),
            self.state.external_root_agents.clone(),
        )
    }

    #[cfg(test)]
    pub(crate) fn root_external_agent_control_for_tests(&self) -> AgentControl {
        self.root_external_agent_control()
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

    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
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

    /// List `thread_id` plus all known descendants in its spawn subtree.
    pub(crate) async fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        let thread = self.get_thread(thread_id).await?;

        let mut subtree_thread_ids = Vec::new();
        let mut seen_thread_ids = HashSet::new();
        subtree_thread_ids.push(thread_id);
        seen_thread_ids.insert(thread_id);

        if let Some(state_db_ctx) = thread.state_db() {
            for descendant_id in state_db_ctx
                .list_thread_spawn_descendants(thread_id)
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

    pub(crate) async fn persisted_external_root_thread_facts(
        &self,
        selector: PersistedThreadProviderFactsSelector,
    ) -> CodexResult<Option<PersistedExternalRootThreadFacts>> {
        let stored_thread = match selector {
            PersistedThreadProviderFactsSelector::ThreadId(thread_id) => match self
                .thread_store
                .read_thread(ReadThreadParams {
                    thread_id,
                    include_archived: true,
                    include_history: true,
                })
                .await
            {
                Ok(stored_thread) => stored_thread,
                Err(ThreadStoreError::ThreadNotFound { .. }) => return Ok(None),
                Err(ThreadStoreError::InvalidRequest { message })
                    if message == format!("no rollout found for thread id {thread_id}") =>
                {
                    return Ok(None);
                }
                Err(err) => return Err(persisted_provider_facts_read_error(thread_id, err)),
            },
            PersistedThreadProviderFactsSelector::RolloutPath(rollout_path) => match self
                .thread_store
                .read_thread_by_rollout_path(ReadThreadByRolloutPathParams {
                    rollout_path,
                    include_archived: true,
                    include_history: true,
                })
                .await
            {
                Ok(stored_thread) => stored_thread,
                Err(ThreadStoreError::ThreadNotFound { .. }) => return Ok(None),
                Err(err) => return Err(persisted_provider_facts_rollout_read_error(err)),
            },
        };

        let Some(provider_id) = persisted_external_root_provider_id(&stored_thread) else {
            return Ok(None);
        };
        Ok(Some(PersistedExternalRootThreadFacts {
            thread_id: stored_thread.thread_id,
            provider_id: provider_id.to_string(),
            restore_eligibility: external_live_restore_eligibility(&stored_thread),
        }))
    }

    pub(crate) async fn create_external_thread_persistence(
        &self,
        cwd: &AbsolutePathBuf,
        model_provider_id: String,
        generate_memories: bool,
        thread_id: ThreadId,
        session_source: SessionSource,
        thread_source: ThreadSource,
        agent_metadata: AgentMetadata,
    ) -> CodexResult<SharedLiveThread> {
        self.live_thread_factory
            .create(
                Arc::clone(&self.thread_store),
                CreateThreadParams {
                    thread_id,
                    forked_from_id: None,
                    source: session_source,
                    thread_source: Some(thread_source),
                    base_instructions: BaseInstructions {
                        text: String::new(),
                    },
                    dynamic_tools: Vec::new(),
                    metadata: ThreadPersistenceMetadata {
                        cwd: Some(cwd.to_path_buf()),
                        model_provider: model_provider_id,
                        memory_mode: if generate_memories {
                            ThreadMemoryMode::Enabled
                        } else {
                            ThreadMemoryMode::Disabled
                        },
                        root_agent_role: agent_metadata.agent_role,
                        root_agent_path: agent_metadata
                            .agent_path
                            .as_ref()
                            .map(ToString::to_string),
                    },
                    event_persistence_mode: ThreadEventPersistenceMode::Limited,
                },
            )
            .await
            .map_err(|err| {
                CodexErr::Fatal(format!(
                    "failed to create persisted external thread {thread_id}: {err}"
                ))
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn register_external_live_thread_snapshot(
        &self,
        thread_id: ThreadId,
        snapshot: LiveThreadSnapshot,
        status: AgentStatus,
    ) {
        self.register_external_live_thread_snapshot_with_features(
            thread_id,
            snapshot,
            codex_features::Features::with_defaults(),
            status,
        )
        .await;
    }

    pub(crate) async fn register_external_live_thread_snapshot_with_features(
        &self,
        thread_id: ThreadId,
        snapshot: LiveThreadSnapshot,
        features: codex_features::Features,
        status: AgentStatus,
    ) {
        let (status_tx, _status_rx) = tokio::sync::watch::channel(status.clone());
        self.external_live_threads.write().await.insert(
            thread_id,
            ExternalLiveThreadRecord {
                snapshot,
                features,
                status,
                status_tx,
            },
        );
    }

    pub(crate) async fn update_external_live_thread_status(
        &self,
        thread_id: ThreadId,
        status: AgentStatus,
    ) {
        if let Some(record) = self.external_live_threads.write().await.get_mut(&thread_id) {
            record.status = status.clone();
            record.status_tx.send_replace(status);
        }
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

    pub(crate) async fn remove_live_thread(&self, thread_id: ThreadId) -> bool {
        let native_removed = self.threads.write().await.remove(&thread_id).is_some();
        if native_removed {
            self.root_agent_registry
                .release_uncounted_thread_metadata(thread_id);
        }
        let external_removed = self
            .external_live_threads
            .write()
            .await
            .remove(&thread_id)
            .is_some();
        native_removed || external_removed
    }

    pub(crate) async fn shutdown_native_threads_bounded(
        &self,
        timeout: Duration,
    ) -> ThreadShutdownReport {
        let threads = {
            let threads = self.threads.read().await;
            threads
                .iter()
                .map(|(thread_id, thread)| (*thread_id, Arc::clone(thread)))
                .collect::<Vec<_>>()
        };

        let mut shutdowns = threads
            .into_iter()
            .map(|(thread_id, thread)| async move {
                let agent_control = thread.codex.session.services.agent_control.clone();
                let native_shutdown = tokio::time::timeout(timeout, thread.shutdown_and_wait());
                let external_shutdown =
                    agent_control.shutdown_all_live_external_threads_bounded(timeout);
                let (native_shutdown, external_report) =
                    tokio::join!(native_shutdown, external_shutdown);
                let outcome = match native_shutdown {
                    Ok(Ok(())) => ShutdownOutcome::Complete,
                    Ok(Err(_)) => ShutdownOutcome::SubmitFailed,
                    Err(_) => ShutdownOutcome::TimedOut,
                };
                (thread_id, outcome, external_report)
            })
            .collect::<FuturesUnordered<_>>();
        let mut report = ThreadShutdownReport::default();

        while let Some((thread_id, outcome, external_report)) = shutdowns.next().await {
            merge_thread_shutdown_report(&mut report, external_report);
            match outcome {
                ShutdownOutcome::Complete => report.completed.push(thread_id),
                ShutdownOutcome::SubmitFailed => report.submit_failed.push(thread_id),
                ShutdownOutcome::TimedOut => report.timed_out.push(thread_id),
            }
        }

        let mut tracked_threads = self.threads.write().await;
        for thread_id in &report.completed {
            if tracked_threads.remove(thread_id).is_some() {
                self.root_agent_registry
                    .release_uncounted_thread_metadata(*thread_id);
            }
        }

        sort_thread_shutdown_report(&mut report);
        report
    }

    pub(crate) async fn shutdown_live_thread(&self, thread_id: ThreadId) -> CodexResult<String> {
        if let Ok(thread) = self.get_thread(thread_id).await {
            thread.codex.session.ensure_rollout_materialized().await;
            thread.codex.session.flush_rollout().await?;
            let status = thread.agent_status().await;
            let result = match live_agent_shutdown_action(/*thread_found*/ true, Some(&status)) {
                LiveAgentShutdownAction::SubmitWithoutWait
                | LiveAgentShutdownAction::SubmitAndWait => {
                    self.send_op(thread_id, Op::Shutdown {}).await
                }
                LiveAgentShutdownAction::AlreadyShutdownWait => Ok(String::new()),
            };
            thread.wait_until_terminated().await;
            return match result {
                Err(CodexErr::InternalAgentDied) => Ok(String::new()),
                result => result,
            };
        }

        match live_agent_shutdown_action(/*thread_found*/ false, None) {
            LiveAgentShutdownAction::SubmitWithoutWait => {
                self.send_op(thread_id, Op::Shutdown {}).await
            }
            LiveAgentShutdownAction::SubmitAndWait
            | LiveAgentShutdownAction::AlreadyShutdownWait => Ok(String::new()),
        }
    }

    pub(crate) async fn live_thread_agent_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<AgentStatus> {
        if let Some(record) = self.external_live_threads.read().await.get(&thread_id) {
            return Ok(record.status.clone());
        }
        let thread = self.get_thread(thread_id).await?;
        Ok(thread.agent_status().await)
    }

    pub(crate) async fn live_thread_runtime_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<thread_service_api::ThreadRuntimeStatus> {
        match self.get_thread(thread_id).await {
            Ok(thread) => Ok(thread.runtime_thread_status().await),
            Err(CodexErr::ThreadNotFound(_)) => self
                .external_live_threads
                .read()
                .await
                .get(&thread_id)
                .map(|record| external_agent_status_to_thread_runtime_status(&record.status))
                .ok_or(CodexErr::ThreadNotFound(thread_id)),
            Err(err) => Err(err),
        }
    }

    pub(crate) async fn subscribe_live_thread_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<tokio::sync::watch::Receiver<AgentStatus>> {
        match self.get_thread(thread_id).await {
            Ok(thread) => Ok(thread.subscribe_status()),
            Err(CodexErr::ThreadNotFound(_)) => self
                .external_live_threads
                .read()
                .await
                .get(&thread_id)
                .map(|record| record.status_tx.subscribe())
                .ok_or(CodexErr::ThreadNotFound(thread_id)),
            Err(err) => Err(err),
        }
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
        inherited_exec_policy: Option<Arc<permissions_service::ExecPolicyManager>>,
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
            /*root_agent_metadata*/ None,
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
            /*root_agent_metadata*/ None,
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
        inherited_exec_policy: Option<Arc<permissions_service::ExecPolicyManager>>,
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
            /*root_agent_metadata*/ None,
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
        dynamic_tools: Vec<protocol::dynamic_tools::DynamicToolSpec>,
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
            /*root_agent_metadata*/ None,
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
        root_agent_metadata: Option<AgentMetadata>,
        thread_source: Option<ThreadSource>,
        dynamic_tools: Vec<protocol::dynamic_tools::DynamicToolSpec>,
        persist_extended_history: bool,
        metrics_service_name: Option<String>,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<permissions_service::ExecPolicyManager>>,
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
            environment_manager,
            skill_service: Arc::clone(&self.skill_service),
            plugins_manager: self.plugin_runtime.clone(),
            mcp_service: Arc::clone(&self.mcp_service),
            mcp_auth_runtime: Arc::clone(&self.mcp_auth_runtime),
            mcp_connection_runtime_factory: Arc::clone(&self.mcp_connection_runtime_factory),
            network_proxy_runtime_factory: Arc::clone(&self.network_proxy_runtime_factory),
            sandbox_runtime: Arc::clone(&self.sandbox_runtime),
            command_service_api: Arc::clone(&self.command_service_api),
            extensions: Arc::clone(&self.extensions),
            conversation_history: initial_history,
            session_source,
            thread_source,
            root_agent_metadata,
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
            approval_service: Arc::clone(&self.approval_service),
            goal_service: Arc::clone(&self.goal_service),
            tool_service: Arc::clone(&self.tool_service),
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

    pub(crate) fn notify_thread_live_event(
        &self,
        thread_id: ThreadId,
        turn_id: String,
        event: protocol::protocol::EventMsg,
    ) {
        let _ = self.thread_created_tx.send(ThreadCreatedEvent::LiveEvent {
            thread_id,
            turn_id,
            event,
        });
    }

    #[allow(dead_code)]
    pub(crate) fn notify_thread_status_changed(&self, thread_id: ThreadId) {
        self.notify_thread_status_changed_with_status(thread_id, None);
    }

    pub(crate) fn notify_thread_status_changed_with_status(
        &self,
        thread_id: ThreadId,
        agent_status: Option<AgentStatus>,
    ) {
        let _ = self
            .thread_created_tx
            .send(ThreadCreatedEvent::StatusChanged {
                thread_id,
                agent_status,
            });
    }

    async fn parent_rollout_thread_trace_for_source(
        &self,
        session_source: &SessionSource,
        initial_history: &InitialHistory,
    ) -> rollout_trace_api::ThreadTraceContext {
        // A fresh v2 child belongs to the same rollout tree as its parent, so
        // session startup derives its child trace from the parent's thread
        // context. Resumed children already have a prior `ThreadStarted` event
        // for this thread id; deriving a child trace during resume would write
        // that start event again and make the bundle unreplayable.
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) = session_source
        else {
            return rollout_trace_api::ThreadTraceContext::disabled();
        };
        if matches!(initial_history, InitialHistory::Resumed(_)) {
            return rollout_trace_api::ThreadTraceContext::disabled();
        }
        // Parent lookup can fail if the parent was closed or released between
        // spawn preparation and session construction. Tracing is diagnostic, so
        // that race should not block child creation; the child simply starts
        // without a parent rollout trace.
        self.get_thread(*parent_thread_id)
            .await
            .ok()
            .map(|thread| thread.codex.session.services.rollout_thread_trace.clone())
            .unwrap_or_else(rollout_trace_api::ThreadTraceContext::disabled)
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadActivitySource for ThreadServiceState {
    fn live_thread_activity_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = thread_service_api::LiveThreadActivitySnapshot> + Send + '_
    {
        async move {
            let active_event_subscription_count =
                self.active_event_subscriptions.active_count(thread_id);
            let Ok(thread) = self.get_thread(thread_id).await else {
                return thread_service_api::LiveThreadActivitySnapshot {
                    manager_available: true,
                    active_event_subscription_count,
                    thread_found: false,
                    has_active_turn: false,
                    status: None,
                };
            };
            let has_active_turn = {
                let active_turn = thread.codex.session.active_turn.lock().await;
                active_turn.is_some()
            };
            thread_service_api::LiveThreadActivitySnapshot {
                manager_available: true,
                active_event_subscription_count,
                thread_found: true,
                has_active_turn,
                status: Some(thread.agent_status().await),
            }
        }
    }
}

impl thread_service_api::ThreadLifecycleRuntime for ThreadServiceState {
    fn shutdown_all_threads_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> thread_service_api::ThreadServiceFuture<'a, ThreadShutdownReport> {
        Box::pin(ThreadServiceState::shutdown_native_threads_bounded(
            self, timeout,
        ))
    }

    fn shutdown_all_threads_for_runtime_teardown_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> thread_service_api::ThreadServiceFuture<'a, ThreadShutdownReport> {
        Box::pin(ThreadServiceState::shutdown_native_threads_bounded(
            self, timeout,
        ))
    }

    fn shutdown_live_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> thread_service_api::ThreadServiceFuture<'a, CodexResult<String>> {
        Box::pin(ThreadServiceState::shutdown_live_thread(self, thread_id))
    }

    fn remove_live_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> thread_service_api::ThreadServiceFuture<'a, bool> {
        Box::pin(ThreadServiceState::remove_live_thread(self, thread_id))
    }

    fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadCreatedEvent> {
        self.thread_created_tx.subscribe()
    }

    fn live_thread_agent_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> thread_service_api::ThreadServiceFuture<'a, CodexResult<AgentStatus>> {
        Box::pin(ThreadServiceState::live_thread_agent_status(
            self, thread_id,
        ))
    }

    fn live_thread_runtime_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> thread_service_api::ThreadServiceFuture<
        'a,
        CodexResult<thread_service_api::ThreadRuntimeStatus>,
    > {
        Box::pin(ThreadServiceState::live_thread_runtime_status(
            self, thread_id,
        ))
    }

    fn subscribe_live_thread_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> thread_service_api::ThreadServiceFuture<
        'a,
        CodexResult<tokio::sync::watch::Receiver<AgentStatus>>,
    > {
        Box::pin(ThreadServiceState::subscribe_live_thread_status(
            self, thread_id,
        ))
    }

    fn active_event_subscriptions(&self) -> Arc<ActiveEventSubscriptionTracker> {
        Arc::clone(&self.active_event_subscriptions)
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadCommandRuntime for ThreadServiceState {
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.send_op(thread_id, op)
    }

    fn submit_live_thread_op_with_trace(
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

    fn set_live_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: thread_service_api::AppServerClientInfo,
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
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadConversationRuntime for ThreadServiceState {
    fn append_live_thread_conversation_item(
        &self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.append_message(item).await
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadConversationInjectionRuntime for ThreadServiceState {
    fn inject_live_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread.inject_conversation_items(items).await
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadHistoryRuntime for ThreadServiceState {
    fn live_thread_history(
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

    fn read_live_thread(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
        include_history: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThread>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await.map_err(|err| match err {
                CodexErr::ThreadNotFound(thread_id) => {
                    ThreadStoreError::ThreadNotFound { thread_id }
                }
                err => ThreadStoreError::Internal {
                    message: err.to_string(),
                },
            })?;
            thread.read_thread(include_archived, include_history).await
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadListenerRuntime for ThreadServiceState {
    type ListenerHandle = CodexThread;

    fn live_thread_listener_handle(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Arc<Self::ListenerHandle>>> + Send + '_ {
        self.get_thread(thread_id)
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadTurnRuntime for ThreadServiceState {
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: thread_service_api::CodexThreadTurnContextOverrides,
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
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadInspectionRuntime for ThreadServiceState {
    fn list_live_thread_ids(&self) -> impl std::future::Future<Output = Vec<ThreadId>> + Send + '_ {
        async move {
            let mut thread_ids = self.list_thread_ids().await;
            let mut seen: HashSet<ThreadId> = thread_ids.iter().copied().collect();
            for thread_id in self.external_live_threads.read().await.keys().copied() {
                if seen.insert(thread_id) {
                    thread_ids.push(thread_id);
                }
            }
            thread_ids
        }
    }

    fn is_live_thread_loaded(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        async move {
            self.get_thread(thread_id).await.is_ok()
                || self
                    .external_live_threads
                    .read()
                    .await
                    .contains_key(&thread_id)
        }
    }

    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::LiveThreadInfo>> + Send + '_
    {
        async move {
            match self.get_thread(thread_id).await {
                Ok(thread) => Ok(thread_service_api::LiveThreadInfo {
                    session_id: thread.session_configured().session_id,
                    rollout_path: thread.rollout_path(),
                }),
                Err(CodexErr::ThreadNotFound(_)) => self
                    .external_live_threads
                    .read()
                    .await
                    .get(&thread_id)
                    .map(|record| record.snapshot.info.clone())
                    .ok_or(CodexErr::ThreadNotFound(thread_id)),
                Err(err) => Err(err),
            }
        }
    }

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::LiveThreadSnapshot>> + Send + '_
    {
        async move {
            if let Some(record) = self.external_live_threads.read().await.get(&thread_id) {
                return Ok(record.snapshot.clone());
            }
            let thread = self.get_thread(thread_id).await?;
            Ok(thread_service_api::LiveThreadSnapshot {
                info: thread_service_api::LiveThreadInfo {
                    session_id: thread.session_configured().session_id,
                    rollout_path: thread.rollout_path(),
                },
                config_snapshot: thread.config_snapshot().await,
            })
        }
    }

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::ThreadConfigSnapshot>>
    + Send
    + '_ {
        async move {
            if let Some(record) = self.external_live_threads.read().await.get(&thread_id) {
                return Ok(record.snapshot.config_snapshot.clone());
            }
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.config_snapshot().await)
        }
    }

    fn live_thread_config_refresh_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<
        Output = CodexResult<thread_service_api::LiveThreadConfigRefreshSnapshot>,
    > + Send
    + '_ {
        async move {
            if let Some(record) = self.external_live_threads.read().await.get(&thread_id) {
                return Ok(thread_service_api::LiveThreadConfigRefreshSnapshot {
                    cwd: record.snapshot.config_snapshot.cwd.clone(),
                    session_layers: Vec::new(),
                });
            }
            let thread = self.get_thread(thread_id).await?;
            let config = thread.config().await;
            let session_layers = config
                .config_layer_stack
                .get_layers(
                    ConfigLayerStackOrdering::LowestPrecedenceFirst,
                    /*include_disabled*/ true,
                )
                .into_iter()
                .filter(|layer| {
                    matches!(
                        layer.name,
                        codex_config_types::ConfigLayerSource::SessionFlags
                    )
                })
                .cloned()
                .collect();
            Ok(thread_service_api::LiveThreadConfigRefreshSnapshot {
                cwd: config.cwd.clone(),
                session_layers,
            })
        }
    }

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> impl std::future::Future<Output = CodexResult<bool>> + Send + '_ {
        async move {
            if let Some(record) = self.external_live_threads.read().await.get(&thread_id) {
                return Ok(record.features.enabled(feature));
            }
            let thread = self.get_thread(thread_id).await?;
            Ok(thread.enabled(feature))
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadFeedbackRuntime for ThreadServiceState {
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<ThreadId>>> + Send + '_ {
        ThreadServiceState::list_agent_subtree_thread_ids(self, thread_id)
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

    fn session_source(&self) -> SessionSource {
        self.session_source.clone()
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadSkillWatchRuntime for ThreadServiceState {
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<SkillWatchPath>>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            let environments = thread.environment_selections().await;
            let Some(environment_selection) = environments.first() else {
                return Ok(Vec::new());
            };
            let Some(environment) = self
                .environment_manager
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
            let skills_input = SkillsLoadInput::new(
                config.cwd.clone(),
                self.plugin_runtime
                    .effective_skill_roots_for_config(&plugins_input)
                    .await,
                skill_config_layer_stack_from_config_layer_stack(&config.config_layer_stack),
                config.bundled_skills_enabled(),
            );
            let paths = self
                .skill_service
                .skill_root_paths_for_config(&skills_input, Some(environment.get_filesystem()))
                .await
                .into_iter()
                .map(|root| SkillWatchPath {
                    path: root.into_path_buf(),
                    recursive: true,
                })
                .collect();
            Ok(paths)
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadUsageRuntime for ThreadServiceState {
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
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadGoalRuntime for ThreadServiceState {
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

    fn apply_thread_goal_resume_runtime_effects(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread_service_api::LiveThreadHandle::apply_goal_resume_runtime_effects(thread.as_ref())
                .await
        }
    }

    fn continue_thread_active_goal_if_idle(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        async move {
            let thread = self.get_thread(thread_id).await?;
            thread_service_api::LiveThreadHandle::continue_active_goal_if_idle(thread.as_ref())
                .await
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadElicitationRuntime for ThreadServiceState {
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

impl thread_service_api::LiveThreadStateRuntimeSource for ThreadServiceState {
    fn thread_state_runtime(&self) -> Option<state_api::SharedStateDbRuntime> {
        self.state_db
            .as_ref()
            .map(|state_db| Arc::clone(state_db) as state_api::SharedStateDbRuntime)
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadCommandRuntime for ThreadService {
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        thread_service_api::LiveThreadCommandRuntime::submit_live_thread_op(
            self.state.as_ref(),
            thread_id,
            op,
        )
    }

    fn submit_live_thread_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        thread_service_api::LiveThreadCommandRuntime::submit_live_thread_op_with_trace(
            self.state.as_ref(),
            thread_id,
            op,
            trace,
        )
    }

    fn set_live_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: thread_service_api::AppServerClientInfo,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadCommandRuntime::set_live_thread_app_server_client_info(
            self.state.as_ref(),
            thread_id,
            info,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadConversationRuntime for ThreadService {
    fn append_live_thread_conversation_item(
        &self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        thread_service_api::LiveThreadConversationRuntime::append_live_thread_conversation_item(
            self.state.as_ref(),
            thread_id,
            item,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadConversationInjectionRuntime for ThreadService {
    fn inject_live_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadConversationInjectionRuntime::inject_live_thread_conversation_items(
            self.state.as_ref(),
            thread_id,
            items,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadHistoryRuntime for ThreadService {
    fn live_thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThreadHistory>> + Send + '_ {
        thread_service_api::LiveThreadHistoryRuntime::live_thread_history(
            self.state.as_ref(),
            thread_id,
            include_archived,
        )
    }

    fn read_live_thread(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
        include_history: bool,
    ) -> impl std::future::Future<Output = ThreadStoreResult<StoredThread>> + Send + '_ {
        thread_service_api::LiveThreadHistoryRuntime::read_live_thread(
            self.state.as_ref(),
            thread_id,
            include_archived,
            include_history,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadListenerRuntime for ThreadService {
    type ListenerHandle = CodexThread;

    fn live_thread_listener_handle(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Arc<Self::ListenerHandle>>> + Send + '_ {
        thread_service_api::LiveThreadListenerRuntime::live_thread_listener_handle(
            self.state.as_ref(),
            thread_id,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadTurnRuntime for ThreadService {
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: thread_service_api::CodexThreadTurnContextOverrides,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadTurnRuntime::validate_live_thread_turn_context_overrides(
            self.state.as_ref(),
            thread_id,
            overrides,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadInspectionRuntime for ThreadService {
    fn list_live_thread_ids(&self) -> impl std::future::Future<Output = Vec<ThreadId>> + Send + '_ {
        thread_service_api::LiveThreadInspectionRuntime::list_live_thread_ids(self.state.as_ref())
    }

    fn is_live_thread_loaded(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = bool> + Send + '_ {
        thread_service_api::LiveThreadInspectionRuntime::is_live_thread_loaded(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::LiveThreadInfo>> + Send + '_
    {
        thread_service_api::LiveThreadInspectionRuntime::live_thread_info(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::LiveThreadSnapshot>> + Send + '_
    {
        thread_service_api::LiveThreadInspectionRuntime::live_thread_snapshot(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<thread_service_api::ThreadConfigSnapshot>>
    + Send
    + '_ {
        thread_service_api::LiveThreadInspectionRuntime::live_thread_config_snapshot(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn live_thread_config_refresh_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<
        Output = CodexResult<thread_service_api::LiveThreadConfigRefreshSnapshot>,
    > + Send
    + '_ {
        thread_service_api::LiveThreadInspectionRuntime::live_thread_config_refresh_snapshot(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> impl std::future::Future<Output = CodexResult<bool>> + Send + '_ {
        thread_service_api::LiveThreadInspectionRuntime::live_thread_feature_enabled(
            self.state.as_ref(),
            thread_id,
            feature,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadFeedbackRuntime for ThreadService {
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<ThreadId>>> + Send + '_ {
        async move {
            self.agent_control()
                .list_agent_subtree_thread_ids(thread_id)
                .await
        }
    }

    fn thread_guardian_trunk_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Option<PathBuf>>> + Send + '_ {
        thread_service_api::LiveThreadFeedbackRuntime::thread_guardian_trunk_rollout_path(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn session_source(&self) -> SessionSource {
        thread_service_api::LiveThreadFeedbackRuntime::session_source(self.state.as_ref())
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadSkillWatchRuntime for ThreadService {
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Vec<SkillWatchPath>>> + Send + '_ {
        thread_service_api::LiveThreadSkillWatchRuntime::thread_skill_watch_paths(
            self.state.as_ref(),
            thread_id,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadUsageRuntime for ThreadService {
    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<Option<TokenUsageInfo>>> + Send + '_ {
        thread_service_api::LiveThreadUsageRuntime::thread_token_usage_info(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<ThreadContextUsage>> + Send + '_ {
        thread_service_api::LiveThreadUsageRuntime::thread_context_usage(
            self.state.as_ref(),
            thread_id,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadGoalRuntime for ThreadService {
    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadGoalRuntime::prepare_thread_external_goal_mutation(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadGoalRuntime::apply_thread_external_goal_set(
            self.state.as_ref(),
            thread_id,
            external_set,
        )
    }

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadGoalRuntime::apply_thread_external_goal_clear(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn apply_thread_goal_resume_runtime_effects(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadGoalRuntime::apply_thread_goal_resume_runtime_effects(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn continue_thread_active_goal_if_idle(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        thread_service_api::LiveThreadGoalRuntime::continue_thread_active_goal_if_idle(
            self.state.as_ref(),
            thread_id,
        )
    }
}

#[allow(clippy::manual_async_fn)]
impl thread_service_api::LiveThreadElicitationRuntime for ThreadService {
    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        thread_service_api::LiveThreadElicitationRuntime::increment_thread_out_of_band_elicitation_count(
            self.state.as_ref(),
            thread_id,
        )
    }

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl std::future::Future<Output = CodexResult<u64>> + Send + '_ {
        thread_service_api::LiveThreadElicitationRuntime::decrement_thread_out_of_band_elicitation_count(
            self.state.as_ref(),
            thread_id,
        )
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

#[allow(clippy::manual_async_fn)]
fn thread_store_rollout_read_error(err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        err => CodexErr::Fatal(format!("failed to read thread by rollout path: {err}")),
    }
}

fn persisted_provider_facts_read_error(thread_id: ThreadId, err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message }
            if message == format!("no rollout found for thread id {thread_id}") =>
        {
            CodexErr::ThreadNotFound(thread_id)
        }
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        err => CodexErr::Fatal(format!(
            "failed to read persisted provider facts for thread {thread_id}: {err}"
        )),
    }
}

fn persisted_provider_facts_rollout_read_error(err: ThreadStoreError) -> CodexErr {
    match err {
        ThreadStoreError::ThreadNotFound { thread_id } => CodexErr::ThreadNotFound(thread_id),
        ThreadStoreError::InvalidRequest { message } => CodexErr::InvalidRequest(message),
        err => CodexErr::Fatal(format!(
            "failed to read persisted provider facts by rollout path: {err}"
        )),
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
