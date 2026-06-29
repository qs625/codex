//! Core-independent live session operation API.
//!
//! These contracts are now owned by `thread-service-api` as part of the unified
//! live thread runtime boundary. Concrete session loop implementations still
//! live in runtime crates and implement these traits by adapting their
//! internal state.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_file_system::FileSystemSandboxContext;
use codex_protocol::ThreadId;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::approvals::NetworkApprovalContext;
use codex_protocol::approvals::NetworkPolicyAmendment;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::Submission;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::W3cTraceContext;
use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
use codex_sandboxing_api::ResolvedExecCommandEnvironment;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_sandboxing_api::ToolSandboxContext;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_types::DiscoverableTool;
use codex_tool_types::FunctionCallError;
use codex_tool_types::RequestPluginInstallElicitationRequest;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub use codex_runtime_capability_api::ThreadCapability;

#[path = "pending_input.rs"]
mod pending_input;

pub use pending_input::PendingInputItem;

/// Boxed future returned by object-safe session capability traits.
pub type SessionCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type SharedToolTurnDiffTracker = Arc<Mutex<crate::TurnDiffTracker>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewRejectionRecord {
    pub rationale: String,
    pub source: codex_protocol::protocol::GuardianAssessmentDecisionSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAssessmentRecord {
    pub risk_level: codex_protocol::protocol::GuardianRiskLevel,
    pub user_authorization: codex_protocol::protocol::GuardianUserAuthorization,
    pub outcome: codex_protocol::protocol::GuardianAssessmentOutcome,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewRuntimeError {
    PromptBuild { message: String },
    Session { message: String },
    Parse { message: String },
    Timeout,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewRuntimeOutcome {
    Completed(ReviewAssessmentRecord),
    Error(ReviewRuntimeError),
}

#[derive(Debug)]
pub struct ReviewRuntimeResult {
    pub outcome: ReviewRuntimeOutcome,
    pub analytics_result: codex_analytics_api::GuardianReviewAnalyticsResult,
}

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct HookToolName {
    name: String,
    matcher_aliases: Vec<String>,
}

impl HookToolName {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            matcher_aliases: Vec::new(),
        }
    }

    pub fn bash() -> Self {
        Self::new("Bash")
    }

    pub fn apply_patch() -> Self {
        Self {
            name: "apply_patch".to_string(),
            matcher_aliases: vec!["Write".to_string(), "Edit".to_string()],
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn matcher_aliases(&self) -> &[String] {
        &self.matcher_aliases
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequestPayload {
    pub tool_name: HookToolName,
    pub tool_input: serde_json::Value,
}

impl PermissionRequestPayload {
    pub fn bash(command: String, description: Option<String>) -> Self {
        let mut tool_input = serde_json::Map::new();
        tool_input.insert("command".to_string(), serde_json::Value::String(command));
        if let Some(description) = description {
            tool_input.insert(
                "description".to_string(),
                serde_json::Value::String(description),
            );
        }

        Self {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::Value::Object(tool_input),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPermissionGrants {
    pub session: Option<AdditionalPermissionProfile>,
    pub turn: Option<AdditionalPermissionProfile>,
}

#[derive(Clone, Default, Debug)]
pub struct ApprovalStore {
    map: HashMap<String, ReviewDecision>,
}

impl ApprovalStore {
    pub fn get<K>(&self, key: &K) -> Option<ReviewDecision>
    where
        K: Serialize,
    {
        let key = serde_json::to_string(key).ok()?;
        self.map.get(&key).cloned()
    }

    pub fn put<K>(&mut self, key: K, value: ReviewDecision)
    where
        K: Serialize,
    {
        if let Ok(key) = serde_json::to_string(&key) {
            self.map.insert(key, value);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPluginInstallContext {
    pub server_name: String,
    pub thread_id: String,
    pub turn_id: String,
    pub app_server_client_name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestPluginInstallElicitationOutcome {
    pub user_confirmed: bool,
}

pub struct AgentJobRunnerOptions<SpawnConfig> {
    pub max_concurrency: usize,
    pub spawn_config: SpawnConfig,
}

pub enum AgentJobSpawnWorkerError {
    LimitReached,
    Other(String),
}

pub type ToolTelemetryTags = Vec<(&'static str, String)>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDiscoveryContext {
    pub home_root: PathBuf,
    #[serde(default)]
    pub project_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadAgentMode {
    Normal,
    Management,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadSpawnAgentForkMode {
    FullHistory,
    LastNTurns { last_n_turns: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSpawnAgentRequest {
    pub message: String,
    pub task_name: String,
    pub agent_type: Option<String>,
    pub cwd: Option<AbsolutePathBuf>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub service_tier: Option<String>,
    pub agent_mode: Option<ThreadAgentMode>,
    pub fork_mode: Option<ThreadSpawnAgentForkMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThreadSpawnAgentResult {
    WithNickname {
        task_name: String,
        nickname: Option<String>,
    },
    HiddenMetadata {
        task_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadWaitAgentReason {
    PendingMessage,
    MailboxMessage,
    FinalStatus,
    StatusUpdate,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadWaitAgentResult {
    pub target: String,
    pub agent_name: String,
    pub reason: ThreadWaitAgentReason,
    pub timed_out: bool,
    pub status: AgentStatus,
    pub message_operation: Option<String>,
    pub message_author: Option<String>,
    pub message_excerpt: Option<String>,
    pub waited_ms: i64,
    pub initial_timeout_ms: i64,
    pub current_timeout_ms: i64,
    pub hard_cap_timeout_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreToolUsePayload {
    pub tool_name: HookToolName,
    pub tool_input: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostToolUsePayload {
    pub tool_name: HookToolName,
    pub tool_use_id: String,
    pub tool_input: serde_json::Value,
    pub tool_response: serde_json::Value,
}

pub enum PreToolUseHookOutcome {
    Continue {
        updated_input: Option<serde_json::Value>,
    },
    Blocked(String),
}

#[derive(Default)]
pub struct PostToolUseHookOutcome {
    pub replacement_text: Option<String>,
}

/// Session-owned trace lifecycle for one tool dispatch.
///
/// Tool services call this handle to finish trace records without depending on
/// the concrete rollout trace implementation owned by the session runtime.
pub trait ToolSessionDispatchTrace: Send {
    /// Record a completed tool dispatch.
    fn record_completed(&self, call_id: &str, payload: &ToolPayload, result: &dyn ToolOutput);

    /// Record a failed tool dispatch.
    fn record_failed(&self, error: &FunctionCallError);
}

/// Session-owned side effects required by tool dispatch.
///
/// Tool services own dispatch ordering and result shaping. Implementations of
/// this capability own session state mutations, hook execution, and goal
/// accounting for the active turn. Tool-domain code should hold this contract
/// as a weak trait object rather than depending on a concrete session runtime.
/// Turn-owned data required by session-owned tool dispatch capabilities.
///
/// Concrete turn runtimes implement this trait directly. Tool services receive
/// it only as an API-level view, so session capability traits do not need to be
/// generic over concrete turn types.
pub trait ThreadTurnCapability: Send + Sync + 'static {
    /// Implementation-owned typed view for the session service that created the
    /// turn. External services should not downcast this value.
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    /// Implementation-owned erased `Arc` for owner-side downcasting.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Telemetry sink for tool dispatch spans and handler results.
    fn tool_dispatch_telemetry(&self) -> SharedSessionTelemetry;

    /// Base tags applied to tool result telemetry for this turn.
    fn base_tool_result_tags(&self) -> ToolTelemetryTags;

    /// Runtime turn identifier used in rollout trace records.
    fn rollout_turn_id(&self) -> String;

    /// Read the current thread goal visible to this turn.
    fn get_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>>;

    /// Create a new active thread goal from this turn.
    fn create_thread_goal<'a>(
        &'a self,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Mark the current thread goal complete from this turn.
    fn complete_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Return immutable workspace discovery inputs visible to this turn.
    fn discovery_context(&self) -> ThreadDiscoveryContext;

    /// Active turn approval policy.
    fn approval_policy(&self) -> AskForApproval;

    /// Configured approval reviewer for the active turn.
    fn approvals_reviewer(&self) -> ApprovalsReviewer;

    /// Active turn permission profile.
    fn permission_profile(&self) -> PermissionProfile;

    /// Filesystem sandbox policy for the active turn.
    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy;

    /// Windows sandbox level for the active turn.
    fn windows_sandbox_level(&self) -> WindowsSandboxLevel;

    /// Shared sandbox context for tool execution in the active turn.
    fn tool_sandbox_context(&self) -> ToolSandboxContext;

    /// Resolve one apply-patch execution environment for the active turn.
    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;

    /// Thread identifier that owns the active turn.
    fn thread_id(&self) -> ThreadId;

    /// Runtime turn identifier used by emitted items and event lifecycles.
    fn runtime_turn_id_str(&self) -> &str;

    /// Output truncation policy for tool-visible text.
    fn truncation_policy(&self) -> TruncationPolicy;

    /// Collaboration mode configured for the active turn.
    fn collaboration_mode_kind(&self) -> codex_protocol::config_types::ModeKind;

    /// Session cwd used for relative-path argument normalization.
    fn legacy_cwd(&self) -> AbsolutePathBuf;

    /// Whether the active turn belongs to a non-root agent thread.
    fn is_non_root_agent(&self) -> bool;

    /// Whether the current client supports image input items.
    fn supports_image_input(&self) -> bool;

    /// Collaboration mode currently configured on the owning session.
    fn session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, codex_protocol::config_types::ModeKind>;

    /// Emit one typed event for the active turn.
    fn emit_event<'a>(&'a self, event: EventMsg) -> SessionCapabilityFuture<'a, ()>;

    /// Request additional permissions from the client/runtime.
    fn request_permissions<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_permissions::RequestPermissionsResponse>,
    >;

    /// Request structured user input from the client/runtime.
    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_user_input::RequestUserInputResponse>,
    >;

    /// Dispatch one dynamic tool call through the active thread runtime.
    fn request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<codex_protocol::dynamic_tools::DynamicToolResponse>>;
}

impl<Turn> ThreadTurnCapability for Arc<Turn>
where
    Turn: ThreadTurnCapability,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self.as_ref().as_any()
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn tool_dispatch_telemetry(&self) -> SharedSessionTelemetry {
        self.as_ref().tool_dispatch_telemetry()
    }

    fn base_tool_result_tags(&self) -> ToolTelemetryTags {
        self.as_ref().base_tool_result_tags()
    }

    fn rollout_turn_id(&self) -> String {
        self.as_ref().rollout_turn_id()
    }

    fn get_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        self.as_ref().get_thread_goal()
    }

    fn create_thread_goal<'a>(
        &'a self,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref().create_thread_goal(objective, token_budget)
    }

    fn complete_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref().complete_thread_goal()
    }

    fn discovery_context(&self) -> ThreadDiscoveryContext {
        self.as_ref().discovery_context()
    }

    fn approval_policy(&self) -> AskForApproval {
        self.as_ref().approval_policy()
    }

    fn approvals_reviewer(&self) -> ApprovalsReviewer {
        self.as_ref().approvals_reviewer()
    }

    fn permission_profile(&self) -> PermissionProfile {
        self.as_ref().permission_profile()
    }

    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        self.as_ref().file_system_sandbox_policy()
    }

    fn windows_sandbox_level(&self) -> WindowsSandboxLevel {
        self.as_ref().windows_sandbox_level()
    }

    fn tool_sandbox_context(&self) -> ToolSandboxContext {
        self.as_ref().tool_sandbox_context()
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        self.as_ref()
            .resolve_apply_patch_environment(environment_id)
    }

    fn thread_id(&self) -> ThreadId {
        self.as_ref().thread_id()
    }

    fn runtime_turn_id_str(&self) -> &str {
        self.as_ref().runtime_turn_id_str()
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.as_ref().truncation_policy()
    }

    fn collaboration_mode_kind(&self) -> codex_protocol::config_types::ModeKind {
        self.as_ref().collaboration_mode_kind()
    }

    fn legacy_cwd(&self) -> AbsolutePathBuf {
        self.as_ref().legacy_cwd()
    }

    fn is_non_root_agent(&self) -> bool {
        self.as_ref().is_non_root_agent()
    }

    fn supports_image_input(&self) -> bool {
        self.as_ref().supports_image_input()
    }

    fn session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, codex_protocol::config_types::ModeKind> {
        self.as_ref().session_collaboration_mode()
    }

    fn emit_event<'a>(&'a self, event: EventMsg) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref().emit_event(event)
    }

    fn request_permissions<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_permissions::RequestPermissionsResponse>,
    > {
        self.as_ref()
            .request_permissions(call_id, args, cancellation_token)
    }

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_user_input::RequestUserInputResponse>,
    > {
        self.as_ref().request_user_input(call_id, args)
    }

    fn request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<codex_protocol::dynamic_tools::DynamicToolResponse>>
    {
        self.as_ref()
            .request_dynamic_tool(call_id, tool_name, arguments)
    }
}

/// Boxed future returned by thread domain service APIs.
pub type ThreadServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Thread domain service API.
///
/// This trait is the owner boundary for thread-specific business operations
/// such as multi-agent lifecycle actions and thread-owned model/display item
/// emission. Callers should depend on this trait instead of concrete session
/// runtime types.
pub trait ThreadServiceApi: Send + Sync + 'static {
    fn spawn_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>>;

    fn followup_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        message: String,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>>;

    fn wait_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadWaitAgentResult, FunctionCallError>>;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>>;
}

/// Thread-runtime-owned session reference passed into tool service dispatch.
///
/// This is still a migration-time bridge while tool handlers are being moved
/// away from concrete session/runtime generics. The owner stays in
/// `thread-service-api` because the referenced runtime object belongs to the
/// thread domain, not the tool domain.
pub trait ToolServiceSessionRef: Send + Sync + 'static {
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl<T> ToolServiceSessionRef for T
where
    T: Send + Sync + 'static,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Thread-runtime-owned turn reference passed into tool service dispatch.
///
/// Like [`ToolServiceSessionRef`], this is a temporary bridge that keeps
/// runtime ownership on the thread side while the tool domain is being
/// converted from generic handlers to capability-driven dyn dispatch.
pub trait ToolServiceTurnRef: Send + Sync + 'static {
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

impl<T> ToolServiceTurnRef for T
where
    T: Send + Sync + 'static,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

pub trait ThreadSessionCapability: Send + Sync + 'static {
    /// Implementation-owned typed view for the session service that created
    /// this capability. External services should not downcast this value.
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    /// Implementation-owned erased `Arc` for owner-side downcasting.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Thread identifier owned by this session runtime.
    fn conversation_id(&self) -> ThreadId;

    /// Telemetry sink for tool dispatch spans and handler results.
    fn tool_dispatch_telemetry(&self, turn: &dyn ThreadTurnCapability) -> SharedSessionTelemetry;

    /// Base tags applied to tool result telemetry for the active turn.
    fn base_tool_result_tags(&self, turn: &dyn ThreadTurnCapability) -> ToolTelemetryTags;

    /// Record that a model-visible tool call started.
    fn record_tool_call_started<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Start trace lifecycle for one tool dispatch.
    fn start_tool_dispatch_trace(
        &self,
        turn: &dyn ThreadTurnCapability,
        call_id: &str,
        tool_name: &ToolName,
        source: &ToolCallSource,
        payload: &ToolPayload,
    ) -> Box<dyn ToolSessionDispatchTrace>;

    /// Run pre-tool hooks for a tool invocation.
    fn run_pre_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        payload: PreToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PreToolUseHookOutcome>;

    /// Run post-tool hooks for a completed tool invocation.
    fn run_post_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        payload: PostToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PostToolUseHookOutcome>;

    /// Emit memory/read telemetry derived from a completed tool invocation.
    fn emit_tool_read_metric<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a ToolName,
        payload: &'a ToolPayload,
        success: bool,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Account a completed tool call against active goal runtime state.
    fn account_goal_tool_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a ToolName,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Emit one typed event for the active turn.
    fn emit_event<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        event: EventMsg,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Record model-visible items and emit display events for the active turn.
    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        items: Vec<ResponseItem>,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Sandbox runtime shared by the owning session.
    fn sandbox_runtime(&self) -> SharedSandboxRuntime;

    /// Whether strict auto review is enabled for the active turn.
    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> SessionCapabilityFuture<'a, bool>;

    /// Return the current active turn runtime when one exists.
    fn active_turn_runtime<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Option<Arc<dyn ThreadRuntimeCapability>>>;

    /// Remove and return one stored guardian rejection record.
    fn take_review_rejection<'a>(
        &'a self,
        review_id: &'a str,
    ) -> SessionCapabilityFuture<'a, Option<ReviewRejectionRecord>>;

    /// Store or clear one guardian rejection record by review id.
    fn set_review_rejection<'a>(
        &'a self,
        review_id: String,
        rejection: Option<ReviewRejectionRecord>,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Record guardian review analytics at the session owner boundary.
    fn track_review_analytics<'a>(
        &'a self,
        tracking: codex_analytics_api::GuardianReviewTrackContext,
        result: codex_analytics_api::GuardianReviewAnalyticsResult,
        completed_at_ms: u64,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Run the locked-down guardian review session and return the parsed outcome.
    fn run_review_session<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        request: serde_json::Value,
        retry_reason: Option<String>,
    ) -> SessionCapabilityFuture<'a, ReviewRuntimeResult>;

    /// Record one non-denial guardian review result for circuit-breaker state.
    fn record_review_non_rejection<'a>(
        &'a self,
        turn_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Record one denial guardian review result and apply any interrupt side effects.
    fn record_review_rejection<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        turn_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit an exec approval prompt and await the resulting review decision.
    #[allow(clippy::too_many_arguments)]
    fn request_command_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> SessionCapabilityFuture<'a, ReviewDecision>;

    /// Emit an apply-patch approval prompt and await the resulting review decision.
    fn request_patch_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        changes: HashMap<PathBuf, FileChange>,
        reason: Option<String>,
        grant_root: Option<PathBuf>,
    ) -> SessionCapabilityFuture<'a, ReviewDecision>;

    /// Read one cached approval decision by serialized approval key.
    fn cached_approval_decision<'a>(
        &'a self,
        key: String,
    ) -> SessionCapabilityFuture<'a, Option<ReviewDecision>>;

    /// Persist an `ApprovedForSession` decision for serialized approval keys.
    fn cache_approval_decision<'a>(
        &'a self,
        keys: Vec<String>,
        decision: ReviewDecision,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Record approval request telemetry for the specified tool.
    fn record_approval_request_telemetry<'a>(
        &'a self,
        tool_name: &'a str,
        decision: &'a ReviewDecision,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Persist one network policy amendment in runtime and execpolicy state.
    fn persist_network_policy_amendment<'a>(
        &'a self,
        amendment: &'a NetworkPolicyAmendment,
        network_approval_context: &'a NetworkApprovalContext,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Record one model-visible message describing a persisted network policy amendment.
    fn record_network_policy_amendment_message<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        amendment: &'a NetworkPolicyAmendment,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Subscribe to out-of-band elicitation pause state for this session.
    fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool>;

    /// Run permission request hooks for one tool permission request.
    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> SessionCapabilityFuture<'a, Option<codex_hooks_api::PermissionRequestDecision>>;

    /// Effective tool permission grants cached on the current session.
    fn tool_permission_grants<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, ToolPermissionGrants>;
}

/// Common turn-runtime capability shared by tool services that need active turn
/// identity, image-detail support, or filesystem-backed environment access.
pub trait ThreadRuntimeCapability: ThreadCapability + ThreadTurnCapability {
    /// Runtime turn identifier used by trace records and emitted items.
    fn runtime_turn_id(&self) -> String;

    /// Whether the active turn may request original-detail images.
    fn can_request_original_image_detail(&self) -> bool;

    /// Resolve one optional turn environment into a filesystem-backed runtime view.
    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;

    /// Build a filesystem sandbox context for the selected cwd.
    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext;

    /// Return the single local environment cwd used for agent-job CSV input/output.
    fn single_local_environment_cwd(&self) -> Result<AbsolutePathBuf, FunctionCallError>;

    /// Default runtime timeout for each spawned agent-job worker.
    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64>;

    /// Whether approvals for this turn route through guardian.
    fn routes_approval_to_guardian(&self) -> bool;

    /// Shell environment policy configured for this turn.
    fn shell_environment_policy(&self) -> codex_protocol::config_types::ShellEnvironmentPolicy;

    /// Unified exec shell mode configured for this turn.
    fn unified_exec_shell_mode(&self) -> codex_tool_config::UnifiedExecShellMode;

    /// Whether login shells are allowed for this turn.
    fn allow_login_shell(&self) -> bool;

    /// Active managed network runtime for this turn.
    fn active_network(&self) -> Option<codex_network_proxy_api::SharedNetworkProxyRuntime>;

    /// Emit a unified exec tty metric for this turn.
    fn emit_unified_exec_tty_metric(&self, tty: bool);

    /// Resolve exec-command environment for this turn.
    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError>;

    /// Execute one MCP tool call in the active turn runtime.
    fn call_mcp_tool(
        &self,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> SessionCapabilityFuture<'_, (CallToolResult, serde_json::Value)>;

    /// Whether MCP tool outputs may request original-detail images.
    fn mcp_original_image_detail_supported(&self) -> bool;

    /// Truncation policy used for MCP resource tool outputs.
    fn mcp_truncation_policy(&self) -> TruncationPolicy;

    /// List MCP resources for one server in the active turn context.
    fn list_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>>;

    /// List MCP resources for all servers visible to the active turn.
    fn list_all_resources(&self) -> SessionCapabilityFuture<'_, HashMap<String, Vec<Resource>>>;

    /// List MCP resource templates for one server in the active turn context.
    fn list_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>>;

    /// List MCP resource templates for all servers visible to the active turn.
    fn list_all_resource_templates(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<ResourceTemplate>>>;

    /// Read one MCP resource in the active turn context.
    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>>;

    /// Emit the started lifecycle event for one MCP resource tool call.
    fn emit_mcp_resource_tool_call_begin<'a>(
        &'a self,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit the completed lifecycle event for one MCP resource tool call.
    fn emit_mcp_resource_tool_call_end<'a>(
        &'a self,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: std::time::Duration,
        result: Result<CallToolResult, String>,
    ) -> SessionCapabilityFuture<'a, ()>;
}

impl<Turn> ThreadRuntimeCapability for Arc<Turn>
where
    Turn: ThreadRuntimeCapability,
{
    fn runtime_turn_id(&self) -> String {
        self.as_ref().runtime_turn_id()
    }

    fn can_request_original_image_detail(&self) -> bool {
        self.as_ref().can_request_original_image_detail()
    }

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        self.as_ref().resolve_environment(environment_id)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        self.as_ref()
            .file_system_sandbox_context(additional_permissions, cwd)
    }

    fn single_local_environment_cwd(&self) -> Result<AbsolutePathBuf, FunctionCallError> {
        self.as_ref().single_local_environment_cwd()
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        self.as_ref().default_agent_job_max_runtime_seconds()
    }

    fn routes_approval_to_guardian(&self) -> bool {
        self.as_ref().routes_approval_to_guardian()
    }

    fn shell_environment_policy(&self) -> codex_protocol::config_types::ShellEnvironmentPolicy {
        self.as_ref().shell_environment_policy()
    }

    fn unified_exec_shell_mode(&self) -> codex_tool_config::UnifiedExecShellMode {
        self.as_ref().unified_exec_shell_mode()
    }

    fn allow_login_shell(&self) -> bool {
        self.as_ref().allow_login_shell()
    }

    fn active_network(&self) -> Option<codex_network_proxy_api::SharedNetworkProxyRuntime> {
        self.as_ref().active_network()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.as_ref().emit_unified_exec_tty_metric(tty);
    }

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError> {
        self.as_ref()
            .resolve_exec_command_environment(environment_id, workdir)
    }

    fn call_mcp_tool(
        &self,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> SessionCapabilityFuture<'_, (CallToolResult, serde_json::Value)> {
        self.as_ref()
            .call_mcp_tool(call_id, server, tool_name, hook_tool_name, arguments)
    }

    fn mcp_original_image_detail_supported(&self) -> bool {
        self.as_ref().mcp_original_image_detail_supported()
    }

    fn mcp_truncation_policy(&self) -> TruncationPolicy {
        self.as_ref().mcp_truncation_policy()
    }

    fn list_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>> {
        self.as_ref().list_resources(server, params)
    }

    fn list_all_resources(&self) -> SessionCapabilityFuture<'_, HashMap<String, Vec<Resource>>> {
        self.as_ref().list_all_resources()
    }

    fn list_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>> {
        self.as_ref().list_resource_templates(server, params)
    }

    fn list_all_resource_templates(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<ResourceTemplate>>> {
        self.as_ref().list_all_resource_templates()
    }

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>> {
        self.as_ref().read_resource(server, params)
    }

    fn emit_mcp_resource_tool_call_begin<'a>(
        &'a self,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref()
            .emit_mcp_resource_tool_call_begin(call_id, invocation)
    }

    fn emit_mcp_resource_tool_call_end<'a>(
        &'a self,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: std::time::Duration,
        result: Result<CallToolResult, String>,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref()
            .emit_mcp_resource_tool_call_end(call_id, invocation, duration, result)
    }
}

/// Turn-owned capability for apply-patch streamed argument diff events.
pub trait ApplyPatchDiffContext: Send + Sync + 'static {
    fn apply_patch_streaming_events_enabled(&self) -> bool;
}

impl<Turn> ApplyPatchDiffContext for Arc<Turn>
where
    Turn: ApplyPatchDiffContext,
{
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.as_ref().apply_patch_streaming_events_enabled()
    }
}


/// Session-owned code-mode capability consumed by code-mode tool handlers.
///
/// Implementations own the runtime service, persisted code-mode values, and
/// rollout trace updates for code cell lifecycle events. Tool-domain code calls
/// this contract through [`SessionCodeModeHost`] instead of depending on a
/// broader runtime host implementation.
pub trait SessionCodeModeCaller: Send + Sync + 'static {
    /// Snapshot persisted code-mode values visible to the current session.
    fn code_mode_stored_values(
        &self,
    ) -> impl Future<Output = HashMap<String, serde_json::Value>> + Send + '_;

    /// Replace the persisted code-mode values for the current session.
    fn code_mode_replace_stored_values(
        &self,
        values: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = ()> + Send + '_;

    /// Allocate a new runtime cell id before starting a code-mode execution.
    fn code_mode_allocate_cell_id(&self) -> String;

    /// Execute one code-mode request.
    fn code_mode_execute(
        &self,
        request: ExecuteRequest,
    ) -> impl Future<Output = Result<RuntimeResponse, String>> + Send + '_;

    /// Wait for one bounded code-mode runtime update.
    fn code_mode_wait(
        &self,
        request: WaitRequest,
    ) -> impl Future<Output = Result<WaitOutcome, String>> + Send + '_;

    /// Record the start of one code cell trace.
    fn record_code_mode_cell_started(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    );

    /// Record the first runtime response for one code cell trace.
    fn record_code_mode_cell_initial_response(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    );

    /// Record the terminal runtime response for one code cell trace.
    fn record_code_mode_cell_ended(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    );
}

impl<Session> SessionCodeModeCaller for Arc<Session>
where
    Session: SessionCodeModeCaller,
{
    fn code_mode_stored_values(
        &self,
    ) -> impl Future<Output = HashMap<String, serde_json::Value>> + Send + '_ {
        self.as_ref().code_mode_stored_values()
    }

    fn code_mode_replace_stored_values(
        &self,
        values: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = ()> + Send + '_ {
        self.as_ref().code_mode_replace_stored_values(values)
    }

    fn code_mode_allocate_cell_id(&self) -> String {
        self.as_ref().code_mode_allocate_cell_id()
    }

    fn code_mode_execute(
        &self,
        request: ExecuteRequest,
    ) -> impl Future<Output = Result<RuntimeResponse, String>> + Send + '_ {
        self.as_ref().code_mode_execute(request)
    }

    fn code_mode_wait(
        &self,
        request: WaitRequest,
    ) -> impl Future<Output = Result<WaitOutcome, String>> + Send + '_ {
        self.as_ref().code_mode_wait(request)
    }

    fn record_code_mode_cell_started(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    ) {
        self.as_ref().record_code_mode_cell_started(
            turn,
            runtime_cell_id,
            model_visible_call_id,
            source_js,
        );
    }

    fn record_code_mode_cell_initial_response(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        self.as_ref()
            .record_code_mode_cell_initial_response(turn, runtime_cell_id, response);
    }

    fn record_code_mode_cell_ended(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        self.as_ref()
            .record_code_mode_cell_ended(turn, runtime_cell_id, response);
    }
}

