use std::sync::Arc;

use crate::agent::SpawnAgentOptions;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::tool_support::build_agent_spawn_config;
use crate::config::Config;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use codex_thread_api::SessionAgentJobCaller;
use codex_thread_api::ThreadRuntimeCapability;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_runtime_api::AgentJobRunnerOptions;
use codex_tool_runtime_api::AgentJobSpawnWorkerError;
use codex_tool_types::FunctionCallError;
use tokio::sync::watch;

impl SessionAgentJobCaller for Session {
    type SpawnConfig = Config;

    fn agent_job_state_db(&self) -> Option<SharedStateDbRuntime> {
        self.state_db()
            .map(|state_db| state_db as SharedStateDbRuntime)
    }

    fn agent_job_conversation_id_string(&self) -> String {
        self.thread_id().to_string()
    }

    async fn build_agent_job_runner_options(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        requested_concurrency: Option<usize>,
    ) -> Result<AgentJobRunnerOptions<Config>, FunctionCallError> {
        let turn = turn_context_from_capability(turn);
        let child_depth = turn.next_child_spawn_depth();
        let max_depth = turn.agent_max_depth();
        if exceeds_thread_spawn_depth_limit(child_depth, max_depth) {
            return Err(FunctionCallError::RespondToModel(
                "agent depth limit reached; this session cannot spawn more subagents".to_string(),
            ));
        }
        let agent_max_threads = turn.agent_max_threads();
        if agent_max_threads == Some(0) {
            return Err(FunctionCallError::RespondToModel(
                "agent thread limit reached; this session cannot spawn more subagents".to_string(),
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
            spawn_config,
        })
    }

    async fn spawn_agent_job_worker(
        self: Arc<Self>,
        turn: &dyn ThreadRuntimeCapability,
        spawn_config: Self::SpawnConfig,
        job_id: &str,
        prompt: String,
    ) -> Result<ThreadId, AgentJobSpawnWorkerError> {
        let turn = turn_context_from_capability(turn);
        let items = vec![UserInput::Text {
            text: prompt,
            text_elements: Vec::new(),
        }];
        self.spawn_agent_with_metadata(
            spawn_config,
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
    }

    async fn shutdown_agent_job_worker(self: Arc<Self>, thread_id: ThreadId) {
        Session::shutdown_agent_job_worker(self.as_ref(), thread_id).await;
    }

    async fn get_agent_job_worker_status(self: Arc<Self>, thread_id: ThreadId) -> AgentStatus {
        Session::agent_status(self.as_ref(), thread_id).await
    }

    async fn subscribe_agent_job_worker_status(
        self: Arc<Self>,
        thread_id: ThreadId,
    ) -> Option<watch::Receiver<AgentStatus>> {
        Session::subscribe_agent_status(self.as_ref(), thread_id)
            .await
            .ok()
    }
}

fn turn_context_from_capability(capability: &dyn ThreadRuntimeCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("agent job turn capability must be backed by TurnContext")
}
