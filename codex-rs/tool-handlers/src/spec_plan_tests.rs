use super::*;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::ListedAgent;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::ExecOptions;
use codex_command_runtime::ExecServerEnvConfig;
use codex_command_runtime::SpawnLifecycleHandle;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::UnifiedExecProcess;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_connectors_types::AppInfo;
use codex_exec_server_api::ExecEnvironment;
use codex_extension_api::ExtensionToolExecutor;
use codex_extension_api::ToolCall as ExtensionToolCall;
use codex_extension_api::ToolExecutor;
use codex_features::Feature;
use codex_features::Features;
use codex_file_system::ExecutorFileSystem;
use codex_file_system::FileSystemSandboxContext;
use codex_mcp_tool_types::McpTool;
use codex_mcp_tool_types::ToolInfo;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_process_exec::ExecParams;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::WebSearchConfig;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::error::Result as ProtocolResult;
use codex_protocol::exec_output::ExecToolCallOutput;
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
use codex_protocol::models::SandboxPermissions;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_protocol::models::VIEW_IMAGE_TOOL_NAME;
use codex_protocol::models::WorkflowRunProgressKind;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::WebSearchToolType;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_protocol::request_permissions::RequestPermissionsResponse;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformError;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_config::ToolUserShellType;
use codex_tool_planning::AdditionalProperties;
use codex_tool_planning::CommandToolOptions;
use codex_tool_planning::DiscoverablePluginInfo;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::FreeformTool;
use codex_tool_planning::JsonSchema;
use codex_tool_planning::JsonSchemaPrimitiveType;
use codex_tool_planning::JsonSchemaType;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
use codex_tool_planning::REQUEST_USER_INPUT_TOOL_NAME;
use codex_tool_planning::RequestPluginInstallElicitationRequest;
use codex_tool_planning::ResponsesApiNamespaceTool;
use codex_tool_planning::ResponsesApiTool;
use codex_tool_planning::ResponsesApiWebSearchFilters;
use codex_tool_planning::ResponsesApiWebSearchUserLocation;
use codex_tool_planning::SpawnAgentToolOptions;
use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
use codex_tool_planning::ToolEnvironmentMode;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::ToolsConfig;
use codex_tool_planning::ToolsConfigParams;
use codex_tool_planning::ViewImageToolOptions;
use codex_tool_planning::create_apply_patch_freeform_tool;
use codex_tool_planning::create_close_agent_tool_v2;
use codex_tool_planning::create_command_wait_tool;
use codex_tool_planning::create_create_goal_tool;
use codex_tool_planning::create_exec_command_tool;
use codex_tool_planning::create_followup_task_tool;
use codex_tool_planning::create_get_goal_tool;
use codex_tool_planning::create_image_generation_tool;
use codex_tool_planning::create_list_agents_tool;
use codex_tool_planning::create_request_permissions_tool;
use codex_tool_planning::create_request_user_input_tool;
use codex_tool_planning::create_spawn_agent_tool_v2;
use codex_tool_planning::create_update_goal_tool;
use codex_tool_planning::create_update_plan_tool;
use codex_tool_planning::create_view_image_tool;
use codex_tool_planning::create_wait_agent_tool_v2;
use codex_tool_planning::create_workflow_abort_tool;
use codex_tool_planning::create_workflow_describe_tool;
use codex_tool_planning::create_workflow_list_tool;
use codex_tool_planning::create_workflow_resume_tool;
use codex_tool_planning::create_workflow_start_tool;
use codex_tool_planning::create_workflow_status_tool;
use codex_tool_planning::create_write_stdin_tool;
use codex_tool_planning::hosted_model_tool_specs;
use codex_tool_planning::mcp_call_tool_result_output_schema;
use codex_tool_planning::request_permissions_tool_description;
use codex_tool_planning::request_user_input_available_modes;
use codex_tool_planning::request_user_input_tool_description;
use codex_tool_runtime_api::AgentJobRunnerOptions;
use codex_tool_runtime_api::AgentJobSpawnWorkerError;
use codex_tool_runtime_api::AgentJobToolHost;
use codex_tool_runtime_api::ApplyPatchApprovalKey;
use codex_tool_runtime_api::ApplyPatchApprovalRequest;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ApplyPatchEnvironment;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::ApplyPatchRequest;
use codex_tool_runtime_api::ApplyPatchRuntimeHost;
use codex_tool_runtime_api::ApprovalCtx;
use codex_tool_runtime_api::CloseAgentToolResult;
use codex_tool_runtime_api::CodeModeToolHost;
use codex_tool_runtime_api::CommandInteractionHost;
use codex_tool_runtime_api::ExecApprovalRequirement;
use codex_tool_runtime_api::ExecCommandHandlerHost;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::FunctionToolHost;
use codex_tool_runtime_api::GoalToolHost;
use codex_tool_runtime_api::ListAgentsToolResult;
use codex_tool_runtime_api::McpResourceHost;
use codex_tool_runtime_api::McpToolCallHost;
use codex_tool_runtime_api::McpToolCallOutcome;
use codex_tool_runtime_api::MultiAgentToolHost;
use codex_tool_runtime_api::NetworkApprovalMode;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::OrchestratorRunResult;
use codex_tool_runtime_api::PermissionRequestPayload;
use codex_tool_runtime_api::RequestPluginInstallContext;
use codex_tool_runtime_api::RequestPluginInstallElicitationOutcome;
use codex_tool_runtime_api::RequestPluginInstallHost;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ResolvedExecCommand;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use codex_tool_runtime_api::RunExecLikeArgs;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::SandboxAttempt;
use codex_tool_runtime_api::ShellCommandHandlerHost;
use codex_tool_runtime_api::ShellExecutionHost;
use codex_tool_runtime_api::ShellRuntimeBackend;
use codex_tool_runtime_api::ShellRuntimeHost;
use codex_tool_runtime_api::SpawnAgentToolRequest;
use codex_tool_runtime_api::SpawnAgentToolResult;
use codex_tool_runtime_api::ToolError;
use codex_tool_runtime_api::ToolEventHost;
use codex_tool_runtime_api::ToolOrchestratorHost;
use codex_tool_runtime_api::ToolPatchTrackerUpdate;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_runtime_api::ToolSandboxContext;
use codex_tool_runtime_api::UnifiedExecRuntimeHost;
use codex_tool_runtime_api::WaitAgentToolResult;
use codex_tool_runtime_api::WorkflowToolHost;
use codex_tool_types::FunctionCallError;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_workflow_api::WorkflowRegistry;
use codex_workflow_api::WorkflowRun;
use codex_workflow_api::WorkflowRunController;
use codex_workflow_api::WorkflowRuntimeBridge;
use futures::future::BoxFuture;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const CODEX_APPS_MCP_SERVER_NAME: &str = "codex_apps";
const DEFAULT_AGENT_TYPE_DESCRIPTION: &str = "Test agent type description.";

type SpecInvocation = ToolInvocation<SpecSession, SpecTurn, SpecTracker>;
type SpecToolRegistry = ToolRegistry<SpecInvocation, SpecDiffContext>;

#[derive(Clone)]
struct SpecOnlyToolDomainHost;

#[derive(Clone)]
struct SpecSession;

#[derive(Clone)]
struct SpecTurn;

#[derive(Clone)]
struct SpecTracker;

struct SpecDiffContext;

#[derive(Clone)]
struct SpecApplyPatchRuntimeHost;

#[derive(Clone)]
struct SpecShellRuntimeHost;

#[derive(Clone)]
struct SpecOrchestratorHost;

struct SpecEventHost;

fn spec_only<T>() -> T {
    panic!("spec-only tool-domain host method should not execute")
}

fn spec_cwd() -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(Path::new("/tmp")).expect("absolute test path")
}

impl ApplyPatchDiffContext for SpecDiffContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        false
    }
}

impl ApplyPatchRuntimeHost for SpecApplyPatchRuntimeHost {
    type NetworkApprovalTrigger = ();
    type Session = SpecSession;
    type Turn = SpecTurn;

    fn start_apply_patch_approval_async<'a>(
        &'a self,
        _req: &'a ApplyPatchRequest,
        _ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
        _keys: Vec<ApplyPatchApprovalKey>,
        _approval_request: ApplyPatchApprovalRequest,
    ) -> BoxFuture<'a, ReviewDecision> {
        Box::pin(async { spec_only() })
    }
}

impl ToolOrchestratorHost<SpecSession, SpecTurn, ()> for SpecOrchestratorHost {
    type ActiveNetworkApproval = ();
    type DeferredNetworkApproval = ();

    async fn strict_auto_review_enabled_for_turn(&self, _session: &SpecSession) -> bool {
        spec_only()
    }

    fn routes_approval_to_guardian(&self, _turn: &SpecTurn) -> bool {
        spec_only()
    }

    fn new_guardian_review_id(&self) -> String {
        spec_only()
    }

    async fn guardian_rejection_message(&self, _session: &SpecSession, _review_id: &str) -> String {
        spec_only()
    }

    fn guardian_timeout_message(&self) -> String {
        spec_only()
    }

    async fn run_permission_request_hooks(
        &self,
        _session: &SpecSession,
        _turn: &SpecTurn,
        _permission_request_run_id: &str,
        _permission_request: PermissionRequestPayload,
    ) -> Option<codex_hooks_api::PermissionRequestDecision> {
        spec_only()
    }

    async fn begin_network_approval(
        &self,
        _session: &SpecSession,
        _turn_id: &str,
        _managed_network_active: bool,
        _spec: Option<NetworkApprovalSpec<()>>,
    ) -> Option<Self::ActiveNetworkApproval> {
        spec_only()
    }

    fn active_network_approval_mode(
        &self,
        _active: &Self::ActiveNetworkApproval,
    ) -> NetworkApprovalMode {
        spec_only()
    }

    fn active_network_approval_cancellation_token(
        &self,
        _active: &Self::ActiveNetworkApproval,
    ) -> CancellationToken {
        spec_only()
    }

    fn into_deferred_network_approval(
        &self,
        _active: Self::ActiveNetworkApproval,
    ) -> Option<Self::DeferredNetworkApproval> {
        spec_only()
    }

    async fn finish_immediate_network_approval(
        &self,
        _session: &SpecSession,
        _active: Self::ActiveNetworkApproval,
    ) -> Result<(), ToolError> {
        spec_only()
    }

    async fn finish_deferred_network_approval(
        &self,
        _session: &SpecSession,
        _deferred: Option<Self::DeferredNetworkApproval>,
    ) -> Result<(), ToolError> {
        spec_only()
    }
}

impl ToolEventHost for SpecEventHost {
    fn turn_id(&self) -> &str {
        "spec-turn"
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        TruncationPolicy::Bytes(10_000)
    }

    async fn send_exec_command_begin(&self, _event: ExecCommandBeginEvent) {}

    async fn send_exec_command_end(&self, _event: ExecCommandEndEvent) {}

    async fn emit_file_change_started(&self, _item: FileChangeItem) {}

    async fn emit_file_change_completed(&self, _item: FileChangeItem) {}

    async fn record_model_items_and_emit_display_events(&self, _items: Vec<ResponseItem>) {}

    async fn update_patch_diff<'a>(&'a self, _tracker_update: ToolPatchTrackerUpdate<'a>) {}
}

