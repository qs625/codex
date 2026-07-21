use crate::agent::AgentStatus;
use crate::agent::external::ExternalAgentRun;
use crate::agent::external::SharedExternalAgentRegistry;
use crate::agent::external::completion_communication;
use crate::agent::external::external_live_agent;
use crate::agent::external::external_metadata;
use crate::agent::external::run_external_cli;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::session::emit_subagent_session_started;
use crate::thread::NewThread;
use crate::thread::ResumeThreadWithHistoryOptions;
use crate::thread::ThreadConfigSnapshot;
use crate::thread::ThreadServiceState;
use chrono::DateTime;
use chrono::Utc;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_roles::resolve_role_config;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::AgentPathReservation;
use codex_agent_runtime::AgentRegistry;
use codex_agent_runtime::ListedAgent;
use codex_agent_runtime::LiveAgent;
use codex_agent_runtime::SpawnAgentOptions;
use codex_agent_runtime::SpawnAgentProvider;
use codex_agent_runtime::SpawnReservation;
use codex_agent_runtime::ThreadLifecycleInputs;
use codex_agent_runtime::ThreadSpawnChild;
use codex_agent_runtime::ThreadSpawnPlanInput;
use codex_agent_runtime::agent_status_from_event;
use codex_agent_runtime::agent_subtree_thread_ids;
use codex_agent_runtime::any_agent_thread_active;
use codex_agent_runtime::build_thread_spawn_children_by_parent;
use codex_agent_runtime::current_agent_path_for_session;
use codex_agent_runtime::direct_subagent_paths_from_children;
use codex_agent_runtime::is_final;
use codex_agent_runtime::list_agents_plan;
use codex_agent_runtime::normalized_thread_lifecycle_from_inputs;
use codex_agent_runtime::prepare_thread_spawn_plan;
use codex_agent_runtime::render_input_preview;
use codex_agent_runtime::resolve_agent_reference_path;
use codex_agent_runtime::root_listed_agent;
use codex_agent_runtime::select_forked_rollout_items;
use codex_agent_runtime::should_ignore_descendant_shutdown_error;
use codex_agent_runtime::should_release_agent_after_thread_request_error;
use codex_agent_runtime::thread_lifecycle_is_active;
#[cfg(any(test, feature = "test-support"))]
use codex_agent_runtime::thread_spawn_depth;
use codex_agent_runtime::thread_spawn_descendants;
use codex_agent_runtime::thread_spawn_parent_thread_id;
#[cfg(any(test, feature = "test-support"))]
use codex_features::Feature;
use protocol::AgentPath;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
#[cfg(test)]
use protocol::models::ResponseItem;
use protocol::protocol::InitialHistory;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::Op;
use protocol::protocol::ResumedHistory;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::ThreadSource;
use state_api::DirectionalThreadSpawnEdgeStatus;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Weak;
use thread_service_api::LiveThreadActivitySource;
use thread_service_api::LiveThreadCommandRuntime;
use thread_service_api::LiveThreadInspectionRuntime;
use thread_service_api::LiveThreadShutdownRuntime;
use thread_service_api::LiveThreadStateRuntimeSource;
use thread_service_api::LiveThreadStatusRuntime;
use thread_store_api::ReadThreadParams;
use tokio::sync::watch;
use tracing::warn;

/// Control-plane handle for multi-agent operations.
/// `AgentControl` is held by each session (via `SessionServices`). It provides capability to
/// spawn new agents and the inter-agent communication layer.
/// An `AgentControl` instance is intended to be created at most once per root thread/session
/// tree. That same `AgentControl` is then shared with every sub-agent spawned from that root,
/// which keeps the registry scoped to that root thread rather than the entire `ThreadService`.
#[derive(Clone, Default)]
pub(crate) struct AgentControl {
    /// ID shared by the whole agent control session. This means every sub-agents from a common
    /// root share the same session ID.
    session_id: SessionId,
    /// Weak handle back to the global thread registry/state.
    /// This is `Weak` to avoid reference cycles and shadow persistence of the form
    /// `ThreadServiceState -> CodexThread -> Session -> SessionServices -> ThreadServiceState`.
    manager: Weak<ThreadServiceState>,
    state: Arc<AgentRegistry>,
    external_agents: SharedExternalAgentRegistry,
}

struct PersistedAgentTarget {
    thread_id: ThreadId,
    parent_thread_id: ThreadId,
    depth: i32,
}

struct PersistedAgentPathCandidate {
    target: PersistedAgentTarget,
    updated_at: DateTime<Utc>,
    final_status: Option<AgentStatus>,
}

impl AgentControl {
    pub(crate) fn new_with_registry(
        manager: Weak<ThreadServiceState>,
        state: Arc<AgentRegistry>,
    ) -> Self {
        Self {
            manager,
            state,
            ..Default::default()
        }
    }

    pub(crate) fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = session_id;
        self
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// Spawn a new agent thread and submit the initial prompt.
    #[cfg(test)]
    pub(crate) async fn spawn_agent(
        &self,
        config: config_service::Config,
        initial_operation: Op,
        session_source: Option<SessionSource>,
    ) -> CodexResult<ThreadId> {
        let spawned_agent = Box::pin(self.spawn_agent_internal(
            config,
            initial_operation,
            session_source,
            SpawnAgentOptions::default(),
        ))
        .await?;
        Ok(spawned_agent.thread_id)
    }

    /// Spawn an agent thread with some metadata.
    pub(crate) async fn spawn_agent_with_metadata(
        &self,
        config: config_service::Config,
        initial_operation: Op,
        session_source: Option<SessionSource>,
        options: SpawnAgentOptions, // TODO(jif) drop with new fork.
    ) -> CodexResult<LiveAgent> {
        Box::pin(self.spawn_agent_internal(config, initial_operation, session_source, options))
            .await
    }

