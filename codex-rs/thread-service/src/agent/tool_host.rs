//! Implements the MultiAgentV2 collaboration tool host adapter.

use crate::agent::SpawnAgentOptions;
use crate::agent::agent_resolver::resolve_agent_target;
use crate::agent::exceeds_thread_spawn_depth_limit;
use crate::agent::role::apply_role_to_config;
use crate::agent::tool_support::*;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_agent_roles::DEFAULT_ROLE_NAME;
use codex_agent_runtime::CloseAgentToolResult;
use codex_agent_runtime::ListAgentsToolResult;
use codex_agent_runtime::MultiAgentToolSession;
use codex_agent_runtime::SpawnAgentToolRequest;
use codex_agent_runtime::SpawnAgentToolResult;
use codex_agent_runtime::WaitAgentReason;
use codex_agent_runtime::WaitAgentToolResult;
use codex_agent_runtime::is_final;
use codex_agent_runtime::render_input_preview;
use codex_agent_runtime::wait_agent_result_from_message;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentRef;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabAgentStatusEntry;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::protocol::CollabListAgentsBeginEvent;
use codex_protocol::protocol::CollabListAgentsEndEvent;
use codex_protocol::protocol::CollabListedAgent;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use thread_service_api::PendingInputItem;
use codex_tool_types::FunctionCallError;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::watch;

