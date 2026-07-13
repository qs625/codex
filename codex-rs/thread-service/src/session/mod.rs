use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::Mailbox;
use crate::MailboxReceiver;
use crate::PendingInputItem;
use crate::agent::AgentControl;
use crate::agent::AgentMode;
use crate::agent::AgentStatus;
use crate::agent::SpawnAgentOptions;
use crate::agent::agent_status_from_event;
use crate::agent::status::is_final;
use crate::build_skill_service_input_from_config;
use crate::compact;
use crate::context_usage::build_thread_context_usage;
use crate::context_usage::build_thread_context_usage_from_history;
use crate::environment_selection::ResolvedTurnEnvironments;
use crate::event_mapping::completed_display_event_from_model_item;
use crate::event_mapping::injected_context_item_from_response_items;
use crate::event_mapping::is_structured_display_response_item;
use crate::event_mapping::started_display_event_from_model_item;
use crate::parse_turn_item;
use crate::path_utils::normalize_for_native_workdir;
use crate::realtime_conversation::RealtimeConversationManager;
use crate::session_prefix::format_subagent_notification_message;
use crate::turn_metadata::TurnMetadataState;
use crate::turn_timing::now_unix_timestamp_ms;
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
use codex_auth_types::AuthEnvTelemetryInput;
use codex_auth_types::AuthRuntime;
use codex_auth_types::SharedAuthRuntime;
use codex_auth_types::collect_auth_env_telemetry;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_extension_api::PromptSlot;
use codex_features::FEATURES;
use codex_features::Feature;
use codex_features::unstable_features_warning_event;
use codex_file_system::FileSystemSandboxContext;
use codex_network_proxy_api::BlockedRequestObserver;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkProxyAuditMetadata;
use codex_network_proxy_api::NetworkProxyRuntimeFactory;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_network_proxy_api::SharedNetworkProxyRuntimeFactory;
use codex_openai_files_api::SharedOpenAiFileUploader;
use codex_sandboxing_api::normalize_request_permissions_response;
use codex_shell_utils::parse_command::parse_command;
use codex_utils_output_truncation::TruncationPolicy;
use command_service_api::CommandSessionError;
use command_service_api::CommandWaitOperation;
use command_service_api::CommandWaitRequest;
use command_service_api::WriteStdinOutput;
use command_service_api::WriteStdinRequest;
use config_service::ManagedFeatures;
use config_service::hook_config_layer_stack_from_config_layer_stack;
use config_service::resolve_tool_suggest_config_from_layer_stack;
use exec_server_api::ExecEnvironmentProvider;
use futures::future::BoxFuture;
use futures::future::Shared;
use futures::prelude::*;
use hooks::PreToolUseHookResult;
use hooks::record_additional_contexts;
use hooks::run_post_tool_use_hooks;
use hooks::run_pre_tool_use_hooks;
use hooks_api::HooksConfig;
use hooks_api::SharedHookRuntime;
use hooks_api::SharedHookRuntimeFactory;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpServiceApi;
use mcp_types::McpClientElicitationSupport;
use mcp_types::codex_apps_tools_cache_key;
use model_service::AttestationProvider;
use model_service::ModelService;
use model_service::ModelServiceRuntimeDeps;
use model_service_api::ListModelsRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelSelectionPolicy;
use model_service_api::ModelServiceApi;
use model_service_api::ResolveDefaultModelRequest;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelProviderFactory;
use permissions_service::ExecPolicyLoader;
use permissions_service::ExecPolicyManager;
use permissions_service::validate_network_policy_amendment_host;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::approvals::ExecPolicyAmendment;
use protocol::approvals::NetworkPolicyAmendment;
use protocol::approvals::NetworkPolicyRuleAction;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::ModeKind;
use protocol::config_types::Settings;
use protocol::config_types::WebSearchMode;
use protocol::dynamic_tools::DynamicToolResponse;
use protocol::dynamic_tools::DynamicToolSpec;
use protocol::items::TurnItem;
use protocol::items::UserMessageItem;
use protocol::models::ActivePermissionProfile;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::BaseInstructions;
use protocol::models::PermissionProfile;
use protocol::models::format_allow_prefixes;
use protocol::openai_models::ModelInfo;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::permissions::NetworkSandboxPolicy;
use protocol::protocol::FileChange;
use protocol::protocol::HasLegacyEvent;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::ItemCompletedEvent;
use protocol::protocol::ItemStartedEvent;
use protocol::protocol::ResponseItemCompletedEvent;
use protocol::protocol::ResponseItemStartedEvent;
use protocol::protocol::ReviewRequest;
use protocol::protocol::RolloutItem;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadContextUsageUpdatedEvent;
use protocol::protocol::ThreadSource;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnContextItem;
use protocol::protocol::TurnContextNetworkItem;
use protocol::protocol::TurnEnvironmentSelection;
use protocol::protocol::W3cTraceContext;
use protocol::request_permissions::PermissionGrantScope;
use protocol::request_permissions::RequestPermissionProfile;
use protocol::request_permissions::RequestPermissionsArgs;
use protocol::request_permissions::RequestPermissionsEvent;
use protocol::request_permissions::RequestPermissionsResponse;
use protocol::request_user_input::RequestUserInputArgs;
use protocol::request_user_input::RequestUserInputResponse;
use rollout_trace_api::AgentResultTracePayload;
use rollout_trace_api::ThreadStartedTraceMetadata;
use rollout_trace_api::ThreadTraceContext;
use serde_json::Value;
use skill_service_api::build_available_skills;
use skill_service_api::default_skill_metadata_budget;
use skill_service_api::render::SkillRenderSideEffects;
use thread_store_api::CreateThreadParams;
use thread_store_api::LiveThreadFactory;
use thread_store_api::LiveThreadHandle;
use thread_store_api::ReadThreadParams;
use thread_store_api::ResumeThreadParams;
use thread_store_api::SharedLiveThread;
use thread_store_api::ThreadEventPersistenceMode;
use thread_store_api::ThreadPersistenceMetadata;
use thread_store_api::ThreadStore;
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
use transport_client_identity::originator;
use uuid::Uuid;

