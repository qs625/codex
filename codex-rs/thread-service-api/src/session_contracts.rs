//! Core-independent live session operation API.
//!
//! These contracts are now owned by `thread-service-api` as part of the unified
//! live thread runtime boundary. Concrete session loop implementations still
//! live in runtime crates and implement these traits by adapting their
//! internal state.

use std::any::Any;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::ActiveEventSubscriptionTracker;
use crate::ThreadCreatedEvent;
use crate::ThreadRuntimeStatus;
use crate::ThreadShutdownReport;
use codex_agent_roles::AgentRoleConfig;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_connectors_api::AppInfo;
use codex_features::Features;
use codex_file_system::FileSystemSandboxContext;
use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
use codex_sandboxing_api::ResolvedExecCommandEnvironment;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_sandboxing_api::ToolSandboxContext;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use mcp_types::CodexAppsAuthContext;
use mcp_types::ElicitationResponse;
use mcp_types::ElicitationReviewerHandle;
use mcp_types::McpServerElicitationRequestParams;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::SandboxState;
use mcp_types::ToolInfo;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::Personality;
use protocol::config_types::WindowsSandboxLevel;
use protocol::error::Result as CodexResult;
use protocol::mcp::CallToolResult;
use protocol::mcp::ListResourceTemplatesResult;
use protocol::mcp::ListResourcesResult;
use protocol::mcp::PaginatedRequestParams;
use protocol::mcp::ReadResourceRequestParams;
use protocol::mcp::ReadResourceResult;
use protocol::mcp::RequestId;
use protocol::mcp::Resource;
use protocol::mcp::ResourceTemplate;
use protocol::models::ActivePermissionProfile;
use protocol::models::CommandExecutionNotificationKind;
use protocol::models::PermissionProfile;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AgentStatus;
use protocol::protocol::AskForApproval;
use protocol::protocol::EventMsg;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentContentPart;
use protocol::protocol::McpServerRefreshConfig;
use protocol::protocol::Op;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::SessionSource;
use protocol::protocol::Submission;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::W3cTraceContext;
use protocol::subscriptions::PersistedSubscription;
use serde::Deserialize;
use serde::Serialize;
use session_telemetry_api::SharedSessionTelemetry;
use state_api::ExternalGoalSet;
use state_api::SharedStateDbRuntime;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tool_types::FunctionCallError;
use tool_types::ToolCallSource;
use tool_types::ToolName;
use tool_types::ToolOutput;
use tool_types::ToolPayload;

use crate::NetworkApprovalSpec;
use crate::ResolvedExecCommand;
use crate::RuntimeShell;
use crate::ToolRuntimeNetworkApprovalHandle;
use crate::ToolRuntimeNetworkApprovalTrigger;

#[path = "pending_input.rs"]
mod pending_input;

pub use pending_input::PendingInputItem;

/// Common runtime capability shared by service APIs that need active-thread or
/// active-turn context during one tool dispatch.
///
/// Domain-specific service API crates should depend on this trait rather than
/// baking concrete runtime types such as `TurnContext` into their public API.
pub trait ThreadCapability: Send + Sync + 'static {
    /// Return the concrete runtime object behind this capability.
    fn as_any(&self) -> &(dyn Any + Send + Sync);
}

impl<T> ThreadCapability for Arc<T>
where
    T: ThreadCapability,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self.as_ref().as_any()
    }
}