impl ApplyPatchHandlerHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type EventHost<'a> = SpecEventHost;
    type OrchestratorHost = SpecOrchestratorHost;
    type RuntimeHost = SpecApplyPatchRuntimeHost;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn runtime_host(&self) -> Self::RuntimeHost {
        SpecApplyPatchRuntimeHost
    }

    fn orchestrator_host(&self) -> Self::OrchestratorHost {
        SpecOrchestratorHost
    }

    fn sandbox_runtime(&self, _session: &Self::Session) -> SharedSandboxRuntime {
        spec_only()
    }

    fn tool_sandbox_context(&self, _turn: &Self::Turn) -> ToolSandboxContext {
        spec_only()
    }

    fn approval_policy(&self, _turn: &Self::Turn) -> AskForApproval {
        spec_only()
    }

    fn permission_profile(&self, _turn: &Self::Turn) -> PermissionProfile {
        spec_only()
    }

    fn file_system_sandbox_policy(&self, _turn: &Self::Turn) -> FileSystemSandboxPolicy {
        spec_only()
    }

    fn windows_sandbox_level(&self, _turn: &Self::Turn) -> WindowsSandboxLevel {
        spec_only()
    }

    fn file_system_sandbox_context(
        &self,
        _turn: &Self::Turn,
        _additional_permissions: Option<AdditionalPermissionProfile>,
        _cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        spec_only()
    }

    fn resolve_environment(
        &self,
        _turn: &Self::Turn,
        _environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        Ok(None)
    }

    async fn permission_grants(&self, _session: &Self::Session) -> ToolPermissionGrants {
        ToolPermissionGrants::default()
    }

    fn event_host<'a>(
        &'a self,
        _session: &'a Self::Session,
        _turn: &'a Self::Turn,
        _tracker: Option<&'a Self::Tracker>,
    ) -> Self::EventHost<'a> {
        SpecEventHost
    }
}

impl ShellRuntimeHost for SpecShellRuntimeHost {
    type ExecRequest = ();
    type NetworkApprovalTrigger = ();
    type Session = SpecSession;
    type StdoutStream = ();
    type Turn = SpecTurn;

    fn user_shell(&self, _session: &Self::Session) -> RuntimeShell {
        spec_only()
    }

    fn stdout_stream(
        &self,
        _ctx: &codex_tool_runtime_api::ToolCtx<Self::Session, Self::Turn>,
    ) -> Option<Self::StdoutStream> {
        spec_only()
    }

    fn network_approval_trigger(
        &self,
        _req: &codex_tool_runtime_api::ShellRequest,
        _ctx: &codex_tool_runtime_api::ToolCtx<Self::Session, Self::Turn>,
    ) -> Self::NetworkApprovalTrigger {
        spec_only()
    }

    fn start_shell_approval_async<'a>(
        &'a self,
        _req: &'a codex_tool_runtime_api::ShellRequest,
        _ctx: ApprovalCtx<'a, Self::Session, Self::Turn>,
        _keys: Vec<codex_tool_runtime_api::ShellApprovalKey>,
    ) -> BoxFuture<'a, ReviewDecision> {
        Box::pin(async { spec_only() })
    }

    fn transform_sandbox_attempt(
        &self,
        _attempt: &SandboxAttempt<'_>,
        _command: SandboxCommand,
        _options: ExecOptions,
        _network: Option<codex_network_proxy_api::SharedNetworkProxyRuntime>,
    ) -> Result<Self::ExecRequest, SandboxTransformError> {
        spec_only()
    }

    fn execute_env<'a>(
        &'a self,
        _exec_request: Self::ExecRequest,
        _stdout_stream: Option<Self::StdoutStream>,
    ) -> BoxFuture<'a, ProtocolResult<ExecToolCallOutput>> {
        Box::pin(async { spec_only() })
    }
}

impl ShellExecutionHost for SpecOnlyToolDomainHost {
    type ShellHost = SpecShellRuntimeHost;
    type ShellOrchestratorHost = SpecOrchestratorHost;

    fn shell_runtime_host(&self) -> Self::ShellHost {
        SpecShellRuntimeHost
    }

    fn shell_orchestrator_host(&self) -> Self::ShellOrchestratorHost {
        SpecOrchestratorHost
    }

    fn primary_environment(
        &self,
        _turn: &Self::Turn,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, FunctionCallError> {
        Ok(None)
    }

    async fn dependency_env(&self, _session: &Self::Session) -> HashMap<String, String> {
        HashMap::new()
    }

    fn explicit_env_overrides(&self, _turn: &Self::Turn) -> HashMap<String, String> {
        HashMap::new()
    }

    fn exec_permission_approvals_enabled(&self, _session: &Self::Session) -> bool {
        false
    }

    fn request_permissions_tool_enabled(&self, _session: &Self::Session) -> bool {
        false
    }

    async fn create_exec_approval_requirement(
        &self,
        _session: &Self::Session,
        _request: ExecPolicyApprovalRequest<'_>,
    ) -> ExecApprovalRequirement {
        ExecApprovalRequirement::Skip {
            bypass_sandbox: false,
            proposed_execpolicy_amendment: None,
        }
    }

    fn truncation_policy(&self, _turn: &Self::Turn) -> TruncationPolicy {
        TruncationPolicy::Bytes(10_000)
    }
}

impl ShellCommandHandlerHost for SpecOnlyToolDomainHost {
    fn resolve_workdir_base_path(
        &self,
        _turn: &Self::Turn,
        _arguments: &str,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        Ok(spec_cwd())
    }

    fn parse_shell_command_params(
        &self,
        _arguments: &str,
        _base_path: &AbsolutePathBuf,
    ) -> Result<ShellCommandToolCallParams, FunctionCallError> {
        spec_only()
    }

    fn resolve_shell_workdir(
        &self,
        _turn: &Self::Turn,
        _workdir: Option<String>,
    ) -> AbsolutePathBuf {
        spec_cwd()
    }

    async fn maybe_emit_implicit_skill_invocation(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _command: &str,
        _workdir: &AbsolutePathBuf,
    ) {
    }

    fn shell_command_exec_params(
        &self,
        _params: &ShellCommandToolCallParams,
        _session: &Self::Session,
        _turn: &Self::Turn,
    ) -> Result<ExecParams, FunctionCallError> {
        spec_only()
    }

    fn shell_type(&self, _session: &Self::Session) -> Option<ToolUserShellType> {
        None
    }
}

impl CommandInteractionHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn new_response_item_id(&self) -> String {
        "spec-response-item".to_string()
    }

    async fn begin_command_wait(
        &self,
        _session: &Self::Session,
        _request: CommandWaitRequest,
    ) -> Result<Box<dyn CommandWaitOperation>, CommandSessionError> {
        spec_only()
    }

    async fn write_command_stdin(
        &self,
        _session: &Self::Session,
        _request: WriteStdinRequest<'_>,
    ) -> Result<WriteStdinOutput, CommandSessionError> {
        spec_only()
    }

    async fn emit_model_item_started_display_event(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _item: &ResponseItem,
    ) {
    }

    async fn record_model_items_and_emit_display_events(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _items: &[ResponseItem],
    ) {
    }

    async fn send_terminal_interaction(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _event: TerminalInteractionEvent,
    ) {
    }
}

impl McpToolCallHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    async fn call_mcp_tool(
        &self,
        _session: Self::Session,
        _turn: &Self::Turn,
        _call_id: String,
        _server: String,
        _tool_name: String,
        _hook_tool_name: String,
        _arguments: String,
    ) -> McpToolCallOutcome {
        spec_only()
    }

    fn mcp_original_image_detail_supported(&self, _turn: &Self::Turn) -> bool {
        false
    }

    fn mcp_truncation_policy(&self, _turn: &Self::Turn) -> TruncationPolicy {
        TruncationPolicy::Bytes(10_000)
    }
}

impl McpResourceHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    async fn list_resources(
        &self,
        _session: &Self::Session,
        _server: &str,
        _params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourcesResult, String> {
        spec_only()
    }

    async fn list_all_resources(&self, _session: &Self::Session) -> HashMap<String, Vec<Resource>> {
        spec_only()
    }

    async fn list_resource_templates(
        &self,
        _session: &Self::Session,
        _server: &str,
        _params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourceTemplatesResult, String> {
        spec_only()
    }

    async fn list_all_resource_templates(
        &self,
        _session: &Self::Session,
    ) -> HashMap<String, Vec<ResourceTemplate>> {
        spec_only()
    }

    async fn read_resource(
        &self,
        _session: &Self::Session,
        _server: &str,
        _params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult, String> {
        spec_only()
    }

    async fn emit_mcp_tool_call_begin(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: &str,
        _invocation: McpInvocation,
    ) {
    }

    async fn emit_mcp_tool_call_end(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: &str,
        _invocation: McpInvocation,
        _duration: Duration,
        _result: Result<CallToolResult, String>,
    ) {
    }
}

impl FunctionToolHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn turn_collaboration_mode(&self, _turn: &Self::Turn) -> ModeKind {
        ModeKind::Default
    }

    fn turn_cwd(&self, _turn: &Self::Turn) -> AbsolutePathBuf {
        spec_cwd()
    }

    fn turn_id(&self, _turn: &Self::Turn) -> String {
        "spec-turn".to_string()
    }

    fn turn_is_non_root_agent(&self, _turn: &Self::Turn) -> bool {
        false
    }

    fn turn_supports_image_input(&self, _turn: &Self::Turn) -> bool {
        true
    }

    fn turn_can_request_original_image_detail(&self, _turn: &Self::Turn) -> bool {
        false
    }

    async fn session_collaboration_mode(&self, _session: &Self::Session) -> ModeKind {
        ModeKind::Default
    }

    async fn emit_plan_update(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _args: UpdatePlanArgs,
    ) {
    }

    async fn emit_image_view(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: String,
        _path: AbsolutePathBuf,
    ) {
    }

    async fn request_permissions(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: String,
        _args: RequestPermissionsArgs,
        _cancellation_token: CancellationToken,
    ) -> Option<RequestPermissionsResponse> {
        spec_only()
    }

    async fn request_user_input(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: String,
        _args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        spec_only()
    }

    async fn request_dynamic_tool(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: String,
        _tool_name: ToolName,
        _arguments: serde_json::Value,
    ) -> Option<DynamicToolResponse> {
        spec_only()
    }
}

impl GoalToolHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    async fn get_thread_goal(
        &self,
        _session: &Self::Session,
    ) -> Result<Option<ThreadGoal>, String> {
        spec_only()
    }

    async fn create_thread_goal(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _objective: String,
        _token_budget: Option<i64>,
    ) -> Result<ThreadGoal, String> {
        spec_only()
    }

    async fn complete_thread_goal(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
    ) -> Result<ThreadGoal, String> {
        spec_only()
    }
}

impl AgentJobToolHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type SpawnConfig = ();
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn state_db(&self, _session: &Self::Session) -> Option<SharedStateDbRuntime> {
        None
    }

    fn conversation_id_string(&self, _session: &Self::Session) -> String {
        "spec-conversation".to_string()
    }

    fn single_local_environment_cwd(
        &self,
        _turn: &Self::Turn,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        Ok(spec_cwd())
    }

    fn default_agent_job_max_runtime_seconds(&self, _turn: &Self::Turn) -> Option<u64> {
        None
    }

    async fn build_agent_job_runner_options(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _requested_concurrency: Option<usize>,
    ) -> Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError> {
        Ok(AgentJobRunnerOptions {
            max_concurrency: 1,
            spawn_config: (),
        })
    }

    async fn spawn_agent_job_worker(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _spawn_config: Self::SpawnConfig,
        _job_id: &str,
        _prompt: String,
    ) -> Result<ThreadId, AgentJobSpawnWorkerError> {
        spec_only()
    }

    async fn shutdown_agent_job_worker(&self, _session: &Self::Session, _thread_id: ThreadId) {}

    async fn get_agent_job_worker_status(
        &self,
        _session: &Self::Session,
        _thread_id: ThreadId,
    ) -> AgentStatus {
        spec_only()
    }

    async fn subscribe_agent_job_worker_status(
        &self,
        _session: &Self::Session,
        _thread_id: ThreadId,
    ) -> Option<watch::Receiver<AgentStatus>> {
        spec_only()
    }
}