impl MultiAgentToolSession<Arc<TurnContext>> for Session {
    fn spawn_agent_tool(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: String,
        request: SpawnAgentToolRequest,
    ) -> impl Future<Output = Result<SpawnAgentToolResult, FunctionCallError>> + Send + '_ {
        async move {
            let prompt = message_content(request.message.clone())?;
            self.send_event(
                turn.as_ref(),
                CollabAgentSpawnBeginEvent {
                    call_id: call_id.clone(),
                    started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id: self.thread_id(),
                    sender_agent_path: self.current_agent_path_for_turn(turn.as_ref()).to_string(),
                    prompt,
                    model: request.model.clone().unwrap_or_default(),
                    reasoning_effort: request.reasoning_effort.unwrap_or_default(),
                }
                .into(),
            )
            .await;
            handle_spawn_agent_request(self, Arc::clone(turn), call_id, request).await
        }
    }

    fn followup_task_tool(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: String,
        target: String,
        message: String,
    ) -> impl Future<Output = Result<(), FunctionCallError>> + Send + '_ {
        async move {
            let prompt = message_content(message)?;
            let sender_thread_id = self.thread_id();
            let sender_agent_path = self.current_agent_path_for_turn(turn.as_ref());
            let receiver_thread_id = resolve_agent_target(&self, turn, &target).await?;
            let receiver_agent = self.agent_metadata(receiver_thread_id);
            reject_root_agent(
                receiver_agent.agent_path.as_ref(),
                "Tasks can't be assigned to the root agent",
            )?;
            let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "target agent is missing an agent_path".to_string(),
                )
            })?;
            let receiver_agent_path_string = receiver_agent_path.to_string();
            self.send_event(
                turn.as_ref(),
                CollabAgentInteractionBeginEvent {
                    call_id: call_id.clone(),
                    started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    receiver_thread_id,
                    receiver_agent_path: receiver_agent_path_string.clone(),
                    prompt: prompt.clone(),
                }
                .into(),
            )
            .await;
            let receiver_is_direct_child =
                is_direct_child(&sender_agent_path, &receiver_agent_path);
            let receiver_will_send_completion =
                receiver_agent.agent_mode != crate::agent::AgentMode::Management;
            if receiver_is_direct_child && receiver_will_send_completion {
                self.mark_direct_child_completion_pending(receiver_thread_id)
                    .await;
            }

            let communication = InterAgentCommunication::new(
                sender_agent_path.clone(),
                receiver_agent_path,
                Vec::new(),
                prompt.clone(),
                InterAgentOperation::FollowupTask,
            )
            .with_thread_ids(sender_thread_id, receiver_thread_id);
            let result = self
                .send_inter_agent_communication(
                    receiver_thread_id,
                    communication.with_trigger_turn(true),
                )
                .await
                .map_err(|err| collab_agent_error(receiver_thread_id, err));
            let status = self.agent_status(receiver_thread_id).await;
            self.send_event(
                turn.as_ref(),
                CollabAgentInteractionEndEvent {
                    call_id,
                    completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    receiver_thread_id,
                    receiver_agent_path: receiver_agent_path_string,
                    receiver_agent_nickname: receiver_agent.agent_nickname,
                    receiver_agent_role: receiver_agent.agent_role,
                    prompt,
                    status,
                }
                .into(),
            )
            .await;
            if let Err(err) = result {
                if receiver_is_direct_child
                    && receiver_will_send_completion
                    && self
                        .mark_direct_child_completion_received(receiver_thread_id)
                        .await
                {
                    self.maybe_notify_parent_of_final_status_for_current_source()
                        .await;
                }
                return Err(err);
            }
            Ok(())
        }
    }

    fn wait_agent_tool(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<WaitAgentToolResult, FunctionCallError>> + Send + '_ {
        async move {
            let sender_thread_id = self.thread_id();
            let receiver_thread_id = resolve_agent_target(&self, turn, &target).await?;
            let receiver_agent = self.agent_metadata(receiver_thread_id);
            reject_root_agent(
                receiver_agent.agent_path.as_ref(),
                "root is not a spawned agent",
            )?;
            let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "target agent is missing an agent_path".to_string(),
                )
            })?;
            let sender_agent_path = self.current_agent_path_for_turn(turn.as_ref());
            let (initial_timeout_ms, hard_cap_timeout_ms) = turn.default_wait_agent_timeouts();
            let current_timeout = self
                .wait_agent_current_window(
                    sender_thread_id,
                    receiver_thread_id,
                    initial_timeout_ms,
                    hard_cap_timeout_ms,
                )
                .await;
            self.send_event(
                turn.as_ref(),
                CollabWaitingBeginEvent {
                    call_id: call_id.clone(),
                    started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    receiver_thread_ids: vec![receiver_thread_id],
                    receiver_agents: vec![CollabAgentRef {
                        thread_id: receiver_thread_id,
                        agent_path: Some(receiver_agent_path.to_string()),
                        agent_nickname: receiver_agent.agent_nickname.clone(),
                        agent_role: receiver_agent.agent_role.clone(),
                    }],
                    timeout_ms: duration_to_ms(current_timeout),
                }
                .into(),
            )
            .await;

            let started = Instant::now();
            let mailbox_seq_rx = self.subscribe_mailbox_seq();
            let snapshot_status = self.agent_status(receiver_thread_id).await;
            let agent_name = receiver_agent_path.to_string();
            let current_timeout_ms = duration_to_ms(current_timeout);
            let result = if let Some(message) = self
                .find_pending_input(|item| {
                    matching_communication(item, receiver_thread_id, &receiver_agent_path)
                })
                .await
            {
                build_wait_result(
                    target,
                    agent_name,
                    WaitAgentReason::PendingMessage,
                    message.status.clone().unwrap_or(snapshot_status),
                    Some(message),
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                )
            } else if is_final(&snapshot_status) {
                build_wait_result(
                    target,
                    agent_name,
                    WaitAgentReason::FinalStatus,
                    snapshot_status,
                    None,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                )
            } else {
                let mut status_rx = self
                    .subscribe_agent_status(receiver_thread_id)
                    .await
                    .map_err(|err| collab_agent_error(receiver_thread_id, err))?;
                let initial_status = status_rx.borrow_and_update().clone();
                if let Some(message) = self
                    .find_pending_input(|item| {
                        matching_communication(item, receiver_thread_id, &receiver_agent_path)
                    })
                    .await
                {
                    build_wait_result(
                        target,
                        agent_name,
                        WaitAgentReason::MailboxMessage,
                        message.status.clone().unwrap_or(initial_status),
                        Some(message),
                        started,
                        initial_timeout_ms,
                        current_timeout_ms,
                        hard_cap_timeout_ms,
                    )
                } else if is_final(&initial_status) {
                    build_wait_result(
                        target,
                        agent_name,
                        WaitAgentReason::FinalStatus,
                        initial_status,
                        None,
                        started,
                        initial_timeout_ms,
                        current_timeout_ms,
                        hard_cap_timeout_ms,
                    )
                } else {
                    wait_for_update(
                        &self,
                        receiver_thread_id,
                        receiver_agent_path.clone(),
                        mailbox_seq_rx,
                        status_rx,
                        current_timeout,
                        WaitAgentContext {
                            target,
                            agent_name,
                            started,
                            initial_timeout_ms,
                            current_timeout_ms,
                            hard_cap_timeout_ms,
                        },
                    )
                    .await
                }
            };
            if result.timed_out {
                self.advance_wait_agent_backoff(sender_thread_id, receiver_thread_id)
                    .await;
            } else {
                self.reset_wait_agent_backoff(sender_thread_id, receiver_thread_id)
                    .await;
            }

            let mut statuses = HashMap::new();
            statuses.insert(receiver_thread_id, result.status.clone());
            self.send_event(
                turn.as_ref(),
                CollabWaitingEndEvent {
                    call_id,
                    completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    timeout_ms: duration_to_ms(current_timeout),
                    agent_statuses: vec![CollabAgentStatusEntry {
                        thread_id: receiver_thread_id,
                        agent_path: Some(receiver_agent_path.to_string()),
                        agent_nickname: receiver_agent.agent_nickname,
                        agent_role: receiver_agent.agent_role,
                        status: result.status.clone(),
                    }],
                    statuses,
                }
                .into(),
            )
            .await;

            Ok(result)
        }
    }

    fn close_agent_tool(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: String,
        target: String,
    ) -> impl Future<Output = Result<CloseAgentToolResult, FunctionCallError>> + Send + '_ {
        async move {
            let sender_thread_id = self.thread_id();
            let sender_agent_path = self.current_agent_path_for_turn(turn.as_ref());
            let agent_id = resolve_agent_target(&self, turn, &target).await?;
            let receiver_agent = self.agent_metadata(agent_id);
            reject_root_agent(
                receiver_agent.agent_path.as_ref(),
                "root is not a spawned agent",
            )?;
            let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "target agent is missing an agent_path".to_string(),
                )
            })?;
            let receiver_is_direct_child =
                is_direct_child(&sender_agent_path, &receiver_agent_path);
            self.send_event(
                turn.as_ref(),
                CollabCloseBeginEvent {
                    call_id: call_id.clone(),
                    started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    receiver_thread_id: agent_id,
                    receiver_agent_path: receiver_agent_path.to_string(),
                }
                .into(),
            )
            .await;
            let status = self.agent_status(agent_id).await;
            let result = self
                .close_agent(agent_id)
                .await
                .map_err(|err| collab_agent_error(agent_id, err));
            self.send_event(
                turn.as_ref(),
                CollabCloseEndEvent {
                    call_id,
                    completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.to_string(),
                    receiver_thread_id: agent_id,
                    receiver_agent_path: receiver_agent_path.to_string(),
                    receiver_agent_nickname: receiver_agent.agent_nickname,
                    receiver_agent_role: receiver_agent.agent_role,
                    status: status.clone(),
                }
                .into(),
            )
            .await;
            result?;
            if receiver_is_direct_child
                && self.clear_direct_child_completion_pending(agent_id).await
            {
                self.maybe_notify_parent_of_final_status_for_current_source()
                    .await;
            }

            Ok(CloseAgentToolResult {
                previous_status: status,
            })
        }
    }

    fn list_agents_tool(
        self: Arc<Self>,
        turn: &Arc<TurnContext>,
        call_id: String,
        path_prefix: Option<String>,
    ) -> impl Future<Output = Result<ListAgentsToolResult, FunctionCallError>> + Send + '_ {
        async move {
            let sender_thread_id = self.thread_id();
            let sender_agent_path = self.current_agent_path_for_turn(turn.as_ref()).to_string();
            self.send_event(
                turn.as_ref(),
                CollabListAgentsBeginEvent {
                    call_id: call_id.clone(),
                    started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path: sender_agent_path.clone(),
                    path_prefix: path_prefix.clone(),
                }
                .into(),
            )
            .await;
            self.register_session_root_for_turn(turn.as_ref());
            let agents = self
                .list_agents_for_turn(turn.as_ref(), path_prefix.as_deref())
                .await
                .map_err(collab_spawn_error);
            let listed_agents = agents.as_ref().map_or_else(
                |_| Vec::new(),
                |agents| {
                    agents
                        .iter()
                        .map(|agent| CollabListedAgent {
                            agent_path: agent.agent_name.clone(),
                            status: agent.agent_status.clone(),
                            last_task_message: agent.last_task_message.clone(),
                        })
                        .collect()
                },
            );
            self.send_event(
                turn.as_ref(),
                CollabListAgentsEndEvent {
                    call_id,
                    completed_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                    sender_thread_id,
                    sender_agent_path,
                    path_prefix,
                    success: agents.is_ok(),
                    agents: listed_agents,
                }
                .into(),
            )
            .await;

            Ok(ListAgentsToolResult { agents: agents? })
        }
    }
}

