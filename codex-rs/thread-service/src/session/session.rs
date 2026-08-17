use super::*;
#[path = "approval_review_runtime.rs"]
pub(crate) mod approval_review_runtime_impl;
#[path = "approval_review_session.rs"]
pub(crate) mod approval_review_session_impl;
#[path = "approval_support.rs"]
pub(crate) mod approval_support_impl;
#[path = "mcp_session.rs"]
pub(crate) mod mcp_session_impl;
pub(crate) use super::thread_wait::ThreadWaitSource;
use super::thread_wait::ThreadWaitState;
use crate::SessionPermissionProfileUpdate;
use crate::SessionSettingsApplyCurrent;
use crate::build_session_settings_apply_plan;
use crate::initial_thread_skills;
use crate::merge_thread_skills;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::GoalRuntimeState;
use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ApprovalSessionCapability;
use codex_auth_types::AuthRuntime;
use codex_auth_types::RequestAuthSnapshot;
use codex_auth_types::SharedAuthRuntime;
use codex_code_mode_api::CodeModeRuntimeFactory;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_config_types::RequirementSource;
use command_service_api::CommandServiceApi;
use config_service::ConstraintError;
use goal_service_api::GoalServiceApi;
use mcp_service_api::McpAuthRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpServiceApi;
use mcp_types::EffectiveMcpServer;
use mcp_types::McpAuthStatusEntry;
use memory_service_api::SharedMemoryToolDeveloperInstructionsProvider;
use model_service_api::CreateModelClientRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelSelectionPolicy;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelServiceApi;
use plugin_service_api::SharedPluginRuntime;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::ThreadSkill;
use protocol::protocol::ThreadSource;
use protocol::protocol::TurnEnvironmentSelection;
use session_telemetry_api::SessionTelemetryCreateParams;
use session_telemetry_api::SharedSessionTelemetryFactory;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::Weak;
use tokio::sync::Semaphore;

/// Context for an initialized model agent
///
/// A session has at most 1 running task at a time, and can be interrupted by user input.
#[cfg(test)]
pub(crate) struct GoalContinuationBeforeLaunchHook {
    pub(crate) started_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub(crate) continue_rx: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

pub struct Session {
    pub(crate) self_weak: OnceLock<Weak<Session>>,
    pub(crate) conversation_id: ThreadId,
    pub(crate) installation_id: String,
    pub(super) tx_event: Sender<Event>,
    pub(super) agent_status: watch::Sender<AgentStatus>,
    pub(super) out_of_band_elicitation_paused: watch::Sender<bool>,
    pub(super) state: Mutex<SessionState>,
    /// Serializes rebuild/apply cycles for the running proxy; each cycle
    /// rebuilds from the current SessionState while holding this lock.
    pub(super) managed_network_proxy_refresh_lock: Semaphore,
    /// The set of enabled features should be invariant for the lifetime of the
    /// session.
    pub(super) features: ManagedFeatures,
    pub(crate) pending_mcp_server_refresh_config: Mutex<Option<McpServerRefreshConfig>>,
    pub(crate) conversation: Arc<RealtimeConversationManager>,
    pub(crate) active_turn: Mutex<Option<ActiveTurn>>,
    pub(super) mailbox: Mailbox,
    pub(super) mailbox_rx: Mutex<MailboxReceiver>,
    pub(crate) idle_pending_input: Mutex<Vec<crate::PendingInputItem>>,
    pub(crate) last_parent_child_notification_status: Mutex<Option<ThreadLifecycleStatus>>,
    pub(crate) last_system_error_message: Mutex<Option<String>>,
    pub(crate) model_observed_display_events: Mutex<HashMap<String, Vec<EventMsg>>>,
    pub(crate) scheduler: Mutex<()>,
    #[cfg(test)]
    pub(crate) force_wait_command_for_tests: AtomicBool,
    #[cfg(test)]
    pub(crate) goal_continuation_before_launch_hook:
        Mutex<Option<Arc<GoalContinuationBeforeLaunchHook>>>,
    pub(crate) goal_runtime: GoalRuntimeState,
    pub(crate) guardian_review_session: approval_review_session_impl::GuardianReviewSessionManager,
    pub(crate) services: SessionServices,
    pub(super) next_internal_sub_id: AtomicU64,
    pub(super) thread_wait: ThreadWaitState,
}

#[derive(Clone)]
pub(crate) struct SessionConfiguration {
    /// Provider identifier ("openai", "openrouter", ...).
    pub(super) provider: ModelProviderInfo,