/// Global request-plugin-install service API consumed by tool handlers.
///
/// Implementations own discoverable-tool lookup, confirmation elicitation, and
/// post-install verification. Handlers should depend on this service API
/// instead of calling install logic through the session object directly.
pub trait RequestPluginInstallApi: Send + Sync + 'static {
    /// Build the model-visible request context for this turn.
    fn request_plugin_install_context(
        &self,
        capability: &dyn ThreadCapability,
    ) -> RequestPluginInstallContext;

    /// List discoverable plugin/connector install candidates for this turn.
    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Vec<DiscoverableTool>, FunctionCallError>>;

    /// Ask the client to confirm one plugin/connector install request.
    fn request_plugin_install_elicitation<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, RequestPluginInstallElicitationOutcome>;

    /// Verify and apply session state after a confirmed install request.
    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, bool>;
}

impl<Service> RequestPluginInstallApi for Arc<Service>
where
    Service: RequestPluginInstallApi,
{
    fn request_plugin_install_context(
        &self,
        capability: &dyn ThreadCapability,
    ) -> RequestPluginInstallContext {
        self.as_ref().request_plugin_install_context(capability)
    }

    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Vec<DiscoverableTool>, FunctionCallError>> {
        self.as_ref()
            .list_request_plugin_install_discoverable_tools(capability)
    }

    fn request_plugin_install_elicitation<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, RequestPluginInstallElicitationOutcome> {
        self.as_ref()
            .request_plugin_install_elicitation(capability, call_id, request, tool)
    }

    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, bool> {
        self.as_ref()
            .complete_request_plugin_install_if_ready(capability, tool)
    }
}


