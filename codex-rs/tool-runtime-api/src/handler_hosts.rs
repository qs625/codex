use crate::ApplyPatchRuntimeHost;
use crate::ShellRuntimeHost;
use crate::ToolEventHost;
use crate::ToolOrchestratorHost;
use crate::ToolSandboxContext;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::ListedAgent;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_command_runtime::CommandNotificationFilter;
use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_exec_server_api::ExecEnvironment;
use codex_file_system::FileSystemSandboxContext;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_process_exec::ExecParams;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::dynamic_tools::DynamicToolResponse;
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
use codex_protocol::models::SandboxPermissions;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_config::ToolUserShellType;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::RequestPluginInstallElicitationRequest;
use codex_tool_planning::ToolName;
use codex_tool_types::FunctionCallError;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowRun;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::WorkflowRuntimeBridge;
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::ApplyPatchEnvironment;
use crate::RuntimeShell;
use crate::ShellRuntimeBackend;

pub trait ApplyPatchDiffContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolPermissionGrants {
    pub session: Option<AdditionalPermissionProfile>,
    pub turn: Option<AdditionalPermissionProfile>,
}

pub struct ResolvedApplyPatchEnvironment {
    pub cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ApplyPatchEnvironment>,
}

/// Host capabilities required by the apply-patch handler.
///
/// The handler owns parsing and permission composition; the host owns session
/// state, filesystem environment lookup, approval policy, sandbox runtime, and
/// conversation event emission.
pub trait ApplyPatchHandlerHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: ApplyPatchDiffContext + 'static;
    type RuntimeHost: ApplyPatchRuntimeHost<Session = Self::Session, Turn = Self::Turn>
        + Send
        + Sync
        + 'static;
    type OrchestratorHost: ToolOrchestratorHost<
            Self::Session,
            Self::Turn,
            <Self::RuntimeHost as ApplyPatchRuntimeHost>::NetworkApprovalTrigger,
        > + Send
        + Sync
        + 'static;
    type EventHost<'a>: ToolEventHost + Send + 'a
    where
        Self: 'a,
        Self::Session: 'a,
        Self::Turn: 'a,
        Self::Tracker: 'a;

    fn runtime_host(&self) -> Self::RuntimeHost;

    fn orchestrator_host(&self) -> Self::OrchestratorHost;

    fn sandbox_runtime(&self, session: &Self::Session) -> SharedSandboxRuntime;

    fn tool_sandbox_context(&self, turn: &Self::Turn) -> ToolSandboxContext;

    fn approval_policy(&self, turn: &Self::Turn) -> AskForApproval;

    fn permission_profile(&self, turn: &Self::Turn) -> PermissionProfile;

    fn file_system_sandbox_policy(&self, turn: &Self::Turn) -> FileSystemSandboxPolicy;

    fn windows_sandbox_level(&self, turn: &Self::Turn) -> WindowsSandboxLevel;

    fn file_system_sandbox_context(
        &self,
        turn: &Self::Turn,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext;

    fn resolve_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;

    fn permission_grants<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = ToolPermissionGrants> + Send + 'a;

    fn event_host<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        tracker: Option<&'a Self::Tracker>,
    ) -> Self::EventHost<'a>;
}

pub struct RunExecLikeArgs<Session, Turn, Tracker> {
    pub tool_name: ToolName,
    pub exec_params: ExecParams,
    pub hook_command: String,
    pub shell_type: Option<ToolUserShellType>,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub prefix_rule: Option<Vec<String>>,
    pub session: Session,
    pub turn: Turn,
    pub tracker: Tracker,
    pub call_id: String,
    pub freeform: bool,
    pub shell_runtime_backend: ShellRuntimeBackend,
}

pub struct ResolvedExecCommandEnvironment {
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub apply_patch_environment: Arc<dyn ApplyPatchEnvironment>,
}

#[derive(Clone, Debug)]
pub struct ResolvedExecCommand {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
}

#[derive(Clone)]
pub struct ExecCommandRunRequest {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
    pub hook_command: String,
    pub process_id: i32,
    pub yield_time_ms: u64,
    pub max_output_tokens: Option<usize>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    pub additional_permissions_preapproved: bool,
    pub justification: Option<String>,
    pub prefix_rule: Option<Vec<String>>,
    pub notify_on: CommandNotificationFilter,
}