impl WorkflowToolHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn load_workflow_registry(&self, _turn: &Self::Turn) -> WorkflowRegistry {
        spec_only()
    }

    fn workflow_run_controller(&self, _session: &Self::Session) -> Arc<dyn WorkflowRunController> {
        spec_only()
    }

    fn create_workflow_runtime_bridge(
        &self,
        _session: Self::Session,
        _turn: Self::Turn,
        _cancellation_token: CancellationToken,
        _tracker: Self::Tracker,
    ) -> Arc<dyn WorkflowRuntimeBridge> {
        spec_only()
    }

    async fn record_workflow_progress(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _run: &WorkflowRun,
        _kind: WorkflowRunProgressKind,
    ) {
    }
}

impl ExecCommandHandlerHost for SpecOnlyToolDomainHost {
    fn resolve_exec_command_environment(
        &self,
        _turn: &Self::Turn,
        _environment_id: Option<&str>,
        _workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, FunctionCallError> {
        Ok(None)
    }

    fn resolve_model_shell(&self, _shell: &Path) -> RuntimeShell {
        spec_only()
    }

    fn resolve_exec_command(
        &self,
        _command: &str,
        _login: Option<bool>,
        _model_shell: Option<&RuntimeShell>,
        _session: &Self::Session,
        _turn: &Self::Turn,
    ) -> Result<ResolvedExecCommand, String> {
        spec_only()
    }

    async fn maybe_emit_implicit_skill_invocation(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _command: &str,
        _workdir: &AbsolutePathBuf,
    ) {
    }

    async fn allocate_exec_process_id(&self, _session: &Self::Session) -> i32 {
        spec_only()
    }

    async fn release_exec_process_id(&self, _session: &Self::Session, _process_id: i32) {}

    async fn run_exec_command(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: &str,
        _request: ExecCommandRunRequest,
    ) -> Result<ExecCommandRunOutput, UnifiedExecError> {
        spec_only()
    }

    fn emit_unified_exec_tty_metric(&self, _turn: &Self::Turn, _tty: bool) {}
}

impl MultiAgentToolHost for SpecOnlyToolDomainHost {
    type DiffContext = SpecDiffContext;
    type Session = SpecSession;
    type Tracker = SpecTracker;
    type Turn = SpecTurn;

    fn thread_id(&self, _session: &Self::Session) -> ThreadId {
        spec_only()
    }

    fn sender_agent_path(&self, _session: &Self::Session, _turn: &Self::Turn) -> AgentPath {
        spec_only()
    }

    async fn send_collab_event(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _event: EventMsg,
    ) {
    }

    async fn resolve_agent_target(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _target: &str,
    ) -> Result<ThreadId, FunctionCallError> {
        spec_only()
    }

    fn agent_metadata(&self, _session: &Self::Session, _thread_id: ThreadId) -> AgentMetadata {
        spec_only()
    }

    async fn agent_status(&self, _session: &Self::Session, _thread_id: ThreadId) -> AgentStatus {
        spec_only()
    }

    async fn subscribe_agent_status(
        &self,
        _session: &Self::Session,
        _thread_id: ThreadId,
    ) -> Result<watch::Receiver<AgentStatus>, FunctionCallError> {
        spec_only()
    }

    fn subscribe_mailbox_seq(&self, _session: &Self::Session) -> watch::Receiver<u64> {
        spec_only()
    }

    async fn find_pending_inter_agent_communication(
        &self,
        _session: &Self::Session,
        _receiver_thread_id: ThreadId,
        _receiver_agent_path: &AgentPath,
    ) -> Option<InterAgentCommunication> {
        spec_only()
    }

    async fn wait_agent_current_window(
        &self,
        _session: &Self::Session,
        _sender_thread_id: ThreadId,
        _receiver_thread_id: ThreadId,
        _initial_timeout_ms: i64,
        _hard_cap_timeout_ms: i64,
    ) -> Duration {
        spec_only()
    }

    async fn advance_wait_agent_backoff(
        &self,
        _session: &Self::Session,
        _sender_thread_id: ThreadId,
        _receiver_thread_id: ThreadId,
    ) {
    }

    async fn reset_wait_agent_backoff(
        &self,
        _session: &Self::Session,
        _sender_thread_id: ThreadId,
        _receiver_thread_id: ThreadId,
    ) {
    }

    fn wait_agent_timeouts(&self, _turn: &Self::Turn) -> (i64, i64) {
        (1, 1)
    }

    fn register_session_root(&self, _session: &Self::Session, _turn: &Self::Turn) {}

    async fn list_agents(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _path_prefix: Option<&str>,
    ) -> Result<Vec<ListedAgent>, FunctionCallError> {
        spec_only()
    }

    async fn send_followup_task(
        &self,
        _session: &Self::Session,
        _sender_agent_path: AgentPath,
        _receiver_thread_id: ThreadId,
        _receiver_agent_path: AgentPath,
        _prompt: String,
    ) -> Result<(), FunctionCallError> {
        spec_only()
    }

    async fn mark_direct_child_completion_pending(
        &self,
        _session: &Self::Session,
        _receiver_thread_id: ThreadId,
    ) {
    }

    async fn mark_direct_child_completion_received(
        &self,
        _session: &Self::Session,
        _receiver_thread_id: ThreadId,
    ) -> bool {
        spec_only()
    }

    async fn clear_direct_child_completion_pending(
        &self,
        _session: &Self::Session,
        _receiver_thread_id: ThreadId,
    ) -> bool {
        spec_only()
    }

    async fn maybe_notify_parent_of_final_status(&self, _session: &Self::Session) {}

    async fn close_agent(
        &self,
        _session: &Self::Session,
        _thread_id: ThreadId,
    ) -> Result<(), FunctionCallError> {
        spec_only()
    }

    async fn spawn_agent(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: &str,
        _request: SpawnAgentToolRequest,
    ) -> Result<SpawnAgentToolResult, FunctionCallError> {
        spec_only()
    }
}

impl RequestPluginInstallHost for SpecOnlyToolDomainHost {
    fn request_plugin_install_context(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
    ) -> RequestPluginInstallContext {
        spec_only()
    }

    async fn list_request_plugin_install_discoverable_tools(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
    ) -> Result<Vec<DiscoverableTool>, FunctionCallError> {
        spec_only()
    }

    async fn request_plugin_install_elicitation(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _call_id: &str,
        _request: RequestPluginInstallElicitationRequest,
        _tool: &DiscoverableTool,
    ) -> RequestPluginInstallElicitationOutcome {
        spec_only()
    }

    async fn complete_request_plugin_install_if_ready(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _tool: &DiscoverableTool,
    ) -> bool {
        spec_only()
    }
}

impl CodeModeToolHost for SpecOnlyToolDomainHost {
    fn code_mode_turn_id(&self, _turn: &Self::Turn) -> String {
        "spec-turn".to_string()
    }

    fn can_request_original_image_detail(&self, _turn: &Self::Turn) -> bool {
        false
    }

    async fn code_mode_stored_values(
        &self,
        _session: &Self::Session,
    ) -> HashMap<String, serde_json::Value> {
        HashMap::new()
    }

    async fn code_mode_replace_stored_values(
        &self,
        _session: &Self::Session,
        _values: HashMap<String, serde_json::Value>,
    ) {
    }

    fn code_mode_allocate_cell_id(&self, _session: &Self::Session) -> String {
        "spec-cell".to_string()
    }

    async fn code_mode_execute(
        &self,
        _session: &Self::Session,
        _request: ExecuteRequest,
    ) -> Result<RuntimeResponse, String> {
        spec_only()
    }

    async fn code_mode_wait(
        &self,
        _session: &Self::Session,
        _request: WaitRequest,
    ) -> Result<WaitOutcome, String> {
        spec_only()
    }

    fn record_code_mode_cell_started(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _runtime_cell_id: &str,
        _model_visible_call_id: &str,
        _source_js: &str,
    ) {
    }

    fn record_code_mode_cell_initial_response(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _runtime_cell_id: &str,
        _response: &RuntimeResponse,
    ) {
    }

    fn record_code_mode_cell_ended(
        &self,
        _session: &Self::Session,
        _turn: &Self::Turn,
        _runtime_cell_id: &str,
        _response: &RuntimeResponse,
    ) {
    }
}
fn extension_tool_executor(name: &str, description: &str) -> Arc<dyn ExtensionToolExecutor> {
    struct SpecOnlyExtensionExecutor {
        name: String,
        description: String,
    }

    impl ToolExecutor<ExtensionToolCall> for SpecOnlyExtensionExecutor {
        type Output = codex_tool_planning::JsonToolOutput;

        fn tool_name(&self) -> ToolName {
            ToolName::plain(self.name.as_str())
        }

        fn spec(&self) -> Option<ToolSpec> {
            Some(ToolSpec::Function(ResponsesApiTool {
                name: self.name.clone(),
                description: self.description.clone(),
                strict: true,
                parameters: JsonSchema::object(
                    BTreeMap::from([(
                        "message".to_string(),
                        JsonSchema::string(/*description*/ None),
                    )]),
                    Some(vec!["message".to_string()]),
                    Some(false.into()),
                ),
                output_schema: None,
                defer_loading: None,
            }))
        }

        fn handle<'a>(
            &'a self,
            _call: ExtensionToolCall,
        ) -> codex_extension_api::ToolExecutorFuture<'a, Self::Output>
        where
            Self: 'a,
        {
            Box::pin(async move { panic!("spec planning should not execute extension tools") })
        }
    }

    Arc::new(SpecOnlyExtensionExecutor {
        name: name.to_string(),
        description: description.to_string(),
    })
}

#[test]
fn extension_tools_do_not_replace_builtin_tools() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let extension_tool_executors = vec![extension_tool_executor(
        "update_plan",
        "Extension attempt to replace a built-in tool.",
    )];
    let (tools, _) = build_specs_with_inputs_for_test(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        /*discoverable_tools*/ None,
        &extension_tool_executors,
        &[],
    );

    assert_eq!(
        find_tool(&tools, "update_plan").clone(),
        create_update_plan_tool()
    );
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name() == "update_plan")
            .count(),
        1
    );
}

