use std::sync::Arc;

use crate::agent::SpawnAgentOptions;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::spawn_support::build_agent_spawn_config;
use crate::config::Config;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::protocol::AgentStatus;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::user_input::UserInput;
use state_api::SharedStateDbRuntime;
use thread_service_api::AgentJobRunnerOptions;
use thread_service_api::AgentJobSpawnWorkerError;
use thread_service_api::SessionAgentJobCaller;
use thread_service_api::ThreadCapability;
use thread_service_api::ThreadRuntimeCapability;
use tokio::sync::watch;
use tool_service_api::FunctionCallError;

impl SessionAgentJobCaller for Session {
    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime> {
        self.state_db()
            .map(|state_db| state_db as SharedStateDbRuntime)
    }

    fn agent_job_conversation_id_string(&self) -> String {
        self.thread_id().to_string()
    }

    fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        requested_concurrency: Option<usize>,
    ) -> thread_service_api::SessionCapabilityFuture<
        '_,
        Result<AgentJobRunnerOptions<thread_service_api::AgentJobSpawnConfig>, FunctionCallError>,
    > {
        Box::pin(async move {
            let turn = turn_context_from_capability(turn);
            let child_depth = turn.next_child_spawn_depth();
            let max_depth = turn.agent_max_depth();
            if exceeds_thread_spawn_depth_limit(child_depth, max_depth) {
                return Err(FunctionCallError::RespondToModel(
                    "agent depth limit reached; this session cannot spawn more subagents"
                        .to_string(),
                ));
            }
            let agent_max_threads = turn.agent_max_threads();
            if agent_max_threads == Some(0) {
                return Err(FunctionCallError::RespondToModel(
                    "agent thread limit reached; this session cannot spawn more subagents"
                        .to_string(),
                ));
            }
            let max_concurrency = codex_agent_runtime::bounded_agent_job_concurrency(
                requested_concurrency,
                agent_max_threads,
            );
            let base_instructions = self.get_base_instructions().await;
            let spawn_config =
                build_agent_spawn_config(&base_instructions, turn, /*cwd*/ None)?;
            Ok(AgentJobRunnerOptions {
                max_concurrency,
                spawn_config: Arc::new(spawn_config) as thread_service_api::AgentJobSpawnConfig,
            })
        })
    }

    fn spawn_agent_job_worker<'a>(
        self: Arc<Self>,
        turn: &'a dyn ThreadRuntimeCapability,
        spawn_config: thread_service_api::AgentJobSpawnConfig,
        job_id: &'a str,
        prompt: String,
    ) -> thread_service_api::SessionCapabilityFuture<'a, Result<ThreadId, AgentJobSpawnWorkerError>>
    {
        Box::pin(async move {
            let turn = turn_context_from_capability(turn);
            let spawn_config = Arc::downcast::<Config>(spawn_config).map_err(|_| {
                AgentJobSpawnWorkerError::Other(
                    "agent job spawn config had unexpected concrete type".to_string(),
                )
            })?;
            let items = vec![UserInput::Text {
                text: prompt,
                text_elements: Vec::new(),
            }];
            self.spawn_agent_with_metadata(
                (*spawn_config).clone(),
                items.into(),
                Some(SessionSource::SubAgent(SubAgentSource::Other(format!(
                    "agent_job:{job_id}"
                )))),
                SpawnAgentOptions {
                    environments: Some(turn.turn_environment_selections()),
                    ..Default::default()
                },
            )
            .await
            .map(|spawned_agent| spawned_agent.thread_id)
            .map_err(|err| match err {
                CodexErr::AgentLimitReached { .. } => AgentJobSpawnWorkerError::LimitReached,
                err => AgentJobSpawnWorkerError::Other(err.to_string()),
            })
        })
    }

    fn shutdown_agent_job_worker(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> thread_service_api::SessionCapabilityFuture<'static, ()> {
        Box::pin(async move {
            Session::shutdown_agent_job_worker(self.as_ref(), thread_id).await;
        })
    }

    fn get_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> thread_service_api::SessionCapabilityFuture<'static, AgentStatus> {
        Box::pin(async move { Session::agent_status(self.as_ref(), thread_id).await })
    }

    fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> thread_service_api::SessionCapabilityFuture<'static, Option<watch::Receiver<AgentStatus>>>
    {
        Box::pin(async move {
            Session::subscribe_agent_status(self.as_ref(), thread_id)
                .await
                .ok()
        })
    }
}

fn turn_context_from_capability(capability: &dyn ThreadRuntimeCapability) -> &TurnContext {
    ThreadCapability::as_any(capability)
        .downcast_ref::<TurnContext>()
        .unwrap_or_else(|| panic!("agent job turn capability must be backed by TurnContext"))
}
