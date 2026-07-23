mod agent_jobs;

use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::planning::SpawnAgentToolOptions;
use crate::planning::ToolSpec;
use crate::planning::create_close_agent_tool_v2;
use crate::planning::create_close_external_agent_tool;
use crate::planning::create_followup_external_task_tool;
use crate::planning::create_followup_task_tool;
use crate::planning::create_list_agents_tool;
use crate::planning::create_list_external_agents_tool;
use crate::planning::create_poll_event_tool;
use crate::planning::create_poll_external_event_tool;
use crate::planning::create_report_agent_job_result_tool;
use crate::planning::create_spawn_agent_tool_v2;
use crate::planning::create_spawn_agents_on_csv_tool;
use crate::planning::create_spawn_external_agent_tool;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentProvider;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::BuiltinToolCallDisplayEvent;
use protocol::protocol::BuiltinToolCallStatus;
use protocol::protocol::EventMsg;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use thread_service_api::SessionAgentJobCaller;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadServiceApi;
use thread_service_api::ThreadSessionCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolPayload;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

const SPAWN_AGENT_TOOL_NAME: &str = "spawn_agent";
const FOLLOWUP_TASK_TOOL_NAME: &str = "followup_task";
const POLL_EVENT_TOOL_NAME: &str = "poll_event";
const LIST_AGENTS_TOOL_NAME: &str = "list_agents";
const CLOSE_AGENT_TOOL_NAME: &str = "close_agent";
const SPAWN_AGENTS_ON_CSV_TOOL_NAME: &str = "spawn_agents_on_csv";
const REPORT_AGENT_JOB_RESULT_TOOL_NAME: &str = "report_agent_job_result";
const SPAWN_EXTERNAL_AGENT_TOOL_NAME: &str = "spawn_external_agent";
const FOLLOWUP_EXTERNAL_TASK_TOOL_NAME: &str = "followup_external_task";
const POLL_EXTERNAL_EVENT_TOOL_NAME: &str = "poll_external_event";
const LIST_EXTERNAL_AGENTS_TOOL_NAME: &str = "list_external_agents";
const CLOSE_EXTERNAL_AGENT_TOOL_NAME: &str = "close_external_agent";

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    vec![
        create_spawn_agent_tool_v2(SpawnAgentToolOptions {
            available_models: request.config.available_models.clone(),
            agent_type_description: request.params.default_agent_type_description.to_string(),
            hide_agent_type_model_reasoning: false,
            include_usage_hint: false,
            usage_hint_text: None,
            max_concurrent_threads_per_session: None,
        }),
        create_followup_task_tool(),
        create_poll_event_tool(),
        create_list_agents_tool(),
        create_close_agent_tool_v2(),
        create_spawn_external_agent_tool(),
        create_followup_external_task_tool(),
        create_poll_external_event_tool(),
        create_list_external_agents_tool(),
        create_close_external_agent_tool(),
        create_spawn_agents_on_csv_tool(),
        create_report_agent_job_result_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            SPAWN_AGENT_TOOL_NAME
                | FOLLOWUP_TASK_TOOL_NAME
                | POLL_EVENT_TOOL_NAME
                | LIST_AGENTS_TOOL_NAME
                | CLOSE_AGENT_TOOL_NAME
                | SPAWN_EXTERNAL_AGENT_TOOL_NAME
                | FOLLOWUP_EXTERNAL_TASK_TOOL_NAME
                | POLL_EXTERNAL_EVENT_TOOL_NAME
                | LIST_EXTERNAL_AGENTS_TOOL_NAME
                | CLOSE_EXTERNAL_AGENT_TOOL_NAME
                | SPAWN_AGENTS_ON_CSV_TOOL_NAME
                | REPORT_AGENT_JOB_RESULT_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    session_capability: Arc<dyn ThreadSessionCapability>,
    session: Arc<dyn SessionAgentJobCaller>,
    thread_service_api: Arc<dyn ThreadServiceApi>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match call.tool_name.name.as_str() {
        SPAWN_AGENT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let request = spawn_agent_request_from_arguments(&arguments)?;
            let result = thread_service_api
                .spawn_agent(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    from_runtime_spawn_request(request),
                )
                .await?;
            function_tool_json_output(&result, SPAWN_AGENT_TOOL_NAME)?
        }
        FOLLOWUP_TASK_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let (target, message) = followup_task_from_arguments(&arguments)?;
            thread_service_api
                .followup_task(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    target,
                    message,
                )
                .await?;
            FunctionToolOutput::from_text(String::new(), Some(true))
        }
        SPAWN_EXTERNAL_AGENT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let args: SpawnExternalAgentArgs = parse_arguments_with_base_path(&arguments, None)?;
            let result = thread_service_api
                .spawn_external_agent(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    thread_service_api::ThreadSpawnExternalAgentRequest {
                        message: args.message,
                        task_name: args.task_name,
                        provider: to_thread_spawn_provider(args.provider),
                        cwd: args.cwd,
                    },
                )
                .await?;
            function_tool_json_output(&result, SPAWN_EXTERNAL_AGENT_TOOL_NAME)?
        }
        FOLLOWUP_EXTERNAL_TASK_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let (target, message) = followup_task_from_arguments(&arguments)?;
            thread_service_api
                .followup_external_task(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    target,
                    message,
                )
                .await?;
            FunctionToolOutput::from_text(String::new(), Some(true))
        }
        POLL_EXTERNAL_EVENT_TOOL_NAME => {
            return Err(FunctionCallError::RespondToModel(
                "poll_external_event is not supported until external CLI sessions expose an interactive input channel; external tool calls are handled by the backend bridge when they appear in provider output".to_string(),
            ));
        }
        LIST_EXTERNAL_AGENTS_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let args: ListAgentsArgs = parse_arguments(&arguments)?;
            let result = thread_service_api
                .list_external_agents(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    args.path_prefix,
                )
                .await?;
            function_tool_json_output(&result, LIST_EXTERNAL_AGENTS_TOOL_NAME)?
        }
        CLOSE_EXTERNAL_AGENT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let args: CloseAgentArgs = parse_arguments(&arguments)?;
            let result = thread_service_api
                .close_external_agent(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    args.target,
                )
                .await?;
            function_tool_json_output(&result, CLOSE_EXTERNAL_AGENT_TOOL_NAME)?
        }
        POLL_EVENT_TOOL_NAME => {
            let item_id = format!("builtin-tool-{}", uuid::Uuid::new_v4());
            let arguments = json!({});
            let timeout_metadata = thread_service_api
                .poll_event_timeout_metadata(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    thread_service_api::ThreadPollEventRequest {
                        initial_timeout_ms: None,
                        hard_cap_timeout_ms: None,
                    },
                )
                .await;
            let started_output = match timeout_metadata {
                Ok(timeout_metadata) => Some(
                    serde_json::to_value(&timeout_metadata)
                        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?,
                ),
                Err(_) => None,
            };
            session_capability
                .emit_event(
                    turn.as_ref(),
                    EventMsg::BuiltinToolCallStarted(BuiltinToolCallDisplayEvent {
                        thread_id: session_capability.conversation_id(),
                        turn_id: turn.runtime_turn_id_str().to_string(),
                        id: item_id.clone(),
                        tool: POLL_EVENT_TOOL_NAME.to_string(),
                        arguments: arguments.clone(),
                        status: BuiltinToolCallStatus::InProgress,
                        output: started_output,
                        lifecycle_at_ms: now_unix_timestamp_ms(),
                    }),
                )
                .await;
            let result = thread_service_api
                .poll_event(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    thread_service_api::ThreadPollEventRequest {
                        initial_timeout_ms: None,
                        hard_cap_timeout_ms: None,
                    },
                )
                .await;
            match result {
                Ok(result) => {
                    let output = serde_json::to_value(&result)
                        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                    session_capability
                        .emit_event(
                            turn.as_ref(),
                            EventMsg::BuiltinToolCallCompleted(BuiltinToolCallDisplayEvent {
                                thread_id: session_capability.conversation_id(),
                                turn_id: turn.runtime_turn_id_str().to_string(),
                                id: item_id,
                                tool: POLL_EVENT_TOOL_NAME.to_string(),
                                arguments,
                                status: BuiltinToolCallStatus::Completed,
                                output: Some(output),
                                lifecycle_at_ms: now_unix_timestamp_ms(),
                            }),
                        )
                        .await;
                    function_tool_json_output(&result, POLL_EVENT_TOOL_NAME)?
                }
                Err(err) => {
                    session_capability
                        .emit_event(
                            turn.as_ref(),
                            EventMsg::BuiltinToolCallCompleted(BuiltinToolCallDisplayEvent {
                                thread_id: session_capability.conversation_id(),
                                turn_id: turn.runtime_turn_id_str().to_string(),
                                id: item_id,
                                tool: POLL_EVENT_TOOL_NAME.to_string(),
                                arguments,
                                status: BuiltinToolCallStatus::Failed,
                                output: Some(json!({
                                    "error": err.to_string(),
                                })),
                                lifecycle_at_ms: now_unix_timestamp_ms(),
                            }),
                        )
                        .await;
                    return Err(err);
                }
            }
        }
        LIST_AGENTS_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let args: ListAgentsArgs = parse_arguments(&arguments)?;
            let result = thread_service_api
                .list_agents(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    args.path_prefix,
                )
                .await?;
            function_tool_json_output(&result, LIST_AGENTS_TOOL_NAME)?
        }
        CLOSE_AGENT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let args: CloseAgentArgs = parse_arguments(&arguments)?;
            let result = thread_service_api
                .close_agent(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    args.target,
                )
                .await?;
            function_tool_json_output(&result, CLOSE_AGENT_TOOL_NAME)?
        }
        SPAWN_AGENTS_ON_CSV_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            agent_jobs::handle_spawn_agents_on_csv(session, turn, arguments).await?
        }
        REPORT_AGENT_JOB_RESULT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            agent_jobs::handle_report_agent_job_result(session, arguments).await?
        }
        _ => {
            return Err(FunctionCallError::Fatal(format!(
                "unsupported agent tool {}",
                call.tool_name
            )));
        }
    };

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn from_runtime_spawn_request(
    request: SpawnAgentToolRequest,
) -> thread_service_api::ThreadSpawnAgentRequest {
    thread_service_api::ThreadSpawnAgentRequest {
        message: request.message,
        task_name: request.task_name,
        provider: request.provider.map(|provider| match provider {
            codex_agent_runtime::SpawnAgentProvider::Native => {
                thread_service_api::ThreadSpawnAgentProvider::Native
            }
            codex_agent_runtime::SpawnAgentProvider::CodexCli => {
                thread_service_api::ThreadSpawnAgentProvider::CodexCli
            }
            codex_agent_runtime::SpawnAgentProvider::ClaudeCli => {
                thread_service_api::ThreadSpawnAgentProvider::ClaudeCli
            }
            codex_agent_runtime::SpawnAgentProvider::Opencode => {
                thread_service_api::ThreadSpawnAgentProvider::Opencode
            }
        }),
        agent_type: request.agent_type,
        cwd: request.cwd,
        model: request.model,
        reasoning_effort: request.reasoning_effort,
        service_tier: request.service_tier,
        fork_mode: request.fork_mode.map(|mode| match mode {
            SpawnAgentForkMode::FullHistory => {
                thread_service_api::ThreadSpawnAgentForkMode::FullHistory
            }
            SpawnAgentForkMode::LastNTurns(last_n_turns) => {
                thread_service_api::ThreadSpawnAgentForkMode::LastNTurns { last_n_turns }
            }
        }),
    }
}