#[test]
fn agent_tool_allowlist_filters_specs_and_registry_handlers() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_agent_tool_patterns(Some(vec!["exec_command".to_string()]));

    let (specs, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        /*dynamic_tools*/ &[],
    );

    let tool_names = specs
        .iter()
        .filter_map(|spec| match spec {
            ToolSpec::Function(tool) => Some(tool.name.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(vec!["exec_command".to_string()], tool_names);
    assert!(registry.has_handler(&ToolName::plain("exec_command")));
    assert!(!registry.has_handler(&ToolName::plain("apply_patch")));
}

#[test]
fn agent_tool_empty_allowlist_filters_all_optional_specs_and_registry_handlers() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_agent_tool_patterns(Some(Vec::new()));

    let (specs, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        /*dynamic_tools*/ &[],
    );

    assert!(specs.is_empty());
    assert!(!registry.has_handler(&ToolName::plain("exec_command")));
    assert!(!registry.has_handler(&ToolName::plain("apply_patch")));
}

#[test]
fn test_full_toolset_specs_for_gpt5_codex_unified_exec_web_search() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let mut actual = BTreeMap::new();
    let mut duplicate_names = Vec::new();
    for tool in &tools {
        let name = tool.name().to_string();
        if actual.insert(name.clone(), tool.clone()).is_some() {
            duplicate_names.push(name);
        }
    }
    assert!(
        duplicate_names.is_empty(),
        "duplicate tool entries detected: {duplicate_names:?}"
    );

    let mut expected = BTreeMap::new();
    for spec in [
        create_exec_command_tool(CommandToolOptions {
            allow_login_shell: true,
            exec_permission_approvals_enabled: false,
        }),
        create_command_wait_tool(),
        create_write_stdin_tool(),
        create_update_plan_tool(),
        request_user_input_tool_spec(&request_user_input_available_modes(&features)),
        create_workflow_list_tool(),
        create_workflow_describe_tool(),
        create_workflow_start_tool(),
        create_workflow_status_tool(),
        create_workflow_resume_tool(),
        create_workflow_abort_tool(),
        create_apply_patch_freeform_tool(/*include_environment_id*/ false),
        ToolSpec::WebSearch {
            external_web_access: Some(true),
            filters: None,
            user_location: None,
            search_context_size: None,
            search_content_types: None,
        },
        create_image_generation_tool("png"),
        create_view_image_tool(ViewImageToolOptions {
            can_request_original_image_detail: config.can_request_original_image_detail,
            include_environment_id: false,
        }),
    ] {
        expected.insert(spec.name().to_string(), spec);
    }
    if config.goal_tools {
        for spec in [
            create_get_goal_tool(),
            create_create_goal_tool(),
            create_update_goal_tool(),
        ] {
            expected.insert(spec.name().to_string(), spec);
        }
    }
    if config.collab_tools {
        for spec in [
            create_spawn_agent_tool_v2(spawn_agent_tool_options(&config)),
            create_followup_task_tool(),
            create_wait_agent_tool_v2(),
            create_close_agent_tool_v2(),
            create_list_agents_tool(),
        ] {
            expected.insert(spec.name().to_string(), spec);
        }
    }

    if config.exec_permission_approvals_enabled {
        let spec = create_request_permissions_tool(request_permissions_tool_description());
        expected.insert(spec.name().to_string(), spec);
    }

    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "tool name set mismatch"
    );

    for name in expected.keys() {
        let mut actual_spec = actual.get(name).expect("present").clone();
        let mut expected_spec = expected.get(name).expect("present").clone();
        strip_descriptions_tool(&mut actual_spec);
        strip_descriptions_tool(&mut expected_spec);
        assert_eq!(actual_spec, expected_spec, "spec mismatch for {name}");
    }
}

#[test]
fn exec_command_spec_includes_environment_id_only_for_multiple_selected_environments() {
    let model_info = model_info();
    let available_models = Vec::new();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (single_environment_tools, _) = build_specs(
        &config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_process_tool_environment_id(
        &single_environment_tools,
        "exec_command",
        /*expected_present*/ false,
    );

    let multi_environment_config = config.with_environment_mode(ToolEnvironmentMode::Multiple);
    let (multi_environment_tools, _) = build_specs(
        &multi_environment_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_process_tool_environment_id(
        &multi_environment_tools,
        "exec_command",
        /*expected_present*/ true,
    );
}

#[test]
fn apply_patch_spec_includes_environment_id_only_for_multiple_selected_environments() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (single_environment_tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_apply_patch_environment_id(&single_environment_tools, /*expected_present*/ false);

    let multi_environment_config =
        tools_config.with_environment_mode(ToolEnvironmentMode::Multiple);
    let (multi_environment_tools, _) = build_specs(
        &multi_environment_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_apply_patch_environment_id(&multi_environment_tools, /*expected_present*/ true);
}

#[test]
fn test_build_specs_collab_tools_enabled() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Collab);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &[
            "spawn_agent",
            "followup_task",
            "wait_agent",
            "close_agent",
            "list_agents",
        ],
    );
    assert!(registry.has_handler(&ToolName::plain("wait_agent")));
    assert_lacks_tool_name(&tools, "spawn_agents_on_csv");
    assert_lacks_tool_name(&tools, "send_input");
    assert_lacks_tool_name(&tools, "resume_agent");

    let spawn_agent = find_tool(&tools, "spawn_agent");
    let ToolSpec::Function(ResponsesApiTool { parameters, .. }) = spawn_agent else {
        panic!("spawn_agent should be a function tool");
    };
    let (properties, _) = expect_object_schema(parameters);
    assert!(properties.contains_key("fork_turns"));
    assert!(!properties.contains_key("fork_context"));
}

#[test]
fn goal_tools_require_goals_feature() {
    let model_info = model_info();
    let available_models = Vec::new();
    let mut features = Features::with_defaults();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_lacks_tool_name(&tools, "get_goal");
    assert_lacks_tool_name(&tools, "create_goal");
    assert_lacks_tool_name(&tools, "update_goal");

    features.enable(Feature::Goals);
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_contains_tool_names(&tools, &["get_goal", "create_goal", "update_goal"]);
}

#[test]
fn test_build_specs_multi_agent_v2_uses_task_names_and_hides_resume() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Collab);
    features.enable(Feature::MultiAgentV2);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &[
            "spawn_agent",
            "followup_task",
            "wait_agent",
            "close_agent",
            "list_agents",
        ],
    );
    assert!(registry.has_handler(&ToolName::plain("wait_agent")));

    let spawn_agent = find_tool(&tools, "spawn_agent");
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = spawn_agent
    else {
        panic!("spawn_agent should be a function tool");
    };
    let (properties, required) = expect_object_schema(parameters);
    assert!(properties.contains_key("task_name"));
    assert!(properties.contains_key("message"));
    assert!(properties.contains_key("fork_turns"));
    assert!(!properties.contains_key("items"));
    assert!(!properties.contains_key("fork_context"));
    assert_eq!(
        required,
        Some(&vec!["task_name".to_string(), "message".to_string()])
    );
    let output_schema = output_schema
        .as_ref()
        .expect("spawn_agent should define output schema");
    assert_eq!(output_schema["required"], json!(["task_name", "nickname"]));

    let followup_task = find_tool(&tools, "followup_task");
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = followup_task
    else {
        panic!("followup_task should be a function tool");
    };
    assert_eq!(output_schema, &None);
    let (properties, required) = expect_object_schema(parameters);
    assert!(properties.contains_key("target"));
    assert!(properties.contains_key("message"));
    assert!(!properties.contains_key("items"));
    assert_eq!(
        required,
        Some(&vec!["target".to_string(), "message".to_string()])
    );

    let list_agents = find_tool(&tools, "list_agents");
    let ToolSpec::Function(ResponsesApiTool {
        parameters,
        output_schema,
        ..
    }) = list_agents
    else {
        panic!("list_agents should be a function tool");
    };
    let (properties, required) = expect_object_schema(parameters);
    assert!(properties.contains_key("path_prefix"));
    assert_eq!(required, None);
    let output_schema = output_schema
        .as_ref()
        .expect("list_agents should define output schema");
    assert_eq!(
        output_schema["properties"]["agents"]["items"]["required"],
        json!(["agent_name", "agent_status", "last_task_message"])
    );
    assert_lacks_tool_name(&tools, "send_input");
    assert_lacks_tool_name(&tools, "resume_agent");
}

#[test]
fn test_build_specs_multi_agent_v2_does_not_expose_collab_without_collab_feature() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.disable(Feature::Collab);
    features.enable(Feature::MultiAgentV2);
    assert!(!features.enabled(Feature::Collab));
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_lacks_tool_name(&tools, "spawn_agent");
    assert_lacks_tool_name(&tools, "followup_task");
    assert_lacks_tool_name(&tools, "close_agent");
    assert_lacks_tool_name(&tools, "list_agents");
    assert_lacks_tool_name(&tools, "wait_agent");
    assert_lacks_tool_name(&tools, "send_input");
    assert_lacks_tool_name(&tools, "resume_agent");
}

#[test]
fn test_build_specs_enable_fanout_enables_agent_jobs_and_collab_tools() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::SpawnCsv);
    features.normalize_dependencies();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &[
            "spawn_agent",
            "followup_task",
            "wait_agent",
            "close_agent",
            "list_agents",
            "spawn_agents_on_csv",
        ],
    );
}

#[test]
fn view_image_tool_omits_detail_without_original_detail_support() {
    let mut model_info = model_info();
    model_info.supports_image_detail_original = false;
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let view_image = find_tool(&tools, VIEW_IMAGE_TOOL_NAME);
    let ToolSpec::Function(ResponsesApiTool { parameters, .. }) = view_image else {
        panic!("view_image should be a function tool");
    };
    let (properties, _) = expect_object_schema(parameters);
    assert!(!properties.contains_key("detail"));
}

#[test]
fn view_image_tool_includes_detail_with_original_detail_support() {
    let mut model_info = model_info();
    model_info.supports_image_detail_original = true;
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let view_image = find_tool(&tools, VIEW_IMAGE_TOOL_NAME);
    let ToolSpec::Function(ResponsesApiTool { parameters, .. }) = view_image else {
        panic!("view_image should be a function tool");
    };
    let (properties, _) = expect_object_schema(parameters);
    assert!(properties.contains_key("detail"));
    let description = expect_string_description(
        properties
            .get("detail")
            .expect("view_image detail should include a description"),
    );
    assert!(description.contains("only supported value is `original`"));
    assert!(description.contains("omit this field for default resized behavior"));
}

#[test]
fn disabled_environment_omits_environment_backed_tools() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_environment_mode(ToolEnvironmentMode::None);
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_lacks_tool_name(&tools, "exec_command");
    assert_lacks_tool_name(&tools, "write_stdin");
    assert_lacks_tool_name(&tools, "apply_patch");
    assert_lacks_tool_name(&tools, VIEW_IMAGE_TOOL_NAME);
}

#[test]
fn view_image_spec_includes_environment_id_only_for_multiple_selected_environments() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (single_environment_tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_process_tool_environment_id(
        &single_environment_tools,
        VIEW_IMAGE_TOOL_NAME,
        /*expected_present*/ false,
    );

    let multi_environment_config =
        tools_config.with_environment_mode(ToolEnvironmentMode::Multiple);
    let (multi_environment_tools, _) = build_specs(
        &multi_environment_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_process_tool_environment_id(
        &multi_environment_tools,
        VIEW_IMAGE_TOOL_NAME,
        /*expected_present*/ true,
    );
}

#[test]
fn test_build_specs_agent_job_worker_tools_enabled() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::SpawnCsv);
    features.normalize_dependencies();
    features.enable(Feature::Sqlite);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::SubAgent(SubAgentSource::Other(
            "agent_job:test".to_string(),
        )),
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &[
            "spawn_agent",
            "followup_task",
            "wait_agent",
            "close_agent",
            "list_agents",
            "spawn_agents_on_csv",
            "report_agent_job_result",
            REQUEST_USER_INPUT_TOOL_NAME,
        ],
    );
}

#[test]
fn request_user_input_description_reflects_default_mode_feature_flag() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let request_user_input_tool = find_tool(&tools, REQUEST_USER_INPUT_TOOL_NAME);
    assert_eq!(
        request_user_input_tool.clone(),
        request_user_input_tool_spec(&request_user_input_available_modes(&features))
    );

    features.enable(Feature::DefaultModeRequestUserInput);
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let request_user_input_tool = find_tool(&tools, REQUEST_USER_INPUT_TOOL_NAME);
    assert_eq!(
        request_user_input_tool.clone(),
        request_user_input_tool_spec(&request_user_input_available_modes(&features))
    );
}

#[test]
fn request_permissions_requires_feature_flag() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_lacks_tool_name(&tools, "request_permissions");

    let mut features = Features::with_defaults();
    features.enable(Feature::RequestPermissionsTool);
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let request_permissions_tool = find_tool(&tools, "request_permissions");
    assert_eq!(
        request_permissions_tool.clone(),
        create_request_permissions_tool(request_permissions_tool_description())
    );
}

