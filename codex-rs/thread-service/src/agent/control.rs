use crate::agent::AgentStatus;
use crate::agent::external::ExternalAgentInput;
use crate::agent::external::ExternalAgentInputSink;
use crate::agent::external::ExternalAgentRun;
use crate::agent::external::ExternalProcessEvent;
use crate::agent::external::ExternalProviderSession;
use crate::agent::external::ExternalSpawnConfig;
use crate::agent::external::ExternalStreamingSession;
use crate::agent::external::ExternalToolCall;
use crate::agent::external::ExternalToolName;
use crate::agent::external::ExternalToolResult;
use crate::agent::external::SharedExternalAgentRegistry;
use crate::agent::external::bounded_external_output;
use crate::agent::external::bounded_external_tool_arguments;
use crate::agent::external::bounded_external_tool_result;
use crate::agent::external::completion_communication;
use crate::agent::external::external_agent_context_prompt;
use crate::agent::external::external_live_agent;
use crate::agent::external::external_metadata;
use crate::agent::external::external_restore_plan_support;
use crate::agent::external::external_session_spec;
use crate::agent::external::external_tool_name;
use crate::agent::external::external_tool_result_input;
use crate::agent::spawn_support::thread_spawn_source;
use crate::runtime_shell_snapshot::ShellSnapshot;
use crate::session::emit_subagent_session_started;
use crate::session::session::ThreadWaitSource;
use crate::thread::NewExternalRootThread;
use crate::thread::NewThread;
use crate::thread::ResumeThreadWithHistoryOptions;
use crate::thread::ThreadConfigSnapshot;
use crate::thread::ThreadServiceState;
use chrono::DateTime;
use chrono::Utc;
use codex_agent_roles::AgentRoleConfig;
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
use codex_utils_absolute_path::AbsolutePathBuf;
use futures::future::BoxFuture;
use protocol::AgentPath;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::error::CodexErr;
use protocol::error::Result as CodexResult;
#[cfg(test)]
use protocol::models::ResponseItem;
use protocol::protocol::AgentMessageEvent;
use protocol::protocol::ErrorEvent;
use protocol::protocol::ExternalReconnectDescriptor;
use protocol::protocol::ExternalTerminalStatus;
use protocol::protocol::ExternalTerminalStatusEvent;
use protocol::protocol::ExternalToolCallDisplayEvent;
use protocol::protocol::ExternalToolCallStatus;
use protocol::protocol::InitialHistory;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::Op;
use protocol::protocol::ResumedHistory;
use protocol::protocol::RolloutItem;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::ThreadSource;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::UserMessageEvent;
use serde::Deserialize;
use serde_json::json;
use state_api::DirectionalThreadSpawnEdgeStatus;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use thread_service_api::AgentDirectoryEntry;
use thread_service_api::AgentDirectoryEntrySource;
use thread_service_api::AgentDirectoryListRequest;
use thread_service_api::AgentDirectoryListResult;
use thread_service_api::AgentReferenceResolution;
use thread_service_api::AgentReferenceResolutionRequest;
use thread_service_api::LiveThreadActivitySource;
use thread_service_api::LiveThreadCommandRuntime;
use thread_service_api::LiveThreadInfo;
use thread_service_api::LiveThreadInspectionRuntime;
use thread_service_api::LiveThreadSnapshot;
use thread_service_api::LiveThreadStateRuntimeSource;
use thread_service_api::ThreadLifecycleRuntime;
use thread_store_api::ExternalLiveRestoreEligibility;
use thread_store_api::ReadThreadParams;
use thread_store_api::SharedLiveThread;
use thread_store_api::ThreadMetadataPatch;
use thread_store_api::external_live_restore_eligibility;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tool_service_api::FunctionCallError;
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

#[derive(Clone)]
struct AgentDirectoryMetadata {
    metadata: AgentMetadata,
    source: AgentDirectoryEntrySource,
    parent_thread_id: Option<ThreadId>,
    depth: Option<i32>,
}

struct PersistedAgentPathCandidate {
    target: PersistedAgentTarget,
    updated_at: DateTime<Utc>,
    final_status: Option<AgentStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalSpawnAgentArgs {
    task_name: String,
    provider: SpawnAgentProvider,
    cwd: AbsolutePathBuf,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalFollowupTaskArgs {
    target: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalListAgentsArgs {
    path_prefix: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalCloseAgentArgs {
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalPollEventArgs {}

struct ExternalToolContext {
    thread_id: ThreadId,
    parent_thread_id: ThreadId,
    agent_path: AgentPath,
    provider: SpawnAgentProvider,
    depth: i32,
    spawn_config: Option<ExternalSpawnConfig>,
}

fn parse_external_arguments<T>(arguments: &serde_json::Value) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments.clone()).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse external tool arguments: {err}"))
    })
}

fn external_tool_context(run: &ExternalAgentRun) -> ExternalToolContext {
    ExternalToolContext {
        thread_id: run.thread_id,
        parent_thread_id: run.parent_thread_id,
        agent_path: run.agent_path.clone(),
        provider: run.provider,
        depth: run.depth,
        spawn_config: run.spawn_config.clone(),
    }
}

fn external_session_source_for(
    parent_thread_id: ThreadId,
    depth: i32,
    agent_path: AgentPath,
    provider: SpawnAgentProvider,
) -> SessionSource {
    SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth,
        agent_path: Some(agent_path),
        agent_nickname: Some(provider_label(provider).to_string()),
        agent_role: Some(provider_label(provider).to_string()),
    })
}

fn external_tool_directory_session_source(sender: &ExternalToolContext) -> SessionSource {
    if sender.agent_path.is_root() {
        SessionSource::Unknown
    } else {
        external_session_source_for(
            sender.parent_thread_id,
            sender.depth,
            sender.agent_path.clone(),
            sender.provider,
        )
    }
}

fn external_live_thread_snapshot(
    config: &ExternalSpawnConfig,
    thread_id: ThreadId,
    session_source: SessionSource,
    agent_metadata: &AgentMetadata,
) -> LiveThreadSnapshot {
    external_live_thread_snapshot_with_source(
        config,
        thread_id,
        session_source,
        ThreadSource::Subagent,
        agent_metadata,
    )
}

fn external_live_thread_snapshot_with_source(
    config: &ExternalSpawnConfig,
    thread_id: ThreadId,
    session_source: SessionSource,
    thread_source: ThreadSource,
    agent_metadata: &AgentMetadata,
) -> LiveThreadSnapshot {
    LiveThreadSnapshot {
        info: LiveThreadInfo {
            session_id: SessionId::from(thread_id),
            rollout_path: None,
        },
        config_snapshot: ThreadConfigSnapshot {
            model: config.model.clone(),
            model_provider_id: config.model_provider_id.clone(),
            service_tier: config.service_tier.clone(),
            approval_policy: config.approval_policy,
            approvals_reviewer: config.approvals_reviewer,
            permission_profile: config.permission_profile.clone(),
            active_permission_profile: config.active_permission_profile.clone(),
            cwd: config.cwd.clone(),
            workspace_roots: config.workspace_roots.clone(),
            profile_workspace_roots: Vec::new(),
            ephemeral: false,
            reasoning_effort: config.reasoning_effort,
            personality: config.personality,
            session_source,
            root_agent_path: agent_metadata.agent_path.as_ref().map(ToString::to_string),
            root_agent_role: agent_metadata.agent_role.clone(),
            thread_source: Some(thread_source),
        },
    }
}

fn provider_label(provider: SpawnAgentProvider) -> &'static str {
    match provider {
        SpawnAgentProvider::Native => "native",
        SpawnAgentProvider::CodexCli => "codex_cli",
        SpawnAgentProvider::ClaudeCli => "claude_cli",
        SpawnAgentProvider::Opencode => "opencode",
    }
}

impl AgentControl {
    pub(crate) fn new_with_registry(
        manager: Weak<ThreadServiceState>,
        state: Arc<AgentRegistry>,
    ) -> Self {
        Self::new_with_external_registry(manager, state, SharedExternalAgentRegistry::default())
    }