#[cfg(test)]
use crate::compact::collect_user_messages;
use crate::thread::ThreadConfigSnapshot;
use codex_config_types::ConfigLayerSource;
use codex_context_manager::ContextManager;
use codex_context_manager::PreviousTurnSettingsView;
use codex_context_manager::SettingsUpdateInput;
use config_service::CONFIG_TOML_FILE;
use config_service::Config;
use config_service::ConfigLayerStackOrdering;
use config_service::Constrained;
use config_service::ConstraintResult;
use config_service::PermissionProfileState;
use config_service::StartedNetworkProxy;
use model_service_api::ModelProviderInfo;
use protocol::config_types::ShellEnvironmentPolicy;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
#[cfg(test)]
use protocol::exec_output::StreamOutput;
use rollout_api::initial_history_has_prior_user_turns;
use thread_service_api::PostToolUsePayload;
use tool_service_api::UPDATE_GOAL_TOOL_NAME;

mod codex_runtime;
mod config_lock;
mod events_history;
mod handlers;
mod multi_agents;
mod pending_input;
mod review;
mod rollout_reconstruction;
#[allow(clippy::module_inception)]
pub(crate) mod session;
pub(crate) mod turn;
pub(crate) mod turn_context;
use self::codex_runtime::CYBER_SAFETY_URL;
use self::codex_runtime::CYBER_VERIFY_URL;
pub(crate) use self::codex_runtime::Codex;
pub(crate) use self::codex_runtime::CodexSpawnArgs;
pub(crate) use self::codex_runtime::CodexSpawnOk;
pub(crate) use self::codex_runtime::INITIAL_SUBMIT_ID;
use self::codex_runtime::LiveThreadInitGuard;
pub(crate) use self::codex_runtime::SUBMISSION_CHANNEL_CAPACITY;
use self::codex_runtime::SessionLoopTermination;
use self::codex_runtime::duration_from_config_ms;
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