pub struct ExecCommandRunOutput {
    pub event_call_id: String,
    pub chunk_id: String,
    pub wall_time: Duration,
    pub raw_output: Vec<u8>,
    pub max_output_tokens: Option<usize>,
    pub process_id: Option<i32>,
    pub exit_code: Option<i32>,
    pub original_token_count: Option<usize>,
    pub hook_command: Option<String>,
}

/// Host capabilities required by the model-visible `exec_command` handler.
///
/// The handler owns argument parsing, hook payloads, permission composition,
/// apply-patch interception, and output shaping. The host owns shell discovery,
/// concrete environment lookup, process id allocation, and the unified-exec
/// process manager.
pub trait ExecCommandHandlerHost: ShellExecutionHost {
    fn resolve_exec_command_environment(
        &self,
        turn: &Self::Turn,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError>;

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell;

    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> Result<ResolvedExecCommand, String>;

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn allocate_exec_process_id<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = i32> + Send + 'a;

    fn release_exec_process_id<'a>(
        &'a self,
        session: &'a Self::Session,
        process_id: i32,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn run_exec_command<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        request: ExecCommandRunRequest,
    ) -> impl Future<Output = Result<ExecCommandRunOutput, UnifiedExecError>> + Send + 'a;

    fn emit_unified_exec_tty_metric(&self, turn: &Self::Turn, tty: bool);
}

/// Host capabilities required by shell-like handlers before delegating to the
/// shell runtime.
pub trait ShellExecutionHost: ApplyPatchHandlerHost {
    type ShellHost: ShellRuntimeHost<Session = Self::Session, Turn = Self::Turn>
        + Send
        + Sync
        + 'static;
    type ShellOrchestratorHost: ToolOrchestratorHost<
            Self::Session,
            Self::Turn,
            <Self::ShellHost as ShellRuntimeHost>::NetworkApprovalTrigger,
        > + Send
        + Sync
        + 'static;

    fn shell_runtime_host(&self) -> Self::ShellHost;

    fn shell_orchestrator_host(&self) -> Self::ShellOrchestratorHost;

    fn primary_environment(
        &self,
        turn: &Self::Turn,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError>;

    fn dependency_env<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, String>> + Send + 'a;

    fn explicit_env_overrides(&self, turn: &Self::Turn) -> HashMap<String, String>;

    fn exec_permission_approvals_enabled(&self, session: &Self::Session) -> bool;

    fn request_permissions_tool_enabled(&self, session: &Self::Session) -> bool;

    fn create_exec_approval_requirement<'a>(
        &'a self,
        session: &'a Self::Session,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> impl Future<Output = crate::ExecApprovalRequirement> + Send + 'a;

    fn truncation_policy(&self, turn: &Self::Turn) -> TruncationPolicy;
}

/// Host capabilities required by the model-visible `shell_command` handler.
///
/// The handler owns hook integration and shell-runtime dispatch, while the host
/// keeps session/turn-specific path resolution, environment construction, and
/// implicit skill invocation logic outside the runtime implementation crate.
pub trait ShellCommandHandlerHost: ShellExecutionHost {
    fn resolve_workdir_base_path(
        &self,
        turn: &Self::Turn,
        arguments: &str,
    ) -> Result<AbsolutePathBuf, FunctionCallError>;

    fn parse_shell_command_params(
        &self,
        arguments: &str,
        base_path: &AbsolutePathBuf,
    ) -> Result<ShellCommandToolCallParams, FunctionCallError>;

    fn resolve_shell_workdir(&self, turn: &Self::Turn, workdir: Option<String>) -> AbsolutePathBuf;

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn shell_command_exec_params(
        &self,
        params: &ShellCommandToolCallParams,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> Result<ExecParams, FunctionCallError>;

    fn shell_type(&self, session: &Self::Session) -> Option<ToolUserShellType>;
}

/// Host capabilities required by command-session interaction handlers.
///
/// `command_wait` and `command_write_stdin` own argument parsing and typed item
/// construction. The host owns access to the concrete command session
/// controller and the conversation display/event sinks.
pub trait CommandInteractionHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn new_response_item_id(&self) -> String;

