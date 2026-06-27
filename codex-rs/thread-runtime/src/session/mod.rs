use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::agent::AgentControl;
use crate::agent::AgentMode;
use crate::agent::AgentStatus;
use crate::agent::SpawnAgentOptions;
use crate::agent::agent_status_from_event;
use crate::agent::status::is_final;
use crate::build_available_skills;
use crate::compact;
use crate::connectors;
use crate::context_usage::build_thread_context_usage;
use crate::context_usage::build_thread_context_usage_from_history;
use crate::default_skill_metadata_budget;
use crate::environment_selection::ResolvedTurnEnvironments;
use crate::event_mapping::completed_display_event_from_model_item;
use crate::event_mapping::injected_context_item_from_response_items;
use crate::event_mapping::is_structured_display_response_item;
use crate::event_mapping::started_display_event_from_model_item;
use crate::parse_turn_item;
use crate::path_utils::normalize_for_native_workdir;
use crate::realtime_conversation::RealtimeConversationManager;
use crate::session_prefix::format_subagent_notification_message;
use crate::skills::SkillRenderSideEffects;
use crate::skills_load_input_from_config;
use crate::turn_metadata::TurnMetadataState;
use crate::turn_timing::now_unix_timestamp_ms;
use crate::workflows::load_workflow_registry;
use async_channel::Receiver;
use async_channel::Sender;
use chrono::Local;
use chrono::Utc;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::BudgetLimitSteering;
use codex_agent_runtime::ListedAgent;
use codex_agent_runtime::LiveAgent;
use codex_agent_runtime::TerminalMetricEmission;
use codex_agent_runtime::ThreadPostTurnState;
use codex_analytics_api::AnalyticsEventsClient;
use codex_analytics_api::AppInvocation;
use codex_analytics_api::InvocationType;
use codex_analytics_api::SubAgentThreadStartedInput;
use codex_analytics_api::build_track_events_context;
use codex_api_runtime_api::SharedApiRuntimeFactory;
use codex_auth_types::AuthEnvTelemetryInput;
use codex_auth_types::AuthRuntime;
use codex_auth_types::SharedAuthRuntime;
use codex_auth_types::collect_auth_env_telemetry;
use codex_client_identity::originator;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_command_service_api::CommandSessionError;
use codex_command_service_api::CommandWaitOperation;
use codex_command_service_api::CommandWaitRequest;
use codex_command_service_api::WriteStdinOutput;
use codex_command_service_api::WriteStdinRequest;
use codex_config::ManagedFeatures;
use codex_config::hook_config_layer_stack_from_config_layer_stack;
use codex_config::resolve_tool_suggest_config_from_layer_stack;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_exec_server_api::ExecEnvironmentProvider;
use codex_extension_api::PromptSlot;
use codex_features::FEATURES;
use codex_features::Feature;
use codex_features::unstable_features_warning_event;
use codex_file_system::FileSystemSandboxContext;
use codex_file_system::LOCAL_FS;
use codex_hooks::PreToolUseHookResult;
use codex_hooks::record_additional_contexts;
use codex_hooks::run_post_tool_use_hooks;
use codex_hooks::run_pre_tool_use_hooks;
use codex_hooks_api::HooksConfig;
use codex_hooks_api::SharedHookRuntime;
use codex_hooks_api::SharedHookRuntimeFactory;
use codex_mcp_runtime_api::McpAuthRuntime;
use codex_mcp_runtime_api::McpConnectionRuntimeFactory;
use codex_mcp_runtime_api::McpConnectionRuntimeStartRequest;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::McpClientElicitationSupport;
use codex_mcp_types::McpServerElicitationRequest;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::codex_apps_tools_cache_key;
use codex_model_client::AttestationProvider;
use codex_model_provider_api::SharedModelProviderAuthManager;
use codex_model_provider_api::SharedModelProviderFactory;
use codex_models_manager_api::RefreshStrategy;
use codex_models_manager_api::SharedModelsManager;
use codex_network_proxy_api::BlockedRequestObserver;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkProxyAuditMetadata;
use codex_network_proxy_api::NetworkProxyRuntimeFactory;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::SharedOpenAiFileUploader;
use codex_permissions_runtime::ExecPolicyLoader;
use codex_permissions_runtime::ExecPolicyManager;
use codex_permissions_runtime::validate_network_policy_amendment_host;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::approvals::ElicitationRequestEvent;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::approvals::NetworkPolicyAmendment;
use codex_protocol::approvals::NetworkPolicyRuleAction;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::Settings;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::format_allow_prefixes;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::HasLegacyEvent;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::ResponseItemCompletedEvent;
use codex_protocol::protocol::ResponseItemStartedEvent;
use codex_protocol::protocol::ReviewRequest;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::ThreadContextUsage;
use codex_protocol::protocol::ThreadContextUsageUpdatedEvent;
use codex_protocol::protocol::ThreadSource;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::TurnContextItem;
use codex_protocol::protocol::TurnContextNetworkItem;
use codex_protocol::protocol::TurnEnvironmentSelection;
use codex_protocol::protocol::W3cTraceContext;
use codex_protocol::request_permissions::PermissionGrantScope;
use codex_protocol::request_permissions::RequestPermissionProfile;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_permissions::RequestPermissionsEvent;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_rollout_trace_api::AgentResultTracePayload;
use codex_rollout_trace_api::ThreadStartedTraceMetadata;
use codex_rollout_trace_api::ThreadTraceContext;
use codex_sandboxing_api::normalize_request_permissions_response;
use crate::Mailbox;
use crate::MailboxDeliveryPhase;
use crate::MailboxReceiver;
use crate::PendingInputItem;
use codex_shell_command::parse_command::parse_command;
use codex_thread_store_api::CreateThreadParams;
use codex_thread_store_api::LiveThreadFactory;
use codex_thread_store_api::LiveThreadHandle;
use codex_thread_store_api::ReadThreadParams;
use codex_thread_store_api::ResumeThreadParams;
use codex_thread_store_api::SharedLiveThread;
use codex_thread_store_api::ThreadEventPersistenceMode;
use codex_thread_store_api::ThreadPersistenceMetadata;
use codex_thread_store_api::ThreadStore;
use codex_utils_output_truncation::TruncationPolicy;
use futures::future::BoxFuture;
use futures::future::Shared;
use futures::prelude::*;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use toml::Value as TomlValue;
use tracing::Instrument;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::info_span;
use tracing::instrument;
use tracing::warn;
use uuid::Uuid;

#[cfg(test)]
use crate::compact::collect_user_messages;
use crate::thread::ThreadConfigSnapshot;
use codex_config::CONFIG_TOML_FILE;
use codex_config::Config;
use codex_config::Constrained;
use codex_config::ConstraintResult;
use codex_config::PermissionProfileState;
use codex_config::StartedNetworkProxy;
use codex_config_state::ConfigLayerStackOrdering;
use codex_config_types::ConfigLayerSource;
use codex_config_types::McpServerConfig;
use codex_context_manager::ContextManager;
use codex_context_manager::PreviousTurnSettingsView;
use codex_context_manager::SettingsUpdateInput;
use codex_context_manager::TotalTokenUsageBreakdown;
use codex_model_client::ModelClient;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
#[cfg(test)]
use codex_protocol::exec_output::StreamOutput;
use codex_rollout_api::initial_history_has_prior_user_turns;
use codex_thread_api::PostToolUsePayload;
use codex_tool_types::ToolName;
use codex_tool_types::UPDATE_GOAL_TOOL_NAME;
use codex_thread_api::ApprovalStore;

mod config_lock;
mod handlers;
mod mcp;
mod multi_agents;
mod review;
mod rollout_reconstruction;
#[allow(clippy::module_inception)]
pub(crate) mod session;
pub(crate) mod turn;
pub(crate) mod turn_context;
use self::config_lock::export_config_lock_if_configured;
use self::config_lock::validate_config_lock_if_configured;
#[cfg(test)]
use self::handlers::submission_dispatch_span;
use self::handlers::submission_loop;
use self::review::spawn_review_thread;
use self::session::AppServerClientMetadata;
use self::session::Session;
use self::session::SessionConfiguration;
use self::turn_context::TurnContext;
use self::turn_context::TurnSkillsContext;
#[cfg(test)]
mod rollout_reconstruction_tests;

pub(crate) use codex_rollout_api::PreviousTurnSettings;
use crate::ActiveSteerTurn;
pub(crate) use crate::SessionSettingsUpdate;
pub use crate::SteerInputError;
use crate::SteerableTaskKind;
use crate::resolve_session_service_tier;
use crate::validate_steer_input;

fn previous_turn_settings_view(
    previous_turn_settings: Option<&PreviousTurnSettings>,
) -> Option<PreviousTurnSettingsView<'_>> {
    previous_turn_settings.map(|settings| PreviousTurnSettingsView {
        model: settings.model.as_str(),
        realtime_active: settings.realtime_active,
    })
}

#[cfg(test)]
use crate::SkillLoadOutcome;
#[cfg(test)]
use crate::SkillMetadata;
use crate::agents_md::AgentsMdManager;
use crate::guardian::GuardianReviewSessionManager;
use crate::network_approval::NetworkApprovalService;
use crate::network_approval::build_blocked_request_observer;
use crate::network_approval::build_network_policy_decider;
use crate::network_policy_decision::execpolicy_network_rule_amendment;
use crate::rollout::map_session_init_error;
use crate::session_startup_prewarm::SessionStartupPrewarmHandle;
use crate::runtime_shell_model as shell;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::state::ActiveTurn;
use crate::state::PendingRequestPermissions;
use crate::state::SessionServices;
use crate::state::SessionState;
use crate::state::TaskKind;
use crate::state_db_bridge as state_db;
use crate::state_db_bridge::StateDbHandle;
#[cfg(test)]
use crate::stream_events_utils::HandleOutputCtx;
#[cfg(test)]
use crate::stream_events_utils::handle_output_item_done;
use crate::tasks::ReviewTask;
use crate::turn_timing::TurnTimingState;
use crate::turn_timing::record_turn_ttfm_metric;
use crate::windows_sandbox::WindowsSandboxLevelExt;
use codex_auth_types::TelemetryAuthMode;
use codex_context_manager::ApprovedCommandPrefixSaved;
use codex_context_manager::AppsInstructions;
use codex_context_manager::AvailableAgentsInstructions;
use codex_context_manager::AvailablePluginsInstructions;
use codex_context_manager::AvailableSkillsInstructions;
use codex_context_manager::AvailableWorkflowsInstructions;
use codex_context_manager::CollaborationModeInstructions;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::MultiagentContext;
use codex_context_manager::NetworkRuleSaved;
use codex_context_manager::PermissionsInstructions;
use codex_context_manager::PersonalitySpecInstructions;
use codex_context_manager::UserInstructions;
use codex_core_plugins_api::PluginLoadOutcome;
use codex_core_plugins_api::PluginRuntime;
use codex_core_plugins_api::SharedPluginRuntime;
use codex_core_skills_api::SharedSkillsRuntime;
use codex_git_info::get_git_repo_root;
use codex_mcp_runtime::McpManager;
use codex_mcp_types::effective_mcp_servers_from_configured;
use codex_mcp_types::host_owned_codex_apps_enabled;
use codex_memories_read_api::SharedMemoryToolDeveloperInstructionsProvider;
use codex_metrics_api::THREAD_STARTED_METRIC;
use codex_permissions_runtime::ExecPolicyUpdateError;
use codex_protocol::config_types::CollaborationMode;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::protocol::ApplyPatchApprovalRequestEvent;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::CodexErrorInfo;
use codex_protocol::protocol::CompactedItem;
use codex_protocol::protocol::DeprecationNoticeEvent;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecApprovalRequestEvent;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::McpServerRefreshConfig;
use codex_protocol::protocol::ModelRerouteEvent;
use codex_protocol::protocol::ModelRerouteReason;
use codex_protocol::protocol::ModelVerification;
use codex_protocol::protocol::ModelVerificationEvent;
use codex_protocol::protocol::NetworkApprovalContext;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::RateLimitSnapshot;
use codex_protocol::protocol::RequestUserInputEvent;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::protocol::SessionNetworkProxyRuntime;
use codex_protocol::protocol::StreamErrorEvent;
use codex_protocol::protocol::Submission;
use codex_protocol::protocol::ThreadMemoryMode;
use codex_protocol::protocol::TokenCountEvent;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_session_telemetry_api::SharedSessionTelemetryFactory;
use codex_tool_config::ToolEnvironmentMode;
use codex_tool_config::ToolsConfig;
use codex_tool_config::ToolsConfigParams;
use codex_command_service_api::ExecCommandRunOutput;
use codex_command_service_api::ExecCommandRunRequest;
use codex_trace_context::context_from_w3c_trace_context;
use codex_trace_context::current_span_trace_id;
use codex_trace_context::current_span_w3c_trace_context;
use codex_trace_context::set_parent_from_w3c_trace_context;
use codex_turn_items::realtime_text_for_event;
use codex_utils_absolute_path::AbsolutePathBuf;

/// The high-level interface to the Codex system.
/// It operates as a queue pair where you send submissions and receive events.
pub struct Codex {
    pub(crate) tx_sub: Sender<Submission>,
    pub(crate) rx_event: Receiver<Event>,
    // Last known status of the agent.
    pub(crate) agent_status: watch::Receiver<AgentStatus>,
    pub(crate) session: Arc<Session>,
    // Shared future for the background submission loop completion so multiple
    // callers can wait for shutdown.
    pub(crate) session_loop_termination: SessionLoopTermination,
}

pub(crate) type SessionLoopTermination = Shared<BoxFuture<'static, ()>>;

/// Wrapper returned by [`Codex::spawn`] containing the spawned [`Codex`] and
/// the unique session id.
pub struct CodexSpawnOk {
    pub codex: Codex,
    pub thread_id: ThreadId,
}

pub(crate) struct CodexSpawnArgs {
    pub(crate) config: Config,
    pub(crate) installation_id: String,
    pub(crate) terminal_type: String,
    pub(crate) auth_runtime: SharedAuthRuntime,
    pub(crate) provider_auth_manager: Option<SharedModelProviderAuthManager>,
    pub(crate) model_provider_factory: SharedModelProviderFactory,
    pub(crate) api_runtime_factory: SharedApiRuntimeFactory,
    pub(crate) session_telemetry_factory: SharedSessionTelemetryFactory,
    pub(crate) memory_tool_developer_instructions_provider:
        SharedMemoryToolDeveloperInstructionsProvider,
    pub(crate) hook_runtime_factory: SharedHookRuntimeFactory,
    pub(crate) sandbox_runtime: SharedSandboxRuntime,
    pub(crate) models_manager: SharedModelsManager,
    pub(crate) environment_manager: Arc<dyn ExecEnvironmentProvider>,
    pub(crate) skills_manager: SharedSkillsRuntime,
    pub(crate) plugins_manager: SharedPluginRuntime,
    pub(crate) mcp_manager: Arc<McpManager>,
    pub(crate) mcp_auth_runtime: Arc<dyn McpAuthRuntime>,
    pub(crate) mcp_connection_runtime_factory: Arc<dyn McpConnectionRuntimeFactory>,
    pub(crate) network_proxy_runtime_factory: SharedNetworkProxyRuntimeFactory,
    pub(crate) extensions: Arc<codex_extension_api::ExtensionRegistry<codex_config::Config>>,
    pub(crate) conversation_history: InitialHistory,
    pub(crate) session_source: SessionSource,
    pub(crate) thread_source: Option<ThreadSource>,
    pub(crate) agent_control: AgentControl,
    pub(crate) dynamic_tools: Vec<DynamicToolSpec>,
    pub(crate) persist_extended_history: bool,
    pub(crate) metrics_service_name: Option<String>,
    pub(crate) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(crate) inherited_exec_policy: Option<Arc<ExecPolicyManager>>,
    pub(crate) exec_policy_loader: Arc<dyn ExecPolicyLoader>,
    /// Parent rollout trace used only to derive fresh spawned child traces.
    ///
    /// Root sessions and non-thread-spawn subagents pass a disabled context;
    /// `Session::new` creates the root trace itself when rollout tracing is enabled.
    pub(crate) parent_rollout_thread_trace: ThreadTraceContext,
    pub(crate) user_shell_override: Option<shell::Shell>,
    pub(crate) parent_trace: Option<W3cTraceContext>,
    pub(crate) environment_selections: ResolvedTurnEnvironments,
    pub(crate) analytics_events_client: Option<AnalyticsEventsClient>,
    pub(crate) thread_store: Arc<dyn ThreadStore>,
    pub(crate) state_db: Option<StateDbHandle>,
    pub(crate) live_thread_factory: Arc<dyn LiveThreadFactory>,
    pub(crate) attestation_provider: Option<Arc<dyn AttestationProvider>>,
    pub(crate) active_event_subscriptions: Arc<crate::ActiveEventSubscriptionTracker>,
    pub(crate) openai_file_uploader: SharedOpenAiFileUploader,
    pub(crate) code_mode_service: Arc<dyn CodeModeRuntimeService>,
    pub(crate) code_mode_runtime_factory: Arc<dyn CodeModeRuntimeFactory>,
    pub(crate) tool_service: Arc<crate::CoreToolServiceApi>,
    pub(crate) workflow_runs: Arc<dyn codex_workflow_api::WorkflowRunController>,
}