use self::session::approval_support_impl::ApprovalStore;
use crate::ActiveSteerTurn;
pub(crate) use crate::SessionSettingsUpdate;
pub use crate::SteerInputError;
use crate::SteerableTaskKind;
use crate::resolve_session_service_tier;
use crate::validate_steer_input;
pub(crate) use rollout_api::PreviousTurnSettings;

fn previous_turn_settings_view(
    previous_turn_settings: Option<&PreviousTurnSettings>,
) -> Option<PreviousTurnSettingsView<'_>> {
    previous_turn_settings.map(|settings| PreviousTurnSettingsView {
        model: settings.model.as_str(),
        realtime_active: settings.realtime_active,
    })
}

use self::session::approval_review_session_impl::GuardianReviewSessionManager;
use crate::agents_md::AgentsMdManager;
use crate::rollout::map_session_init_error;
use crate::runtime_shell_model as shell;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::session_startup_prewarm::SessionStartupPrewarmHandle;
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
use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::GuardianReviewDispatch;
use codex_approval_service_api::execpolicy_network_rule_amendment;
use codex_approval_service_api::is_guardian_reviewer_source;
use codex_approval_service_api::routes_approval_to_guardian;
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
use codex_git_info::get_git_repo_root;
use codex_otel::context_from_w3c_trace_context;
use codex_otel::current_span_trace_id;
use codex_otel::current_span_w3c_trace_context;
use codex_otel::set_parent_from_w3c_trace_context;
use codex_sandboxing::WindowsSandboxLevelExt;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_turn_items::realtime_text_for_event;
use codex_utils_absolute_path::AbsolutePathBuf;
use command_service_api::ExecCommandRunOutput;
use command_service_api::ExecCommandRunRequest;
use memory_service_api::SharedMemoryToolDeveloperInstructionsProvider;
use metrics_api::THREAD_STARTED_METRIC;
use permissions_service::ExecPolicyUpdateError;
use plugin_service_api::PluginRuntime;
use plugin_service_api::SharedPluginRuntime;
use protocol::config_types::CollaborationMode;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::config_types::WindowsSandboxLevel;
use protocol::models::ContentItem;
use protocol::models::ResponseInputItem;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use protocol::protocol::ApplyPatchApprovalRequestEvent;
use protocol::protocol::AskForApproval;
use protocol::protocol::CodexErrorInfo;
use protocol::protocol::CompactedItem;
use protocol::protocol::DeprecationNoticeEvent;
use protocol::protocol::ErrorEvent;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::ExecApprovalRequestEvent;
use protocol::protocol::InitialHistory;
use protocol::protocol::McpServerRefreshConfig;
use protocol::protocol::ModelRerouteEvent;
use protocol::protocol::ModelRerouteReason;
use protocol::protocol::ModelVerification;
use protocol::protocol::ModelVerificationEvent;
use protocol::protocol::NetworkApprovalContext;
use protocol::protocol::Op;
use protocol::protocol::RateLimitSnapshot;
use protocol::protocol::RequestUserInputEvent;
use protocol::protocol::ReviewDecision;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::SessionNetworkProxyRuntime;
use protocol::protocol::StreamErrorEvent;
use protocol::protocol::Submission;
use protocol::protocol::ThreadMemoryMode;
use protocol::protocol::TokenCountEvent;
use protocol::protocol::TokenUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::WarningEvent;
use protocol::user_input::UserInput;
use session_telemetry_api::SharedSessionTelemetry;
use session_telemetry_api::SharedSessionTelemetryFactory;
use skill_service_api::SharedSkillServiceApi;
#[cfg(test)]
use skill_service_api::SkillLoadOutcome;
use tool_config::ToolEnvironmentMode;
use tool_config::ToolsConfig;
use tool_config::ToolsConfigParams;

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

impl Session {
    pub(crate) fn thread_id(&self) -> ThreadId {
        self.conversation_id
    }