fn to_thread_spawn_provider(
    provider: SpawnAgentProvider,
) -> thread_service_api::ThreadSpawnAgentProvider {
    match provider {
        codex_agent_runtime::SpawnAgentProvider::Native => {
            thread_service_api::ThreadSpawnAgentProvider::Native
        }
        codex_agent_runtime::SpawnAgentProvider::CodexCli => {
            thread_service_api::ThreadSpawnAgentProvider::CodexCli
        }
        codex_agent_runtime::SpawnAgentProvider::ClaudeCli => {
            thread_service_api::ThreadSpawnAgentProvider::ClaudeCli
        }
        codex_agent_runtime::SpawnAgentProvider::Opencode => {
            thread_service_api::ThreadSpawnAgentProvider::Opencode
        }
    }
}

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
    fork_turns: Option<String>,
    fork_context: Option<bool>,
}

impl SpawnAgentArgs {
    fn into_request(self) -> Result<SpawnAgentToolRequest, FunctionCallError> {
        let fork_mode = self.fork_mode()?;
        Ok(SpawnAgentToolRequest {
            message: self.message,
            task_name: self.task_name,
            provider: None,
            agent_type: self.agent_type,
            cwd: self.cwd,
            model: self.model,
            reasoning_effort: self.reasoning_effort,
            service_tier: self.service_tier,
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
struct SpawnExternalAgentArgs {
    message: String,
    task_name: String,
    provider: SpawnAgentProvider,
    cwd: AbsolutePathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowupTaskArgs {
    target: String,
    message: String,
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

fn function_arguments(call: &ToolCall) -> Result<String, FunctionCallError> {
    match &call.payload {
        ToolPayload::Function { arguments } => Ok(arguments.clone()),
        _ => Err(FunctionCallError::RespondToModel(format!(
            "{} received unsupported payload",
            call.tool_name
        ))),
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

fn function_tool_json_output<T>(
    value: &T,
    tool_name: &str,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    T: Serialize,
{
    serde_json::to_string(value)
        .map(|text| FunctionToolOutput::from_text(text, Some(true)))
        .map_err(|err| {
            FunctionCallError::Fatal(format!("failed to serialize {tool_name} result: {err}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    use protocol::AgentPath;
    use protocol::ThreadId;
    use protocol::protocol::SessionSource;
    use protocol::protocol::SubAgentSource;
    use thread_service::test_support;
    use thread_service_api::NativeAgentRuntime;
    use thread_service_api::ThreadCloseAgentResult;
    use thread_service_api::ThreadCollaborationRuntime;
    use thread_service_api::ThreadEventRuntime;
    use thread_service_api::ThreadLifecycleRuntime;
    use thread_service_api::ThreadListAgentsResult;
    use thread_service_api::ThreadPollEventRequest;
    use thread_service_api::ThreadPollEventResult;
    use thread_service_api::ThreadPollEventTimeoutMetadata;
    use thread_service_api::ThreadServiceFuture;
    use thread_service_api::ThreadSpawnAgentRequest;
    use thread_service_api::ThreadSpawnAgentResult;
    use thread_service_api::ThreadSpawnExternalAgentRequest;

    struct StubThreadServiceApi;

    impl ThreadLifecycleRuntime for StubThreadServiceApi {
        fn shutdown_all_threads_bounded<'a>(
            &'a self,
            _timeout: std::time::Duration,
        ) -> ThreadServiceFuture<'a, thread_service_api::ThreadShutdownReport> {
            Box::pin(async {
                unreachable!("shutdown_all_threads_bounded should not be called in this test")
            })
        }

        fn shutdown_live_thread<'a>(
            &'a self,
            _thread_id: ThreadId,
        ) -> ThreadServiceFuture<'a, protocol::error::Result<String>> {
            Box::pin(async {
                unreachable!("shutdown_live_thread should not be called in this test")
            })
        }

        fn remove_live_thread<'a>(
            &'a self,
            _thread_id: ThreadId,
        ) -> ThreadServiceFuture<'a, bool> {
            Box::pin(async {
                unreachable!("remove_live_thread should not be called in this test")
            })
        }

        fn subscribe_thread_created(
            &self,
        ) -> tokio::sync::broadcast::Receiver<thread_service_api::ThreadCreatedEvent> {
            unreachable!("subscribe_thread_created should not be called in this test")
        }

        fn live_thread_agent_status<'a>(
            &'a self,
            _thread_id: ThreadId,
        ) -> ThreadServiceFuture<'a, protocol::error::Result<protocol::protocol::AgentStatus>> {
            Box::pin(async {
                unreachable!("live_thread_agent_status should not be called in this test")
            })
        }

        fn subscribe_live_thread_status<'a>(
            &'a self,
            _thread_id: ThreadId,
        ) -> ThreadServiceFuture<
            'a,
            protocol::error::Result<
                tokio::sync::watch::Receiver<protocol::protocol::AgentStatus>,
            >,
        > {
            Box::pin(async {
                unreachable!("subscribe_live_thread_status should not be called in this test")
            })
        }

        fn active_event_subscriptions(
            &self,
        ) -> Arc<thread_service_api::ActiveEventSubscriptionTracker> {
            unreachable!("active_event_subscriptions should not be called in this test")
        }
    }

    impl NativeAgentRuntime for StubThreadServiceApi {
        fn spawn_agent<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _request: ThreadSpawnAgentRequest,
        ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
            Box::pin(async {
                Err(FunctionCallError::RespondToModel(
                    "agent depth limit reached: cannot spawn depth 2; configured agents.max_depth is 1"
                        .to_string(),
                ))
            })
        }

        fn followup_task<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _target: String,
            _message: String,
        ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
            Box::pin(async { unreachable!("followup_task should not be called in this test") })
        }

        fn close_agent<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _target: String,
        ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
            Box::pin(async { unreachable!("close_agent should not be called in this test") })
        }

        fn list_agents<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _path_prefix: Option<String>,
        ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
            Box::pin(async { unreachable!("list_agents should not be called in this test") })
        }
    }