#[test]
fn request_permissions_tool_is_independent_from_additional_permissions() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::ExecPermissionApprovals);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_lacks_tool_name(&tools, "request_permissions");
}

#[test]
fn image_generation_tools_require_feature_and_supported_model() {
    let supported_model_info = model_info();
    let mut unsupported_model_info = supported_model_info.clone();
    unsupported_model_info.input_modalities = vec![InputModality::Text];
    let mut image_generation_disabled_features = Features::with_defaults();
    image_generation_disabled_features.disable(Feature::ImageGeneration);
    let mut image_generation_features = Features::with_defaults();
    image_generation_features.enable(Feature::ImageGeneration);

    let available_models = Vec::new();
    let default_tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &supported_model_info,
        available_models: &available_models,
        features: &image_generation_disabled_features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (default_tools, _) = build_specs(
        &default_tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert!(
        !default_tools
            .iter()
            .any(|tool| tool.name() == "image_generation"),
        "image_generation should be disabled when the feature is disabled"
    );

    let supported_tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &supported_model_info,
        available_models: &available_models,
        features: &image_generation_features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (supported_tools, _) = build_specs(
        &supported_tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert_contains_tool_names(&supported_tools, &["image_generation"]);
    let image_generation_tool = find_tool(&supported_tools, "image_generation");
    assert_eq!(
        serde_json::to_value(image_generation_tool).expect("serialize image tool"),
        serde_json::json!({
            "type": "image_generation",
            "output_format": "png"
        })
    );

    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &unsupported_model_info,
        available_models: &available_models,
        features: &image_generation_features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    assert!(
        !tools.iter().any(|tool| tool.name() == "image_generation"),
        "image_generation should be disabled for unsupported models"
    );
}

#[test]
fn web_search_mode_cached_sets_external_web_access_false() {
    let model_info = model_info();
    let features = Features::with_defaults();

    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_tool(&tools, "web_search");
    assert_eq!(
        tool.clone(),
        ToolSpec::WebSearch {
            external_web_access: Some(false),
            filters: None,
            user_location: None,
            search_context_size: None,
            search_content_types: None,
        }
    );
}

#[test]
fn web_search_mode_live_sets_external_web_access_true() {
    let model_info = model_info();
    let features = Features::with_defaults();

    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_tool(&tools, "web_search");
    assert_eq!(
        tool.clone(),
        ToolSpec::WebSearch {
            external_web_access: Some(true),
            filters: None,
            user_location: None,
            search_context_size: None,
            search_content_types: None,
        }
    );
}

#[test]
fn web_search_config_is_forwarded_to_tool_spec() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let web_search_config = WebSearchConfig {
        filters: Some(codex_protocol::config_types::WebSearchFilters {
            allowed_domains: Some(vec!["example.com".to_string()]),
        }),
        user_location: Some(codex_protocol::config_types::WebSearchUserLocation {
            r#type: codex_protocol::config_types::WebSearchUserLocationType::Approximate,
            country: Some("US".to_string()),
            region: Some("California".to_string()),
            city: Some("San Francisco".to_string()),
            timezone: Some("America/Los_Angeles".to_string()),
        }),
        search_context_size: Some(codex_protocol::config_types::WebSearchContextSize::High),
    };

    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_web_search_config(Some(web_search_config.clone()));
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_tool(&tools, "web_search");
    assert_eq!(
        tool.clone(),
        ToolSpec::WebSearch {
            external_web_access: Some(true),
            filters: web_search_config
                .filters
                .map(ResponsesApiWebSearchFilters::from),
            user_location: web_search_config
                .user_location
                .map(ResponsesApiWebSearchUserLocation::from),
            search_context_size: web_search_config.search_context_size,
            search_content_types: None,
        }
    );
}

#[test]
fn web_search_tool_type_text_and_image_sets_search_content_types() {
    let mut model_info = model_info();
    model_info.web_search_tool_type = WebSearchToolType::TextAndImage;
    let features = Features::with_defaults();

    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_tool(&tools, "web_search");
    assert_eq!(
        tool.clone(),
        ToolSpec::WebSearch {
            external_web_access: Some(true),
            filters: None,
            user_location: None,
            search_context_size: None,
            search_content_types: Some(vec!["text".to_string(), "image".to_string()]),
        }
    );
}

#[test]
fn mcp_resource_tools_are_hidden_without_mcp_servers() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert!(
        !tools.iter().any(|tool| matches!(
            tool.name(),
            "list_mcp_resources" | "list_mcp_resource_templates" | "read_mcp_resource"
        )),
        "MCP resource tools should be omitted when no MCP servers are configured"
    );
}

#[test]
fn mcp_resource_tools_are_included_when_mcp_servers_are_present() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::new()),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &[
            "list_mcp_resources",
            "list_mcp_resource_templates",
            "read_mcp_resource",
        ],
    );
}

#[test]
#[ignore]
fn test_parallel_support_flags() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_contains_tool_names(
        &tools,
        &["exec_command", "command_wait", "command_write_stdin"],
    );
}

#[test]
fn test_test_model_info_includes_sync_tool() {
    let mut model_info = model_info();
    model_info.experimental_supported_tools = vec!["test_sync_tool".to_string()];
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert!(tools.iter().any(|tool| tool.name() == "test_sync_tool"));
}

#[test]
fn test_build_specs_mcp_tools_converted() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("test_server/", "do_something_cool"),
            mcp_tool(
                "do_something_cool",
                "Do something cool",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "string_argument": { "type": "string" },
                        "number_argument": { "type": "number" },
                        "object_argument": {
                            "type": "object",
                            "properties": {
                                "string_property": { "type": "string" },
                                "number_property": { "type": "number" },
                            },
                            "required": ["string_property", "number_property"],
                            "additionalProperties": false,
                        },
                    },
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "test_server/", "do_something_cool");
    assert_eq!(
        tool,
        &ResponsesApiTool {
            name: "do_something_cool".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([
                    (
                        "string_argument".to_string(),
                        JsonSchema::string(/*description*/ None),
                    ),
                    (
                        "number_argument".to_string(),
                        JsonSchema::number(/*description*/ None),
                    ),
                    (
                        "object_argument".to_string(),
                        JsonSchema::object(
                            BTreeMap::from([
                                (
                                    "string_property".to_string(),
                                    JsonSchema::string(/*description*/ None),
                                ),
                                (
                                    "number_property".to_string(),
                                    JsonSchema::number(/*description*/ None),
                                ),
                            ]),
                            Some(vec![
                                "string_property".to_string(),
                                "number_property".to_string(),
                            ]),
                            Some(false.into()),
                        ),
                    ),
                ]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "Do something cool".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

#[test]
fn agent_tool_allowlist_filters_namespace_children_and_registry_handlers() {
    let model_info = model_info();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &Features::with_defaults(),
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Live),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    })
    .with_agent_tool_patterns(Some(vec!["test_server/do_something_cool".to_string()]));
    let (tools, registry) = build_specs(
        &tools_config,
        Some(HashMap::from([
            (
                ToolName::namespaced("test_server/", "do_something_cool"),
                mcp_tool("do_something_cool", "Do something cool", json!({})),
            ),
            (
                ToolName::namespaced("test_server/", "delete_everything"),
                mcp_tool("delete_everything", "Delete everything", json!({})),
            ),
        ])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_eq!(
        namespace_function_names(&tools, "test_server/"),
        vec!["do_something_cool".to_string()]
    );
    assert!(registry.has_handler(&ToolName::namespaced("test_server/", "do_something_cool")));
    assert!(!registry.has_handler(&ToolName::namespaced("test_server/", "delete_everything")));
}

#[test]
fn namespace_specs_are_hidden_when_namespace_tools_are_disabled() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    tools_config.namespace_tools = false;

    let (tools, registry) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("mcp__sample__", "echo"),
            mcp_tool("echo", "Echo", serde_json::json!({"type": "object"})),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_lacks_tool_name(&tools, "mcp__sample__");
    assert!(registry.has_handler(&ToolName::namespaced("mcp__sample__", "echo")));
}

#[test]
fn namespaced_dynamic_specs_are_hidden_when_namespace_tools_are_disabled() {
    let model_info = model_info();
    let features = Features::with_defaults();
    let available_models = Vec::new();
    let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    tools_config.namespace_tools = false;
    let dynamic_tools = vec![
        DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_update".to_string(),
            description: "Create or update automations.".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
            defer_loading: false,
        },
        DynamicToolSpec {
            namespace: None,
            name: "plain_dynamic".to_string(),
            description: "Plain dynamic tool.".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
            defer_loading: false,
        },
    ];

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &dynamic_tools,
    );

    assert_lacks_tool_name(&tools, "codex_app");
    assert_contains_tool_names(&tools, &["plain_dynamic"]);
}

#[test]
fn test_build_specs_mcp_namespace_description_falls_back_when_missing() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("test_server/", "do_something_cool"),
            mcp_tool(
                "do_something_cool",
                "Do something cool",
                serde_json::json!({"type": "object"}),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let namespace_tool = find_tool(&tools, "test_server/");
    let ToolSpec::Namespace(namespace) = namespace_tool else {
        panic!("expected namespace tool");
    };
    assert_eq!(
        namespace.description,
        "Tools in the test_server/ namespace."
    );
}

#[test]
fn test_build_specs_mcp_tools_sorted_by_name() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let tools_map = HashMap::from([
        (
            ToolName::namespaced("test_server/", "do"),
            mcp_tool("do", "a", serde_json::json!({"type": "object"})),
        ),
        (
            ToolName::namespaced("test_server/", "something"),
            mcp_tool("something", "b", serde_json::json!({"type": "object"})),
        ),
        (
            ToolName::namespaced("test_server/", "cool"),
            mcp_tool("cool", "c", serde_json::json!({"type": "object"})),
        ),
    ]);

    let (tools, _) = build_specs(
        &tools_config,
        Some(tools_map),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert_eq!(
        namespace_function_names(&tools, "test_server/"),
        vec![
            "cool".to_string(),
            "do".to_string(),
            "something".to_string(),
        ]
    );
}

#[test]
fn search_tool_description_lists_each_mcp_source_once() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        Some(vec![
            deferred_mcp_tool(
                "_create_event",
                "mcp__codex_apps__calendar",
                CODEX_APPS_MCP_SERVER_NAME,
                Some("Calendar"),
                Some("Plan events and manage your calendar."),
            ),
            deferred_mcp_tool(
                "_list_events",
                "mcp__codex_apps__calendar",
                CODEX_APPS_MCP_SERVER_NAME,
                Some("Calendar"),
                Some("Plan events and manage your calendar."),
            ),
            deferred_mcp_tool(
                "_search_threads",
                "mcp__codex_apps__gmail",
                CODEX_APPS_MCP_SERVER_NAME,
                Some("Gmail"),
                Some("Find and summarize email threads."),
            ),
            deferred_mcp_tool(
                "echo",
                "mcp__rmcp__",
                "rmcp",
                /*connector_name*/ None,
                Some("Remote memory tools."),
            ),
        ]),
        &[],
    );

    let search_tool = find_tool(&tools, TOOL_SEARCH_TOOL_NAME);
    let ToolSpec::ToolSearch { description, .. } = search_tool else {
        panic!("expected tool_search tool");
    };
    let description = description.as_str();
    assert!(description.contains("- Calendar: Plan events and manage your calendar."));
    assert!(description.contains("- Gmail: Find and summarize email threads."));
    assert_eq!(
        description
            .matches("- Calendar: Plan events and manage your calendar.")
            .count(),
        1
    );
    assert!(description.contains("- rmcp: Remote memory tools."));
    assert!(!description.contains("mcp__rmcp__echo"));

    assert!(registry.has_handler(&ToolName::namespaced(
        "mcp__codex_apps__calendar",
        "_create_event",
    )));
    assert!(registry.has_handler(&ToolName::namespaced("mcp__rmcp__", "echo")));
}