    pub fn thread_id_string(&self) -> String {
        self.conversation_id.to_string()
    }

    pub(crate) fn tool_user_shell_type(&self) -> tool_config::ToolUserShellType {
        self.user_shell().shell_type.tool_user_shell_type()
    }

    pub(crate) fn runtime_shell(&self) -> command_service_api::RuntimeShell {
        self.user_shell().as_ref().to_runtime_shell()
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
        let config = self.get_config().await.as_ref().clone();
        self.services
            .agent_control
            .resolve_agent_reference(
                self.conversation_id,
                &turn.session_source(),
                Some(config),
                target,
            )
            .await
    }

    pub(crate) async fn resolve_agent_thread_id_for_turn(
        &self,
        turn: &TurnContext,
        target_thread_id: ThreadId,
    ) -> CodexResult<ThreadId> {
        let config = self.get_config().await.as_ref().clone();
        self.services
            .agent_control
            .resolve_agent_thread_id(
                self.conversation_id,
                &turn.session_source(),
                Some(config),
                target_thread_id,
            )
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

    #[allow(clippy::await_holding_invalid_type)]
    pub async fn list_all_mcp_tools(&self) -> Vec<mcp_types::ToolInfo> {
        let manager = self.services.mcp_connection_manager.read().await;
        manager.list_all_tools().await
    }

    pub async fn mcp_server_origin(&self, server: &str) -> Option<String> {
        let manager = self.services.mcp_connection_manager.read().await;
        mcp_service_api::McpToolRuntime::server_origin(manager.as_ref(), server)
    }

    pub async fn mcp_server_is_host_owned_codex_apps(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        mcp_service_api::McpToolRuntime::is_host_owned_codex_apps_server(manager.as_ref(), server)
    }

    #[allow(clippy::await_holding_invalid_type)]
    pub async fn mcp_server_supports_sandbox_state_meta(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        mcp_service_api::McpToolRuntime::server_supports_sandbox_state_meta_capability(
            manager.as_ref(),
            server,
        )
        .await
        .unwrap_or(false)
    }

    pub(crate) async fn mcp_server_pollutes_memory(&self, server: &str) -> bool {
        let manager = self.services.mcp_connection_manager.read().await;
        mcp_service_api::McpToolRuntime::server_pollutes_memory(manager.as_ref(), server)
    }

    #[allow(clippy::await_holding_invalid_type)]
    pub(crate) async fn hard_refresh_codex_apps_tools_cache(
        &self,
    ) -> anyhow::Result<Vec<mcp_types::ToolInfo>> {
        let manager = self.services.mcp_connection_manager.read().await;
        manager.hard_refresh_codex_apps_tools_cache().await
    }

    pub(crate) async fn queue_mcp_server_refresh(&self, refresh_config: McpServerRefreshConfig) {
        let mut guard = self.pending_mcp_server_refresh_config.lock().await;
        *guard = Some(refresh_config);
    }

    pub async fn rewrite_mcp_tool_arguments_for_openai_files(
        &self,
        turn: &TurnContext,
        arguments_value: Option<serde_json::Value>,
        openai_file_input_params: Option<&[String]>,
    ) -> Result<Option<serde_json::Value>, String> {
        let auth = self.services.auth_runtime.auth().await;
        self.services
            .mcp_service
            .rewrite_tool_arguments_for_openai_files(
                self.services.openai_file_uploader.as_ref(),
                auth.as_ref(),
                turn.chatgpt_base_url(),
                turn,
                arguments_value,
                openai_file_input_params,
            )
            .await
    }

    pub fn add_optional_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        self.services
            .rollout_thread_trace
            .start_mcp_call_trace(call_id)
            .add_request_meta(meta)
    }

