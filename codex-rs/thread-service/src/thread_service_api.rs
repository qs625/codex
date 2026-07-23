use std::sync::Arc;
use std::time::Duration;

use crate::agent::multi_agent;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::thread::ThreadService;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::SpawnAgentProvider;
use protocol::models::ResponseItem;
use thread_service_api::NativeAgentRuntime;
use thread_service_api::ThreadCloseAgentResult;
use thread_service_api::ThreadCollaborationRuntime;
use thread_service_api::ThreadCreatedEvent;
use thread_service_api::ThreadEventRuntime;
use thread_service_api::ThreadLifecycleRuntime;
use thread_service_api::ThreadListAgentsResult;
use thread_service_api::ThreadListedAgent;
use thread_service_api::ThreadPollEventRequest;
use thread_service_api::ThreadPollEventResult;
use thread_service_api::ThreadPollEventTimeoutMetadata;
use thread_service_api::ThreadServiceFuture;
use thread_service_api::ThreadShutdownReport;
use thread_service_api::ThreadSpawnAgentForkMode;
use thread_service_api::ThreadSpawnAgentRequest;
use thread_service_api::ThreadSpawnAgentResult;
use thread_service_api::ThreadSpawnExternalAgentRequest;
use thread_service_api::ThreadTurnCapability;
use tokio::sync::broadcast;
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
        provider: request.provider.map(|provider| match provider {
            thread_service_api::ThreadSpawnAgentProvider::Native => SpawnAgentProvider::Native,
            thread_service_api::ThreadSpawnAgentProvider::CodexCli => SpawnAgentProvider::CodexCli,
            thread_service_api::ThreadSpawnAgentProvider::ClaudeCli => {
                SpawnAgentProvider::ClaudeCli
            }
            thread_service_api::ThreadSpawnAgentProvider::Opencode => SpawnAgentProvider::Opencode,
        }),
        agent_type: request.agent_type,
        cwd: request.cwd,
        model: request.model,
        reasoning_effort: request.reasoning_effort,
        service_tier: request.service_tier,
        fork_mode: request.fork_mode.map(|mode| match mode {
            ThreadSpawnAgentForkMode::FullHistory => SpawnAgentForkMode::FullHistory,
            ThreadSpawnAgentForkMode::LastNTurns { last_n_turns } => {
                SpawnAgentForkMode::LastNTurns(last_n_turns)
            }
        }),
    }
}

fn to_runtime_spawn_external_request(
    request: ThreadSpawnExternalAgentRequest,
) -> codex_agent_runtime::SpawnExternalAgentToolRequest {
    codex_agent_runtime::SpawnExternalAgentToolRequest {
        message: request.message,
        task_name: request.task_name,
        provider: match request.provider {
            thread_service_api::ThreadSpawnAgentProvider::Native => SpawnAgentProvider::Native,
            thread_service_api::ThreadSpawnAgentProvider::CodexCli => SpawnAgentProvider::CodexCli,
            thread_service_api::ThreadSpawnAgentProvider::ClaudeCli => {
                SpawnAgentProvider::ClaudeCli
            }
            thread_service_api::ThreadSpawnAgentProvider::Opencode => SpawnAgentProvider::Opencode,
        },
        cwd: request.cwd,
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
                agent_nickname: agent.agent_nickname,
                agent_role: agent.agent_role,
                lifecycle_status: agent.lifecycle_status,
                last_task_message: agent.last_task_message,
            })
            .collect(),
    }
}

