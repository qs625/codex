//! Core-independent live session operation API.
//!
//! These contracts are now owned by `codex-thread-api` as part of the unified
//! live thread runtime boundary. Concrete session loop implementations still
//! live in runtime crates and implement these traits by adapting their
//! internal state.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_exec_server_api::ExecEnvironment;
use codex_file_system::ExecutorFileSystem;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_permissions_runtime::ExecApprovalRequirement;
use codex_protocol::ThreadId;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_file_system::FileSystemSandboxContext;
use codex_protocol::items::FileChangeItem;
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
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::Submission;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::TurnDiffEvent;
use codex_protocol::protocol::W3cTraceContext;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_state_api::SharedStateDbRuntime;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_tool_types::DiscoverableTool;
use codex_tool_types::RequestPluginInstallElicitationRequest;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use tokio::sync::Mutex;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use serde::Serialize;

pub use codex_runtime_capability_api::ThreadCapability;

#[path = "pending_input.rs"]
mod pending_input;

pub use pending_input::PendingInputItem;

/// Boxed future returned by object-safe session capability traits.
pub type SessionCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type SharedToolTurnDiffTracker = Arc<Mutex<crate::TurnDiffTracker>>;

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

/// Filesystem/environment boundary needed by thread-owned tool execution.
pub trait ApplyPatchEnvironment: Send + Sync {
    fn environment_id(&self) -> &str;

    fn filesystem(&self) -> Arc<dyn ExecutorFileSystem>;
}

pub struct ToolSandboxContext {
    pub turn_id: String,
    pub telemetry: SharedSessionTelemetry,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub permission_profile: PermissionProfile,
    pub managed_network_active: bool,
    pub cwd: AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
}

pub struct ResolvedApplyPatchEnvironment {
    pub cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ApplyPatchEnvironment>,
}

pub struct ResolvedExecCommandEnvironment {
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub apply_patch_environment: Arc<dyn ApplyPatchEnvironment>,
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

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<codex_network_proxy_api::SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct McpToolCallOutcome {
    pub result: CallToolResult,
    pub tool_input: serde_json::Value,
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
pub trait ToolTurnCapability: Send + Sync + 'static {
    /// Implementation-owned typed view for the session service that created the
    /// turn. External services should not downcast this value.
    fn as_any(&self) -> &(dyn Any + Send + Sync);

    /// Telemetry sink for tool dispatch spans and handler results.
    fn tool_dispatch_telemetry(&self) -> SharedSessionTelemetry;

    /// Base tags applied to tool result telemetry for this turn.
    fn base_tool_result_tags(&self) -> ToolTelemetryTags;

    /// Runtime turn identifier used in rollout trace records.
    fn rollout_turn_id(&self) -> String;
}

impl<Turn> ToolTurnCapability for Arc<Turn>
where
    Turn: ToolTurnCapability,
{
    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self.as_ref().as_any()
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
}

/// Thread-runtime-owned session reference passed into tool service dispatch.
///
/// This is still a migration-time bridge while tool handlers are being moved
/// away from concrete session/runtime generics. The owner stays in
/// `codex-thread-api` because the referenced runtime object belongs to the
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

pub trait ToolSessionCapability: Send + Sync + 'static {
    /// Telemetry sink for tool dispatch spans and handler results.
    fn tool_dispatch_telemetry(&self, turn: &dyn ToolTurnCapability) -> SharedSessionTelemetry;

    /// Base tags applied to tool result telemetry for the active turn.
    fn base_tool_result_tags(&self, turn: &dyn ToolTurnCapability) -> ToolTelemetryTags;

    /// Record that a model-visible tool call started.
    fn record_tool_call_started<'a>(
        &'a self,
        turn: &'a dyn ToolTurnCapability,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Start trace lifecycle for one tool dispatch.
    fn start_tool_dispatch_trace(
        &self,
        turn: &dyn ToolTurnCapability,
        call_id: &str,
        tool_name: &ToolName,
        source: &ToolCallSource,
        payload: &ToolPayload,
    ) -> Box<dyn ToolSessionDispatchTrace>;