    pub async fn mark_thread_memory_mode_polluted_for_mcp_tool_call(
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

    pub async fn track_codex_app_used_for_mcp_tool(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) {
        if server != mcp_types::CODEX_APPS_MCP_SERVER_NAME {
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
    ) -> Option<mcp_service_api::McpAppUsageMetadata> {
        let tools = self.list_all_mcp_tools().await;
        self.services
            .mcp_service
            .lookup_app_usage_metadata(&tools, server, tool_name)
    }

    pub async fn mcp_tool_approval_is_remembered(
        &self,
        key: &mcp_types::McpToolApprovalKey,
    ) -> bool {
        let store = self.services.tool_approvals.lock().await;
        matches!(store.get(key), Some(ReviewDecision::ApprovedForSession))
    }

    pub async fn remember_mcp_tool_approval(&self, key: mcp_types::McpToolApprovalKey) {
        let mut store = self.services.tool_approvals.lock().await;
        store.put(key, ReviewDecision::ApprovedForSession);
    }

    pub(crate) fn plugins_manager(&self) -> &dyn plugin_service_api::PluginRuntime {
        self.services.plugins_manager.as_ref()
    }

    pub async fn custom_mcp_tool_approval_mode(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) -> codex_config_types::AppToolApproval {
        self.services
            .mcp_service
            .custom_tool_approval_mode(
                self.services.plugins_manager.as_ref(),
                turn.config.as_ref(),
                server,
                tool_name,
            )
            .await
    }

    pub async fn fetch_accessible_connectors_from_mcp_tools(
        &self,
        turn: &TurnContext,
        auth_snapshot: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> anyhow::Result<Vec<codex_connectors_api::AppInfo>> {
        self.services
            .mcp_service
            .fetch_accessible_connectors(
                self.services.plugins_manager.as_ref(),
                turn.config.as_ref(),
                auth_snapshot,
                self.services.environment_manager.as_ref(),
                self.services.mcp_auth_runtime.as_ref(),
                self.services.mcp_connection_runtime_factory.as_ref(),
            )
            .await
    }

    pub async fn persist_codex_app_tool_approval_for_turn(
        &self,
        turn: &TurnContext,
        connector_id: &str,
        tool_name: &str,
    ) -> anyhow::Result<()> {
        self.services
            .mcp_service
            .persist_codex_app_tool_approval(turn.config.as_ref(), connector_id, tool_name)
            .await
    }

    pub async fn persist_non_app_mcp_tool_approval_for_turn(
        &self,
        turn: &TurnContext,
        server: &str,
        tool_name: &str,
    ) -> anyhow::Result<()> {
        self.services
            .mcp_service
            .persist_non_app_mcp_tool_approval(
                self.services.plugins_manager.as_ref(),
                turn.config.as_ref(),
                server,
                tool_name,
            )
            .await
    }

    pub(crate) async fn configured_mcp_servers(
        &self,
        config: &Config,
    ) -> HashMap<String, codex_config_types::McpServerConfig> {
        self.services
            .mcp_service
            .configured_servers(self.services.plugins_manager.as_ref(), config)
            .await
    }

    pub(crate) async fn mcp_oauth_login_support(
        &self,
        transport: &codex_config_types::McpServerTransportConfig,
    ) -> mcp_types::McpOAuthLoginSupport {
        self.services
            .mcp_auth_runtime
            .oauth_login_support(transport)
            .await
    }

    pub(crate) async fn perform_mcp_oauth_login(
        &self,
        request: mcp_service_api::McpOAuthLoginRequest,
    ) -> anyhow::Result<()> {
        self.services
            .mcp_auth_runtime
            .perform_oauth_login(request)
            .await
    }

    pub(crate) fn should_retry_mcp_oauth_without_scopes(
        &self,
        scopes: &mcp_types::ResolvedMcpOAuthScopes,
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
    ) -> Vec<protocol::openai_models::ModelPreset> {
        self.services
            .model_service
            .list_models(ListModelsRequest {
                include_hidden: true,
                refresh: ModelCatalogRefresh::Offline,
            })
            .await
            .unwrap_or_default()
    }

    pub(crate) async fn spawn_agent_model_info(
        &self,
        model: &str,
        _config: &Config,
    ) -> CodexResult<ModelInfo> {
        self.services
            .model_service
            .get_model_info(model)
            .await
            .map_err(|err| CodexErr::Fatal(format!("failed to resolve model info: {err}")))
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
    ) -> hooks::PostToolUseOutcome {
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
        tool_name: &str,
    ) -> Result<(), String> {
        if tool_name == UPDATE_GOAL_TOOL_NAME {
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
    ) -> Result<ExecCommandRunOutput, command_service_api::UnifiedExecError> {
        self.services
            .command_service_state
            .run_exec_command(
                Arc::clone(self) as Arc<dyn thread_service_api::ThreadSessionCapability>,
                Arc::clone(self) as Arc<dyn codex_approval_service_api::ApprovalSessionCapability>,
                turn as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
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
    /// `model-service` 创建的 session-scoped client 不直接依赖完整 `Config`，所以这里预先
    /// 计算启用的实验特性 header，并在线程创建时传入模型 client。
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

    #[allow(clippy::too_many_arguments)]
    async fn start_managed_network_proxy(
        spec: &config_service::NetworkProxySpec,
        factory: &dyn NetworkProxyRuntimeFactory,
        exec_policy: &permissions_service_api::Policy,
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
        {
            let mut state = self.state.lock().await;
            state.replace_history_with_compact_window_start(
                reconstructed_rollout.history,
                reconstructed_rollout.reference_context_item,
                Self::last_compact_window_start_from_rollout(rollout_items),
            );
        }
        self.set_previous_turn_settings(previous_turn_settings.clone())
            .await;
        previous_turn_settings
    }

    fn last_compact_window_start_from_rollout(rollout_items: &[RolloutItem]) -> Option<usize> {
        rollout_items.iter().rev().find_map(|item| match item {
            RolloutItem::Compacted(compacted) => compacted
                .replacement_history
                .as_ref()
                .map(std::vec::Vec::len),
            _ => None,
        })
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
        self.services.skill_service.clear_cache();
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

    pub async fn reload_user_config_layer(&self) {
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
    ) -> Option<hooks_api::SessionStartSource> {
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

/// Builds the hook engine for one config snapshot, including any enabled plugin hooks.
async fn build_hooks_for_config(
    config: &Config,
    plugins_manager: &dyn PluginRuntime,
    user_shell: &crate::runtime_shell_model::Shell,
    hook_runtime_factory: &dyn hooks_api::HookRuntimeFactory,
) -> SharedHookRuntime {
    let mut hook_shell_argv = user_shell.derive_exec_args("", /*use_login_shell*/ false);
    let hook_shell_program = hook_shell_argv.remove(0);
    let _ = hook_shell_argv.pop();
    let plugins_input = config.plugins_config_input();
    let (plugin_hook_sources, plugin_hook_load_warnings) = plugins_manager
        .plugin_hook_sources_for_config(
            &plugins_input,
            config.features.enabled(Feature::PluginHooks),
        )
        .await;

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

async fn merge_plugin_agent_roles_for_config(
    plugins_manager: &dyn PluginRuntime,
    plugins_input: &plugin_service_api::PluginsConfigInput,
    agent_roles: &mut std::collections::BTreeMap<String, crate::config::AgentRoleConfig>,
    startup_warnings: &mut Vec<String>,
) {
    let plugin_agent_dirs = plugins_manager
        .plugin_agent_dirs_for_config(plugins_input)
        .await;
    if plugin_agent_dirs.is_empty() {
        return;
    }

    let plugin_agent_dirs = plugin_agent_dirs
        .into_iter()
        .map(|agent_dir| (agent_dir.plugin_id, agent_dir.path))
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if let Err(err) = config_service::agent_roles::merge_missing_agent_roles_from_plugin_dirs(
        codex_file_system::LOCAL_FS.as_ref(),
        agent_roles,
        &plugin_agent_dirs,
        &mut warnings,
    )
    .await
    {
        warn!("failed to load plugin agent definitions: {err}");
    }
    startup_warnings.extend(warnings);
}

#[cfg(test)]
pub(crate) mod tests;
