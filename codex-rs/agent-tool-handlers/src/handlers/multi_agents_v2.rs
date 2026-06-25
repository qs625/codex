use codex_agent_runtime::AgentMode;
use codex_agent_runtime::CloseAgentToolResult;
use codex_agent_runtime::ListAgentsToolResult;
use codex_agent_runtime::MultiAgentToolSession;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_agent_runtime::SpawnAgentToolResult;
use codex_agent_runtime::WaitAgentToolResult;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_tool_planning::SpawnAgentToolOptions;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_close_agent_tool_v2;
use codex_tool_planning::create_followup_task_tool;
use codex_tool_planning::create_list_agents_tool;
use codex_tool_planning::create_spawn_agent_tool_v2;
use codex_tool_planning::create_wait_agent_tool_v2;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_runtime_api::ToolInvocationView;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnAgentArgs {
    message: String,
    task_name: String,
    agent_type: Option<String>,
    cwd: Option<AbsolutePathBuf>,
    model: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    service_tier: Option<String>,
    agent_mode: Option<AgentMode>,
    fork_turns: Option<String>,
    fork_context: Option<bool>,
}

impl SpawnAgentArgs {
    fn into_request(self) -> Result<SpawnAgentToolRequest, FunctionCallError> {
        let fork_mode = self.fork_mode()?;
        Ok(SpawnAgentToolRequest {
            message: self.message,
            task_name: self.task_name,
            agent_type: self.agent_type,
            cwd: self.cwd,
            model: self.model,
            reasoning_effort: self.reasoning_effort,
            service_tier: self.service_tier,
            agent_mode: self.agent_mode,
            fork_mode,
        })
    }

    fn fork_mode(&self) -> Result<Option<SpawnAgentForkMode>, FunctionCallError> {
        if self.fork_context.is_some() {
            return Err(FunctionCallError::RespondToModel(
                "fork_context is not supported in MultiAgentV2; use fork_turns instead".to_string(),
            ));
        }

        let fork_turns = self
            .fork_turns
            .as_deref()
            .map(str::trim)
            .filter(|fork_turns| !fork_turns.is_empty())
            .unwrap_or("all");

        if fork_turns.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        if fork_turns.eq_ignore_ascii_case("all") {
            return Ok(Some(SpawnAgentForkMode::FullHistory));
        }

        let last_n_turns = fork_turns.parse::<usize>().map_err(|_| {
            FunctionCallError::RespondToModel(
                "fork_turns must be `none`, `all`, or a positive integer string".to_string(),
            )
        })?;
        if last_n_turns == 0 {
            return Err(FunctionCallError::RespondToModel(
                "fork_turns must be `none`, `all`, or a positive integer string".to_string(),
            ));
        }

        Ok(Some(SpawnAgentForkMode::LastNTurns(last_n_turns)))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowupTaskArgs {
    target: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitAgentArgs {
    target: String,
}

#[derive(Default)]
pub struct SpawnAgentHandler {
    options: SpawnAgentToolOptions,
}

impl SpawnAgentHandler {
    pub fn new(options: SpawnAgentToolOptions) -> Self {
        Self { options }
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for SpawnAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = SpawnAgentOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("spawn_agent")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_spawn_agent_tool_v2(self.options.clone()))
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_spawn_agent(invocation).await })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for SpawnAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct FollowupTaskHandler;

impl FollowupTaskHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for FollowupTaskHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("followup_task")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_followup_task_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let arguments = function_arguments(invocation.payload().clone())?;
            let (target, message) = followup_task_from_arguments(&arguments)?;
            handle_message_string_tool(invocation, target, message).await
        })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for FollowupTaskHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct WaitAgentHandler;

impl WaitAgentHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for WaitAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = WaitAgentOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("wait_agent")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_wait_agent_tool_v2())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_wait_agent(invocation).await })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for WaitAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct CloseAgentHandler;

impl CloseAgentHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for CloseAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = CloseAgentOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("close_agent")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_close_agent_tool_v2())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_close_agent(invocation).await })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for CloseAgentHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct ListAgentsHandler;

impl ListAgentsHandler {
    pub fn new() -> Self {
        Self
    }
}

impl<Session, Turn, Tracker> ToolExecutor<ToolInvocation<Session, Turn, Tracker>>
    for ListAgentsHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    type Output = ListAgentsOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_agents")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_list_agents_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Session, Turn, Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_list_agents(invocation).await })
    }
}

impl<Session, Turn, Tracker, DiffContext>
    ToolHandler<ToolInvocation<Session, Turn, Tracker>, DiffContext> for ListAgentsHandler
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub async fn handle_workflow_spawn_agent<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<JsonValue, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let result = handle_spawn_agent(invocation).await?;
    serde_json::to_value(result.0).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow spawn result: {err}"))
    })
}

pub async fn handle_workflow_followup_task<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
    target: String,
    message: String,
) -> Result<JsonValue, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    handle_message_string_tool(invocation, target, message).await?;
    Ok(serde_json::json!({ "ok": true }))
}

