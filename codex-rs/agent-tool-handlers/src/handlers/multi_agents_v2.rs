use codex_protocol::AgentPath;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabListAgentsBeginEvent;
use codex_protocol::protocol::CollabListAgentsEndEvent;
use codex_protocol::protocol::CollabListedAgent;
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
use codex_tool_runtime_api::CloseAgentToolResult;
use codex_tool_runtime_api::ListAgentsToolResult;
use codex_tool_runtime_api::MultiAgentToolHost;
use codex_tool_runtime_api::SpawnAgentToolResult;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_runtime_api::ToolInvocationView;
use codex_tool_runtime_api::WaitAgentToolResult;
use codex_tool_runtime_api::followup_task_from_arguments;
use codex_tool_runtime_api::function_arguments_from_payload;
use codex_tool_runtime_api::run_followup_task_tool;
use codex_tool_runtime_api::run_spawn_agent_tool;
use codex_tool_runtime_api::run_wait_agent_tool;
use codex_tool_runtime_api::spawn_agent_request_from_arguments;
use codex_tool_runtime_api::wait_agent_target_from_arguments;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[derive(Default)]
pub struct SpawnAgentHandler<Host> {
    host: Host,
    options: SpawnAgentToolOptions,
}

impl<Host> SpawnAgentHandler<Host> {
    pub fn new(host: Host, options: SpawnAgentToolOptions) -> Self {
        Self { host, options }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for SpawnAgentHandler<Host>
where
    Host: MultiAgentToolHost,
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
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_spawn_agent(&self.host, invocation).await })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for SpawnAgentHandler<Host>
where
    Host: MultiAgentToolHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct FollowupTaskHandler<Host> {
    host: Host,
}

impl<Host> FollowupTaskHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for FollowupTaskHandler<Host>
where
    Host: MultiAgentToolHost,
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
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let arguments = function_arguments_from_payload(invocation.payload().clone())?;
            let (target, message) = followup_task_from_arguments(&arguments)?;
            handle_message_string_tool(&self.host, invocation, target, message).await
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for FollowupTaskHandler<Host>
where
    Host: MultiAgentToolHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct WaitAgentHandler<Host> {
    host: Host,
}

impl<Host> WaitAgentHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for WaitAgentHandler<Host>
where
    Host: MultiAgentToolHost,
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
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_wait_agent(&self.host, invocation).await })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for WaitAgentHandler<Host>
where
    Host: MultiAgentToolHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct CloseAgentHandler<Host> {
    host: Host,
}

impl<Host> CloseAgentHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for CloseAgentHandler<Host>
where
    Host: MultiAgentToolHost,
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
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_close_agent(&self.host, invocation).await })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for CloseAgentHandler<Host>
where
    Host: MultiAgentToolHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub struct ListAgentsHandler<Host> {
    host: Host,
}

impl<Host> ListAgentsHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ListAgentsHandler<Host>
where
    Host: MultiAgentToolHost,
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
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_list_agents(&self.host, invocation).await })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ListAgentsHandler<Host>
where
    Host: MultiAgentToolHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

pub async fn handle_workflow_spawn_agent<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<JsonValue, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let result = handle_spawn_agent(host, invocation).await?;
    serde_json::to_value(result.0).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow spawn result: {err}"))
    })
}

pub async fn handle_workflow_followup_task<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    target: String,
    message: String,
) -> Result<JsonValue, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    handle_message_string_tool(host, invocation, target, message).await?;
    Ok(serde_json::json!({ "ok": true }))
}

pub async fn handle_workflow_wait_agent<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<JsonValue, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let result = handle_wait_agent(host, invocation).await?;
    serde_json::to_value(result.0).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow wait result: {err}"))
    })
}

async fn handle_spawn_agent<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<SpawnAgentOutput, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments_from_payload(metadata.payload)?;
    let request = spawn_agent_request_from_arguments(&arguments)?;
    let result = run_spawn_agent_tool(host, session, turn, call_id, request).await?;
    Ok(SpawnAgentOutput(result))
}

async fn handle_message_string_tool<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    target: String,
    message: String,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    run_followup_task_tool(host, session, turn, call_id, target, message).await?;
    Ok(FunctionToolOutput::from_text(String::new(), Some(true)))
}