/// Session-owned agent-job capability consumed by CSV agent-job tools.
///
/// Implementations own state DB access, subagent spawning, worker lifecycle,
/// and status subscriptions.
pub trait SessionAgentJobCaller: Send + Sync + 'static {
    type SpawnConfig: Clone + Send + Sync + 'static;

    /// Return the state DB runtime if this session supports agent jobs.
    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime>;

    /// Return the current thread id as a string for result attribution.
    fn agent_job_conversation_id_string(&self) -> String;

    /// Build runner options and the spawn config for agent-job workers.
    fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        requested_concurrency: Option<usize>,
    ) -> impl Future<Output = Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError>>
    + Send
    + '_;

    /// Spawn one agent-job worker.
    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a dyn ThreadRuntimeCapability,
        spawn_config: Self::SpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> impl Future<Output = Result<ThreadId, AgentJobSpawnWorkerError>> + Send + 'a;

    /// Shutdown one worker thread.
    fn shutdown_agent_job_worker(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send;

    /// Read one worker status.
    fn get_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = AgentStatus> + Send;

    /// Subscribe to worker status changes.
    fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = Option<watch::Receiver<AgentStatus>>> + Send;
}

impl<Session> SessionAgentJobCaller for Arc<Session>
where
    Session: SessionAgentJobCaller,
{
    type SpawnConfig = <Session as SessionAgentJobCaller>::SpawnConfig;

    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime> {
        self.as_ref().agent_job_state_db()
    }

    fn agent_job_conversation_id_string(&self) -> String {
        self.as_ref().agent_job_conversation_id_string()
    }

    fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        requested_concurrency: Option<usize>,
    ) -> impl Future<Output = Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError>>
    + Send
    + '_ {
        Arc::clone(self.as_ref()).build_agent_job_runner_options(turn, requested_concurrency)
    }

    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a dyn ThreadRuntimeCapability,
        spawn_config: Self::SpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> impl Future<Output = Result<ThreadId, AgentJobSpawnWorkerError>> + Send + 'a {
        Arc::clone(self.as_ref()).spawn_agent_job_worker(turn, spawn_config, job_id, prompt)
    }

    fn shutdown_agent_job_worker(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send {
        Arc::clone(self.as_ref()).shutdown_agent_job_worker(thread_id)
    }

    fn get_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = AgentStatus> + Send {
        Arc::clone(self.as_ref()).get_agent_job_worker_status(thread_id)
    }

    fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> impl Future<Output = Option<watch::Receiver<AgentStatus>>> + Send {
        Arc::clone(self.as_ref()).subscribe_agent_job_worker_status(thread_id)
    }
}