fn matching_communication(
    item: &PendingInputItem,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
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
    .then_some(communication.clone())
}

struct WaitAgentContext {
    target: String,
    agent_name: String,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
}

async fn wait_for_update(
    session: &Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: AgentPath,
    mut mailbox_seq_rx: watch::Receiver<u64>,
    mut status_rx: watch::Receiver<crate::agent::AgentStatus>,
    current_timeout: Duration,
    wait_context: WaitAgentContext,
) -> WaitAgentToolResult {
    if let Some(result) = pending_message_result(
        session,
        receiver_thread_id,
        &receiver_agent_path,
        &mut status_rx,
        &wait_context,
    )
    .await
    {
        return result;
    }
    let deadline = Instant::now() + current_timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            let status = session.agent_status(receiver_thread_id).await;
            return build_wait_result(
                wait_context.target,
                wait_context.agent_name,
                WaitAgentReason::Timeout,
                status,
                None,
                wait_context.started,
                wait_context.initial_timeout_ms,
                wait_context.current_timeout_ms,
                wait_context.hard_cap_timeout_ms,
            );
        }
        let event = tokio::time::timeout(deadline.saturating_duration_since(now), async {
            tokio::select! {
                status_changed = status_rx.changed() => {
                    WaitEvent::Status(status_changed.is_ok())
                }
                mailbox_changed = mailbox_seq_rx.changed() => {
                    WaitEvent::Mailbox(mailbox_changed.is_ok())
                }
            }
        })
        .await;

        match event {
            Ok(WaitEvent::Status(true)) => {
                let status = status_rx.borrow_and_update().clone();
                if let Some(result) = pending_message_result_with_fallback_status(
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    status.clone(),
                    &wait_context,
                )
                .await
                {
                    return result;
                }
                let reason = if is_final(&status) {
                    WaitAgentReason::FinalStatus
                } else {
                    WaitAgentReason::StatusUpdate
                };
                return build_wait_result(
                    wait_context.target,
                    wait_context.agent_name,
                    reason,
                    status,
                    None,
                    wait_context.started,
                    wait_context.initial_timeout_ms,
                    wait_context.current_timeout_ms,
                    wait_context.hard_cap_timeout_ms,
                );
            }
            Ok(WaitEvent::Status(false)) => {
                let status = session.agent_status(receiver_thread_id).await;
                if let Some(result) = pending_message_result_with_fallback_status(
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    status.clone(),
                    &wait_context,
                )
                .await
                {
                    return result;
                }
                return build_wait_result(
                    wait_context.target,
                    wait_context.agent_name,
                    WaitAgentReason::StatusUpdate,
                    status,
                    None,
                    wait_context.started,
                    wait_context.initial_timeout_ms,
                    wait_context.current_timeout_ms,
                    wait_context.hard_cap_timeout_ms,
                );
            }
            Ok(WaitEvent::Mailbox(true)) => {
                if let Some(result) = pending_message_result(
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    &mut status_rx,
                    &wait_context,
                )
                .await
                {
                    return result;
                }
            }
            Ok(WaitEvent::Mailbox(false)) | Err(_) => {
                let status = session.agent_status(receiver_thread_id).await;
                return build_wait_result(
                    wait_context.target,
                    wait_context.agent_name,
                    WaitAgentReason::Timeout,
                    status,
                    None,
                    wait_context.started,
                    wait_context.initial_timeout_ms,
                    wait_context.current_timeout_ms,
                    wait_context.hard_cap_timeout_ms,
                );
            }
        }
    }
}