    pub(super) collaboration_mode: CollaborationMode,
    pub(super) model_reasoning_summary: Option<ReasoningSummaryConfig>,
    pub(super) service_tier: Option<String>,

    /// Developer instructions that supplement the base instructions.
    pub(super) developer_instructions: Option<String>,

    /// Model instructions that are appended to the base instructions.
    pub(super) user_instructions: Option<String>,

    /// Personality preference for the model.
    pub(super) personality: Option<Personality>,

    /// Base instructions for the session.
    pub(super) base_instructions: String,

    /// Compact prompt override.
    pub(super) compact_prompt: Option<String>,

    /// When to escalate for approval for execution
    pub(super) approval_policy: Constrained<AskForApproval>,
    pub(super) approval_policy_is_session_override: bool,
    pub(super) approvals_reviewer: ApprovalsReviewer,
    /// Permission profile state for the session. Keep the constrained profile,
    /// active profile id, and profile-defined workspace roots in sync by using
    /// the methods below instead of mutating the fields independently.
    pub(super) permission_profile_state: PermissionProfileState,
    pub(super) permission_profile_is_session_override: bool,
    pub(super) windows_sandbox_level: WindowsSandboxLevel,

    /// Absolute working directory that should be treated as the *root* of the
    /// session. All relative paths supplied by the model as well as the
    /// execution sandbox are resolved against this directory **instead** of
    /// the process-wide current working directory.
    pub(super) cwd: AbsolutePathBuf,
    /// Thread-scoped runtime workspace roots for materializing symbolic
    /// workspace permissions at session runtime.
    pub(super) workspace_roots: Vec<AbsolutePathBuf>,
    /// Directory containing all Codex state for this session.
    pub(super) codex_home: AbsolutePathBuf,
    /// Optional user-facing name for the thread, updated during the session.
    pub(super) thread_name: Option<String>,
    /// Sticky environments for turns that do not provide a turn-local override.
    pub(super) environments: Vec<TurnEnvironmentSelection>,

    // TODO(pakrym): Remove config from here
    pub(super) original_config_do_not_use: Arc<Config>,
    /// Optional service name tag for session metrics.
    pub(super) metrics_service_name: Option<String>,
    /// Terminal identifier resolved by the composition root.
    pub(super) terminal_type: String,
    pub(super) app_server_client_name: Option<String>,
    pub(super) app_server_client_version: Option<String>,
    /// Source of the session (cli, vscode, exec, mcp, ...)
    pub(super) session_source: SessionSource,
    /// Optional analytics source classification for this thread.
    pub(super) thread_source: Option<ThreadSource>,
    /// Metadata for a root-scope agent supplied at thread creation time.
    ///
    /// Root-scope agent threads are not represented as `SessionSource::SubAgent`,
    /// and their registry entry cannot be keyed by thread id until after the
    /// session has spawned. Keep the creation-time metadata here so initial
    /// context materialized during spawn can still use the real canonical path.
    pub(super) root_agent_metadata: Option<AgentMetadata>,
    pub(super) dynamic_tools: Vec<DynamicToolSpec>,
    pub(super) persist_extended_history: bool,
    pub(super) inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
    pub(super) user_shell_override: Option<shell::Shell>,
}

pub(crate) struct AppServerClientMetadata {
    pub(crate) client_name: Option<String>,
    pub(crate) client_version: Option<String>,
}

mod bootstrap;
mod bootstrap_support;
mod configuration;