    pub(crate) fn new_with_external_registry(
        manager: Weak<ThreadServiceState>,
        state: Arc<AgentRegistry>,
        external_agents: SharedExternalAgentRegistry,
    ) -> Self {
        Self {
            manager,
            state,
            external_agents,
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
        let config = ExternalSpawnConfig::from_config(&config);
        self.spawn_external_agent_with_metadata_sync(
            config,
            provider,
            message,
            session_source,
            /*register_global_agent_metadata*/ true,
            options,
        )
        .await
    }

    pub(crate) async fn start_external_root_thread(
        &self,
        config: ExternalSpawnConfig,
        provider: SpawnAgentProvider,
        session_source: SessionSource,
    ) -> CodexResult<NewExternalRootThread> {
        if provider == SpawnAgentProvider::Native {
            return Err(CodexErr::UnsupportedOperation(
                "native is not an external CLI provider".to_string(),
            ));
        }

        let mut config = config;
        config.model_provider_id = provider_label(provider).to_string();
        external_session_spec(provider, config.cwd.as_path())
            .map_err(CodexErr::UnsupportedOperation)?;

        let thread_id = ThreadId::new();
        let agent_metadata = AgentMetadata::default();
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let live_thread = self
            .create_external_thread_persistence(
                &config,
                thread_id,
                session_source.clone(),
                ThreadSource::User,
                &agent_metadata,
            )
            .await?;
        live_thread.persist().await.map_err(|err| {
            CodexErr::Fatal(format!(
                "failed to persist external root thread {thread_id}: {err}"
            ))
        })?;
        let rollout_path = live_thread.local_rollout_path().await.map_err(|err| {
            CodexErr::Fatal(format!("failed to load external rollout path: {err}"))
        })?;
        let session_configured = SessionConfiguredEvent {
            session_id: SessionId::from(thread_id),
            thread_id,
            forked_from_id: None,
            thread_source: Some(ThreadSource::User),
            thread_name: None,
            model: config.model.clone(),
            model_provider_id: config.model_provider_id.clone(),
            service_tier: config.service_tier.clone(),
            approval_policy: config.approval_policy,
            approvals_reviewer: config.approvals_reviewer,
            permission_profile: config.permission_profile.clone(),
            active_permission_profile: config.active_permission_profile.clone(),
            cwd: config.cwd.clone(),
            reasoning_effort: config.reasoning_effort,
            initial_messages: Some(Vec::new()),
            network_proxy: None,
            rollout_path,
        };
        let snapshot = external_live_thread_snapshot_with_source(
            &config,
            thread_id,
            session_source,
            ThreadSource::User,
            &agent_metadata,
        );
        self.upgrade()?
            .register_external_live_thread_snapshot_with_features(
                thread_id,
                snapshot,
                config.features.clone(),
                AgentStatus::Running,
            )
            .await;

        let run = ExternalAgentRun {
            thread_id,
            parent_thread_id: thread_id,
            agent_path: AgentPath::root(),
            provider,
            depth: 0,
            spawn_config: Some(config.clone()),
            input_sink: Some(ExternalAgentInputSink::new(input_tx)),
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        };
        self.external_agents.insert_running(run);
        if let Ok(state) = self.upgrade() {
            state.notify_thread_started(thread_id);
        }

        let task_control = self.clone();
        let cwd = config.cwd.as_path().to_path_buf();
        let handle = tokio::spawn(async move {
            let status = task_control
                .run_external_agent_loop(thread_id, provider, cwd, None, input_rx)
                .await;
            task_control
                .complete_external_agent(thread_id, status)
                .await;
        });
        self.external_agents
            .attach_abort_handle(thread_id, handle.abort_handle());

        Ok(NewExternalRootThread {
            thread_id,
            session_configured,
        })
    }

    pub(crate) fn has_external_root_thread(&self, thread_id: ThreadId) -> bool {
        self.external_agents
            .get(thread_id)
            .is_some_and(|run| run.agent_path.is_root())
    }

    pub(crate) async fn send_external_root_input(
        &self,
        thread_id: ThreadId,
        message: String,
    ) -> CodexResult<String> {
        let Some(run) = self.external_agents.get(thread_id) else {
            return Err(CodexErr::ThreadNotFound(thread_id));
        };
        if !run.agent_path.is_root() {
            return Err(CodexErr::ThreadNotFound(thread_id));
        }
        let turn_id = uuid::Uuid::new_v4().to_string();
        let input_sink = self
            .external_agents
            .begin_root_turn(thread_id, turn_id.clone())
            .map_err(CodexErr::UnsupportedOperation)?;
        input_sink
            .send_with_turn_id(Some(turn_id.clone()), message.clone())
            .map_err(|err| {
                self.external_agents.clear_active_turn(thread_id, &turn_id);
                CodexErr::UnsupportedOperation(err)
            })?;
        self.external_agents
            .note_thread_wait_event(thread_id, ThreadWaitSource::UserInput);
        self.external_agents
            .update_last_task_message(thread_id, message);
        Ok(turn_id)
    }

    pub(crate) async fn close_external_root_thread(
        &self,
        thread_id: ThreadId,
    ) -> CodexResult<String> {
        if !self.has_external_root_thread(thread_id) {
            return Err(CodexErr::ThreadNotFound(thread_id));
        }
        let shutdown_run = self.external_agents.shutdown_and_remove(thread_id);
        let status = shutdown_run
            .as_ref()
            .map(|run| run.status.clone())
            .unwrap_or(AgentStatus::Shutdown);
        self.persist_external_terminal_status(thread_id, &status)
            .await;
        if let Ok(state) = self.upgrade() {
            state
                .update_external_live_thread_status(thread_id, status.clone())
                .await;
            state.notify_thread_status_changed_with_status(thread_id, Some(status));
            let _ = ThreadLifecycleRuntime::remove_live_thread(state.as_ref(), thread_id).await;
        }
        Ok(String::new())
    }

    fn spawn_external_agent_with_metadata_sync(
        &self,
        config: ExternalSpawnConfig,
        provider: SpawnAgentProvider,
        message: String,
        session_source: SessionSource,
        register_global_agent_metadata: bool,
        options: SpawnAgentOptions,
    ) -> BoxFuture<'static, CodexResult<LiveAgent>> {
        let control = self.clone();
        Box::pin(async move {
            external_session_spec(provider, config.cwd.as_path())
                .map_err(CodexErr::UnsupportedOperation)?;
            let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth,
                agent_path,
                agent_role: _,
                ..
            }) = session_source
            else {
                return Err(CodexErr::UnsupportedOperation(
                    "external agents must be spawned as thread-spawn subagents".to_string(),
                ));
            };

            let (session_source, agent_metadata, thread_id, agent_path) = {
                let mut reservation = control.state.reserve_spawn_slot(config.agent_max_threads)?;
                let (session_source, mut agent_metadata) = if register_global_agent_metadata {
                    control.prepare_thread_spawn_with_roles(
                        &mut reservation,
                        &config.agent_roles,
                        parent_thread_id,
                        depth,
                        agent_path,
                        Some(provider_label(provider).to_string()),
                        options.agent_mode,
                        None,
                    )?
                } else {
                    let agent_path = agent_path.ok_or_else(|| {
                        CodexErr::UnsupportedOperation(
                            "external agent is missing agent path".to_string(),
                        )
                    })?;
                    let provider_name = provider_label(provider).to_string();
                    (
                        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                            parent_thread_id,
                            depth,
                            agent_path: Some(agent_path.clone()),
                            agent_nickname: Some(provider_name.clone()),
                            agent_role: Some(provider_name.clone()),
                        }),
                        AgentMetadata {
                            agent_id: None,
                            agent_path: Some(agent_path),
                            agent_nickname: Some(provider_name.clone()),
                            agent_role: Some(provider_name),
                            agent_mode: options.agent_mode,
                            last_task_message: None,
                            counted: true,
                        },
                    )
                };
                let thread_id = ThreadId::new();
                let agent_path = agent_metadata.agent_path.clone().ok_or_else(|| {
                    CodexErr::UnsupportedOperation(
                        "external agent is missing agent path".to_string(),
                    )
                })?;
                agent_metadata.agent_id = Some(thread_id);
                agent_metadata.last_task_message = Some(message.clone());
                let registry_metadata = if register_global_agent_metadata {
                    agent_metadata.clone()
                } else {
                    AgentMetadata {
                        agent_id: Some(thread_id),
                        agent_path: None,
                        agent_nickname: None,
                        agent_role: None,
                        agent_mode: options.agent_mode,
                        last_task_message: Some(message.clone()),
                        counted: true,
                    }
                };
                reservation.commit(registry_metadata);
                (session_source, agent_metadata, thread_id, agent_path)
            };
            let (input_tx, input_rx) = mpsc::unbounded_channel();
            let live_thread = match control
                .create_external_thread_persistence(
                    &config,
                    thread_id,
                    session_source.clone(),
                    ThreadSource::Subagent,
                    &agent_metadata,
                )
                .await
            {
                Ok(live_thread) => live_thread,
                Err(err) => {
                    control.state.release_spawned_thread(thread_id);
                    return Err(err);
                }
            };
            control
                .persist_thread_spawn_edge_for_source(thread_id, Some(&session_source))
                .await;
            control
                .upgrade()?
                .register_external_live_thread_snapshot_with_features(
                    thread_id,
                    external_live_thread_snapshot(
                        &config,
                        thread_id,
                        session_source.clone(),
                        &agent_metadata,
                    ),
                    config.features.clone(),
                    AgentStatus::Running,
                )
                .await;

            let run = ExternalAgentRun {
                thread_id,
                parent_thread_id,
                agent_path,
                provider,
                depth,
                spawn_config: Some(config.clone()),
                input_sink: Some(ExternalAgentInputSink::new(input_tx)),
                live_thread: Some(live_thread),
                status: AgentStatus::Running,
                active_turn_id: None,
                last_task_message: Some(message.clone()),
                abort_handle: None,
            };
            let live_agent = external_live_agent(&run);
            control.external_agents.insert_running(run);
            if let Ok(state) = control.upgrade() {
                state.notify_thread_started(thread_id);
            }

            let task_control = control.clone();
            let cwd = config.cwd.as_path().to_path_buf();
            let handle = tokio::spawn(async move {
                let status = task_control
                    .run_external_agent_loop(thread_id, provider, cwd, Some(message), input_rx)
                    .await;
                task_control
                    .complete_external_agent(thread_id, status)
                    .await;
            });
            control
                .external_agents
                .attach_abort_handle(thread_id, handle.abort_handle());

            Ok(live_agent)
        })
    }

    async fn run_external_agent_loop(
        &self,
        thread_id: ThreadId,
        provider: SpawnAgentProvider,
        cwd: PathBuf,
        initial_message: Option<String>,
        input_rx: mpsc::UnboundedReceiver<ExternalAgentInput>,
    ) -> AgentStatus {
        let mut stream = match ExternalStreamingSession::start(provider, cwd).await {
            Ok(stream) => stream,
            Err(message) => {
                self.persist_external_error(thread_id, &message).await;
                self.persist_external_terminal_status(
                    thread_id,
                    &AgentStatus::Errored(message.clone()),
                )
                .await;
                return AgentStatus::Errored(message);
            }
        };
        if let Some(descriptor) = stream.reconnect_descriptor()
            && let Err(err) = self
                .persist_external_reconnect_descriptor(thread_id, descriptor)
                .await
        {
            warn!("failed to persist external reconnect descriptor for {thread_id}: {err}");
        }
        self.run_external_agent_stream_loop(thread_id, initial_message, input_rx, &mut stream)
            .await
    }

    async fn run_external_agent_stream_loop<S>(
        &self,
        thread_id: ThreadId,
        initial_message: Option<String>,
        input_rx: mpsc::UnboundedReceiver<ExternalAgentInput>,
        stream: &mut S,
    ) -> AgentStatus
    where
        S: ExternalProviderSession + ?Sized,
    {
        let provider_input = stream.input_sink();
        let mut current_turn_id = None::<String>;
        if let Some(message) = initial_message {
            let turn_id = uuid::Uuid::new_v4().to_string();
            self.persist_external_terminal_status_with_turn_id(
                thread_id,
                Some(&turn_id),
                &AgentStatus::Running,
            )
            .await;
            current_turn_id = Some(turn_id);
            let initial_input = external_agent_context_prompt(&message);
            self.persist_external_user_message(thread_id, &message)
                .await;
            if let Err(err) = provider_input.send(initial_input) {
                self.persist_external_error(thread_id, &err).await;
                self.persist_external_terminal_status_with_turn_id(
                    thread_id,
                    current_turn_id.as_deref(),
                    &AgentStatus::Errored(err.clone()),
                )
                .await;
                return AgentStatus::Errored(err);
            }
        }
        let mut input_rx = Some(input_rx);
        let mut last_status = None::<String>;
        loop {
            tokio::select! {
                biased;
                input = async {
                    match input_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending::<Option<ExternalAgentInput>>().await,
                    }
                } => {
                    match input {
                        Some(input) => {
                            if let Some(turn_id) = input.turn_id {
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    Some(&turn_id),
                                    &AgentStatus::Running,
                                ).await;
                                current_turn_id = Some(turn_id);
                            }
                            self.persist_external_user_message(thread_id, &input.content).await;
                            if let Err(err) = provider_input.send(input.content) {
                                self.persist_external_error(thread_id, &err).await;
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    current_turn_id.as_deref(),
                                    &AgentStatus::Errored(err.clone()),
                                ).await;
                                return AgentStatus::Errored(err);
                            }
                        }
                        None => {
                            input_rx = None;
                        }
                    }
                }
                event = stream.next_event() => {
                    match event {
                        Ok(ExternalProcessEvent::Cli(crate::agent::external::ExternalCliEvent::ToolCall(call))) => {
                            let tool = external_tool_name(&call.tool);
                            let arguments = bounded_external_tool_arguments(&call.arguments);
                            if current_turn_id.is_none() {
                                let turn_id = uuid::Uuid::new_v4().to_string();
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    Some(&turn_id),
                                    &AgentStatus::Running,
                                ).await;
                                current_turn_id = Some(turn_id);
                            }
                            let turn_id = current_turn_id.clone().expect("external turn id");
                            self.persist_external_tool_call_started(thread_id, &turn_id, &call).await;
                            let result = self.dispatch_external_tool_call(thread_id, call).await;
                            self.persist_external_tool_call_completed(
                                thread_id,
                                &turn_id,
                                tool,
                                arguments,
                                &result,
                            ).await;
                            let result_input = external_tool_result_input(&result);
                            if let Err(err) = provider_input.send(result_input) {
                                self.persist_external_error(thread_id, &err).await;
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    current_turn_id.as_deref(),
                                    &AgentStatus::Errored(err.clone()),
                                ).await;
                                return AgentStatus::Errored(err);
                            }
                        }
                        Ok(ExternalProcessEvent::Cli(crate::agent::external::ExternalCliEvent::ToolCallError(result))) => {
                            if current_turn_id.is_none() {
                                let turn_id = uuid::Uuid::new_v4().to_string();
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    Some(&turn_id),
                                    &AgentStatus::Running,
                                ).await;
                                current_turn_id = Some(turn_id);
                            }
                            let turn_id = current_turn_id.clone().expect("external turn id");
                            self.persist_external_tool_call_completed(
                                thread_id,
                                &turn_id,
                                "external_tool".to_string(),
                                serde_json::Value::Null,
                                &result,
                            ).await;
                            let result_input = external_tool_result_input(&result);
                            if let Err(err) = provider_input.send(result_input) {
                                self.persist_external_error(thread_id, &err).await;
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    current_turn_id.as_deref(),
                                    &AgentStatus::Errored(err.clone()),
                                ).await;
                                return AgentStatus::Errored(err);
                            }
                        }
                        Ok(ExternalProcessEvent::Cli(crate::agent::external::ExternalCliEvent::Message(text)))
                        | Ok(ExternalProcessEvent::Cli(crate::agent::external::ExternalCliEvent::Completion(text))) => {
                            if current_turn_id.is_none() {
                                let turn_id = uuid::Uuid::new_v4().to_string();
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    Some(&turn_id),
                                    &AgentStatus::Running,
                                ).await;
                                current_turn_id = Some(turn_id);
                            }
                            let output = bounded_external_output(&text);
                            let status = AgentStatus::Completed(Some(output.clone()));
                            self.persist_external_agent_message(thread_id, &output).await;
                            self.persist_external_terminal_status_with_turn_id(
                                thread_id,
                                current_turn_id.as_deref(),
                                &status,
                            ).await;
                            return status;
                        }
                        Ok(ExternalProcessEvent::Cli(crate::agent::external::ExternalCliEvent::Status(text))) => {
                            if !text.trim().is_empty() {
                                last_status = Some(text);
                            }
                        }
                        Ok(ExternalProcessEvent::StdinError(error)) => {
                            self.persist_external_error(thread_id, &error).await;
                            self.persist_external_terminal_status_with_turn_id(
                                thread_id,
                                current_turn_id.as_deref(),
                                &AgentStatus::Errored(error.clone()),
                            ).await;
                            return AgentStatus::Errored(error);
                        }
                        Ok(ExternalProcessEvent::ProcessExited { success, status }) => {
                            if success {
                                let status = AgentStatus::Completed(last_status);
                                self.persist_external_terminal_status_with_turn_id(
                                    thread_id,
                                    current_turn_id.as_deref(),
                                    &status,
                                ).await;
                                return status;
                            }
                            let error = last_status.unwrap_or_else(|| {
                                    format!("external provider exited with status {status}")
                                });
                            self.persist_external_error(thread_id, &error).await;
                            self.persist_external_terminal_status_with_turn_id(
                                thread_id,
                                current_turn_id.as_deref(),
                                &AgentStatus::Errored(error.clone()),
                            ).await;
                            return AgentStatus::Errored(error);
                        }
                        Err(err) => {
                            self.persist_external_error(thread_id, &err).await;
                            self.persist_external_terminal_status_with_turn_id(
                                thread_id,
                                current_turn_id.as_deref(),
                                &AgentStatus::Errored(err.clone()),
                            ).await;
                            return AgentStatus::Errored(err);
                        }
                    }
                }
            }
        }
    }

    async fn create_external_thread_persistence(
        &self,
        config: &ExternalSpawnConfig,
        thread_id: ThreadId,
        session_source: SessionSource,
        thread_source: ThreadSource,
        agent_metadata: &AgentMetadata,
    ) -> CodexResult<SharedLiveThread> {
        let state = self.upgrade()?;
        state
            .create_external_thread_persistence(
                &config.cwd,
                config.model_provider_id.clone(),
                config.generate_memories,
                thread_id,
                session_source,
                thread_source,
                agent_metadata.clone(),
            )
            .await
    }

    async fn external_live_thread(&self, thread_id: ThreadId) -> Option<SharedLiveThread> {
        self.external_agents
            .get(thread_id)
            .and_then(|run| run.live_thread)
    }

    async fn persist_external_reconnect_descriptor(
        &self,
        thread_id: ThreadId,
        descriptor: ExternalReconnectDescriptor,
    ) -> CodexResult<()> {
        let Some(live_thread) = self.external_live_thread(thread_id).await else {
            return Ok(());
        };
        live_thread.persist().await.map_err(|err| {
            CodexErr::Fatal(format!(
                "failed to persist external thread before reconnect descriptor for {thread_id}: {err}"
            ))
        })?;
        live_thread
            .update_metadata(
                ThreadMetadataPatch {
                    external_reconnect: Some(descriptor),
                    ..Default::default()
                },
                /*include_archived*/ false,
            )
            .await
            .map_err(|err| {
                CodexErr::Fatal(format!(
                    "failed to persist external reconnect descriptor for {thread_id}: {err}"
                ))
            })?;
        Ok(())
    }

    async fn persist_external_items(&self, thread_id: ThreadId, items: Vec<RolloutItem>) {
        let Some(live_thread) = self.external_live_thread(thread_id).await else {
            return;
        };
        if let Err(err) = live_thread.append_items(&items).await {
            warn!("failed to persist external thread items for {thread_id}: {err}");
            return;
        }
        if let Err(err) = live_thread.flush().await {
            warn!("failed to flush external thread items for {thread_id}: {err}");
        }
    }

    async fn persist_external_user_message(&self, thread_id: ThreadId, message: &str) {
        self.persist_external_items(
            thread_id,
            vec![RolloutItem::EventMsg(
                protocol::protocol::EventMsg::UserMessage(UserMessageEvent {
                    message: bounded_external_output(message),
                    images: None,
                    local_images: Vec::new(),
                    skills: Vec::new(),
                    text_elements: Vec::new(),
                }),
            )],
        )
        .await;
    }

    async fn persist_external_agent_message(&self, thread_id: ThreadId, message: &str) {
        self.persist_external_items(
            thread_id,
            vec![RolloutItem::EventMsg(
                protocol::protocol::EventMsg::AgentMessage(AgentMessageEvent {
                    message: bounded_external_output(message),
                    phase: None,
                    memory_citation: None,
                }),
            )],
        )
        .await;
    }

    async fn persist_external_error(&self, thread_id: ThreadId, message: &str) {
        self.persist_external_items(
            thread_id,
            vec![RolloutItem::EventMsg(protocol::protocol::EventMsg::Error(
                ErrorEvent {
                    message: bounded_external_output(message),
                    codex_error_info: None,
                },
            ))],
        )
        .await;
    }

    async fn persist_external_terminal_status(&self, thread_id: ThreadId, status: &AgentStatus) {
        self.persist_external_terminal_status_with_turn_id(thread_id, None, status)
            .await;
    }

    async fn persist_external_terminal_status_with_turn_id(
        &self,
        thread_id: ThreadId,
        turn_id: Option<&str>,
        status: &AgentStatus,
    ) {
        let turn_id = || {
            turn_id
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        };
        let event = match status {
            AgentStatus::Completed(last_agent_message) => {
                protocol::protocol::EventMsg::TurnComplete(TurnCompleteEvent {
                    turn_id: turn_id(),
                    last_agent_message: last_agent_message.clone(),
                    completed_at: Some(Utc::now().timestamp()),
                    duration_ms: None,
                    time_to_first_token_ms: None,
                })
            }
            AgentStatus::Errored(message) => {
                protocol::protocol::EventMsg::ExternalTerminalStatus(ExternalTerminalStatusEvent {
                    thread_id,
                    turn_id: turn_id(),
                    status: ExternalTerminalStatus::Errored,
                    message: Some(bounded_external_output(message)),
                    terminal_at_ms: Utc::now().timestamp_millis(),
                })
            }
            AgentStatus::Shutdown => {
                protocol::protocol::EventMsg::ExternalTerminalStatus(ExternalTerminalStatusEvent {
                    thread_id,
                    turn_id: turn_id(),
                    status: ExternalTerminalStatus::Shutdown,
                    message: None,
                    terminal_at_ms: Utc::now().timestamp_millis(),
                })
            }
            AgentStatus::Interrupted => {
                protocol::protocol::EventMsg::TurnAborted(protocol::protocol::TurnAbortedEvent {
                    turn_id: Some(turn_id()),
                    reason: protocol::protocol::TurnAbortReason::Interrupted,
                    completed_at: Some(Utc::now().timestamp()),
                    duration_ms: None,
                })
            }
            AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::NotFound => {
                protocol::protocol::EventMsg::TurnStarted(TurnStartedEvent {
                    turn_id: turn_id(),
                    started_at: Some(Utc::now().timestamp()),
                    model_context_window: None,
                    collaboration_mode_kind: Default::default(),
                })
            }
        };
        self.persist_external_items(thread_id, vec![RolloutItem::EventMsg(event)])
            .await;
        if is_final(status)
            && let Some(live_thread) = self.external_live_thread(thread_id).await
            && let Err(err) = live_thread.shutdown().await
        {
            warn!("failed to shutdown external thread persistence for {thread_id}: {err}");
        }
    }

    async fn persist_external_tool_call_started(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
        call: &ExternalToolCall,
    ) {
        let event =
            protocol::protocol::EventMsg::ExternalToolCallStarted(ExternalToolCallDisplayEvent {
                thread_id,
                turn_id: turn_id.to_string(),
                id: call.id.clone(),
                tool: external_tool_name(&call.tool),
                arguments: bounded_external_tool_arguments(&call.arguments),
                status: ExternalToolCallStatus::InProgress,
                output: None,
                lifecycle_at_ms: Utc::now().timestamp_millis(),
            });
        self.persist_external_items(thread_id, vec![RolloutItem::EventMsg(event)])
            .await;
    }

    async fn persist_external_tool_call_completed(
        &self,
        thread_id: ThreadId,
        turn_id: &str,
        tool: String,
        arguments: serde_json::Value,
        result: &ExternalToolResult,
    ) {
        let event =
            protocol::protocol::EventMsg::ExternalToolCallCompleted(ExternalToolCallDisplayEvent {
                thread_id,
                turn_id: turn_id.to_string(),
                id: result.id.clone(),
                tool,
                arguments,
                status: if result.ok {
                    ExternalToolCallStatus::Completed
                } else {
                    ExternalToolCallStatus::Failed
                },
                output: Some(bounded_external_tool_result(result)),
                lifecycle_at_ms: Utc::now().timestamp_millis(),
            });
        self.persist_external_items(thread_id, vec![RolloutItem::EventMsg(event)])
            .await;
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
        if let Some(run) = self.external_agents.get(agent_id) {
            let input_sink = run.input_sink.ok_or_else(|| {
                CodexErr::UnsupportedOperation(
                    "external agent is not ready to receive followup input".to_string(),
                )
            })?;
            let last_task_message = communication.content.clone();
            let source = if communication.operation == InterAgentOperation::ChildCompletion {
                ThreadWaitSource::ChildCompletion
            } else {
                ThreadWaitSource::InterAgent
            };
            input_sink.send(communication.content).map_err(|err| {
                CodexErr::UnsupportedOperation(format!(
                    "failed to deliver followup_task to external agent: {err}"
                ))
            })?;
            self.external_agents
                .note_thread_wait_event(agent_id, source);
            self.external_agents
                .update_last_task_message(agent_id, last_task_message);
            return Ok(agent_id.to_string());
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
        runtime: &(impl ThreadLifecycleRuntime + ?Sized),
        result: CodexResult<String>,
    ) -> CodexResult<String> {
        if result
            .as_ref()
            .err()
            .is_some_and(should_release_agent_after_thread_request_error)
        {
            let _ = ThreadLifecycleRuntime::remove_live_thread(runtime, agent_id).await;
            self.state.release_spawned_thread(agent_id);
        }
        result
    }

    /// Submit a shutdown request for a live agent without marking it explicitly closed in
    /// persisted spawn-edge state.
    pub(crate) async fn shutdown_live_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        let state = self.upgrade()?;
        let result = ThreadLifecycleRuntime::shutdown_live_thread(state.as_ref(), agent_id).await;
        let _ = ThreadLifecycleRuntime::remove_live_thread(state.as_ref(), agent_id).await;
        self.state.release_spawned_thread(agent_id);
        result
    }

    /// Mark `agent_id` as explicitly closed in persisted spawn-edge state, then shut down the
    /// agent and any live descendants reached from the in-memory tree.
    pub(crate) async fn close_agent(&self, agent_id: ThreadId) -> CodexResult<String> {
        if self.external_agents.get(agent_id).is_some() {
            let state = self.upgrade().ok();
            if let Some(state) = state.as_ref() {
                if let Some(state_db_ctx) = state.thread_state_runtime()
                    && let Err(err) = state_db_ctx
                        .set_thread_spawn_edge_status(
                            agent_id,
                            DirectionalThreadSpawnEdgeStatus::Closed,
                        )
                        .await
                {
                    warn!(
                        "failed to persist external thread-spawn edge status for {agent_id}: {err}"
                    );
                }
            }
            return Box::pin(self.shutdown_agent_tree(agent_id)).await;
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
        let result = self.shutdown_live_agent_record(agent_id).await;
        for descendant_id in descendant_ids {
            match self.shutdown_live_agent_record(descendant_id).await {
                Ok(_) => {}
                Err(err) if should_ignore_descendant_shutdown_error(&err) => {}
                Err(err) => return Err(err),
            }
        }
        result
    }

    async fn shutdown_live_agent_record(&self, agent_id: ThreadId) -> CodexResult<String> {
        if self.external_agents.get(agent_id).is_some() {
            let shutdown_run = self.external_agents.shutdown_and_remove(agent_id);
            let state = self.upgrade().ok();
            if let Some(state) = state.as_ref() {
                let agent_status = shutdown_run
                    .as_ref()
                    .map(|run| run.status.clone())
                    .unwrap_or(AgentStatus::Shutdown);
                state
                    .update_external_live_thread_status(agent_id, agent_status.clone())
                    .await;
                state.notify_thread_status_changed_with_status(agent_id, Some(agent_status));
            }
            self.persist_external_terminal_status(agent_id, &AgentStatus::Shutdown)
                .await;
            if let Some(state) = state.as_ref() {
                let _ = ThreadLifecycleRuntime::remove_live_thread(state.as_ref(), agent_id).await;
            }
            self.state.release_spawned_thread(agent_id);
            return Ok(agent_id.to_string());
        }

        self.shutdown_live_agent(agent_id).await
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
        ThreadLifecycleRuntime::live_thread_agent_status(state.as_ref(), agent_id)
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
        let Ok(state) = self.upgrade() else {
            return;
        };
        state
            .update_external_live_thread_status(thread_id, run.status.clone())
            .await;
        state.notify_thread_status_changed_with_status(thread_id, Some(run.status.clone()));
        let Some(communication) = completion_communication(&run) else {
            return;
        };
        let parent_thread_id = run.parent_thread_id;
        if self.external_agents.get(parent_thread_id).is_some() {
            if let Err(err) = self
                .send_inter_agent_communication(parent_thread_id, communication)
                .await
            {
                warn!(
                    "failed to notify external parent thread {parent_thread_id} of external agent completion: {err}"
                );
            }
            return;
        }
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

    pub(crate) async fn dispatch_external_tool_call(
        &self,
        sender_thread_id: ThreadId,
        call: ExternalToolCall,
    ) -> ExternalToolResult {
        let sender = {
            let Some(sender_run) = self.external_agents.get(sender_thread_id) else {
                return ExternalToolResult::error(
                    call.id,
                    "agent_not_found",
                    "external sender is not registered",
                );
            };
            external_tool_context(&sender_run)
        };
        match self.dispatch_external_tool_call_inner(sender, &call).await {
            Ok(result) => ExternalToolResult::ok(call.id, result),
            Err(err) => ExternalToolResult::error(call.id, "tool_error", err.to_string()),
        }
    }

    async fn dispatch_external_tool_call_inner(
        &self,
        sender: ExternalToolContext,
        call: &ExternalToolCall,
    ) -> Result<serde_json::Value, FunctionCallError> {
        match call.tool {
            ExternalToolName::ListExternalAgents => {
                let args: ExternalListAgentsArgs = parse_external_arguments(&call.arguments)?;
                let source = if sender.agent_path.is_root() {
                    SessionSource::Unknown
                } else {
                    external_session_source_for(
                        sender.parent_thread_id,
                        sender.depth,
                        sender.agent_path.clone(),
                        sender.provider,
                    )
                };
                let agents = self
                    .list_agents(sender.thread_id, &source, args.path_prefix.as_deref())
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                serde_json::to_value(codex_agent_runtime::ListAgentsToolResult { agents })
                    .map_err(|err| FunctionCallError::Fatal(err.to_string()))
            }
            ExternalToolName::FollowupExternalTask => {
                let args: ExternalFollowupTaskArgs = parse_external_arguments(&call.arguments)?;
                let receiver_thread_id = self
                    .resolve_external_live_target(&sender, &args.target, "follow up to")
                    .await?;
                if sender.agent_path.is_root() && receiver_thread_id == sender.thread_id {
                    return Err(FunctionCallError::RespondToModel(
                        "root external agents cannot follow up to themselves".to_string(),
                    ));
                }
                let receiver_agent = self
                    .state
                    .agent_metadata_for_thread(receiver_thread_id)
                    .or_else(|| {
                        self.external_agents
                            .get(receiver_thread_id)
                            .map(|run| external_metadata(&run))
                    })
                    .unwrap_or_default();
                let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "target agent is missing an agent_path".to_string(),
                    )
                })?;
                let communication = InterAgentCommunication::new(
                    sender.agent_path.clone(),
                    receiver_agent_path,
                    Vec::new(),
                    args.message,
                    InterAgentOperation::FollowupTask,
                )
                .with_thread_ids(sender.thread_id, receiver_thread_id)
                .with_trigger_turn(true);
                self.send_inter_agent_communication(receiver_thread_id, communication)
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                Ok(json!({ "delivered": true }))
            }
            ExternalToolName::SpawnExternalAgent => {
                let args: ExternalSpawnAgentArgs = parse_external_arguments(&call.arguments)?;
                if matches!(args.provider, SpawnAgentProvider::Native) {
                    return Err(FunctionCallError::RespondToModel(
                        "spawn_external_agent requires an external provider".to_string(),
                    ));
                }
                let mut config = sender.spawn_config.clone().ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "external sender cannot spawn children without spawn config".to_string(),
                    )
                })?;
                config.cwd = args.cwd;
                let child_depth = sender.depth.saturating_add(1);
                let spawn_source = thread_spawn_source(
                    sender.thread_id,
                    &external_session_source_for(
                        sender.parent_thread_id,
                        sender.depth,
                        sender.agent_path.clone(),
                        sender.provider,
                    ),
                    child_depth,
                    None,
                    Some(args.task_name),
                )
                .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                let scoped_to_external_root = self
                    .external_agents
                    .get(sender.parent_thread_id)
                    .is_some_and(|run| run.agent_path.is_root());
                if scoped_to_external_root
                    && let Some(agent_path) = spawn_source.get_agent_path()
                    && self.external_agents.list().into_iter().any(|run| {
                        run.parent_thread_id == sender.parent_thread_id
                            && run.agent_path == agent_path
                            && !matches!(run.status, AgentStatus::Shutdown)
                    })
                {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "agent path `{agent_path}` already exists in external root scope"
                    )));
                }
                let spawned = self
                    .spawn_external_agent_with_metadata_sync(
                        config,
                        args.provider,
                        args.message,
                        spawn_source,
                        /*register_global_agent_metadata*/ !scoped_to_external_root,
                        SpawnAgentOptions {
                            fork_parent_spawn_call_id: None,
                            fork_mode: None,
                            environments: None,
                            agent_mode: AgentMode::default(),
                        },
                    )
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                Ok(json!({
                    "task_name": spawned
                        .metadata
                        .agent_path
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| spawned.thread_id.to_string()),
                    "provider": provider_label(args.provider),
                }))
            }
            ExternalToolName::CloseExternalAgent => {
                let args: ExternalCloseAgentArgs = parse_external_arguments(&call.arguments)?;
                let target = self
                    .resolve_external_live_target(&sender, &args.target, "close")
                    .await?;
                if sender.agent_path.is_root() && target == sender.thread_id {
                    return Err(FunctionCallError::RespondToModel(
                        "root external agents cannot close themselves with close_external_agent"
                            .to_string(),
                    ));
                }
                let previous_status = self.get_status(target).await;
                self.close_agent(target)
                    .await
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
                Ok(json!({ "previous_status": previous_status }))
            }
            ExternalToolName::PollExternalEvent => {
                let _args: ExternalPollEventArgs = parse_external_arguments(&call.arguments)?;
                let result = self
                    .external_agents
                    .poll_event(
                        sender.thread_id,
                        thread_service_api::ThreadPollEventRequest {
                            initial_timeout_ms: None,
                            hard_cap_timeout_ms: None,
                        },
                    )
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
                serde_json::to_value(result)
                    .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
            }
        }
    }

    async fn resolve_external_live_target(
        &self,
        sender: &ExternalToolContext,
        target: &str,
        action: &str,
    ) -> Result<ThreadId, FunctionCallError> {
        let agent_path = resolve_agent_reference_path(&sender.agent_path, target)
            .map_err(FunctionCallError::RespondToModel)?;
        if sender.agent_path.is_root() && agent_path.is_root() {
            return Ok(sender.thread_id);
        }

        let current_session_source = external_tool_directory_session_source(sender);
        let resolution = self
            .resolve_agent_reference_in_directory(AgentReferenceResolutionRequest {
                current_thread_id: sender.thread_id,
                current_session_source,
                agent_reference: target.to_string(),
            })
            .await
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
        match resolution {
            AgentReferenceResolution::Live { thread_id } => Ok(thread_id),
            AgentReferenceResolution::PersistedExternalReadOnly { agent_path, .. } => {
                Err(FunctionCallError::RespondToModel(format!(
                    "external agent `{agent_path}` is persisted and read-only; cannot {action} it"
                )))
            }
            AgentReferenceResolution::PersistedNative { agent_path, .. } => {
                Err(FunctionCallError::RespondToModel(format!(
                    "agent `{agent_path}` is persisted and cannot be restored through external tools"
                )))
            }
            AgentReferenceResolution::Unsupported { message, .. } => {
                Err(FunctionCallError::RespondToModel(message))
            }
            AgentReferenceResolution::NotFound { agent_path } => {
                Err(FunctionCallError::RespondToModel(format!(
                    "unknown external agent target `{agent_path}`"
                )))
            }
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

    pub(crate) async fn list_agent_subtree_thread_ids(
        &self,
        agent_id: ThreadId,
    ) -> CodexResult<Vec<ThreadId>> {
        let state = self.upgrade()?;
        let mut subtree_thread_ids = vec![agent_id];
        let mut seen_thread_ids = HashSet::from([agent_id]);

        if let Some(state_db_ctx) = state.thread_state_runtime() {
            for status in [
                DirectionalThreadSpawnEdgeStatus::Open,
                DirectionalThreadSpawnEdgeStatus::Closed,
            ] {
                for descendant_id in state_db_ctx
                    .list_thread_spawn_descendants_with_status(agent_id, status)
                    .await
                    .map_err(|err| {
                        CodexErr::Fatal(format!("failed to load thread-spawn descendants: {err}"))
                    })?
                {
                    if seen_thread_ids.insert(descendant_id) {
                        subtree_thread_ids.push(descendant_id);
                    }
                }
            }
        }

        if let Ok(descendant_ids) = self.live_thread_spawn_descendants(agent_id).await {
            for descendant_id in descendant_ids {
                if seen_thread_ids.insert(descendant_id) {
                    subtree_thread_ids.push(descendant_id);
                }
            }
        }

        Ok(subtree_thread_ids)
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
        let resolution = self
            .resolve_agent_reference_in_directory(AgentReferenceResolutionRequest {
                current_thread_id,
                current_session_source: current_session_source.clone(),
                agent_reference: agent_reference.to_string(),
            })
            .await?;
        match resolution {
            AgentReferenceResolution::Live { thread_id } => Ok(thread_id),
            AgentReferenceResolution::PersistedExternalReadOnly {
                thread_id,
                agent_path,
            } => {
                self.register_persisted_external_agent_metadata(thread_id, agent_path.as_str())
                    .await?;
                Ok(thread_id)
            }
            AgentReferenceResolution::PersistedNative {
                thread_id,
                parent_thread_id,
                depth,
                agent_path,
            } => {
                let Some(config) = config else {
                    return Err(CodexErr::UnsupportedOperation(format!(
                        "agent path `{agent_path}` not found"
                    )));
                };
                let agent_path = AgentPath::try_from(agent_path.as_str()).map_err(|err| {
                    CodexErr::UnsupportedOperation(format!(
                        "agent path `{agent_path}` could not be restored: {err}"
                    ))
                })?;
                let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                    parent_thread_id,
                    depth,
                    agent_path: Some(agent_path.clone()),
                    agent_nickname: None,
                    agent_role: None,
                });
                Box::pin(self.resume_single_agent_from_rollout(config, thread_id, session_source))
                    .await?;
                self.state.agent_id_for_path(&agent_path).ok_or_else(|| {
                    CodexErr::UnsupportedOperation(format!(
                        "agent path `{agent_path}` could not be restored"
                    ))
                })
            }
            AgentReferenceResolution::Unsupported { message, .. } => {
                Err(CodexErr::UnsupportedOperation(message))
            }
            AgentReferenceResolution::NotFound { agent_path } => Err(
                CodexErr::UnsupportedOperation(format!("agent path `{agent_path}` not found")),
            ),
        }
    }

    pub(crate) async fn resolve_agent_reference_in_directory(
        &self,
        request: AgentReferenceResolutionRequest,
    ) -> CodexResult<AgentReferenceResolution> {
        let current_agent_path =
            self.current_agent_path(request.current_thread_id, &request.current_session_source);
        let agent_path =
            resolve_agent_reference_path(&current_agent_path, &request.agent_reference)
                .map_err(CodexErr::UnsupportedOperation)?;
        if let Some(thread_id) = self.state.agent_id_for_path(&agent_path) {
            if self.agent_thread_is_live(thread_id).await {
                return Ok(AgentReferenceResolution::Live { thread_id });
            }
        }
        if let Some(thread_id) = self
            .live_directory_thread_id_for_path(
                request.current_thread_id,
                &request.current_session_source,
                &agent_path,
            )
            .await?
        {
            return Ok(AgentReferenceResolution::Live { thread_id });
        }
        let Some(root_thread_id) = self
            .root_thread_id_for_persisted_agent_lookup(
                request.current_thread_id,
                &request.current_session_source,
            )
            .await
        else {
            return Ok(AgentReferenceResolution::NotFound {
                agent_path: agent_path.to_string(),
            });
        };
        let Some(target) = self
            .persisted_agent_target_for_path(root_thread_id, &agent_path)
            .await?
        else {
            return Ok(AgentReferenceResolution::NotFound {
                agent_path: agent_path.to_string(),
            });
        };
        self.persisted_agent_reference_resolution(&target, &agent_path)
            .await
    }

    async fn live_directory_thread_id_for_path(
        &self,
        current_thread_id: ThreadId,
        current_session_source: &SessionSource,
        agent_path: &AgentPath,
    ) -> CodexResult<Option<ThreadId>> {
        let directory = self
            .list_agent_directory(AgentDirectoryListRequest {
                current_thread_id,
                current_session_source: current_session_source.clone(),
                path_prefix: Some(agent_path.to_string()),
            })
            .await?;
        Ok(directory.entries.into_iter().find_map(|entry| {
            let is_live = matches!(
                entry.source,
                AgentDirectoryEntrySource::NativeLive | AgentDirectoryEntrySource::ExternalLive
            );
            if is_live && entry.agent_path.as_deref() == Some(agent_path.as_str()) {
                Some(entry.thread_id)
            } else {
                None
            }
        }))
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

    async fn persisted_agent_reference_resolution(
        &self,
        target: &PersistedAgentTarget,
        agent_path: &AgentPath,
    ) -> CodexResult<AgentReferenceResolution> {
        let state = self.upgrade()?;
        let stored_thread = state
            .read_stored_thread(ReadThreadParams {
                thread_id: target.thread_id,
                include_archived: true,
                include_history: true,
            })
            .await?;
        let eligibility = external_live_restore_eligibility(&stored_thread);
        if !eligibility.is_external() {
            return Ok(AgentReferenceResolution::PersistedNative {
                thread_id: target.thread_id,
                parent_thread_id: target.parent_thread_id,
                depth: target.depth,
                agent_path: agent_path.to_string(),
            });
        }

        match eligibility {
            ExternalLiveRestoreEligibility::TerminalReadOnly => {}
            ExternalLiveRestoreEligibility::RunningNoDescriptor => {
                return Ok(AgentReferenceResolution::Unsupported {
                    agent_path: agent_path.to_string(),
                    message: format!(
                        "external agent `{}` was interrupted after restart and cannot reconnect to its external provider session because no reconnect descriptor was persisted",
                        agent_path.as_str()
                    ),
                });
            }
            ExternalLiveRestoreEligibility::RunningDescriptorPresentRestoreDisabled {
                provider,
                plan,
            } => {
                let support = external_restore_plan_support(&provider, &plan);
                return Ok(AgentReferenceResolution::Unsupported {
                    agent_path: agent_path.to_string(),
                    message: format!(
                        "external agent `{}` was interrupted after restart; reconnect descriptor is present but external live restore is disabled: {}",
                        agent_path.as_str(),
                        support.diagnostic(),
                    ),
                });
            }
            ExternalLiveRestoreEligibility::RunningReconnectable { .. } => {
                return Ok(AgentReferenceResolution::Unsupported {
                    agent_path: agent_path.to_string(),
                    message: format!(
                        "external agent `{}` has reconnect facts but external live restore is not implemented",
                        agent_path.as_str()
                    ),
                });
            }
            ExternalLiveRestoreEligibility::NotExternal => {
                return Ok(AgentReferenceResolution::PersistedNative {
                    thread_id: target.thread_id,
                    parent_thread_id: target.parent_thread_id,
                    depth: target.depth,
                    agent_path: agent_path.to_string(),
                });
            }
        }

        Ok(AgentReferenceResolution::PersistedExternalReadOnly {
            thread_id: target.thread_id,
            agent_path: agent_path.to_string(),
        })
    }

    async fn register_persisted_external_agent_metadata(
        &self,
        thread_id: ThreadId,
        agent_path: &str,
    ) -> CodexResult<()> {
        let state = self.upgrade()?;
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return Err(CodexErr::UnsupportedOperation(format!(
                "external agent `{agent_path}` is missing persisted agent metadata"
            )));
        };
        let Some(agent_metadata) = self
            .persisted_agent_metadata(thread_id, state_db_ctx.as_ref())
            .await
        else {
            return Err(CodexErr::UnsupportedOperation(format!(
                "external agent `{agent_path}` is missing persisted agent metadata"
            )));
        };
        self.state.register_agent_metadata(agent_metadata);
        Ok(())
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
        ThreadLifecycleRuntime::subscribe_live_thread_status(state.as_ref(), agent_id).await
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
        let directory = self
            .list_agent_directory(AgentDirectoryListRequest {
                current_thread_id,
                current_session_source: current_session_source.clone(),
                path_prefix: path_prefix.map(ToString::to_string),
            })
            .await?;

        Ok(directory
            .entries
            .into_iter()
            .filter_map(|entry| {
                Some(ListedAgent {
                    agent_name: entry.agent_path?,
                    agent_nickname: entry.agent_nickname,
                    agent_role: entry.agent_role,
                    lifecycle_status: entry.lifecycle_status,
                    last_task_message: entry.last_task_message,
                })
            })
            .collect())
    }

    pub(crate) async fn list_agent_directory(
        &self,
        request: AgentDirectoryListRequest,
    ) -> CodexResult<AgentDirectoryListResult> {
        let _state = self.upgrade()?;
        let current_agent_path = self
            .current_agent_path_with_persisted_metadata(
                request.current_thread_id,
                &request.current_session_source,
            )
            .await;
        let root_thread_id =
            if thread_spawn_parent_thread_id(&request.current_session_source).is_none() {
                Some(request.current_thread_id)
            } else {
                self.persisted_thread_spawn_root(request.current_thread_id)
                    .await
                    .or_else(|| self.state.agent_id_for_path(&AgentPath::root()))
            };
        let directory_metadata = self
            .registered_agent_directory_metadata(root_thread_id)
            .await;
        let source_by_thread_id = directory_metadata
            .iter()
            .filter_map(|entry| {
                entry
                    .metadata
                    .agent_id
                    .map(|thread_id| (thread_id, entry.source))
            })
            .collect::<HashMap<_, _>>();
        let tree_facts_by_thread_id = directory_metadata
            .iter()
            .filter_map(|entry| {
                entry
                    .metadata
                    .agent_id
                    .map(|thread_id| (thread_id, (entry.parent_thread_id, entry.depth)))
            })
            .collect::<HashMap<_, _>>();
        let metadata = directory_metadata
            .into_iter()
            .map(|entry| entry.metadata)
            .collect::<Vec<_>>();
        let plan = list_agents_plan(
            &current_agent_path,
            request.path_prefix.as_deref(),
            metadata,
        )
        .map_err(CodexErr::UnsupportedOperation)?;

        let mut entries = Vec::with_capacity(plan.candidates.len().saturating_add(1));
        let root_path = AgentPath::root();
        if plan.include_root
            && let Some(root_thread_id) = self.state.agent_id_for_path(&root_path)
            && let Some(lifecycle_status) = self.listed_thread_lifecycle(root_thread_id).await
        {
            let root_agent = root_listed_agent(lifecycle_status.clone());
            entries.push(AgentDirectoryEntry {
                thread_id: root_thread_id,
                parent_thread_id: None,
                depth: Some(0),
                agent_path: Some(root_agent.agent_name),
                agent_nickname: root_agent.agent_nickname,
                agent_role: root_agent.agent_role,
                last_task_message: root_agent.last_task_message,
                lifecycle_status,
                source: source_by_thread_id
                    .get(&root_thread_id)
                    .copied()
                    .unwrap_or(AgentDirectoryEntrySource::NativeLive),
            });
        }

        for candidate in plan.candidates {
            let lifecycle_status = self.listed_thread_lifecycle(candidate.thread_id).await;
            let Some(lifecycle_status) = lifecycle_status else {
                continue;
            };
            entries.push(AgentDirectoryEntry {
                thread_id: candidate.thread_id,
                parent_thread_id: tree_facts_by_thread_id
                    .get(&candidate.thread_id)
                    .and_then(|(parent_thread_id, _)| *parent_thread_id),
                depth: tree_facts_by_thread_id
                    .get(&candidate.thread_id)
                    .and_then(|(_, depth)| *depth),
                agent_path: Some(candidate.agent_name),
                agent_nickname: candidate.agent_nickname,
                agent_role: candidate.agent_role,
                last_task_message: candidate.last_task_message,
                lifecycle_status,
                source: source_by_thread_id
                    .get(&candidate.thread_id)
                    .copied()
                    .unwrap_or(AgentDirectoryEntrySource::Persisted),
            })
        }

        Ok(AgentDirectoryListResult { entries })
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

    async fn agent_thread_is_live(&self, thread_id: ThreadId) -> bool {
        if self.external_agents.get(thread_id).is_some() {
            return true;
        }
        let Ok(state) = self.upgrade() else {
            return false;
        };
        state
            .live_thread_activity_snapshot(thread_id)
            .await
            .thread_found
    }

    async fn registered_agent_directory_source(
        &self,
        thread_id: Option<ThreadId>,
    ) -> AgentDirectoryEntrySource {
        let Some(thread_id) = thread_id else {
            return AgentDirectoryEntrySource::NativeLive;
        };
        if self.external_agents.get(thread_id).is_some() {
            return AgentDirectoryEntrySource::ExternalLive;
        }
        if self.agent_thread_is_live(thread_id).await {
            AgentDirectoryEntrySource::NativeLive
        } else {
            AgentDirectoryEntrySource::Persisted
        }
    }

    async fn live_agent_directory_tree_facts(
        &self,
        thread_id: ThreadId,
    ) -> (Option<ThreadId>, Option<i32>) {
        let Ok(state) = self.upgrade() else {
            return (None, None);
        };
        let Ok(thread) = state.get_thread(thread_id).await else {
            return (None, None);
        };
        let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id,
            depth,
            ..
        }) = &thread.session_source
        else {
            return (None, None);
        };
        (Some(*parent_thread_id), Some(*depth))
    }

    async fn registered_agent_directory_metadata(
        &self,
        root_thread_id: Option<ThreadId>,
    ) -> Vec<AgentDirectoryMetadata> {
        let mut registered_agents = Vec::new();
        for metadata in self.state.registered_agents() {
            let source = self
                .registered_agent_directory_source(metadata.agent_id)
                .await;
            let (parent_thread_id, depth) = match metadata.agent_id {
                Some(thread_id) => self.live_agent_directory_tree_facts(thread_id).await,
                None => (None, None),
            };
            registered_agents.push(AgentDirectoryMetadata {
                metadata,
                source,
                parent_thread_id,
                depth,
            });
        }
        let Some(root_thread_id) = root_thread_id else {
            return registered_agents;
        };
        let mut scoped_thread_ids = HashSet::from([root_thread_id]);
        if let Ok(descendant_ids) = self.live_thread_spawn_descendants(root_thread_id).await {
            scoped_thread_ids.extend(descendant_ids);
        }
        registered_agents.retain(|metadata| {
            metadata
                .metadata
                .agent_id
                .is_some_and(|thread_id| scoped_thread_ids.contains(&thread_id))
        });
        let mut registered_thread_ids = registered_agents
            .iter()
            .filter_map(|metadata| metadata.metadata.agent_id)
            .collect::<HashSet<_>>();
        let mut registered_agent_paths = registered_agents
            .iter()
            .filter_map(|metadata| {
                metadata
                    .metadata
                    .agent_path
                    .as_ref()
                    .map(ToString::to_string)
            })
            .collect::<HashSet<_>>();
        for run in self.external_agents.list().into_iter().filter(|run| {
            scoped_thread_ids.contains(&run.parent_thread_id)
                || scoped_thread_ids.contains(&run.thread_id)
        }) {
            let metadata = external_metadata(&run);
            if let Some(existing) = registered_agents
                .iter_mut()
                .find(|candidate| candidate.metadata.agent_id == Some(run.thread_id))
            {
                *existing = AgentDirectoryMetadata {
                    metadata,
                    source: AgentDirectoryEntrySource::ExternalLive,
                    parent_thread_id: Some(run.parent_thread_id),
                    depth: Some(run.depth),
                };
                continue;
            }
            if registered_thread_ids.insert(run.thread_id)
                && registered_agent_paths.insert(run.agent_path.to_string())
            {
                registered_agents.push(AgentDirectoryMetadata {
                    metadata,
                    source: AgentDirectoryEntrySource::ExternalLive,
                    parent_thread_id: Some(run.parent_thread_id),
                    depth: Some(run.depth),
                });
            }
        }
        let Ok(state) = self.upgrade() else {
            return registered_agents;
        };
        let Some(state_db_ctx) = state.thread_state_runtime() else {
            return registered_agents;
        };
        let mut queue = VecDeque::from([(root_thread_id, 0)]);
        let mut seen = HashSet::from([root_thread_id]);
        while let Some((parent_thread_id, parent_depth)) = queue.pop_front() {
            let Ok(child_ids) = state_db_ctx
                .list_thread_spawn_children_with_status(
                    parent_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
            else {
                return registered_agents;
            };
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
                if let Some(existing) = registered_agents
                    .iter_mut()
                    .find(|candidate| candidate.metadata.agent_id == Some(child_thread_id))
                {
                    existing.parent_thread_id.get_or_insert(parent_thread_id);
                    existing.depth.get_or_insert(depth);
                    queue.push_back((child_thread_id, depth));
                    continue;
                }
                let Some(agent_metadata) =
                    persisted_agent_metadata_from_state_metadata(child_thread_id, &metadata)
                else {
                    queue.push_back((child_thread_id, depth));
                    continue;
                };
                if agent_metadata
                    .agent_path
                    .as_ref()
                    .is_some_and(AgentPath::is_root)
                {
                    queue.push_back((child_thread_id, depth));
                    continue;
                }
                let Some(agent_path) = agent_metadata.agent_path.as_ref() else {
                    queue.push_back((child_thread_id, depth));
                    continue;
                };
                if !registered_agent_paths.insert(agent_path.to_string()) {
                    queue.push_back((child_thread_id, depth));
                    continue;
                }

                registered_agents.push(AgentDirectoryMetadata {
                    metadata: agent_metadata,
                    source: AgentDirectoryEntrySource::Persisted,
                    parent_thread_id: Some(parent_thread_id),
                    depth: Some(depth),
                });
                registered_thread_ids.insert(child_thread_id);
                queue.push_back((child_thread_id, depth));
            }
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
        let eligibility = external_live_restore_eligibility(&stored_thread);
        let history = stored_thread.history?;
        let latest_status = history
            .items
            .iter()
            .filter_map(|item| match item {
                protocol::protocol::RolloutItem::EventMsg(event) => agent_status_from_event(event),
                _ => None,
            })
            .next_back();
        latest_status
            .clone()
            .filter(is_final)
            .or_else(|| {
                matches!(latest_status, Some(AgentStatus::Interrupted))
                    .then_some(AgentStatus::Interrupted)
            })
            .or_else(|| match eligibility {
                ExternalLiveRestoreEligibility::RunningNoDescriptor
                | ExternalLiveRestoreEligibility::RunningDescriptorPresentRestoreDisabled {
                    ..
                }
                | ExternalLiveRestoreEligibility::RunningReconnectable { .. } => {
                    Some(AgentStatus::Interrupted)
                }
                ExternalLiveRestoreEligibility::NotExternal
                | ExternalLiveRestoreEligibility::TerminalReadOnly => None,
            })
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
        self.prepare_thread_spawn_with_roles(
            reservation,
            &config.agent_roles,
            parent_thread_id,
            depth,
            agent_path,
            agent_role,
            agent_mode,
            preferred_agent_nickname,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_thread_spawn_with_roles(
        &self,
        reservation: &mut SpawnReservation,
        agent_roles: &std::collections::BTreeMap<String, AgentRoleConfig>,
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
        let configured_candidates = resolve_role_config(agent_roles, role_name)
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