pub(crate) const INITIAL_SUBMIT_ID: &str = "";
pub(crate) const SUBMISSION_CHANNEL_CAPACITY: usize = 512;
const CYBER_VERIFY_URL: &str = "https://chatgpt.com/cyber";
const CYBER_SAFETY_URL: &str = "https://developers.openai.com/codex/concepts/cyber-safety";

fn duration_from_config_ms(ms: i64) -> Duration {
    Duration::from_millis(ms.max(0) as u64)
}

/// Owns a live thread while session initialization is still fallible.
struct LiveThreadInitGuard {
    live_thread: Option<SharedLiveThread>,
}

impl LiveThreadInitGuard {
    fn new(live_thread: Option<SharedLiveThread>) -> Self {
        Self { live_thread }
    }

    fn as_ref(&self) -> Option<&SharedLiveThread> {
        self.live_thread.as_ref()
    }

    fn commit(&mut self) {
        self.live_thread = None;
    }

    async fn discard(&mut self) {
        let Some(live_thread) = self.live_thread.take() else {
            return;
        };
        if let Err(err) = live_thread.discard().await {
            warn!("failed to discard thread persistence for failed session init: {err}");
        }
    }
}

impl Drop for LiveThreadInitGuard {
    fn drop(&mut self) {
        let Some(live_thread) = self.live_thread.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            warn!("failed to discard thread persistence for failed session init: no Tokio runtime");
            return;
        };
        handle.spawn(async move {
            if let Err(err) = live_thread.discard().await {
                warn!("failed to discard thread persistence for failed session init: {err}");
            }
        });
    }
}

impl Codex {
    /// Spawn a new [`Codex`] and initialize the session.
    pub(crate) async fn spawn(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let parent_trace = match args.parent_trace {
            Some(trace) => {
                if context_from_w3c_trace_context(&trace).is_some() {
                    Some(trace)
                } else {
                    warn!("ignoring invalid thread spawn trace carrier");
                    None
                }
            }
            None => None,
        };
        let thread_spawn_span = info_span!("thread_spawn", otel.name = "thread_spawn");
        if let Some(trace) = parent_trace.as_ref() {
            let _ = set_parent_from_w3c_trace_context(&thread_spawn_span, trace);
        }
        Self::spawn_internal(CodexSpawnArgs {
            parent_trace,
            ..args
        })
        .instrument(thread_spawn_span)
        .await
    }

    async fn spawn_internal(args: CodexSpawnArgs) -> CodexResult<CodexSpawnOk> {
        let CodexSpawnArgs {
            mut config,
            installation_id,
            terminal_type,
            auth_runtime,
            provider_auth_manager,
            model_provider_factory,
            api_runtime_factory,
            session_telemetry_factory,
            models_manager,
            environment_manager,
            sandbox_runtime,
            skills_manager,
            plugins_manager,
            mcp_manager,
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            network_proxy_runtime_factory,
            hook_runtime_factory,
            extensions,
            conversation_history,
            session_source,
            thread_source,
            agent_control,
            dynamic_tools,
            persist_extended_history,
            metrics_service_name,
            inherited_shell_snapshot,
            user_shell_override,
            inherited_exec_policy,
            exec_policy_loader,
            parent_rollout_thread_trace,
            parent_trace: _,
            environment_selections,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            attestation_provider,
            active_event_subscriptions,
            openai_file_uploader,
            code_mode_service,
            code_mode_runtime_factory,
            tool_service,
            memory_tool_developer_instructions_provider,
            workflow_runs,
        } = args;
        let (tx_sub, rx_sub) = async_channel::bounded(SUBMISSION_CHANNEL_CAPACITY);
        let (tx_event, rx_event) = async_channel::unbounded();
        let fs = environment_selections.primary_filesystem();
        let plugins_input = config.plugins_config_input();
        let plugin_outcome = plugins_manager.plugins_for_config(&plugins_input).await;
        merge_plugin_agent_roles(&mut config, &plugin_outcome).await;
        let effective_skill_roots = plugin_outcome.effective_plugin_skill_roots();
        let skills_input = skills_load_input_from_config(&config, effective_skill_roots);
        let loaded_skills = skills_manager.skills_for_config(&skills_input, fs).await;

        for err in &loaded_skills.errors {
            error!(
                "failed to load skill {}: {}",
                err.path.display(),
                err.message
            );
        }

        let primary_environment = environment_selections.primary_environment();
        let user_instructions = AgentsMdManager::new(&config)
            .user_instructions(primary_environment.as_deref())
            .await;

        let exec_policy = if crate::guardian::is_guardian_reviewer_source(&session_source) {
            // Guardian review should rely on the built-in shell safety checks,
            // not on caller-provided exec-policy rules that could shape the
            // reviewer or silently auto-approve commands.
            Arc::new(ExecPolicyManager::default())
        } else if let Some(exec_policy) = &inherited_exec_policy {
            Arc::clone(exec_policy)
        } else {
            Arc::new(
                ExecPolicyManager::load(&config.config_layer_stack, exec_policy_loader.as_ref())
                    .await
                    .map_err(|err| CodexErr::Fatal(format!("failed to load rules: {err}")))?,
            )
        };

        let config = Arc::new(config);
        let refresh_strategy = if session_source.is_non_root_agent() {
            RefreshStrategy::Offline
        } else {
            RefreshStrategy::OnlineIfUncached
        };
        if config.model.is_none() || !matches!(refresh_strategy, RefreshStrategy::Offline) {
            let _ = models_manager.list_models(refresh_strategy).await;
        }
        let model = models_manager
            .get_default_model(&config.model, refresh_strategy)
            .await;

        // Resolve base instructions for the session. Priority order:
        // 1. config.base_instructions override
        // 2. conversation history => session_meta.base_instructions
        // 3. base_instructions for current model
        let model_info = models_manager
            .get_model_info(model.as_str(), &config.to_models_manager_config())
            .await;
        let base_instructions = config
            .base_instructions
            .clone()
            .or_else(|| conversation_history.get_base_instructions().map(|s| s.text))
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality));

        // Respect thread-start tools. When missing (resumed/forked threads), read from the db
        // first, then fall back to rollout-file tools.
        let persisted_tools = if dynamic_tools.is_empty() {
            let thread_id = match &conversation_history {
                InitialHistory::Resumed(resumed) => Some(resumed.conversation_id),
                InitialHistory::Forked(_) => conversation_history.forked_from_id(),
                InitialHistory::New | InitialHistory::Cleared => None,
            };
            match thread_id {
                Some(thread_id) => {
                    let state_db_ctx = if config.ephemeral {
                        None
                    } else {
                        state_db.clone()
                    };
                    state_db::get_dynamic_tools(state_db_ctx.as_deref(), thread_id, "codex_spawn")
                        .await
                }
                None => None,
            }
        } else {
            None
        };
        let dynamic_tools = if dynamic_tools.is_empty() {
            persisted_tools
                .or_else(|| conversation_history.get_dynamic_tools())
                .unwrap_or_default()
        } else {
            dynamic_tools
        };
        // TODO (aibrahim): Consolidate config.model and config.model_reasoning_effort into config.collaboration_mode
        // to avoid extracting these fields separately and constructing CollaborationMode here.
        let collaboration_mode = CollaborationMode {
            mode: ModeKind::Default,
            settings: Settings {
                model: model.clone(),
                reasoning_effort: config.model_reasoning_effort,
                developer_instructions: None,
            },
        };
        let auth_runtime_ref: &dyn AuthRuntime = auth_runtime.as_ref();
        let uses_enterprise_default_service_tier = auth_runtime_ref
            .telemetry_snapshot()
            .uses_enterprise_default_service_tier;
        let service_tier = resolve_session_service_tier(
            config.service_tier.clone(),
            config.notices.fast_default_opt_out.unwrap_or(false),
            uses_enterprise_default_service_tier,
            config.features.enabled(Feature::FastMode),
        );
        let session_configuration = SessionConfiguration {
            provider: config.model_provider.clone(),
            collaboration_mode,
            model_reasoning_summary: config.model_reasoning_summary,
            service_tier,
            developer_instructions: config.developer_instructions.clone(),
            user_instructions,
            personality: config.personality,
            base_instructions,
            compact_prompt: config.compact_prompt.clone(),
            approval_policy: config.permissions.approval_policy.clone(),
            approvals_reviewer: config.approvals_reviewer,
            permission_profile_state: session_permission_profile_state_from_config(&config)?,
            windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
            cwd: config.cwd.clone(),
            workspace_roots: config.workspace_roots.clone(),
            codex_home: config.codex_home.clone(),
            thread_name: None,
            environments: environment_selections.to_selections(),
            original_config_do_not_use: Arc::clone(&config),
            metrics_service_name,
            terminal_type,
            app_server_client_name: None,
            app_server_client_version: None,
            session_source,
            thread_source,
            dynamic_tools,
            persist_extended_history,
            inherited_shell_snapshot,
            user_shell_override,
        };

        // Generate a unique ID for the lifetime of this Codex session.
        let session_source_clone = session_configuration.session_source.clone();
        let (agent_status_tx, agent_status_rx) = watch::channel(AgentStatus::PendingInit);

        let session = Session::new(
            session_configuration,
            config.clone(),
            installation_id,
            auth_runtime,
            provider_auth_manager,
            model_provider_factory,
            models_manager.clone(),
            exec_policy,
            exec_policy_loader,
            tx_event.clone(),
            agent_status_tx.clone(),
            conversation_history,
            session_source_clone,
            skills_manager,
            plugins_manager,
            mcp_manager.clone(),
            mcp_auth_runtime,
            mcp_connection_runtime_factory,
            api_runtime_factory,
            session_telemetry_factory,
            memory_tool_developer_instructions_provider,
            hook_runtime_factory,
            sandbox_runtime,
            network_proxy_runtime_factory,
            extensions,
            agent_control,
            environment_manager,
            analytics_events_client,
            thread_store,
            state_db,
            live_thread_factory,
            parent_rollout_thread_trace,
            attestation_provider,
            active_event_subscriptions,
            openai_file_uploader,
            code_mode_service,
            code_mode_runtime_factory,
            tool_service,
            workflow_runs,
        )
        .await
        .map_err(|e| {
            error!("Failed to create session: {e:#}");
            map_session_init_error(&e, &config.codex_home)
        })?;
        let thread_id = session.conversation_id;

        // This task will run until Op::Shutdown is received.
        let session_for_loop = Arc::clone(&session);
        let session_loop_handle = tokio::spawn(async move {
            submission_loop(session_for_loop, config, rx_sub)
                .instrument(info_span!("session_loop", thread_id = %thread_id))
                .await;
        });
        let codex = Codex {
            tx_sub,
            rx_event,
            agent_status: agent_status_rx,
            session,
            session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
        };

        Ok(CodexSpawnOk { codex, thread_id })
    }

    /// Submit the `op` wrapped in a `Submission` with a unique ID.
    pub async fn submit(&self, op: Op) -> CodexResult<String> {
        self.submit_with_trace(op, /*trace*/ None).await
    }

    pub async fn submit_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> CodexResult<String> {
        let id = Uuid::now_v7().to_string();
        let sub = Submission {
            id: id.clone(),
            op,
            trace,
        };
        self.submit_with_id(sub).await?;
        Ok(id)
    }

    /// Use sparingly: prefer `submit()` so Codex is responsible for generating
    /// unique IDs for each submission.
    pub async fn submit_with_id(&self, mut sub: Submission) -> CodexResult<()> {
        if sub.trace.is_none() {
            sub.trace = current_span_w3c_trace_context();
        }
        self.tx_sub
            .send(sub)
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(())
    }

    /// Persist a thread-level memory mode update for the active session.
    ///
    /// This is a local-only operation that updates rollout metadata directly
    /// and does not involve the model.
    pub async fn set_thread_memory_mode(
        &self,
        mode: codex_protocol::protocol::ThreadMemoryMode,
    ) -> anyhow::Result<()> {
        handlers::persist_thread_memory_mode_update(&self.session, mode).await
    }

    pub async fn shutdown_and_wait(&self) -> CodexResult<()> {
        let session_loop_termination = self.session_loop_termination.clone();
        match self.submit(Op::Shutdown).await {
            Ok(_) => {}
            Err(CodexErr::InternalAgentDied) => {}
            Err(err) => return Err(err),
        }
        session_loop_termination.await;
        Ok(())
    }

    pub async fn next_event(&self) -> CodexResult<Event> {
        let event = self
            .rx_event
            .recv()
            .await
            .map_err(|_| CodexErr::InternalAgentDied)?;
        Ok(event)
    }

    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        self.session
            .steer_input(input, expected_turn_id, responsesapi_client_metadata)
            .await
    }

    pub(crate) async fn set_app_server_client_info(
        &self,
        app_server_client_name: Option<String>,
        app_server_client_version: Option<String>,
        mcp_elicitations_auto_deny: bool,
    ) -> ConstraintResult<()> {
        self.session
            .update_settings(SessionSettingsUpdate {
                app_server_client_name,
                app_server_client_version,
                ..Default::default()
            })
            .await?;
        let mcp_connection_manager = self.session.services.mcp_connection_manager.read().await;
        mcp_connection_manager.set_elicitations_auto_deny(mcp_elicitations_auto_deny);
        Ok(())
    }

    pub(crate) async fn agent_status(&self) -> AgentStatus {
        self.agent_status.borrow().clone()
    }

    pub(crate) async fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        let state = self.session.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    }

    pub(crate) async fn thread_environment_selections(&self) -> Vec<TurnEnvironmentSelection> {
        let state = self.session.state.lock().await;
        state.session_configuration.environments.clone()
    }

    pub(crate) fn state_db(&self) -> Option<state_db::StateDbHandle> {
        self.session.state_db()
    }

    pub(crate) fn enabled(&self, feature: Feature) -> bool {
        self.session.enabled(feature)
    }
}

async fn merge_plugin_agent_roles(config: &mut Config, plugin_outcome: &PluginLoadOutcome) {
    let plugin_agent_dirs = plugin_outcome
        .effective_plugin_agent_dirs()
        .into_iter()
        .map(|agent_dir| (agent_dir.plugin_id, agent_dir.path))
        .collect::<Vec<_>>();
    if plugin_agent_dirs.is_empty() {
        return;
    }

    let mut warnings = Vec::new();
    if let Err(err) = codex_config::agent_roles::merge_missing_agent_roles_from_plugin_dirs(
        LOCAL_FS.as_ref(),
        &mut config.agent_roles,
        &plugin_agent_dirs,
        &mut warnings,
    )
    .await
    {
        warn!("failed to load plugin agent definitions: {err}");
    }
    config.startup_warnings.extend(warnings);
}

fn session_permission_profile_state_from_config(
    config: &Config,
) -> CodexResult<PermissionProfileState> {
    Ok(config.permissions.permission_profile_state().clone())
}

fn steerable_task_kind(kind: TaskKind) -> SteerableTaskKind {
    match kind {
        TaskKind::Regular => SteerableTaskKind::Regular,
        TaskKind::Review => SteerableTaskKind::Review,
        TaskKind::Compact => SteerableTaskKind::Compact,
    }
}

#[cfg(test)]
pub(crate) fn completed_session_loop_termination() -> SessionLoopTermination {
    futures::future::ready(()).boxed().shared()
}

pub(crate) fn session_loop_termination_from_handle(
    handle: JoinHandle<()>,
) -> SessionLoopTermination {
    async move {
        let _ = handle.await;
    }
    .boxed()
    .shared()
}

async fn thread_title_from_thread_store(
    live_thread: Option<&dyn LiveThreadHandle>,
    thread_store: &Arc<dyn ThreadStore>,
    conversation_id: ThreadId,
) -> Option<String> {
    let thread = match live_thread {
        Some(live_thread) => {
            live_thread
                .read_thread(
                    /*include_archived*/ true, /*include_history*/ false,
                )
                .await
        }
        None => {
            thread_store
                .read_thread(ReadThreadParams {
                    thread_id: conversation_id,
                    include_archived: true,
                    include_history: false,
                })
                .await
        }
    }
    .ok()?;

    let title = thread.name.as_deref()?.trim();
    (!title.is_empty() && thread.preview.trim() != title).then(|| title.to_string())
}

fn tool_user_shell_type_to_core_shell_type(
    shell_type: &codex_tool_config::ToolUserShellType,
) -> crate::runtime_shell_model::ShellType {
    match shell_type {
        codex_tool_config::ToolUserShellType::Zsh => crate::runtime_shell_model::ShellType::Zsh,
        codex_tool_config::ToolUserShellType::Bash => crate::runtime_shell_model::ShellType::Bash,
        codex_tool_config::ToolUserShellType::PowerShell => crate::runtime_shell_model::ShellType::PowerShell,
        codex_tool_config::ToolUserShellType::Sh => crate::runtime_shell_model::ShellType::Sh,
        codex_tool_config::ToolUserShellType::Cmd => crate::runtime_shell_model::ShellType::Cmd,
    }
}

struct SessionTurnOpenAiFilePathResolver<'a> {
    turn_context: &'a TurnContext,
}

impl codex_mcp_runtime::OpenAiFilePathResolver for SessionTurnOpenAiFilePathResolver<'_> {
    fn resolve_path(&self, file_path: &str) -> PathBuf {
        #[allow(deprecated)]
        self.turn_context
            .resolve_path(Some(file_path.to_string()))
            .to_path_buf()
    }
}

impl Session {
    pub(crate) fn thread_id(&self) -> ThreadId {
        self.conversation_id
    }

