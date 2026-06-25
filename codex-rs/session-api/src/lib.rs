//! Core-independent live session operation API.
//!
//! This crate owns the narrow trait surface that callers need to drive an
//! existing session. Concrete session loop implementations live in runtime
//! crates and implement these traits by adapting their internal state.

use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use codex_protocol::ThreadId;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::Submission;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::W3cTraceContext;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_config::ToolsConfig;
use codex_tool_runtime_api::AgentJobRunnerOptions;
use codex_tool_runtime_api::AgentJobSpawnWorkerError;
use codex_tool_runtime_api::AgentJobToolHost;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_runtime_api::GoalToolHost;
use codex_tool_runtime_api::McpResourceHost;
use codex_tool_runtime_api::McpToolCallHost;
use codex_tool_runtime_api::McpToolCallOutcome;
use codex_tool_runtime_api::ToolArgumentDiffConsumer;
use codex_tool_runtime_api::ToolRouterBuildParams;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolSpec;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowRun;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::WorkflowRuntimeBridge;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

mod pending_input;

pub use pending_input::PendingInputItem;

/// Boxed tool dispatch future returned by session-facing tool routers.
///
/// The concrete tool implementation crate owns handler execution. Session and
/// turn runtimes only need a stable async contract they can poll while managing
/// cancellation, context, and response history.
pub type SessionToolDispatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AnyToolResult, FunctionCallError>> + Send + 'a>>;

/// Tool capability surface consumed by the session/turn loop.
///
/// Implementations own tool registry construction, handler dispatch, hook
/// integration, telemetry, and code-mode result shaping. Session runtimes
/// should treat this as an injected capability and must not depend on concrete
/// tool handler or host implementations.
pub trait SessionToolRouter<Session, Turn, Tracker, DiffContext>: Send + Sync + 'static {
    /// Tool specs visible to the model for the current turn.
    fn model_visible_specs(&self) -> Vec<ToolSpec>;

    /// Create a streaming argument diff consumer for a tool, when supported.
    fn create_diff_consumer(
        &self,
        tool_name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>>;

    /// Whether the given call can run concurrently with other tool calls.
    fn tool_supports_parallel(&self, call: &ToolCall) -> bool;

    /// Dispatch a parsed tool call through the implementation-owned registry.
    fn dispatch_tool_call_with_code_mode_result(
        &self,
        session: Session,
        turn: Turn,
        cancellation_token: CancellationToken,
        tracker: Tracker,
        call: ToolCall,
        source: ToolCallSource,
    ) -> SessionToolDispatchFuture<'_>;
}

/// Factory for building a session-facing tool router for one turn.
///
/// Composition roots or owner runtime crates implement this trait by wiring
/// concrete tool handlers to the injected session capability implementation.
/// The session runtime only stores this trait object.
pub trait SessionToolRouterFactory<Session, Turn, Tracker, DiffContext>:
    Send + Sync + 'static
{
    /// Build the router for a single turn using already-resolved discovery data.
    fn build_tool_router(
        &self,
        config: &ToolsConfig,
        params: ToolRouterBuildParams<'_>,
    ) -> Arc<dyn SessionToolRouter<Session, Turn, Tracker, DiffContext>>;
}

/// Session-owned MCP call capability consumed by tool handlers.
///
/// Implementations own concrete MCP approval, elicitation, telemetry, event,
/// connector, and tool-call side effects for one live session. Tool-domain code
/// should depend on this contract through [`SessionMcpToolCallHost`] instead of
/// requiring a broad session/runtime host.
pub trait SessionMcpToolCaller<Turn>: Send + Sync + 'static {
    /// Execute one MCP tool call for the given turn and return the model-visible
    /// result plus the normalized tool input used for history/output shaping.
    fn call_mcp_tool(
        self: Arc<Self>,
        turn: &Turn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> impl Future<Output = McpToolCallOutcome> + Send + '_;
}

/// Turn-owned MCP display/output capability consumed by tool handlers.
pub trait SessionMcpToolTurn: Send + Sync + 'static {
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