    fn begin_command_wait<'a>(
        &'a self,
        session: &'a Self::Session,
        request: CommandWaitRequest,
    ) -> impl Future<Output = Result<Box<dyn CommandWaitOperation>, CommandSessionError>> + Send + 'a;

    fn write_command_stdin<'a>(
        &'a self,
        session: &'a Self::Session,
        request: WriteStdinRequest<'a>,
    ) -> impl Future<Output = Result<WriteStdinOutput, CommandSessionError>> + Send + 'a;

    fn emit_model_item_started_display_event<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        item: &'a ResponseItem,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        items: &'a [ResponseItem],
    ) -> impl Future<Output = ()> + Send + 'a;

    fn send_terminal_interaction<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        event: TerminalInteractionEvent,
    ) -> impl Future<Output = ()> + Send + 'a;
}

#[derive(Clone, Debug)]
pub struct McpToolCallOutcome {
    pub result: CallToolResult,
    pub tool_input: serde_json::Value,
}

/// Host capabilities required by model-visible MCP tool-call handlers.
///
/// `codex-tool-runtime` owns MCP tool spec/search/hook semantics and output
/// shaping. The embedding host owns the concrete MCP call lifecycle, approval,
/// connector policy, telemetry side effects, and display event emission.
pub trait McpToolCallHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn call_mcp_tool<'a>(
        &'a self,
        session: Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> impl Future<Output = McpToolCallOutcome> + Send + 'a;

    fn mcp_original_image_detail_supported(&self, turn: &Self::Turn) -> bool;

    fn mcp_truncation_policy(&self, turn: &Self::Turn) -> TruncationPolicy;
}

/// Host capabilities required by MCP resource handlers.
///
/// The handlers own argument parsing, output serialization, and MCP tool-call
/// lifecycle shaping. The host owns access to the concrete MCP manager and the
/// conversation event sinks for the current session/turn.
pub trait McpResourceHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn list_resources<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourcesResult, String>> + Send + 'a;

    fn list_all_resources<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, Vec<Resource>>> + Send + 'a;

    fn list_resource_templates<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, String>> + Send + 'a;

    fn list_all_resource_templates<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, Vec<ResourceTemplate>>> + Send + 'a;

    fn read_resource<'a>(
        &'a self,
        session: &'a Self::Session,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> impl Future<Output = Result<ReadResourceResult, String>> + Send + 'a;

    fn emit_mcp_tool_call_begin<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        invocation: McpInvocation,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn emit_mcp_tool_call_end<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        invocation: McpInvocation,
        duration: Duration,
        result: Result<CallToolResult, String>,
    ) -> impl Future<Output = ()> + Send + 'a;
}

/// Host capabilities required by regular function-style tool handlers.
///
/// These methods keep session/turn state, event sinks, and external client
/// request/response wiring in the embedding crate while `codex-tool-runtime`
/// owns argument parsing, model-visible output shaping, and tool specs.
pub trait FunctionToolHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn turn_collaboration_mode(&self, turn: &Self::Turn) -> ModeKind;

    fn turn_cwd(&self, turn: &Self::Turn) -> AbsolutePathBuf;

    fn turn_id(&self, turn: &Self::Turn) -> String;

    fn turn_is_non_root_agent(&self, turn: &Self::Turn) -> bool;

    fn turn_supports_image_input(&self, turn: &Self::Turn) -> bool;

    fn turn_can_request_original_image_detail(&self, turn: &Self::Turn) -> bool;

    fn session_collaboration_mode<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = ModeKind> + Send + 'a;

    fn emit_plan_update<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        args: UpdatePlanArgs,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn emit_image_view<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        path: AbsolutePathBuf,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn request_permissions<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        args: RequestPermissionsArgs,
        cancellation_token: CancellationToken,
    ) -> impl Future<Output = Option<RequestPermissionsResponse>> + Send + 'a;

    fn request_user_input<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> impl Future<Output = Option<RequestUserInputResponse>> + Send + 'a;

    fn request_dynamic_tool<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> impl Future<Output = Option<DynamicToolResponse>> + Send + 'a;
}