    pub(crate) fn thread_id_string(&self) -> String {
        self.conversation_id.to_string()
    }

    pub(crate) fn derive_shell_exec_args(
        &self,
        command: &str,
        use_login_shell: bool,
    ) -> Vec<String> {
        self.user_shell().derive_exec_args(command, use_login_shell)
    }

    pub(crate) fn tool_user_shell_type(&self) -> codex_tool_config::ToolUserShellType {
        crate::runtime_shell::runtime_shell_type(&self.user_shell().shell_type)
    }

    pub(crate) fn runtime_shell(&self) -> codex_command_service_api::RuntimeShell {
        let shell = crate::runtime_shell::runtime_shell(self.user_shell().as_ref());
        codex_command_service_api::RuntimeShell {
            shell_type: shell.shell_type,
            shell_path: shell.shell_path,
            shell_snapshot: shell
                .shell_snapshot
                .map(|snapshot| codex_command_service_api::RuntimeShellSnapshot {
                    path: snapshot.path,
                    cwd: snapshot.cwd,
                }),
        }
    }

    pub(crate) async fn create_exec_approval_requirement(
        &self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'_>,
    ) -> codex_command_service_api::ExecApprovalRequirement {
        self.services
            .exec_policy
            .create_exec_approval_requirement_for_command(request)
            .await
    }

    pub(crate) fn sandbox_runtime(&self) -> codex_sandboxing_api::SharedSandboxRuntime {
        Arc::clone(&self.services.sandbox_runtime)
    }

    pub(crate) async fn collaboration_mode_kind(&self) -> ModeKind {
        self.collaboration_mode().await.mode
    }

    pub(crate) async fn code_mode_stored_values(&self) -> HashMap<String, serde_json::Value> {
        self.services.code_mode_service.stored_values().await
    }

    pub(crate) async fn code_mode_replace_stored_values(
        &self,
        values: HashMap<String, serde_json::Value>,
    ) {
        self.services
            .code_mode_service
            .replace_stored_values(values)
            .await;
    }

    pub(crate) fn code_mode_allocate_cell_id(&self) -> String {
        self.services.code_mode_service.allocate_cell_id()
    }

    pub(crate) async fn code_mode_execute(
        &self,
        request: ExecuteRequest,
    ) -> Result<RuntimeResponse, String> {
        self.services.code_mode_service.execute(request).await
    }

    pub(crate) async fn code_mode_wait(&self, request: WaitRequest) -> Result<WaitOutcome, String> {
        self.services.code_mode_service.wait(request).await
    }

    pub(crate) fn record_code_mode_cell_started(
        &self,
        turn_id: &str,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    ) {
        self.services.rollout_thread_trace.start_code_cell_trace(
            turn_id,
            runtime_cell_id,
            model_visible_call_id,
            source_js,
        );
    }

    pub(crate) fn record_code_mode_cell_initial_response(
        &self,
        turn_id: &str,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        self.services
            .rollout_thread_trace
            .code_cell_trace_context(turn_id, runtime_cell_id)
            .record_initial_response(response);
    }

    pub(crate) fn record_code_mode_cell_ended(
        &self,
        turn_id: &str,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        self.services
            .rollout_thread_trace
            .code_cell_trace_context(turn_id, runtime_cell_id)
            .record_ended(response);
    }

    pub(crate) fn current_agent_path_for_turn(&self, turn: &TurnContext) -> AgentPath {
        self.services
            .agent_control
            .current_agent_path(self.conversation_id, &turn.session_source())
    }

    pub(crate) fn register_session_root_for_turn(&self, turn: &TurnContext) {
        self.services
            .agent_control
            .register_session_root(self.conversation_id, &turn.session_source());
    }

    pub(crate) async fn resolve_agent_reference_for_turn(
        &self,
        turn: &TurnContext,
        target: &str,
    ) -> CodexResult<ThreadId> {
        self.services
            .agent_control
            .resolve_agent_reference(self.conversation_id, &turn.session_source(), target)
            .await
    }

    pub(crate) fn agent_metadata(&self, thread_id: ThreadId) -> AgentMetadata {
        self.services
            .agent_control
            .get_agent_metadata(thread_id)
            .unwrap_or_default()
    }

    pub(crate) async fn agent_status(&self, thread_id: ThreadId) -> AgentStatus {
        self.services.agent_control.get_status(thread_id).await
    }

    pub(crate) async fn subscribe_agent_status(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<watch::Receiver<AgentStatus>> {
        self.services
            .agent_control
            .subscribe_status(thread_id)
            .await
    }

    pub(crate) async fn list_agents_for_turn(
        &self,
        turn: &TurnContext,
        path_prefix: Option<&str>,
    ) -> CodexResult<Vec<ListedAgent>> {
        self.services
            .agent_control
            .list_agents(self.conversation_id, &turn.session_source(), path_prefix)
            .await
    }

    pub(crate) async fn send_inter_agent_communication(
        &self,
        receiver_thread_id: ThreadId,
        communication: InterAgentCommunication,
    ) -> CodexResult<()> {
        self.services
            .agent_control
            .send_inter_agent_communication(receiver_thread_id, communication)
            .await
            .map(|_| ())
    }

    pub(crate) async fn close_agent(&self, thread_id: ThreadId) -> CodexResult<()> {
        self.services
            .agent_control
            .close_agent(thread_id)
            .await
            .map(|_| ())
    }

    pub(crate) async fn shutdown_agent_job_worker(&self, thread_id: ThreadId) {
        let _ = self
            .services
            .agent_control
            .shutdown_live_agent(thread_id)
            .await;
    }

    pub(crate) async fn agent_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> Option<ThreadConfigSnapshot> {
        self.services
            .agent_control
            .get_agent_config_snapshot(thread_id)
            .await
    }

    pub(crate) async fn spawn_agent_with_metadata(
        &self,
        config: Config,
        initial_operation: Op,
        session_source: Option<SessionSource>,
        options: SpawnAgentOptions,
    ) -> CodexResult<LiveAgent> {
        self.services
            .agent_control
            .spawn_agent_with_metadata(config, initial_operation, session_source, options)
            .await
    }

    pub(crate) async fn list_all_mcp_tools(&self) -> Vec<codex_mcp_tool_types::ToolInfo> {
        let manager = self.services.mcp_connection_manager.read().await;
        manager.list_all_tools().await
    }

    pub(crate) async fn mcp_server_origin(&self, server: &str) -> Option<String> {
        let manager = self.services.mcp_connection_manager.read().await;
        codex_mcp_runtime_api::McpToolRuntime::server_origin(manager.as_ref(), server)
    }

    pub(crate) async fn mcp_server_is_host_owned_codex_apps(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        codex_mcp_runtime_api::McpToolRuntime::is_host_owned_codex_apps_server(
            manager.as_ref(),
            server,
        )
    }

    pub(crate) async fn mcp_server_supports_sandbox_state_meta(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        codex_mcp_runtime_api::McpToolRuntime::server_supports_sandbox_state_meta_capability(
            manager.as_ref(),
            server,
        )
        .await
        .unwrap_or(false)
    }

    pub(crate) async fn mcp_server_pollutes_memory(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        codex_mcp_runtime_api::McpToolRuntime::server_pollutes_memory(manager.as_ref(), server)
    }

    pub(crate) async fn hard_refresh_codex_apps_tools_cache(
        &self,
    ) -> anyhow::Result<Vec<codex_mcp_tool_types::ToolInfo>> {
        let manager = self.services.mcp_connection_manager.read().await;
        manager.hard_refresh_codex_apps_tools_cache().await
    }

    pub(crate) async fn rewrite_mcp_tool_arguments_for_openai_files(
        &self,
        turn: &TurnContext,
        arguments_value: Option<serde_json::Value>,
        openai_file_input_params: Option<&[String]>,
    ) -> Result<Option<serde_json::Value>, String> {
        let auth = self.services.auth_runtime.auth().await;
        let path_resolver = SessionTurnOpenAiFilePathResolver { turn_context: turn };
        codex_mcp_runtime::rewrite_mcp_tool_arguments_for_openai_files(
            self.services.openai_file_uploader.as_ref(),
            auth.as_ref(),
            turn.chatgpt_base_url(),
            &path_resolver,
            arguments_value,
            openai_file_input_params,
        )
        .await
    }

    pub(crate) fn add_optional_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        self.services
            .rollout_thread_trace
            .start_mcp_call_trace(call_id)
            .add_request_meta(meta)
    }

    pub(crate) async fn mark_thread_memory_mode_polluted_for_mcp_tool_call(
        &self,
        turn: &TurnContext,
        server: &str,
    ) {
        if !turn.external_context_pollutes_memory_mode() {
            return;
        }
        if !self.mcp_server_pollutes_memory(server).await {
            return;
        }
        state_db::mark_thread_memory_mode_polluted(
            self.services.state_db.as_deref(),
            self.conversation_id,
            "mcp_tool_call",
        )
        .await;
    }

    pub(crate) async fn track_codex_app_used_for_mcp_tool(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) {
        if server != codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME {
            return;
        }
        let metadata = self
            .lookup_mcp_app_usage_metadata(server, tool_name)
            .await
            .map(|metadata| (metadata.connector_id, metadata.app_name))
            .unwrap_or((None, None));
        let (connector_id, app_name) = metadata;
        let invocation_type = if let Some(connector_id) = connector_id.as_deref() {
            let mentioned_connector_ids = self.get_connector_selection().await;
            if mentioned_connector_ids.contains(connector_id) {
                InvocationType::Explicit
            } else {
                InvocationType::Implicit
            }
        } else {
            InvocationType::Implicit
        };

        let tracking = build_track_events_context(
            turn.model_slug().to_string(),
            self.conversation_id.to_string(),
            turn.turn_id(),
        );
        self.services.analytics_events_client.track_app_used(
            tracking,
            AppInvocation {
                connector_id,
                app_name,
                invocation_type: Some(invocation_type),
            },
        );
    }

    pub(crate) async fn lookup_mcp_app_usage_metadata(
        &self,
        server: &str,
        tool_name: &str,
    ) -> Option<codex_mcp_runtime::McpAppUsageMetadata> {
        let tools = self.list_all_mcp_tools().await;
        codex_mcp_runtime::lookup_mcp_app_usage_metadata(&tools, server, tool_name)
    }

    pub(crate) async fn mcp_tool_approval_is_remembered(
        &self,
        key: &codex_mcp_types::McpToolApprovalKey,
    ) -> bool {
        let store = self.services.tool_approvals.lock().await;
        matches!(store.get(key), Some(ReviewDecision::ApprovedForSession))
    }

    pub(crate) async fn remember_mcp_tool_approval(
        &self,
        key: codex_mcp_types::McpToolApprovalKey,
    ) {
        let mut store = self.services.tool_approvals.lock().await;
        store.put(key, ReviewDecision::ApprovedForSession);
    }

    pub(crate) fn plugins_manager(&self) -> &dyn codex_core_plugins_api::PluginRuntime {
        self.services.plugins_manager.as_ref()
    }

    pub(crate) async fn custom_mcp_tool_approval_mode(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) -> codex_config_types::AppToolApproval {
        codex_mcp_runtime::custom_mcp_tool_approval_mode(
            turn.config.as_ref(),
            self.services.plugins_manager.as_ref(),
            server,
            tool_name,
        )
        .await
    }

