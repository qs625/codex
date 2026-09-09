use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;

use crate::attestation::app_server_attestation_provider;
use crate::config_manager::ConfigManager;
use crate::connection_rpc_gate::ConnectionRpcGate;
use crate::error_code::invalid_request;
use crate::extensions::FileSubscriptionThreadHost;
use crate::extensions::thread_extensions;
use crate::fs_watch::FsWatchManager;
use crate::host_lifecycle::AppServerHostLifecycleToolRuntime;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::ConnectionRequestId;
use crate::outgoing_message::OutgoingMessageSender;
use crate::outgoing_message::RequestContext;
use crate::request_processors::AccountRequestProcessor;
use crate::request_processors::AppsRequestProcessor;
use crate::request_processors::CatalogRequestProcessor;
use crate::request_processors::CommandExecRequestProcessor;
use crate::request_processors::ConfigRequestProcessor;
use crate::request_processors::EnvironmentRequestProcessor;
use crate::request_processors::ExternalAgentConfigRequestProcessor;
use crate::request_processors::FeedbackRequestProcessor;
use crate::request_processors::FsRequestProcessor;
use crate::request_processors::InitializeRequestProcessor;
use crate::request_processors::MarketplaceRequestProcessor;
use crate::request_processors::McpRequestProcessor;
use crate::request_processors::PluginRequestProcessor;
use crate::request_processors::ProcessExecRequestProcessor;
use crate::request_processors::RemoteControlRequestProcessor;
use crate::request_processors::SearchRequestProcessor;
use crate::request_processors::ThreadGoalRequestProcessor;
use crate::request_processors::ThreadRequestProcessor;
use crate::request_processors::TurnRequestProcessor;
use crate::request_processors::WindowsSandboxRequestProcessor;
use crate::request_processors::WorkflowRequestProcessor;
use crate::request_serialization::QueuedInitializedRequest;
use crate::request_serialization::RequestSerializationQueueKey;
use crate::request_serialization::RequestSerializationQueues;
use crate::skills_watcher::SkillsWatcher;
use crate::thread_state::ConnectionCapabilities;
use crate::thread_state::HostLifecycleRegistrationError;
use crate::thread_state::ThreadStateManager;
use crate::thread_store_factory::thread_store_from_config;
use crate::transport::AppServerTransport;
use crate::transport::RemoteControlHandle;
use app_server_protocol::ChatgptAuthTokensRefreshParams;
use app_server_protocol::ChatgptAuthTokensRefreshReason;
use app_server_protocol::ChatgptAuthTokensRefreshResponse;
use app_server_protocol::ClientLifecycleRecoveryRecordParams;
use app_server_protocol::ClientLifecycleRecoveryRecordResponse;
use app_server_protocol::ClientLifecycleRegisterResponse;
use app_server_protocol::ClientNotification;
use app_server_protocol::ClientRequest;
use app_server_protocol::ClientResponsePayload;
use app_server_protocol::ConfigWarningNotification;
use app_server_protocol::ExperimentalApi;
use app_server_protocol::JSONRPCError;
use app_server_protocol::JSONRPCErrorError;
use app_server_protocol::JSONRPCNotification;
use app_server_protocol::JSONRPCRequest;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::ServerRequestPayload;
use app_server_protocol::experimental_required_message;
use async_trait::async_trait;
use codex_analytics::AnalyticsEventsClient;
use codex_analytics::AppServerRpcTransport;
use codex_arg0::Arg0DispatchPaths;
use codex_auth_types::AuthMode as LoginAuthMode;
use codex_chatgpt::workspace_settings;
use codex_exec_server::EnvironmentManager;
use codex_feedback::CodexFeedback;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::auth::ExternalAuth;
use codex_login::auth::ExternalAuthRefreshContext;
use codex_login::auth::ExternalAuthRefreshReason;
use codex_login::auth::ExternalAuthTokens;
use codex_login::model_provider_auth_manager;
use codex_terminal_detection::user_agent;
use exec_server_api::ExecEnvironmentProvider;
use plugin_service::PluginAnalyticsEventSink;
use plugin_service::PluginsManager;
use plugin_service::RemotePluginAuth;
use plugin_service::RemotePluginAuthFuture;
use plugin_service::RemotePluginAuthProvider;
use plugin_service_api::PluginTelemetryMetadata;
use protocol::ThreadId;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::ClientRecoveryRecordedEvent;
use protocol::protocol::EventMsg;
use protocol::protocol::SessionSource;
use protocol::protocol::W3cTraceContext;
use rollout::StateDbHandle;
use state::log_db::LogDbLayer;
use state_api::SharedStateDbRuntime;
use thread_service::ThreadAuthRuntimes;
use thread_service::ThreadService;
use thread_service::config::Config;
use thread_service_api::LiveThreadHistoryRuntime;
use thread_service_api::LiveThreadRecoveryRuntime;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio::time::Duration;
use tokio::time::timeout;
use tracing::Instrument;
use uuid::Uuid;

const HOST_LIFECYCLE_CLIENT_NAME: &str = "root_worker_prototype_electron";

pub(crate) fn is_host_lifecycle_connection(
    transport: &AppServerTransport,
    client_name: Option<&str>,
) -> bool {
    matches!(transport, AppServerTransport::Stdio)
        && client_name == Some(HOST_LIFECYCLE_CLIENT_NAME)
}

const EXTERNAL_AUTH_REFRESH_TIMEOUT: Duration = Duration::from_secs(10);

struct LoginRemotePluginAuthProvider {
    auth_manager: Arc<AuthManager>,
}

impl RemotePluginAuthProvider for LoginRemotePluginAuthProvider {
    fn remote_plugin_auth(&self) -> RemotePluginAuthFuture {
        let auth_manager = Arc::clone(&self.auth_manager);
        Box::pin(async move {
            auth_manager
                .auth()
                .await
                .as_ref()
                .map(remote_plugin_auth_from_codex_auth)
        })
    }
}

fn remote_plugin_auth_from_codex_auth(auth: &CodexAuth) -> RemotePluginAuth {
    RemotePluginAuth::new(
        auth.request_auth_snapshot(),
        auth.get_account_id(),
        auth.get_chatgpt_user_id(),
        auth.is_workspace_account(),
    )
}

#[derive(Clone)]
struct ExternalAuthRefreshBridge {
    outgoing: Arc<OutgoingMessageSender>,
}

struct AppServerPluginAnalyticsEventSink {
    analytics_events_client: AnalyticsEventsClient,
}

impl PluginAnalyticsEventSink for AppServerPluginAnalyticsEventSink {
    fn track_plugin_installed(&self, plugin: PluginTelemetryMetadata) {
        self.analytics_events_client.track_plugin_installed(plugin);
    }

    fn track_plugin_uninstalled(&self, plugin: PluginTelemetryMetadata) {
        self.analytics_events_client
            .track_plugin_uninstalled(plugin);
    }
}

impl ExternalAuthRefreshBridge {
    fn map_reason(reason: ExternalAuthRefreshReason) -> ChatgptAuthTokensRefreshReason {
        match reason {
            ExternalAuthRefreshReason::Unauthorized => ChatgptAuthTokensRefreshReason::Unauthorized,
        }
    }
}

#[async_trait]
impl ExternalAuth for ExternalAuthRefreshBridge {
    fn auth_mode(&self) -> LoginAuthMode {
        LoginAuthMode::Chatgpt
    }

    async fn refresh(
        &self,
        context: ExternalAuthRefreshContext,
    ) -> std::io::Result<ExternalAuthTokens> {
        let params = ChatgptAuthTokensRefreshParams {
            reason: Self::map_reason(context.reason),
            previous_account_id: context.previous_account_id,
        };

        let (request_id, rx) = self
            .outgoing
            .send_request(ServerRequestPayload::ChatgptAuthTokensRefresh(params))
            .await;

        let result = match timeout(EXTERNAL_AUTH_REFRESH_TIMEOUT, rx).await {
            Ok(result) => {
                // Two failure scenarios:
                // 1) `oneshot::Receiver` failed (sender dropped) => request canceled/channel closed.
                // 2) client answered with JSON-RPC error payload => propagate code/message.
                let result = result.map_err(|err| {
                    std::io::Error::other(format!("auth refresh request canceled: {err}"))
                })?;
                result.map_err(|err| {
                    std::io::Error::other(format!(
                        "auth refresh request failed: code={} message={}",
                        err.code, err.message
                    ))
                })?
            }
            Err(_) => {
                let _canceled = self.outgoing.cancel_request(&request_id).await;
                return Err(std::io::Error::other(format!(
                    "auth refresh request timed out after {}s",
                    EXTERNAL_AUTH_REFRESH_TIMEOUT.as_secs()
                )));
            }
        };

        let response: ChatgptAuthTokensRefreshResponse =
            serde_json::from_value(result).map_err(std::io::Error::other)?;

        Ok(ExternalAuthTokens::chatgpt(
            response.access_token,
            response.chatgpt_account_id,
            response.chatgpt_plan_type,
        ))
    }
}