/// Generic tool-handler host that delegates MCP tool calls to session API traits.
///
/// This adapter lets composition roots wire MCP tool handling through the
/// narrow session contract without requiring the broader `ToolDomainHost` to
/// implement MCP call execution.
pub struct SessionMcpToolCallHost<Session, Turn, Tracker, DiffContext> {
    _marker: PhantomData<fn() -> (Session, Turn, Tracker, DiffContext)>,
}

impl<Session, Turn, Tracker, DiffContext> Clone
    for SessionMcpToolCallHost<Session, Turn, Tracker, DiffContext>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Session, Turn, Tracker, DiffContext> Copy
    for SessionMcpToolCallHost<Session, Turn, Tracker, DiffContext>
{
}

impl<Session, Turn, Tracker, DiffContext> Default
    for SessionMcpToolCallHost<Session, Turn, Tracker, DiffContext>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext> McpToolCallHost
    for SessionMcpToolCallHost<Session, Turn, Tracker, DiffContext>
where
    Session: SessionMcpToolCaller<Turn>,
    Turn: Clone + SessionMcpToolTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = Tracker;
    type DiffContext = DiffContext;

    fn call_mcp_tool<'a>(
        &'a self,
        session: Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> impl Future<Output = McpToolCallOutcome> + Send + 'a {
        session.call_mcp_tool(turn, call_id, server, tool_name, hook_tool_name, arguments)
    }

    fn mcp_original_image_detail_supported(&self, turn: &Self::Turn) -> bool {
        turn.mcp_original_image_detail_supported()
    }

    fn mcp_truncation_policy(&self, turn: &Self::Turn) -> TruncationPolicy {
        turn.mcp_truncation_policy()
    }
}

/// Session-owned MCP resource capability consumed by resource tool handlers.
pub trait SessionMcpResourceCaller<Turn>: Send + Sync + 'static {
    /// List resources from one MCP server.
    fn list_resources(
        self: Arc<Self>,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourcesResult, String>> + Send + '_;

    /// List resources from all connected MCP servers.
    fn list_all_resources(
        self: Arc<Self>,
    ) -> impl Future<Output = HashMap<String, Vec<Resource>>> + Send;

    /// List resource templates from one MCP server.
    fn list_resource_templates(
        self: Arc<Self>,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, String>> + Send + '_;

    /// List resource templates from all connected MCP servers.
    fn list_all_resource_templates(
        self: Arc<Self>,
    ) -> impl Future<Output = HashMap<String, Vec<ResourceTemplate>>> + Send;

    /// Read a single MCP resource.
    fn read_resource(
        self: Arc<Self>,
        server: &str,
        params: ReadResourceRequestParams,
    ) -> impl Future<Output = Result<ReadResourceResult, String>> + Send + '_;

    /// Emit the resource-backed MCP lifecycle start event.
    fn emit_mcp_resource_tool_call_begin<'a>(
        self: Arc<Self>,
        turn: &'a Turn,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> impl Future<Output = ()> + Send + 'a;

    /// Emit the resource-backed MCP lifecycle completion event.
    fn emit_mcp_resource_tool_call_end<'a>(
        self: Arc<Self>,
        turn: &'a Turn,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> impl Future<Output = ()> + Send + 'a;
}

/// Generic resource-tool host that delegates MCP resource work to session API traits.
pub struct SessionMcpResourceHost<Session, Turn, Tracker, DiffContext> {
    _marker: PhantomData<fn() -> (Session, Turn, Tracker, DiffContext)>,
}

impl<Session, Turn, Tracker, DiffContext> Clone
    for SessionMcpResourceHost<Session, Turn, Tracker, DiffContext>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Session, Turn, Tracker, DiffContext> Copy
    for SessionMcpResourceHost<Session, Turn, Tracker, DiffContext>
{
}

impl<Session, Turn, Tracker, DiffContext> Default
    for SessionMcpResourceHost<Session, Turn, Tracker, DiffContext>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext> McpResourceHost
    for SessionMcpResourceHost<Session, Turn, Tracker, DiffContext>
where
    Session: SessionMcpResourceCaller<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = Tracker;
    type DiffContext = DiffContext;

    fn list_resources<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourcesResult, String>> + Send + 'a {
        Arc::clone(session).list_resources(server, params)
    }

    fn list_all_resources<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, Vec<Resource>>> + Send + 'a {
        Arc::clone(session).list_all_resources()
    }

    fn list_resource_templates<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, String>> + Send + 'a {
        Arc::clone(session).list_resource_templates(server, params)
    }

    fn list_all_resource_templates<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, Vec<ResourceTemplate>>> + Send + 'a {
        Arc::clone(session).list_all_resource_templates()
    }

    fn read_resource<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> impl Future<Output = Result<ReadResourceResult, String>> + Send + 'a {
        Arc::clone(session).read_resource(server, params)
    }

    fn emit_mcp_tool_call_begin<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> impl Future<Output = ()> + Send + 'a {
        Arc::clone(session).emit_mcp_resource_tool_call_begin(turn, call_id, invocation)
    }

    fn emit_mcp_tool_call_end<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> impl Future<Output = ()> + Send + 'a {
        Arc::clone(session)
            .emit_mcp_resource_tool_call_end(turn, call_id, invocation, duration, result)
    }
}