    pub(crate) async fn fetch_accessible_connectors_from_mcp_tools(
        &self,
        turn: &TurnContext,
        auth_snapshot: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> anyhow::Result<Vec<codex_connectors_types::AppInfo>> {
        connectors::list_accessible_connectors_from_mcp_tools(
            turn.config.as_ref(),
            auth_snapshot,
            self.services.plugins_manager.as_ref(),
            self.services.environment_manager.as_ref(),
            self.services.mcp_auth_runtime.as_ref(),
            self.services.mcp_connection_runtime_factory.as_ref(),
        )
        .await
    }

    pub(crate) async fn persist_codex_app_tool_approval_for_turn(
        &self,
        turn: &TurnContext,
        connector_id: &str,
        tool_name: &str,
    ) -> anyhow::Result<()> {
        codex_mcp_runtime::persist_codex_app_tool_approval(&turn.config, connector_id, tool_name)
            .await
    }

    pub(crate) async fn persist_non_app_mcp_tool_approval_for_turn(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) -> anyhow::Result<()> {
        codex_mcp_runtime::persist_non_app_mcp_tool_approval(
            &turn.config,
            self.services.plugins_manager.as_ref(),
            server,
            tool_name,
        )
        .await
    }

    pub(crate) async fn configured_mcp_servers(
        &self,
        config: &Config,
    ) -> HashMap<String, codex_config_types::McpServerConfig> {
        self.services.mcp_manager.configured_servers(config).await
    }

    pub(crate) async fn mcp_oauth_login_support(
        &self,
        transport: &codex_config_types::McpServerTransportConfig,
    ) -> codex_mcp_types::McpOAuthLoginSupport {
        self.services
            .mcp_auth_runtime
            .oauth_login_support(transport)
            .await
    }

    pub(crate) async fn perform_mcp_oauth_login(
        &self,
        request: codex_mcp_runtime_api::McpOAuthLoginRequest,
    ) -> anyhow::Result<()> {
        self.services
            .mcp_auth_runtime
            .perform_oauth_login(request)
            .await
    }

    pub(crate) fn should_retry_mcp_oauth_without_scopes(
        &self,
        scopes: &codex_mcp_types::ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool {
        self.services
            .mcp_auth_runtime
            .should_retry_without_scopes(scopes, error)
    }

    pub(crate) async fn configured_plugin_installed(&self, tool_id: &str) -> bool {
        let config = self.get_config().await;
        let plugins_input = config.plugins_config_input();
        self.plugins_manager()
            .is_configured_plugin_installed(&plugins_input, tool_id)
    }

    pub(crate) async fn list_spawn_agent_models(
        &self,
    ) -> Vec<codex_protocol::openai_models::ModelPreset> {
        self.services
            .models_manager
            .list_models(RefreshStrategy::Offline)
            .await
    }

    pub(crate) async fn spawn_agent_model_info(&self, model: &str, config: &Config) -> ModelInfo {
        self.services
            .models_manager
            .get_model_info(model, &config.to_models_manager_config())
            .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "tool dispatch must keep active-turn accounting atomic"
    )]
    pub(crate) async fn record_tool_call_started(&self) {
        let mut active = self.active_turn.lock().await;
        if let Some(active_turn) = active.as_mut() {
            let mut turn_state = active_turn.turn_state.lock().await;
            turn_state.tool_calls = turn_state.tool_calls.saturating_add(1);
        }
    }

    pub(crate) async fn run_pre_tool_use_hooks_for_turn(
        &self,
        turn: &TurnContext,
        call_id: String,
        tool_name: &str,
        matcher_aliases: Vec<String>,
        tool_input: &serde_json::Value,
    ) -> PreToolUseHookResult {
        run_pre_tool_use_hooks(self, turn, call_id, tool_name, matcher_aliases, tool_input).await
    }

    pub(crate) async fn run_post_tool_use_hooks_for_turn(
        &self,
        turn: &TurnContext,
        payload: PostToolUsePayload,
    ) -> codex_hooks::PostToolUseOutcome {
        let outcome = run_post_tool_use_hooks(
            self,
            turn,
            payload.tool_use_id,
            payload.tool_name.name().to_string(),
            payload.tool_name.matcher_aliases().to_vec(),
            payload.tool_input,
            payload.tool_response,
        )
        .await;

        record_additional_contexts(self, turn, outcome.additional_contexts.clone()).await;
        outcome
    }

    pub(crate) async fn account_goal_tool_completed(
        &self,
        turn: &TurnContext,
        tool_name: &ToolName,
    ) -> Result<(), String> {
        if tool_name.name == UPDATE_GOAL_TOOL_NAME {
            return Ok(());
        }

        self.account_thread_goal_progress(
            turn,
            BudgetLimitSteering::Allowed,
            TerminalMetricEmission::Emit,
        )
        .await
        .map_err(|err| err.to_string())
    }

    pub async fn begin_command_wait(
        &self,
        request: CommandWaitRequest,
    ) -> Result<Box<dyn CommandWaitOperation>, CommandSessionError> {
        self.services
            .command_service_state
            .begin_command_wait(request)
            .await
    }

    pub async fn write_command_stdin(
        &self,
        request: WriteStdinRequest<'_>,
    ) -> Result<WriteStdinOutput, CommandSessionError> {
        self.services
            .command_service_state
            .write_command_stdin(request)
            .await
    }

    pub async fn allocate_unified_exec_process_id(&self) -> i32 {
        self.services
            .command_service_state
            .allocate_process_id()
            .await
    }

    pub async fn release_unified_exec_process_id(&self, process_id: i32) {
        self.services
            .command_service_state
            .release_process_id(process_id)
            .await;
    }

    pub async fn run_unified_exec_command(
        self: &Arc<Self>,
        turn: Arc<TurnContext>,
        call_id: String,
        request: ExecCommandRunRequest,
    ) -> Result<ExecCommandRunOutput, codex_command_service_api::UnifiedExecError> {
        self.services
            .command_service_state
            .run_exec_command(
                Arc::clone(self) as Arc<dyn codex_command_service_api::CommandServiceSessionCapability>,
                turn as Arc<dyn codex_command_service_api::CommandServiceTurnCapability>,
                call_id,
                request,
            )
            .await
    }

    pub(crate) async fn thread_config_snapshot(&self) -> ThreadConfigSnapshot {
        let state = self.state.lock().await;
        state.session_configuration.thread_config_snapshot()
    }

    pub(crate) async fn terminal_type(&self) -> String {
        let state = self.state.lock().await;
        state.session_configuration.terminal_type.clone()
    }

    pub(crate) async fn app_server_client_metadata(&self) -> AppServerClientMetadata {
        let state = self.state.lock().await;
        AppServerClientMetadata {
            client_name: state.session_configuration.app_server_client_name.clone(),
            client_version: state
                .session_configuration
                .app_server_client_version
                .clone(),
        }
    }

    pub(crate) async fn configured_multi_agent_v2_usage_hint_texts(&self) -> Vec<String> {
        let state = self.state.lock().await;
        let config = &state.session_configuration.original_config_do_not_use;
        [
            config.multi_agent_v2.root_agent_usage_hint_text.clone(),
            config.multi_agent_v2.subagent_usage_hint_text.clone(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn managed_network_proxy_active_for_permission_profile(
        permission_profile: &PermissionProfile,
    ) -> bool {
        !matches!(permission_profile, PermissionProfile::Disabled)
    }

    /// Builds the `x-codex-beta-features` header value for this session.
    ///
    /// `ModelClient` is session-scoped and intentionally does not depend on the full `Config`, so
    /// we precompute the comma-separated list of enabled experimental feature keys at session
    /// creation time and thread it into the client.
    fn build_model_client_beta_features_header(config: &Config) -> Option<String> {
        let beta_features_header = FEATURES
            .iter()
            .filter_map(|spec| {
                let advertise_in_model_client_header =
                    spec.stage.experimental_menu_description().is_some();
                if advertise_in_model_client_header && config.features.enabled(spec.id) {
                    Some(spec.key)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(",");

        if beta_features_header.is_empty() {
            None
        } else {
            Some(beta_features_header)
        }
    }

    async fn start_managed_network_proxy(
        spec: &codex_config::NetworkProxySpec,
        factory: &dyn NetworkProxyRuntimeFactory,
        exec_policy: &codex_execpolicy_api::Policy,
        permission_profile: &PermissionProfile,
        network_policy_decider: Option<Arc<dyn NetworkPolicyDecider>>,
        blocked_request_observer: Option<Arc<dyn BlockedRequestObserver>>,
        managed_network_requirements_enabled: bool,
        audit_metadata: NetworkProxyAuditMetadata,
    ) -> anyhow::Result<(StartedNetworkProxy, SessionNetworkProxyRuntime)> {
        let spec = spec
            .with_exec_policy_network_rules(exec_policy)
            .map_err(|err| {
                tracing::warn!(
                    "failed to apply execpolicy network rules to managed proxy; continuing with configured network policy: {err}"
                );
                err
            })
            .unwrap_or_else(|_| spec.clone());
        let network_proxy = spec
            .start_proxy(
                factory,
                permission_profile,
                network_policy_decider,
                blocked_request_observer,
                managed_network_requirements_enabled,
                audit_metadata,
            )
            .await
            .map_err(|err| anyhow::anyhow!("failed to start managed network proxy: {err}"))?;
        let session_network_proxy = {
            let proxy = network_proxy.proxy();
            SessionNetworkProxyRuntime {
                http_addr: proxy.http_addr().to_string(),
                socks_addr: proxy.socks_addr().to_string(),
            }
        };
        Ok((network_proxy, session_network_proxy))
    }

    async fn refresh_managed_network_proxy_for_current_permission_profile(&self) {
        let Some(started_proxy) = self.services.network_proxy.as_ref() else {
            return;
        };
        let Ok(_refresh_guard) = self.managed_network_proxy_refresh_lock.acquire().await else {
            error!("managed network proxy refresh semaphore closed");
            return;
        };
        let session_configuration = {
            let state = self.state.lock().await;
            state.session_configuration.clone()
        };
        let Some(spec) = session_configuration
            .original_config_do_not_use
            .permissions
            .network
            .as_ref()
        else {
            return;
        };

        let spec = match spec
            .recompute_for_permission_profile(&session_configuration.permission_profile())
        {
            Ok(spec) => spec,
            Err(err) => {
                warn!("failed to rebuild managed network proxy policy for sandbox change: {err}");
                return;
            }
        };
        let current_exec_policy = self.services.exec_policy.current();
        let spec = match spec.with_exec_policy_network_rules(current_exec_policy.as_ref()) {
            Ok(spec) => spec,
            Err(err) => {
                warn!(
                    "failed to apply execpolicy network rules while refreshing managed network proxy: {err}"
                );
                spec
            }
        };
        if let Err(err) = spec.apply_to_started_proxy(started_proxy).await {
            warn!("failed to refresh managed network proxy for sandbox change: {err}");
        }
    }

    #[cfg(test)]
    pub(crate) async fn codex_home(&self) -> AbsolutePathBuf {
        let state = self.state.lock().await;
        state.session_configuration.codex_home().clone()
    }

    pub(crate) fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool> {
        self.out_of_band_elicitation_paused.subscribe()
    }

    pub(crate) fn set_out_of_band_elicitation_pause_state(&self, paused: bool) {
        self.out_of_band_elicitation_paused.send_replace(paused);
    }

    pub(crate) fn get_tx_event(&self) -> Sender<Event> {
        self.tx_event.clone()
    }

    pub(crate) fn state_db(&self) -> Option<state_db::StateDbHandle> {
        self.services.state_db.clone()
    }

    pub(crate) fn live_thread_for_persistence(
        &self,
        operation: &str,
    ) -> anyhow::Result<&dyn LiveThreadHandle> {
        self.live_thread()
            .ok_or_else(|| anyhow::anyhow!("Session persistence is disabled; cannot {operation}."))
    }

    pub(crate) fn live_thread(&self) -> Option<&dyn LiveThreadHandle> {
        self.services.live_thread.as_deref()
    }

    /// Flush rollout writes and return the final durability-barrier result.
    pub(crate) async fn flush_rollout(&self) -> std::io::Result<()> {
        if let Some(live_thread) = self.live_thread() {
            live_thread.flush().await.map_err(std::io::Error::other)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn try_ensure_rollout_materialized(&self) -> std::io::Result<()> {
        if let Some(live_thread) = self.live_thread() {
            live_thread.persist().await.map_err(std::io::Error::other)?;
        }
        Ok(())
    }

    pub(crate) async fn ensure_rollout_materialized(&self) {
        if let Err(e) = self.try_ensure_rollout_materialized().await {
            warn!("failed to materialize thread persistence: {e}");
        }
    }

    fn next_internal_sub_id(&self) -> String {
        let id = self
            .next_internal_sub_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        format!("auto-compact-{id}")
    }

    pub(crate) async fn route_realtime_text_input(self: &Arc<Self>, text: String) {
        handlers::user_input_or_turn_inner(
            self,
            self.next_internal_sub_id(),
            Op::UserInput {
                environments: None,
                items: vec![UserInput::Text {
                    text,
                    text_elements: Vec::new(),
                }],
                final_output_json_schema: None,
                responsesapi_client_metadata: None,
            },
            /*mirror_user_text_to_realtime*/ None,
        )
        .await;
    }

    pub(crate) async fn get_total_token_usage(&self) -> i64 {
        let state = self.state.lock().await;
        state.get_total_token_usage(state.server_reasoning_included())
    }

    pub(crate) async fn get_total_token_usage_breakdown(&self) -> TotalTokenUsageBreakdown {
        let state = self.state.lock().await;
        state.history.get_total_token_usage_breakdown()
    }

    pub(crate) async fn total_token_usage(&self) -> Option<TokenUsage> {
        let state = self.state.lock().await;
        state.token_info().map(|info| info.total_token_usage)
    }

    /// Returns the complete token usage snapshot currently cached for this session.
    ///
    /// Resume and fork reconstruction seed this state from the last persisted rollout
    /// `TokenCount` event. Callers that need to replay restored usage to a client
    /// should use this accessor instead of `total_token_usage`, because the app-server
    /// notification includes both total and last-turn usage.
    pub(crate) async fn token_usage_info(&self) -> Option<TokenUsageInfo> {
        let state = self.state.lock().await;
        state.token_info()
    }

    /// Recomputes the context usage snapshot from the current reconstructed history.
    ///
    /// Restored sessions may predate persisted `ThreadContextUsageUpdated` rollout
    /// events. Recomputing from live history lets app-server restore the context usage
    /// panel as soon as a client attaches, matching the token usage restore path.
    pub(crate) async fn thread_context_usage(&self) -> ThreadContextUsage {
        let state = self.state.lock().await;
        build_thread_context_usage_from_history(&state.history, &state.thread_skills())
    }

    pub(crate) async fn get_estimated_token_count(
        &self,
        turn_context: &TurnContext,
    ) -> Option<i64> {
        let state = self.state.lock().await;
        estimate_history_token_count(&state.history, turn_context)
    }

    pub(crate) async fn get_base_instructions(&self) -> BaseInstructions {
        let state = self.state.lock().await;
        BaseInstructions {
            text: state.session_configuration.base_instructions.clone(),
        }
    }

    // Merges connector IDs into the session-level explicit connector selection.
    pub(crate) async fn merge_connector_selection(
        &self,
        connector_ids: HashSet<String>,
    ) -> HashSet<String> {
        let mut state = self.state.lock().await;
        state.merge_connector_selection(connector_ids)
    }

    // Returns the connector IDs currently selected for this session.
    pub(crate) async fn get_connector_selection(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state.get_connector_selection()
    }

    // Clears connector IDs that were accumulated for explicit selection.
    pub(crate) async fn clear_connector_selection(&self) {
        let mut state = self.state.lock().await;
        state.clear_connector_selection();
    }

    async fn record_initial_history(&self, conversation_history: InitialHistory) {
        let turn_context = self.new_default_turn().await;
        let is_subagent = {
            let state = self.state.lock().await;
            state
                .session_configuration
                .session_source
                .is_non_root_agent()
        };
        let has_prior_user_turns = initial_history_has_prior_user_turns(&conversation_history);
        {
            let mut state = self.state.lock().await;
            state.set_next_turn_is_first(!has_prior_user_turns);
        }
        match conversation_history {
            InitialHistory::New | InitialHistory::Cleared => {
                self.set_previous_turn_settings(/*previous_turn_settings*/ None)
                    .await;
                self.record_context_updates_and_set_reference_context_item(&turn_context)
                    .await;
                self.ensure_rollout_materialized().await;
                let _ = self.flush_rollout().await;
            }
            InitialHistory::Resumed(resumed_history) => {
                let rollout_items = resumed_history.history;
                let previous_turn_settings = self
                    .apply_rollout_reconstruction(&turn_context, &rollout_items)
                    .await;

                // If resuming, warn when the last recorded model differs from the current one.
                let curr: &str = turn_context.model_info.slug.as_str();
                if let Some(prev) = previous_turn_settings
                    .as_ref()
                    .map(|settings| settings.model.as_str())
                    .filter(|model| *model != curr)
                {
                    warn!("resuming session with different model: previous={prev}, current={curr}");
                    self.send_event(
                        &turn_context,
                        EventMsg::Warning(WarningEvent {
                            message: format!(
                                "This session was recorded with model `{prev}` but is resuming with `{curr}`. \
                         Consider switching back to `{prev}` as it may affect Codex performance."
                            ),
                        }),
                    )
                    .await;
                }

                // Seed usage info from the recorded rollout so UIs can show token counts
                // immediately on resume/fork.
                if let Some(info) = Self::last_token_info_from_rollout(&rollout_items) {
                    let mut state = self.state.lock().await;
                    state.set_token_info(Some(info));
                }

                // Defer seeding the session's initial context until the first turn starts so
                // turn/start overrides can be merged before we write to the rollout.
                if !is_subagent {
                    let _ = self.flush_rollout().await;
                }
            }
            InitialHistory::Forked(rollout_items) => {
                self.apply_rollout_reconstruction(&turn_context, &rollout_items)
                    .await;

                // Seed usage info from the recorded rollout so UIs can show token counts
                // immediately on resume/fork.
                if let Some(info) = Self::last_token_info_from_rollout(&rollout_items) {
                    let mut state = self.state.lock().await;
                    state.set_token_info(Some(info));
                }

                // If persisting, persist all rollout items as-is (the store filters).
                if !rollout_items.is_empty() {
                    self.persist_rollout_items(&rollout_items).await;
                }

                // Forked threads should remain file-backed immediately after startup.
                self.ensure_rollout_materialized().await;

                // Flush after seeding history and any persisted rollout copy.
                if !is_subagent {
                    let _ = self.flush_rollout().await;
                }
            }
        }
    }

    async fn apply_rollout_reconstruction(
        &self,
        turn_context: &TurnContext,
        rollout_items: &[RolloutItem],
    ) -> Option<PreviousTurnSettings> {
        let reconstructed_rollout = self
            .reconstruct_history_from_rollout(turn_context, rollout_items)
            .await;
        let previous_turn_settings = reconstructed_rollout.previous_turn_settings.clone();
        self.replace_history(
            reconstructed_rollout.history,
            reconstructed_rollout.reference_context_item,
        )
        .await;
        self.set_previous_turn_settings(previous_turn_settings.clone())
            .await;
        previous_turn_settings
    }

    fn last_token_info_from_rollout(rollout_items: &[RolloutItem]) -> Option<TokenUsageInfo> {
        rollout_items.iter().rev().find_map(|item| match item {
            RolloutItem::EventMsg(EventMsg::TokenCount(ev)) => ev.info.clone(),
            _ => None,
        })
    }

    async fn previous_turn_settings(&self) -> Option<PreviousTurnSettings> {
        let state = self.state.lock().await;
        state.previous_turn_settings()
    }

    pub(crate) async fn set_previous_turn_settings(
        &self,
        previous_turn_settings: Option<PreviousTurnSettings>,
    ) {
        let mut state = self.state.lock().await;
        state.set_previous_turn_settings(previous_turn_settings);
    }

    fn maybe_refresh_shell_snapshot_for_cwd(
        &self,
        previous_cwd: &AbsolutePathBuf,
        next_cwd: &AbsolutePathBuf,
        codex_home: &AbsolutePathBuf,
        session_source: &SessionSource,
    ) {
        if previous_cwd == next_cwd {
            return;
        }

        if !self.features.enabled(Feature::ShellSnapshot) {
            return;
        }

        if matches!(
            session_source,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
        ) {
            return;
        }

        ShellSnapshot::refresh_snapshot(
            codex_home.clone(),
            self.conversation_id,
            next_cwd.clone(),
            self.services.user_shell.as_ref().clone(),
            self.services.shell_snapshot_tx.clone(),
            self.services.session_telemetry.clone(),
            self.services.state_db.clone(),
        );
    }

    pub(crate) async fn update_settings(
        &self,
        updates: SessionSettingsUpdate,
    ) -> ConstraintResult<()> {
        let notify_config_contributors = !self.services.extensions.config_contributors().is_empty();
        let (
            previous_config,
            new_config,
            previous_cwd,
            permission_profile_changed,
            next_cwd,
            codex_home,
            session_source,
        ) = {
            let mut state = self.state.lock().await;
            let updated = match state.session_configuration.apply(&updates) {
                Ok(updated) => updated,
                Err(err) => {
                    warn!("rejected session settings update: {err}");
                    return Err(err);
                }
            };

            let previous_config = notify_config_contributors
                .then(|| Self::build_effective_session_config(&state.session_configuration));
            let new_config =
                notify_config_contributors.then(|| Self::build_effective_session_config(&updated));
            let previous_cwd = state.session_configuration.cwd.clone();
            let previous_permission_profile = state.session_configuration.permission_profile();
            let updated_permission_profile = updated.permission_profile();
            let permission_profile_changed =
                previous_permission_profile != updated_permission_profile;
            let next_cwd = updated.cwd.clone();
            let codex_home = updated.codex_home.clone();
            let session_source = updated.session_source.clone();
            state.session_configuration = updated;
            (
                previous_config,
                new_config,
                previous_cwd,
                permission_profile_changed,
                next_cwd,
                codex_home,
                session_source,
            )
        };

        self.emit_config_changed_contributors(previous_config.as_ref(), new_config.as_ref());
        self.maybe_refresh_shell_snapshot_for_cwd(
            &previous_cwd,
            &next_cwd,
            &codex_home,
            &session_source,
        );
        if permission_profile_changed {
            self.refresh_managed_network_proxy_for_current_permission_profile()
                .await;
        }

        Ok(())
    }

    pub(crate) async fn validate_settings(
        &self,
        updates: &SessionSettingsUpdate,
    ) -> ConstraintResult<()> {
        let state = self.state.lock().await;
        state.session_configuration.apply(updates).map(|_| ())
    }

    pub(crate) async fn set_session_startup_prewarm(
        &self,
        startup_prewarm: SessionStartupPrewarmHandle,
    ) {
        let mut state = self.state.lock().await;
        state.set_session_startup_prewarm(startup_prewarm);
    }

    pub(crate) async fn take_session_startup_prewarm(&self) -> Option<SessionStartupPrewarmHandle> {
        let mut state = self.state.lock().await;
        state.take_session_startup_prewarm()
    }

    pub(crate) async fn get_config(&self) -> std::sync::Arc<Config> {
        let state = self.state.lock().await;
        state
            .session_configuration
            .original_config_do_not_use
            .clone()
    }

    pub(crate) async fn provider(&self) -> ModelProviderInfo {
        let state = self.state.lock().await;
        state.session_configuration.provider.clone()
    }

    pub(crate) async fn refresh_runtime_config(&self, next_config: Config) {
        // Refresh only the user layer from the incoming snapshot. Preserve thread-local
        // layers such as request/session overrides that were present when this session
        // was created.
        let notify_config_contributors = !self.services.extensions.config_contributors().is_empty();
        let (previous_config, new_config, config) = {
            let mut state = self.state.lock().await;
            let previous_config = notify_config_contributors
                .then(|| Self::build_effective_session_config(&state.session_configuration));
            let mut config = (*state.session_configuration.original_config_do_not_use).clone();
            config.config_layer_stack = config
                .config_layer_stack
                .with_user_layer_from(&next_config.config_layer_stack);
            config.tool_suggest =
                resolve_tool_suggest_config_from_layer_stack(&config.config_layer_stack);
            let config = Arc::new(config);
            state.session_configuration.original_config_do_not_use = Arc::clone(&config);
            let new_config = notify_config_contributors
                .then(|| Self::build_effective_session_config(&state.session_configuration));
            (previous_config, new_config, config)
        };
        self.emit_config_changed_contributors(previous_config.as_ref(), new_config.as_ref());
        self.services.skills_manager.clear_cache();
        self.services.plugins_manager.clear_cache();
        let hooks = build_hooks_for_config(
            config.as_ref(),
            self.services.plugins_manager.as_ref(),
            self.services.user_shell.as_ref(),
            self.services.hook_runtime_factory.as_ref(),
        )
        .await;

        let state = self.state.lock().await;
        // A newer refresh may have updated the config while this hook build was in flight.
        // Only publish hooks derived from the current config snapshot.
        if Arc::ptr_eq(
            &state.session_configuration.original_config_do_not_use,
            &config,
        ) {
            *self
                .services
                .hooks
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = hooks;
        }
    }

    fn emit_config_changed_contributors(
        &self,
        previous_config: Option<&Config>,
        new_config: Option<&Config>,
    ) {
        let (Some(previous_config), Some(new_config)) = (previous_config, new_config) else {
            return;
        };
        if previous_config == new_config {
            return;
        }
        for contributor in self.services.extensions.config_contributors() {
            contributor.on_config_changed(
                &self.services.session_extension_data,
                &self.services.thread_extension_data,
                previous_config,
                new_config,
            );
        }
    }

    pub(crate) async fn reload_user_config_layer(&self) {
        // Refresh layer-backed runtime state for an existing session, including enabled plugin,
        // skill, and hook state. Derived config fields such as feature gates and legacy notify
        // settings remain session-static.
        //
        // Prefer `refresh_runtime_config()` when the host can already provide a materialized
        // config snapshot. This file-based path exists for legacy local reload flows.
        let config_toml_paths = {
            let state = self.state.lock().await;
            let config = &state.session_configuration.original_config_do_not_use;
            let user_config_paths = config
                .config_layer_stack
                .get_user_layers(
                    ConfigLayerStackOrdering::LowestPrecedenceFirst,
                    /*include_disabled*/ true,
                )
                .into_iter()
                .filter_map(|layer| match &layer.name {
                    ConfigLayerSource::User { file, .. } => Some(file.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if user_config_paths.is_empty() {
                vec![
                    state
                        .session_configuration
                        .codex_home
                        .join(CONFIG_TOML_FILE),
                ]
            } else {
                user_config_paths
            }
        };

        let mut reloaded_user_configs = Vec::with_capacity(config_toml_paths.len());
        for config_toml_path in config_toml_paths {
            let user_config = match std::fs::read_to_string(&config_toml_path) {
                Ok(contents) => match toml::from_str::<toml::Value>(&contents) {
                    Ok(config) => config,
                    Err(err) => {
                        warn!("failed to parse user config while reloading layer: {err}");
                        return;
                    }
                },
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    toml::Value::Table(Default::default())
                }
                Err(err) => {
                    warn!("failed to read user config while reloading layer: {err}");
                    return;
                }
            };
            reloaded_user_configs.push((config_toml_path, user_config));
        }

        let next_config = {
            let state = self.state.lock().await;
            let mut config = (*state.session_configuration.original_config_do_not_use).clone();
            for (config_toml_path, user_config) in reloaded_user_configs {
                config.config_layer_stack = config
                    .config_layer_stack
                    .with_user_config(&config_toml_path, user_config);
            }
            config.tool_suggest =
                resolve_tool_suggest_config_from_layer_stack(&config.config_layer_stack);
            config
        };
        self.refresh_runtime_config(next_config).await;
    }

    async fn build_settings_update_items(
        &self,
        reference_context_item: Option<&TurnContextItem>,
        current_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        // TODO: Make context updates a pure diff of persisted previous/current TurnContextItem
        // state so replay/backtracking is deterministic. Runtime inputs that affect model-visible
        // context (shell, exec policy, feature gates, previous-turn bridge) should be persisted
        // state or explicit non-state replay events.
        let previous_turn_settings = {
            let state = self.state.lock().await;
            state.previous_turn_settings()
        };
        let shell = self.user_shell();
        let exec_policy = self.services.exec_policy.current();
        let environment_context =
            crate::context::environment_context_from_turn_context(current_context, shell.as_ref());
        codex_context_manager::build_settings_update_items(SettingsUpdateInput {
            previous: reference_context_item,
            previous_turn_settings: previous_turn_settings_view(previous_turn_settings.as_ref()),
            include_environment_context: current_context.config.include_environment_context,
            environment_context: Some(&environment_context),
            shell_name: shell.name(),
            include_permissions_instructions: current_context
                .config
                .include_permissions_instructions,
            permission_profile: &current_context.permission_profile,
            approval_policy: current_context.approval_policy.value(),
            approvals_reviewer: current_context.config.approvals_reviewer,
            exec_policy: exec_policy.as_ref(),
            #[allow(deprecated)]
            cwd: &current_context.cwd,
            exec_permission_approvals_enabled: current_context
                .features
                .enabled(Feature::ExecPermissionApprovals),
            request_permissions_tool_enabled: current_context
                .features
                .enabled(Feature::RequestPermissionsTool),
            include_collaboration_mode_instructions: current_context
                .config
                .include_collaboration_mode_instructions,
            collaboration_mode: &current_context.collaboration_mode,
            realtime_active: current_context.realtime_active,
            experimental_realtime_start_instructions: current_context
                .config
                .experimental_realtime_start_instructions
                .as_deref(),
            personality_feature_enabled: self.features.enabled(Feature::Personality),
            model_info: &current_context.model_info,
            personality: current_context.personality,
        })
    }

    /// Persist the event to rollout and send it to clients.
    pub(crate) async fn send_event(&self, turn_context: &TurnContext, msg: EventMsg) {
        let legacy_source = msg.clone();
        self.services
            .rollout_thread_trace
            .record_codex_turn_event(&turn_context.sub_id, &legacy_source);
        self.services
            .rollout_thread_trace
            .record_tool_call_event(turn_context.sub_id.clone(), &legacy_source);
        let event = Event {
            id: turn_context.sub_id.clone(),
            msg,
        };
        self.send_event_raw(event).await;
        Box::pin(self.maybe_notify_parent_of_final_status(turn_context)).await;
        self.maybe_mirror_event_text_to_realtime(&legacy_source)
            .await;
        self.maybe_clear_realtime_handoff_for_event(&legacy_source)
            .await;

        let show_raw_agent_reasoning = self.show_raw_agent_reasoning();
        for legacy in legacy_source.as_legacy_events(show_raw_agent_reasoning) {
            let legacy_event = Event {
                id: turn_context.sub_id.clone(),
                msg: legacy,
            };
            self.send_event_raw(legacy_event).await;
        }
    }

    /// Forwards finished spawned MultiAgentV2 children to their direct parent once inactive.
    pub(crate) async fn maybe_notify_parent_of_final_status(&self, turn_context: &TurnContext) {
        self.maybe_notify_parent_of_final_status_for_source(
            turn_context.sub_id.as_str(),
            &turn_context.session_source,
        )
        .await;
    }

    pub(crate) async fn maybe_notify_parent_of_final_status_for_current_source(&self) {
        let session_source = {
            let state = self.state.lock().await;
            state.session_configuration.session_source.clone()
        };
        let sub_id = self.next_internal_sub_id();
        self.maybe_notify_parent_of_final_status_for_source(&sub_id, &session_source)
            .await;
    }

    async fn maybe_notify_parent_of_final_status_for_source(
        &self,
        sub_id: &str,
        session_source: &SessionSource,
    ) {
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id,
            agent_path: Some(child_agent_path),
            ..
        }) = session_source
        else {
            return;
        };

        let status = self.agent_status.borrow().clone();
        if !is_final(&status) {
            return;
        }
        if self
            .services
            .agent_control
            .get_agent_metadata(self.conversation_id)
            .is_some_and(|metadata| metadata.agent_mode == AgentMode::Management)
        {
            return;
        }
        match Box::pin(self.thread_post_turn_state()).await {
            ThreadPostTurnState::ThreadCompletion => {}
            ThreadPostTurnState::ThreadActive
            | ThreadPostTurnState::ThreadIdle(_)
            | ThreadPostTurnState::GoContextContinuation { .. } => {
                self.child_completion.mark_delivery_active();
                return;
            }
        }

        if !self.child_completion.try_begin_delivery() {
            return;
        }

        if !Box::pin(self.forward_child_completion_to_parent(
            sub_id,
            *parent_thread_id,
            child_agent_path,
            status,
        ))
        .await
        {
            self.child_completion.mark_delivery_active();
        }
    }

    /// Sends the standard completion envelope from a spawned MultiAgentV2 child to its parent.
    async fn forward_child_completion_to_parent(
        &self,
        turn_id: &str,
        parent_thread_id: ThreadId,
        child_agent_path: &codex_protocol::AgentPath,
        status: AgentStatus,
    ) -> bool {
        let Some(parent_agent_path) = child_agent_path
            .as_str()
            .rsplit_once('/')
            .and_then(|(parent, _)| codex_protocol::AgentPath::try_from(parent).ok())
        else {
            return false;
        };

        let message = format_subagent_notification_message(child_agent_path.as_str(), &status);
        // `communication` owns the message. Keep a second copy only when the
        // recorder will actually need it after parent delivery succeeds.
        let trace_message = self
            .services
            .rollout_thread_trace
            .is_enabled()
            .then(|| message.clone());
        let communication = InterAgentCommunication::new(
            child_agent_path.clone(),
            parent_agent_path,
            Vec::new(),
            message,
            codex_protocol::protocol::InterAgentOperation::ChildCompletion,
        )
        .with_trigger_turn(true)
        .with_thread_ids(self.conversation_id, parent_thread_id)
        .with_status(status.clone());
        if let Err(err) = self
            .services
            .agent_control
            .send_inter_agent_communication(parent_thread_id, communication)
            .await
        {
            debug!("failed to notify parent thread {parent_thread_id}: {err}");
            return false;
        }
        if let Some(message) = trace_message {
            self.services
                .rollout_thread_trace
                .record_agent_result_interaction(
                    turn_id,
                    parent_thread_id,
                    &AgentResultTracePayload {
                        child_agent_path: child_agent_path.as_str(),
                        message: &message,
                        status: &status,
                    },
                );
        }
        true
    }

    pub(crate) async fn has_active_child_completion_work(&self) -> bool {
        if self.has_pending_direct_child_completions().await
            || self.has_queued_response_items_for_next_turn().await
            || self.has_pending_mailbox_items().await
            || self
                .services
                .command_service_state
                .has_running_process_for_thread(self.conversation_id)
                .await
        {
            return true;
        }

        Box::pin(
            self.services
                .agent_control
                .agent_subtree_is_active(self.conversation_id),
        )
        .await
    }

    pub(crate) async fn has_active_post_turn_work(&self) -> bool {
        self.has_pending_turn_input().await
            || Box::pin(self.has_incomplete_direct_child()).await
            || Box::pin(self.has_wait_command()).await
    }

    pub(crate) async fn has_pending_turn_input(&self) -> bool {
        self.has_queued_response_items_for_next_turn().await
            || self.has_pending_mailbox_items().await
    }

    pub(crate) async fn has_incomplete_direct_child(&self) -> bool {
        if self.has_pending_direct_child_completions().await {
            return true;
        }

        Box::pin(
            self.services
                .agent_control
                .direct_agent_children_are_active(self.conversation_id),
        )
        .await
    }

    pub(crate) async fn has_wait_command(&self) -> bool {
        self.services
            .active_event_subscriptions
            .active_count(self.conversation_id)
            > 0
            || self
                .services
                .command_service_state
                .has_running_process_for_thread(self.conversation_id)
                .await
    }

    pub(crate) async fn mark_direct_child_completion_pending(&self, child_thread_id: ThreadId) {
        self.child_completion.mark_pending(child_thread_id).await;
    }

    pub(crate) async fn mark_direct_child_completion_received(
        &self,
        child_thread_id: ThreadId,
    ) -> bool {
        self.child_completion.mark_received(child_thread_id).await
    }

    pub(crate) async fn clear_direct_child_completion_pending(
        &self,
        child_thread_id: ThreadId,
    ) -> bool {
        self.child_completion.clear_pending(child_thread_id).await
    }

    pub(crate) fn mark_child_completion_active(&self) {
        self.child_completion.mark_delivery_active();
    }

    pub(crate) async fn has_pending_direct_child_completions(&self) -> bool {
        self.child_completion.has_pending().await
    }

    pub(crate) async fn mark_direct_child_completions_received_from_pending_input<'a>(
        &self,
        pending_input: impl IntoIterator<Item = &'a PendingInputItem>,
    ) {
        let child_thread_ids = pending_input
            .into_iter()
            .filter_map(|item| match item {
                PendingInputItem::InterAgentCommunication(communication)
                    if matches!(
                        communication.operation,
                        InterAgentOperation::ChildCompletion
                    ) =>
                {
                    communication.sender_thread_id
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if child_thread_ids.is_empty() {
            return;
        }
        self.child_completion
            .mark_received_many(child_thread_ids)
            .await;
    }

    async fn maybe_mirror_event_text_to_realtime(&self, msg: &EventMsg) {
        let Some(text) = realtime_text_for_event(msg) else {
            return;
        };
        if self.conversation.running_state().await.is_none()
            || self.conversation.active_handoff_id().await.is_none()
        {
            return;
        }
        if let Err(err) = self.conversation.handoff_out(text).await {
            debug!("failed to mirror event text to realtime conversation: {err}");
        }
    }

    async fn maybe_clear_realtime_handoff_for_event(&self, msg: &EventMsg) {
        if !matches!(msg, EventMsg::TurnComplete(_)) {
            return;
        }
        if let Err(err) = self.conversation.handoff_complete().await {
            debug!("failed to finalize realtime handoff output: {err}");
        }
        self.conversation.clear_active_handoff().await;
    }

    pub(crate) async fn send_event_raw(&self, event: Event) {
        let status_update = agent_status_from_event(&event.msg);
        // Persist the event into rollout storage (the store filters as needed).
        let rollout_items = vec![RolloutItem::EventMsg(event.msg.clone())];
        self.persist_rollout_items(&rollout_items).await;
        self.services
            .rollout_thread_trace
            .record_protocol_event(&event.msg);
        self.deliver_event_raw(event).await;
        if status_update.as_ref().is_some_and(is_final) {
            Box::pin(self.maybe_notify_parent_of_final_status_for_current_source()).await;
        }
    }

    async fn deliver_event_raw(&self, event: Event) {
        // Record the last known agent status.
        if let Some(status) = agent_status_from_event(&event.msg) {
            self.agent_status.send_replace(status);
        }
        if let Err(e) = self.tx_event.send(event).await {
            debug!("dropping event because channel is closed: {e}");
        }
    }

    pub(crate) async fn emit_turn_item_started(&self, turn_context: &TurnContext, item: &TurnItem) {
        self.send_event(
            turn_context,
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                started_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    pub(crate) async fn emit_turn_item_completed(
        &self,
        turn_context: &TurnContext,
        item: TurnItem,
    ) {
        record_turn_ttfm_metric(turn_context, &item).await;
        self.send_event(
            turn_context,
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item,
                completed_at_ms: now_unix_timestamp_ms(),
            }),
        )
        .await;
    }

    pub(crate) async fn emit_model_item_started_display_event(
        &self,
        turn_context: &TurnContext,
        item: &ResponseItem,
    ) {
        let now = now_unix_timestamp_ms();
        let event = match started_display_event_from_model_item(
            self.conversation_id,
            turn_context.sub_id.clone(),
            item,
            now,
        ) {
            Some(event) => event,
            None => EventMsg::ResponseItemStarted(ResponseItemStartedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                started_at_ms: now,
            }),
        };
        self.send_event(turn_context, event).await;
    }

    /// Adds an execpolicy amendment to both the in-memory and on-disk policies so future
    /// commands can use the newly approved prefix.
    pub(crate) async fn persist_execpolicy_amendment(
        &self,
        amendment: &ExecPolicyAmendment,
    ) -> Result<(), ExecPolicyUpdateError> {
        let codex_home = self
            .state
            .lock()
            .await
            .session_configuration
            .codex_home()
            .clone();

        self.services
            .exec_policy
            .append_amendment_and_update(&codex_home, amendment)
            .await?;

        Ok(())
    }

    pub(crate) async fn turn_context_for_sub_id(&self, sub_id: &str) -> Option<Arc<TurnContext>> {
        let active = self.active_turn.lock().await;
        active
            .as_ref()
            .and_then(|turn| turn.tasks.get(sub_id))
            .map(|task| Arc::clone(&task.turn_context))
    }

    async fn active_turn_context_and_cancellation_token(
        &self,
    ) -> Option<(Arc<TurnContext>, CancellationToken)> {
        let active = self.active_turn.lock().await;
        let (_, task) = active.as_ref()?.tasks.first()?;
        Some((
            Arc::clone(&task.turn_context),
            task.cancellation_token.child_token(),
        ))
    }

    pub(crate) async fn record_execpolicy_amendment_message(
        &self,
        sub_id: &str,
        amendment: &ExecPolicyAmendment,
    ) {
        let Some(prefixes) = format_allow_prefixes(vec![amendment.command.clone()]) else {
            warn!("execpolicy amendment for {sub_id} had no command prefix");
            return;
        };
        let fragment = ApprovedCommandPrefixSaved::new(prefixes);
        let text = fragment.render();
        let message: ResponseItem = ContextualUserFragment::into(fragment);

        if let Some(turn_context) = self.turn_context_for_sub_id(sub_id).await {
            self.record_conversation_items(&turn_context, std::slice::from_ref(&message))
                .await;
            return;
        }

        if self
            .inject_hook_inspectable_items(vec![ResponseInputItem::Message {
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text }],
                phase: None,
            }])
            .await
            .is_err()
        {
            warn!("no active turn found to record execpolicy amendment message for {sub_id}");
        }
    }

    pub(crate) async fn persist_network_policy_amendment(
        &self,
        amendment: &NetworkPolicyAmendment,
        network_approval_context: &NetworkApprovalContext,
    ) -> anyhow::Result<()> {
        let _refresh_guard = self
            .managed_network_proxy_refresh_lock
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("managed network proxy refresh semaphore closed"))?;
        let host = validate_network_policy_amendment_host(amendment, network_approval_context)
            .map_err(anyhow::Error::msg)?;
        let codex_home = self
            .state
            .lock()
            .await
            .session_configuration
            .codex_home()
            .clone();
        let execpolicy_amendment =
            execpolicy_network_rule_amendment(amendment, network_approval_context, &host);

        if let Some(started_network_proxy) = self.services.network_proxy.as_ref() {
            let proxy = started_network_proxy.proxy();
            match amendment.action {
                NetworkPolicyRuleAction::Allow => proxy
                    .add_allowed_domain(&host)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to update runtime allowlist: {err}"))?,
                NetworkPolicyRuleAction::Deny => proxy
                    .add_denied_domain(&host)
                    .await
                    .map_err(|err| anyhow::anyhow!("failed to update runtime denylist: {err}"))?,
            }
        }

        self.services
            .exec_policy
            .append_network_rule_and_update(
                &codex_home,
                &host,
                execpolicy_amendment.protocol,
                execpolicy_amendment.decision,
                Some(execpolicy_amendment.justification),
            )
            .await
            .map_err(|err| {
                anyhow::anyhow!("failed to persist network policy amendment to execpolicy: {err}")
            })?;

        Ok(())
    }

    pub(crate) async fn record_network_policy_amendment_message(
        &self,
        sub_id: &str,
        amendment: &NetworkPolicyAmendment,
    ) {
        let fragment = NetworkRuleSaved::new(amendment);
        let text = fragment.render();
        let message: ResponseItem = ContextualUserFragment::into(fragment);

        if let Some(turn_context) = self.turn_context_for_sub_id(sub_id).await {
            self.record_conversation_items(&turn_context, std::slice::from_ref(&message))
                .await;
            return;
        }

        if self
            .inject_hook_inspectable_items(vec![ResponseInputItem::Message {
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text }],
                phase: None,
            }])
            .await
            .is_err()
        {
            warn!("no active turn found to record network policy amendment message for {sub_id}");
        }
    }

    /// Emit an exec approval request event and await the user's decision.
    ///
    /// The request is keyed by `call_id` + `approval_id` so matching responses
    /// are delivered to the correct in-flight turn. If the pending approval is
    /// cleared before a response arrives, treat it as an abort so interrupted
    /// turns do not continue on a synthetic denial.
    ///
    /// Note that if `available_decisions` is `None`, then the other fields will
    /// be used to derive the available decisions via
    /// [ExecApprovalRequestEvent::default_available_decisions].
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_command_approval(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> ReviewDecision {
        //  command-level approvals use `call_id`.
        // `approval_id` is only present for subcommand callbacks (execve intercept)
        let effective_approval_id = approval_id.clone().unwrap_or_else(|| call_id.clone());
        // Add the tx_approve callback to the map before sending the request.
        let (tx_approve, rx_approve) = oneshot::channel();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_approval(effective_approval_id.clone(), tx_approve)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending approval for call_id: {effective_approval_id}");
        }

        let parsed_cmd = parse_command(&command);
        let proposed_network_policy_amendments = network_approval_context.as_ref().map(|context| {
            vec![
                NetworkPolicyAmendment {
                    host: context.host.clone(),
                    action: NetworkPolicyRuleAction::Allow,
                },
                NetworkPolicyAmendment {
                    host: context.host.clone(),
                    action: NetworkPolicyRuleAction::Deny,
                },
            ]
        });
        let available_decisions = available_decisions.unwrap_or_else(|| {
            ExecApprovalRequestEvent::default_available_decisions(
                network_approval_context.as_ref(),
                proposed_execpolicy_amendment.as_ref(),
                proposed_network_policy_amendments.as_deref(),
                additional_permissions.as_ref(),
            )
        });
        let event = EventMsg::ExecApprovalRequest(ExecApprovalRequestEvent {
            call_id,
            approval_id,
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            command,
            cwd,
            reason,
            network_approval_context,
            proposed_execpolicy_amendment,
            proposed_network_policy_amendments,
            additional_permissions,
            available_decisions: Some(available_decisions),
            parsed_cmd,
        });
        self.send_event(turn_context, event).await;
        rx_approve.await.unwrap_or(ReviewDecision::Abort)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_patch_approval(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        changes: HashMap<PathBuf, FileChange>,
        reason: Option<String>,
        grant_root: Option<PathBuf>,
    ) -> oneshot::Receiver<ReviewDecision> {
        // Add the tx_approve callback to the map before sending the request.
        let (tx_approve, rx_approve) = oneshot::channel();
        let approval_id = call_id.clone();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_approval(approval_id.clone(), tx_approve)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending approval for call_id: {approval_id}");
        }

        let event = EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
            call_id,
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            changes,
            reason,
            grant_root,
        });
        self.send_event(turn_context, event).await;
        rx_approve
    }

    pub async fn request_permissions(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        self.request_permissions_for_cwd(
            turn_context,
            call_id,
            args,
            #[allow(deprecated)]
            turn_context.cwd.clone(),
            cancellation_token,
        )
        .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub(crate) async fn request_permissions_for_cwd(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        call_id: String,
        args: RequestPermissionsArgs,
        cwd: AbsolutePathBuf,
        cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        match turn_context.as_ref().approval_policy.value() {
            AskForApproval::Never => {
                return Some(RequestPermissionsResponse {
                    permissions: RequestPermissionProfile::default(),
                    scope: PermissionGrantScope::Turn,
                    strict_auto_review: false,
                });
            }
            AskForApproval::Granular(granular_config)
                if !granular_config.allows_request_permissions() =>
            {
                return Some(RequestPermissionsResponse {
                    permissions: RequestPermissionProfile::default(),
                    scope: PermissionGrantScope::Turn,
                    strict_auto_review: false,
                });
            }
            AskForApproval::OnFailure
            | AskForApproval::OnRequest
            | AskForApproval::UnlessTrusted
            | AskForApproval::Granular(_) => {}
        }

        let requested_permissions = args.permissions;

        if crate::guardian::routes_approval_to_guardian(turn_context.as_ref()) {
            let originating_turn_state = {
                let active = self.active_turn.lock().await;
                active.as_ref().map(|active| Arc::clone(&active.turn_state))
            };
            let review_id = crate::guardian::new_guardian_review_id();
            let session = Arc::clone(self);
            let turn = Arc::clone(turn_context);
            let request = crate::guardian::GuardianApprovalRequest::RequestPermissions {
                id: call_id,
                turn_id: turn_context.sub_id.clone(),
                reason: args.reason,
                permissions: requested_permissions.clone(),
            };
            let review_rx = crate::guardian::spawn_approval_request_review(
                session,
                turn,
                review_id,
                request,
                /*retry_reason*/ None,
                codex_analytics_api::GuardianApprovalRequestSource::MainTurn,
                cancellation_token.clone(),
            );
            let decision = tokio::select! {
                biased;
                _ = cancellation_token.cancelled() => return None,
                decision = review_rx => decision.unwrap_or(ReviewDecision::Denied),
            };
            let response = match decision {
                ReviewDecision::Approved | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
                    RequestPermissionsResponse {
                        permissions: requested_permissions.clone(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    }
                }
                ReviewDecision::ApprovedForSession => RequestPermissionsResponse {
                    permissions: requested_permissions.clone(),
                    scope: PermissionGrantScope::Session,
                    strict_auto_review: false,
                },
                ReviewDecision::NetworkPolicyAmendment {
                    network_policy_amendment,
                } => match network_policy_amendment.action {
                    NetworkPolicyRuleAction::Allow => RequestPermissionsResponse {
                        permissions: requested_permissions.clone(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    },
                    NetworkPolicyRuleAction::Deny => RequestPermissionsResponse {
                        permissions: RequestPermissionProfile::default(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    },
                },
                ReviewDecision::Abort | ReviewDecision::Denied | ReviewDecision::TimedOut => {
                    RequestPermissionsResponse {
                        permissions: RequestPermissionProfile::default(),
                        scope: PermissionGrantScope::Turn,
                        strict_auto_review: false,
                    }
                }
            };
            let response = normalize_request_permissions_response(
                requested_permissions,
                response,
                cwd.as_path(),
            );
            self.record_granted_request_permissions_for_turn(
                &response,
                originating_turn_state.as_ref(),
            )
            .await;
            return Some(response);
        }

        let (tx_response, rx_response) = oneshot::channel();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_request_permissions(
                        call_id.clone(),
                        PendingRequestPermissions {
                            tx_response,
                            requested_permissions: requested_permissions.clone(),
                            cwd: cwd.clone(),
                        },
                    )
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending request_permissions for call_id: {call_id}");
        }

        let event = EventMsg::RequestPermissions(RequestPermissionsEvent {
            call_id: call_id.clone(),
            turn_id: turn_context.sub_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            reason: args.reason,
            permissions: requested_permissions,
            cwd: Some(cwd),
        });
        self.send_event(turn_context.as_ref(), event).await;
        tokio::select! {
            biased;
            _ = cancellation_token.cancelled() => {
                let mut active = self.active_turn.lock().await;
                if let Some(at) = active.as_mut() {
                    let mut ts = at.turn_state.lock().await;
                    let _ = ts.remove_pending_request_permissions(&call_id);
                }
                None
            }
            response = rx_response => response.ok(),
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn request_user_input(
        &self,
        turn_context: &TurnContext,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        let sub_id = turn_context.sub_id.clone();
        let (tx_response, rx_response) = oneshot::channel();
        let event_id = sub_id.clone();
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_user_input(sub_id, tx_response)
                }
                None => None,
            }
        };
        if prev_entry.is_some() {
            warn!("Overwriting existing pending user input for sub_id: {event_id}");
        }

        let event = EventMsg::RequestUserInput(RequestUserInputEvent {
            call_id,
            turn_id: turn_context.sub_id.clone(),
            questions: args.questions,
        });
        turn_context
            .turn_metadata_state
            .mark_user_input_requested_during_turn();
        self.send_event(turn_context, event).await;
        rx_response.await.ok()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_user_input_response(
        &self,
        sub_id: &str,
        response: RequestUserInputResponse,
    ) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_user_input(sub_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_response) => {
                tx_response.send(response).ok();
            }
            None => {
                warn!("No pending user input found for sub_id: {sub_id}");
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_request_permissions_response(
        &self,
        call_id: &str,
        response: RequestPermissionsResponse,
    ) {
        let (entry, originating_turn_state) = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    let entry = ts.remove_pending_request_permissions(call_id);
                    let originating_turn_state = entry.as_ref().map(|_| Arc::clone(&at.turn_state));
                    (entry, originating_turn_state)
                }
                None => (None, None),
            }
        };
        match entry {
            Some(entry) => {
                let response = normalize_request_permissions_response(
                    entry.requested_permissions,
                    response,
                    entry.cwd.as_path(),
                );
                self.record_granted_request_permissions_for_turn(
                    &response,
                    originating_turn_state.as_ref(),
                )
                .await;
                entry.tx_response.send(response).ok();
            }
            None => {
                warn!("No pending request_permissions found for call_id: {call_id}");
            }
        }
    }

    async fn record_granted_request_permissions_for_turn(
        &self,
        response: &RequestPermissionsResponse,
        originating_turn_state: Option<&Arc<Mutex<crate::state::TurnState>>>,
    ) {
        if response.permissions.is_empty() {
            return;
        }
        match response.scope {
            PermissionGrantScope::Turn => {
                if let Some(turn_state) = originating_turn_state {
                    let mut ts = turn_state.lock().await;
                    let permissions: AdditionalPermissionProfile =
                        response.permissions.clone().into();
                    ts.record_granted_permissions(permissions);
                    if response.strict_auto_review {
                        ts.enable_strict_auto_review();
                    }
                }
            }
            PermissionGrantScope::Session => {
                let mut state = self.state.lock().await;
                state.record_granted_permissions(response.permissions.clone().into());
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn reads must stay consistent with the matching turn state"
    )]
    pub(crate) async fn granted_turn_permissions(&self) -> Option<AdditionalPermissionProfile> {
        let active = self.active_turn.lock().await;
        let active = active.as_ref()?;
        let ts = active.turn_state.lock().await;
        ts.granted_permissions()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn reads must stay consistent with the matching turn state"
    )]
    pub(crate) async fn strict_auto_review_enabled_for_turn(&self) -> bool {
        let active = self.active_turn.lock().await;
        let Some(active) = active.as_ref() else {
            return false;
        };
        let ts = active.turn_state.lock().await;
        ts.strict_auto_review_enabled()
    }

    pub(crate) async fn granted_session_permissions(&self) -> Option<AdditionalPermissionProfile> {
        let state = self.state.lock().await;
        state.granted_permissions()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_dynamic_tool_response(&self, call_id: &str, response: DynamicToolResponse) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_dynamic_tool(call_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_response) => {
                tx_response.send(response).ok();
            }
            None => {
                warn!("No pending dynamic tool call found for call_id: {call_id}");
            }
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and dynamic tool response registration must remain atomic"
    )]
    pub(crate) async fn register_pending_dynamic_tool_response(
        &self,
        call_id: String,
        tx_response: oneshot::Sender<DynamicToolResponse>,
    ) -> bool {
        let prev_entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.insert_pending_dynamic_tool(call_id, tx_response)
                }
                None => None,
            }
        };
        prev_entry.is_some()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn notify_approval(&self, approval_id: &str, decision: ReviewDecision) {
        let entry = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    ts.remove_pending_approval(approval_id)
                }
                None => None,
            }
        };
        match entry {
            Some(tx_approve) => {
                tx_approve.send(decision).ok();
            }
            None => {
                warn!("No pending approval found for call_id: {approval_id}");
            }
        }
    }

    /// Records input items: always append to conversation history and
    /// persist these response items to rollout.
    pub(crate) async fn record_conversation_items(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        self.record_into_history(items, turn_context).await;
        self.persist_rollout_response_items(items).await;
        self.send_thread_context_usage_event(turn_context).await;
    }

    pub(crate) async fn record_model_items_and_emit_display_events(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        let items: Vec<ResponseItem> = items
            .iter()
            .cloned()
            .map(|mut item| {
                if !is_structured_display_response_item(&item) {
                    return item;
                }

                let id = match &mut item {
                    ResponseItem::CommandWait { id, .. }
                    | ResponseItem::CommandWriteStdin { id, .. }
                    | ResponseItem::CommandExecutionNotification { id, .. }
                    | ResponseItem::WorkflowRunProgress { id, .. }
                    | ResponseItem::EventCommandEvent { id, .. }
                    | ResponseItem::EventDrivenTool { id, .. }
                    | ResponseItem::ThreadGoalUpdate { id, .. }
                    | ResponseItem::InterAgentCommunication { id, .. } => id,
                    _ => return item,
                };
                if id.is_none() {
                    *id = Some(format!("response-item-{}", Uuid::new_v4()));
                }
                item
            })
            .collect();
        self.record_conversation_items(turn_context, &items).await;
        self.emit_completed_model_item_display_events(turn_context, &items)
            .await;
    }

    /// Append ResponseItems to the in-memory conversation history only.
    pub(crate) async fn record_into_history(
        &self,
        items: &[ResponseItem],
        turn_context: &TurnContext,
    ) {
        let mut state = self.state.lock().await;
        state.record_items(items.iter(), turn_context.truncation_policy);
    }

    async fn maybe_warn_on_server_model_mismatch(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        server_model: String,
    ) -> bool {
        let requested_model = turn_context.model_info.slug.clone();
        let server_model_normalized = server_model.to_ascii_lowercase();
        let requested_model_normalized = requested_model.to_ascii_lowercase();
        if server_model_normalized == requested_model_normalized {
            info!("server reported model {server_model} (matches requested model)");
            return false;
        }

        warn!("server reported model {server_model} while requested model was {requested_model}");

        let warning_message = format!(
            "Your account was flagged for potentially high-risk cyber activity and this request was routed to gpt-5.2 as a fallback. To regain access to gpt-5.3-codex, apply for trusted access: {CYBER_VERIFY_URL} or learn more: {CYBER_SAFETY_URL}"
        );

        self.send_event(
            turn_context,
            EventMsg::ModelReroute(ModelRerouteEvent {
                from_model: requested_model.clone(),
                to_model: server_model.clone(),
                reason: ModelRerouteReason::HighRiskCyberActivity,
            }),
        )
        .await;

        self.send_event(
            turn_context,
            EventMsg::Warning(WarningEvent {
                message: warning_message.clone(),
            }),
        )
        .await;
        true
    }

    pub(crate) async fn emit_model_verification(
        self: &Arc<Self>,
        turn_context: &Arc<TurnContext>,
        verifications: Vec<ModelVerification>,
    ) {
        self.send_event(
            turn_context,
            EventMsg::ModelVerification(ModelVerificationEvent { verifications }),
        )
        .await;
    }

    pub(crate) async fn replace_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
    ) {
        let mut state = self.state.lock().await;
        state.replace_history(items, reference_context_item);
    }

    pub(crate) async fn replace_compacted_history(
        &self,
        items: Vec<ResponseItem>,
        reference_context_item: Option<TurnContextItem>,
        compacted_item: CompactedItem,
    ) {
        self.replace_history(items, reference_context_item.clone())
            .await;

        self.persist_rollout_items(&[RolloutItem::Compacted(compacted_item)])
            .await;
        if let Some(turn_context_item) = reference_context_item {
            self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item)])
                .await;
        }
        self.services.model_client.advance_window_generation();
    }

    async fn persist_rollout_response_items(&self, items: &[ResponseItem]) {
        let rollout_items: Vec<RolloutItem> = items
            .iter()
            .cloned()
            .map(RolloutItem::ResponseItem)
            .collect();
        self.persist_rollout_items(&rollout_items).await;
    }

    pub fn enabled(&self, feature: Feature) -> bool {
        self.features.enabled(feature)
    }

    pub(crate) fn features(&self) -> ManagedFeatures {
        self.features.clone()
    }

    pub(crate) async fn collaboration_mode(&self) -> CollaborationMode {
        let state = self.state.lock().await;
        state.session_configuration.collaboration_mode.clone()
    }

    async fn emit_completed_model_item_display_events(
        &self,
        turn_context: &TurnContext,
        items: &[ResponseItem],
    ) {
        for item in items {
            if !is_structured_display_response_item(item) {
                continue;
            }

            self.emit_completed_model_item_display_event(turn_context, item)
                .await;
        }
    }

    async fn emit_completed_model_item_display_event(
        &self,
        turn_context: &TurnContext,
        item: &ResponseItem,
    ) {
        let now = now_unix_timestamp_ms();
        let event = match completed_display_event_from_model_item(
            self.conversation_id,
            turn_context.sub_id.clone(),
            item,
            now,
        ) {
            Some(event) => event,
            None => EventMsg::ResponseItemCompleted(ResponseItemCompletedEvent {
                thread_id: self.conversation_id,
                turn_id: turn_context.sub_id.clone(),
                item: item.clone(),
                completed_at_ms: now,
            }),
        };
        self.send_event(turn_context, event).await;
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "MCP app context rendering reads through the session-owned manager guard"
    )]
    pub(crate) async fn build_initial_context(
        &self,
        turn_context: &TurnContext,
    ) -> Vec<ResponseItem> {
        let mut developer_sections = Vec::<String>::with_capacity(8);
        let mut contextual_user_sections = Vec::<String>::with_capacity(2);
        let mut separate_developer_sections = Vec::<String>::new();
        let (
            reference_context_item,
            previous_turn_settings,
            collaboration_mode,
            base_instructions,
            session_source,
        ) = {
            let state = self.state.lock().await;
            (
                state.reference_context_item(),
                state.previous_turn_settings(),
                state.session_configuration.collaboration_mode.clone(),
                state.session_configuration.base_instructions.clone(),
                state.session_configuration.session_source.clone(),
            )
        };
        if let Some(model_switch_message) =
            codex_context_manager::build_model_instructions_update_item(
                previous_turn_settings_view(previous_turn_settings.as_ref()),
                &turn_context.model_info,
                turn_context.personality,
            )
        {
            developer_sections.push(model_switch_message);
        }
        if turn_context.config.include_permissions_instructions {
            developer_sections.push(
                PermissionsInstructions::from_permission_profile(
                    &turn_context.permission_profile,
                    turn_context.approval_policy.value(),
                    turn_context.config.approvals_reviewer,
                    self.services.exec_policy.current().as_ref(),
                    #[allow(deprecated)]
                    &turn_context.cwd,
                    turn_context
                        .features
                        .enabled(Feature::ExecPermissionApprovals),
                    turn_context
                        .features
                        .enabled(Feature::RequestPermissionsTool),
                )
                .render(),
            );
        }
        let separate_guardian_developer_message =
            crate::guardian::is_guardian_reviewer_source(&session_source);
        // Keep the guardian policy prompt out of the aggregated developer bundle so it
        // stays isolated as its own top-level developer message for guardian subagents.
        if !separate_guardian_developer_message
            && let Some(developer_instructions) = turn_context.developer_instructions.as_deref()
            && !developer_instructions.is_empty()
        {
            developer_sections.push(developer_instructions.to_string());
        }
        // Add developer instructions for memories.
        if turn_context.features.enabled(Feature::MemoryTool)
            && turn_context.config.memories.use_memories
            && let Some(memory_prompt) = self
                .services
                .memory_tool_developer_instructions_provider
                .build_memory_tool_developer_instructions(&turn_context.config.codex_home)
                .await
        {
            developer_sections.push(memory_prompt);
        }
        // Add developer instructions from collaboration_mode if they exist and are non-empty
        if turn_context.config.include_collaboration_mode_instructions
            && let Some(collab_instructions) =
                CollaborationModeInstructions::from_collaboration_mode(&collaboration_mode)
        {
            developer_sections.push(collab_instructions.render());
        }
        if let Some(realtime_update) = codex_context_manager::build_initial_realtime_item(
            reference_context_item.as_ref(),
            previous_turn_settings_view(previous_turn_settings.as_ref()),
            turn_context.realtime_active,
            turn_context
                .config
                .experimental_realtime_start_instructions
                .as_deref(),
        ) {
            developer_sections.push(realtime_update);
        }
        if self.features.enabled(Feature::Personality)
            && let Some(personality) = turn_context.personality
        {
            let model_info = turn_context.model_info.clone();
            let has_baked_personality = model_info.supports_personality()
                && base_instructions == model_info.get_model_instructions(Some(personality));
            if !has_baked_personality
                && let Some(personality_message) =
                    codex_context_manager::personality_message_for(&model_info, personality)
            {
                developer_sections
                    .push(PersonalitySpecInstructions::new(personality_message).render());
            }
        }
        if turn_context.config.include_apps_instructions && turn_context.apps_enabled() {
            let mcp_connection_manager = self.services.mcp_connection_manager.read().await;
            let accessible_and_enabled_connectors =
                connectors::list_accessible_and_enabled_connectors_from_manager(
                    mcp_connection_manager.as_ref(),
                    &turn_context.config,
                )
                .await;
            if let Some(apps_instructions) =
                AppsInstructions::from_connectors(&accessible_and_enabled_connectors)
            {
                developer_sections.push(apps_instructions.render());
            }
        }
        if turn_context.config.include_skill_instructions {
            let available_skills = build_available_skills(
                &turn_context.turn_skills.outcome,
                default_skill_metadata_budget(turn_context.model_info.context_window),
                SkillRenderSideEffects::ThreadStart {
                    session_telemetry: self.services.session_telemetry.as_ref(),
                },
            );
            if let Some(available_skills) = available_skills {
                let warning_message = available_skills.warning_message.clone();
                let skills_instructions = AvailableSkillsInstructions::from(available_skills);
                if let Some(warning_message) = warning_message {
                    self.send_event_raw(Event {
                        id: String::new(),
                        msg: EventMsg::Warning(WarningEvent {
                            message: warning_message,
                        }),
                    })
                    .await;
                }
                developer_sections.push(skills_instructions.render());
            }
        }
        let workflow_registry = load_workflow_registry(&turn_context.config);
        if let Some(workflow_instructions) =
            AvailableWorkflowsInstructions::from_registry(&workflow_registry)
        {
            developer_sections.push(workflow_instructions.render());
        }
        if let Some(agent_instructions) =
            AvailableAgentsInstructions::from_agent_roles(&turn_context.config.agent_roles)
        {
            developer_sections.push(agent_instructions.render());
        }
        let loaded_plugins = self
            .services
            .plugins_manager
            .plugins_for_config(&turn_context.config.plugins_config_input())
            .await;
        if let Some(plugin_instructions) =
            AvailablePluginsInstructions::from_plugins(loaded_plugins.capability_summaries())
        {
            developer_sections.push(plugin_instructions.render());
        }
        let context_contributors = self.services.extensions.context_contributors().to_vec();
        for contributor in context_contributors {
            for fragment in contributor
                .contribute(
                    &self.services.session_extension_data,
                    &self.services.thread_extension_data,
                )
                .await
            {
                match fragment.slot() {
                    PromptSlot::DeveloperPolicy | PromptSlot::DeveloperCapabilities => {
                        developer_sections.push(fragment.text().to_string());
                    }
                    PromptSlot::ContextualUser => {
                        contextual_user_sections.push(fragment.text().to_string());
                    }
                    PromptSlot::SeparateDeveloper => {
                        separate_developer_sections.push(fragment.text().to_string());
                    }
                }
            }
        }
        if let Some(user_instructions) = turn_context.user_instructions.as_deref() {
            contextual_user_sections.push(
                UserInstructions {
                    text: user_instructions.to_string(),
                    #[allow(deprecated)]
                    directory: turn_context.cwd.to_string_lossy().into_owned(),
                }
                .render(),
            );
        }
        if turn_context.config.include_environment_context {
            let shell = self.user_shell();
            contextual_user_sections.push(
                crate::context::environment_context_from_turn_context(turn_context, shell.as_ref())
                    .render(),
            );
            contextual_user_sections.push(
                MultiagentContext::new(
                    self.services
                        .agent_control
                        .current_agent_path(self.conversation_id, &session_source),
                    self.services
                        .agent_control
                        .direct_subagent_paths(self.conversation_id)
                        .await,
                )
                .render(),
            );
        }

        let multi_agent_v2_usage_hint_text =
            multi_agents::usage_hint_text(turn_context, &session_source);

        let mut items = Vec::with_capacity(4);
        if let Some(developer_message) =
            codex_context_manager::build_developer_update_item(developer_sections)
        {
            items.push(developer_message);
        }
        for section in separate_developer_sections {
            if let Some(developer_message) =
                codex_context_manager::build_developer_update_item(vec![section])
            {
                items.push(developer_message);
            }
        }
        if let Some(usage_hint_text) = multi_agent_v2_usage_hint_text
            && let Some(usage_hint_message) =
                codex_context_manager::build_developer_update_item(vec![
                    usage_hint_text.to_string(),
                ])
        {
            items.push(usage_hint_message);
        }
        if let Some(contextual_user_message) =
            codex_context_manager::build_contextual_user_message(contextual_user_sections)
        {
            items.push(contextual_user_message);
        }
        // Emit the guardian policy prompt as a separate developer item so the guardian
        // subagent sees a distinct, easy-to-audit instruction block.
        if separate_guardian_developer_message
            && let Some(developer_instructions) = turn_context.developer_instructions.as_deref()
            && !developer_instructions.is_empty()
            && let Some(guardian_developer_message) =
                codex_context_manager::build_developer_update_item(vec![
                    developer_instructions.to_string(),
                ])
        {
            items.push(guardian_developer_message);
        }
        items
    }

    pub(crate) async fn persist_rollout_items(&self, items: &[RolloutItem]) {
        if let Some(live_thread) = self.live_thread()
            && let Err(e) = live_thread.append_items(items).await
        {
            error!("failed to record rollout items: {e:#}");
        }
    }

    pub(crate) async fn clone_history(&self) -> ContextManager {
        let state = self.state.lock().await;
        state.clone_history()
    }

    pub(crate) async fn reference_context_item(&self) -> Option<TurnContextItem> {
        let state = self.state.lock().await;
        state.reference_context_item()
    }

    /// Persist the latest turn context snapshot for the first real user turn and for
    /// steady-state turns that emit model-visible context updates.
    ///
    /// When the reference snapshot is missing, this injects full initial context. Otherwise, it
    /// emits only settings diff items.
    ///
    /// If full context is injected and a model switch occurred, this prepends the
    /// `<model_switch>` developer message so model-specific instructions are not lost.
    ///
    /// This is the normal runtime path that establishes a new `reference_context_item`.
    /// Mid-turn compaction is the other path that can re-establish that baseline when it
    /// reinjects full initial context into replacement history. Other non-regular tasks
    /// intentionally do not update the baseline.
    pub(crate) async fn record_context_updates_and_set_reference_context_item(
        &self,
        turn_context: &TurnContext,
    ) {
        let reference_context_item = {
            let state = self.state.lock().await;
            state.reference_context_item()
        };
        let should_inject_full_context = reference_context_item.is_none();
        let context_items = if should_inject_full_context {
            self.build_initial_context(turn_context).await
        } else {
            // Steady-state path: append only context diffs to minimize token overhead.
            self.build_settings_update_items(reference_context_item.as_ref(), turn_context)
                .await
        };
        let turn_context_item = turn_context.to_turn_context_item();
        if !context_items.is_empty() {
            self.record_conversation_items(turn_context, &context_items)
                .await;
            if should_inject_full_context
                && let Some(item) = injected_context_item_from_response_items(&context_items)
            {
                self.emit_turn_item_completed(turn_context, item).await;
            }
        }
        // Persist one `TurnContextItem` per real user turn so resume/lazy replay can recover the
        // latest durable baseline even when this turn emitted no model-visible context diffs.
        self.persist_rollout_items(&[RolloutItem::TurnContext(turn_context_item.clone())])
            .await;

        // Advance the in-memory diff baseline even when this turn emitted no model-visible
        // context items. This keeps later runtime diffing aligned with the current turn state.
        let mut state = self.state.lock().await;
        state.set_reference_context_item(Some(turn_context_item));
    }

    pub(crate) async fn update_token_usage_info(
        &self,
        turn_context: &TurnContext,
        token_usage: Option<&TokenUsage>,
    ) {
        self.record_token_usage_info(turn_context, token_usage)
            .await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_token_usage_info(
        &self,
        turn_context: &TurnContext,
        token_usage: Option<&TokenUsage>,
    ) {
        if let Some(token_usage) = token_usage {
            let token_info = {
                let mut state = self.state.lock().await;
                state
                    .update_token_info_from_usage(token_usage, turn_context.model_context_window());
                state.token_info()
            };
            if let Some(token_info) = token_info.as_ref() {
                for contributor in self.services.extensions.token_usage_contributors() {
                    contributor.on_token_usage(
                        &self.services.session_extension_data,
                        &self.services.thread_extension_data,
                        turn_context.extension_data.as_ref(),
                        token_info,
                    );
                }
            }
        }
    }

    pub(crate) async fn recompute_token_usage(&self, turn_context: &TurnContext) {
        let history = self.clone_history().await;
        let base_instructions = self.get_base_instructions().await;
        let Some(estimated_total_tokens) =
            history.estimate_token_count_with_base_instructions(&base_instructions)
        else {
            return;
        };
        {
            let mut state = self.state.lock().await;
            let mut info = state.token_info().unwrap_or(TokenUsageInfo {
                total_token_usage: TokenUsage::default(),
                last_token_usage: TokenUsage::default(),
                model_context_window: None,
            });

            info.last_token_usage = TokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                reasoning_output_tokens: 0,
                total_tokens: estimated_total_tokens.max(0),
            };

            if let Some(model_context_window) = turn_context.model_context_window() {
                info.model_context_window = Some(model_context_window);
            }

            state.set_token_info(Some(info));
        }
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn update_rate_limits(
        &self,
        turn_context: &TurnContext,
        new_rate_limits: RateLimitSnapshot,
    ) {
        self.record_rate_limits_info(new_rate_limits).await;
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_rate_limits_info(&self, new_rate_limits: RateLimitSnapshot) {
        {
            let mut state = self.state.lock().await;
            state.set_rate_limits(new_rate_limits);
        }
    }

    pub(crate) async fn mcp_dependency_prompted(&self) -> HashSet<String> {
        let state = self.state.lock().await;
        state.mcp_dependency_prompted()
    }

    pub(crate) async fn record_mcp_dependency_prompted<I>(&self, names: I)
    where
        I: IntoIterator<Item = String>,
    {
        let mut state = self.state.lock().await;
        state.record_mcp_dependency_prompted(names);
    }

    pub async fn dependency_env(&self) -> HashMap<String, String> {
        let state = self.state.lock().await;
        state.dependency_env()
    }

    pub async fn set_dependency_env(&self, values: HashMap<String, String>) {
        let mut state = self.state.lock().await;
        state.set_dependency_env(values);
    }

    pub(crate) async fn set_server_reasoning_included(&self, included: bool) {
        let mut state = self.state.lock().await;
        state.set_server_reasoning_included(included);
    }

    pub(crate) async fn send_token_count_event(&self, turn_context: &TurnContext) {
        let (info, rate_limits) = {
            let state = self.state.lock().await;
            state.token_info_and_rate_limits()
        };
        let event = EventMsg::TokenCount(TokenCountEvent { info, rate_limits });
        self.send_event(turn_context, event).await;
        self.send_thread_context_usage_event(turn_context).await;
    }

    pub(crate) async fn send_thread_context_usage_event(&self, turn_context: &TurnContext) {
        let usage = {
            let state = self.state.lock().await;
            build_thread_context_usage(&state.history, turn_context, &state.thread_skills())
        };
        self.send_event(
            turn_context,
            EventMsg::ThreadContextUsageUpdated(ThreadContextUsageUpdatedEvent { usage }),
        )
        .await;
    }

    pub(crate) async fn set_total_tokens_full(&self, turn_context: &TurnContext) {
        if let Some(context_window) = turn_context.model_context_window() {
            let mut state = self.state.lock().await;
            state.set_token_usage_full(context_window);
        }
        self.send_token_count_event(turn_context).await;
    }

    pub(crate) async fn record_response_item_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        response_item: ResponseItem,
    ) {
        // Add to conversation history and persist response item to rollout.
        self.record_conversation_items(turn_context, std::slice::from_ref(&response_item))
            .await;

        // Derive a turn item and emit lifecycle events if applicable.
        if let Some(item) = parse_turn_item(&response_item) {
            self.emit_turn_item_started(turn_context, &item).await;
            self.emit_turn_item_completed(turn_context, item).await;
        }
    }

    pub(crate) async fn record_user_prompt_and_emit_turn_item(
        &self,
        turn_context: &TurnContext,
        input: &[UserInput],
        response_item: ResponseItem,
    ) {
        // Persist the user message to history, but emit the turn item from `UserInput` so
        // UI-only `text_elements` are preserved. `ResponseItem::Message` does not carry
        // those spans, and `record_response_item_and_emit_turn_item` would drop them.
        self.record_conversation_items(turn_context, std::slice::from_ref(&response_item))
            .await;
        let turn_item = TurnItem::UserMessage(UserMessageItem::new(input));
        self.emit_turn_item_started(turn_context, &turn_item).await;
        self.emit_turn_item_completed(turn_context, turn_item).await;
        self.ensure_rollout_materialized().await;
    }

    pub(crate) async fn notify_stream_error(
        &self,
        turn_context: &TurnContext,
        message: impl Into<String>,
        codex_error: CodexErr,
    ) {
        let additional_details = codex_error.to_string();
        let codex_error_info = CodexErrorInfo::ResponseStreamDisconnected {
            http_status_code: codex_error.http_status_code_value(),
        };
        let event = EventMsg::StreamError(StreamErrorEvent {
            message: message.into(),
            codex_error_info: Some(codex_error_info),
            additional_details: Some(additional_details),
        });
        self.send_event(turn_context, event).await;
    }

    /// Inject additional user input into the currently active turn.
    ///
    /// Returns the active turn id when accepted.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn steer_input(
        &self,
        input: Vec<UserInput>,
        expected_turn_id: Option<&str>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> Result<String, SteerInputError> {
        let mut active = self.active_turn.lock().await;
        let Some(active_turn) = active.as_mut() else {
            return validate_steer_input(input, expected_turn_id, None)
                .map(|validated| validated.active_turn_id);
        };

        let Some((active_turn_id, active_task)) = active_turn.tasks.first() else {
            return validate_steer_input(input, expected_turn_id, None)
                .map(|validated| validated.active_turn_id);
        };

        let active_turn_id = active_turn_id.clone();
        let active_task_kind = active_task.kind;
        let active_turn_context = Arc::clone(&active_task.turn_context);
        let validated = validate_steer_input(
            input,
            expected_turn_id,
            Some(ActiveSteerTurn {
                turn_id: &active_turn_id,
                task_kind: steerable_task_kind(active_task_kind),
            }),
        )?;

        if let Some(responsesapi_client_metadata) = responsesapi_client_metadata {
            active_turn_context
                .turn_metadata_state
                .set_responsesapi_client_metadata(responsesapi_client_metadata);
        }

        let mut turn_state = active_turn.turn_state.lock().await;
        turn_state.push_pending_input(PendingInputItem::from(
            codex_model_input::response_input_item_from_user_input(validated.input),
        ));
        turn_state.accept_mailbox_delivery_for_current_turn();
        Ok(validated.active_turn_id)
    }

    /// Returns the input if there was no task running to inject into.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn inject_hook_inspectable_items(
        &self,
        input: Vec<ResponseInputItem>,
    ) -> Result<(), Vec<ResponseInputItem>> {
        let mut active = self.active_turn.lock().await;
        match active.as_mut() {
            Some(at) => {
                let mut ts = at.turn_state.lock().await;
                for item in input {
                    ts.push_pending_input(PendingInputItem::from(item));
                }
                Ok(())
            }
            None => Err(input),
        }
    }

    pub(crate) async fn defer_mailbox_delivery_to_next_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        let mut turn_state = turn_state.lock().await;
        if turn_state.has_pending_input() {
            return;
        }
        turn_state.set_mailbox_delivery_phase(MailboxDeliveryPhase::NextTurn);
    }

    pub(crate) async fn accept_mailbox_delivery_for_current_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        turn_state
            .lock()
            .await
            .set_mailbox_delivery_phase(MailboxDeliveryPhase::CurrentTurn);
    }

    pub(crate) async fn record_memory_citation_for_turn(&self, sub_id: &str) {
        let turn_state = self.turn_state_for_sub_id(sub_id).await;
        let Some(turn_state) = turn_state else {
            return;
        };
        turn_state.lock().await.has_memory_citation = true;
    }

    async fn turn_state_for_sub_id(
        &self,
        sub_id: &str,
    ) -> Option<Arc<tokio::sync::Mutex<crate::state::TurnState>>> {
        let active = self.active_turn.lock().await;
        active.as_ref().and_then(|active_turn| {
            active_turn
                .tasks
                .contains_key(sub_id)
                .then(|| Arc::clone(&active_turn.turn_state))
        })
    }

    pub(crate) fn subscribe_mailbox_seq(&self) -> watch::Receiver<u64> {
        self.mailbox.subscribe()
    }

    pub(crate) async fn wait_agent_current_window(
        &self,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> Duration {
        let mut guard = self.wait_agent_backoff.lock().await;
        guard
            .entry((sender_thread_id, receiver_thread_id))
            .or_insert_with(|| {
                codex_command_service_api::WaitBackoffState::new(
                    duration_from_config_ms(initial_timeout_ms),
                    duration_from_config_ms(hard_cap_timeout_ms),
                )
            })
            .current_window()
    }

    pub(crate) async fn advance_wait_agent_backoff(
        &self,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) {
        let mut guard = self.wait_agent_backoff.lock().await;
        if let Some(state) = guard.get_mut(&(sender_thread_id, receiver_thread_id)) {
            state.advance_after_timeout();
        }
    }

    pub(crate) async fn reset_wait_agent_backoff(
        &self,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) {
        let mut guard = self.wait_agent_backoff.lock().await;
        if let Some(state) = guard.get_mut(&(sender_thread_id, receiver_thread_id)) {
            state.reset_after_event();
        }
    }

    pub(crate) fn enqueue_mailbox_communication(&self, communication: InterAgentCommunication) {
        self.mailbox.send(PendingInputItem::from(communication));
    }

    pub(crate) fn enqueue_async_input(&self, input: PendingInputItem) {
        self.mailbox.send(input);
    }

    pub(crate) async fn has_trigger_turn_mailbox_items(&self) -> bool {
        self.mailbox_rx.lock().await.has_pending_trigger_turn()
    }

    pub(crate) async fn has_pending_mailbox_items(&self) -> bool {
        self.mailbox_rx.lock().await.has_pending()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state reads must remain atomic"
    )]
    pub(crate) async fn find_pending_input<F, R>(&self, mut f: F) -> Option<R>
    where
        F: FnMut(&PendingInputItem) -> Option<R>,
    {
        let accepts_mailbox_delivery = {
            let active = self.active_turn.lock().await;
            match active.as_ref() {
                Some(at) => {
                    let ts = at.turn_state.lock().await;
                    if let Some(found) = ts.pending_input().iter().find_map(&mut f) {
                        return Some(found);
                    }
                    ts.accepts_mailbox_delivery_for_current_turn()
                }
                None => true,
            }
        };
        if !accepts_mailbox_delivery {
            return None;
        }
        {
            let idle_pending_input = self.idle_pending_input.lock().await;
            if let Some(found) = idle_pending_input.iter().find_map(&mut f) {
                return Some(found);
            }
        }
        let mut mailbox_rx = self.mailbox_rx.lock().await;
        mailbox_rx.pending().find_map(f)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn prepend_pending_input(&self, input: Vec<PendingInputItem>) -> Result<(), ()> {
        let mut active = self.active_turn.lock().await;
        match active.as_mut() {
            Some(at) => {
                let mut ts = at.turn_state.lock().await;
                ts.prepend_pending_input(input);
                Ok(())
            }
            None => Err(()),
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state updates must remain atomic"
    )]
    pub async fn get_pending_input(&self) -> Vec<PendingInputItem> {
        let (pending_input, accepts_mailbox_delivery) = {
            let mut active = self.active_turn.lock().await;
            match active.as_mut() {
                Some(at) => {
                    let mut ts = at.turn_state.lock().await;
                    (
                        ts.take_pending_input(),
                        ts.accepts_mailbox_delivery_for_current_turn(),
                    )
                }
                None => (Vec::new(), true),
            }
        };
        if !accepts_mailbox_delivery {
            return pending_input;
        }
        let mailbox_items = {
            let mut mailbox_rx = self.mailbox_rx.lock().await;
            mailbox_rx.drain()
        };
        if pending_input.is_empty() {
            mailbox_items
        } else if mailbox_items.is_empty() {
            pending_input
        } else {
            let mut pending_input = pending_input;
            pending_input.extend(mailbox_items);
            pending_input
        }
    }

    /// Queue response items to be injected into the next active turn created for this session.
    pub(crate) async fn queue_response_items_for_next_turn(&self, items: Vec<PendingInputItem>) {
        if items.is_empty() {
            return;
        }

        let mut idle_pending_input = self.idle_pending_input.lock().await;
        idle_pending_input.extend(items);
    }

    pub(crate) async fn take_queued_response_items_for_next_turn(&self) -> Vec<PendingInputItem> {
        std::mem::take(&mut *self.idle_pending_input.lock().await)
    }

    pub(crate) async fn has_queued_response_items_for_next_turn(&self) -> bool {
        !self.idle_pending_input.lock().await.is_empty()
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "active turn checks and turn state reads must remain atomic"
    )]
    pub async fn has_pending_input(&self) -> bool {
        let (has_turn_pending_input, accepts_mailbox_delivery) = {
            let active = self.active_turn.lock().await;
            match active.as_ref() {
                Some(at) => {
                    let ts = at.turn_state.lock().await;
                    (
                        ts.has_pending_input(),
                        ts.accepts_mailbox_delivery_for_current_turn(),
                    )
                }
                None => (false, true),
            }
        };
        if has_turn_pending_input {
            return true;
        }
        if !accepts_mailbox_delivery {
            return false;
        }
        self.has_pending_mailbox_items().await
    }

    pub async fn interrupt_task(self: &Arc<Self>) {
        info!("interrupt received: abort current task, if any");
        let had_active_turn = self.active_turn.lock().await.is_some();
        // Even without an active task, interrupt handling pauses any active goal.
        self.abort_all_tasks(TurnAbortReason::Interrupted).await;
        if !had_active_turn {
            self.cancel_mcp_startup().await;
        }
    }

    pub(crate) fn hooks(&self) -> SharedHookRuntime {
        let hooks = self
            .services
            .hooks
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(&*hooks)
    }

    pub(crate) fn user_shell(&self) -> Arc<shell::Shell> {
        Arc::clone(&self.services.user_shell)
    }

    pub(crate) async fn current_rollout_path(&self) -> anyhow::Result<Option<PathBuf>> {
        let Some(live_thread) = self.live_thread() else {
            return Ok(None);
        };
        live_thread.local_rollout_path().await.map_err(Into::into)
    }

    pub(crate) async fn hook_transcript_path(&self) -> Option<PathBuf> {
        self.ensure_rollout_materialized().await;
        match self.current_rollout_path().await {
            Ok(path) => path,
            Err(err) => {
                warn!("{err}");
                None
            }
        }
    }

    pub(crate) async fn take_pending_session_start_source(
        &self,
    ) -> Option<codex_hooks_api::SessionStartSource> {
        let mut state = self.state.lock().await;
        state.take_pending_session_start_source()
    }

    fn show_raw_agent_reasoning(&self) -> bool {
        self.services.show_raw_agent_reasoning
    }
}