/// Minimal command surface for an already-created live session.
///
/// Implementations are expected to enqueue operations onto the session's normal
/// turn loop. They should not bypass pending-input hooks, lifecycle events, or
/// status transitions owned by the concrete runtime.
pub trait SessionCommandHandle: Send + Sync {
    /// Submit a high-level operation and let the runtime assign the submission id.
    fn submit_op(&self, op: Op) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Submit a high-level operation with optional request trace context.
    fn submit_op_with_trace(
        &self,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;

    /// Submit a prebuilt submission with a caller-provided id.
    fn submit_with_id(
        &self,
        submission: Submission,
    ) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Request shutdown through the normal session operation queue.
    fn shutdown(&self) -> impl Future<Output = CodexResult<()>> + Send + '_;

    /// Append a model-visible conversation item outside the normal user-input path.
    ///
    /// Implementations should record the item through the same history/context
    /// path used by the live session runtime, including any display/event
    /// projection side effects owned by that runtime.
    fn append_conversation_item(
        &self,
        item: ResponseItem,
    ) -> impl Future<Output = CodexResult<String>> + Send + '_;
}

/// Read-only live status surface for a session.
pub trait SessionStatusHandle: Send + Sync {
    /// Return the latest lifecycle status observed by the session runtime.
    fn agent_status(&self) -> impl Future<Output = AgentStatus> + Send + '_;
}