/// Session-owned goal capability consumed by persisted goal tool handlers.
///
/// Implementations own goal persistence, accounting, lifecycle side effects,
/// and display/model event emission. Tool-domain code should depend on this
/// contract through [`SessionGoalHost`] instead of requiring a broad
/// session/runtime host.
pub trait SessionGoalCaller<Turn>: Send + Sync + 'static {
    /// Read the current thread goal.
    fn get_thread_goal(
        self: Arc<Self>,
    ) -> impl Future<Output = Result<Option<ThreadGoal>, String>> + Send;

    /// Create a new active thread goal for the current turn.
    fn create_thread_goal(
        self: Arc<Self>,
        turn: &Turn,
        objective: String,
        token_budget: Option<i64>,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + '_;

    /// Mark the current thread goal complete through the normal goal runtime.
    fn complete_thread_goal(
        self: Arc<Self>,
        turn: &Turn,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + '_;
}

/// Generic goal-tool host that delegates goal work to session API traits.
pub struct SessionGoalHost<Session, Turn, Tracker, DiffContext> {
    _marker: PhantomData<fn() -> (Session, Turn, Tracker, DiffContext)>,
}

impl<Session, Turn, Tracker, DiffContext> Clone
    for SessionGoalHost<Session, Turn, Tracker, DiffContext>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Session, Turn, Tracker, DiffContext> Copy
    for SessionGoalHost<Session, Turn, Tracker, DiffContext>
{
}

impl<Session, Turn, Tracker, DiffContext> Default
    for SessionGoalHost<Session, Turn, Tracker, DiffContext>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext> GoalToolHost
    for SessionGoalHost<Session, Turn, Tracker, DiffContext>
where
    Session: SessionGoalCaller<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = Tracker;
    type DiffContext = DiffContext;

    fn get_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = Result<Option<ThreadGoal>, String>> + Send + 'a {
        Arc::clone(session).get_thread_goal()
    }

    fn create_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        objective: String,
        token_budget: Option<i64>,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + 'a {
        Arc::clone(session).create_thread_goal(turn, objective, token_budget)
    }

    fn complete_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + 'a {
        Arc::clone(session).complete_thread_goal(turn)
    }
}

/// Turn-owned agent-job capability consumed by CSV agent-job tools.
pub trait SessionAgentJobTurn: Send + Sync + 'static {
    /// Return the single local environment cwd used for CSV input/output.
    fn single_local_environment_cwd(&self) -> Result<AbsolutePathBuf, FunctionCallError>;

    /// Default runtime timeout for each spawned agent-job worker.
    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64>;
}