pub async fn handle_workflow_wait_agent<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<JsonValue, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let result = handle_wait_agent(invocation).await?;
    serde_json::to_value(result.0).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow wait result: {err}"))
    })
}

async fn handle_spawn_agent<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<SpawnAgentOutput, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments(metadata.payload)?;
    let request = spawn_agent_request_from_arguments(&arguments)?;
    let result = Arc::new(session)
        .spawn_agent_tool(&turn, call_id, request)
        .await?;
    Ok(SpawnAgentOutput(result))
}

async fn handle_message_string_tool<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
    target: String,
    message: String,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    Arc::new(session)
        .followup_task_tool(&turn, call_id, target, message)
        .await?;
    Ok(FunctionToolOutput::from_text(String::new(), Some(true)))
}

async fn handle_close_agent<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<CloseAgentOutput, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments(metadata.payload)?;
    let args: CloseAgentArgs = parse_arguments(&arguments)?;
    let result = Arc::new(session)
        .close_agent_tool(&turn, call_id, args.target)
        .await?;
    Ok(CloseAgentOutput(result))
}

async fn handle_list_agents<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<ListAgentsOutput, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments(metadata.payload)?;
    let args: ListAgentsArgs = parse_arguments(&arguments)?;
    let result = Arc::new(session)
        .list_agents_tool(&turn, call_id, args.path_prefix)
        .await?;
    Ok(ListAgentsOutput(result))
}

async fn handle_wait_agent<Session, Turn, Tracker>(
    invocation: ToolInvocation<Session, Turn, Tracker>,
) -> Result<WaitAgentOutput, FunctionCallError>
where
    Session: MultiAgentToolSession<Turn>,
    Turn: Clone + Send + Sync + 'static,
    Tracker: Clone + Send + Sync + 'static,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments(metadata.payload)?;
    let target = wait_agent_target_from_arguments(&arguments)?;
    let result = Arc::new(session)
        .wait_agent_tool(&turn, call_id, target)
        .await?;

    Ok(WaitAgentOutput(result))
}

fn function_arguments(payload: ToolPayload) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "collab handler received unsupported payload".to_string(),
        )),
    }
}

fn spawn_agent_request_from_arguments(
    arguments: &str,
) -> Result<SpawnAgentToolRequest, FunctionCallError> {
    let args: SpawnAgentArgs = parse_arguments_with_base_path(arguments, None)?;
    args.into_request()
}

fn followup_task_from_arguments(arguments: &str) -> Result<(String, String), FunctionCallError> {
    let args: FollowupTaskArgs = parse_arguments(arguments)?;
    Ok((args.target, args.message))
}

fn wait_agent_target_from_arguments(arguments: &str) -> Result<String, FunctionCallError> {
    let args: WaitAgentArgs = parse_arguments(arguments)?;
    Ok(args.target)
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: Option<&AbsolutePathBuf>,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = base_path.map(|path| AbsolutePathBufGuard::new(path.as_path()));
    parse_arguments(arguments)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseAgentArgs {
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAgentsArgs {
    path_prefix: Option<String>,
}

pub struct SpawnAgentOutput(pub SpawnAgentToolResult);
pub struct WaitAgentOutput(pub WaitAgentToolResult);
pub struct CloseAgentOutput(pub CloseAgentToolResult);
pub struct ListAgentsOutput(pub ListAgentsToolResult);

impl ToolOutput for SpawnAgentOutput {
    fn log_preview(&self) -> String {
        tool_output_json_text(&self.0, "spawn_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, &self.0, Some(true), "spawn_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(&self.0, "spawn_agent")
    }
}

impl ToolOutput for WaitAgentOutput {
    fn log_preview(&self) -> String {
        tool_output_json_text(&self.0, "wait_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, &self.0, Some(true), "wait_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(&self.0, "wait_agent")
    }
}

impl ToolOutput for CloseAgentOutput {
    fn log_preview(&self) -> String {
        tool_output_json_text(&self.0, "close_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, &self.0, Some(true), "close_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(&self.0, "close_agent")
    }
}

impl ToolOutput for ListAgentsOutput {
    fn log_preview(&self) -> String {
        tool_output_json_text(&self.0, "list_agents")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, &self.0, Some(true), "list_agents")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(&self.0, "list_agents")
    }
}

fn tool_output_json_text<T>(value: &T, tool_name: &str) -> String
where
    T: Serialize,
{
    serde_json::to_string(value).unwrap_or_else(|err| {
        JsonValue::String(format!("failed to serialize {tool_name} result: {err}")).to_string()
    })
}

fn tool_output_response_item<T>(
    call_id: &str,
    payload: &ToolPayload,
    value: &T,
    success: Option<bool>,
    tool_name: &str,
) -> ResponseInputItem
where
    T: Serialize,
{
    FunctionToolOutput::from_text(tool_output_json_text(value, tool_name), success)
        .to_response_item(call_id, payload)
}

fn tool_output_code_mode_result<T>(value: &T, tool_name: &str) -> JsonValue
where
    T: Serialize,
{
    serde_json::to_value(value).unwrap_or_else(|err| {
        JsonValue::String(format!("failed to serialize {tool_name} result: {err}"))
    })
}