    /// Run pre-tool hooks for a tool invocation.
    fn run_pre_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ToolTurnCapability,
        call_id: String,
        payload: PreToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PreToolUseHookOutcome>;

    /// Run post-tool hooks for a completed tool invocation.
    fn run_post_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ToolTurnCapability,
        payload: PostToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PostToolUseHookOutcome>;

    /// Emit memory/read telemetry derived from a completed tool invocation.
    fn emit_tool_read_metric<'a>(
        &'a self,
        turn: &'a dyn ToolTurnCapability,
        tool_name: &'a ToolName,
        payload: &'a ToolPayload,
        success: bool,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Account a completed tool call against active goal runtime state.
    fn account_goal_tool_completed<'a>(
        &'a self,
        turn: &'a dyn ToolTurnCapability,
        tool_name: &'a ToolName,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;
}

/// Session-owned MCP call capability consumed by tool handlers.
///
/// Implementations own concrete MCP approval, elicitation, telemetry, event,
/// connector, and tool-call side effects for one live session.
pub trait SessionMcpToolCaller: Send + Sync + 'static {
    /// Execute one MCP tool call for the given turn and return the model-visible
    /// result plus the normalized tool input used for history/output shaping.
    fn call_mcp_tool(
        self: Arc<Self>,
        turn: &dyn SessionMcpToolTurn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> impl Future<Output = McpToolCallOutcome> + Send + '_;
}

impl<Session> SessionMcpToolCaller for Arc<Session>
where
    Session: SessionMcpToolCaller,
{
    fn call_mcp_tool(
        self: Arc<Self>,
        turn: &dyn SessionMcpToolTurn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> impl Future<Output = McpToolCallOutcome> + Send + '_ {
        Arc::clone(self.as_ref())
            .call_mcp_tool(turn, call_id, server, tool_name, hook_tool_name, arguments)
    }
}

/// Turn-owned MCP display/output capability consumed by tool handlers.
pub trait SessionMcpToolTurn: ThreadCapability {
    /// Whether original image detail can be requested for MCP output images.
    fn mcp_original_image_detail_supported(&self) -> bool;

    /// Truncation policy used when presenting MCP output to the model.
    fn mcp_truncation_policy(&self) -> TruncationPolicy;
}

impl<Turn> SessionMcpToolTurn for Arc<Turn>
where
    Turn: SessionMcpToolTurn,
{
    fn mcp_original_image_detail_supported(&self) -> bool {
        self.as_ref().mcp_original_image_detail_supported()
    }

    fn mcp_truncation_policy(&self) -> TruncationPolicy {
        self.as_ref().mcp_truncation_policy()
    }
}

/// Global MCP resource service API consumed by tool handlers.
///
/// Implementations own MCP resource listing/reading plus the runtime lifecycle
/// events needed around one call. Handlers should depend on this API instead of
/// requiring the embedding session runtime to implement the whole resource
/// surface directly.
pub trait McpResourceApi: Send + Sync + 'static {
    /// List resources from one MCP server.
    fn list_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>>;

    /// List resources from all connected MCP servers.
    fn list_all_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, HashMap<String, Vec<Resource>>>;

    /// List resource templates from one MCP server.
    fn list_resource_templates<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>>;

    /// List resource templates from all connected MCP servers.
    fn list_all_resource_templates<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, HashMap<String, Vec<ResourceTemplate>>>;

    /// Read a single MCP resource.
    fn read_resource<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>>;

    /// Emit the resource-backed MCP lifecycle start event.
    fn emit_mcp_resource_tool_call_begin<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit the resource-backed MCP lifecycle completion event.
    fn emit_mcp_resource_tool_call_end<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> SessionCapabilityFuture<'a, ()>;
}