/// Host capabilities required by persisted goal model tools.
///
/// Goal tools are part of the tool domain; persistence, runtime accounting,
/// and thread events stay behind the embedding host.
pub trait GoalToolHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn get_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = Result<Option<ThreadGoal>, String>> + Send + 'a;

    fn create_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        objective: String,
        token_budget: Option<i64>,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + 'a;

    fn complete_thread_goal<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
    ) -> impl Future<Output = Result<ThreadGoal, String>> + Send + 'a;
}

pub struct AgentJobRunnerOptions<SpawnConfig> {
    pub max_concurrency: usize,
    pub spawn_config: SpawnConfig,
}

pub enum AgentJobSpawnWorkerError {
    LimitReached,
    Other(String),
}

/// Host capabilities required by agent-job tools.
///
/// The tool runtime owns CSV/job orchestration and state-api mutation. The host
/// supplies core-owned agent spawning, live status, environment selection, and
/// parent-turn config preparation.
pub trait AgentJobToolHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;
    type SpawnConfig: Clone + Send + Sync + 'static;

    fn state_db(&self, session: &Self::Session) -> Option<SharedStateDbRuntime>;

    fn conversation_id_string(&self, session: &Self::Session) -> String;

    fn single_local_environment_cwd(
        &self,
        turn: &Self::Turn,
    ) -> Result<AbsolutePathBuf, FunctionCallError>;

    fn default_agent_job_max_runtime_seconds(&self, turn: &Self::Turn) -> Option<u64>;

    fn build_agent_job_runner_options<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        requested_concurrency: Option<usize>,
    ) -> impl Future<Output = Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError>>
    + Send
    + 'a;

    fn spawn_agent_job_worker<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        spawn_config: Self::SpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> impl Future<Output = Result<ThreadId, AgentJobSpawnWorkerError>> + Send + 'a;

    fn shutdown_agent_job_worker<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn get_agent_job_worker_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = AgentStatus> + Send + 'a;

    fn subscribe_agent_job_worker_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = Option<watch::Receiver<AgentStatus>>> + Send + 'a;
}

/// Host capabilities required by workflow tools.
///
/// The tool runtime owns argument parsing, tool specs, workflow progress item
/// shaping, and handler ordering. The embedding host owns workflow registry
/// roots, run controller persistence/runner wiring, and the bridge from
/// workflow SDK runtime calls back into Codex primitives.
pub trait WorkflowToolHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn load_workflow_registry(&self, turn: &Self::Turn) -> WorkflowRegistry;

    fn workflow_run_controller(&self, session: &Self::Session) -> Arc<dyn WorkflowRunController>;

    fn create_workflow_runtime_bridge(
        &self,
        session: Self::Session,
        turn: Self::Turn,
        cancellation_token: CancellationToken,
        tracker: Self::Tracker,
    ) -> Arc<dyn WorkflowRuntimeBridge>;

    fn record_workflow_progress<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        run: &'a WorkflowRun,
        kind: WorkflowRunProgressKind,
    ) -> impl Future<Output = ()> + Send + 'a;
}

#[derive(Clone, Debug)]
pub struct SpawnAgentToolRequest {
    pub message: String,
    pub task_name: String,
    pub agent_type: Option<String>,
    pub cwd: Option<AbsolutePathBuf>,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub service_tier: Option<String>,
    pub agent_mode: Option<AgentMode>,
    pub fork_mode: Option<SpawnAgentForkMode>,
}