fn estimate_history_token_count(
    history: &ContextManager,
    turn_context: &TurnContext,
) -> Option<i64> {
    let personality = turn_context.personality.or(turn_context.config.personality);
    let base_instructions = BaseInstructions {
        text: turn_context.model_info.get_model_instructions(personality),
    };
    history.estimate_token_count_with_base_instructions(&base_instructions)
}

pub(crate) fn emit_subagent_session_started(
    analytics_events_client: &AnalyticsEventsClient,
    client_metadata: AppServerClientMetadata,
    thread_id: ThreadId,
    parent_thread_id: Option<ThreadId>,
    thread_config: ThreadConfigSnapshot,
    subagent_source: SubAgentSource,
) {
    let AppServerClientMetadata {
        client_name,
        client_version,
    } = client_metadata;
    let (Some(client_name), Some(client_version)) = (client_name, client_version) else {
        tracing::warn!("skipping subagent thread analytics: missing inherited client metadata");
        return;
    };
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    analytics_events_client.track_subagent_thread_started(SubAgentThreadStartedInput {
        thread_id: thread_id.to_string(),
        parent_thread_id: parent_thread_id.map(|thread_id| thread_id.to_string()),
        product_client_id: client_name.clone(),
        client_name,
        client_version,
        model: thread_config.model,
        ephemeral: thread_config.ephemeral,
        subagent_source,
        created_at,
    });
}