impl<Turn> SessionAgentJobTurn for Arc<Turn>
where
    Turn: SessionAgentJobTurn,
{
    fn single_local_environment_cwd(&self) -> Result<AbsolutePathBuf, FunctionCallError> {
        self.as_ref().single_local_environment_cwd()
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        self.as_ref().default_agent_job_max_runtime_seconds()
    }
}

/// Session-owned agent-job capability consumed by CSV agent-job tools.
///
/// Implementations own state DB access, subagent spawning, worker lifecycle,
/// and status subscriptions. Tool-domain code should depend on this contract
/// through [`SessionAgentJobHost`] instead of requiring a broad session/runtime
/// host.
pub trait SessionAgentJobCaller<Turn, SpawnConfig>: Send + Sync + 'static
where
    SpawnConfig: Clone + Send + Sync + 'static,
{
    /// Return the state DB runtime if this session supports agent jobs.
    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime>;

    /// Return the current thread id as a string for result attribution.
    fn agent_job_conversation_id_string(&self) -> String;

    /// Build runner options and the spawn config for agent-job workers.
    fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &Turn,
        requested_concurrency: Option<usize>,
    ) -> impl Future<Output = Result<AgentJobRunnerOptions<SpawnConfig>, FunctionCallError>> + Send + '_;

    /// Spawn one agent-job worker.
    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a Turn,
        spawn_config: SpawnConfig,
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

/// Generic agent-job host that delegates agent-job work to session API traits.
pub struct SessionAgentJobHost<Session, Turn, Tracker, DiffContext, SpawnConfig> {
    _marker: PhantomData<fn() -> (Session, Turn, Tracker, DiffContext, SpawnConfig)>,
}

impl<Session, Turn, Tracker, DiffContext, SpawnConfig> Clone
    for SessionAgentJobHost<Session, Turn, Tracker, DiffContext, SpawnConfig>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Session, Turn, Tracker, DiffContext, SpawnConfig> Copy
    for SessionAgentJobHost<Session, Turn, Tracker, DiffContext, SpawnConfig>
{
}

impl<Session, Turn, Tracker, DiffContext, SpawnConfig> Default
    for SessionAgentJobHost<Session, Turn, Tracker, DiffContext, SpawnConfig>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext, SpawnConfig> AgentJobToolHost
    for SessionAgentJobHost<Session, Turn, Tracker, DiffContext, SpawnConfig>
where
    Session: SessionAgentJobCaller<Turn, SpawnConfig>,
    Turn: Clone + SessionAgentJobTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
    SpawnConfig: Clone + Send + Sync + 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = Tracker;
    type DiffContext = DiffContext;
    type SpawnConfig = SpawnConfig;

    fn state_db(&self, session: &Self::Session) -> Option<SharedStateDbRuntime> {
        session.agent_job_state_db()
    }

    fn conversation_id_string(&self, session: &Self::Session) -> String {
        session.agent_job_conversation_id_string()
    }

    fn single_local_environment_cwd(
        &self,
        turn: &Self::Turn,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        turn.single_local_environment_cwd()
    }

    fn default_agent_job_max_runtime_seconds(&self, turn: &Self::Turn) -> Option<u64> {
        turn.default_agent_job_max_runtime_seconds()
    }

    fn build_agent_job_runner_options<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        requested_concurrency: Option<usize>,
    ) -> impl Future<Output = Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError>>
    + Send
    + 'a {
        Arc::clone(session).build_agent_job_runner_options(turn, requested_concurrency)
    }

    fn spawn_agent_job_worker<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        spawn_config: Self::SpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> impl Future<Output = Result<ThreadId, AgentJobSpawnWorkerError>> + Send + 'a {
        Arc::clone(session).spawn_agent_job_worker(turn, spawn_config, job_id, prompt)
    }

    fn shutdown_agent_job_worker<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send + 'a {
        Arc::clone(session).shutdown_agent_job_worker(thread_id)
    }

    fn get_agent_job_worker_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = AgentStatus> + Send + 'a {
        Arc::clone(session).get_agent_job_worker_status(thread_id)
    }

    fn subscribe_agent_job_worker_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = Option<watch::Receiver<AgentStatus>>> + Send + 'a {
        Arc::clone(session).subscribe_agent_job_worker_status(thread_id)
    }
}

