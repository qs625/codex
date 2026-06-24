use std::sync::Arc;

use crate::agent::SpawnAgentOptions;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::next_thread_spawn_depth;
use crate::agent::tool_support::build_agent_spawn_config;
use crate::config::Config;
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::CoreToolDomainHost;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_protocol::user_input::UserInput;
use codex_state_api::SharedStateDbRuntime;
use codex_tool_runtime_api::AgentJobRunnerOptions;
use codex_tool_runtime_api::AgentJobSpawnWorkerError;
use codex_tool_runtime_api::AgentJobToolHost;
use codex_utils_absolute_path::AbsolutePathBuf;
use tokio::sync::watch;

impl AgentJobToolHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;
    type SpawnConfig = Config;

    fn state_db(&self, session: &Self::Session) -> Option<SharedStateDbRuntime> {
        session
            .state_db()
            .map(|state_db| state_db as SharedStateDbRuntime)
    }

    fn conversation_id_string(&self, session: &Self::Session) -> String {
        session.conversation_id.to_string()
    }

    fn single_local_environment_cwd(
        &self,
        turn: &Self::Turn,
    ) -> Result<AbsolutePathBuf, FunctionCallError> {
        let [turn_environment] = turn.environments.turn_environments.as_slice() else {
            return Err(FunctionCallError::RespondToModel(
                "spawn_agents_on_csv requires exactly one local environment".to_string(),
            ));
        };

        if turn_environment.environment.is_remote() {
            return Err(FunctionCallError::RespondToModel(
                "spawn_agents_on_csv is not supported for remote environments".to_string(),
            ));
        }

        Ok(turn_environment.cwd.clone())
    }

    fn default_agent_job_max_runtime_seconds(&self, turn: &Self::Turn) -> Option<u64> {
        turn.config.agent_job_max_runtime_seconds
    }

    async fn build_agent_job_runner_options(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        requested_concurrency: Option<usize>,
    ) -> Result<AgentJobRunnerOptions<Self::SpawnConfig>, FunctionCallError> {
        let session_source = turn.session_source.clone();
        let child_depth = next_thread_spawn_depth(&session_source);
        let max_depth = turn.config.agent_max_depth;
        if exceeds_thread_spawn_depth_limit(child_depth, max_depth) {
            return Err(FunctionCallError::RespondToModel(
                "agent depth limit reached; this session cannot spawn more subagents".to_string(),
            ));
        }
        if turn.config.agent_max_threads == Some(0) {
            return Err(FunctionCallError::RespondToModel(
                "agent thread limit reached; this session cannot spawn more subagents".to_string(),
            ));
        }
        let max_concurrency = codex_agent_runtime::bounded_agent_job_concurrency(
            requested_concurrency,
            turn.config.agent_max_threads,
        );
        let base_instructions = session.get_base_instructions().await;
        let spawn_config =
            build_agent_spawn_config(&base_instructions, turn.as_ref(), /*cwd*/ None)?;
        Ok(AgentJobRunnerOptions {
            max_concurrency,
            spawn_config,
        })
    }

    async fn spawn_agent_job_worker(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        spawn_config: Self::SpawnConfig,
        job_id: &str,
        prompt: String,
    ) -> Result<ThreadId, AgentJobSpawnWorkerError> {
        let items = vec![UserInput::Text {
            text: prompt,
            text_elements: Vec::new(),
        }];
        session
            .services
            .agent_control
            .spawn_agent_with_metadata(
                spawn_config,
                items.into(),
                Some(SessionSource::SubAgent(SubAgentSource::Other(format!(
                    "agent_job:{job_id}"
                )))),
                SpawnAgentOptions {
                    environments: Some(turn.environments.to_selections()),
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

    async fn shutdown_agent_job_worker(&self, session: &Self::Session, thread_id: ThreadId) {
        let _ = session
            .services
            .agent_control
            .shutdown_live_agent(thread_id)
            .await;
    }

    async fn get_agent_job_worker_status(
        &self,
        session: &Self::Session,
        thread_id: ThreadId,
    ) -> AgentStatus {
        session.services.agent_control.get_status(thread_id).await
    }

    async fn subscribe_agent_job_worker_status(
        &self,
        session: &Self::Session,
        thread_id: ThreadId,
    ) -> Option<watch::Receiver<AgentStatus>> {
        session
            .services
            .agent_control
            .subscribe_status(thread_id)
            .await
            .ok()
    }
}
