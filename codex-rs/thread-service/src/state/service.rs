use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock as StdRwLock;

use crate::StateDbHandle;
use crate::agent::AgentControl;
use crate::guardian::GuardianRejection;
use crate::guardian::GuardianRejectionCircuitBreaker;
use crate::network_approval::NetworkApprovalService;
use codex_analytics_api::AnalyticsEventsClient;
use codex_api_runtime_api::SharedApiRuntimeFactory;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_command_service_api::CommandServiceSessionState;
use codex_config::StartedNetworkProxy;
use codex_core_plugins_api::SharedPluginRuntime;
use codex_core_skills_api::SharedSkillsRuntime;
use codex_exec_server_api::ExecEnvironmentProvider;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionRegistry;
use codex_memories_read_api::SharedMemoryToolDeveloperInstructionsProvider;
use codex_model_client::AttestationProvider;
use codex_model_client::ModelClient;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_models_manager_api::SharedModelsManager;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::SharedOpenAiFileUploader;
use codex_permissions_runtime::ExecPolicyLoader;
use codex_permissions_runtime::ExecPolicyManager;
use codex_rollout_trace_api::ThreadTraceContext;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_session_telemetry_api::SharedSessionTelemetryFactory;
use thread_service_api::ActiveEventSubscriptionTracker;
use thread_service_api::ApprovalStore;
use codex_thread_store_api::LiveThreadFactory;
use codex_thread_store_api::SharedLiveThread;
use codex_thread_store_api::ThreadStore;
use mcp_service::McpManager;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use std::path::PathBuf;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(crate) struct SessionServices {
    pub(crate) mcp_connection_manager: Arc<RwLock<Box<dyn McpConnectionRuntime>>>,
    pub(crate) mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    pub(crate) mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    pub(crate) network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    pub(crate) sandbox_runtime: SharedSandboxRuntime,
    pub(crate) mcp_startup_cancellation_token: Mutex<CancellationToken>,
    pub(crate) command_service_state: Arc<dyn CommandServiceSessionState>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) shell_zsh_path: Option<PathBuf>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) main_execve_wrapper_exe: Option<PathBuf>,
    pub(crate) analytics_events_client: AnalyticsEventsClient,
    pub(crate) hooks: StdRwLock<codex_hooks_api::SharedHookRuntime>,
    pub(crate) hook_runtime_factory: codex_hooks_api::SharedHookRuntimeFactory,
    pub(crate) rollout_thread_trace: ThreadTraceContext,
    pub(crate) user_shell: Arc<crate::runtime_shell_model::Shell>,
    pub(crate) shell_snapshot_tx:
        watch::Sender<Option<Arc<crate::runtime_shell_snapshot::ShellSnapshot>>>,
    pub(crate) show_raw_agent_reasoning: bool,
    pub(crate) exec_policy: Arc<ExecPolicyManager>,
    pub(crate) exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    pub(crate) auth_runtime: SharedAuthRuntime,
    pub(crate) model_provider_factory: SharedModelProviderFactory,
    pub(crate) api_runtime_factory: SharedApiRuntimeFactory,
    pub(crate) session_telemetry_factory: SharedSessionTelemetryFactory,
    pub(crate) memory_tool_developer_instructions_provider:
        SharedMemoryToolDeveloperInstructionsProvider,
    pub(crate) models_manager: SharedModelsManager,
    pub(crate) session_telemetry: SharedSessionTelemetry,
    pub(crate) tool_approvals: Mutex<ApprovalStore>,
    pub(crate) guardian_rejections: Mutex<HashMap<String, GuardianRejection>>,
    pub(crate) guardian_rejection_circuit_breaker: Mutex<GuardianRejectionCircuitBreaker>,
    pub(crate) runtime_handle: Handle,
    pub(crate) skills_manager: SharedSkillsRuntime,
    pub(crate) plugins_manager: SharedPluginRuntime,
    pub(crate) mcp_manager: Arc<McpManager>,
    pub(crate) extensions: Arc<ExtensionRegistry<codex_config::Config>>,
    pub(crate) session_extension_data: ExtensionData,
    pub(crate) thread_extension_data: ExtensionData,
    pub(crate) agent_control: AgentControl,
    pub(crate) network_proxy: Option<StartedNetworkProxy>,
    pub(crate) network_approval: Arc<NetworkApprovalService>,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) live_thread: Option<SharedLiveThread>,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) live_thread_factory: Arc<dyn LiveThreadFactory>,
    pub(crate) attestation_provider: Option<Arc<dyn AttestationProvider>>,
    pub(crate) active_event_subscriptions: Arc<ActiveEventSubscriptionTracker>,
    /// Session-scoped model client shared across turns.
    pub(crate) model_client: ModelClient,
    pub(crate) openai_file_uploader: SharedOpenAiFileUploader,
    pub(crate) code_mode_service: Arc<dyn CodeModeRuntimeService>,
    pub(crate) code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    pub(crate) tool_service: Arc<crate::CoreToolServiceApi>,
    /// Shared process-level environment registry. Sessions carry an `Arc` handle so they can pass
    /// the same manager through child-thread spawn paths without reconstructing it.
    pub(crate) environment_manager: Arc<dyn ExecEnvironmentProvider>,
}