#[derive(Debug, serde::Serialize)]
#[serde(untagged)]
pub enum SpawnAgentToolResult {
    WithNickname {
        task_name: String,
        nickname: Option<String>,
    },
    HiddenMetadata {
        task_name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitAgentReason {
    PendingMessage,
    MailboxMessage,
    StatusUpdate,
    FinalStatus,
    Timeout,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WaitAgentToolResult {
    pub target: String,
    pub agent_name: String,
    pub reason: WaitAgentReason,
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

#[derive(Debug, serde::Serialize)]
pub struct CloseAgentToolResult {
    pub previous_status: AgentStatus,
}

#[derive(Debug, serde::Serialize)]
pub struct ListAgentsToolResult {
    pub agents: Vec<ListedAgent>,
}

/// Host capabilities required by MultiAgent V2 collaboration tools.
///
/// The tool runtime owns tool specs, argument parsing, result shaping, and
/// wait/backoff/event sequencing. The embedding host owns concrete
/// AgentControl, Session/Turn state, and spawn config construction.
pub trait MultiAgentToolHost: Clone + Send + Sync + 'static {
    type Session: Clone + Send + Sync + 'static;
    type Turn: Clone + Send + Sync + 'static;
    type Tracker: Clone + Send + Sync + 'static;
    type DiffContext: 'static;

    fn thread_id(&self, session: &Self::Session) -> ThreadId;

    fn sender_agent_path(&self, session: &Self::Session, turn: &Self::Turn) -> AgentPath;

    fn send_collab_event<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        event: EventMsg,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn resolve_agent_target<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        target: &'a str,
    ) -> impl Future<Output = Result<ThreadId, FunctionCallError>> + Send + 'a;

    fn agent_metadata(&self, session: &Self::Session, thread_id: ThreadId) -> AgentMetadata;

    fn agent_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = AgentStatus> + Send + 'a;

    fn subscribe_agent_status<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = Result<watch::Receiver<AgentStatus>, FunctionCallError>> + Send + 'a;

    fn subscribe_mailbox_seq(&self, session: &Self::Session) -> watch::Receiver<u64>;

    fn find_pending_inter_agent_communication<'a>(
        &'a self,
        session: &'a Self::Session,
        receiver_thread_id: ThreadId,
        receiver_agent_path: &'a AgentPath,
    ) -> impl Future<Output = Option<InterAgentCommunication>> + Send + 'a;

    fn wait_agent_current_window<'a>(
        &'a self,
        session: &'a Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> impl Future<Output = Duration> + Send + 'a;

    fn advance_wait_agent_backoff<'a>(
        &'a self,
        session: &'a Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn reset_wait_agent_backoff<'a>(
        &'a self,
        session: &'a Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn wait_agent_timeouts(&self, turn: &Self::Turn) -> (i64, i64);

    fn register_session_root(&self, session: &Self::Session, turn: &Self::Turn);

    fn list_agents<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        path_prefix: Option<&'a str>,
    ) -> impl Future<Output = Result<Vec<ListedAgent>, FunctionCallError>> + Send + 'a;

    fn send_followup_task<'a>(
        &'a self,
        session: &'a Self::Session,
        sender_agent_path: AgentPath,
        receiver_thread_id: ThreadId,
        receiver_agent_path: AgentPath,
        prompt: String,
    ) -> impl Future<Output = Result<(), FunctionCallError>> + Send + 'a;

    fn mark_direct_child_completion_pending<'a>(
        &'a self,
        session: &'a Self::Session,
        receiver_thread_id: ThreadId,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn mark_direct_child_completion_received<'a>(
        &'a self,
        session: &'a Self::Session,
        receiver_thread_id: ThreadId,
    ) -> impl Future<Output = bool> + Send + 'a;

    fn clear_direct_child_completion_pending<'a>(
        &'a self,
        session: &'a Self::Session,
        receiver_thread_id: ThreadId,
    ) -> impl Future<Output = bool> + Send + 'a;

    fn maybe_notify_parent_of_final_status<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn close_agent<'a>(
        &'a self,
        session: &'a Self::Session,
        thread_id: ThreadId,
    ) -> impl Future<Output = Result<(), FunctionCallError>> + Send + 'a;

    fn spawn_agent<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        request: SpawnAgentToolRequest,
    ) -> impl Future<Output = Result<SpawnAgentToolResult, FunctionCallError>> + Send + 'a;
}

pub struct RequestPluginInstallContext {
    pub server_name: String,
    pub thread_id: String,
    pub turn_id: String,
    pub app_server_client_name: Option<String>,
}

pub struct RequestPluginInstallElicitationOutcome {
    pub user_confirmed: bool,
}