    pub(crate) async fn spawn_external_agent_with_metadata(
        &self,
        config: config_service::Config,
        provider: SpawnAgentProvider,
        message: String,
        session_source: SessionSource,
        options: SpawnAgentOptions,
    ) -> CodexResult<LiveAgent> {
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id,
            depth,
            agent_path,
            agent_role,
            ..
        }) = session_source
        else {
            return Err(CodexErr::UnsupportedOperation(
                "external agents must be spawned as thread-spawn subagents".to_string(),
            ));
        };

        let mut reservation = self.state.reserve_spawn_slot(config.agent_max_threads)?;
        let (_session_source, mut agent_metadata) = self.prepare_thread_spawn(
            &mut reservation,
            &config,
            parent_thread_id,
            depth,
            agent_path,
            agent_role,
            options.agent_mode,
            None,
        )?;
        let thread_id = ThreadId::new();
        let agent_path = agent_metadata.agent_path.clone().ok_or_else(|| {
            CodexErr::UnsupportedOperation("external agent is missing agent path".to_string())
        })?;
        agent_metadata.agent_id = Some(thread_id);
        agent_metadata.last_task_message = Some(message.clone());
        reservation.commit(agent_metadata);

        let run = ExternalAgentRun {
            thread_id,
            parent_thread_id,
            agent_path,
            provider,
            status: AgentStatus::Running,
            last_task_message: Some(message.clone()),
            abort_handle: None,
        };
        self.external_agents.insert_running(run.clone());

        let control = self.clone();
        let cwd = config.cwd.as_path().to_path_buf();
        let handle = tokio::spawn(async move {
            let status = run_external_cli(provider, cwd, message).await;
            control.complete_external_agent(thread_id, status).await;
        });
        self.external_agents
            .attach_abort_handle(thread_id, handle.abort_handle());

        Ok(external_live_agent(&run))
    }

    async fn spawn_agent_internal(
        &self,
        config: config_service::Config,
        initial_operation: Op,
        session_source: Option<SessionSource>,
        options: SpawnAgentOptions,
    ) -> CodexResult<LiveAgent> {
        let state = self.upgrade()?;
        let mut reservation = self.state.reserve_spawn_slot(config.agent_max_threads)?;
        let inherited_shell_snapshot = self
            .inherited_shell_snapshot_for_source(&state, session_source.as_ref())
            .await;
        let inherited_exec_policy = self
            .inherited_exec_policy_for_source(&state, session_source.as_ref(), &config)
            .await;
        let (session_source, mut agent_metadata) = match session_source {
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth,
                agent_path,
                agent_role,
                ..
            })) => {
                let (session_source, agent_metadata) = self.prepare_thread_spawn(
                    &mut reservation,
                    &config,
                    parent_thread_id,
                    depth,
                    agent_path,
                    agent_role,
                    options.agent_mode,
                    /*preferred_agent_nickname*/ None,
                )?;
                (Some(session_source), agent_metadata)
            }
            other => (other, AgentMetadata::default()),
        };
        let notification_source = session_source.clone();

        // The same `AgentControl` is sent to spawn the thread.
        let new_thread = match (session_source, options.fork_mode.as_ref()) {
            (Some(session_source), Some(_)) => {
                Box::pin(self.spawn_forked_thread(
                    &state,
                    config,
                    session_source,
                    &options,
                    inherited_shell_snapshot,
                    inherited_exec_policy,
                ))
                .await?
            }
            (Some(session_source), None) => {
                Box::pin(state.spawn_new_thread_with_source(
                    config.clone(),
                    self.clone(),
                    session_source,
                    /*thread_source*/ Some(ThreadSource::Subagent),
                    /*persist_extended_history*/ false,
                    /*metrics_service_name*/ None,
                    inherited_shell_snapshot,
                    inherited_exec_policy,
                    options.environments.clone(),
                ))
                .await?
            }
            (None, _) => Box::pin(state.spawn_new_thread(config.clone(), self.clone())).await?,
        };
        agent_metadata.agent_id = Some(new_thread.thread_id);
        reservation.commit(agent_metadata.clone());

        if let Some(SessionSource::SubAgent(
            subagent_source @ SubAgentSource::ThreadSpawn {
                parent_thread_id, ..
            },
        )) = notification_source.as_ref()
        {
            let client_metadata = match state.get_thread(*parent_thread_id).await {
                Ok(parent_thread) => {
                    parent_thread
                        .codex
                        .session
                        .app_server_client_metadata()
                        .await
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        parent_thread_id = %parent_thread_id,
                        "skipping subagent thread analytics: failed to load parent thread metadata"
                    );
                    crate::session::session::AppServerClientMetadata {
                        client_name: None,
                        client_version: None,
                    }
                }
            };
            let thread_config = new_thread.thread.codex.thread_config_snapshot().await;
            emit_subagent_session_started(
                &new_thread
                    .thread
                    .codex
                    .session
                    .services
                    .analytics_events_client,
                client_metadata,
                new_thread.thread_id,
                /*parent_thread_id*/ None,
                thread_config,
                subagent_source.clone(),
            );
        }

        // Notify a new thread has been created. This notification will be processed by clients
        // to subscribe or drain this newly created thread.
        // TODO(jif) add helper for drain
        state.notify_thread_started(new_thread.thread_id);

        self.persist_thread_spawn_edge_for_source(
            new_thread.thread_id,
            notification_source.as_ref(),
        )
        .await;

        if let Err(err) = self
            .send_input(new_thread.thread_id, initial_operation)
            .await
        {
            return Err(err);
        }
        Ok(LiveAgent {
            thread_id: new_thread.thread_id,
            metadata: agent_metadata,
            status: self.get_status(new_thread.thread_id).await,
        })
    }

    async fn spawn_forked_thread(
        &self,
        state: &Arc<ThreadServiceState>,
        config: config_service::Config,
        session_source: SessionSource,
        options: &SpawnAgentOptions,
        inherited_shell_snapshot: Option<Arc<ShellSnapshot>>,
        inherited_exec_policy: Option<Arc<permissions_service::ExecPolicyManager>>,
    ) -> CodexResult<NewThread> {
        if options.fork_parent_spawn_call_id.is_none() {
            return Err(CodexErr::Fatal(
                "spawn_agent fork requires a parent spawn call id".to_string(),
            ));
        }
        let Some(fork_mode) = options.fork_mode.as_ref() else {
            return Err(CodexErr::Fatal(
                "spawn_agent fork requires a fork mode".to_string(),
            ));
        };
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        }) = &session_source
        else {
            return Err(CodexErr::Fatal(
                "spawn_agent fork requires a thread-spawn session source".to_string(),
            ));
        };

        let parent_thread_id = *parent_thread_id;
        let parent_thread = state.get_thread(parent_thread_id).await.ok();
        if let Some(parent_thread) = parent_thread.as_ref() {
            // `record_conversation_items` only queues persistence writes asynchronously.
            // Flush before snapshotting store history for a fork.
            parent_thread.ensure_rollout_materialized().await;
            parent_thread.flush_rollout().await?;
        }

        let parent_history = state
            .read_stored_thread(ReadThreadParams {
                thread_id: parent_thread_id,
                include_archived: true,
                include_history: true,
            })
            .await?
            .history
            .ok_or_else(|| {
                CodexErr::Fatal(format!(
                    "parent thread history unavailable for fork: {parent_thread_id}"
                ))
            })?;

        // MultiAgentV2 root/subagent usage hints are injected as standalone developer
        // messages at thread start. When forking history, drop hints from the parent
        // so the child gets a fresh hint that matches its own session source/config.
        let multi_agent_v2_usage_hint_texts_to_filter: Vec<String> =
            if let Some(parent_thread) = parent_thread.as_ref() {
                parent_thread
                    .codex
                    .session
                    .configured_multi_agent_v2_usage_hint_texts()
                    .await
            } else {
                [
                    config.multi_agent_v2.root_agent_usage_hint_text.clone(),
                    config.multi_agent_v2.subagent_usage_hint_text.clone(),
                ]
                .into_iter()
                .flatten()
                .collect()
            };
        let forked_rollout_items = select_forked_rollout_items(
            parent_history.items,
            fork_mode,
            &multi_agent_v2_usage_hint_texts_to_filter,
        );

        state
            .fork_thread_with_source(
                config.clone(),
                InitialHistory::Forked(forked_rollout_items),
                self.clone(),
                session_source,
                /*thread_source*/ Some(ThreadSource::Subagent),
                /*persist_extended_history*/ false,
                inherited_shell_snapshot,
                inherited_exec_policy,
                options.environments.clone(),
            )
            .await
    }

    /// Resume an existing agent thread from a recorded rollout file.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn resume_agent_from_rollout(
        &self,
        config: config_service::Config,
        thread_id: ThreadId,
        session_source: SessionSource,
    ) -> CodexResult<ThreadId> {
        let root_depth = thread_spawn_depth(&session_source).unwrap_or(0);
        let resumed_thread_id = Box::pin(self.resume_single_agent_from_rollout(
            config.clone(),
            thread_id,
            session_source,
        ))
        .await?;
        let state = self.upgrade()?;
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return Ok(resumed_thread_id);
        };

        let tree_root_thread_id = self
            .persisted_thread_spawn_root(thread_id)
            .await
            .unwrap_or(thread_id);
        let mut resume_queue = VecDeque::from([(thread_id, root_depth)]);
        while let Some((parent_thread_id, parent_depth)) = resume_queue.pop_front() {
            let child_ids = match state_db_ctx
                .list_thread_spawn_children_with_status(
                    parent_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
            {
                Ok(child_ids) => child_ids,
                Err(err) => {
                    warn!(
                        "failed to load persisted thread-spawn children for {parent_thread_id}: {err}"
                    );
                    continue;
                }
            };

            for child_thread_id in child_ids {
                if !self
                    .persisted_child_is_auto_resumable_generation(
                        tree_root_thread_id,
                        child_thread_id,
                        state_db_ctx.as_ref(),
                    )
                    .await
                {
                    continue;
                }
                let child_depth = parent_depth + 1;
                let child_resumed = if state
                    .live_thread_config_snapshot(child_thread_id)
                    .await
                    .is_ok()
                {
                    true
                } else {
                    let child_session_source =
                        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                            parent_thread_id,
                            depth: child_depth,
                            agent_path: None,
                            agent_nickname: None,
                            agent_role: None,
                        });
                    match self
                        .resume_single_agent_from_rollout(
                            config.clone(),
                            child_thread_id,
                            child_session_source,
                        )
                        .await
                    {
                        Ok(_) => true,
                        Err(err) => {
                            warn!("failed to resume descendant thread {child_thread_id}: {err}");
                            false
                        }
                    }
                };
                if child_resumed {
                    resume_queue.push_back((child_thread_id, child_depth));
                }
            }
        }

        Ok(resumed_thread_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    async fn persisted_child_is_auto_resumable_generation(
        &self,
        tree_root_thread_id: ThreadId,
        child_thread_id: ThreadId,
        state_db_ctx: &dyn state_api::ThreadStateRuntime,
    ) -> bool {
        let Some(metadata) = state_db_ctx
            .get_thread(child_thread_id)
            .await
            .ok()
            .flatten()
        else {
            return false;
        };
        if metadata.archived_at.is_some() {
            return false;
        }
        let Some(agent_path) = metadata
            .agent_path
            .and_then(|path| AgentPath::from_string(path).ok())
        else {
            return true;
        };
        match self
            .persisted_agent_target_for_path(tree_root_thread_id, &agent_path)
            .await
        {
            Ok(Some(target)) => target.thread_id == child_thread_id,
            Ok(None) => false,
            Err(err) => {
                warn!(
                    "skipping persisted child {child_thread_id}: failed to resolve selected generation for path {}: {err}",
                    agent_path.as_str()
                );
                false
            }
        }
    }

    async fn resume_single_agent_from_rollout(
        &self,
        config: config_service::Config,
        thread_id: ThreadId,
        session_source: SessionSource,
    ) -> CodexResult<ThreadId> {
        let state = self.upgrade()?;
        let state_db_ctx = state.thread_state_runtime();
        let mut reservation = self.state.reserve_spawn_slot(config.agent_max_threads)?;
        let (session_source, agent_metadata) = match session_source {
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth,
                agent_path,
                agent_role: _,
                agent_nickname: _,
            }) => {
                let (resumed_agent_path, resumed_agent_nickname, resumed_agent_role) =
                    if let Some(state_db_ctx) = state_db_ctx.as_ref() {
                        match state_db_ctx.get_thread(thread_id).await {
                        Ok(Some(metadata)) => (
                            metadata
                                .agent_path
                                .map(AgentPath::from_string)
                                .transpose()
                                .map_err(|err| {
                                    CodexErr::Fatal(format!(
                                        "stored agent path for thread {thread_id} is invalid: {err}"
                                    ))
                                })?,
                            metadata.agent_nickname,
                            metadata.agent_role,
                        ),
                        Ok(None) | Err(_) => (None, None, None),
                    }
                    } else {
                        (None, None, None)
                    };
                self.prepare_thread_spawn(
                    &mut reservation,
                    &config,
                    parent_thread_id,
                    depth,
                    agent_path.or(resumed_agent_path),
                    resumed_agent_role,
                    AgentMode::Normal,
                    resumed_agent_nickname,
                )?
            }
            other => (other, AgentMetadata::default()),
        };
        let notification_source = session_source.clone();
        let inherited_shell_snapshot = self
            .inherited_shell_snapshot_for_source(&state, Some(&session_source))
            .await;
        let inherited_exec_policy = self
            .inherited_exec_policy_for_source(&state, Some(&session_source), &config)
            .await;
        let stored_thread = state
            .read_stored_thread(ReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: true,
            })
            .await?;
        let history = stored_thread
            .history
            .ok_or(CodexErr::ThreadNotFound(thread_id))?
            .items;

        let resumed_thread = state
            .resume_thread_with_history_with_source(ResumeThreadWithHistoryOptions {
                config: config.clone(),
                initial_history: InitialHistory::Resumed(ResumedHistory {
                    conversation_id: thread_id,
                    history,
                    rollout_path: stored_thread.rollout_path,
                }),
                agent_control: self.clone(),
                session_source,
                inherited_shell_snapshot,
                inherited_exec_policy,
            })
            .await?;
        let mut agent_metadata = agent_metadata;
        agent_metadata.agent_id = Some(resumed_thread.thread_id);
        reservation.commit(agent_metadata.clone());
        // Resumed threads are re-registered in-memory and need the same listener
        // attachment path as freshly spawned threads.
        state.notify_thread_resumed(resumed_thread.thread_id);
        self.persist_thread_spawn_edge_for_source(
            resumed_thread.thread_id,
            Some(&notification_source),
        )
        .await;

        Ok(resumed_thread.thread_id)
    }

    /// Send rich user input items to an existing agent thread.
    pub(crate) async fn send_input(
        &self,
        agent_id: ThreadId,
        initial_operation: Op,
    ) -> CodexResult<String> {
        let last_task_message = render_input_preview(&initial_operation);
        let state = self.upgrade()?;
        let result = self
            .handle_thread_request_result(
                agent_id,
                state.as_ref(),
                state
                    .submit_live_thread_op(agent_id, initial_operation)
                    .await,
            )
            .await;
        if result.is_ok() {
            self.state
                .update_last_task_message(agent_id, last_task_message);
        }
        result
    }

    /// Append a prebuilt message to an existing agent thread outside the normal user-input path.
    #[cfg(test)]
    pub(crate) async fn append_message(
        &self,
        agent_id: ThreadId,
        message: ResponseItem,
    ) -> CodexResult<String> {
        let state = self.upgrade()?;
        self.handle_thread_request_result(
            agent_id,
            state.as_ref(),
            state.append_message(agent_id, message).await,
        )
        .await
    }

    pub(crate) async fn send_inter_agent_communication(
        &self,
        agent_id: ThreadId,
        communication: InterAgentCommunication,
    ) -> CodexResult<String> {
        if self.external_agents.get(agent_id).is_some() {
            return Err(CodexErr::UnsupportedOperation(
                "followup_task is not supported for one-shot external CLI agents".to_string(),
            ));
        }
        let last_task_message = communication.content.clone();
        let state = self.upgrade()?;
        let op = Op::InterAgentCommunication { communication };
        let result = self
            .handle_thread_request_result(
                agent_id,
                state.as_ref(),
                state.submit_live_thread_op(agent_id, op).await,
            )
            .await;
        if result.is_ok() {
            self.state
                .update_last_task_message(agent_id, last_task_message);
        }
        result
    }

    /// Interrupt the current task for an existing agent thread.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) async fn interrupt_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        let state = self.upgrade()?;
        state.submit_live_thread_op(agent_id, Op::Interrupt).await
    }

    async fn handle_thread_request_result(
        &self,
        agent_id: ThreadId,
        runtime: &(impl LiveThreadCommandRuntime + ?Sized),
        result: CodexResult<String>,
    ) -> CodexResult<String> {
        if result
            .as_ref()
            .err()
            .is_some_and(should_release_agent_after_thread_request_error)
        {
            let _ = runtime.remove_live_thread(agent_id).await;
            self.state.release_spawned_thread(agent_id);
        }
        result
    }

    /// Submit a shutdown request for a live agent without marking it explicitly closed in
    /// persisted spawn-edge state.
    pub(crate) async fn shutdown_live_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        let state = self.upgrade()?;
        let result = state.shutdown_live_thread(agent_id).await;
        let _ = state.remove_live_thread(agent_id).await;
        self.state.release_spawned_thread(agent_id);
        result
    }

    /// Mark `agent_id` as explicitly closed in persisted spawn-edge state, then shut down the
    /// agent and any live descendants reached from the in-memory tree.
    pub(crate) async fn close_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        if self.external_agents.get(agent_id).is_some() {
            self.external_agents.shutdown(agent_id);
            self.state.release_spawned_thread(agent_id);
            return Ok(agent_id.to_string());
        }
        let state = self.upgrade()?;
        if let Some(state_db_ctx) = state.thread_state_runtime()
            && let Err(err) = state_db_ctx
                .set_thread_spawn_edge_status(agent_id, DirectionalThreadSpawnEdgeStatus::Closed)
                .await
        {
            warn!("failed to persist thread-spawn edge status for {agent_id}: {err}");
        }
        Box::pin(self.shutdown_agent_tree(agent_id)).await
    }

    /// Shut down `agent_id` and any live descendants reachable from the in-memory spawn tree.
    async fn shutdown_agent_tree(&self, agent_id: ThreadId) -> CodexResult<String> {
        let descendant_ids = self.live_thread_spawn_descendants(agent_id).await?;
        let result = self.shutdown_live_agent(agent_id).await;
        for descendant_id in descendant_ids {
            match self.shutdown_live_agent(descendant_id).await {
                Ok(_) => {}
                Err(err) if should_ignore_descendant_shutdown_error(&err) => {}
                Err(err) => return Err(err),
            }
        }
        result
    }

    /// Fetch the last known status for `agent_id`, returning `NotFound` when unavailable.
    pub(crate) async fn get_status(&self, agent_id: ThreadId) -> AgentStatus {
        if let Some(run) = self.external_agents.get(agent_id) {
            return run.status;
        }
        let Ok(state) = self.upgrade() else {
            // No agent available if upgrade fails.
            return AgentStatus::NotFound;
        };
        state
            .live_thread_agent_status(agent_id)
            .await
            .unwrap_or(AgentStatus::NotFound)
    }

    async fn complete_external_agent(&self, thread_id: ThreadId, status: AgentStatus) {
        let Some(run) = self
            .external_agents
            .set_terminal_status_if_active(thread_id, status)
        else {
            return;
        };
        let Some(communication) = completion_communication(&run) else {
            return;
        };
        let Ok(state) = self.upgrade() else {
            return;
        };
        let parent_thread_id = run.parent_thread_id;
        if let Err(err) = state
            .submit_live_thread_op(
                parent_thread_id,
                Op::InterAgentCommunication { communication },
            )
            .await
        {
            warn!(
                "failed to notify parent thread {parent_thread_id} of external agent completion: {err}"
            );
        }
    }

    /// Returns whether the live agent thread has `feature` enabled.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) async fn agent_thread_enabled(&self, agent_id: ThreadId, feature: Feature) -> bool {
        let Ok(state) = self.upgrade() else {
            return false;
        };
        state
            .live_thread_feature_enabled(agent_id, feature)
            .await
            .unwrap_or(false)
    }

    /// Returns whether `agent_id` is active according to the same runtime facts
    /// used by thread status: current turn, active event subscriptions, or
    /// non-final lifecycle status.
    pub(crate) async fn agent_thread_is_active(&self, agent_id: ThreadId) -> bool {
        let lifecycle = Box::pin(self.normalized_thread_lifecycle(agent_id)).await;
        thread_lifecycle_is_active(&lifecycle)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) async fn agent_subtree_is_active(&self, agent_id: ThreadId) -> bool {
        let Ok(thread_ids) = Box::pin(self.list_live_agent_subtree_thread_ids(agent_id)).await
        else {
            return false;
        };
        let mut active_flags = Vec::with_capacity(thread_ids.len());
        for thread_id in thread_ids {
            active_flags.push(Box::pin(self.agent_thread_is_active(thread_id)).await);
        }
        any_agent_thread_active(active_flags)
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) async fn agent_descendants_are_active(&self, agent_id: ThreadId) -> bool {
        let Ok(thread_ids) = Box::pin(self.live_thread_spawn_descendants(agent_id)).await else {
            return false;
        };
        let mut active_flags = Vec::with_capacity(thread_ids.len());
        for thread_id in thread_ids {
            active_flags.push(Box::pin(self.agent_thread_is_active(thread_id)).await);
        }
        any_agent_thread_active(active_flags)
    }

    pub(crate) async fn direct_agent_children_are_active(&self, agent_id: ThreadId) -> bool {
        if self.external_agents.direct_children_are_active(agent_id) {
            return true;
        }
        let Ok(children) = Box::pin(self.open_thread_spawn_children(agent_id)).await else {
            return false;
        };
        let mut active_flags = Vec::with_capacity(children.len());
        for (thread_id, _) in children {
            active_flags.push(Box::pin(self.agent_thread_is_active(thread_id)).await);
        }
        any_agent_thread_active(active_flags)
    }

    pub(crate) fn register_session_root(
        &self,
        _current_thread_id: ThreadId,
        _current_session_source: &SessionSource,
    ) {
        // Root-scope project threads may have canonical paths like `/project`.
        // Registering every root session as `/root` would create a second alias
        // for those threads. Legacy `/root` registration is handled when a root
        // thread actually spawns a child and has no canonical metadata.
    }

    pub(crate) fn register_root_scope_agent_metadata(&self, agent_metadata: AgentMetadata) {
        self.state.register_agent_metadata(agent_metadata);
    }

    pub(crate) fn reserve_root_scope_agent_path(
        &self,
        agent_path: &AgentPath,
    ) -> CodexResult<AgentPathReservation> {
        self.state.reserve_agent_path_registration(agent_path)
    }

    pub(crate) fn get_agent_metadata(&self, agent_id: ThreadId) -> Option<AgentMetadata> {
        self.state.agent_metadata_for_thread(agent_id)
    }

    pub(crate) async fn list_live_agent_subtree_thread_ids(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        Ok(agent_subtree_thread_ids(
            agent_id,
            self.live_thread_spawn_descendants(agent_id).await?,
        ))
    }

    pub(crate) async fn get_agent_config_snapshot(
        &self,
        agent_id: ThreadId,
    ) -> Option<ThreadConfigSnapshot> {
        let Ok(state) = self.upgrade() else {
            return None;
        };
        state.live_thread_config_snapshot(agent_id).await.ok()
    }

    pub(crate) fn current_agent_path(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
    ) -> AgentPath {
        current_agent_path_for_session(
            current_session_source,
            self.state
                .agent_metadata_for_thread(current_thread_id)
                .as_ref(),
        )
    }

    pub(crate) async fn resolve_agent_reference(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        config: Option<config_service::Config>,
        agent_reference: &str,
    ) -> CodexResult<ThreadId> {
        let current_agent_path = self.current_agent_path(current_thread_id, current_session_source);
        let agent_path = resolve_agent_reference_path(&current_agent_path, agent_reference)
            .map_err(CodexErr::UnsupportedOperation)?;
        if let Some(thread_id) = self.state.agent_id_for_path(&agent_path) {
            return Ok(thread_id);
        }
        if let Some(config) = config
            && let Some(thread_id) = self
                .resolve_persisted_agent_reference(
                    current_thread_id,
                    current_session_source,
                    config,
                    &agent_path,
                )
                .await?
        {
            return Ok(thread_id);
        }
        Err(CodexErr::UnsupportedOperation(format!(
            "agent path `{}` not found",
            agent_path.as_str()
        )))
    }

    pub(crate) async fn resolve_agent_thread_id(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        config: Option<config_service::Config>,
        target_thread_id: ThreadId,
    ) -> CodexResult<ThreadId> {
        let state = self.upgrade()?;
        if state
            .live_thread_config_snapshot(target_thread_id)
            .await
            .is_ok()
        {
            return Ok(target_thread_id);
        }

        if let Some(config) = config
            && let Some(target) = self
                .persisted_agent_target_for_thread_id(
                    current_thread_id,
                    current_session_source,
                    target_thread_id,
                )
                .await?
        {
            let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: target.parent_thread_id,
                depth: target.depth,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            });
            Box::pin(self.resume_single_agent_from_rollout(
                config,
                target.thread_id,
                session_source,
            ))
            .await?;
        }

        Ok(target_thread_id)
    }

    async fn resolve_persisted_agent_reference(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        config: config_service::Config,
        agent_path: &AgentPath,
    ) -> CodexResult<Option<ThreadId>> {
        let Some(root_thread_id) = self
            .root_thread_id_for_persisted_agent_lookup(current_thread_id, current_session_source)
            .await
        else {
            return Ok(None);
        };
        let Some(target) = self
            .persisted_agent_target_for_path(root_thread_id, agent_path)
            .await?
        else {
            return Ok(None);
        };

        if self.state.agent_id_for_path(agent_path).is_none() {
            let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: target.parent_thread_id,
                depth: target.depth,
                agent_path: Some(agent_path.clone()),
                agent_nickname: None,
                agent_role: None,
            });
            Box::pin(self.resume_single_agent_from_rollout(
                config,
                target.thread_id,
                session_source,
            ))
            .await?;
        }

        self.state
            .agent_id_for_path(agent_path)
            .map(Some)
            .ok_or_else(|| {
                CodexErr::UnsupportedOperation(format!(
                    "agent path `{}` could not be restored",
                    agent_path.as_str()
                ))
            })
    }

    async fn persisted_agent_target_for_thread_id(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        target_thread_id: ThreadId,
    ) -> CodexResult<Option<PersistedAgentTarget>> {
        let Some(root_thread_id) = self
            .root_thread_id_for_persisted_agent_lookup(current_thread_id, current_session_source)
            .await
        else {
            return Ok(None);
        };
        if target_thread_id == root_thread_id {
            return Ok(None);
        }
        self.persisted_agent_target_for_thread_id_from_root(root_thread_id, target_thread_id)
            .await
    }

    async fn persisted_agent_target_for_thread_id_from_root(
        &self,
        root_thread_id: ThreadId,
        target_thread_id: ThreadId,
    ) -> CodexResult<Option<PersistedAgentTarget>> {
        let state = self.upgrade()?;
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return Ok(None);
        };
        let mut queue = VecDeque::from([(root_thread_id, 0)]);
        let mut seen = HashSet::from([root_thread_id]);
        while let Some((parent_thread_id, parent_depth)) = queue.pop_front() {
            let child_ids = state_db_ctx
                .list_thread_spawn_children_with_status(
                    parent_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
                .map_err(|err| {
                    CodexErr::Fatal(format!(
                        "failed to load persisted thread-spawn children: {err}"
                    ))
                })?;
            for child_thread_id in child_ids {
                if !seen.insert(child_thread_id) {
                    continue;
                }
                let depth = parent_depth + 1;
                let child_metadata = state_db_ctx
                    .get_thread(child_thread_id)
                    .await
                    .ok()
                    .flatten();
                if child_thread_id == target_thread_id {
                    let Some(metadata) = child_metadata.as_ref() else {
                        return Err(CodexErr::UnsupportedOperation(format!(
                            "agent thread `{target_thread_id}` is missing persisted agent metadata"
                        )));
                    };
                    if metadata.archived_at.is_some() {
                        return Err(CodexErr::UnsupportedOperation(format!(
                            "agent thread `{target_thread_id}` is archived"
                        )));
                    }
                    if persisted_agent_metadata_from_state_metadata(child_thread_id, metadata)
                        .is_none()
                    {
                        return Err(CodexErr::UnsupportedOperation(format!(
                            "agent thread `{target_thread_id}` is missing persisted agent metadata"
                        )));
                    }
                    return Ok(Some(PersistedAgentTarget {
                        thread_id: child_thread_id,
                        parent_thread_id,
                        depth,
                    }));
                }
                let Some(metadata) = child_metadata else {
                    continue;
                };
                if metadata.archived_at.is_some() {
                    continue;
                }
                queue.push_back((child_thread_id, depth));
            }
        }
        Ok(None)
    }

    async fn root_thread_id_for_persisted_agent_lookup(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
    ) -> Option<ThreadId> {
        if thread_spawn_parent_thread_id(current_session_source).is_none() {
            return Some(current_thread_id);
        }
        self.persisted_thread_spawn_root(current_thread_id)
            .await
            .or_else(|| self.state.agent_id_for_path(&AgentPath::root()))
    }

    async fn persisted_agent_target_for_path(
        &self,
        root_thread_id: ThreadId,
        agent_path: &AgentPath,
    ) -> CodexResult<Option<PersistedAgentTarget>> {
        let state = self.upgrade()?;
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return Ok(None);
        };
        let mut queue = VecDeque::from([(root_thread_id, 0)]);
        let mut seen = HashSet::from([root_thread_id]);
        let mut candidates = Vec::new();
        while let Some((parent_thread_id, parent_depth)) = queue.pop_front() {
            let child_ids = state_db_ctx
                .list_thread_spawn_children_with_status(
                    parent_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
                .map_err(|err| {
                    CodexErr::Fatal(format!(
                        "failed to load persisted thread-spawn children: {err}"
                    ))
                })?;
            for child_thread_id in child_ids {
                if !seen.insert(child_thread_id) {
                    continue;
                }
                let depth = parent_depth + 1;
                let Some(metadata) = state_db_ctx
                    .get_thread(child_thread_id)
                    .await
                    .ok()
                    .flatten()
                else {
                    continue;
                };
                if metadata.archived_at.is_some() {
                    continue;
                }
                if metadata.agent_path.as_deref() == Some(agent_path.as_str()) {
                    let target = PersistedAgentTarget {
                        thread_id: child_thread_id,
                        parent_thread_id,
                        depth,
                    };
                    candidates.push(PersistedAgentPathCandidate {
                        target,
                        updated_at: metadata.updated_at,
                        final_status: self.persisted_final_agent_status(child_thread_id).await,
                    });
                }
                queue.push_back((child_thread_id, depth));
            }
        }
        self.select_persisted_agent_path_target(agent_path, candidates)
    }

    fn select_persisted_agent_path_target(
        &self,
        agent_path: &AgentPath,
        mut candidates: Vec<PersistedAgentPathCandidate>,
    ) -> CodexResult<Option<PersistedAgentTarget>> {
        match candidates.len() {
            0 => return Ok(None),
            1 => return Ok(candidates.pop().map(|candidate| candidate.target)),
            _ => {}
        }

        let non_final_indices = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| candidate.final_status.is_none().then_some(index))
            .collect::<Vec<_>>();
        if non_final_indices.len() == 1
            && candidates.iter().enumerate().all(|(index, candidate)| {
                index == non_final_indices[0] || candidate.final_status.is_some()
            })
        {
            return Ok(Some(candidates.swap_remove(non_final_indices[0]).target));
        }
        if non_final_indices.len() > 1 {
            return Err(CodexErr::UnsupportedOperation(format!(
                "agent path `{}` is ambiguous in persisted agent registry",
                agent_path.as_str()
            )));
        }

        let Some(parent_thread_id) = candidates
            .first()
            .map(|candidate| candidate.target.parent_thread_id)
        else {
            return Ok(None);
        };
        if !candidates
            .iter()
            .all(|candidate| candidate.target.parent_thread_id == parent_thread_id)
        {
            return Err(CodexErr::UnsupportedOperation(format!(
                "agent path `{}` is ambiguous in persisted agent registry",
                agent_path.as_str()
            )));
        }

        candidates.sort_by(|left, right| {
            right.updated_at.cmp(&left.updated_at).then_with(|| {
                right
                    .target
                    .thread_id
                    .to_string()
                    .cmp(&left.target.thread_id.to_string())
            })
        });
        let newest = &candidates[0];
        if candidates
            .get(1)
            .is_some_and(|next| next.updated_at == newest.updated_at)
        {
            return Err(CodexErr::UnsupportedOperation(format!(
                "agent path `{}` is ambiguous in persisted agent registry",
                agent_path.as_str()
            )));
        }

        Ok(Some(candidates.swap_remove(0).target))
    }

    /// Subscribe to status updates for `agent_id`, yielding the latest value and changes.
    pub(crate) async fn subscribe_status(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<watch::Receiver<AgentStatus>> {
        let state = self.upgrade()?;
        state.subscribe_live_thread_status(agent_id).await
    }

    pub(crate) async fn direct_subagent_paths(&self, parent_thread_id: ThreadId) -> Vec<AgentPath> {
        let Ok(agents) = self.open_thread_spawn_children(parent_thread_id).await else {
            return Vec::new();
        };

        direct_subagent_paths_from_children(agents)
    }

    pub(crate) async fn list_agents(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        path_prefix: Option<&str>,
    ) -> CodexResult<Vec<ListedAgent>> {
        let _state = self.upgrade()?;
        let current_agent_path = self
            .current_agent_path_with_persisted_metadata(current_thread_id, current_session_source)
            .await;
        let root_thread_id = if thread_spawn_parent_thread_id(current_session_source).is_none() {
            Some(current_thread_id)
        } else {
            self.persisted_thread_spawn_root(current_thread_id)
                .await
                .or_else(|| self.state.agent_id_for_path(&AgentPath::root()))
        };
        let mut registered_agents = self
            .registered_agents_with_persisted_descendants(root_thread_id)
            .await;
        let mut registered_thread_ids = registered_agents
            .iter()
            .filter_map(|metadata| metadata.agent_id)
            .collect::<HashSet<_>>();
        let mut registered_agent_paths = registered_agents
            .iter()
            .filter_map(|metadata| metadata.agent_path.as_ref().map(ToString::to_string))
            .collect::<HashSet<_>>();
        let scoped_thread_ids = if let Some(root_thread_id) = root_thread_id {
            let mut scoped_thread_ids = HashSet::from([root_thread_id]);
            if let Ok(descendant_ids) = self.live_thread_spawn_descendants(root_thread_id).await {
                scoped_thread_ids.extend(descendant_ids);
            }
            Some(scoped_thread_ids)
        } else {
            None
        };
        registered_agents.extend(
            self.external_agents
                .list()
                .into_iter()
                .filter(|run| {
                    scoped_thread_ids
                        .as_ref()
                        .is_none_or(|thread_ids| thread_ids.contains(&run.parent_thread_id))
                })
                .filter(|run| {
                    registered_thread_ids.insert(run.thread_id)
                        && registered_agent_paths.insert(run.agent_path.to_string())
                })
                .map(|run| external_metadata(&run)),
        );
        let plan = list_agents_plan(&current_agent_path, path_prefix, registered_agents)
            .map_err(CodexErr::UnsupportedOperation)?;

        let mut agents = Vec::with_capacity(plan.candidates.len().saturating_add(1));
        let root_path = AgentPath::root();
        if plan.include_root
            && let Some(root_thread_id) = self.state.agent_id_for_path(&root_path)
            && let Some(lifecycle_status) = self.listed_thread_lifecycle(root_thread_id).await
        {
            agents.push(root_listed_agent(lifecycle_status));
        }

        for candidate in plan.candidates {
            let lifecycle_status = self.listed_thread_lifecycle(candidate.thread_id).await;
            let Some(lifecycle_status) = lifecycle_status else {
                continue;
            };
            agents.push(ListedAgent {
                agent_name: candidate.agent_name,
                lifecycle_status,
                last_task_message: candidate.last_task_message,
            });
        }

        Ok(agents)
    }

    async fn listed_thread_lifecycle(&self, thread_id: ThreadId) -> Option<ThreadLifecycleStatus> {
        let lifecycle = Box::pin(self.normalized_thread_lifecycle(thread_id)).await;
        if matches!(lifecycle, ThreadLifecycleStatus::NotLoaded) {
            None
        } else {
            Some(lifecycle)
        }
    }

    pub(crate) async fn normalized_thread_lifecycle(
        &self,
        thread_id: ThreadId,
    ) -> ThreadLifecycleStatus {
        if let Some(run) = self.external_agents.get(thread_id) {
            return normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
                manager_available: true,
                thread_found: true,
                live_agent_status: Some(run.status),
                ..Default::default()
            });
        }
        let Ok(state) = self.upgrade() else {
            return normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs::default());
        };
        let snapshot = state.live_thread_activity_snapshot(thread_id).await;
        normalized_thread_lifecycle_from_inputs(ThreadLifecycleInputs {
            manager_available: snapshot.manager_available,
            active_event_subscription_count: snapshot.active_event_subscription_count,
            thread_found: snapshot.thread_found,
            has_active_turn: snapshot.has_active_turn,
            live_agent_status: snapshot.status,
            persisted_final_agent_status: self.persisted_final_agent_status(thread_id).await,
        })
    }

    async fn registered_agents_with_persisted_descendants(
        &self,
        root_thread_id: Option<ThreadId>,
    ) -> Vec<AgentMetadata> {
        let mut registered_agents = self.state.registered_agents();
        let Some(root_thread_id) = root_thread_id else {
            return registered_agents;
        };
        let mut scoped_thread_ids = HashSet::from([root_thread_id]);
        if let Ok(descendant_ids) = self.live_thread_spawn_descendants(root_thread_id).await {
            scoped_thread_ids.extend(descendant_ids);
        }
        registered_agents.retain(|metadata| {
            metadata
                .agent_id
                .is_some_and(|thread_id| scoped_thread_ids.contains(&thread_id))
        });
        let Ok(state) = self.upgrade() else {
            return registered_agents;
        };
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return registered_agents;
        };
        let Ok(descendant_ids) = state_db_ctx
            .list_thread_spawn_descendants_with_status(
                root_thread_id,
                DirectionalThreadSpawnEdgeStatus::Open,
            )
            .await
        else {
            return registered_agents;
        };

        let mut known_thread_ids = registered_agents
            .iter()
            .filter_map(|metadata| metadata.agent_id)
            .collect::<std::collections::HashSet<_>>();
        let mut known_agent_paths = registered_agents
            .iter()
            .filter_map(|metadata| metadata.agent_path.as_ref().map(ToString::to_string))
            .collect::<std::collections::HashSet<_>>();

        for descendant_id in descendant_ids {
            if known_thread_ids.contains(&descendant_id) {
                continue;
            }
            let Some(agent_metadata) = self
                .persisted_agent_metadata(descendant_id, state_db_ctx.as_ref())
                .await
            else {
                continue;
            };
            if agent_metadata
                .agent_path
                .as_ref()
                .is_some_and(AgentPath::is_root)
            {
                continue;
            }
            let Some(agent_path) = agent_metadata.agent_path.as_ref() else {
                continue;
            };
            if !known_agent_paths.insert(agent_path.to_string()) {
                continue;
            }

            registered_agents.push(agent_metadata);
            known_thread_ids.insert(descendant_id);
        }

        registered_agents
    }

    async fn persisted_agent_metadata(
        &self,
        thread_id: ThreadId,
        state_db_ctx: &dyn state_api::ThreadStateRuntime,
    ) -> Option<AgentMetadata> {
        let metadata = state_db_ctx.get_thread(thread_id).await.ok().flatten()?;
        persisted_agent_metadata_from_state_metadata(thread_id, &metadata)
    }

    async fn current_agent_path_with_persisted_metadata(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
    ) -> AgentPath {
        let current_agent_path = self.current_agent_path(current_thread_id, current_session_source);
        if !current_agent_path.is_root()
            || thread_spawn_parent_thread_id(current_session_source).is_none()
        {
            return current_agent_path;
        }

        let Ok(state) = self.upgrade() else {
            return current_agent_path;
        };
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return current_agent_path;
        };
        self.persisted_agent_metadata(current_thread_id, state_db_ctx.as_ref())
            .await
            .and_then(|metadata| metadata.agent_path)
            .unwrap_or(current_agent_path)
    }

    async fn persisted_thread_spawn_root(&self, thread_id: ThreadId) -> Option<ThreadId> {
        let state = self.upgrade().ok()?;
        let state_db_ctx = state.thread_state_runtime()?;
        state_db_ctx
            .find_thread_spawn_root(thread_id)
            .await
            .ok()
            .flatten()
    }

    async fn persisted_final_agent_status(&self, thread_id: ThreadId) -> Option<AgentStatus> {
        let state = self.upgrade().ok()?;
        let stored_thread = state
            .read_stored_thread(ReadThreadParams {
                thread_id,
                include_archived: true,
                include_history: true,
            })
            .await
            .ok()?;
        let history = stored_thread.history?;
        let mut status = history.items.iter().filter_map(|item| match item {
            protocol::protocol::RolloutItem::EventMsg(event) => agent_status_from_event(event),
            _ => None,
        });
        status.next_back().filter(is_final)
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_thread_spawn(
        &self,
        reservation: &mut SpawnReservation,
        config: &config_service::Config,
        parent_thread_id: ThreadId,
        depth: i32,
        agent_path: Option<AgentPath>,
        agent_role: Option<String>,
        agent_mode: AgentMode,
        preferred_agent_nickname: Option<String>,
    ) -> CodexResult<(SessionSource, AgentMetadata)> {
        if depth == 1
            && self
                .state
                .agent_metadata_for_thread(parent_thread_id)
                .and_then(|metadata| metadata.agent_path)
                .is_none()
        {
            self.state.register_root_thread(parent_thread_id);
        }
        let role_name = agent_role.as_deref().unwrap_or(DEFAULT_ROLE_NAME);
        let configured_candidates = resolve_role_config(&config.agent_roles, role_name)
            .and_then(|role| role.nickname_candidates.as_deref());
        prepare_thread_spawn_plan(
            reservation,
            ThreadSpawnPlanInput {
                parent_thread_id,
                depth,
                agent_path,
                agent_role,
                agent_mode,
                configured_nickname_candidates: configured_candidates,
                preferred_agent_nickname: preferred_agent_nickname.as_deref(),
            },
        )
    }

    fn upgrade(&self) -> CodexResult<Arc<ThreadServiceState>> {
        self.manager
            .upgrade()
            .ok_or_else(|| CodexErr::UnsupportedOperation("thread manager dropped".to_string()))
    }

    async fn inherited_shell_snapshot_for_source(
        &self,
        state: &Arc<ThreadServiceState>,
        session_source: Option<&SessionSource>,
    ) -> Option<Arc<ShellSnapshot>> {
        let Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        })) = session_source
        else {
            return None;
        };

        let parent_thread = state.get_thread(*parent_thread_id).await.ok()?;
        parent_thread.codex.session.user_shell().shell_snapshot()
    }

    async fn inherited_exec_policy_for_source(
        &self,
        state: &Arc<ThreadServiceState>,
        session_source: Option<&SessionSource>,
        child_config: &config_service::Config,
    ) -> Option<Arc<permissions_service::ExecPolicyManager>> {
        let Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id, ..
        })) = session_source
        else {
            return None;
        };

        let parent_thread = state.get_thread(*parent_thread_id).await.ok()?;
        let parent_config = parent_thread.codex.session.get_config().await;
        if !config_service::child_uses_parent_exec_policy(&parent_config, child_config) {
            return None;
        }

        Some(Arc::clone(
            &parent_thread.codex.session.services.exec_policy,
        ))
    }

    async fn open_thread_spawn_children(
        &self,
        parent_thread_id: ThreadId,
    ) -> CodexResult<Vec<(ThreadId, AgentMetadata)>> {
        let state = self.upgrade()?;
        let mut children_by_parent = self.live_thread_spawn_children().await?;
        let mut children = children_by_parent
            .remove(&parent_thread_id)
            .unwrap_or_default();
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return Ok(children);
        };

        let mut seen_child_ids = children
            .iter()
            .map(|(thread_id, _)| *thread_id)
            .collect::<std::collections::HashSet<_>>();
        let mut seen_agent_paths = children
            .iter()
            .filter_map(|(_, metadata)| metadata.agent_path.as_ref().map(ToString::to_string))
            .collect::<std::collections::HashSet<_>>();
        let child_ids = state_db_ctx
            .list_thread_spawn_children_with_status(
                parent_thread_id,
                DirectionalThreadSpawnEdgeStatus::Open,
            )
            .await
            .unwrap_or_default();
        for child_thread_id in child_ids {
            if !seen_child_ids.insert(child_thread_id) {
                continue;
            }
            if let Some(metadata) = self
                .persisted_agent_metadata(child_thread_id, state_db_ctx.as_ref())
                .await
            {
                if let Some(agent_path) = metadata.agent_path.as_ref()
                    && !seen_agent_paths.insert(agent_path.to_string())
                {
                    continue;
                }
                children.push((child_thread_id, metadata));
            }
        }
        children.sort_by(|left, right| {
            left.1
                .agent_path
                .as_deref()
                .unwrap_or_default()
                .cmp(right.1.agent_path.as_deref().unwrap_or_default())
                .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
        });
        Ok(children)
    }

    async fn live_thread_spawn_children(
        &self,
    ) -> CodexResult<HashMap<ThreadId, Vec<(ThreadId, AgentMetadata)>>> {
        let state = self.upgrade()?;
        let state_db_ctx = state.thread_state_runtime();
        let mut children = Vec::new();

        for thread_id in state.list_live_thread_ids().await {
            let Ok(snapshot) = state.live_thread_config_snapshot(thread_id).await else {
                continue;
            };
            let Some(parent_thread_id) = thread_spawn_parent_thread_id(&snapshot.session_source)
            else {
                continue;
            };
            let mut metadata =
                self.state
                    .agent_metadata_for_thread(thread_id)
                    .unwrap_or(AgentMetadata {
                        agent_id: Some(thread_id),
                        ..Default::default()
                    });
            if metadata.agent_path.is_none() {
                metadata.agent_path = snapshot.session_source.get_agent_path();
            }
            if metadata.agent_nickname.is_none() {
                metadata.agent_nickname = snapshot.session_source.get_nickname();
            }
            if metadata.agent_role.is_none() {
                metadata.agent_role = snapshot.session_source.get_agent_role();
            }
            if (metadata.agent_path.is_none()
                || metadata.agent_nickname.is_none()
                || metadata.agent_role.is_none())
                && let Some(state_db_ctx) = state_db_ctx.as_ref()
                && let Some(persisted_metadata) = self
                    .persisted_agent_metadata(thread_id, state_db_ctx.as_ref())
                    .await
            {
                if metadata.agent_path.is_none() {
                    metadata.agent_path = persisted_metadata.agent_path;
                }
                if metadata.agent_nickname.is_none() {
                    metadata.agent_nickname = persisted_metadata.agent_nickname;
                }
                if metadata.agent_role.is_none() {
                    metadata.agent_role = persisted_metadata.agent_role;
                }
            }
            children.push(ThreadSpawnChild {
                parent_thread_id,
                thread_id,
                metadata,
            });
        }

        Ok(build_thread_spawn_children_by_parent(children))
    }

    async fn persist_thread_spawn_edge_for_source(
        &self,
        child_thread_id: ThreadId,
        session_source: Option<&SessionSource>,
    ) {
        let Some(parent_thread_id) = session_source.and_then(thread_spawn_parent_thread_id) else {
            return;
        };
        let child_agent_path = session_source.and_then(SessionSource::get_agent_path);
        let Ok(state) = self.upgrade() else {
            return;
        };
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return;
        };
        if let Some(child_agent_path) = child_agent_path.as_ref()
            && let Ok(open_child_ids) = state_db_ctx
                .list_thread_spawn_children_with_status(
                    parent_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
        {
            for open_child_id in open_child_ids {
                if open_child_id == child_thread_id {
                    continue;
                }
                let has_same_path = state_db_ctx
                    .get_thread(open_child_id)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|metadata| metadata.agent_path)
                    .is_some_and(|path| path == child_agent_path.as_str());
                if has_same_path
                    && let Err(err) = state_db_ctx
                        .set_thread_spawn_edge_status(
                            open_child_id,
                            DirectionalThreadSpawnEdgeStatus::Closed,
                        )
                        .await
                {
                    warn!(
                        "failed to close superseded thread-spawn edge for {open_child_id}: {err}"
                    );
                }
            }
        }
        if let Err(err) = state_db_ctx
            .upsert_thread_spawn_edge(
                parent_thread_id,
                child_thread_id,
                DirectionalThreadSpawnEdgeStatus::Open,
            )
            .await
        {
            warn!("failed to persist thread-spawn edge: {err}");
        }
    }

    async fn live_thread_spawn_descendants(
        &self,
        root_thread_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        Ok(thread_spawn_descendants(
            root_thread_id,
            self.live_thread_spawn_children().await?,
        ))
    }
}

fn persisted_agent_metadata_from_state_metadata(
    thread_id: ThreadId,
    metadata: &state_api::ThreadMetadata,
) -> Option<AgentMetadata> {
    if metadata.archived_at.is_some() {
        return None;
    }
    let agent_path = metadata
        .agent_path
        .as_deref()
        .map(|path| AgentPath::from_string(path.to_string()))
        .transpose()
        .ok()??;

    Some(AgentMetadata {
        agent_id: Some(thread_id),
        agent_path: Some(agent_path),
        agent_nickname: metadata.agent_nickname.clone(),
        agent_role: metadata.agent_role.clone(),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