pub(crate) struct MessageProcessor {
    outgoing: Arc<OutgoingMessageSender>,
    account_processor: AccountRequestProcessor,
    apps_processor: AppsRequestProcessor,
    catalog_processor: CatalogRequestProcessor,
    command_exec_processor: CommandExecRequestProcessor,
    process_exec_processor: ProcessExecRequestProcessor,
    config_processor: ConfigRequestProcessor,
    environment_processor: EnvironmentRequestProcessor,
    external_agent_config_processor: ExternalAgentConfigRequestProcessor,
    feedback_processor: FeedbackRequestProcessor,
    fs_processor: FsRequestProcessor,
    initialize_processor: InitializeRequestProcessor,
    marketplace_processor: MarketplaceRequestProcessor,
    mcp_processor: McpRequestProcessor,
    plugin_processor: PluginRequestProcessor,
    remote_control_processor: RemoteControlRequestProcessor,
    search_processor: SearchRequestProcessor,
    thread_goal_processor: ThreadGoalRequestProcessor,
    thread_processor: ThreadRequestProcessor,
    turn_processor: TurnRequestProcessor,
    windows_sandbox_processor: WindowsSandboxRequestProcessor,
    workflow_processor: WorkflowRequestProcessor,
    config: Arc<Config>,
    thread_service: Arc<ThreadService>,
    thread_state_manager: ThreadStateManager,
    host_recovery_lock: Mutex<()>,
    request_serialization_queues: RequestSerializationQueues,
}

#[derive(Debug)]
pub(crate) struct ConnectionSessionState {
    pub(crate) rpc_gate: Arc<ConnectionRpcGate>,
    initialized: OnceLock<InitializedConnectionSessionState>,
}

#[derive(Debug)]
pub(crate) struct InitializedConnectionSessionState {
    pub(crate) experimental_api_enabled: bool,
    pub(crate) opted_out_notification_methods: HashSet<String>,
    pub(crate) app_server_client_name: String,
    pub(crate) client_version: String,
    pub(crate) request_attestation: bool,
}

impl Default for ConnectionSessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionSessionState {
    pub(crate) fn new() -> Self {
        Self {
            rpc_gate: Arc::new(ConnectionRpcGate::new()),
            initialized: OnceLock::new(),
        }
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized.get().is_some()
    }

    pub(crate) fn experimental_api_enabled(&self) -> bool {
        self.initialized
            .get()
            .is_some_and(|session| session.experimental_api_enabled)
    }

    pub(crate) fn opted_out_notification_methods(&self) -> HashSet<String> {
        self.initialized
            .get()
            .map(|session| session.opted_out_notification_methods.clone())
            .unwrap_or_default()
    }

    pub(crate) fn app_server_client_name(&self) -> Option<&str> {
        self.initialized
            .get()
            .map(|session| session.app_server_client_name.as_str())
    }

    pub(crate) fn client_version(&self) -> Option<&str> {
        self.initialized
            .get()
            .map(|session| session.client_version.as_str())
    }

    pub(crate) fn request_attestation(&self) -> bool {
        self.initialized
            .get()
            .is_some_and(|session| session.request_attestation)
    }

    pub(crate) fn initialize(&self, session: InitializedConnectionSessionState) -> Result<(), ()> {
        self.initialized.set(session).map_err(|_| ())
    }
}

pub(crate) struct MessageProcessorArgs {
    pub(crate) outgoing: Arc<OutgoingMessageSender>,
    pub(crate) analytics_events_client: AnalyticsEventsClient,
    pub(crate) arg0_paths: Arg0DispatchPaths,
    pub(crate) config: Arc<Config>,
    pub(crate) config_manager: ConfigManager,
    pub(crate) environment_manager: Arc<EnvironmentManager>,
    pub(crate) feedback: CodexFeedback,
    pub(crate) log_db: Option<LogDbLayer>,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) config_warnings: Vec<ConfigWarningNotification>,
    pub(crate) session_source: SessionSource,
    pub(crate) auth_manager: Arc<AuthManager>,
    pub(crate) installation_id: String,
    pub(crate) rpc_transport: AppServerRpcTransport,
    pub(crate) remote_control_handle: Option<RemoteControlHandle>,
    pub(crate) plugin_startup_tasks: crate::PluginStartupTasks,
}