/// Host capabilities required by the `request_plugin_install` tool.
///
/// The runtime handler owns the model-visible tool contract, argument
/// validation, elicitation request construction, and model output shaping. The
/// host owns concrete connector/plugin discovery, MCP elicitation dispatch,
/// persistence side effects, and completion verification.
pub trait RequestPluginInstallHost: ApplyPatchHandlerHost {
    fn request_plugin_install_context(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> RequestPluginInstallContext;

    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
    ) -> impl Future<Output = Result<Vec<DiscoverableTool>, FunctionCallError>> + Send + 'a;

    fn request_plugin_install_elicitation<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> impl Future<Output = RequestPluginInstallElicitationOutcome> + Send + 'a;

    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        session: &'a Self::Session,
        turn: &'a Self::Turn,
        tool: &'a DiscoverableTool,
    ) -> impl Future<Output = bool> + Send + 'a;
}

/// Host capabilities required by code-mode execute/wait tools.
///
/// The runtime handler owns the model-visible tool contract, argument parsing,
/// response shaping, and code-mode wait/execute sequencing. The host owns the
/// concrete code-mode runtime service, rollout trace lifecycle, image detail
/// policy, and stored value persistence.
pub trait CodeModeToolHost: ApplyPatchHandlerHost {
    fn code_mode_turn_id(&self, turn: &Self::Turn) -> String;

    fn can_request_original_image_detail(&self, turn: &Self::Turn) -> bool;

    fn code_mode_stored_values<'a>(
        &'a self,
        session: &'a Self::Session,
    ) -> impl Future<Output = HashMap<String, serde_json::Value>> + Send + 'a;

    fn code_mode_replace_stored_values<'a>(
        &'a self,
        session: &'a Self::Session,
        values: HashMap<String, serde_json::Value>,
    ) -> impl Future<Output = ()> + Send + 'a;

    fn code_mode_allocate_cell_id(&self, session: &Self::Session) -> String;

    fn code_mode_execute<'a>(
        &'a self,
        session: &'a Self::Session,
        request: ExecuteRequest,
    ) -> impl Future<Output = Result<RuntimeResponse, String>> + Send + 'a;

    fn code_mode_wait<'a>(
        &'a self,
        session: &'a Self::Session,
        request: WaitRequest,
    ) -> impl Future<Output = Result<WaitOutcome, String>> + Send + 'a;

    fn record_code_mode_cell_started(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    );

    fn record_code_mode_cell_initial_response(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    );

    fn record_code_mode_cell_ended(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    );
}

/// Coarse host facade for the tool domain.
///
/// This is the migration target for moving `core/src/tools` as a domain: tool
/// runtime code should depend on this typed service facade, while `codex-core`
/// implements it with `Session` / `TurnContext` adapters. Narrow subtraits may
/// still exist to document focused capability sets, but new tool-domain moves
/// should compose through this facade instead of growing one-off core-facing
/// adapter traits.
pub trait ToolDomainHost:
    ShellCommandHandlerHost
    + CommandInteractionHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + McpResourceHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + McpToolCallHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + FunctionToolHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + GoalToolHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + AgentJobToolHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + WorkflowToolHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + ExecCommandHandlerHost
    + MultiAgentToolHost<
        Session = <Self as ApplyPatchHandlerHost>::Session,
        Turn = <Self as ApplyPatchHandlerHost>::Turn,
        Tracker = <Self as ApplyPatchHandlerHost>::Tracker,
        DiffContext = <Self as ApplyPatchHandlerHost>::DiffContext,
    > + RequestPluginInstallHost
    + CodeModeToolHost
{
}

impl<T> ToolDomainHost for T where
    T: ShellCommandHandlerHost
        + CommandInteractionHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + McpResourceHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + McpToolCallHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + FunctionToolHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + GoalToolHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + AgentJobToolHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + WorkflowToolHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + ExecCommandHandlerHost
        + MultiAgentToolHost<
            Session = <T as ApplyPatchHandlerHost>::Session,
            Turn = <T as ApplyPatchHandlerHost>::Turn,
            Tracker = <T as ApplyPatchHandlerHost>::Tracker,
            DiffContext = <T as ApplyPatchHandlerHost>::DiffContext,
        > + RequestPluginInstallHost
        + CodeModeToolHost
{
}