impl<Service> McpResourceApi for Arc<Service>
where
    Service: McpResourceApi,
{
    fn list_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourcesResult, String>> {
        self.as_ref().list_resources(capability, server, params)
    }

    fn list_all_resources<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, HashMap<String, Vec<Resource>>> {
        self.as_ref().list_all_resources(capability)
    }

    fn list_resource_templates<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<ListResourceTemplatesResult, String>> {
        self.as_ref()
            .list_resource_templates(capability, server, params)
    }

    fn list_all_resource_templates<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, HashMap<String, Vec<ResourceTemplate>>> {
        self.as_ref().list_all_resource_templates(capability)
    }

    fn read_resource<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<ReadResourceResult, String>> {
        self.as_ref().read_resource(capability, server, params)
    }

    fn emit_mcp_resource_tool_call_begin<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref()
            .emit_mcp_resource_tool_call_begin(capability, call_id, invocation)
    }

    fn emit_mcp_resource_tool_call_end<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref().emit_mcp_resource_tool_call_end(
            capability, call_id, invocation, duration, result,
        )
    }
}


/// Common turn-runtime capability shared by tool services that need active turn
/// identity, image-detail support, or filesystem-backed environment access.
pub trait ThreadRuntimeCapability: ThreadCapability {
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
}

/// Turn-runtime capability required by built-in function-style tools.
///
/// These tools do not own a separate global service. They execute against the
/// active thread turn and need a runtime capability that can both expose turn
/// metadata and perform session-owned side effects such as permission
/// elicitation or dynamic-tool dispatch.
pub trait FunctionToolCapability: ThreadRuntimeCapability {
    /// Collaboration mode for the active turn.
    fn function_tool_collaboration_mode(&self) -> codex_protocol::config_types::ModeKind;

    /// Session cwd used for relative-path argument normalization.
    fn function_tool_cwd(&self) -> AbsolutePathBuf;

    /// Whether the active turn belongs to a non-root agent thread.
    fn function_tool_is_non_root_agent(&self) -> bool;

    /// Whether the current client supports image input items.
    fn function_tool_supports_image_input(&self) -> bool;

    /// Collaboration mode currently configured on the owning thread session.
    fn function_tool_session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, codex_protocol::config_types::ModeKind>;

    /// Emit the typed plan update event for `update_plan`.
    fn function_tool_emit_plan_update<'a>(
        &'a self,
        args: codex_protocol::plan_tool::UpdatePlanArgs,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Emit the typed image-view lifecycle item for `view_image`.
    fn function_tool_emit_image_view<'a>(
        &'a self,
        call_id: String,
        path: AbsolutePathBuf,
    ) -> SessionCapabilityFuture<'a, ()>;

    /// Request additional permissions from the client/runtime.
    fn function_tool_request_permissions<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_permissions::RequestPermissionsResponse>,
    >;

    /// Request structured user input from the client/runtime.
    fn function_tool_request_user_input<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_user_input::RequestUserInputResponse>,
    >;

    /// Dispatch one dynamic tool call through the active thread runtime.
    fn function_tool_request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<codex_protocol::dynamic_tools::DynamicToolResponse>>;
}