#[test]
fn search_tool_requires_model_capability_and_enabled_feature() {
    let model_info = search_capable_model_info();
    let deferred_mcp_tools = Some(vec![deferred_mcp_tool(
        "_create_event",
        "mcp__codex_apps__calendar",
        CODEX_APPS_MCP_SERVER_NAME,
        Some("Calendar"),
        /*description*/ None,
    )]);

    let features = Features::with_defaults();
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &ModelInfo {
            supports_search_tool: false,
            ..model_info.clone()
        },
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        deferred_mcp_tools.clone(),
        &[],
    );
    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);

    let mut features_without_tool_search = Features::with_defaults();
    features_without_tool_search.disable(Feature::ToolSearch);
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features_without_tool_search,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        deferred_mcp_tools.clone(),
        &[],
    );
    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);

    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        deferred_mcp_tools,
        &[],
    );
    assert_contains_tool_names(&tools, &[TOOL_SEARCH_TOOL_NAME]);
}

#[test]
fn search_tool_is_hidden_without_deferred_tools() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        Some(Vec::new()),
        &[],
    );

    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);
}

#[test]
fn search_tool_description_falls_back_to_connector_name_without_description() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        Some(vec![deferred_mcp_tool(
            "_create_event",
            "mcp__codex_apps__calendar",
            CODEX_APPS_MCP_SERVER_NAME,
            Some("Calendar"),
            /*description*/ None,
        )]),
        &[],
    );
    let search_tool = find_tool(&tools, TOOL_SEARCH_TOOL_NAME);
    let ToolSpec::ToolSearch { description, .. } = search_tool else {
        panic!("expected tool_search tool");
    };

    assert!(description.contains("- Calendar"));
    assert!(!description.contains("- Calendar:"));
}

#[test]
fn search_tool_registers_namespaced_mcp_tool_aliases() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (_, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        Some(vec![
            deferred_mcp_tool(
                "_create_event",
                "mcp__codex_apps__calendar",
                CODEX_APPS_MCP_SERVER_NAME,
                Some("Calendar"),
                /*description*/ None,
            ),
            deferred_mcp_tool(
                "_list_events",
                "mcp__codex_apps__calendar",
                CODEX_APPS_MCP_SERVER_NAME,
                Some("Calendar"),
                /*description*/ None,
            ),
            deferred_mcp_tool(
                "echo",
                "mcp__rmcp__",
                "rmcp",
                /*connector_name*/ None,
                /*description*/ None,
            ),
        ]),
        &[],
    );

    let app_alias = ToolName::namespaced("mcp__codex_apps__calendar", "_create_event");
    let mcp_alias = ToolName::namespaced("mcp__rmcp__", "echo");

    assert!(registry.has_handler(&ToolName::plain(TOOL_SEARCH_TOOL_NAME)));
    assert!(registry.has_handler(&app_alias));
    assert!(registry.has_handler(&mcp_alias));
}

#[test]
fn no_search_tool_when_namespaces_disabled() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    tools_config.namespace_tools = false;

    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        Some(vec![deferred_mcp_tool(
            "_create_event",
            "mcp__codex_apps__calendar",
            CODEX_APPS_MCP_SERVER_NAME,
            Some("Calendar"),
            Some("Plan events and manage your calendar."),
        )]),
        &[],
    );

    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);
    assert!(!registry.has_handler(&ToolName::plain(TOOL_SEARCH_TOOL_NAME)));
}

#[test]
fn search_tool_registers_for_deferred_dynamic_tools() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let dynamic_tools = vec![
        DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_update".to_string(),
            description: "Create, update, view, or delete recurring automations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string" },
                },
            }),
            defer_loading: true,
        },
        DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_list".to_string(),
            description: "List recurring automations.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
            }),
            defer_loading: true,
        },
    ];

    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &dynamic_tools,
    );

    let search_tool = find_tool(&tools, TOOL_SEARCH_TOOL_NAME);
    let ToolSpec::ToolSearch { description, .. } = search_tool else {
        panic!("expected tool_search tool");
    };
    assert!(description.contains("- Dynamic tools: Tools provided by the current Codex thread."));
    assert_contains_tool_names(&tools, &[TOOL_SEARCH_TOOL_NAME]);
    assert_lacks_tool_name(&tools, "codex_app");
    assert!(registry.has_handler(&ToolName::plain(TOOL_SEARCH_TOOL_NAME)));
    assert!(registry.has_handler(&ToolName::namespaced("codex_app", "automation_update")));
    assert!(registry.has_handler(&ToolName::namespaced("codex_app", "automation_list")));
}

#[test]
fn search_tool_is_hidden_for_deferred_dynamic_tools_when_namespace_tools_are_disabled() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::ToolSearch);
    let available_models = Vec::new();
    let mut tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    tools_config.namespace_tools = false;
    let dynamic_tools = vec![
        DynamicToolSpec {
            namespace: Some("codex_app".to_string()),
            name: "automation_update".to_string(),
            description: "Create or update automations.".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
            defer_loading: true,
        },
        DynamicToolSpec {
            namespace: None,
            name: "plain_dynamic".to_string(),
            description: "Plain dynamic tool.".to_string(),
            input_schema: json!({"type": "object", "properties": {}}),
            defer_loading: true,
        },
    ];

    let (tools, registry) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &dynamic_tools,
    );

    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);
    assert_lacks_tool_name(&tools, "codex_app");
    assert_lacks_tool_name(&tools, "plain_dynamic");
    assert!(!registry.has_handler(&ToolName::plain(TOOL_SEARCH_TOOL_NAME)));
    assert!(registry.has_handler(&ToolName::namespaced("codex_app", "automation_update")));
    assert!(registry.has_handler(&ToolName::plain("plain_dynamic")));
}

#[test]
fn request_plugin_install_is_not_registered_without_feature_flag() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::ToolSearch);
    features.enable(Feature::Apps);
    features.enable(Feature::Plugins);
    features.disable(Feature::ToolSuggest);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs_with_inputs_for_test(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        Some(vec![discoverable_connector(
            "connector_2128aebfecb84f64a069897515042a44",
            "Google Calendar",
            "Plan events and schedules.",
        )]),
        /*extension_tool_executors*/ &[],
        &[],
    );

    assert!(
        !tools
            .iter()
            .any(|tool| tool.name() == REQUEST_PLUGIN_INSTALL_TOOL_NAME)
    );
}

#[test]
fn request_plugin_install_requires_apps_and_plugins_features() {
    let model_info = search_capable_model_info();
    let discoverable_tools = Some(vec![discoverable_connector(
        "connector_2128aebfecb84f64a069897515042a44",
        "Google Calendar",
        "Plan events and schedules.",
    )]);
    let available_models = Vec::new();

    for disabled_feature in [Feature::Apps, Feature::Plugins] {
        let mut features = Features::with_defaults();
        features.enable(Feature::ToolSearch);
        features.enable(Feature::ToolSuggest);
        features.enable(Feature::Apps);
        features.enable(Feature::Plugins);
        features.disable(disabled_feature);

        let tools_config = ToolsConfig::new(&ToolsConfigParams {
            model_info: &model_info,
            available_models: &available_models,
            features: &features,
            image_generation_tool_auth_allowed: true,
            web_search_mode: Some(WebSearchMode::Cached),
            session_source: SessionSource::Cli,
            permission_profile: &PermissionProfile::Disabled,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
        });
        let (tools, _) = build_specs_with_inputs_for_test(
            &tools_config,
            /*mcp_tools*/ None,
            /*deferred_mcp_tools*/ None,
            discoverable_tools.clone(),
            /*extension_tool_executors*/ &[],
            &[],
        );

        assert!(
            !tools
                .iter()
                .any(|tool| tool.name() == REQUEST_PLUGIN_INSTALL_TOOL_NAME),
            "tool_suggest should be absent when {disabled_feature:?} is disabled"
        );
    }
}

#[test]
fn request_plugin_install_can_be_registered_without_search_tool() {
    let model_info = ModelInfo {
        supports_search_tool: false,
        ..search_capable_model_info()
    };
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::Plugins);
    features.enable(Feature::ToolSuggest);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });
    let (tools, _) = build_specs_with_inputs_for_test(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        Some(vec![discoverable_connector(
            "connector_2128aebfecb84f64a069897515042a44",
            "Google Calendar",
            "Plan events and schedules.",
        )]),
        /*extension_tool_executors*/ &[],
        &[],
    );

    assert_contains_tool_names(&tools, &[REQUEST_PLUGIN_INSTALL_TOOL_NAME]);
    let request_plugin_install = find_tool(&tools, REQUEST_PLUGIN_INSTALL_TOOL_NAME);
    assert_lacks_tool_name(&tools, TOOL_SEARCH_TOOL_NAME);

    let ToolSpec::Function(ResponsesApiTool { description, .. }) = request_plugin_install else {
        panic!("expected function tool");
    };
    assert!(description.contains(
        "Use this tool only to ask the user to install one known plugin or connector from the list below. The list contains known candidates that are not currently installed."
    ));
    assert!(description.contains(
        "`tool_search` is not available, or it has already been called and did not find or make the requested tool callable."
    ));
}

#[test]
fn request_plugin_install_description_lists_discoverable_tools() {
    let model_info = search_capable_model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::Apps);
    features.enable(Feature::Plugins);
    features.enable(Feature::ToolSearch);
    features.enable(Feature::ToolSuggest);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let discoverable_tools = vec![
        discoverable_connector(
            "connector_2128aebfecb84f64a069897515042a44",
            "Google Calendar",
            "Plan events and schedules.",
        ),
        discoverable_connector(
            "connector_68df038e0ba48191908c8434991bbac2",
            "Gmail",
            "Find and summarize email threads.",
        ),
        DiscoverableTool::Plugin(Box::new(DiscoverablePluginInfo {
            id: "sample@test".to_string(),
            name: "Sample Plugin".to_string(),
            description: None,
            has_skills: true,
            mcp_server_names: vec!["sample-docs".to_string()],
            app_connector_ids: vec!["connector_sample".to_string()],
        })),
    ];

    let (tools, registry) = build_specs_with_inputs_for_test(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        Some(discoverable_tools),
        /*extension_tool_executors*/ &[],
        &[],
    );
    assert!(registry.has_handler(&ToolName::plain(REQUEST_PLUGIN_INSTALL_TOOL_NAME)));

    let request_plugin_install = find_tool(&tools, REQUEST_PLUGIN_INSTALL_TOOL_NAME);
    let ToolSpec::Function(ResponsesApiTool {
        description,
        parameters,
        ..
    }) = request_plugin_install
    else {
        panic!("expected function tool");
    };
    assert!(description.contains(
        "Use this tool only to ask the user to install one known plugin or connector from the list below. The list contains known candidates that are not currently installed."
    ));
    assert!(description.contains("Google Calendar"));
    assert!(description.contains("Gmail"));
    assert!(description.contains("Sample Plugin"));
    assert!(description.contains("Plan events and schedules."));
    assert!(description.contains("Find and summarize email threads."));
    assert!(description.contains("id: `sample@test`, type: plugin, action: install"));
    assert!(description.contains("`action_type`: `install`"));
    assert!(
        description.contains("skills; MCP servers: sample-docs; app connectors: connector_sample")
    );
    assert!(
        description.contains(
            "The user explicitly asks to use a specific plugin or connector that is not already available in the current context or active `tools` list."
        )
    );
    assert!(description.contains(
        "`tool_search` is not available, or it has already been called and did not find or make the requested tool callable."
    ));
    assert!(description.contains(
        "The plugin or connector is one of the known installable plugins or connectors listed below. Only ask to install plugins or connectors from this list."
    ));
    assert!(description.contains(
        "Do not use this tool for adjacent capabilities, broad recommendations, or tools that merely seem useful."
    ));
    assert!(description.contains("IMPORTANT: DO NOT call this tool in parallel with other tools."));
    assert!(description.contains(
        "If current active tools aren't relevant and `tool_search` is available, only call this tool after `tool_search` has already been tried and found no relevant tool."
    ));
    assert!(!description.contains("targeted lookup"));
    assert!(!description.contains("broad or speculative searches"));
    assert!(description.contains("Only proceed when one listed plugin or connector exactly fits."));
    assert!(description.contains(
        "If we found both connectors and plugins to install, use plugins first, only use connectors if the corresponding plugin is installed but the connector is not."
    ));
    assert!(!description.contains("{{discoverable_tools}}"));
    assert!(!description.contains("tool_search fails to find a good match"));
    let (_, required) = expect_object_schema(parameters);
    assert_eq!(
        required,
        Some(&vec![
            "tool_type".to_string(),
            "action_type".to_string(),
            "tool_id".to_string(),
            "suggest_reason".to_string(),
        ])
    );
}