impl ThreadLifecycleRuntime for ThreadService {
    fn shutdown_all_threads_bounded<'a>(
        &'a self,
        timeout: Duration,
    ) -> ThreadServiceFuture<'a, ThreadShutdownReport> {
        Box::pin(ThreadService::shutdown_all_threads_bounded(self, timeout))
    }

    fn shutdown_live_thread<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, protocol::error::Result<String>> {
        Box::pin(thread_service_api::LiveThreadShutdownRuntime::shutdown_live_thread(
            self, thread_id,
        ))
    }

    fn remove_live_thread<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, bool> {
        Box::pin(thread_service_api::LiveThreadCommandRuntime::remove_live_thread(
            self, thread_id,
        ))
    }

    fn subscribe_thread_created(&self) -> broadcast::Receiver<ThreadCreatedEvent> {
        ThreadService::subscribe_thread_created(self)
    }

    fn live_thread_agent_status<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<'a, protocol::error::Result<protocol::protocol::AgentStatus>> {
        Box::pin(thread_service_api::LiveThreadStatusRuntime::live_thread_agent_status(
            self, thread_id,
        ))
    }

    fn subscribe_live_thread_status<'a>(
        &'a self,
        thread_id: protocol::ThreadId,
    ) -> ThreadServiceFuture<
        'a,
        protocol::error::Result<tokio::sync::watch::Receiver<protocol::protocol::AgentStatus>>,
    > {
        Box::pin(
            thread_service_api::LiveThreadStatusRuntime::subscribe_live_thread_status(
                self, thread_id,
            ),
        )
    }

    fn active_event_subscriptions(
        &self,
    ) -> Arc<thread_service_api::ActiveEventSubscriptionTracker> {
        ThreadService::active_event_subscriptions(self)
    }
}

impl NativeAgentRuntime for ThreadService {
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
}

impl ThreadCollaborationRuntime for ThreadService {
    fn spawn_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        request: ThreadSpawnExternalAgentRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadSpawnAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::spawn_external_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                to_runtime_spawn_external_request(request),
            )
            .await
            .map(from_runtime_spawn_result)
        })
    }

    fn followup_external_task<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
        message: String,
    ) -> ThreadServiceFuture<'a, Result<(), FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::followup_external_task_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
                message,
            )
            .await
        })
    }

    fn close_external_agent<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        target: String,
    ) -> ThreadServiceFuture<'a, Result<ThreadCloseAgentResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::close_external_agent_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                target,
            )
            .await
            .map(from_runtime_close_result)
        })
    }

    fn list_external_agents<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> ThreadServiceFuture<'a, Result<ThreadListAgentsResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            multi_agent::list_external_agents_tool(
                session(turn.as_ref()),
                Arc::clone(&turn),
                call_id,
                path_prefix,
            )
            .await
            .map(from_runtime_list_result)
        })
    }
}

impl ThreadEventRuntime for ThreadService {
    fn poll_event<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventResult, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            let (default_initial_timeout_ms, default_hard_cap_timeout_ms) =
                turn.default_wait_agent_timeouts();
            session(turn.as_ref())
                .poll_event(ThreadPollEventRequest {
                    initial_timeout_ms: Some(
                        request
                            .initial_timeout_ms
                            .unwrap_or(default_initial_timeout_ms),
                    ),
                    hard_cap_timeout_ms: Some(
                        request
                            .hard_cap_timeout_ms
                            .unwrap_or(default_hard_cap_timeout_ms),
                    ),
                })
                .await
        })
    }

    fn poll_event_timeout_metadata<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
        request: ThreadPollEventRequest,
    ) -> ThreadServiceFuture<'a, Result<ThreadPollEventTimeoutMetadata, FunctionCallError>> {
        Box::pin(async move {
            let turn = turn_context(turn)?;
            let (default_initial_timeout_ms, default_hard_cap_timeout_ms) =
                turn.default_wait_agent_timeouts();
            session(turn.as_ref())
                .poll_event_timeout_metadata(ThreadPollEventRequest {
                    initial_timeout_ms: Some(
                        request
                            .initial_timeout_ms
                            .unwrap_or(default_initial_timeout_ms),
                    ),
                    hard_cap_timeout_ms: Some(
                        request
                            .hard_cap_timeout_ms
                            .unwrap_or(default_hard_cap_timeout_ms),
                    ),
                })
                .await
        })
    }

    fn reset_thread_wait_backoff<'a>(
        &'a self,
        turn: Arc<dyn ThreadTurnCapability>,
    ) -> ThreadServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = match turn_context(turn) {
                Ok(turn) => turn,
                Err(_) => return,
            };
            session(turn.as_ref()).reset_thread_wait_backoff().await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_thread_service_api_split<T>()
    where
        T: ThreadLifecycleRuntime
            + NativeAgentRuntime
            + ThreadCollaborationRuntime
            + ThreadEventRuntime
            + thread_service_api::ThreadServiceApi,
    {
    }

    #[test]
    fn thread_service_implements_split_runtime_traits() {
        assert_thread_service_api_split::<ThreadService>();
    }
}
