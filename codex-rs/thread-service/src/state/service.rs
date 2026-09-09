use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use crate::StateDbHandle;
use crate::agent::AgentControl;
use crate::session::session::approval_support_impl::ApprovalStore;
use codex_analytics_api::AnalyticsEventsClient;
use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::SessionNetworkApprovalApi;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use config_service::StartedNetworkProxy;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistry;
use codex_guardian::GuardianRejection;
use codex_guardian::GuardianRejectionCircuitBreaker;
use model_service::AttestationProvider;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::SharedOpenAiFileUploader;
use permissions_service::ExecPolicyLoader;
use permissions_service::ExecPolicyManager;
use codex_sandboxing_api::SharedSandboxRuntime;
use command_service_api::CommandServiceApi;
use command_service_api::CommandServiceSessionState;
use exec_server_api::ExecEnvironmentProvider;
use goal_service_api::GoalServiceApi;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpServiceApi;
use memory_service_api::SharedMemoryToolDeveloperInstructionsProvider;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelClientApi;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelProviderFactory;
use model_service_api::SharedModelServiceApi;
use plugin_service_api::SharedPluginRuntime;
use rollout_trace_api::ThreadTraceContext;
use session_telemetry_api::SharedSessionTelemetry;
use session_telemetry_api::SharedSessionTelemetryFactory;
use skill_service_api::SharedSkillServiceApi;
use std::path::PathBuf;
use thread_service_api::ActiveEventSubscriptionTracker;
use thread_store_api::LiveThreadFactory;
use thread_store_api::SharedLiveThread;
use thread_store_api::ThreadStore;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(crate) struct SessionServices {
    pub(crate) mcp_connection_manager: Arc<RwLock<Box<dyn McpConnectionRuntime>>>,
    pub(crate) mcp_service: Arc<dyn McpServiceApi>,
    pub(crate) mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    pub(crate) mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    pub(crate) network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    pub(crate) sandbox_runtime: SharedSandboxRuntime,
    pub(crate) mcp_startup_cancellation_token: Mutex<CancellationToken>,
    pub(crate) command_service_state: Arc<dyn CommandServiceSessionState>,
    pub(crate) command_service_api: Arc<dyn CommandServiceApi>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) shell_zsh_path: Option<PathBuf>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) main_execve_wrapper_exe: Option<PathBuf>,
    pub(crate) analytics_events_client: AnalyticsEventsClient,
    pub(crate) hooks: StdRwLock<hooks_api::SharedHookRuntime>,
    pub(crate) hook_runtime_factory: hooks_api::SharedHookRuntimeFactory,
    pub(crate) rollout_thread_trace: ThreadTraceContext,
    pub(crate) user_shell: Arc<crate::runtime_shell_model::Shell>,
    pub(crate) shell_snapshot_tx:
        watch::Sender<Option<Arc<crate::runtime_shell_snapshot::ShellSnapshot>>>,
    pub(crate) show_raw_agent_reasoning: bool,
    pub(crate) exec_policy: Arc<ExecPolicyManager>,
    pub(crate) exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    pub(crate) auth_runtime: SharedAuthRuntime,
    pub(crate) provider_auth_manager: Option<SharedModelProviderAuthManager>,
    pub(crate) model_provider_factory: SharedModelProviderFactory,
    pub(crate) api_runtime_factory: SharedApiRuntimeFactory,
    pub(crate) session_telemetry_factory: SharedSessionTelemetryFactory,
    pub(crate) memory_tool_developer_instructions_provider:
        SharedMemoryToolDeveloperInstructionsProvider,
    pub(crate) model_service: SharedModelServiceApi,
    pub(crate) session_telemetry: SharedSessionTelemetry,
    pub(crate) tool_approvals: Mutex<ApprovalStore>,
    pub(crate) guardian_rejections: Mutex<HashMap<String, GuardianRejection>>,
    pub(crate) guardian_rejection_circuit_breaker: Mutex<GuardianRejectionCircuitBreaker>,
    pub(crate) runtime_handle: Handle,
    pub(crate) skill_service: SharedSkillServiceApi,
    pub(crate) plugins_manager: SharedPluginRuntime,
    pub(crate) extensions: Arc<ExtensionRegistry<config_service::Config>>,
    pub(crate) session_extension_data: ExtensionData,
    pub(crate) thread_extension_data: ExtensionData,
    pub(crate) agent_control: AgentControl,
    pub(crate) network_proxy: Option<StartedNetworkProxy>,
    pub(crate) network_approval: Arc<dyn SessionNetworkApprovalApi>,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) live_thread: Option<SharedLiveThread>,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) live_thread_factory: Arc<dyn LiveThreadFactory>,
    pub(crate) attestation_provider: Option<Arc<dyn AttestationProvider>>,
    pub(crate) active_event_subscriptions: Arc<ActiveEventSubscriptionTracker>,
    pub(crate) model_client_api: SharedModelClientApi,
    pub(crate) openai_file_uploader: SharedOpenAiFileUploader,
    pub(crate) code_mode_service: Arc<dyn CodeModeRuntimeService>,
    pub(crate) code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    pub(crate) approval_service: Arc<dyn ApprovalServiceApi>,
    pub(crate) goal_service: Arc<dyn GoalServiceApi>,
    pub(crate) tool_service: Arc<crate::ToolServiceApi>,
    /// Shared process-level environment registry. Sessions carry an `Arc` handle so they can pass
    /// the same manager through child-thread spawn paths without reconstructing it.
    pub(crate) environment_manager: Arc<dyn ExecEnvironmentProvider>,
}