#[test]
fn code_mode_augments_mcp_tool_descriptions_with_namespaced_sample() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::CodeModeOnly);
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("mcp__sample__", "echo"),
            mcp_tool(
                "echo",
                "Echo text",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": {"type": "string"}
                    },
                    "required": ["message"],
                    "additionalProperties": false
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let ResponsesApiTool { description, .. } =
        find_namespace_function_tool(&tools, "mcp__sample__", "echo");

    assert_eq!(
        description,
        r#"Echo text

exec tool declaration:
```ts
declare const tools: { mcp__sample__echo(args: { message: string; }): Promise<CallToolResult>; };
```"#
    );
}

#[test]
fn code_mode_preserves_nullable_and_literal_mcp_input_shapes() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("mcp__sample__", "fn"),
            mcp_tool(
                "fn",
                "Sample fn",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "open": {
                            "anyOf": [
                                {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "ref_id": {"type": "string"},
                                            "lineno": {"anyOf": [{"type": "integer"}, {"type": "null"}]}
                                        },
                                        "required": ["ref_id"],
                                        "additionalProperties": false
                                    }
                                },
                                {"type": "null"}
                            ]
                        },
                        "tagged_list": {
                            "anyOf": [
                                {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "kind": {"type": "const", "const": "tagged"},
                                            "variant": {"type": "enum", "enum": ["alpha", "beta"]},
                                            "scope": {"type": "enum", "enum": ["one", "two"]}
                                        },
                                        "required": ["kind", "variant", "scope"]
                                    }
                                },
                                {"type": "null"}
                            ]
                        },
                        "response_length": {"type": "enum", "enum": ["short", "medium", "long"]}
                    },
                    "additionalProperties": false
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let ResponsesApiTool { description, .. } =
        find_namespace_function_tool(&tools, "mcp__sample__", "fn");

    assert!(description.contains(
        r#"exec tool declaration:
```ts
declare const tools: { mcp__sample__fn(args: { open?: Array<{ lineno?: number | null; ref_id: string; }> | null; response_length?: "short" | "medium" | "long"; tagged_list?: Array<{ kind: "tagged"; scope: "one" | "two"; variant: "alpha" | "beta"; }> | null; }): Promise<CallToolResult>; };
```"#
    ));
}

#[test]
fn code_mode_augments_builtin_tool_descriptions_with_typed_sample() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let ToolSpec::Function(ResponsesApiTool { description, .. }) =
        find_tool(&tools, VIEW_IMAGE_TOOL_NAME)
    else {
        panic!("expected function tool");
    };

    assert_eq!(
        description,
        "View a local image from the filesystem (only use if given a full filepath by the user, and the image isn't already attached to the thread context within <image ...> tags).\n\nexec tool declaration:\n```ts\ndeclare const tools: { view_image(args: {\n  // Local filesystem path to an image file\n  path: string;\n}): Promise<{\n  // Image detail hint returned by view_image. Returns `original` when original resolution is preserved, otherwise `null`.\n  detail: string | null;\n  // Data URL for the loaded image.\n  image_url: string;\n}>; };\n```"
    );
}

#[test]
fn code_mode_only_exec_description_includes_full_nested_tool_details() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::CodeModeOnly);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let ToolSpec::Freeform(FreeformTool { description, .. }) = find_tool(&tools, "exec") else {
        panic!("expected freeform tool");
    };

    assert!(!description.contains("Enabled nested tools:"));
    assert!(!description.contains("Nested tool reference:"));
    assert!(description.starts_with("Run JavaScript code to orchestrate/compose tool calls"));
    assert!(!description.contains("do not attempt to use any other tools directly"));
    assert!(description.contains("### `update_plan`"));
    assert!(description.contains("### `view_image`"));
}

#[test]
fn code_mode_only_exec_description_includes_extension_tool_details() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::CodeModeOnly);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let extension_tool_executors = vec![extension_tool_executor(
        "extension_echo",
        "Echoes arguments through an extension tool.",
    )];
    let (tools, _) = build_specs_with_inputs_for_test(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        /*discoverable_tools*/ None,
        &extension_tool_executors,
        &[],
    );
    let ToolSpec::Freeform(FreeformTool { description, .. }) = find_tool(&tools, "exec") else {
        panic!("expected freeform tool");
    };

    assert!(description.contains("### `extension_echo`"));
    assert!(description.contains("Echoes arguments through an extension tool."));
}

#[test]
fn code_mode_exec_description_omits_nested_tool_details_when_not_code_mode_only() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        /*mcp_tools*/ None,
        /*deferred_mcp_tools*/ None,
        &[],
    );
    let ToolSpec::Freeform(FreeformTool { description, .. }) = find_tool(&tools, "exec") else {
        panic!("expected freeform tool");
    };

    assert!(!description.starts_with(
        "Use `exec/wait` tool to run all other tools, do not attempt to use any other tools directly"
    ));
    assert!(!description.contains("### `update_plan`"));
    assert!(!description.contains("### `view_image`"));
}

#[test]
fn direct_mcp_tools_register_namespaced_handlers() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (_, registry) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("mcp__test_server__", "echo"),
            mcp_tool("echo", "Echo", serde_json::json!({"type": "object"})),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    assert!(registry.has_handler(&ToolName::namespaced("mcp__test_server__", "echo")));
    assert!(!registry.has_handler(&ToolName::plain("mcp__test_server__echo")));
}

#[test]
fn mcp_tool_property_missing_type_defaults_to_string() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("dash/", "search"),
            mcp_tool(
                "search",
                "Search docs",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"description": "search query"}
                    }
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "dash/", "search");
    assert_eq!(
        *tool,
        ResponsesApiTool {
            name: "search".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "query".to_string(),
                    JsonSchema::string(Some("search query".to_string())),
                )]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "Search docs".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

#[test]
fn mcp_tool_preserves_integer_schema() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("dash/", "paginate"),
            mcp_tool(
                "paginate",
                "Pagination",
                serde_json::json!({
                    "type": "object",
                    "properties": {"page": {"type": "integer"}}
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "dash/", "paginate");
    assert_eq!(
        *tool,
        ResponsesApiTool {
            name: "paginate".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "page".to_string(),
                    JsonSchema::integer(/*description*/ None),
                )]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "Pagination".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

#[test]
fn mcp_tool_array_without_items_gets_default_string_items() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("dash/", "tags"),
            mcp_tool(
                "tags",
                "Tags",
                serde_json::json!({
                    "type": "object",
                    "properties": {"tags": {"type": "array"}}
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "dash/", "tags");
    assert_eq!(
        *tool,
        ResponsesApiTool {
            name: "tags".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "tags".to_string(),
                    JsonSchema::array(
                        JsonSchema::string(/*description*/ None),
                        /*description*/ None,
                    ),
                )]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "Tags".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

#[test]
fn mcp_tool_anyof_defaults_to_string() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("dash/", "value"),
            mcp_tool(
                "value",
                "AnyOf Value",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "value": {"anyOf": [{"type": "string"}, {"type": "number"}]}
                    }
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "dash/", "value");
    assert_eq!(
        *tool,
        ResponsesApiTool {
            name: "value".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([(
                    "value".to_string(),
                    JsonSchema::any_of(
                        vec![
                            JsonSchema::string(/*description*/ None),
                            JsonSchema::number(/*description*/ None),
                        ],
                        /*description*/ None,
                    ),
                )]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "AnyOf Value".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

#[test]
fn mcp_tool_additional_properties_schema_is_preserved() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("test_server/", "do_something_cool"),
            mcp_tool(
                "do_something_cool",
                "Do something cool",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "string_argument": {"type": "string"},
                        "number_argument": {"type": "number"},
                        "object_argument": {
                            "type": "object",
                            "properties": {
                                "string_property": {"type": "string"},
                                "number_property": {"type": "number"}
                            },
                            "required": ["string_property", "number_property"],
                            "additionalProperties": {
                                "type": "object",
                                "properties": {
                                    "addtl_prop": {"type": "string"}
                                },
                                "required": ["addtl_prop"],
                                "additionalProperties": false
                            }
                        }
                    }
                }),
            ),
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let tool = find_namespace_function_tool(&tools, "test_server/", "do_something_cool");
    assert_eq!(
        *tool,
        ResponsesApiTool {
            name: "do_something_cool".to_string(),
            parameters: JsonSchema::object(
                BTreeMap::from([
                    (
                        "string_argument".to_string(),
                        JsonSchema::string(/*description*/ None),
                    ),
                    (
                        "number_argument".to_string(),
                        JsonSchema::number(/*description*/ None),
                    ),
                    (
                        "object_argument".to_string(),
                        JsonSchema::object(
                            BTreeMap::from([
                                (
                                    "string_property".to_string(),
                                    JsonSchema::string(/*description*/ None),
                                ),
                                (
                                    "number_property".to_string(),
                                    JsonSchema::number(/*description*/ None),
                                ),
                            ]),
                            Some(vec![
                                "string_property".to_string(),
                                "number_property".to_string(),
                            ]),
                            Some(
                                JsonSchema::object(
                                    BTreeMap::from([(
                                        "addtl_prop".to_string(),
                                        JsonSchema::string(/*description*/ None),
                                    )]),
                                    Some(vec!["addtl_prop".to_string()]),
                                    Some(false.into()),
                                )
                                .into(),
                            ),
                        ),
                    ),
                ]),
                /*required*/ None,
                /*additional_properties*/ None
            ),
            description: "Do something cool".to_string(),
            strict: false,
            output_schema: Some(mcp_call_tool_result_output_schema(serde_json::json!({}))),
            defer_loading: None,
        }
    );
}

fn model_info() -> ModelInfo {
    serde_json::from_value(json!({
        "slug": "gpt-5-codex",
        "display_name": "GPT-5 Codex",
        "description": null,
        "supported_reasoning_levels": [],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 1,
        "availability_nux": null,
        "upgrade": null,
        "base_instructions": "base",
        "model_messages": null,
        "supports_reasoning_summaries": false,
        "default_reasoning_summary": "auto",
        "support_verbosity": false,
        "default_verbosity": null,
        "apply_patch_tool_type": "freeform",
        "truncation_policy": {
            "mode": "bytes",
            "limit": 10000
        },
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": null,
        "auto_compact_token_limit": null,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text", "image"],
        "supports_search_tool": false
    }))
    .expect("deserialize test model")
}

fn search_capable_model_info() -> ModelInfo {
    ModelInfo {
        supports_search_tool: true,
        ..model_info()
    }
}

fn build_specs(
    config: &ToolsConfig,
    mcp_tools: Option<HashMap<ToolName, McpTool>>,
    deferred_mcp_tools: Option<Vec<ToolInfo>>,
    dynamic_tools: &[DynamicToolSpec],
) -> (Vec<ToolSpec>, SpecToolRegistry) {
    build_specs_with_inputs_for_test(
        config,
        mcp_tools,
        deferred_mcp_tools,
        /*discoverable_tools*/ None,
        /*extension_tool_executors*/ &[],
        dynamic_tools,
    )
}

fn build_specs_with_inputs_for_test(
    config: &ToolsConfig,
    mcp_tools: Option<HashMap<ToolName, McpTool>>,
    deferred_mcp_tools: Option<Vec<ToolInfo>>,
    discoverable_tools: Option<Vec<DiscoverableTool>>,
    extension_tool_executors: &[Arc<dyn ExtensionToolExecutor>],
    dynamic_tools: &[DynamicToolSpec],
) -> (Vec<ToolSpec>, SpecToolRegistry) {
    let mcp_tool_inputs = mcp_tools.as_ref().map(|mcp_tools| {
        mcp_tools
            .iter()
            .map(|(name, tool)| tool_info_from_parts(name, tool.clone()))
            .collect::<Vec<_>>()
    });
    let params = ToolRuntimeBuildParams {
        mcp_tools: mcp_tool_inputs.as_deref(),
        deferred_mcp_tools: deferred_mcp_tools.as_deref(),
        discoverable_tools: discoverable_tools.as_deref(),
        extension_tool_executors,
        dynamic_tools,
        default_agent_type_description: DEFAULT_AGENT_TYPE_DESCRIPTION,
    };
    let host = SpecOnlyToolDomainHost;
    let executors = collect_tool_executors(config, &host, params);
    let builder = build_tool_registry_builder_from_executors(
        config,
        executors,
        hosted_model_tool_specs(config),
        &host,
    );
    builder.build()
}

fn mcp_tool(name: &str, description: &str, input_schema: serde_json::Value) -> McpTool {
    McpTool::new(name, description, input_schema)
}

fn tool_info_from_parts(name: &ToolName, tool: McpTool) -> ToolInfo {
    ToolInfo {
        server_name: server_name_from_tool_name(name),
        supports_parallel_tool_calls: false,
        server_origin: None,
        callable_name: name.name.clone(),
        callable_namespace: name.namespace.clone().unwrap_or_default(),
        namespace_description: None,
        tool,
        connector_id: None,
        connector_name: None,
        plugin_display_names: Vec::new(),
    }
}

fn server_name_from_tool_name(name: &ToolName) -> String {
    name.namespace
        .as_deref()
        .and_then(|namespace| {
            namespace
                .strip_prefix("mcp__")
                .and_then(|suffix| suffix.strip_suffix("__"))
        })
        .unwrap_or_else(|| name.namespace.as_deref().unwrap_or("test_server"))
        .to_string()
}

#[test]
fn code_mode_augments_mcp_tool_descriptions_with_structured_output_sample() {
    let model_info = model_info();
    let mut features = Features::with_defaults();
    features.enable(Feature::CodeMode);
    features.enable(Feature::CodeModeOnly);
    features.enable(Feature::UnifiedExec);
    let available_models = Vec::new();
    let tools_config = ToolsConfig::new(&ToolsConfigParams {
        model_info: &model_info,
        available_models: &available_models,
        features: &features,
        image_generation_tool_auth_allowed: true,
        web_search_mode: Some(WebSearchMode::Cached),
        session_source: SessionSource::Cli,
        permission_profile: &PermissionProfile::Disabled,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
    });

    let mut tool = mcp_tool(
        "echo",
        "Echo text",
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {"type": "string"}
            },
            "required": ["message"],
            "additionalProperties": false
        }),
    );
    tool.output_schema = Some(serde_json::json!({
        "type": "object",
        "properties": {
            "echo": {"type": "string"},
            "env": {
                "anyOf": [
                    {"type": "string"},
                    {"type": "null"}
                ]
            }
        },
        "required": ["echo", "env"],
        "additionalProperties": false
    }));

    let (tools, _) = build_specs(
        &tools_config,
        Some(HashMap::from([(
            ToolName::namespaced("mcp__sample__", "echo"),
            tool,
        )])),
        /*deferred_mcp_tools*/ None,
        &[],
    );

    let ResponsesApiTool { description, .. } =
        find_namespace_function_tool(&tools, "mcp__sample__", "echo");

    assert_eq!(
        description,
        r#"Echo text

exec tool declaration:
```ts
declare const tools: { mcp__sample__echo(args: { message: string; }): Promise<CallToolResult<{ echo: string; env: string | null; }>>; };
```"#
    );
}

