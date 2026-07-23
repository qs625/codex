//! Core-independent live thread operation API.
//!
//! `thread-store-api` owns persisted thread storage. This crate is the
//! unified public API surface for live thread runtime access, including the
//! previous session-facing traits that are now treated as part of the thread
//! runtime boundary.

mod exec_runtime;
mod session_contracts;
mod turn_diff_tracker;

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use codex_config_state::ConfigLayerEntry;
use codex_config_types::ConstraintResult;
use codex_features::Feature;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::CollaborationMode;
use protocol::config_types::Personality;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::WindowsSandboxLevel;
use protocol::error::Result as CodexResult;
use protocol::models::ActivePermissionProfile;
use protocol::models::PermissionProfile;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AgentStatus;
use protocol::protocol::AskForApproval;
use protocol::protocol::Event;
use protocol::protocol::Op;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::SessionSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::ThreadSource;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::W3cTraceContext;
use skill_service_api::SkillWatchPath;
use state_api::ExternalGoalSet;
use state_api::SharedStateDbRuntime;
use thread_store_api::StoredThread;
use thread_store_api::StoredThreadHistory;
use thread_store_api::ThreadStoreResult;

pub use exec_runtime::*;
pub use session_contracts::*;
pub use turn_diff_tracker::TurnDiffTracker;

/// Live thread configuration data needed by clients and persisted metadata paths.
///
/// This DTO intentionally contains only copied configuration values. Runtime
/// helpers that require sandbox, permission, or service implementations should
/// live in the consumer crate instead of pulling those implementations into this
/// API crate.
#[derive(Clone, Debug)]
pub struct ThreadConfigSnapshot {
    pub model: String,
    pub model_provider_id: String,
    pub service_tier: Option<String>,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub permission_profile: PermissionProfile,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub cwd: AbsolutePathBuf,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub profile_workspace_roots: Vec<AbsolutePathBuf>,
    pub ephemeral: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub personality: Option<Personality>,
    pub session_source: SessionSource,
    pub root_agent_path: Option<String>,
    pub root_agent_role: Option<String>,
    pub thread_source: Option<ThreadSource>,
}

/// Copied live thread config context needed to refresh app-server-owned MCP config.
///
/// This carries only the working directory and session config layers needed to
/// rebuild the latest effective app-server config while preserving
/// session-scoped overrides. It does not expose the concrete runtime `Config`.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveThreadConfigRefreshSnapshot {
    pub cwd: AbsolutePathBuf,
    pub session_layers: Vec<ConfigLayerEntry>,
}

/// Turn context overrides that a caller wants to apply to the next live turn.
///
/// This is a data-only request shape. Validation and application belong to the
/// concrete live thread runtime.
#[derive(Clone, Default)]
pub struct CodexThreadTurnContextOverrides {
    pub cwd: Option<PathBuf>,
    pub workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub profile_workspace_roots: Option<Vec<AbsolutePathBuf>>,
    pub approval_policy: Option<AskForApproval>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub permission_profile: Option<PermissionProfile>,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub windows_sandbox_level: Option<WindowsSandboxLevel>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<Option<ReasoningEffort>>,
    pub summary: Option<ReasoningSummary>,
    pub service_tier: Option<Option<String>>,
    pub collaboration_mode: Option<CollaborationMode>,
    pub personality: Option<Personality>,
}

/// Canonical coarse runtime state for a live thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadRuntimeStatus {
    Active,
    IdleWaitCommand,
    IdleWaitChild,
    IdleWaitEventSubscription,
    Complete,
}

/// Lightweight identity metadata for a loaded live thread.
///
/// This intentionally excludes config snapshots, stores, and runtime services so
/// app/server consumers can avoid depending on concrete core thread types for
/// basic identity checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveThreadInfo {
    pub session_id: SessionId,
    pub rollout_path: Option<PathBuf>,
}

/// Lightweight live thread snapshot for presentation and status caches.
///
/// This intentionally excludes live history, config handles, stores, and event
/// streams. Consumers that need those runtime capabilities should use a
/// concrete owner crate API instead of growing this DTO.
#[derive(Clone, Debug)]
pub struct LiveThreadSnapshot {
    pub info: LiveThreadInfo,
    pub config_snapshot: ThreadConfigSnapshot,
}