impl<Turn> FunctionToolCapability for Arc<Turn>
where
    Turn: FunctionToolCapability,
{
    fn function_tool_collaboration_mode(&self) -> codex_protocol::config_types::ModeKind {
        self.as_ref().function_tool_collaboration_mode()
    }

    fn function_tool_cwd(&self) -> AbsolutePathBuf {
        self.as_ref().function_tool_cwd()
    }

    fn function_tool_is_non_root_agent(&self) -> bool {
        self.as_ref().function_tool_is_non_root_agent()
    }

    fn function_tool_supports_image_input(&self) -> bool {
        self.as_ref().function_tool_supports_image_input()
    }

    fn function_tool_session_collaboration_mode<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, codex_protocol::config_types::ModeKind> {
        self.as_ref().function_tool_session_collaboration_mode()
    }

    fn function_tool_emit_plan_update<'a>(
        &'a self,
        args: codex_protocol::plan_tool::UpdatePlanArgs,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref().function_tool_emit_plan_update(args)
    }

    fn function_tool_emit_image_view<'a>(
        &'a self,
        call_id: String,
        path: AbsolutePathBuf,
    ) -> SessionCapabilityFuture<'a, ()> {
        self.as_ref().function_tool_emit_image_view(call_id, path)
    }

    fn function_tool_request_permissions<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_permissions::RequestPermissionsResponse>,
    > {
        self.as_ref()
            .function_tool_request_permissions(call_id, args, cancellation_token)
    }

    fn function_tool_request_user_input<'a>(
        &'a self,
        call_id: String,
        args: codex_protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_user_input::RequestUserInputResponse>,
    > {
        self.as_ref().function_tool_request_user_input(call_id, args)
    }

    fn function_tool_request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<codex_protocol::dynamic_tools::DynamicToolResponse>>
    {
        self.as_ref()
            .function_tool_request_dynamic_tool(call_id, tool_name, arguments)
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

/// Turn-owned event capability consumed by tool lifecycle emitters.
///
/// This is narrower than the previous combined tool capability and only carries
/// display/event fields needed by generic tool event emitters.
pub trait ToolEventTurnCapability: ToolTurnCapability + Send + Sync + 'static {
    fn runtime_turn_id_str(&self) -> &str;

    fn truncation_policy(&self) -> TruncationPolicy;
}

impl<Turn> ToolEventTurnCapability for Arc<Turn>
where
    Turn: ToolEventTurnCapability,
{
    fn runtime_turn_id_str(&self) -> &str {
        self.as_ref().runtime_turn_id_str()
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.as_ref().truncation_policy()
    }
}

/// Session-owned event capability consumed by tool lifecycle emitters.
pub trait ToolEventSessionCapability: Send + Sync + 'static {
    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: ExecCommandEndEvent,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_file_change_started<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_turn_diff<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: TurnDiffEvent,
    ) -> impl Future<Output = ()> + Send + 'a;
}

impl<Session> ToolEventSessionCapability for Arc<Session>
where
    Session: ToolEventSessionCapability,
{
    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_send_exec_command_begin(turn, event)
    }

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: ExecCommandEndEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_send_exec_command_end(turn, event)
    }

    fn tool_emit_file_change_started<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_file_change_started(turn, item)
    }

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_file_change_completed(turn, item)
    }

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref()
            .tool_record_model_items_and_emit_display_events(turn, items)
    }

    fn tool_emit_turn_diff<'a>(
        &'a self,
        turn: &'a dyn ToolEventTurnCapability,
        event: TurnDiffEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_turn_diff(turn, event)
    }
}

/// Turn-owned apply-patch capability exposed by thread service.
pub trait ApplyPatchTurnCapability: ThreadRuntimeCapability + ToolEventTurnCapability {
    fn approval_policy(&self) -> AskForApproval;

    fn permission_profile(&self) -> PermissionProfile;

    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy;

    fn windows_sandbox_level(&self) -> WindowsSandboxLevel;

    fn tool_sandbox_context(&self) -> ToolSandboxContext;

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;
}

impl<Turn> ApplyPatchTurnCapability for Arc<Turn>
where
    Turn: ApplyPatchTurnCapability,
{
    fn approval_policy(&self) -> AskForApproval {
        self.as_ref().approval_policy()
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
        self.as_ref().resolve_apply_patch_environment(environment_id)
    }
}

/// Session-owned apply-patch capability exposed by thread service.
pub trait ApplyPatchSessionCapability:
    ToolEventSessionCapability + Send + Sync + 'static
{
    fn sandbox_runtime(&self) -> SharedSandboxRuntime;

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_;

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ApplyPatchTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a;

    fn tool_permission_grants(&self) -> impl Future<Output = ToolPermissionGrants> + Send + '_;
}

