use std::sync::Arc;

use crate::agent::multi_agent;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::thread::ThreadService;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::SpawnAgentForkMode;
use protocol::models::ResponseItem;
use thread_service_api::ThreadAgentMode;
use thread_service_api::ThreadCloseAgentResult;
use thread_service_api::ThreadListAgentsResult;
use thread_service_api::ThreadListedAgent;
use thread_service_api::ThreadServiceApi;
use thread_service_api::ThreadServiceFuture;
use thread_service_api::ThreadSpawnAgentForkMode;
use thread_service_api::ThreadSpawnAgentRequest;
use thread_service_api::ThreadSpawnAgentResult;
use thread_service_api::ThreadTurnCapability;
use thread_service_api::ThreadWaitAgentReason;
use thread_service_api::ThreadWaitAgentResult;
use tool_service_api::FunctionCallError;

fn turn_context(
    turn: Arc<dyn ThreadTurnCapability>,
) -> Result<Arc<TurnContext>, FunctionCallError> {
    turn.into_any_arc().downcast::<TurnContext>().map_err(|_| {
        FunctionCallError::Fatal("thread turn capability must be TurnContext".to_string())
    })
}

fn session(turn: &TurnContext) -> Arc<Session> {
    turn.session_arc()
}

fn to_runtime_spawn_request(
    request: ThreadSpawnAgentRequest,
) -> codex_agent_runtime::SpawnAgentToolRequest {
    codex_agent_runtime::SpawnAgentToolRequest {
        message: request.message,
        task_name: request.task_name,
        agent_type: request.agent_type,
        cwd: request.cwd,
        model: request.model,
        reasoning_effort: request.reasoning_effort,
        service_tier: request.service_tier,
        agent_mode: request.agent_mode.map(|mode| match mode {
            ThreadAgentMode::Normal => AgentMode::Normal,
            ThreadAgentMode::Management => AgentMode::Management,
        }),
        fork_mode: request.fork_mode.map(|mode| match mode {
            ThreadSpawnAgentForkMode::FullHistory => SpawnAgentForkMode::FullHistory,
            ThreadSpawnAgentForkMode::LastNTurns { last_n_turns } => {
                SpawnAgentForkMode::LastNTurns(last_n_turns)
            }
        }),
    }
}

fn from_runtime_spawn_result(
    result: codex_agent_runtime::SpawnAgentToolResult,
) -> ThreadSpawnAgentResult {
    match result {
        codex_agent_runtime::SpawnAgentToolResult::WithNickname {
            task_name,
            nickname,
        } => ThreadSpawnAgentResult::WithNickname {
            task_name,
            nickname,
        },
        codex_agent_runtime::SpawnAgentToolResult::HiddenMetadata { task_name } => {
            ThreadSpawnAgentResult::HiddenMetadata { task_name }
        }
    }
}

fn from_runtime_wait_result(
    result: codex_agent_runtime::WaitAgentToolResult,
) -> ThreadWaitAgentResult {
    ThreadWaitAgentResult {
        target: result.target,
        agent_name: result.agent_name,
        reason: match result.reason {
            codex_agent_runtime::WaitAgentReason::PendingMessage => {
                ThreadWaitAgentReason::PendingMessage
            }
            codex_agent_runtime::WaitAgentReason::MailboxMessage => {
                ThreadWaitAgentReason::MailboxMessage
            }
            codex_agent_runtime::WaitAgentReason::FinalStatus => ThreadWaitAgentReason::FinalStatus,
            codex_agent_runtime::WaitAgentReason::StatusUpdate => {
                ThreadWaitAgentReason::StatusUpdate
            }
            codex_agent_runtime::WaitAgentReason::Timeout => ThreadWaitAgentReason::Timeout,
        },
        timed_out: result.timed_out,
        status: result.status,
        message_operation: result.message_operation,
        message_author: result.message_author,
        message_excerpt: result.message_excerpt,
        waited_ms: result.waited_ms,
        initial_timeout_ms: result.initial_timeout_ms,
        current_timeout_ms: result.current_timeout_ms,
        hard_cap_timeout_ms: result.hard_cap_timeout_ms,
    }
}

fn from_runtime_close_result(
    result: codex_agent_runtime::CloseAgentToolResult,
) -> ThreadCloseAgentResult {
    ThreadCloseAgentResult {
        previous_status: result.previous_status,
    }
}

fn from_runtime_list_result(
    result: codex_agent_runtime::ListAgentsToolResult,
) -> ThreadListAgentsResult {
    ThreadListAgentsResult {
        agents: result
            .agents
            .into_iter()
            .map(|agent| ThreadListedAgent {
                agent_name: agent.agent_name,
                agent_status: agent.agent_status,
                last_task_message: agent.last_task_message,
            })
            .collect(),
    }
}

impl ThreadServiceApi for ThreadService {
    fn spawn_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::spawn_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                to_runtime_spawn_request(request),
            )
            .await
            .map(from_runtime_spawn_result)
        })
    }

    fn followup_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        message: String,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::followup_task_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
                message,
            )
            .await
        })
    }

    fn wait_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadWaitAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::wait_agent_tool(session(turn.as_ref()), Arc::clone(&turn), call_id, target)
                .await
                .map(from_runtime_wait_result)
        })
    }

    fn close_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::close_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
            )
            .await
            .map(from_runtime_close_result)
        })
    }

    fn list_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::list_agents_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                path_prefix,
            )
            .await
            .map(from_runtime_list_result)
        })
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        items: Vec<ResponseItem>,
    ) -> ThreadServiceFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let turn = turn_context(turn).map_err(|err| err.to_string())?;
            session(turn.as_ref())
                .record_model_items_and_emit_display_events(turn.as_ref(), items.as_slice())
                .await;
            Ok(())
        })
    }
}