async fn pending_message_result(
    session: &Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    status_rx: &mut watch::Receiver<crate::agent::AgentStatus>,
    wait_context: &WaitAgentContext,
) -> Option<WaitAgentToolResult> {
    let message = session
        .find_pending_input(|item| {
            matching_communication(item, receiver_thread_id, receiver_agent_path)
        })
        .await?;
    let fallback_status = status_rx.borrow().clone();
    Some(build_pending_message_result(
        &wait_context.target,
        &wait_context.agent_name,
        fallback_status,
        message,
        wait_context.started,
        wait_context.initial_timeout_ms,
        wait_context.current_timeout_ms,
        wait_context.hard_cap_timeout_ms,
    ))
}

async fn pending_message_result_with_fallback_status(
    session: &Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    fallback_status: crate::agent::AgentStatus,
    wait_context: &WaitAgentContext,
) -> Option<WaitAgentToolResult> {
    let message = session
        .find_pending_input(|item| {
            matching_communication(item, receiver_thread_id, receiver_agent_path)
        })
        .await?;
    Some(build_pending_message_result(
        &wait_context.target,
        &wait_context.agent_name,
        fallback_status,
        message,
        wait_context.started,
        wait_context.initial_timeout_ms,
        wait_context.current_timeout_ms,
        wait_context.hard_cap_timeout_ms,
    ))
}