/// Boxed future returned by object-safe session capability traits.
pub type SessionCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type SharedToolTurnDiffTracker = Arc<Mutex<crate::TurnDiffTracker>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutoApprovalSafetyOutcome {
    Ok,
    AskUser(String),
    SteerModel(String),
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
pub struct McpOAuthLoginParams {
    pub server_name: String,
    pub server_url: String,
    pub store_mode: codex_config_types::OAuthCredentialsStoreMode,
    pub http_headers: Option<HashMap<String, String>>,
    pub env_http_headers: Option<HashMap<String, String>>,
    pub scopes: Vec<String>,
    pub oauth_client_id: Option<String>,
    pub oauth_resource: Option<String>,
    pub callback_port: Option<u16>,
    pub callback_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadAppToolPolicy {
    pub enabled: bool,
    pub approval: codex_config_types::AppToolApproval,
}

pub struct AgentJobRunnerOptions<SpawnConfig> {
    pub max_concurrency: usize,
    pub spawn_config: SpawnConfig,
}

/// Opaque, owner-defined spawn config carried between agent-job API calls.
pub type AgentJobSpawnConfig = Arc<dyn Any + Send + Sync>;

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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ThreadSpawnAgentForkMode {
    FullHistory,
    LastNTurns { last_n_turns: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadSpawnAgentProvider {
    Native,
    CodexCli,
    ClaudeCli,
    Opencode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSpawnAgentRequest {
    pub message: String,
    pub task_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ThreadSpawnAgentProvider>,
    pub agent_type: Option<String>,
    pub cwd: Option<AbsolutePathBuf>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub service_tier: Option<String>,
    pub fork_mode: Option<ThreadSpawnAgentForkMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSpawnExternalAgentRequest {
    pub message: String,
    pub task_name: String,
    pub provider: ThreadSpawnAgentProvider,
    pub cwd: AbsolutePathBuf,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadFollowupTaskInput {
    pub message: String,
    #[serde(default)]
    pub content_parts: Vec<InterAgentContentPart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPollEventRequest {
    pub initial_timeout_ms: Option<i64>,
    pub hard_cap_timeout_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPollEventResult {
    pub timed_out: bool,
    pub source_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ThreadPollEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ThreadPollEvent>,
    pub waited_ms: i64,
    pub initial_timeout_ms: i64,
    pub current_timeout_ms: i64,
    pub hard_cap_timeout_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadPollEvent {
    InterAgentCommunication {
        communication: InterAgentCommunication,
    },
    CommandExecutionNotification {
        #[serde(rename = "commandItemId")]
        command_item_id: String,
        kind: CommandExecutionNotificationKind,
        message: String,
        output: Option<String>,
        #[serde(rename = "exitCode")]
        exit_code: Option<i32>,
        #[serde(rename = "createdAtMs")]
        created_at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPollEventTimeoutMetadata {
    pub initial_timeout_ms: i64,
    pub current_timeout_ms: i64,
    pub hard_cap_timeout_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCloseAgentResult {
    pub previous_status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListedAgent {
    pub agent_name: String,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub lifecycle_status: ThreadLifecycleStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListAgentsResult {
    pub agents: Vec<ThreadListedAgent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAgentDetails {
    pub agent_name: String,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub lifecycle_status: ThreadLifecycleStatus,
    pub last_task_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadAgentResult {
    pub agent: ThreadAgentDetails,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAgentRoleLoadResult {
    pub agent_role: String,
    pub effective: String,
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
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

    /// Whether streamed apply-patch preview events are enabled for this turn.
    fn apply_patch_streaming_events_enabled(&self) -> bool;

    /// Collaboration mode configured for the active turn.
    fn collaboration_mode_kind(&self) -> protocol::config_types::ModeKind;

    /// Session cwd used for relative-path argument normalization.
    fn legacy_cwd(&self) -> AbsolutePathBuf;

    /// Whether the active turn belongs to a non-root agent thread.
    fn is_non_root_agent(&self) -> bool;

    /// Whether the current client supports image input items.
    fn supports_image_input(&self) -> bool;

    /// App-server client name associated with the active turn, when present.
    fn app_server_client_name(&self) -> Option<&str>;

    /// Whether auth elicitation is enabled for the active turn.
    fn auth_elicitation_enabled(&self) -> bool;

    /// Whether MCP tool approval elicitation is enabled for the active turn.
    fn tool_call_mcp_elicitation_enabled(&self) -> bool;

    /// MCP request metadata derived from the active turn state.
    fn mcp_turn_metadata(&self) -> Option<serde_json::Value>;

    /// MCP sandbox state snapshot for the active turn.
    fn mcp_sandbox_state(&self) -> SandboxState;

    /// Auth snapshot visible to the active turn.
    fn auth_snapshot<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Option<codex_auth_types::RequestAuthSnapshot>>;

    /// Cached accessible connectors visible to the active turn.
    fn cached_accessible_connectors_from_mcp_tools<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> SessionCapabilityFuture<'a, Option<Vec<AppInfo>>>;

    /// Refresh the accessible connector cache derived from MCP tools.
    fn refresh_accessible_connectors_cache_from_mcp_tools(
        &self,
        connector_auth_context: Option<&CodexAppsAuthContext>,
        mcp_tools: &[ToolInfo],
    );

    /// App-tool policy for one Codex Apps tool under the active turn config.
    fn codex_app_tool_policy(
        &self,
        metadata: Option<&McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> ThreadAppToolPolicy;

    /// Collaboration mode currently configured on the owning session.
    fn session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, protocol::config_types::ModeKind>;

    /// Emit one typed event for the active turn.
    fn emit_event<'a>(&'a self, event: EventMsg) -> SessionCapabilityFuture<'a, ()>;

    /// Request additional permissions from the client/runtime.
    fn request_permissions<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<protocol::request_permissions::RequestPermissionsResponse>,
    >;

    /// Request structured user input from the client/runtime.
    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<'a, Option<protocol::request_user_input::RequestUserInputResponse>>;

    /// Dispatch one dynamic tool call through the active thread runtime.
    fn request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<protocol::dynamic_tools::DynamicToolResponse>>;
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

    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.as_ref().apply_patch_streaming_events_enabled()
    }

    fn collaboration_mode_kind(&self) -> protocol::config_types::ModeKind {
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

    fn app_server_client_name(&self) -> Option<&str> {
        self.as_ref().app_server_client_name()
    }

    fn auth_elicitation_enabled(&self) -> bool {
        self.as_ref().auth_elicitation_enabled()
    }

    fn tool_call_mcp_elicitation_enabled(&self) -> bool {
        self.as_ref().tool_call_mcp_elicitation_enabled()
    }

    fn mcp_turn_metadata(&self) -> Option<serde_json::Value> {
        self.as_ref().mcp_turn_metadata()
    }

    fn mcp_sandbox_state(&self) -> SandboxState {
        self.as_ref().mcp_sandbox_state()
    }

    fn auth_snapshot<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Option<codex_auth_types::RequestAuthSnapshot>> {
        self.as_ref().auth_snapshot()
    }

    fn cached_accessible_connectors_from_mcp_tools<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> SessionCapabilityFuture<'a, Option<Vec<AppInfo>>> {
        self.as_ref()
            .cached_accessible_connectors_from_mcp_tools(auth_snapshot)
    }

    fn refresh_accessible_connectors_cache_from_mcp_tools(
        &self,
        connector_auth_context: Option<&CodexAppsAuthContext>,
        mcp_tools: &[ToolInfo],
    ) {
        self.as_ref()
            .refresh_accessible_connectors_cache_from_mcp_tools(connector_auth_context, mcp_tools);
    }

    fn codex_app_tool_policy(
        &self,
        metadata: Option<&McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> ThreadAppToolPolicy {
        self.as_ref().codex_app_tool_policy(metadata, tool_name)
    }

    fn session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, protocol::config_types::ModeKind> {
        self.as_ref().session_collaboration_mode()
    }

    fn emit_event<'a>(&'a self, event: EventMsg) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref().emit_event(event)
    }

    fn request_permissions<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<protocol::request_permissions::RequestPermissionsResponse>,
    > {
        self.as_ref()
            .request_permissions(call_id, args, cancellation_token)
    }

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<'a, Option<protocol::request_user_input::RequestUserInputResponse>>
    {
        self.as_ref().request_user_input(call_id, args)
    }

    fn request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<protocol::dynamic_tools::DynamicToolResponse>> {
        self.as_ref()
            .request_dynamic_tool(call_id, tool_name, arguments)
    }
}

/// Boxed future returned by thread domain service APIs.
pub type ThreadServiceFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Provider-neutral lifecycle runtime boundary.
///
/// This surface owns thread-level lifecycle coordination that is independent of
/// model-visible tools and turn/session capabilities. Creation requests that
/// still need concrete config/runtime handles stay behind adapter-specific
/// traits until those request DTOs can move here without introducing dependency
/// cycles.
pub trait ThreadLifecycleRuntime: Send + Sync + 'static {
    fn shutdown_all_threads_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> ThreadServiceFuture<'a, ThreadShutdownReport>;

    fn shutdown_all_threads_for_runtime_teardown_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> ThreadServiceFuture<'a, ThreadShutdownReport>;

    fn shutdown_live_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<String>>;

    fn remove_live_thread<'a>(&'a self, thread_id: ThreadId) -> ThreadServiceFuture<'a, bool>;

    fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadCreatedEvent>;

    fn live_thread_agent_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<AgentStatus>>;

    fn live_thread_runtime_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<ThreadRuntimeStatus>>;

    fn subscribe_live_thread_status<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<watch::Receiver<AgentStatus>>>;

    fn active_event_subscriptions(&self) -> Arc<ActiveEventSubscriptionTracker>;
}

/// Morpheus-only native agent runtime operations.
///
/// These methods carry native role/type/model semantics and are not required of
/// external provider adapters. External provider support remains on
/// `ThreadCollaborationRuntime` as a separate model-visible tool surface.
pub trait NativeAgentRuntime: Send + Sync + 'static {
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
        input: ThreadFollowupTaskInput,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>>;

    fn close_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>>;

    fn list_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>>;

    fn read_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadReadAgentResult, FunctionCallError>>;

    fn load_agent_role<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        agent_type: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadAgentRoleLoadResult, FunctionCallError>>;
}

/// External collaboration tool runtime.
///
/// This boundary carries the model-visible external provider tool surface
/// without requiring native role/type/model semantics.
pub trait ThreadCollaborationRuntime: Send + Sync + 'static {
    fn spawn_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnExternalAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>>;

    fn followup_external_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        input: ThreadFollowupTaskInput,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>>;

    fn close_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>>;

    fn list_external_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>>;

    fn read_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadReadAgentResult, FunctionCallError>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDirectoryEntrySource {
    NativeLive,
    ExternalLive,
    Persisted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDirectoryEntry {
    pub thread_id: ThreadId,
    pub parent_thread_id: Option<ThreadId>,
    pub depth: Option<i32>,
    pub agent_path: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
    pub last_task_message: Option<String>,
    pub lifecycle_status: ThreadLifecycleStatus,
    pub source: AgentDirectoryEntrySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDirectoryListRequest {
    pub current_thread_id: ThreadId,
    pub current_session_source: SessionSource,
    pub path_prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDirectoryListResult {
    pub entries: Vec<AgentDirectoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReferenceResolutionRequest {
    pub current_thread_id: ThreadId,
    pub current_session_source: SessionSource,
    pub agent_reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentReferenceResolution {
    Live {
        thread_id: ThreadId,
    },
    PersistedNative {
        thread_id: ThreadId,
        parent_thread_id: ThreadId,
        depth: i32,
        agent_path: String,
    },
    PersistedExternalReadOnly {
        thread_id: ThreadId,
        agent_path: String,
    },
    Unsupported {
        agent_path: String,
        message: String,
    },
    NotFound {
        agent_path: String,
    },
}

/// Provider-neutral agent directory and reference lookup boundary.
///
/// Implementations return copied facts about live and persisted agent trees.
/// This surface must not expose native session handles, external input sinks,
/// provider transports, or app-server protocol DTOs; callers decide which side
/// effects, if any, are allowed after inspecting the returned facts.
pub trait ThreadAgentDirectoryRuntime: Send + Sync + 'static {
    fn list_agent_directory<'a>(
        &'a self,
        request: AgentDirectoryListRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<AgentDirectoryListResult>>;

    fn resolve_agent_reference_in_directory<'a>(
        &'a self,
        request: AgentReferenceResolutionRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<AgentReferenceResolution>>;

    fn list_agent_subtree_thread_ids<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<Vec<ThreadId>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistedThreadProviderFactsSelector {
    ThreadId(ThreadId),
    RolloutPath(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedExternalRootThreadFacts {
    pub thread_id: ThreadId,
    pub provider_id: String,
    pub restore_eligibility: thread_store_api::ExternalLiveRestoreEligibility,
}

pub trait PersistedThreadProviderFactsRuntime: Send + Sync + 'static {
    fn persisted_external_root_thread_facts<'a>(
        &'a self,
        selector: PersistedThreadProviderFactsSelector,
    ) -> ThreadServiceFuture<'a, CodexResult<Option<PersistedExternalRootThreadFacts>>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalRootThreadProvider {
    CodexCli,
    ClaudeCli,
    Opencode,
}

impl ExternalRootThreadProvider {
    pub fn from_provider_id(provider_id: &str) -> Option<Self> {
        match provider_id {
            "codex_cli" => Some(Self::CodexCli),
            "claude_cli" => Some(Self::ClaudeCli),
            "opencode" => Some(Self::Opencode),
            _ => None,
        }
    }

    pub fn provider_id(self) -> &'static str {
        match self {
            Self::CodexCli => "codex_cli",
            Self::ClaudeCli => "claude_cli",
            Self::Opencode => "opencode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadProviderRuntimeKind {
    Native,
    ExternalCli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadProviderRootCapability {
    StartThread,
    RestoreThread,
    RestoreSnapshot,
    ForkThread,
}

impl ThreadProviderRootCapability {
    pub fn method(self) -> &'static str {
        match self {
            Self::StartThread => "thread/start",
            Self::RestoreThread => "thread/resume",
            Self::RestoreSnapshot => "thread/resume",
            Self::ForkThread => "thread/fork",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveExternalRootThreadFacts {
    pub thread_id: ThreadId,
    pub provider: ExternalRootThreadProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalRootThreadInputRoute {
    LiveExternalRoot {
        thread_id: ThreadId,
        provider: ExternalRootThreadProvider,
    },
    UnsupportedPersistedExternalRoot {
        thread_id: ThreadId,
        provider_id: String,
    },
    NativeRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadProviderRuntimeCapabilities {
    pub start_thread: bool,
    pub send_input: bool,
    pub close_thread: bool,
    pub list_children: bool,
    pub restore_thread: bool,
    pub restore_snapshot: bool,
    pub event_stream: bool,
    pub spawn_child: bool,
    pub compact: bool,
    pub workflow: bool,
    pub poll_event: bool,
    pub command_session: bool,
    pub permissions: bool,
    pub dynamic_tools: bool,
    pub fork_thread: bool,
}

impl ThreadProviderRuntimeCapabilities {
    pub fn supports_root_capability(self, capability: ThreadProviderRootCapability) -> bool {
        match capability {
            ThreadProviderRootCapability::StartThread => self.start_thread,
            ThreadProviderRootCapability::RestoreThread => self.restore_thread,
            ThreadProviderRootCapability::RestoreSnapshot => self.restore_snapshot,
            ThreadProviderRootCapability::ForkThread => self.fork_thread,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadProviderRuntimeDescriptor {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub kind: ThreadProviderRuntimeKind,
    pub external_root_provider: Option<ExternalRootThreadProvider>,
    pub capabilities: ThreadProviderRuntimeCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootThreadProviderRoute {
    Native,
    External(ExternalRootThreadProvider),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootThreadProviderResolutionError {
    UnknownProvider {
        provider_id: String,
        capability: ThreadProviderRootCapability,
    },
    UnsupportedCapability {
        provider_id: String,
        capability: ThreadProviderRootCapability,
    },
}

pub trait ThreadProviderCatalogRuntime: Send + Sync + 'static {
    fn list_thread_providers(&self) -> Vec<ThreadProviderRuntimeDescriptor>;

    fn resolve_root_thread_provider(
        &self,
        provider_id: Option<&str>,
        capability: ThreadProviderRootCapability,
    ) -> Result<RootThreadProviderRoute, RootThreadProviderResolutionError>;
}

pub struct ExternalRootThreadStartResult {
    pub thread_id: ThreadId,
    pub session_configured: SessionConfiguredEvent,
}

/// Resolved startup configuration needed to seed an external root thread.
///
/// This is intentionally narrower than the native `Config`: it captures the
/// already-resolved facts used by external provider startup and persistence,
/// without native-only environment, dynamic tool, memory startup, or review
/// runtime inputs.
pub struct ExternalRootThreadStartupConfig {
    pub cwd: AbsolutePathBuf,
    pub workspace_roots: Vec<AbsolutePathBuf>,
    pub agent_max_threads: Option<usize>,
    pub agent_roles: BTreeMap<String, AgentRoleConfig>,
    pub model: String,
    pub model_provider_id: String,
    pub service_tier: Option<String>,
    pub approval_policy: AskForApproval,
    pub approvals_reviewer: ApprovalsReviewer,
    pub permission_profile: PermissionProfile,
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub personality: Option<Personality>,
    pub features: Features,
    pub generate_memories: bool,
    pub default_wait_timeout_ms: i64,
    pub max_wait_timeout_ms: i64,
}

pub struct ExternalRootThreadStartRequest {
    pub startup_config: ExternalRootThreadStartupConfig,
    pub provider: ExternalRootThreadProvider,
    pub agent_metadata: Option<ExternalRootAgentMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRootAgentMetadata {
    pub agent_path: AgentPath,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
}

/// Provider-neutral external root thread runtime.
///
/// This boundary owns external root start and root input delivery. Model-visible
/// external child collaboration remains on `ThreadCollaborationRuntime`.
pub trait ExternalRootThreadRuntime: Send + Sync + 'static {
    fn start_external_root_thread<'a>(
        &'a self,
        request: ExternalRootThreadStartRequest,
    ) -> ThreadServiceFuture<'a, CodexResult<ExternalRootThreadStartResult>>;

    fn has_external_root_thread(&self, thread_id: ThreadId) -> bool;

    fn live_external_root_thread_facts(
        &self,
        thread_id: ThreadId,
    ) -> Option<LiveExternalRootThreadFacts>;

    fn external_root_thread_input_route<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<ExternalRootThreadInputRoute>>;

    fn submit_external_root_input<'a>(
        &'a self,
        thread_id: ThreadId,
        message: String,
    ) -> ThreadServiceFuture<'a, CodexResult<String>>;

    fn close_external_root_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<String>>;
}

/// Native turn-bound event and workflow progress adapter.
///
/// These methods still require a native `ThreadTurnCapability`; they adapt the
/// native session's poll-event and display emission semantics for native tools
/// and workflow runs. External provider poll paths are thread-id scoped and do
/// not implement this turn-bound adapter.
pub trait NativeTurnEventRuntime: Send + Sync + 'static {
    fn poll_event<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>>;

    fn poll_event_timeout_metadata<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventTimeoutMetadata, FunctionCallError>>;

    fn reset_thread_wait_backoff<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
    ) -> ThreadServiceFuture<'a, ()>;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>>;
}

pub trait ThreadSessionCapability: Send + Sync + 'static {
    /// Implementation-owned typed view for the session service that created
    /// this capability. External services should not downcast this value.
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    /// Implementation-owned erased `Arc` for owner-side downcasting.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Thread identifier owned by this session runtime.
    fn conversation_id(&self) -> ThreadId;

    /// Return currently active subscriptions for this thread.
    fn active_subscriptions<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Vec<PersistedSubscription>> {
        Box::pin(async { Vec::new() })
    }

    /// Return the persisted state DB for this thread, materializing any
    /// required thread metadata first.
    fn require_persisted_state_db<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<SharedStateDbRuntime, String>>;

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
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Account goal progress after a goal-mutating tool completes without
    /// steering or terminal metric emission.
    fn account_goal_mutation_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Capture active-goal accounting baselines for a newly started turn.
    fn begin_turn_goal_accounting<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        token_usage: TokenUsage,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Finalize active-goal accounting for one turn.
    fn finish_turn_goal_accounting<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        turn_completed: bool,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Handle goal side effects when the active turn aborts.
    fn handle_goal_turn_abort<'a>(
        &'a self,
        turn: Option<&'a dyn ThreadTurnCapability>,
        reason: TurnAbortReason,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Continue one active goal when the thread is idle.
    fn maybe_continue_active_goal<'a>(&'a self) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Account active goal usage before mutating persisted goal state externally.
    fn prepare_external_goal_mutation<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Apply runtime side effects after an external goal upsert.
    fn apply_external_goal_set<'a>(
        &'a self,
        external_set: ExternalGoalSet,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Clear goal runtime state after an external goal removal.
    fn apply_external_goal_clear<'a>(&'a self) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Restore goal runtime state after resuming a thread.
    fn restore_goal_runtime_after_resume<'a>(
        &'a self,
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

    /// Queue one model-visible item through the pending-input path and ensure
    /// the runtime continues processing it in the current turn or the next one.
    fn append_conversation_item<'a>(
        &'a self,
        item: ResponseItem,
    ) -> SessionCapabilityFuture<'a, Result<String, String>>;

    /// Queue one model-visible item and defer the paired display event until
    /// the item is consumed while constructing a model request.
    fn append_conversation_item_with_observed_event<'a>(
        &'a self,
        item: ResponseItem,
        event: EventMsg,
    ) -> SessionCapabilityFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let _ = event;
            self.append_conversation_item(item).await
        })
    }

    /// Sandbox runtime shared by the owning session.
    fn sandbox_runtime(&self) -> SharedSandboxRuntime;

    /// Subscribe to out-of-band elicitation pause state for this session.
    fn subscribe_out_of_band_elicitation_pause_state(&self) -> watch::Receiver<bool>;

    /// Request one MCP server elicitation through the active turn lifecycle.
    fn request_mcp_server_elicitation<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> SessionCapabilityFuture<'a, Option<ElicitationResponse>>;

    /// Resolve one pending or runtime-owned MCP elicitation.
    fn resolve_mcp_elicitation<'a>(
        &'a self,
        server_name: String,
        request_id: RequestId,
        response: ElicitationResponse,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Refresh MCP servers queued on the session, if any.
    fn refresh_mcp_servers_if_requested<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Queue one MCP server refresh configuration on the session.
    fn queue_mcp_server_refresh<'a>(
        &'a self,
        refresh_config: McpServerRefreshConfig,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// 返回当前配置视角下可见的 MCP server 配置。
    fn configured_mcp_servers<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, HashMap<String, codex_config_types::McpServerConfig>>;

    /// 返回当前 session 已提示过的 MCP dependency key 集合。
    fn mcp_dependency_prompted<'a>(&'a self) -> SessionCapabilityFuture<'a, HashSet<String>>;

    /// 记录当前 session 已提示过的 MCP dependency key。
    fn record_mcp_dependency_prompted<'a>(
        &'a self,
        names: Vec<String>,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// 向当前 session 交付一次结构化 user input 响应。
    fn notify_user_input_response<'a>(
        &'a self,
        sub_id: &'a str,
        response: protocol::request_user_input::RequestUserInputResponse,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// 查询指定 transport 的 MCP OAuth 登录支持情况。
    fn mcp_oauth_login_support<'a>(
        &'a self,
        transport: &'a codex_config_types::McpServerTransportConfig,
    ) -> SessionCapabilityFuture<'a, mcp_types::McpOAuthLoginSupport>;

    /// 执行一次 MCP OAuth 登录流程。
    fn perform_mcp_oauth_login<'a>(
        &'a self,
        params: McpOAuthLoginParams,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>>;

    /// 判断 MCP OAuth 失败后是否应退化为无 scope 重试。
    fn should_retry_mcp_oauth_without_scopes(
        &self,
        scopes: &mcp_types::ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool;

    /// Refresh MCP servers immediately with the provided configuration.
    fn refresh_mcp_servers_now<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        refresh_config: McpServerRefreshConfig,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Cancel any in-flight MCP startup owned by this session.
    fn cancel_mcp_startup<'a>(&'a self) -> SessionCapabilityFuture<'a, ()>;

    /// Hard-refresh the Codex Apps MCP tools cache.
    fn hard_refresh_codex_apps_tools_cache<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<Vec<mcp_types::ToolInfo>, String>>;

    /// Execute one raw MCP tool call through the session-owned MCP runtime.
    fn call_mcp_tool<'a>(
        &'a self,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> SessionCapabilityFuture<'a, Result<CallToolResult, String>>;

    /// List MCP resources for one server through the session-owned MCP runtime.
    fn list_mcp_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>>;

    /// List MCP resources for all visible servers.
    fn list_all_mcp_resources(&self)
    -> SessionCapabilityFuture<'_, HashMap<String, Vec<Resource>>>;

    /// List MCP resource templates for one server through the session-owned MCP runtime.
    fn list_mcp_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>>;

    /// List MCP resource templates for all visible servers.
    fn list_all_mcp_resource_templates(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<ResourceTemplate>>>;

    /// Read one MCP resource through the session-owned MCP runtime.
    fn read_mcp_resource<'a>(
        &'a self,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>>;

    /// List all visible MCP tools for the session-owned MCP runtime.
    fn list_all_mcp_tools<'a>(&'a self) -> SessionCapabilityFuture<'a, Vec<ToolInfo>>;

    /// Return the server origin for one MCP server, if known.
    fn mcp_server_origin<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, Option<String>>;

    /// Whether one MCP server is the host-owned Codex Apps server.
    fn mcp_server_is_host_owned_codex_apps<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, bool>;

    /// Whether one MCP server supports sandbox-state request metadata.
    fn mcp_server_supports_sandbox_state_meta<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, bool>;

    /// Add optional trace metadata for one MCP tool call.
    fn add_optional_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Option<serde_json::Value>;

    /// Rewrite MCP tool arguments for OpenAI file uploads under the active turn.
    fn rewrite_mcp_tool_arguments_for_openai_files<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        arguments: Option<serde_json::Value>,
        openai_file_input_params: Option<&'a [String]>,
    ) -> SessionCapabilityFuture<'a, Result<Option<serde_json::Value>, String>>;

    /// Mark the thread memory mode polluted when the specified MCP server requires it.
    fn mark_thread_memory_mode_polluted_for_mcp_tool_call<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Track one Codex Apps tool usage event for analytics/state.
    fn track_codex_app_used_for_mcp_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Whether one MCP tool approval key is already remembered for the session.
    fn mcp_tool_approval_is_remembered<'a>(
        &'a self,
        key: &'a mcp_types::McpToolApprovalKey,
    ) -> SessionCapabilityFuture<'a, bool>;

    /// Remember one MCP tool approval key for the current session.
    fn remember_mcp_tool_approval<'a>(
        &'a self,
        key: mcp_types::McpToolApprovalKey,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Resolve the custom approval mode for one MCP tool.
    fn custom_mcp_tool_approval_mode<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, codex_config_types::AppToolApproval>;

    /// Fetch accessible connectors derived from MCP tools for the current session.
    fn fetch_accessible_connectors_from_mcp_tools<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<Vec<AppInfo>>>;

    /// Persist approval for one Codex Apps tool and reload user config.
    fn persist_codex_app_tool_approval_for_turn<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        connector_id: String,
        tool_name: String,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>>;

    /// Persist approval for one non-app MCP tool and reload user config.
    fn persist_non_app_mcp_tool_approval_for_turn<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: String,
        tool_name: String,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>>;

    /// Reload the user config layer for subsequent turn decisions.
    fn reload_user_config_layer<'a>(&'a self) -> SessionCapabilityFuture<'a, ()>;

    /// Check whether one plugin is configured as installed in the current session config.
    fn configured_plugin_installed<'a>(
        &'a self,
        tool_id: &'a str,
    ) -> SessionCapabilityFuture<'a, bool>;

    /// Merge connector IDs into the explicit session-level connector selection.
    fn merge_connector_selection<'a>(
        &'a self,
        connector_ids: std::collections::HashSet<String>,
    ) -> SessionCapabilityFuture<'a, std::collections::HashSet<String>>;

    /// Evaluate one auto-approved action with the runtime safety monitor.
    fn monitor_auto_approved_action<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        action: serde_json::Value,
        callsite_mode: &'static str,
    ) -> SessionCapabilityFuture<'a, AutoApprovalSafetyOutcome>;

    /// Emit one started item for the active turn.
    fn emit_turn_item_started<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: &'a protocol::items::TurnItem,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit one completed item for the active turn.
    fn emit_turn_item_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: protocol::items::TurnItem,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit one started response item display event.
    fn emit_model_item_started_display_event<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: &'a ResponseItem,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit one terminal interaction event.
    fn send_terminal_interaction<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        event: protocol::protocol::TerminalInteractionEvent,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Remove one in-flight network approval registration.
    fn unregister_network_approval<'a>(
        &'a self,
        registration_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Snapshot persisted code-mode values visible to the current session.
    fn code_mode_stored_values(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, serde_json::Value>>;

    /// Replace the persisted code-mode values for the current session.
    fn code_mode_replace_stored_values(
        &self,
        values: HashMap<String, serde_json::Value>,
    ) -> SessionCapabilityFuture<'_, ()>;

    /// Allocate a new runtime cell id before starting a code-mode execution.
    fn code_mode_allocate_cell_id(&self) -> String;

    /// Execute one code-mode request.
    fn code_mode_execute(
        &self,
        request: ExecuteRequest,
    ) -> SessionCapabilityFuture<'_, Result<RuntimeResponse, String>>;

    /// Wait for one bounded code-mode runtime update.
    fn code_mode_wait(
        &self,
        request: WaitRequest,
    ) -> SessionCapabilityFuture<'_, Result<WaitOutcome, String>>;

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
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext;

    /// Return the single local environment cwd used for agent-job CSV input/output.
    fn single_local_environment_cwd(&self) -> Result<AbsolutePathBuf, FunctionCallError>;

    /// Default runtime timeout for each spawned agent-job worker.
    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64>;

    /// Whether approvals for this turn route through guardian.
    fn routes_approval_to_guardian(&self) -> bool;

    /// Current exec policy snapshot visible to the active turn.
    fn current_exec_policy(&self) -> std::sync::Arc<permissions_service_api::Policy>;

    /// Shell environment policy configured for this turn.
    fn shell_environment_policy(&self) -> protocol::config_types::ShellEnvironmentPolicy;

    /// Runtime shell resolved from the owning session shell configuration.
    fn runtime_shell(&self) -> RuntimeShell;

    /// Tool-facing shell type derived from the owning session shell.
    fn tool_user_shell_type(&self) -> tool_config::ToolUserShellType;

    /// Optionally emit one implicit skill invocation derived from exec command input.
    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Whether exec permission approvals are enabled for this turn/session.
    fn exec_permission_approvals_enabled(&self) -> bool;

    /// Whether request-permissions tool flow is enabled for this turn/session.
    fn request_permissions_tool_enabled(&self) -> bool;

    /// Resolve one model-provided shell path into runtime shell metadata.
    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell;

    /// Resolve one shell command into the runtime exec argv.
    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String>;

    /// Environment overrides applied to shell execution.
    fn shell_env_overrides(&self) -> HashMap<String, String>;

    /// Resolve the effective shell workdir for one command request.
    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf;

    /// Resolve one user-supplied local path using the active turn's path semantics.
    fn resolve_turn_path(&self, path: Option<String>) -> AbsolutePathBuf;

    /// Start one managed-network approval for a command tool invocation.
    fn begin_tool_network_approval<'a>(
        &'a self,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> SessionCapabilityFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>>;

    /// Unified exec shell mode configured for this turn.
    fn unified_exec_shell_mode(&self) -> tool_config::UnifiedExecShellMode;

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
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
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

    fn current_exec_policy(&self) -> std::sync::Arc<permissions_service_api::Policy> {
        self.as_ref().current_exec_policy()
    }

    fn shell_environment_policy(&self) -> protocol::config_types::ShellEnvironmentPolicy {
        self.as_ref().shell_environment_policy()
    }

    fn runtime_shell(&self) -> RuntimeShell {
        self.as_ref().runtime_shell()
    }

    fn tool_user_shell_type(&self) -> tool_config::ToolUserShellType {
        self.as_ref().tool_user_shell_type()
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref()
            .maybe_emit_implicit_skill_invocation(command, workdir)
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        self.as_ref().exec_permission_approvals_enabled()
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        self.as_ref().request_permissions_tool_enabled()
    }

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell {
        self.as_ref().resolve_model_shell(shell)
    }

    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String> {
        self.as_ref()
            .resolve_exec_command(command, login, model_shell)
    }

    fn shell_env_overrides(&self) -> HashMap<String, String> {
        self.as_ref().shell_env_overrides()
    }

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf {
        self.as_ref().resolve_shell_workdir(workdir)
    }

    fn resolve_turn_path(&self, path: Option<String>) -> AbsolutePathBuf {
        self.as_ref().resolve_turn_path(path)
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> SessionCapabilityFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> {
        self.as_ref().begin_tool_network_approval(spec)
    }

    fn unified_exec_shell_mode(&self) -> tool_config::UnifiedExecShellMode {
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
}

/// Session-owned agent-job capability consumed by CSV agent-job tools.
///
/// Implementations own state DB access, subagent spawning, worker lifecycle,
/// and status subscriptions.
pub trait SessionAgentJobCaller: Send + Sync + 'static {
    /// Return the state DB runtime if this session supports agent jobs.
    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime>;

    /// Return the current thread id as a string for result attribution.
    fn agent_job_conversation_id_string(&self) -> String;

    /// Build runner options and the spawn config for agent-job workers.
    fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        requested_concurrency: Option<usize>,
    ) -> SessionCapabilityFuture<
        '_,
        Result<AgentJobRunnerOptions<AgentJobSpawnConfig>, FunctionCallError>,
    >;

    /// Spawn one agent-job worker.
    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a dyn ThreadRuntimeCapability,
        spawn_config: AgentJobSpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> SessionCapabilityFuture<'a, Result<ThreadId, AgentJobSpawnWorkerError>>;

    /// Shutdown one worker thread.
    fn shutdown_agent_job_worker(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, ()>;

    /// Read one worker status.
    fn get_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, AgentStatus>;

    /// Subscribe to worker status changes.
    fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, Option<watch::Receiver<AgentStatus>>>;
}

impl<Session> SessionAgentJobCaller for Arc<Session>
where
    Session: SessionAgentJobCaller,
{
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
    ) -> SessionCapabilityFuture<
        '_,
        Result<AgentJobRunnerOptions<AgentJobSpawnConfig>, FunctionCallError>,
    > {
        Arc::clone(self.as_ref()).build_agent_job_runner_options(turn, requested_concurrency)
    }

    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a dyn ThreadRuntimeCapability,
        spawn_config: AgentJobSpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> SessionCapabilityFuture<'a, Result<ThreadId, AgentJobSpawnWorkerError>> {
        Arc::clone(self.as_ref()).spawn_agent_job_worker(turn, spawn_config, job_id, prompt)
    }

    fn shutdown_agent_job_worker(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, ()> {
        Arc::clone(self.as_ref()).shutdown_agent_job_worker(thread_id)
    }

    fn get_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, AgentStatus> {
        Arc::clone(self.as_ref()).get_agent_job_worker_status(thread_id)
    }

    fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> SessionCapabilityFuture<'static, Option<watch::Receiver<AgentStatus>>> {
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