async fn handle_close_agent<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<CloseAgentOutput, FunctionCallError>
where
    Host: MultiAgentToolHost,
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
    let sender_thread_id = host.thread_id(&session);
    let sender_agent_path = host.sender_agent_path(&session, &turn);
    let agent_id = host
        .resolve_agent_target(&session, &turn, &args.target)
        .await?;
    let receiver_agent = host.agent_metadata(&session, agent_id);
    reject_root_agent(
        receiver_agent.agent_path.as_ref(),
        "root is not a spawned agent",
    )?;
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let receiver_is_direct_child = is_direct_child(&sender_agent_path, &receiver_agent_path);
    host.send_collab_event(
        &session,
        &turn,
        CollabCloseBeginEvent {
            call_id: call_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.to_string(),
            receiver_thread_id: agent_id,
            receiver_agent_path: receiver_agent_path.to_string(),
        }
        .into(),
    )
    .await;
    let status = host.agent_status(&session, agent_id).await;
    let result = host.close_agent(&session, agent_id).await;
    host.send_collab_event(
        &session,
        &turn,
        CollabCloseEndEvent {
            call_id,
            completed_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.to_string(),
            receiver_thread_id: agent_id,
            receiver_agent_path: receiver_agent_path.to_string(),
            receiver_agent_nickname: receiver_agent.agent_nickname,
            receiver_agent_role: receiver_agent.agent_role,
            status: status.clone(),
        }
        .into(),
    )
    .await;
    result?;
    if receiver_is_direct_child
        && host
            .clear_direct_child_completion_pending(&session, agent_id)
            .await
    {
        host.maybe_notify_parent_of_final_status(&session).await;
    }

    Ok(CloseAgentOutput(CloseAgentToolResult {
        previous_status: status,
    }))
}

async fn handle_list_agents<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<ListAgentsOutput, FunctionCallError>
where
    Host: MultiAgentToolHost,
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
    let sender_thread_id = host.thread_id(&session);
    let sender_agent_path = host.sender_agent_path(&session, &turn).to_string();
    host.send_collab_event(
        &session,
        &turn,
        CollabListAgentsBeginEvent {
            call_id: call_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.clone(),
            path_prefix: args.path_prefix.clone(),
        }
        .into(),
    )
    .await;
    host.register_session_root(&session, &turn);
    let agents = host
        .list_agents(&session, &turn, args.path_prefix.as_deref())
        .await;
    let listed_agents = agents.as_ref().map_or_else(
        |_| Vec::new(),
        |agents| {
            agents
                .iter()
                .map(|agent| CollabListedAgent {
                    agent_path: agent.agent_name.clone(),
                    status: agent.agent_status.clone(),
                    last_task_message: agent.last_task_message.clone(),
                })
                .collect()
        },
    );
    host.send_collab_event(
        &session,
        &turn,
        CollabListAgentsEndEvent {
            call_id,
            completed_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path,
            path_prefix: args.path_prefix,
            success: agents.is_ok(),
            agents: listed_agents,
        }
        .into(),
    )
    .await;

    Ok(ListAgentsOutput(ListAgentsToolResult { agents: agents? }))
}

async fn handle_wait_agent<Host>(
    host: &Host,
    invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<WaitAgentOutput, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let ToolInvocation {
        session,
        turn,
        metadata,
        ..
    } = invocation;
    let call_id = metadata.call_id;
    let arguments = function_arguments_from_payload(metadata.payload)?;
    let target = wait_agent_target_from_arguments(&arguments)?;
    let result = run_wait_agent_tool(host, session, turn, call_id, target).await?;

    Ok(WaitAgentOutput(result))
}

fn reject_root_agent(
    agent_path: Option<&AgentPath>,
    message: &str,
) -> Result<(), FunctionCallError> {
    if agent_path.is_some_and(AgentPath::is_root) {
        return Err(FunctionCallError::RespondToModel(message.to_string()));
    }
    Ok(())
}

fn is_direct_child(sender_agent_path: &AgentPath, receiver_agent_path: &AgentPath) -> bool {
    receiver_agent_path
        .as_str()
        .rsplit_once('/')
        .is_some_and(|(parent, _)| parent == sender_agent_path.as_str())
}

fn function_arguments(payload: ToolPayload) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "collab handler received unsupported payload".to_string(),
        )),
    }
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
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