impl<Session> ApplyPatchSessionCapability for Arc<Session>
where
    Session: ApplyPatchSessionCapability,
{
    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        self.as_ref().sandbox_runtime()
    }

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_ {
        self.as_ref().strict_auto_review_enabled_for_turn()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ApplyPatchTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        self.as_ref().run_permission_request_hooks(
            turn,
            permission_request_run_id,
            permission_request,
        )
    }

    fn tool_permission_grants(&self) -> impl Future<Output = ToolPermissionGrants> + Send + '_ {
        self.as_ref().tool_permission_grants()
    }
}

/// Turn-owned runtime capability required by apply-patch, shell, and unified-exec tools.
///
/// Implementations own the active turn's sandbox, environment, permission, and
/// path-resolution behavior. Tool-domain code should depend on this trait
/// instead of concrete turn runtime types.
pub trait ToolRuntimeTurnCapability: ToolTurnCapability + ThreadRuntimeCapability {
    fn runtime_turn_id_str(&self) -> &str;

    fn routes_approval_to_guardian(&self) -> bool;

    fn tool_sandbox_context(&self) -> ToolSandboxContext;

    fn approval_policy(&self) -> AskForApproval;

    fn permission_profile(&self) -> PermissionProfile;

    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy;

    fn windows_sandbox_level(&self) -> WindowsSandboxLevel;

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext;

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;

    fn primary_apply_patch_environment(
        &self,
    ) -> Option<ResolvedApplyPatchEnvironment>;

    fn explicit_shell_env_overrides(&self) -> HashMap<String, String>;

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf;

    fn legacy_cwd(&self) -> AbsolutePathBuf;

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError>;

    fn truncation_policy(&self) -> TruncationPolicy;

    fn allow_login_shell(&self) -> bool;

    fn emit_unified_exec_tty_metric(&self, tty: bool);
}

impl<Turn> ToolRuntimeTurnCapability for Arc<Turn>
where
    Turn: ToolRuntimeTurnCapability,
{
    fn runtime_turn_id_str(&self) -> &str {
        self.as_ref().runtime_turn_id_str()
    }

    fn routes_approval_to_guardian(&self) -> bool {
        self.as_ref().routes_approval_to_guardian()
    }

    fn tool_sandbox_context(&self) -> ToolSandboxContext {
        self.as_ref().tool_sandbox_context()
    }

    fn approval_policy(&self) -> AskForApproval {
        self.as_ref().approval_policy()
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

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        ToolRuntimeTurnCapability::file_system_sandbox_context(
            self.as_ref(),
            additional_permissions,
            cwd,
        )
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        self.as_ref().resolve_apply_patch_environment(environment_id)
    }

    fn primary_apply_patch_environment(
        &self,
    ) -> Option<ResolvedApplyPatchEnvironment> {
        self.as_ref().primary_apply_patch_environment()
    }

    fn explicit_shell_env_overrides(&self) -> HashMap<String, String> {
        self.as_ref().explicit_shell_env_overrides()
    }

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf {
        self.as_ref().resolve_shell_workdir(workdir)
    }

    fn legacy_cwd(&self) -> AbsolutePathBuf {
        self.as_ref().legacy_cwd()
    }

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError> {
        self.as_ref()
            .resolve_exec_command_environment(environment_id, workdir)
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.as_ref().truncation_policy()
    }

    fn allow_login_shell(&self) -> bool {
        self.as_ref().allow_login_shell()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.as_ref().emit_unified_exec_tty_metric(tty);
    }
}

/// Session-owned runtime capability required by apply-patch, shell, and unified-exec tools.
///
/// Implementations own session-level permission grants, shell/runtime
/// resolution, implicit skill tracking, and concrete unified-exec execution.
/// Tool-domain code should depend on this trait instead of concrete session
/// runtime types.
pub trait ToolRuntimeSessionCapability: Send + Sync + 'static {
    fn sandbox_runtime(&self) -> SharedSandboxRuntime;

    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: ExecCommandEndEvent,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_file_change_started<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_emit_turn_diff<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: TurnDiffEvent,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn tool_permission_grants(&self) -> impl Future<Output = ToolPermissionGrants> + Send + '_;

    fn dependency_env(&self) -> impl Future<Output = HashMap<String, String>> + Send + '_;

    fn exec_permission_approvals_enabled(&self) -> bool;

    fn request_permissions_tool_enabled(&self) -> bool;

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> impl Future<Output = ExecApprovalRequirement> + Send + 'a;

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_;

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> impl Future<Output = String> + Send + 'a;

    fn guardian_timeout_message(&self) -> String;

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a;

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> impl Future<Output = Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> + Send + 'a;

}

