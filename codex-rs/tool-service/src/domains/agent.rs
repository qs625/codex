mod agent_jobs;

use std::sync::Arc;

use crate::planning::SpawnAgentToolOptions;
use crate::planning::ToolSpec;
use crate::planning::create_close_agent_tool_v2;
use crate::planning::create_followup_task_tool;
use crate::planning::create_list_agents_tool;
use crate::planning::create_report_agent_job_result_tool;
use crate::planning::create_spawn_agent_tool_v2;
use crate::planning::create_spawn_agents_on_csv_tool;
use crate::planning::create_wait_agent_tool_v2;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_protocol::openai_models::ReasoningEffort;
use codex_tool_service_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
use serde::Serialize;
use thread_service_api::SessionAgentJobCaller;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadServiceApi;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

const SPAWN_AGENT_TOOL_NAME: &str = "spawn_agent";
const FOLLOWUP_TASK_TOOL_NAME: &str = "followup_task";
const WAIT_AGENT_TOOL_NAME: &str = "wait_agent";
const LIST_AGENTS_TOOL_NAME: &str = "list_agents";
const CLOSE_AGENT_TOOL_NAME: &str = "close_agent";
const SPAWN_AGENTS_ON_CSV_TOOL_NAME: &str = "spawn_agents_on_csv";
const REPORT_AGENT_JOB_RESULT_TOOL_NAME: &str = "report_agent_job_result";

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
        create_wait_agent_tool_v2(),
        create_list_agents_tool(),
        create_close_agent_tool_v2(),
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
                | WAIT_AGENT_TOOL_NAME
                | LIST_AGENTS_TOOL_NAME
                | CLOSE_AGENT_TOOL_NAME
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
        WAIT_AGENT_TOOL_NAME => {
            let arguments = function_arguments(&call)?;
            let target = wait_agent_target_from_arguments(&arguments)?;
            let result = thread_service_api
                .wait_agent(
                    Arc::clone(&turn) as Arc<dyn thread_service_api::ThreadTurnCapability>,
                    call.call_id.clone(),
                    target,
                )
                .await?;
            function_tool_json_output(&result, WAIT_AGENT_TOOL_NAME)?
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

fn from_runtime_spawn_request(
    request: SpawnAgentToolRequest,
) -> thread_service_api::ThreadSpawnAgentRequest {
    thread_service_api::ThreadSpawnAgentRequest {
        message: request.message,
        task_name: request.task_name,
        agent_type: request.agent_type,
        cwd: request.cwd,
        model: request.model,
        reasoning_effort: request.reasoning_effort,
        service_tier: request.service_tier,
        agent_mode: request.agent_mode.map(|mode| match mode {
            AgentMode::Normal => thread_service_api::ThreadAgentMode::Normal,
            AgentMode::Management => thread_service_api::ThreadAgentMode::Management,
        }),
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

    use codex_protocol::AgentPath;
    use codex_protocol::ThreadId;
    use codex_protocol::protocol::SessionSource;
    use codex_protocol::protocol::SubAgentSource;
    use thread_service::test_support;

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
            session,
            turn_context,
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