/// Turn-owned workflow registry capability consumed by workflow tools.
pub trait SessionWorkflowTurn: Send + Sync + 'static {
    /// Load the workflow registry visible to this turn.
    fn load_workflow_registry(&self) -> WorkflowRegistry;
}

impl<Turn> SessionWorkflowTurn for Arc<Turn>
where
    Turn: SessionWorkflowTurn,
{
    fn load_workflow_registry(&self) -> WorkflowRegistry {
        self.as_ref().load_workflow_registry()
    }
}

/// Session-owned workflow runtime capability consumed by workflow tools.
///
/// Implementations own workflow controller persistence, runtime bridge wiring,
/// and conversation progress events. Tool-domain code should depend on this
/// contract through [`SessionWorkflowHost`] instead of requiring a broad
/// session/runtime host.
pub trait SessionWorkflowCaller<Turn, Tracker>: Send + Sync + 'static {
    /// Return the run controller for this live session.
    fn workflow_run_controller(self: Arc<Self>) -> Arc<dyn WorkflowRunController>;

    /// Create the workflow SDK runtime bridge for one running workflow action.
    fn create_workflow_runtime_bridge(
        self: Arc<Self>,
        turn: Turn,
        cancellation_token: CancellationToken,
        tracker: Tracker,
    ) -> Arc<dyn WorkflowRuntimeBridge>;

    /// Record a user/model-visible workflow progress event.
    fn record_workflow_progress<'a>(
        self: Arc<Self>,
        turn: &'a Turn,
        run: &'a WorkflowRun,
        kind: WorkflowRunProgressKind,
    ) -> impl Future<Output = ()> + Send + 'a;
}

/// Generic workflow-tool host that delegates workflow work to session API traits.
pub struct SessionWorkflowHost<Session, Turn, Tracker, DiffContext> {
    _marker: PhantomData<fn() -> (Session, Turn, Tracker, DiffContext)>,
}

impl<Session, Turn, Tracker, DiffContext> Clone
    for SessionWorkflowHost<Session, Turn, Tracker, DiffContext>
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<Session, Turn, Tracker, DiffContext> Copy
    for SessionWorkflowHost<Session, Turn, Tracker, DiffContext>
{
}

impl<Session, Turn, Tracker, DiffContext> Default
    for SessionWorkflowHost<Session, Turn, Tracker, DiffContext>
{
    fn default() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<Session, Turn, Tracker, DiffContext> codex_tool_runtime_api::WorkflowToolHost
    for SessionWorkflowHost<Session, Turn, Tracker, DiffContext>
where
    Session: SessionWorkflowCaller<Turn, Tracker>,
    Turn: Clone + SessionWorkflowTurn,
    Tracker: Clone + Send + Sync + 'static,
    DiffContext: 'static,
{
    type Session = Arc<Session>;
    type Turn = Turn;
    type Tracker = Tracker;
    type DiffContext = DiffContext;

    fn load_workflow_registry(&self, turn: &Self::Turn) -> WorkflowRegistry {
        turn.load_workflow_registry()
    }

    fn workflow_run_controller(&self, session: &Self::Session) -> Arc<dyn WorkflowRunController> {
        Arc::clone(session).workflow_run_controller()
    }

    fn create_workflow_runtime_bridge(
        &self,
        session: Self::Session,
        turn: Self::Turn,
        cancellation_token: CancellationToken,
        tracker: Self::Tracker,
    ) -> Arc<dyn WorkflowRuntimeBridge> {
        session.create_workflow_runtime_bridge(turn, cancellation_token, tracker)
    }

    fn record_workflow_progress<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        run: &'a WorkflowRun,
        kind: WorkflowRunProgressKind,
    ) -> impl Future<Output = ()> + Send + 'a {
        Arc::clone(session).record_workflow_progress(turn, run, kind)
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