impl<Session> ToolRuntimeSessionCapability for Arc<Session>
where
    Session: ToolRuntimeSessionCapability,
{
    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        self.as_ref().sandbox_runtime()
    }

    fn tool_send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_send_exec_command_begin(turn, event)
    }

    fn tool_send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: ExecCommandEndEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_send_exec_command_end(turn, event)
    }

    fn tool_emit_file_change_started<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_file_change_started(turn, item)
    }

    fn tool_emit_file_change_completed<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        item: FileChangeItem,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_file_change_completed(turn, item)
    }

    fn tool_record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        items: Vec<ResponseItem>,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref()
            .tool_record_model_items_and_emit_display_events(turn, items)
    }

    fn tool_emit_turn_diff<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        event: TurnDiffEvent,
    ) -> impl Future<Output = ()> + Send + 'a {
        self.as_ref().tool_emit_turn_diff(turn, event)
    }

    fn tool_permission_grants(&self) -> impl Future<Output = ToolPermissionGrants> + Send + '_ {
        self.as_ref().tool_permission_grants()
    }

    fn dependency_env(&self) -> impl Future<Output = HashMap<String, String>> + Send + '_ {
        self.as_ref().dependency_env()
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        self.as_ref().exec_permission_approvals_enabled()
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        self.as_ref().request_permissions_tool_enabled()
    }

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> impl Future<Output = ExecApprovalRequirement> + Send + 'a {
        self.as_ref().create_exec_approval_requirement(request)
    }

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_ {
        self.as_ref().strict_auto_review_enabled_for_turn()
    }

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> impl Future<Output = String> + Send + 'a {
        self.as_ref().guardian_rejection_message(review_id)
    }

    fn guardian_timeout_message(&self) -> String {
        self.as_ref().guardian_timeout_message()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        self.as_ref().run_permission_request_hooks(
            turn,
            permission_request_run_id,
            permission_request,
        )
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> impl Future<Output = Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> + Send + 'a {
        self.as_ref()
            .begin_tool_network_approval(turn_id, managed_network_active, spec)
    }

}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRuntimeNetworkApprovalTrigger {
    pub call_id: String,
    pub tool_name: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_permissions: codex_protocol::models::SandboxPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Debug)]
pub enum ToolRuntimeNetworkApprovalError {
    Rejected(String),
    Codex(CodexErr),
}

pub trait ToolRuntimeNetworkApprovalHandle: Send + Sync + 'static {
    fn mode(&self) -> NetworkApprovalMode;

    fn registration_id(&self) -> Option<String>;

    fn cancellation_token(&self) -> CancellationToken;

    fn finish<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<(), ToolRuntimeNetworkApprovalError>> + Send + 'a>>;
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


/// Global goal service API consumed by tool handlers.
///
/// Implementations own goal persistence, accounting, lifecycle side effects,
/// and display/model event emission. Handlers should depend on this service API
/// instead of requiring the embedding session runtime to implement goal
/// mutations directly.
pub trait GoalApi: Send + Sync + 'static {
    /// Read the current thread goal.
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>>;

    /// Create a new active thread goal for the current turn.
    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Mark the current thread goal complete through the normal goal runtime.
    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;
}

impl<Service> GoalApi for Arc<Service>
where
    Service: GoalApi,
{
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        self.as_ref().get_thread_goal(capability)
    }

    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref()
            .create_thread_goal(capability, objective, token_budget)
    }

    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref().complete_thread_goal(capability)
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