enum WaitEvent {
    Status(bool),
    Mailbox(bool),
}

fn build_wait_result(
    target: String,
    agent_name: String,
    reason: WaitAgentReason,
    status: crate::agent::AgentStatus,
    message: Option<InterAgentCommunication>,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult {
    wait_agent_result_from_message(
        target,
        agent_name,
        reason,
        status,
        message,
        started.elapsed().as_millis() as i64,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    )
}

fn build_pending_message_result(
    target: &str,
    agent_name: &str,
    fallback_status: crate::agent::AgentStatus,
    message: InterAgentCommunication,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult {
    let status = message.status.clone().unwrap_or(fallback_status);
    build_wait_result(
        target.to_string(),
        agent_name.to_string(),
        WaitAgentReason::MailboxMessage,
        status,
        Some(message),
        started,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    )
}

fn duration_to_ms(duration: Duration) -> i64 {
    duration.as_millis() as i64
}

fn reject_root_agent(
    agent_path: Option<&AgentPath>,
    message: &str,
) -> Result<(), FunctionCallError> {
    if agent_path.is_some_and(AgentPath::is_root) {
        return Err(FunctionCallError::RespondToModel(message.to_string()));
    }
    Ok(())
}

fn is_direct_child(sender_agent_path: &AgentPath, receiver_agent_path: &AgentPath) -> bool {
    receiver_agent_path
        .as_str()
        .rsplit_once('/')
        .is_some_and(|(parent, _)| parent == sender_agent_path.as_str())
}

fn message_content(message: String) -> Result<String, FunctionCallError> {
    if message.trim().is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "Empty message can't be sent to an agent".to_string(),
        ));
    }
    Ok(message)
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
    let child_depth = turn.next_child_spawn_depth();
    let agent_max_depth = turn.agent_max_depth();
    if exceeds_thread_spawn_depth_limit(child_depth, agent_max_depth) {
        return Err(FunctionCallError::RespondToModel(format!(
            "agent depth limit reached: cannot spawn depth {child_depth}; configured agents.max_depth is {}",
            agent_max_depth
        )));
    }
    let current_agent_path = session.current_agent_path_for_turn(turn.as_ref());
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
        turn.service_tier(),
        request.service_tier.as_deref(),
    )
    .await?;

    let spawn_source = thread_spawn_source(
        session.thread_id(),
        &SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: session.thread_id(),
            depth: child_depth.saturating_sub(1),
            agent_path: Some(current_agent_path.clone()),
            agent_nickname: None,
            agent_role: None,
        }),
        child_depth,
        role_name,
        Some(request.task_name.clone()),
    )?;
    let result = Box::pin(session.spawn_agent_with_metadata(
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
            environments: Some(turn.spawn_agent_environment_selections(request.cwd.as_ref())),
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
        Some(thread_id) => session.agent_config_snapshot(thread_id).await,
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
                sender_thread_id: session.thread_id(),
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
    turn.record_multi_agent_spawn_metric(role_tag);
    let task_name = new_agent_path.ok_or_else(|| {
        FunctionCallError::RespondToModel(
            "spawned agent is missing a canonical task name".to_string(),
        )
    })?;

    if turn.hide_spawn_agent_metadata() {
        Ok(SpawnAgentToolResult::HiddenMetadata { task_name })
    } else {
        Ok(SpawnAgentToolResult::WithNickname {
            task_name,
            nickname,
        })
    }
}