    impl ThreadCollaborationRuntime for StubThreadServiceApi {
        fn spawn_external_agent<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _request: ThreadSpawnExternalAgentRequest,
        ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
            Box::pin(async {
                unreachable!("spawn_external_agent should not be called in this test")
            })
        }

        fn followup_external_task<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _target: String,
            _message: String,
        ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
            Box::pin(async {
                unreachable!("followup_external_task should not be called in this test")
            })
        }

        fn close_external_agent<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _target: String,
        ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
            Box::pin(async {
                unreachable!("close_external_agent should not be called in this test")
            })
        }

        fn list_external_agents<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _call_id: String,
            _path_prefix: Option<String>,
        ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
            Box::pin(async {
                unreachable!("list_external_agents should not be called in this test")
            })
        }
    }

    impl ThreadEventRuntime for StubThreadServiceApi {
        fn poll_event<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _request: ThreadPollEventRequest,
        ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>> {
            Box::pin(async { unreachable!("poll_event should not be called in this test") })
        }

        fn poll_event_timeout_metadata<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _request: ThreadPollEventRequest,
        ) -> ThreadServiceFuture<'a, Result<ThreadPollEventTimeoutMetadata, FunctionCallError>>
        {
            Box::pin(async {
                unreachable!("poll_event_timeout_metadata should not be called in this test")
            })
        }

        fn reset_thread_wait_backoff<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
        ) -> ThreadServiceFuture<'a, ()> {
            Box::pin(async {
                unreachable!("reset_thread_wait_backoff should not be called in this test")
            })
        }

        fn record_model_items_and_emit_display_events<'a>(
            &'a self,
            _turn: Arc<dyn thread_service_api::ThreadTurnCapability>,
            _items: Vec<protocol::models::ResponseItem>,
        ) -> ThreadServiceFuture<'a, Result<(), String>> {
            Box::pin(async {
                unreachable!(
                    "record_model_items_and_emit_display_events should not be called in this test"
                )
            })
        }
    }

    #[tokio::test]
    async fn spawn_agent_tool_rejects_depth_limit_at_call_time() {
        let (session, turn_context) = test_support::make_session_and_context_with(
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: ThreadId::new(),
                depth: 1,
                agent_path: Some(AgentPath::root().join("worker").expect("agent path")),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
            |config| {
                config.agent_max_depth = 1;
            },
        )
        .await;

        let response = dispatch(
            Arc::clone(&session) as Arc<dyn ThreadSessionCapability>,
            Arc::clone(&session) as Arc<dyn SessionAgentJobCaller>,
            Arc::new(StubThreadServiceApi),
            Arc::clone(&turn_context) as Arc<dyn ThreadRuntimeCapability>,
            ToolCall {
                call_id: "spawn-depth-limit".to_string(),
                tool_name: ToolName::plain("spawn_agent"),
                payload: ToolPayload::Function {
                    arguments: serde_json::json!({
                        "task_name": "blocked_child",
                        "message": "try to spawn",
                    })
                    .to_string(),
                },
            },
        )
        .await;

        let Err(FunctionCallError::RespondToModel(output)) = response else {
            panic!("expected depth limit error");
        };
        assert_eq!(
            output,
            "agent depth limit reached: cannot spawn depth 2; configured agents.max_depth is 1"
        );
    }
}