fn discoverable_connector(id: &str, name: &str, description: &str) -> DiscoverableTool {
    let slug = name.replace(' ', "-").to_lowercase();
    DiscoverableTool::Connector(Box::new(AppInfo {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        logo_url: None,
        logo_url_dark: None,
        distribution_channel: None,
        branding: None,
        app_metadata: None,
        labels: None,
        install_url: Some(format!("https://chatgpt.com/apps/{slug}/{id}")),
        is_accessible: false,
        is_enabled: true,
        plugin_display_names: Vec::new(),
    }))
}

fn deferred_mcp_tool(
    tool_name: &str,
    tool_namespace: &str,
    server_name: &str,
    connector_name: Option<&str>,
    description: Option<&str>,
) -> ToolInfo {
    ToolInfo {
        server_name: server_name.to_string(),
        supports_parallel_tool_calls: false,
        server_origin: None,
        callable_name: tool_name.to_string(),
        callable_namespace: tool_namespace.to_string(),
        namespace_description: description.map(str::to_string),
        tool: mcp_tool(
            tool_name,
            description.unwrap_or("Deferred MCP tool"),
            json!({}),
        ),
        connector_id: None,
        connector_name: connector_name.map(str::to_string),
        plugin_display_names: Vec::new(),
    }
}

fn assert_contains_tool_names(tools: &[ToolSpec], expected_subset: &[&str]) {
    use std::collections::HashSet;

    let mut names = HashSet::new();
    let mut duplicates = Vec::new();
    for name in tools.iter().map(ToolSpec::name) {
        if !names.insert(name) {
            duplicates.push(name);
        }
    }
    assert!(
        duplicates.is_empty(),
        "duplicate tool entries detected: {duplicates:?}"
    );
    for expected in expected_subset {
        assert!(
            names.contains(expected),
            "expected tool {expected} to be present; had: {names:?}"
        );
    }
}

fn assert_lacks_tool_name(tools: &[ToolSpec], expected_absent: &str) {
    let names = tools.iter().map(ToolSpec::name).collect::<Vec<_>>();
    assert!(
        !names.contains(&expected_absent),
        "expected tool {expected_absent} to be absent; had: {names:?}"
    );
}

fn request_user_input_tool_spec(available_modes: &[ModeKind]) -> ToolSpec {
    create_request_user_input_tool(request_user_input_tool_description(available_modes))
}

fn spawn_agent_tool_options(config: &ToolsConfig) -> SpawnAgentToolOptions {
    SpawnAgentToolOptions {
        available_models: config.available_models.clone(),
        agent_type_description: agent_type_description(config, DEFAULT_AGENT_TYPE_DESCRIPTION)
            .to_string(),
        hide_agent_type_model_reasoning: config.hide_spawn_agent_metadata,
        include_usage_hint: config.spawn_agent_usage_hint,
        usage_hint_text: config.spawn_agent_usage_hint_text.clone(),
        max_concurrent_threads_per_session: config.max_concurrent_threads_per_session,
    }
}

fn find_tool<'a>(tools: &'a [ToolSpec], expected_name: &str) -> &'a ToolSpec {
    tools
        .iter()
        .find(|tool| tool.name() == expected_name)
        .unwrap_or_else(|| panic!("expected tool {expected_name}"))
}

fn assert_process_tool_environment_id(
    tools: &[ToolSpec],
    expected_name: &str,
    expected_present: bool,
) {
    let tool = find_tool(tools, expected_name);
    let ToolSpec::Function(ResponsesApiTool { parameters, .. }) = tool else {
        panic!("expected function tool {expected_name}");
    };
    let (properties, _) = expect_object_schema(parameters);
    assert_eq!(
        properties.contains_key("environment_id"),
        expected_present,
        "{expected_name} environment_id parameter presence"
    );
}

fn assert_apply_patch_environment_id(tools: &[ToolSpec], expected_present: bool) {
    let tool = find_tool(tools, "apply_patch");
    let ToolSpec::Freeform(FreeformTool { format, .. }) = tool else {
        panic!("expected freeform apply_patch tool");
    };
    assert_eq!(
        format.definition.contains("environment_id?"),
        expected_present,
        "apply_patch environment_id grammar presence"
    );
}

fn find_namespace_function_tool<'a>(
    tools: &'a [ToolSpec],
    expected_namespace: &str,
    expected_name: &str,
) -> &'a ResponsesApiTool {
    let namespace_tool = find_tool(tools, expected_namespace);
    let ToolSpec::Namespace(namespace) = namespace_tool else {
        panic!("expected namespace tool {expected_namespace}");
    };
    namespace
        .tools
        .iter()
        .find_map(|tool| match tool {
            ResponsesApiNamespaceTool::Function(tool) if tool.name == expected_name => Some(tool),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected tool {expected_namespace}{expected_name} in namespace"))
}

fn namespace_function_names(tools: &[ToolSpec], expected_namespace: &str) -> Vec<String> {
    let namespace_tool = find_tool(tools, expected_namespace);
    let ToolSpec::Namespace(namespace) = namespace_tool else {
        panic!("expected namespace tool {expected_namespace}");
    };
    namespace
        .tools
        .iter()
        .map(|tool| match tool {
            ResponsesApiNamespaceTool::Function(tool) => tool.name.clone(),
        })
        .collect()
}

fn expect_object_schema(
    schema: &JsonSchema,
) -> (&BTreeMap<String, JsonSchema>, Option<&Vec<String>>) {
    assert_eq!(
        schema.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );
    let properties = schema
        .properties
        .as_ref()
        .expect("expected object properties");
    (properties, schema.required.as_ref())
}

fn expect_string_description(schema: &JsonSchema) -> &str {
    assert_eq!(
        schema.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::String))
    );
    schema.description.as_deref().expect("expected description")
}

fn strip_descriptions_schema(schema: &mut JsonSchema) {
    if let Some(variants) = &mut schema.any_of {
        for variant in variants {
            strip_descriptions_schema(variant);
        }
    }
    if let Some(items) = &mut schema.items {
        strip_descriptions_schema(items);
    }
    if let Some(properties) = &mut schema.properties {
        for value in properties.values_mut() {
            strip_descriptions_schema(value);
        }
    }
    if let Some(AdditionalProperties::Schema(schema)) = &mut schema.additional_properties {
        strip_descriptions_schema(schema);
    }
    schema.description = None;
}

fn strip_descriptions_tool(spec: &mut ToolSpec) {
    match spec {
        ToolSpec::ToolSearch { parameters, .. } => strip_descriptions_schema(parameters),
        ToolSpec::Function(ResponsesApiTool { parameters, .. }) => {
            strip_descriptions_schema(parameters);
        }
        ToolSpec::Namespace(namespace) => {
            for tool in &mut namespace.tools {
                match tool {
                    ResponsesApiNamespaceTool::Function(ResponsesApiTool {
                        parameters, ..
                    }) => {
                        strip_descriptions_schema(parameters);
                    }
                }
            }
        }
        ToolSpec::Freeform(FreeformTool { .. })
        | ToolSpec::ImageGeneration { .. }
        | ToolSpec::WebSearch { .. } => {}
    }
}