impl codex_thread_api::SessionCommandHandle for Codex {
    fn submit_op(
        &self,
        op: Op,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit(op)
    }

    fn submit_op_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        self.submit_with_trace(op, trace)
    }

    fn submit_with_id(
        &self,
        submission: Submission,
    ) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        Codex::submit_with_id(self, submission)
    }

    fn shutdown(&self) -> impl std::future::Future<Output = CodexResult<()>> + Send + '_ {
        self.shutdown_and_wait()
    }

    fn append_conversation_item(
        &self,
        item: ResponseItem,
    ) -> impl std::future::Future<Output = CodexResult<String>> + Send + '_ {
        async move {
            let submission_id = uuid::Uuid::new_v4().to_string();
            self.session
                .enqueue_async_input(PendingInputItem::from(item));
            self.session.maybe_start_turn_for_pending_work().await;
            Ok(submission_id)
        }
    }
}

impl codex_thread_api::SessionStatusHandle for Codex {
    fn agent_status(
        &self,
    ) -> impl std::future::Future<Output = codex_protocol::protocol::AgentStatus> + Send + '_ {
        Codex::agent_status(self)
    }
}

/// Builds the hook engine for one config snapshot, including any enabled plugin hooks.
async fn build_hooks_for_config(
    config: &Config,
    plugins_manager: &dyn PluginRuntime,
    user_shell: &crate::runtime_shell_model::Shell,
    hook_runtime_factory: &dyn codex_hooks_api::HookRuntimeFactory,
) -> SharedHookRuntime {
    let mut hook_shell_argv = user_shell.derive_exec_args("", /*use_login_shell*/ false);
    let hook_shell_program = hook_shell_argv.remove(0);
    let _ = hook_shell_argv.pop();
    let plugin_hooks_enabled = config.features.enabled(Feature::PluginHooks);
    let (plugin_hook_sources, plugin_hook_load_warnings) = if plugin_hooks_enabled {
        let plugins_input = config.plugins_config_input();
        let plugin_outcome = plugins_manager.plugins_for_config(&plugins_input).await;
        (
            plugin_outcome.effective_plugin_hook_sources(),
            plugin_outcome.effective_plugin_hook_warnings(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    hook_runtime_factory.create(HooksConfig {
        legacy_notify_argv: config.notify.clone(),
        feature_enabled: config.features.enabled(Feature::CodexHooks),
        bypass_hook_trust: config.bypass_hook_trust,
        config_layer_stack: Some(hook_config_layer_stack_from_config_layer_stack(
            &config.config_layer_stack,
        )),
        plugin_hook_sources,
        plugin_hook_load_warnings,
        shell_program: Some(hook_shell_program),
        shell_args: hook_shell_argv,
    })
}

#[cfg(test)]
pub(crate) mod tests;