impl MessageProcessor {
    /// Create a new `MessageProcessor`, retaining a handle to the outgoing
    /// `Sender` so handlers can enqueue messages to be written to stdout.
    pub(crate) fn new(args: MessageProcessorArgs) -> Self {
        let MessageProcessorArgs {
            outgoing,
            analytics_events_client,
            arg0_paths,
            config,
            config_manager,
            environment_manager,
            feedback,
            log_db,
            state_db,
            config_warnings,
            session_source,
            auth_manager,
            installation_id,
            rpc_transport,
            remote_control_handle,
            plugin_startup_tasks,
        } = args;
        auth_manager.set_external_auth(Arc::new(ExternalAuthRefreshBridge {
            outgoing: outgoing.clone(),
        }));
        let thread_state_manager = ThreadStateManager::new();
        // The thread store is intentionally process-scoped. Config reloads can
        // affect per-thread behavior, but they must not move newly started,
        // resumed, or forked threads to a different persistence backend/root.
        let thread_store = thread_store_from_config(config.as_ref(), state_db.clone());
        let fs_watch_manager = FsWatchManager::new(outgoing.clone());
        let shared_file_watcher = fs_watch_manager.file_watcher();
        let thread_watch_manager =
            crate::thread_status::ThreadWatchManager::new_with_outgoing(outgoing.clone());
        let plugins_manager = Arc::new(PluginsManager::new_with_restriction_product(
            config.codex_home.to_path_buf(),
            session_source.restriction_product(),
        ));
        let thread_service_plugin_runtime: plugin_service_api::SharedPluginRuntime =
            plugins_manager.clone();
        let host_lifecycle_runtime: Arc<dyn codex_tool_service::HostLifecycleToolRuntime> =
            Arc::new(AppServerHostLifecycleToolRuntime::new(
                outgoing.clone(),
                thread_state_manager.clone(),
            ));
        let workflow_service_slot = std::sync::OnceLock::new();
        let thread_service: Arc<ThreadService> =
            Arc::new_cyclic(|thread_service: &Weak<ThreadService>| {
                let agent_tool_runtime: Weak<dyn codex_tool_service::AgentToolRuntime> =
                    thread_service.clone();
                let host_lifecycle_runtime = Some(Arc::clone(&host_lifecycle_runtime));
                let workflow_thread_runtime: Weak<dyn codex_workflow::WorkflowThreadRuntime> =
                    thread_service.clone();
                let workflow_service = Arc::new(codex_workflow::WorkflowService::new(
                    config.codex_home.clone(),
                    workflow_thread_runtime,
                ));
                workflow_service_slot
                    .set(Arc::clone(&workflow_service))
                    .unwrap_or_else(|_| panic!("workflow service slot should only be set once"));
                let approval_service = Arc::new(approval_service::ApprovalService);
                let mcp_service = Arc::new(mcp_service::McpService::new(approval_service.clone()));
                let tool_service =
                    Arc::new(codex_tool_service::ToolService::new_with_host_lifecycle(
                        approval_service,
                        Arc::new(command_service::CommandService::new()),
                        Arc::new(goal_service::GoalService),
                        mcp_service.clone(),
                        Arc::new(permissions_service::PermissionsService),
                        workflow_service,
                        agent_tool_runtime,
                        host_lifecycle_runtime,
                    ));
                let runtime_environment_provider: Arc<dyn ExecEnvironmentProvider> =
                    environment_manager.clone();
                let auth_runtimes = ThreadAuthRuntimes::from_auth_runtime(
                    auth_manager.clone(),
                    model_provider_auth_manager(Some(auth_manager.clone())),
                );
                let file_subscription_host: Weak<dyn FileSubscriptionThreadHost> =
                    thread_service.clone();
                let thread_state_db: Option<SharedStateDbRuntime> = state_db
                    .clone()
                    .map(|state_db| state_db as SharedStateDbRuntime);
                ThreadService::new_with_openai_file_uploader(
                    config.as_ref(),
                    auth_runtimes,
                    session_source.clone(),
                    runtime_environment_provider,
                    thread_extensions(
                        shared_file_watcher,
                        file_subscription_host,
                        thread_watch_manager.clone(),
                    ),
                    Some(analytics_events_client.api_client()),
                    Arc::clone(&thread_store),
                    thread_state_db,
                    Arc::new(thread_store::DefaultLiveThreadFactory),
                    installation_id,
                    Some(app_server_attestation_provider(
                        outgoing.clone(),
                        thread_state_manager.clone(),
                    )),
                    Arc::new(model_service::DefaultModelProviderFactory),
                    Arc::new(codex_code_mode::V8CodeModeRuntimeFactory),
                    Arc::new(command_service::CommandService::new()),
                    Arc::new(approval_service::ApprovalService),
                    Arc::new(goal_service::GoalService),
                    Arc::new(mcp_service::DefaultMcpAuthRuntime),
                    Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
                    Arc::new(codex_openai_files::ReqwestOpenAiFileUploader),
                    Arc::new(permissions_service::StarlarkExecPolicyLoader),
                    Arc::new(model_service::DefaultApiRuntimeFactory),
                    Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
                    Arc::new(codex_sandboxing::SandboxManager::new()),
                    Arc::new(codex_otel::OtelSessionTelemetryFactory),
                    Arc::new(hooks::HooksRuntimeFactory),
                    Arc::new(memory_service::FsMemoryToolDeveloperInstructionsProvider),
                    Arc::new(skill_service::SkillService::new_with_restriction_product(
                        config.codex_home.clone(),
                        config.bundled_skills_enabled(),
                        session_source.restriction_product(),
                    )),
                    thread_service_plugin_runtime.clone(),
                    tool_service,
                    mcp_service,
                )
                .with_terminal_type(user_agent())
            });
        let workflow_service = workflow_service_slot
            .into_inner()
            .unwrap_or_else(|| panic!("workflow service should be initialized"));
        plugins_manager.set_plugin_analytics_event_sink(Arc::new(
            AppServerPluginAnalyticsEventSink {
                analytics_events_client: analytics_events_client.clone(),
            },
        ));
        let skill_service = thread_service.skill_service();
        let model_service = thread_service.model_service();
        let skills_watcher = SkillsWatcher::new(Arc::clone(&skill_service), outgoing.clone());

        let pending_thread_unloads = Arc::new(Mutex::new(HashSet::new()));
        let thread_list_state_permit = Arc::new(Semaphore::new(/*permits*/ 1));
        let workspace_settings_cache =
            Arc::new(workspace_settings::WorkspaceSettingsCache::default());
        let account_processor = AccountRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&model_service),
            Arc::clone(&skill_service),
            Arc::clone(&plugins_manager),
            outgoing.clone(),
            Arc::clone(&config),
            config_manager.clone(),
        );
        let apps_processor = AppsRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            outgoing.clone(),
            config_manager.clone(),
            Arc::clone(&environment_manager),
            Arc::clone(&workspace_settings_cache),
        );
        let catalog_processor = CatalogRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&skill_service),
            Arc::clone(&plugins_manager),
            Arc::clone(&config),
            config_manager.clone(),
            Arc::clone(&environment_manager),
            Arc::clone(&workspace_settings_cache),
        );
        let command_exec_processor = CommandExecRequestProcessor::new(
            arg0_paths.clone(),
            Arc::clone(&config),
            outgoing.clone(),
            Arc::new(codex_sandboxing::SandboxManager::new()),
        );
        let process_exec_processor = ProcessExecRequestProcessor::new(outgoing.clone());
        let feedback_processor = FeedbackRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&config),
            feedback,
            log_db,
            state_db.clone(),
        );
        let initialize_processor = InitializeRequestProcessor::new(
            outgoing.clone(),
            analytics_events_client.clone(),
            Arc::clone(&config),
            config_warnings,
            rpc_transport,
        );
        let marketplace_processor = MarketplaceRequestProcessor::new(
            Arc::clone(&config),
            config_manager.clone(),
            Arc::clone(&plugins_manager),
        );
        let mcp_processor = McpRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            outgoing.clone(),
            config_manager.clone(),
            Arc::clone(&environment_manager),
        );
        let plugin_processor = PluginRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&model_service),
            Arc::clone(&skill_service),
            Arc::clone(&plugins_manager),
            outgoing.clone(),
            analytics_events_client.clone(),
            config_manager.clone(),
            Arc::clone(&environment_manager),
            workspace_settings_cache,
        );
        let remote_control_processor = RemoteControlRequestProcessor::new(remote_control_handle);
        let search_processor = SearchRequestProcessor::new(outgoing.clone());
        let thread_goal_processor = ThreadGoalRequestProcessor::new(
            Arc::clone(&thread_service),
            outgoing.clone(),
            Arc::clone(&config),
            thread_state_manager.clone(),
            state_db.clone(),
        );
        let thread_processor = ThreadRequestProcessor::new(
            Arc::clone(&thread_service),
            outgoing.clone(),
            arg0_paths.clone(),
            Arc::clone(&config),
            config_manager.clone(),
            Arc::clone(&thread_store),
            Arc::clone(&pending_thread_unloads),
            thread_state_manager.clone(),
            thread_watch_manager.clone(),
            Arc::clone(&thread_list_state_permit),
            thread_goal_processor.clone(),
            state_db.clone(),
            Arc::clone(&skills_watcher),
        );
        let turn_processor = TurnRequestProcessor::new(
            auth_manager.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&model_service),
            outgoing.clone(),
            analytics_events_client.clone(),
            arg0_paths.clone(),
            Arc::clone(&config),
            config_manager.clone(),
            Arc::clone(&thread_store),
            pending_thread_unloads,
            thread_state_manager.clone(),
            thread_watch_manager,
            thread_list_state_permit,
            state_db.clone(),
            Arc::clone(&skills_watcher),
        );
        if matches!(plugin_startup_tasks, crate::PluginStartupTasks::Start) {
            // Keep plugin startup warmups aligned at app-server startup.
            let on_effective_plugins_changed =
                plugin_processor.effective_plugins_changed_callback();
            plugin_service::maybe_start_plugin_startup_tasks_for_config(
                Arc::clone(&plugins_manager),
                Arc::clone(&model_service),
                &config.plugins_config_input(),
                Arc::new(LoginRemotePluginAuthProvider {
                    auth_manager: auth_manager.clone(),
                }),
                Some(on_effective_plugins_changed),
            );
        }
        let config_processor = ConfigRequestProcessor::new(
            outgoing.clone(),
            config_manager.clone(),
            auth_manager,
            thread_service.clone(),
            Arc::clone(&skill_service),
            Arc::clone(&plugins_manager),
            Arc::clone(&environment_manager),
            analytics_events_client,
        );
        let external_agent_config_processor = ExternalAgentConfigRequestProcessor::new(
            outgoing.clone(),
            Arc::clone(&thread_service),
            Arc::clone(&skill_service),
            Arc::clone(&plugins_manager),
            config_manager.clone(),
            config_processor.clone(),
            arg0_paths,
            config.codex_home.to_path_buf(),
        );
        let environment_processor =
            EnvironmentRequestProcessor::new(Arc::clone(&environment_manager));
        let fs_processor = FsRequestProcessor::new(
            environment_manager.local_environment().get_filesystem(),
            fs_watch_manager,
        );
        let windows_sandbox_processor = WindowsSandboxRequestProcessor::new(
            outgoing.clone(),
            Arc::clone(&config),
            config_manager.clone(),
        );
        let workflow_processor =
            WorkflowRequestProcessor::new(config_manager, outgoing.clone(), workflow_service);

        Self {
            outgoing,
            account_processor,
            apps_processor,
            catalog_processor,
            command_exec_processor,
            process_exec_processor,
            config_processor,
            environment_processor,
            external_agent_config_processor,
            feedback_processor,
            fs_processor,
            initialize_processor,
            marketplace_processor,
            mcp_processor,
            plugin_processor,
            remote_control_processor,
            search_processor,
            thread_goal_processor,
            thread_processor,
            turn_processor,
            windows_sandbox_processor,
            workflow_processor,
            config,
            thread_service,
            thread_state_manager,
            host_recovery_lock: Mutex::new(()),
            request_serialization_queues: RequestSerializationQueues::default(),
        }
    }

    pub(crate) fn clear_runtime_references(&self) {
        self.account_processor.clear_external_auth();
    }

    pub(crate) async fn process_request(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        request: JSONRPCRequest,
        transport: &AppServerTransport,
        session: Arc<ConnectionSessionState>,
    ) {
        let request_method = request.method.as_str();
        tracing::trace!(
            ?connection_id,
            request_id = ?request.id,
            "app-server request: {request_method}"
        );
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: request.id.clone(),
        };
        let request_span =
            crate::app_server_tracing::request_span(&request, transport, connection_id, &session);
        let request_trace = request.trace.as_ref().map(|trace| W3cTraceContext {
            traceparent: trace.traceparent.clone(),
            tracestate: trace.tracestate.clone(),
        });
        let request_context = RequestContext::new(request_id.clone(), request_span, request_trace);
        Self::run_request_with_context(
            Arc::clone(&self.outgoing),
            request_context.clone(),
            async {
                let codex_request = serde_json::to_value(&request)
                    .map_err(|err| invalid_request(format!("Invalid request: {err}")))
                    .and_then(|request_json| {
                        serde_json::from_value::<ClientRequest>(request_json)
                            .map_err(|err| invalid_request(format!("Invalid request: {err}")))
                    });
                let result = match codex_request {
                    Ok(codex_request) => {
                        // Websocket callers finalize outbound readiness in lib.rs after mirroring
                        // session state into outbound state and sending initialize notifications to
                        // this specific connection. Passing `None` avoids marking the connection
                        // ready too early from inside the shared request handler.
                        self.handle_client_request(
                            request_id.clone(),
                            codex_request,
                            Arc::clone(&session),
                            /*outbound_initialized*/ None,
                            matches!(transport, AppServerTransport::Stdio),
                            request_context.clone(),
                        )
                        .await
                    }
                    Err(error) => Err(error),
                };
                if let Err(error) = result {
                    self.outgoing.send_error(request_id.clone(), error).await;
                }
            },
        )
        .await;
    }

    /// Handles a typed request path used by in-process embedders.
    ///
    /// This bypasses JSON request deserialization but keeps identical request
    /// semantics by delegating to `handle_client_request`.
    pub(crate) async fn process_client_request(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        request: ClientRequest,
        session: Arc<ConnectionSessionState>,
        outbound_initialized: &AtomicBool,
    ) {
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: request.id().clone(),
        };
        let request_span =
            crate::app_server_tracing::typed_request_span(&request, connection_id, &session);
        let request_context =
            RequestContext::new(request_id.clone(), request_span, /*parent_trace*/ None);
        tracing::trace!(
            ?connection_id,
            request_id = ?request_id.request_id,
            "app-server typed request"
        );
        Self::run_request_with_context(
            Arc::clone(&self.outgoing),
            request_context.clone(),
            async {
                // In-process clients do not have the websocket transport loop that performs
                // post-initialize bookkeeping, so they still finalize outbound readiness in
                // the shared request handler.
                let result = self
                    .handle_client_request(
                        request_id.clone(),
                        request,
                        Arc::clone(&session),
                        Some(outbound_initialized),
                        false,
                        request_context.clone(),
                    )
                    .await;
                if let Err(error) = result {
                    self.outgoing.send_error(request_id.clone(), error).await;
                }
            },
        )
        .await;
    }

    pub(crate) async fn process_notification(&self, notification: JSONRPCNotification) {
        // Currently, we do not expect to receive any notifications from the
        // client, so we just log them.
        tracing::info!("<- notification: {:?}", notification);
    }

    /// Handles typed notifications from in-process clients.
    pub(crate) async fn process_client_notification(&self, notification: ClientNotification) {
        // Currently, we do not expect to receive any typed notifications from
        // in-process clients, so we just log them.
        tracing::info!("<- typed notification: {:?}", notification);
    }

    async fn run_request_with_context<F>(
        outgoing: Arc<OutgoingMessageSender>,
        request_context: RequestContext,
        request_fut: F,
    ) where
        F: Future<Output = ()>,
    {
        outgoing
            .register_request_context(request_context.clone())
            .await;
        request_fut.instrument(request_context.span()).await;
    }

    pub(crate) fn thread_created_receiver(
        &self,
    ) -> broadcast::Receiver<thread_service_api::ThreadCreatedEvent> {
        self.thread_processor.thread_created_receiver()
    }

    pub(crate) async fn send_initialize_notifications_to_connection(
        &self,
        connection_id: ConnectionId,
    ) {
        self.initialize_processor
            .send_initialize_notifications_to_connection(connection_id)
            .await;
    }

    pub(crate) async fn connection_initialized(
        &self,
        connection_id: ConnectionId,
        request_attestation: bool,
        host_lifecycle_eligible: bool,
    ) {
        self.thread_processor
            .connection_initialized(
                connection_id,
                ConnectionCapabilities {
                    host_lifecycle_eligible,
                    request_attestation,
                    ..Default::default()
                },
            )
            .await;
    }

    pub(crate) async fn send_initialize_notifications(&self) {
        self.initialize_processor
            .send_initialize_notifications()
            .await;
    }

    pub(crate) async fn try_attach_thread_listener(
        &self,
        thread_id: ThreadId,
        connection_ids: Vec<ConnectionId>,
    ) {
        self.thread_processor
            .try_attach_thread_listener(thread_id, connection_ids)
            .await;
    }

    pub(crate) async fn emit_thread_started_notification_to_connections(
        &self,
        thread_id: ThreadId,
        connection_ids: &[ConnectionId],
    ) {
        self.thread_processor
            .emit_thread_started_notification_to_connections(thread_id, connection_ids)
            .await;
    }

    pub(crate) async fn emit_thread_status_changed_notification_to_connections(
        &self,
        thread_id: ThreadId,
        authoritative_status: Option<protocol::protocol::AgentStatus>,
        connection_ids: &[ConnectionId],
    ) {
        self.thread_processor
            .emit_thread_status_changed_notification_to_connections(
                thread_id,
                authoritative_status,
                connection_ids,
            )
            .await;
    }

    pub(crate) async fn emit_thread_live_event_notification_to_connections(
        &self,
        thread_id: ThreadId,
        turn_id: String,
        event: protocol::protocol::EventMsg,
        connection_ids: &[ConnectionId],
    ) {
        self.thread_processor
            .emit_thread_live_event_notification_to_connections(
                thread_id,
                turn_id,
                event,
                connection_ids,
            )
            .await;
    }

    pub(crate) async fn drain_background_tasks(&self) {
        self.thread_processor.drain_background_tasks().await;
    }

    pub(crate) async fn cancel_active_login(&self) {
        self.account_processor.cancel_active_login().await;
    }

    pub(crate) async fn clear_all_thread_listeners(&self) {
        self.thread_processor.clear_all_thread_listeners().await;
    }

    pub(crate) async fn shutdown_threads(&self) {
        self.thread_processor.shutdown_threads().await;
    }

    pub(crate) async fn connection_closed(
        &self,
        connection_id: ConnectionId,
        session_state: &ConnectionSessionState,
    ) {
        session_state.rpc_gate.shutdown().await;
        self.outgoing.connection_closed(connection_id).await;
        self.fs_processor.connection_closed(connection_id).await;
        self.command_exec_processor
            .connection_closed(connection_id)
            .await;
        self.process_exec_processor
            .connection_closed(connection_id)
            .await;
        self.thread_processor.connection_closed(connection_id).await;
    }

    pub(crate) fn subscribe_running_assistant_turn_count(&self) -> watch::Receiver<usize> {
        self.thread_processor
            .subscribe_running_assistant_turn_count()
    }

    /// Handle a standalone JSON-RPC response originating from the peer.
    pub(crate) async fn process_response(&self, response: JSONRPCResponse) {
        tracing::info!("<- response: {:?}", response);
        let JSONRPCResponse { id, result, .. } = response;
        self.outgoing.notify_client_response(id, result).await
    }

    /// Handle an error object received from the peer.
    pub(crate) async fn process_error(&self, err: JSONRPCError) {
        tracing::error!("<- error: {:?}", err);
        self.outgoing.notify_client_error(err.id, err.error).await;
    }

    async fn handle_client_request(
        self: &Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        session: Arc<ConnectionSessionState>,
        // `Some(...)` means the caller wants initialize to immediately mark the
        // connection outbound-ready. Websocket JSON-RPC calls pass `None` so
        // lib.rs can deliver connection-scoped initialize notifications first.
        outbound_initialized: Option<&AtomicBool>,
        host_lifecycle_transport_eligible: bool,
        request_context: RequestContext,
    ) -> Result<(), JSONRPCErrorError> {
        let connection_id = connection_request_id.connection_id;
        if let ClientRequest::Initialize { request_id, params } = codex_request {
            let host_lifecycle_eligible = host_lifecycle_transport_eligible
                && params.client_info.name == HOST_LIFECYCLE_CLIENT_NAME;
            let connection_initialized = self
                .initialize_processor
                .initialize(
                    connection_id,
                    request_id,
                    params,
                    &session,
                    outbound_initialized,
                )
                .await?;
            if connection_initialized {
                self.thread_processor
                    .connection_initialized(
                        connection_id,
                        ConnectionCapabilities {
                            host_lifecycle_eligible,
                            request_attestation: session.request_attestation(),
                            ..Default::default()
                        },
                    )
                    .await;
            }
            return Ok(());
        }

        self.dispatch_initialized_client_request(
            connection_request_id,
            codex_request,
            session,
            request_context,
        )
        .await
    }

    async fn dispatch_initialized_client_request(
        self: &Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        session: Arc<ConnectionSessionState>,
        request_context: RequestContext,
    ) -> Result<(), JSONRPCErrorError> {
        if !session.initialized() {
            return Err(invalid_request("Not initialized"));
        }

        if let Some(reason) = codex_request.experimental_reason()
            && !session.experimental_api_enabled()
        {
            return Err(invalid_request(experimental_required_message(reason)));
        }
        let connection_id = connection_request_id.connection_id;
        self.initialize_processor.track_initialized_request(
            connection_id,
            connection_request_id.request_id.clone(),
            &codex_request,
        );

        let serialization_scope = codex_request.serialization_scope();
        let app_server_client_name = session.app_server_client_name().map(str::to_string);
        let client_version = session.client_version().map(str::to_string);
        let error_request_id = connection_request_id.clone();
        let rpc_gate = Arc::clone(&session.rpc_gate);
        let processor = Arc::clone(self);
        let span = request_context.span();
        let request = QueuedInitializedRequest::new(
            rpc_gate,
            async move {
                let processor_for_request = Arc::clone(&processor);
                let result = processor_for_request
                    .handle_initialized_client_request(
                        connection_request_id,
                        codex_request,
                        request_context,
                        app_server_client_name,
                        client_version,
                    )
                    .await;
                if let Err(error) = result {
                    processor.outgoing.send_error(error_request_id, error).await;
                }
            }
            .instrument(span),
        );

        if let Some(scope) = serialization_scope {
            let (key, access) = RequestSerializationQueueKey::from_scope(connection_id, scope);
            self.request_serialization_queues
                .enqueue(key, access, request)
                .await;
        } else {
            tokio::spawn(async move {
                request.run().await;
            });
        }
        Ok(())
    }

    async fn handle_initialized_client_request(
        self: Arc<Self>,
        connection_request_id: ConnectionRequestId,
        codex_request: ClientRequest,
        request_context: RequestContext,
        app_server_client_name: Option<String>,
        client_version: Option<String>,
    ) -> Result<(), JSONRPCErrorError> {
        let connection_id = connection_request_id.connection_id;
        let request_id = ConnectionRequestId {
            connection_id,
            request_id: codex_request.id().clone(),
        };

        let result: Result<Option<ClientResponsePayload>, JSONRPCErrorError> = match codex_request {
            ClientRequest::Initialize { .. } => {
                panic!("Initialize should be handled before initialized request dispatch");
            }
            ClientRequest::ClientLifecycleRegister { params, .. } => {
                let host_id = params.host_id.trim().to_string();
                if host_id.is_empty() {
                    Err(invalid_request("hostId must not be empty"))
                } else {
                    match self
                        .thread_state_manager
                        .register_host_lifecycle_connection(connection_id)
                        .await
                    {
                        Ok(()) => Ok(Some(
                            ClientLifecycleRegisterResponse {
                                registered: true,
                                host_id,
                                reason: None,
                            }
                            .into(),
                        )),
                        Err(HostLifecycleRegistrationError::AlreadyRegistered(
                            existing_connection_id,
                        )) => Ok(Some(
                            ClientLifecycleRegisterResponse {
                                registered: false,
                                host_id,
                                reason: Some(format!(
                                    "Host lifecycle consumer is already registered on connection {}",
                                    existing_connection_id.0
                                )),
                            }
                            .into(),
                        )),
                        Err(HostLifecycleRegistrationError::Ineligible) => Ok(Some(
                            ClientLifecycleRegisterResponse {
                                registered: false,
                                host_id,
                                reason: Some(
                                    "This connection is not the trusted Electron Host transport."
                                        .to_string(),
                                ),
                            }
                            .into(),
                        )),
                        Err(HostLifecycleRegistrationError::UnknownConnection) => Ok(Some(
                            ClientLifecycleRegisterResponse {
                                registered: false,
                                host_id,
                                reason: Some(
                                    "This connection is not registered with app-server."
                                        .to_string(),
                                ),
                            }
                            .into(),
                        )),
                    }
                }
            }
            ClientRequest::ClientLifecycleRecoveryRecord { params, .. } => {
                if !self
                    .thread_state_manager
                    .is_registered_host_lifecycle_connection(connection_id)
                    .await
                {
                    Err(invalid_request(
                        "client/lifecycle/recovery/record requires the registered trusted Host lifecycle connection",
                    ))
                } else {
                    let recovery_identity =
                        normalize_client_recovery_identity(&params.recovery_identity)?;
                    let launcher_claim_id =
                        normalize_client_recovery_claim_id(&params.launcher_claim_id)?;
                    validate_client_recovery_version(&params.launcher_evidence_version)?;
                    if params.transaction_id.trim().is_empty()
                        || params.request_id.trim().is_empty()
                        || params.prompt.trim().is_empty()
                        || params.evidence_path.trim().is_empty()
                    {
                        Err(invalid_request(
                            "transactionId, requestId, prompt, and evidencePath must not be empty",
                        ))
                    } else {
                        let _recovery_guard = self.host_recovery_lock.lock().await;
                        let (resolution_source, requested_target) =
                            client_recovery_resolution_source(
                                &params.target_thread_id,
                                params.requested_by_thread_id.as_deref(),
                            )?;
                        if params.requested_by_thread_id.is_some() {
                            let requester_exists = self
                                .thread_service
                                .thread_exists_live_or_persisted(resolution_source)
                                .await
                                .map_err(|err| {
                                    invalid_request(format!(
                                        "failed to validate requestedByThreadId: {err}"
                                    ))
                                })?;
                            validate_client_recovery_requester_exists(requester_exists)?;
                        }
                        let self_thread_id = self
                            .thread_service
                            .ensure_live_native_agent_reference(
                                resolution_source,
                                &SessionSource::Mcp,
                                self.config.as_ref().clone(),
                                "/self",
                            )
                            .await
                            .map_err(|err| {
                                invalid_request(format!(
                                    "failed to resolve materialized /self thread: {err}"
                                ))
                            })?;
                        validate_client_recovery_target(requested_target, self_thread_id)?;
                        let target_thread_id = self_thread_id;
                        let history = self
                            .thread_service
                            .live_thread_history(target_thread_id, false)
                            .await
                            .map_err(|err| {
                                invalid_request(format!(
                                    "failed to read target thread history: {err}"
                                ))
                            })?;
                        let recovery_state =
                            client_recovery_state(&history.items, &recovery_identity);
                        if let Some(recovery_state) = recovery_state {
                            validate_client_recovery_record(
                                &recovery_state.event,
                                &params,
                                &recovery_identity,
                            )?;
                            if !recovery_state.handled {
                                let input = ResponseItem::Message {
                                    id: Some(recovery_state.event.id.clone()),
                                    role: "user".to_string(),
                                    content: vec![ContentItem::InputText {
                                        text: recovery_state.event.prompt.clone(),
                                    }],
                                    phase: None,
                                };
                                self.thread_service
                                    .resume_live_thread_recovery(
                                        target_thread_id,
                                        input,
                                        recovery_state.event.id.clone(),
                                        recovery_state.event.turn_id.clone(),
                                    )
                                    .await
                                    .map_err(|err| {
                                        invalid_request(format!(
                                            "failed to resume client recovery: {err}"
                                        ))
                                    })?;
                            }
                            Ok(Some(
                                ClientLifecycleRecoveryRecordResponse {
                                    accepted: true,
                                    duplicate: true,
                                }
                                .into(),
                            ))
                        } else {
                            let recorded_at_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                                .min(i64::MAX as u128)
                                as i64;
                            let stable_id = format!("client-recovery:{recovery_identity}");
                            let input = ResponseItem::Message {
                                id: Some(stable_id.clone()),
                                role: "user".to_string(),
                                content: vec![ContentItem::InputText {
                                    text: params.prompt.clone(),
                                }],
                                phase: None,
                            };
                            let event =
                                EventMsg::ClientRecoveryRecorded(ClientRecoveryRecordedEvent {
                                    id: stable_id.clone(),
                                    turn_id: stable_id,
                                    recovery_identity: Some(recovery_identity),
                                    launcher_claim_id: Some(launcher_claim_id),
                                    launcher_evidence_version: Some(
                                        params.launcher_evidence_version,
                                    ),
                                    transaction_id: params.transaction_id.clone(),
                                    request_id: params.request_id.clone(),
                                    failed_build_id: params.failed_build_id,
                                    failed_build_hash: params.failed_build_hash,
                                    source_commit: params.source_commit,
                                    requested_by_thread_id: params.requested_by_thread_id,
                                    mode: params.mode,
                                    failure_phase: params.failure_phase,
                                    exit_code: params.exit_code,
                                    signal: params.signal,
                                    ready_timeout_ms: params.ready_timeout_ms,
                                    log_path: params.log_path,
                                    transaction_path: params.transaction_path,
                                    recovered_build_id: params.recovered_build_id,
                                    prompt: params.prompt,
                                    evidence_path: params.evidence_path,
                                    recorded_at_ms,
                                });
                            self.thread_service
                                .record_live_thread_recovery(target_thread_id, input, event)
                                .await
                                .map_err(|err| {
                                    invalid_request(format!(
                                        "failed to record client recovery: {err}"
                                    ))
                                })?;
                            Ok(Some(
                                ClientLifecycleRecoveryRecordResponse {
                                    accepted: true,
                                    duplicate: false,
                                }
                                .into(),
                            ))
                        }
                    }
                }
            }
            ClientRequest::ConfigRead { params, .. } => self
                .config_processor
                .read(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WindowsSandboxReadiness { .. } => self
                .windows_sandbox_processor
                .windows_sandbox_readiness()
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ExternalAgentConfigDetect { params, .. } => self
                .external_agent_config_processor
                .detect(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ExternalAgentConfigImport { params, .. } => self
                .external_agent_config_processor
                .import(request_id.clone(), params)
                .await
                .map(|()| None),
            ClientRequest::ConfigValueWrite { params, .. } => {
                self.config_processor.value_write(params).await.map(Some)
            }
            ClientRequest::ConfigBatchWrite { params, .. } => {
                self.config_processor.batch_write(params).await.map(Some)
            }
            ClientRequest::ExperimentalFeatureEnablementSet { params, .. } => {
                self.config_processor
                    .experimental_feature_enablement_set(request_id.clone(), params)
                    .await
            }
            ClientRequest::RemoteControlEnable { .. } => self
                .remote_control_processor
                .enable()
                .map(|response| Some(response.into())),
            ClientRequest::RemoteControlDisable { .. } => self
                .remote_control_processor
                .disable()
                .map(|response| Some(response.into())),
            ClientRequest::ConfigRequirementsRead { params: _, .. } => self
                .config_processor
                .config_requirements_read()
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowList { params, .. } => self
                .workflow_processor
                .list(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowDescribe { params, .. } => self
                .workflow_processor
                .describe(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowStart { params, .. } => self
                .workflow_processor
                .start(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowStatus { params, .. } => self
                .workflow_processor
                .status(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowResume { params, .. } => self
                .workflow_processor
                .resume(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::WorkflowAbort { params, .. } => self
                .workflow_processor
                .abort(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::EnvironmentAdd { params, .. } => {
                self.environment_processor.environment_add(params).await
            }
            ClientRequest::FsReadFile { params, .. } => self
                .fs_processor
                .read_file(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsWriteFile { params, .. } => self
                .fs_processor
                .write_file(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsCreateDirectory { params, .. } => self
                .fs_processor
                .create_directory(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsGetMetadata { params, .. } => self
                .fs_processor
                .get_metadata(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsReadDirectory { params, .. } => self
                .fs_processor
                .read_directory(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsRemove { params, .. } => self
                .fs_processor
                .remove(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsCopy { params, .. } => self
                .fs_processor
                .copy(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsWatch { params, .. } => self
                .fs_processor
                .watch(connection_id, params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FsUnwatch { params, .. } => self
                .fs_processor
                .unwatch(connection_id, params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ModelProviderCapabilitiesRead { params: _, .. } => self
                .config_processor
                .model_provider_capabilities_read()
                .await
                .map(|response| Some(response.into())),
            ClientRequest::ThreadStart { params, .. } => {
                self.thread_processor
                    .thread_start(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                        request_context,
                    )
                    .await
            }
            ClientRequest::ThreadUnsubscribe { params, .. } => {
                self.thread_processor
                    .thread_unsubscribe(&request_id, params)
                    .await
            }
            ClientRequest::ThreadResume { params, .. } => {
                self.thread_processor
                    .thread_resume(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::ThreadFork { params, .. } => {
                self.thread_processor
                    .thread_fork(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::ThreadArchive { params, .. } => {
                self.thread_processor
                    .thread_archive(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadIncrementElicitation { params, .. } => {
                self.thread_processor
                    .thread_increment_elicitation(params)
                    .await
            }
            ClientRequest::ThreadDecrementElicitation { params, .. } => {
                self.thread_processor
                    .thread_decrement_elicitation(params)
                    .await
            }
            ClientRequest::ThreadSetName { params, .. } => {
                self.thread_processor
                    .thread_set_name(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadGoalSet { params, .. } => {
                self.thread_goal_processor
                    .thread_goal_set(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadGoalGet { params, .. } => {
                self.thread_goal_processor.thread_goal_get(params).await
            }
            ClientRequest::ThreadGoalClear { params, .. } => {
                self.thread_goal_processor
                    .thread_goal_clear(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadMetadataUpdate { params, .. } => {
                self.thread_processor.thread_metadata_update(params).await
            }
            ClientRequest::ThreadMemoryModeSet { params, .. } => {
                self.thread_processor.thread_memory_mode_set(params).await
            }
            ClientRequest::MemoryReset { .. } => self.thread_processor.memory_reset().await,
            ClientRequest::ThreadUnarchive { params, .. } => {
                self.thread_processor
                    .thread_unarchive(request_id.clone(), params)
                    .await
            }
            ClientRequest::ThreadCompactStart { params, .. } => {
                self.thread_processor
                    .thread_compact_start(&request_id, params)
                    .await
            }
            ClientRequest::ThreadBackgroundTerminalsClean { params, .. } => {
                self.thread_processor
                    .thread_background_terminals_clean(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRollback { params, .. } => {
                self.thread_processor
                    .thread_rollback(&request_id, params)
                    .await
            }
            ClientRequest::ThreadList { params, .. } => {
                self.thread_processor.thread_list(params).await
            }
            ClientRequest::ThreadLoadedList { params, .. } => {
                self.thread_processor.thread_loaded_list(params).await
            }
            ClientRequest::ThreadRead { params, .. } => {
                self.thread_processor.thread_read(params).await
            }
            ClientRequest::ThreadTurnsList { params, .. } => {
                self.thread_processor.thread_turns_list(params).await
            }
            ClientRequest::ThreadTurnsItemsList { params, .. } => {
                self.thread_processor.thread_turns_items_list(params).await
            }
            ClientRequest::ThreadShellCommand { params, .. } => {
                self.thread_processor
                    .thread_shell_command(&request_id, params)
                    .await
            }
            ClientRequest::ThreadApproveGuardianDeniedAction { params, .. } => {
                self.thread_processor
                    .thread_approve_guardian_denied_action(&request_id, params)
                    .await
            }
            ClientRequest::SkillsList { params, .. } => {
                self.catalog_processor.skills_list(params).await
            }
            ClientRequest::HooksList { params, .. } => {
                self.catalog_processor.hooks_list(params).await
            }
            ClientRequest::MarketplaceAdd { params, .. } => {
                self.marketplace_processor.marketplace_add(params).await
            }
            ClientRequest::MarketplaceRemove { params, .. } => {
                self.marketplace_processor.marketplace_remove(params).await
            }
            ClientRequest::MarketplaceUpgrade { params, .. } => {
                self.marketplace_processor.marketplace_upgrade(params).await
            }
            ClientRequest::PluginList { params, .. } => {
                self.plugin_processor.plugin_list(params).await
            }
            ClientRequest::PluginRead { params, .. } => {
                self.plugin_processor.plugin_read(params).await
            }
            ClientRequest::PluginSkillRead { params, .. } => {
                self.plugin_processor.plugin_skill_read(params).await
            }
            ClientRequest::PluginShareSave { params, .. } => {
                self.plugin_processor.plugin_share_save(params).await
            }
            ClientRequest::PluginShareUpdateTargets { params, .. } => {
                self.plugin_processor
                    .plugin_share_update_targets(params)
                    .await
            }
            ClientRequest::PluginShareList { params, .. } => {
                self.plugin_processor.plugin_share_list(params).await
            }
            ClientRequest::PluginShareCheckout { params, .. } => {
                self.plugin_processor.plugin_share_checkout(params).await
            }
            ClientRequest::PluginShareDelete { params, .. } => {
                self.plugin_processor.plugin_share_delete(params).await
            }
            ClientRequest::AppsList { params, .. } => {
                self.apps_processor.apps_list(&request_id, params).await
            }
            ClientRequest::SkillsConfigWrite { params, .. } => {
                self.catalog_processor.skills_config_write(params).await
            }
            ClientRequest::PluginInstall { params, .. } => {
                self.plugin_processor.plugin_install(params).await
            }
            ClientRequest::PluginUninstall { params, .. } => {
                self.plugin_processor.plugin_uninstall(params).await
            }
            ClientRequest::ModelList { params, .. } => {
                self.catalog_processor.model_list(params).await
            }
            ClientRequest::AgentTypeList { params, .. } => {
                self.catalog_processor.agent_type_list(params).await
            }
            ClientRequest::ThreadProviderList { params, .. } => {
                self.catalog_processor.thread_provider_list(params).await
            }
            ClientRequest::ExperimentalFeatureList { params, .. } => {
                self.catalog_processor
                    .experimental_feature_list(params)
                    .await
            }
            ClientRequest::CollaborationModeList { params, .. } => {
                self.catalog_processor.collaboration_mode_list(params).await
            }
            ClientRequest::MockExperimentalMethod { params, .. } => {
                self.catalog_processor
                    .mock_experimental_method(params)
                    .await
            }
            ClientRequest::TurnStart { params, .. } => {
                self.turn_processor
                    .turn_start(
                        request_id.clone(),
                        params,
                        app_server_client_name.clone(),
                        client_version.clone(),
                    )
                    .await
            }
            ClientRequest::ThreadInjectItems { params, .. } => {
                self.turn_processor.thread_inject_items(params).await
            }
            ClientRequest::TurnSteer { params, .. } => {
                self.turn_processor.turn_steer(&request_id, params).await
            }
            ClientRequest::TurnInterrupt { params, .. } => {
                self.turn_processor
                    .turn_interrupt(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRealtimeStart { params, .. } => {
                self.turn_processor
                    .thread_realtime_start(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRealtimeAppendAudio { params, .. } => {
                self.turn_processor
                    .thread_realtime_append_audio(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRealtimeAppendText { params, .. } => {
                self.turn_processor
                    .thread_realtime_append_text(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRealtimeStop { params, .. } => {
                self.turn_processor
                    .thread_realtime_stop(&request_id, params)
                    .await
            }
            ClientRequest::ThreadRealtimeListVoices { params: _, .. } => {
                self.turn_processor.thread_realtime_list_voices().await
            }
            ClientRequest::ReviewStart { params, .. } => {
                self.turn_processor.review_start(&request_id, params).await
            }
            ClientRequest::McpServerOauthLogin { params, .. } => {
                self.mcp_processor.mcp_server_oauth_login(params).await
            }
            ClientRequest::McpServerRefresh { params, .. } => {
                self.mcp_processor.mcp_server_refresh(params).await
            }
            ClientRequest::McpServerStatusList { params, .. } => {
                self.mcp_processor
                    .mcp_server_status_list(&request_id, params)
                    .await
            }
            ClientRequest::McpResourceRead { params, .. } => {
                self.mcp_processor
                    .mcp_resource_read(&request_id, params)
                    .await
            }
            ClientRequest::McpServerToolCall { params, .. } => {
                self.mcp_processor
                    .mcp_server_tool_call(&request_id, params)
                    .await
            }
            ClientRequest::WindowsSandboxSetupStart { params, .. } => {
                self.windows_sandbox_processor
                    .windows_sandbox_setup_start(&request_id, params)
                    .await
            }
            ClientRequest::LoginAccount { params, .. } => {
                self.account_processor
                    .login_account(request_id.clone(), params)
                    .await
            }
            ClientRequest::LogoutAccount { .. } => {
                self.account_processor
                    .logout_account(request_id.clone())
                    .await
            }
            ClientRequest::CancelLoginAccount { params, .. } => {
                self.account_processor.cancel_login_account(params).await
            }
            ClientRequest::GetAccount { params, .. } => {
                self.account_processor.get_account(params).await
            }
            ClientRequest::GetAccountRateLimits { .. } => {
                self.account_processor.get_account_rate_limits().await
            }
            ClientRequest::SendAddCreditsNudgeEmail { params, .. } => {
                self.account_processor
                    .send_add_credits_nudge_email(params)
                    .await
            }
            ClientRequest::FuzzyFileSearch { params, .. } => self
                .search_processor
                .fuzzy_file_search(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FuzzyFileSearchSessionStart { params, .. } => self
                .search_processor
                .fuzzy_file_search_session_start_response(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FuzzyFileSearchSessionUpdate { params, .. } => self
                .search_processor
                .fuzzy_file_search_session_update_response(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::FuzzyFileSearchSessionStop { params, .. } => self
                .search_processor
                .fuzzy_file_search_session_stop(params)
                .await
                .map(|response| Some(response.into())),
            ClientRequest::OneOffCommandExec { params, .. } => {
                self.command_exec_processor
                    .one_off_command_exec(&request_id, params)
                    .await
            }
            ClientRequest::CommandExecWrite { params, .. } => {
                self.command_exec_processor
                    .command_exec_write(request_id.clone(), params)
                    .await
            }
            ClientRequest::CommandExecResize { params, .. } => {
                self.command_exec_processor
                    .command_exec_resize(request_id.clone(), params)
                    .await
            }
            ClientRequest::CommandExecTerminate { params, .. } => {
                self.command_exec_processor
                    .command_exec_terminate(request_id.clone(), params)
                    .await
            }
            ClientRequest::ProcessSpawn { params, .. } => self
                .process_exec_processor
                .process_spawn(request_id.clone(), params)
                .await
                .map(|()| None),
            ClientRequest::ProcessWriteStdin { params, .. } => {
                self.process_exec_processor
                    .process_write_stdin(request_id.clone(), params)
                    .await
            }
            ClientRequest::ProcessKill { params, .. } => {
                self.process_exec_processor
                    .process_kill(request_id.clone(), params)
                    .await
            }
            ClientRequest::ProcessResizePty { params, .. } => {
                self.process_exec_processor
                    .process_resize_pty(request_id.clone(), params)
                    .await
            }
            ClientRequest::FeedbackUpload { params, .. } => {
                self.feedback_processor.feedback_upload(params).await
            }
        };

        match result {
            Ok(Some(response)) => {
                self.outgoing
                    .send_response_as(request_id.clone(), response)
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                self.outgoing.send_error(request_id.clone(), error).await;
            }
        }
        Ok(())
    }
}

struct ClientRecoveryState {
    event: ClientRecoveryRecordedEvent,
    handled: bool,
}

fn client_recovery_state(
    items: &[protocol::protocol::RolloutItem],
    recovery_identity: &str,
) -> Option<ClientRecoveryState> {
    let event = items
        .iter()
        .filter_map(|item| match item {
            protocol::protocol::RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(event)) => {
                Some(event)
            }
            _ => None,
        })
        .find(|event| event.recovery_identity.as_deref() == Some(recovery_identity))?
        .clone();
    let handled = items.iter().any(|item| {
        matches!(
            item,
            protocol::protocol::RolloutItem::EventMsg(
                EventMsg::ClientRecoveryHandled(handled)
            ) if handled.recovery_id == event.id
                && handled
                    .recovery_identity
                    .as_deref()
                    .is_none_or(|identity| {
                        event.recovery_identity.as_deref() == Some(identity)
                    })
        )
    });
    Some(ClientRecoveryState { event, handled })
}

fn client_recovery_resolution_source(
    target_thread_id: &str,
    requested_by_thread_id: Option<&str>,
) -> Result<(ThreadId, Option<ThreadId>), JSONRPCErrorError> {
    if let Some(requested_by_thread_id) = requested_by_thread_id {
        if requested_by_thread_id.trim().is_empty() {
            return Err(invalid_request(
                "requestedByThreadId must be omitted for legacy supervisor evidence or contain a valid thread UUID",
            ));
        }
        let resolution_source = ThreadId::from_string(requested_by_thread_id)
            .map_err(|err| invalid_request(format!("invalid requestedByThreadId: {err}")))?;
        if target_thread_id != "/self" {
            return Err(invalid_request(
                "client recovery evidence with requestedByThreadId requires the literal /self targetThreadId",
            ));
        }
        return Ok((resolution_source, None));
    }

    if target_thread_id == "/self" {
        return Err(invalid_request(
            "legacy supervisor recovery evidence without requestedByThreadId requires the concrete materialized /self targetThreadId UUID",
        ));
    }
    let target_thread_id = ThreadId::from_string(target_thread_id)
        .map_err(|err| invalid_request(format!("invalid targetThreadId: {err}")))?;
    Ok((target_thread_id, Some(target_thread_id)))
}

fn normalize_client_recovery_identity(
    recovery_identity: &str,
) -> Result<String, JSONRPCErrorError> {
    let normalized = Uuid::parse_str(recovery_identity)
        .map(|identity| identity.to_string())
        .map_err(|err| invalid_request(format!("invalid recoveryIdentity UUID: {err}")))?;
    if normalized != recovery_identity {
        return Err(invalid_request("recoveryIdentity must be a canonical UUID"));
    }
    Ok(normalized)
}

fn normalize_client_recovery_claim_id(
    launcher_claim_id: &str,
) -> Result<String, JSONRPCErrorError> {
    let normalized = Uuid::parse_str(launcher_claim_id)
        .map(|identity| identity.to_string())
        .map_err(|err| invalid_request(format!("invalid launcherClaimId UUID: {err}")))?;
    if normalized != launcher_claim_id {
        return Err(invalid_request("launcherClaimId must be a canonical UUID"));
    }
    Ok(normalized)
}

fn validate_client_recovery_version(
    launcher_evidence_version: &str,
) -> Result<(), JSONRPCErrorError> {
    let digest = launcher_evidence_version
        .strip_prefix("sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        });
    if digest.is_none() {
        return Err(invalid_request(
            "launcherEvidenceVersion must be a canonical sha256 token",
        ));
    }
    Ok(())
}

fn validate_client_recovery_record(
    event: &ClientRecoveryRecordedEvent,
    params: &ClientLifecycleRecoveryRecordParams,
    recovery_identity: &str,
) -> Result<(), JSONRPCErrorError> {
    if event.recovery_identity.as_deref() != Some(recovery_identity)
        || event.launcher_claim_id.as_deref() != Some(params.launcher_claim_id.as_str())
        || event.launcher_evidence_version.as_deref()
            != Some(params.launcher_evidence_version.as_str())
        || event.transaction_id != params.transaction_id
        || event.request_id != params.request_id
        || event.failed_build_id != params.failed_build_id
        || event.failed_build_hash != params.failed_build_hash
        || event.source_commit != params.source_commit
        || event.requested_by_thread_id != params.requested_by_thread_id
        || event.mode != params.mode
        || event.failure_phase != params.failure_phase
        || event.exit_code != params.exit_code
        || event.signal != params.signal
        || event.ready_timeout_ms != params.ready_timeout_ms
        || event.log_path != params.log_path
        || event.transaction_path != params.transaction_path
        || event.recovered_build_id != params.recovered_build_id
        || event.prompt != params.prompt
        || event.evidence_path != params.evidence_path
    {
        return Err(invalid_request(
            "recoveryIdentity conflicts with the recorded frozen launcher recovery",
        ));
    }
    Ok(())
}

fn validate_client_recovery_target(
    requested_target: Option<ThreadId>,
    self_thread_id: ThreadId,
) -> Result<(), JSONRPCErrorError> {
    if requested_target.is_some_and(|target| target != self_thread_id) {
        return Err(invalid_request(
            "client recovery target must be the materialized /self thread",
        ));
    }
    Ok(())
}

fn validate_client_recovery_requester_exists(
    requester_exists: bool,
) -> Result<(), JSONRPCErrorError> {
    if !requester_exists {
        return Err(invalid_request(
            "requestedByThreadId does not identify a live or persisted thread",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "message_processor_tracing_tests.rs"]
mod message_processor_tracing_tests;

#[cfg(test)]
mod host_lifecycle_eligibility_tests {
    use super::*;
    use protocol::protocol::RolloutItem;

    #[test]
    fn only_electron_stdio_connection_is_host_lifecycle_eligible() {
        assert!(is_host_lifecycle_connection(
            &AppServerTransport::Stdio,
            Some(HOST_LIFECYCLE_CLIENT_NAME),
        ));
        assert!(!is_host_lifecycle_connection(
            &AppServerTransport::Stdio,
            Some("other-client"),
        ));
        assert!(!is_host_lifecycle_connection(
            &AppServerTransport::WebSocket {
                bind_address: "127.0.0.1:0".parse().expect("socket address"),
            },
            Some(HOST_LIFECYCLE_CLIENT_NAME),
        ));
    }

    #[test]
    fn recovery_idempotency_uses_only_modern_identity() {
        let first_identity = "11111111-1111-4111-8111-111111111111";
        let second_identity = "22222222-2222-4222-8222-222222222222";
        let mut items = vec![RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(
            ClientRecoveryRecordedEvent {
                id: "recovery-1".into(),
                turn_id: "turn-1".into(),
                recovery_identity: Some(first_identity.into()),
                launcher_claim_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
                launcher_evidence_version: Some(
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .into(),
                ),
                transaction_id: "tx-1".into(),
                request_id: "req-1".into(),
                failed_build_id: String::new(),
                failed_build_hash: String::new(),
                source_commit: String::new(),
                requested_by_thread_id: None,
                mode: "full".into(),
                failure_phase: "launch".into(),
                exit_code: None,
                signal: None,
                ready_timeout_ms: None,
                log_path: None,
                transaction_path: None,
                recovered_build_id: "build-good".into(),
                prompt: "recover".into(),
                evidence_path: "/tmp/evidence".into(),
                recorded_at_ms: 1,
            },
        ))];

        assert!(
            !client_recovery_state(&items, first_identity)
                .expect("matching recovery")
                .handled
        );
        assert!(
            client_recovery_state(&items, second_identity).is_none(),
            "the same pair with a different modern identity is independent"
        );

        items.push(RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(
            ClientRecoveryRecordedEvent {
                id: "recovery-2".into(),
                turn_id: "turn-2".into(),
                recovery_identity: Some(second_identity.into()),
                launcher_claim_id: Some("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into()),
                launcher_evidence_version: Some(
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        .into(),
                ),
                transaction_id: "tx-1".into(),
                request_id: "req-1".into(),
                failed_build_id: String::new(),
                failed_build_hash: String::new(),
                source_commit: String::new(),
                requested_by_thread_id: None,
                mode: "full".into(),
                failure_phase: "launch".into(),
                exit_code: None,
                signal: None,
                ready_timeout_ms: None,
                log_path: None,
                transaction_path: None,
                recovered_build_id: "build-good".into(),
                prompt: "recover second failure".into(),
                evidence_path: "/tmp/evidence-2".into(),
                recorded_at_ms: 2,
            },
        )));
        assert_eq!(
            client_recovery_state(&items, first_identity)
                .expect("same evidence retry remains idempotent")
                .event
                .id,
            "recovery-1"
        );
        assert_eq!(
            client_recovery_state(&items, second_identity)
                .expect("same pair with a different identity remains independent")
                .event
                .id,
            "recovery-2"
        );

        items.push(RolloutItem::EventMsg(EventMsg::ClientRecoveryHandled(
            protocol::protocol::ClientRecoveryHandledEvent {
                recovery_id: "recovery-1".into(),
                recovery_identity: Some(first_identity.into()),
                turn_id: "turn-1".into(),
                handled_at_ms: 3,
            },
        )));
        assert!(
            client_recovery_state(&items, first_identity)
                .expect("handled recovery")
                .handled
        );
        assert!(
            !client_recovery_state(&items, second_identity)
                .expect("handling the first recovery must not consume the second")
                .handled
        );

        items.push(RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(
            ClientRecoveryRecordedEvent {
                id: "legacy-recovery".into(),
                turn_id: "legacy-turn".into(),
                recovery_identity: None,
                launcher_claim_id: None,
                launcher_evidence_version: None,
                transaction_id: "legacy-tx".into(),
                request_id: "legacy-request".into(),
                failed_build_id: String::new(),
                failed_build_hash: String::new(),
                source_commit: String::new(),
                requested_by_thread_id: None,
                mode: "full".into(),
                failure_phase: "launch".into(),
                exit_code: None,
                signal: None,
                ready_timeout_ms: None,
                log_path: None,
                transaction_path: None,
                recovered_build_id: "build-good".into(),
                prompt: "recover legacy failure".into(),
                evidence_path: "/tmp/legacy-evidence".into(),
                recorded_at_ms: 4,
            },
        )));
        assert!(
            client_recovery_state(&items, "33333333-3333-4333-8333-333333333333",).is_none(),
            "modern evidence must not fall back to a legacy transaction/request pair"
        );
    }

    #[test]
    fn recovery_identity_requires_a_uuid() {
        assert!(normalize_client_recovery_identity("11111111-1111-4111-8111-111111111111").is_ok());
        assert!(normalize_client_recovery_identity("not-a-uuid").is_err());
        assert!(normalize_client_recovery_claim_id("not-a-uuid").is_err());
        assert!(
            validate_client_recovery_version(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .is_ok()
        );
        assert!(validate_client_recovery_version("sha256:ABC").is_err());
    }

    #[test]
    fn recovery_identity_requires_the_same_frozen_launcher_record() {
        let event = client_recovery_state(
            &[RolloutItem::EventMsg(EventMsg::ClientRecoveryRecorded(
                ClientRecoveryRecordedEvent {
                    id: "recovery".into(),
                    turn_id: "turn".into(),
                    recovery_identity: Some("11111111-1111-4111-8111-111111111111".into()),
                    launcher_claim_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
                    launcher_evidence_version: Some(
                        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                            .into(),
                    ),
                    transaction_id: "tx-1".into(),
                    request_id: "request-1".into(),
                    failed_build_id: String::new(),
                    failed_build_hash: String::new(),
                    source_commit: String::new(),
                    requested_by_thread_id: None,
                    mode: "full".into(),
                    failure_phase: "launch".into(),
                    exit_code: None,
                    signal: None,
                    ready_timeout_ms: None,
                    log_path: None,
                    transaction_path: None,
                    recovered_build_id: "recovered".into(),
                    prompt: "recover".into(),
                    evidence_path: "/tmp/evidence".into(),
                    recorded_at_ms: 1,
                },
            ))],
            "11111111-1111-4111-8111-111111111111",
        )
        .expect("identity match");
        let params = ClientLifecycleRecoveryRecordParams {
            target_thread_id: "/self".into(),
            recovery_identity: "11111111-1111-4111-8111-111111111111".into(),
            launcher_claim_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            launcher_evidence_version:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            transaction_id: "tx-1".into(),
            request_id: "request-1".into(),
            failed_build_id: String::new(),
            failed_build_hash: String::new(),
            source_commit: String::new(),
            requested_by_thread_id: None,
            mode: "full".into(),
            failure_phase: "launch".into(),
            exit_code: None,
            signal: None,
            ready_timeout_ms: None,
            log_path: None,
            transaction_path: None,
            recovered_build_id: "recovered".into(),
            prompt: "recover".into(),
            evidence_path: "/tmp/evidence".into(),
        };
        assert!(
            validate_client_recovery_record(&event.event, &params, &params.recovery_identity,)
                .is_ok(),
            "the same frozen record must remain idempotent"
        );
        for conflicting in [
            ClientLifecycleRecoveryRecordParams {
                launcher_claim_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
                ..params.clone()
            },
            ClientLifecycleRecoveryRecordParams {
                launcher_evidence_version:
                    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                ..params.clone()
            },
            ClientLifecycleRecoveryRecordParams {
                prompt: "different frozen prompt".into(),
                ..params.clone()
            },
        ] {
            assert!(
                validate_client_recovery_record(
                    &event.event,
                    &conflicting,
                    &params.recovery_identity,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn recovery_resolution_rejects_unknown_requester() {
        let target_thread_id = ThreadId::new();
        let error =
            client_recovery_resolution_source(&target_thread_id.to_string(), Some("unknown"))
                .expect_err("unknown requester must be rejected");

        assert!(error.message.contains("invalid requestedByThreadId"));
    }

    #[test]
    fn recovery_resolution_rejects_nonexistent_valid_requester_uuid() {
        let requester = ThreadId::new();
        client_recovery_resolution_source("/self", Some(&requester.to_string()))
            .expect("valid requester UUID syntax");
        let error = validate_client_recovery_requester_exists(false)
            .expect_err("nonexistent requester must be rejected");

        assert!(error.message.contains("live or persisted thread"));
    }

    #[test]
    fn recovery_resolution_rejects_empty_requester() {
        let target_thread_id = ThreadId::new();
        for requester in ["", "   "] {
            let error =
                client_recovery_resolution_source(&target_thread_id.to_string(), Some(requester))
                    .expect_err("empty requester must be rejected");

            assert!(error.message.contains("must be omitted"));
        }
    }

    #[test]
    fn modern_recovery_requires_literal_self_target() {
        let requester = ThreadId::new();
        let (resolution_source, requested_target) =
            client_recovery_resolution_source("/self", Some(&requester.to_string()))
                .expect("modern recovery with literal /self target");
        assert_eq!(resolution_source, requester);
        assert_eq!(requested_target, None);

        let concrete_target = ThreadId::new();
        let error = client_recovery_resolution_source(
            &concrete_target.to_string(),
            Some(&requester.to_string()),
        )
        .expect_err("modern recovery must not accept a concrete target UUID");
        assert!(error.message.contains("literal /self"));
    }

    #[test]
    fn legacy_recovery_requires_concrete_target_uuid() {
        let error = client_recovery_resolution_source("/self", None)
            .expect_err("legacy evidence must not use the /self alias");
        assert!(error.message.contains("concrete materialized /self"));

        let target_thread_id = ThreadId::new();
        let (resolution_source, requested_target) =
            client_recovery_resolution_source(&target_thread_id.to_string(), None)
                .expect("concrete legacy target");
        assert_eq!(resolution_source, target_thread_id);
        assert_eq!(requested_target, Some(target_thread_id));
    }

    #[test]
    fn legacy_recovery_target_must_match_resolved_self_thread() {
        let self_thread_id = ThreadId::new();
        validate_client_recovery_target(Some(self_thread_id), self_thread_id)
            .expect("matching concrete /self target");

        let error = validate_client_recovery_target(Some(ThreadId::new()), self_thread_id)
            .expect_err("non-self target must be rejected");
        assert!(error.message.contains("materialized /self"));
    }
}