/// Runtime facts needed to decide whether a live thread is still active.
///
/// This is intentionally data-only so agent orchestration can consume thread
/// activity without depending on concrete session or thread implementation
/// types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveThreadActivitySnapshot {
    pub manager_available: bool,
    pub active_event_subscription_count: usize,
    pub thread_found: bool,
    pub has_active_turn: bool,
    pub status: Option<AgentStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppServerClientInfo {
    pub app_server_client_name: Option<String>,
    pub app_server_client_version: Option<String>,
    pub mcp_elicitations_auto_deny: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadCreatedEvent {
    Started(ThreadId),
    Resumed(ThreadId),
    StatusChanged(ThreadId),
}

impl ThreadCreatedEvent {
    pub fn thread_id(&self) -> ThreadId {
        match self {
            Self::Started(thread_id)
            | Self::Resumed(thread_id)
            | Self::StatusChanged(thread_id) => *thread_id,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ThreadShutdownReport {
    pub completed: Vec<ThreadId>,
    pub submit_failed: Vec<ThreadId>,
    pub timed_out: Vec<ThreadId>,
}

#[derive(Default)]
pub struct ActiveEventSubscriptionTracker {
    counts_by_thread_id: Mutex<HashMap<ThreadId, usize>>,
}

impl ActiveEventSubscriptionTracker {
    pub fn set_active_count(&self, thread_id: ThreadId, active_count: usize) {
        let mut counts_by_thread_id = self.counts_by_thread_id();
        if active_count == 0 {
            counts_by_thread_id.remove(&thread_id);
        } else {
            counts_by_thread_id.insert(thread_id, active_count);
        }
    }

    pub fn active_count(&self, thread_id: ThreadId) -> usize {
        self.counts_by_thread_id()
            .get(&thread_id)
            .copied()
            .unwrap_or(0)
    }

    fn counts_by_thread_id(&self) -> MutexGuard<'_, HashMap<ThreadId, usize>> {
        match self.counts_by_thread_id.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Read-only activity surface for agent/thread lifecycle decisions.
///
/// Implementations should gather copied runtime facts from the concrete thread
/// owner and must not expose session or thread implementation handles through
/// this API.
pub trait LiveThreadActivitySource: Send + Sync {
    fn live_thread_activity_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = LiveThreadActivitySnapshot> + Send + '_;
}

/// Command surface for live thread operations that do not need concrete handles.
///
/// Implementations own thread lookup, operation submission, and client-info
/// writes. Lifecycle teardown/status callers should use `ThreadLifecycleRuntime`
/// so command runtime stays focused on driving live operations during the
/// provider boundary migration.
pub trait LiveThreadCommandRuntime: Send + Sync {
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;

    fn submit_live_thread_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;

    fn set_live_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: AppServerClientInfo,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;
}

/// Conversation append surface for live threads without exposing concrete handles.
///
/// Implementations enqueue a prebuilt conversation item through the live thread
/// input path so it can be consumed like other async input.
pub trait LiveThreadConversationRuntime: Send + Sync {
    /// Append a single prebuilt conversation item to a specific live thread.
    fn append_live_thread_conversation_item(
        &self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;
}

/// Conversation injection surface for live threads without exposing concrete handles.
///
/// Implementations record prebuilt conversation items directly into a live
/// thread's conversation history. Unlike `LiveThreadConversationRuntime`, this
/// must not enqueue async input or trigger pending work.
pub trait LiveThreadConversationInjectionRuntime: Send + Sync {
    /// Inject prebuilt conversation items into a specific live thread.
    fn inject_live_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;
}

/// Persisted history read surface for loaded live threads without exposing handles.
pub trait LiveThreadHistoryRuntime: Send + Sync {
    /// Return live persisted history for a specific loaded thread.
    fn live_thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> impl Future<Output = ThreadStoreResult<StoredThreadHistory>> + Send + '_;
}

/// Live thread surface needed by listener/event-stream orchestration.
pub trait LiveThreadListenerHandle: Send + Sync {
    fn session_configured(&self) -> SessionConfiguredEvent;

    fn next_event(&self) -> impl Future<Output = CodexResult<Event>> + Send + '_;

    fn submit_thread_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_;

    fn runtime_thread_status(&self) -> impl Future<Output = ThreadRuntimeStatus> + Send + '_;

    fn config_snapshot(&self) -> impl Future<Output = ThreadConfigSnapshot> + Send + '_;

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> impl Future<Output = ThreadStoreResult<StoredThread>> + Send + '_;
}

impl<T> LiveThreadListenerHandle for T
where
    T: LiveThreadHandle + ?Sized,
{
    fn session_configured(&self) -> SessionConfiguredEvent {
        LiveThreadHandle::session_configured(self)
    }

    fn next_event(&self) -> impl Future<Output = CodexResult<Event>> + Send + '_ {
        LiveThreadHandle::next_event(self)
    }

    fn submit_thread_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_ {
        LiveThreadHandle::submit_thread_op(self, op)
    }

    fn runtime_thread_status(&self) -> impl Future<Output = ThreadRuntimeStatus> + Send + '_ {
        LiveThreadHandle::runtime_thread_status(self)
    }

    fn config_snapshot(&self) -> impl Future<Output = ThreadConfigSnapshot> + Send + '_ {
        LiveThreadHandle::config_snapshot(self)
    }

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> impl Future<Output = ThreadStoreResult<StoredThread>> + Send + '_ {
        LiveThreadHandle::read_thread(self, include_archived, include_history)
    }

}

/// Listener/event-stream lookup surface for loaded live threads.
pub trait LiveThreadListenerRuntime: Send + Sync {
    type ListenerHandle: LiveThreadListenerHandle + 'static;

    fn live_thread_listener_handle(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<Arc<Self::ListenerHandle>>> + Send + '_;
}

/// Turn preflight surface for live threads without exposing concrete handles.
///
/// Implementations validate turn-scoped inputs against the live thread but do
/// not enqueue or apply those inputs.
pub trait LiveThreadTurnRuntime: Send + Sync {
    /// Validate turn context overrides before accepting new turn input.
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: CodexThreadTurnContextOverrides,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;
}

/// Read-only inspection surface for live threads without exposing concrete handles.
///
/// Implementations should return copied snapshots derived from the concrete
/// thread owner. Consumers should use this trait when they only need feature
/// flags, configuration, or the loaded live thread set.
pub trait LiveThreadInspectionRuntime: Send + Sync {
    fn list_live_thread_ids(&self) -> impl Future<Output = Vec<ThreadId>> + Send + '_;

    fn is_live_thread_loaded(&self, thread_id: ThreadId) -> impl Future<Output = bool> + Send + '_;

    fn live_thread_info(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<LiveThreadInfo>> + Send + '_;

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<LiveThreadSnapshot>> + Send + '_;

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<ThreadConfigSnapshot>> + Send + '_;

    fn live_thread_config_refresh_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<LiveThreadConfigRefreshSnapshot>> + Send + '_;

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> impl Future<Output = CodexResult<bool>> + Send + '_;
}

/// Feedback collection surface for live thread metadata without exposing handles.
///
/// Implementations own any provider-specific lookup required to gather copied
/// thread ids, rollout paths, and session metadata used by feedback uploads.
pub trait LiveThreadFeedbackRuntime: Send + Sync {
    /// List `thread_id` plus all known live/persisted descendants in its agent subtree.
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<Vec<ThreadId>>> + Send + '_;

    /// Return the guardian trunk rollout path for a specific live thread.
    fn thread_guardian_trunk_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<Option<PathBuf>>> + Send + '_;

    /// Return the session source applied to newly created live threads.
    fn session_source(&self) -> SessionSource;
}

/// Skill watch path resolution surface without exposing concrete thread handles.
///
/// Implementations return copied watch path data for a live thread. Listener
/// setup should fall back independently if this resolution fails.
pub trait LiveThreadSkillWatchRuntime: Send + Sync {
    /// Return file-system paths that should be watched for skill changes.
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<Vec<SkillWatchPath>>> + Send + '_;
}

/// Usage read surface without exposing concrete thread handles.
///
/// Implementations return copied usage snapshots for live threads. Persisted
/// history and live turn merge behavior should stay with their own runtimes.
pub trait LiveThreadUsageRuntime: Send + Sync {
    /// Return the complete token usage snapshot for a specific live thread.
    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<Option<TokenUsageInfo>>> + Send + '_;

    /// Return the context usage snapshot for a specific live thread.
    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<ThreadContextUsage>> + Send + '_;
}

/// Goal runtime side-effect surface without exposing concrete thread handles.
///
/// App-server owns the persisted goal mutation. Implementations should only
/// prepare or apply live runtime effects around that externally persisted fact.
pub trait LiveThreadGoalRuntime: Send + Sync {
    /// Prepare a specific live thread for an externally persisted goal mutation.
    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Apply runtime effects for an externally persisted goal set/update.
    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Apply runtime effects for an externally persisted goal clear.
    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Restore goal runtime state after a caller has replayed or resumed a thread.
    fn apply_thread_goal_resume_runtime_effects(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Continue an active goal for a specific live thread if that thread is idle.
    fn continue_thread_active_goal_if_idle(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;
}

/// Out-of-band elicitation pause counter surface for live threads.
///
/// Implementations own the live counter state transitions. Callers should use
/// the returned count to report paused state instead of deriving or mutating
/// session pause state directly.
pub trait LiveThreadElicitationRuntime: Send + Sync {
    /// Increment the out-of-band elicitation pause counter for a live thread.
    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<u64>> + Send + '_;

    /// Decrement the out-of-band elicitation pause counter for a live thread.
    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> impl Future<Output = CodexResult<u64>> + Send + '_;
}

/// Source for optional persistent thread state runtime owned by the live thread manager.
///
/// Consumers that need spawn-edge or thread metadata persistence should depend
/// on the `state-api` trait returned here instead of reaching through a
/// concrete live thread handle to find the state database implementation.
pub trait LiveThreadStateRuntimeSource: Send + Sync {
    fn thread_state_runtime(&self) -> Option<SharedStateDbRuntime>;
}

/// Minimal command/status surface for a live thread.
///
/// Implementations should delegate to the underlying session runtime instead of
/// reimplementing turn or pending-input behavior at the thread layer.
pub trait LiveThreadHandle: SessionCommandHandle + Send + Sync {
    /// Return the immutable session configured event captured when the thread started.
    fn session_configured(&self) -> SessionConfiguredEvent;

    /// Receive the next core event emitted by this live thread.
    fn next_event(&self) -> impl Future<Output = CodexResult<Event>> + Send + '_;

    /// Submit an operation to this thread's session.
    fn submit_thread_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Return the latest agent lifecycle status for this thread.
    fn agent_status(&self) -> impl Future<Output = AgentStatus> + Send + '_;

    /// Return the coarse runtime status used by thread tree clients.
    fn runtime_thread_status(&self) -> impl Future<Output = ThreadRuntimeStatus> + Send + '_;

    /// Return whether a feature is enabled for this live thread.
    fn feature_enabled(&self, feature: Feature) -> bool;

    /// Return the current copied configuration snapshot for this live thread.
    fn config_snapshot(&self) -> impl Future<Output = ThreadConfigSnapshot> + Send + '_;

    /// Return the guardian trunk rollout path currently associated with this live thread.
    fn guardian_trunk_rollout_path(&self) -> impl Future<Output = Option<PathBuf>> + Send + '_;

    /// Set per-client metadata used by turn/runtime behavior.
    fn set_app_server_client_info(
        &self,
        info: AppServerClientInfo,
    ) -> impl Future<Output = ConstraintResult<()>> + Send + '_;

    /// Validate turn context overrides without committing them.
    fn validate_turn_context_overrides(
        &self,
        overrides: CodexThreadTurnContextOverrides,
    ) -> impl Future<Output = ConstraintResult<()>> + Send + '_;

    /// Return the complete token usage snapshot currently cached for this live thread.
    fn token_usage_info(&self) -> impl Future<Output = Option<TokenUsageInfo>> + Send + '_;

    /// Return a context usage snapshot computed from this live thread's current history.
    fn thread_context_usage(&self) -> impl Future<Output = ThreadContextUsage> + Send + '_;

    /// Return the live persisted history owned by this thread runtime.
    fn load_history(
        &self,
        include_archived: bool,
    ) -> impl Future<Output = ThreadStoreResult<StoredThreadHistory>> + Send + '_;

    /// Return the live persisted thread metadata and optional history owned by this runtime.
    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> impl Future<Output = ThreadStoreResult<StoredThread>> + Send + '_;

    /// Request shutdown and wait until the runtime has terminated.
    fn shutdown_and_wait(&self) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Wait until the runtime terminates without issuing a shutdown request.
    fn wait_until_terminated(&self) -> impl Future<Output = ()> + Send + '_;

    /// Prepare the live runtime for an externally persisted goal mutation.
    fn prepare_external_goal_mutation(&self) -> impl Future<Output = ()> + Send + '_;

    /// Apply runtime effects after a caller has replayed or resumed this thread.
    fn apply_goal_resume_runtime_effects(
        &self,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Continue an active goal if the thread is currently idle.
    fn continue_active_goal_if_idle(&self) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Apply runtime effects after an externally persisted goal set/update.
    fn apply_external_goal_set(
        &self,
        external_set: ExternalGoalSet,
    ) -> impl Future<Output = ()> + Send + '_;

    /// Apply runtime effects after an externally persisted goal clear.
    fn apply_external_goal_clear(&self) -> impl Future<Output = ()> + Send + '_;

    /// Increment the out-of-band elicitation pause counter.
    fn increment_out_of_band_elicitation_count(
        &self,
    ) -> impl Future<Output = CodexResult<u64>> + Send + '_;

    /// Decrement the out-of-band elicitation pause counter.
    fn decrement_out_of_band_elicitation_count(
        &self,
    ) -> impl Future<Output = CodexResult<u64>> + Send + '_;
}
