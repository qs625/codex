//! Implements the MultiAgentV2 collaboration tool host adapter.

use crate::agent::SpawnAgentOptions;
use crate::agent::agent_resolver::resolve_agent_target;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::next_thread_spawn_depth;
use crate::agent::role::apply_role_to_config;
use crate::agent::tool_support::*;
use crate::function_tool::FunctionCallError;
use crate::pending_input::PendingInputItem;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::handlers::CoreToolDomainHost;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_runtime::AgentMetadata;
use codex_agent_runtime::ListedAgent;
use codex_agent_runtime::render_input_preview;
use codex_protocol::ThreadId;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tool_runtime_api::MultiAgentToolHost;
use codex_tool_runtime_api::SpawnAgentToolRequest;
use codex_tool_runtime_api::SpawnAgentToolResult;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

impl MultiAgentToolHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = crate::tools::context::SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    fn thread_id(&self, session: &Self::Session) -> ThreadId {
        session.conversation_id
    }

    fn sender_agent_path(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> codex_protocol::AgentPath {
        session
            .services
            .agent_control
            .current_agent_path(session.conversation_id, &turn.session_source)
    }

    async fn send_collab_event(&self, session: &Self::Session, turn: &Self::Turn, event: EventMsg) {
        session.send_event(turn.as_ref(), event).await;
    }

    async fn resolve_agent_target(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        target: &str,
    ) -> Result<ThreadId, FunctionCallError> {
        resolve_agent_target(session, turn, target).await
    }

    fn agent_metadata(&self, session: &Self::Session, thread_id: ThreadId) -> AgentMetadata {
        session
            .services
            .agent_control
            .get_agent_metadata(thread_id)
            .unwrap_or_default()
    }

    async fn agent_status(
        &self,
        session: &Self::Session,
        thread_id: ThreadId,
    ) -> crate::agent::AgentStatus {
        session.services.agent_control.get_status(thread_id).await
    }

    async fn subscribe_agent_status(
        &self,
        session: &Self::Session,
        thread_id: ThreadId,
    ) -> Result<watch::Receiver<crate::agent::AgentStatus>, FunctionCallError> {
        session
            .services
            .agent_control
            .subscribe_status(thread_id)
            .await
            .map_err(|err| collab_agent_error(thread_id, err))
    }

    fn subscribe_mailbox_seq(&self, session: &Self::Session) -> watch::Receiver<u64> {
        session.subscribe_mailbox_seq()
    }

    async fn find_pending_inter_agent_communication(
        &self,
        session: &Self::Session,
        receiver_thread_id: ThreadId,
        receiver_agent_path: &codex_protocol::AgentPath,
    ) -> Option<InterAgentCommunication> {
        session
            .find_pending_input(|item| {
                matching_communication(item, receiver_thread_id, receiver_agent_path)
            })
            .await
    }

    async fn wait_agent_current_window(
        &self,
        session: &Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> Duration {
        session
            .wait_agent_current_window(
                sender_thread_id,
                receiver_thread_id,
                initial_timeout_ms,
                hard_cap_timeout_ms,
            )
            .await
    }

    async fn advance_wait_agent_backoff(
        &self,
        session: &Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) {
        session
            .advance_wait_agent_backoff(sender_thread_id, receiver_thread_id)
            .await;
    }

    async fn reset_wait_agent_backoff(
        &self,
        session: &Self::Session,
        sender_thread_id: ThreadId,
        receiver_thread_id: ThreadId,
    ) {
        session
            .reset_wait_agent_backoff(sender_thread_id, receiver_thread_id)
            .await;
    }

    fn wait_agent_timeouts(&self, turn: &Self::Turn) -> (i64, i64) {
        (
            turn.config.multi_agent_v2.default_wait_timeout_ms,
            turn.config.multi_agent_v2.max_wait_timeout_ms,
        )
    }

    fn register_session_root(&self, session: &Self::Session, turn: &Self::Turn) {
        session
            .services
            .agent_control
            .register_session_root(session.conversation_id, &turn.session_source);
    }

    async fn list_agents(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        path_prefix: Option<&str>,
    ) -> Result<Vec<ListedAgent>, FunctionCallError> {
        session
            .services
            .agent_control
            .list_agents(session.conversation_id, &turn.session_source, path_prefix)
            .await
            .map_err(collab_spawn_error)
    }

    async fn send_followup_task(
        &self,
        session: &Self::Session,
        sender_agent_path: codex_protocol::AgentPath,
        receiver_thread_id: ThreadId,
        receiver_agent_path: codex_protocol::AgentPath,
        prompt: String,
    ) -> Result<(), FunctionCallError> {
        let communication = InterAgentCommunication::new(
            sender_agent_path,
            receiver_agent_path,
            Vec::new(),
            prompt,
            InterAgentOperation::FollowupTask,
        )
        .with_thread_ids(session.conversation_id, receiver_thread_id);
        session
            .services
            .agent_control
            .send_inter_agent_communication(
                receiver_thread_id,
                communication.with_trigger_turn(true),
            )
            .await
            .map(|_| ())
            .map_err(|err| collab_agent_error(receiver_thread_id, err))
    }

    async fn mark_direct_child_completion_pending(
        &self,
        session: &Self::Session,
        receiver_thread_id: ThreadId,
    ) {
        session
            .mark_direct_child_completion_pending(receiver_thread_id)
            .await;
    }

    async fn mark_direct_child_completion_received(
        &self,
        session: &Self::Session,
        receiver_thread_id: ThreadId,
    ) -> bool {
        session
            .mark_direct_child_completion_received(receiver_thread_id)
            .await
    }

    async fn clear_direct_child_completion_pending(
        &self,
        session: &Self::Session,
        receiver_thread_id: ThreadId,
    ) -> bool {
        session
            .clear_direct_child_completion_pending(receiver_thread_id)
            .await
    }

    async fn maybe_notify_parent_of_final_status(&self, session: &Self::Session) {
        session
            .maybe_notify_parent_of_final_status_for_current_source()
            .await;
    }

    async fn close_agent(
        &self,
        session: &Self::Session,
        thread_id: ThreadId,
    ) -> Result<(), FunctionCallError> {
        session
            .services
            .agent_control
            .close_agent(thread_id)
            .await
            .map_err(|err| collab_agent_error(thread_id, err))
            .map(|_| ())
    }

    async fn spawn_agent(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: &str,
        request: SpawnAgentToolRequest,
    ) -> Result<SpawnAgentToolResult, FunctionCallError> {
        handle_spawn_agent_request(session.clone(), turn.clone(), call_id.to_string(), request)
            .await
    }
}

fn matching_communication(
    item: &PendingInputItem,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &codex_protocol::AgentPath,
) -> Option<InterAgentCommunication> {
    let communication = match item {
        PendingInputItem::InterAgentCommunication(communication) => communication,
        PendingInputItem::ResponseItem(ResponseItem::InterAgentCommunication {
            communication,
            ..
        })
        | PendingInputItem::HookInspectable(ResponseItem::InterAgentCommunication {
            communication,
            ..
        }) => communication,
        PendingInputItem::ResponseItem(_) | PendingInputItem::HookInspectable(_) => return None,
    };
    (communication.author == *receiver_agent_path
        || communication.sender_thread_id == Some(receiver_thread_id))
    .then(|| communication.clone())
}

async fn handle_spawn_agent_request(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    call_id: String,
    request: SpawnAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError> {
    let role_name = request
        .agent_type
        .as_deref()
        .map(str::trim)
        .filter(|role| !role.is_empty());
    let initial_operation = parse_collab_input(Some(request.message.clone()), /*items*/ None)?;
    let prompt = render_input_preview(&initial_operation);
    let session_source = turn.session_source.clone();
    let child_depth = next_thread_spawn_depth(&session_source);
    if exceeds_thread_spawn_depth_limit(child_depth, turn.config.agent_max_depth) {
        return Err(FunctionCallError::RespondToModel(format!(
            "agent depth limit reached: cannot spawn depth {child_depth}; configured agents.max_depth is {}",
            turn.config.agent_max_depth
        )));
    }
    let current_agent_path = session
        .services
        .agent_control
        .current_agent_path(session.conversation_id, &turn.session_source);
    let mut config = build_agent_spawn_config(
        &session.get_base_instructions().await,
        turn.as_ref(),
        request.cwd.clone(),
    )?;
    if matches!(
        request.fork_mode,
        Some(codex_agent_runtime::SpawnAgentForkMode::FullHistory)
    ) {
        reject_full_fork_spawn_overrides(
            role_name,
            request.model.as_deref(),
            request.reasoning_effort,
        )?;
    } else {
        apply_requested_spawn_agent_model_overrides(
            &session,
            turn.as_ref(),
            &mut config,
            request.model.as_deref(),
            request.reasoning_effort,
        )
        .await?;
        refresh_spawn_cwd_agent_roles(&mut config).await?;
        apply_role_to_config(&mut config, role_name)
            .await
            .map_err(FunctionCallError::RespondToModel)?;
    }
    apply_spawn_agent_service_tier(
        &session,
        &mut config,
        turn.config.service_tier.as_deref(),
        request.service_tier.as_deref(),
    )
    .await?;

    let spawn_source = thread_spawn_source(
        session.conversation_id,
        &SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: session.conversation_id,
            depth: child_depth.saturating_sub(1),
            agent_path: Some(current_agent_path.clone()),
            agent_nickname: None,
            agent_role: None,
        }),
        child_depth,
        role_name,
        Some(request.task_name.clone()),
    )?;
    let result = Box::pin(session.services.agent_control.spawn_agent_with_metadata(
        config,
        match (spawn_source.get_agent_path(), initial_operation) {
            (Some(recipient), Op::UserInput { items, .. })
                if items.iter().all(|item| {
                    matches!(item, codex_protocol::user_input::UserInput::Text { .. })
                }) =>
            {
                Op::InterAgentCommunication {
                    communication: InterAgentCommunication::new(
                        current_agent_path.clone(),
                        recipient,
                        Vec::new(),
                        prompt.clone(),
                        InterAgentOperation::SpawnAgent,
                    ),
                }
            }
            (_, initial_operation) => initial_operation,
        },
        Some(spawn_source),
        SpawnAgentOptions {
            fork_parent_spawn_call_id: request.fork_mode.as_ref().map(|_| call_id.clone()),
            fork_mode: request.fork_mode,
            environments: Some(spawn_agent_environment_selections(
                turn.as_ref(),
                request.cwd.as_ref(),
            )),
            agent_mode: request.agent_mode.unwrap_or_default(),
        },
    ))
    .await
    .map_err(collab_spawn_error);
    let (new_thread_id, new_agent_metadata, status) = match &result {
        Ok(spawned_agent) => (
            Some(spawned_agent.thread_id),
            Some(spawned_agent.metadata.clone()),
            spawned_agent.status.clone(),
        ),
        Err(_) => (None, None, crate::agent::AgentStatus::NotFound),
    };
    let agent_snapshot = match new_thread_id {
        Some(thread_id) => {
            session
                .services
                .agent_control
                .get_agent_config_snapshot(thread_id)
                .await
        }
        None => None,
    };
    let (new_agent_path, new_agent_nickname, new_agent_role) =
        match (&agent_snapshot, new_agent_metadata) {
            (Some(snapshot), _) => (
                snapshot.session_source.get_agent_path().map(String::from),
                snapshot.session_source.get_nickname(),
                snapshot.session_source.get_agent_role(),
            ),
            (None, Some(metadata)) => (
                metadata.agent_path.map(String::from),
                metadata.agent_nickname,
                metadata.agent_role,
            ),
            (None, None) => (None, None, None),
        };
    let effective_model = agent_snapshot
        .as_ref()
        .map(|snapshot| snapshot.model.clone())
        .unwrap_or_else(|| request.model.clone().unwrap_or_default());
    let effective_reasoning_effort = agent_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.reasoning_effort)
        .unwrap_or(request.reasoning_effort.unwrap_or_default());
    let nickname = new_agent_nickname.clone();
    session
        .send_event(
            &turn,
            CollabAgentSpawnEndEvent {
                call_id,
                completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: current_agent_path.to_string(),
                new_thread_id,
                new_agent_path: new_agent_path.clone(),
                new_agent_nickname,
                new_agent_role,
                prompt,
                model: effective_model,
                reasoning_effort: effective_reasoning_effort,
                status,
            }
            .into(),
        )
        .await;
    let _ = result?;
    let role_tag = role_name.unwrap_or(DEFAULT_ROLE_NAME);
    turn.session_telemetry.counter(
        "codex.multi_agent.spawn",
        /*inc*/ 1,
        &[("role", role_tag)],
    );
    let task_name = new_agent_path.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawned agent is missing a canonical task name".to_string(),
        )
    })?;

    if turn.config.multi_agent_v2.hide_spawn_agent_metadata {
        Ok(SpawnAgentToolResult::HiddenMetadata { task_name })
    } else {
        Ok(SpawnAgentToolResult::WithNickname {
            task_name,
            nickname,
        })
    }
}
