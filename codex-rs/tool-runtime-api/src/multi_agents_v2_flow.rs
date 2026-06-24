use crate::MultiAgentToolHost;
use crate::SpawnAgentToolRequest;
use crate::SpawnAgentToolResult;
use crate::WaitAgentReason;
use crate::WaitAgentToolResult;
use codex_agent_runtime::AgentMode;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::is_final;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentRef;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentStatusEntry;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

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

pub fn spawn_agent_request_from_arguments(
    arguments: &str,
) -> Result<SpawnAgentToolRequest, FunctionCallError> {
    let args: SpawnAgentArgs = parse_arguments_with_base_path(arguments, None)?;
    args.into_request()
}

pub fn followup_task_from_arguments(
    arguments: &str,
) -> Result<(String, String), FunctionCallError> {
    let args: FollowupTaskArgs = parse_arguments(arguments)?;
    Ok((args.target, args.message))
}

pub fn wait_agent_target_from_arguments(arguments: &str) -> Result<String, FunctionCallError> {
    let args: WaitAgentArgs = parse_arguments(arguments)?;
    Ok(args.target)
}

pub async fn run_spawn_agent_tool<Host>(
    host: &Host,
    session: Host::Session,
    turn: Host::Turn,
    call_id: String,
    request: SpawnAgentToolRequest,
) -> Result<SpawnAgentToolResult, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let prompt = message_content(request.message.clone())?;
    host.send_collab_event(
        &session,
        &turn,
        CollabAgentSpawnBeginEvent {
            call_id: call_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            sender_thread_id: host.thread_id(&session),
            sender_agent_path: host.sender_agent_path(&session, &turn).to_string(),
            prompt,
            model: request.model.clone().unwrap_or_default(),
            reasoning_effort: request.reasoning_effort.unwrap_or_default(),
        }
        .into(),
    )
    .await;

    host.spawn_agent(&session, &turn, &call_id, request).await
}

pub async fn run_followup_task_tool<Host>(
    host: &Host,
    session: Host::Session,
    turn: Host::Turn,
    call_id: String,
    target: String,
    message: String,
) -> Result<(), FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let prompt = message_content(message)?;
    let sender_thread_id = host.thread_id(&session);
    let sender_agent_path = host.sender_agent_path(&session, &turn);
    let receiver_thread_id = host.resolve_agent_target(&session, &turn, &target).await?;
    let receiver_agent = host.agent_metadata(&session, receiver_thread_id);
    reject_root_agent(
        receiver_agent.agent_path.as_ref(),
        "Tasks can't be assigned to the root agent",
    )?;
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let receiver_agent_path_string = receiver_agent_path.to_string();
    host.send_collab_event(
        &session,
        &turn,
        CollabAgentInteractionBeginEvent {
            call_id: call_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.to_string(),
            receiver_thread_id,
            receiver_agent_path: receiver_agent_path_string.clone(),
            prompt: prompt.clone(),
        }
        .into(),
    )
    .await;
    let receiver_is_direct_child = is_direct_child(&sender_agent_path, &receiver_agent_path);
    let receiver_will_send_completion = receiver_agent.agent_mode != AgentMode::Management;
    if receiver_is_direct_child && receiver_will_send_completion {
        host.mark_direct_child_completion_pending(&session, receiver_thread_id)
            .await;
    }

    let result = host
        .send_followup_task(
            &session,
            sender_agent_path.clone(),
            receiver_thread_id,
            receiver_agent_path.clone(),
            prompt.clone(),
        )
        .await;
    let status = host.agent_status(&session, receiver_thread_id).await;
    host.send_collab_event(
        &session,
        &turn,
        CollabAgentInteractionEndEvent {
            call_id,
            completed_at_ms: now_unix_timestamp_ms(),
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
            && host
                .mark_direct_child_completion_received(&session, receiver_thread_id)
                .await
        {
            host.maybe_notify_parent_of_final_status(&session).await;
        }
        return Err(err);
    }
    Ok(())
}

pub async fn run_wait_agent_tool<Host>(
    host: &Host,
    session: Host::Session,
    turn: Host::Turn,
    call_id: String,
    target: String,
) -> Result<WaitAgentToolResult, FunctionCallError>
where
    Host: MultiAgentToolHost,
{
    let sender_thread_id = host.thread_id(&session);
    let receiver_thread_id = host.resolve_agent_target(&session, &turn, &target).await?;
    let receiver_agent = host.agent_metadata(&session, receiver_thread_id);
    reject_root_agent(
        receiver_agent.agent_path.as_ref(),
        "root is not a spawned agent",
    )?;
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let sender_agent_path = host.sender_agent_path(&session, &turn);
    let (initial_timeout_ms, hard_cap_timeout_ms) = host.wait_agent_timeouts(&turn);
    let current_timeout = host
        .wait_agent_current_window(
            &session,
            sender_thread_id,
            receiver_thread_id,
            initial_timeout_ms,
            hard_cap_timeout_ms,
        )
        .await;
    let current_timeout_ms = duration_to_ms(current_timeout);
    host.send_collab_event(
        &session,
        &turn,
        CollabWaitingBeginEvent {
            call_id: call_id.clone(),
            started_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.to_string(),
            receiver_thread_ids: vec![receiver_thread_id],
            receiver_agents: vec![CollabAgentRef {
                thread_id: receiver_thread_id,
                agent_path: Some(receiver_agent_path.to_string()),
                agent_nickname: receiver_agent.agent_nickname.clone(),
                agent_role: receiver_agent.agent_role.clone(),
            }],
            timeout_ms: wait_lifecycle_timeout_ms(current_timeout),
        }
        .into(),
    )
    .await;

    let started = Instant::now();
    let mailbox_seq_rx = host.subscribe_mailbox_seq(&session);
    let snapshot_status = host.agent_status(&session, receiver_thread_id).await;
    let agent_name = receiver_agent_path.to_string();
    let result = if let Some(message) = host
        .find_pending_inter_agent_communication(&session, receiver_thread_id, &receiver_agent_path)
        .await
    {
        build_result(
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
        build_result(
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
        let mut status_rx = host
            .subscribe_agent_status(&session, receiver_thread_id)
            .await?;
        let initial_status = status_rx.borrow_and_update().clone();
        if let Some(message) = host
            .find_pending_inter_agent_communication(
                &session,
                receiver_thread_id,
                &receiver_agent_path,
            )
            .await
        {
            build_result(
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
            build_result(
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
                host,
                &session,
                target,
                agent_name,
                receiver_thread_id,
                receiver_agent_path.clone(),
                mailbox_seq_rx,
                status_rx,
                started,
                initial_timeout_ms,
                current_timeout,
                hard_cap_timeout_ms,
            )
            .await
        }
    };
    if result.timed_out {
        host.advance_wait_agent_backoff(&session, sender_thread_id, receiver_thread_id)
            .await;
    } else {
        host.reset_wait_agent_backoff(&session, sender_thread_id, receiver_thread_id)
            .await;
    }

    let mut statuses = HashMap::new();
    statuses.insert(receiver_thread_id, result.status.clone());
    host.send_collab_event(
        &session,
        &turn,
        CollabWaitingEndEvent {
            call_id,
            completed_at_ms: now_unix_timestamp_ms(),
            sender_thread_id,
            sender_agent_path: sender_agent_path.to_string(),
            timeout_ms: wait_lifecycle_timeout_ms(current_timeout),
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

async fn wait_for_update<Host>(
    host: &Host,
    session: &Host::Session,
    target: String,
    agent_name: String,
    receiver_thread_id: ThreadId,
    receiver_agent_path: AgentPath,
    mut mailbox_seq_rx: tokio::sync::watch::Receiver<u64>,
    mut status_rx: tokio::sync::watch::Receiver<AgentStatus>,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout: Duration,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult
where
    Host: MultiAgentToolHost,
{
    let current_timeout_ms = duration_to_ms(current_timeout);
    if let Some(result) = pending_message_result(
        host,
        session,
        receiver_thread_id,
        &receiver_agent_path,
        &mut status_rx,
        &target,
        &agent_name,
        started,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    )
    .await
    {
        return result;
    }
    let deadline = Instant::now() + current_timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            let status = host.agent_status(session, receiver_thread_id).await;
            return build_result(
                target,
                agent_name,
                WaitAgentReason::Timeout,
                status,
                None,
                started,
                initial_timeout_ms,
                current_timeout_ms,
                hard_cap_timeout_ms,
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
                    host,
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    status.clone(),
                    &target,
                    &agent_name,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
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
                return build_result(
                    target,
                    agent_name,
                    reason,
                    status,
                    None,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                );
            }
            Ok(WaitEvent::Status(false)) => {
                let status = host.agent_status(session, receiver_thread_id).await;
                if let Some(result) = pending_message_result_with_fallback_status(
                    host,
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    status.clone(),
                    &target,
                    &agent_name,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                )
                .await
                {
                    return result;
                }
                return build_result(
                    target,
                    agent_name,
                    WaitAgentReason::StatusUpdate,
                    status,
                    None,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                );
            }
            Ok(WaitEvent::Mailbox(true)) => {
                if let Some(result) = pending_message_result(
                    host,
                    session,
                    receiver_thread_id,
                    &receiver_agent_path,
                    &mut status_rx,
                    &target,
                    &agent_name,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                )
                .await
                {
                    return result;
                }
            }
            Ok(WaitEvent::Mailbox(false)) | Err(_) => {
                let status = host.agent_status(session, receiver_thread_id).await;
                return build_result(
                    target,
                    agent_name,
                    WaitAgentReason::Timeout,
                    status,
                    None,
                    started,
                    initial_timeout_ms,
                    current_timeout_ms,
                    hard_cap_timeout_ms,
                );
            }
        }
    }
}

async fn pending_message_result<Host>(
    host: &Host,
    session: &Host::Session,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    status_rx: &mut tokio::sync::watch::Receiver<AgentStatus>,
    target: &str,
    agent_name: &str,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> Option<WaitAgentToolResult>
where
    Host: MultiAgentToolHost,
{
    let message = host
        .find_pending_inter_agent_communication(session, receiver_thread_id, receiver_agent_path)
        .await?;
    let fallback_status = status_rx.borrow().clone();
    Some(build_pending_message_result(
        target,
        agent_name,
        fallback_status,
        message,
        started,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    ))
}

async fn pending_message_result_with_fallback_status<Host>(
    host: &Host,
    session: &Host::Session,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    fallback_status: AgentStatus,
    target: &str,
    agent_name: &str,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> Option<WaitAgentToolResult>
where
    Host: MultiAgentToolHost,
{
    let message = host
        .find_pending_inter_agent_communication(session, receiver_thread_id, receiver_agent_path)
        .await?;
    Some(build_pending_message_result(
        target,
        agent_name,
        fallback_status,
        message,
        started,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    ))
}

enum WaitEvent {
    Status(bool),
    Mailbox(bool),
}

fn build_result(
    target: String,
    agent_name: String,
    reason: WaitAgentReason,
    status: AgentStatus,
    message: Option<InterAgentCommunication>,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult {
    WaitAgentToolResult {
        target,
        agent_name,
        reason,
        timed_out: matches!(reason, WaitAgentReason::Timeout),
        status,
        message_operation: message
            .as_ref()
            .map(|message| operation_name(message.operation).to_string()),
        message_author: message.as_ref().map(|message| message.author.to_string()),
        message_excerpt: message.map(|message| excerpt(&message.content)),
        waited_ms: started.elapsed().as_millis() as i64,
        initial_timeout_ms,
        current_timeout_ms,
        hard_cap_timeout_ms,
    }
}

fn build_pending_message_result(
    target: &str,
    agent_name: &str,
    fallback_status: AgentStatus,
    message: InterAgentCommunication,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> WaitAgentToolResult {
    let status = message.status.clone().unwrap_or(fallback_status);
    build_result(
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

fn wait_lifecycle_timeout_ms(current_timeout: Duration) -> i64 {
    duration_to_ms(current_timeout)
}

fn operation_name(operation: InterAgentOperation) -> &'static str {
    match operation {
        InterAgentOperation::Unknown => "unknown",
        InterAgentOperation::SpawnAgent => "spawn_agent",
        InterAgentOperation::SendMessage => "send_message",
        InterAgentOperation::FollowupTask => "followup_task",
        InterAgentOperation::ChildCompletion => "child_completion",
    }
}

fn excerpt(content: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 160;
    let mut excerpt = content.chars().take(MAX_EXCERPT_CHARS).collect::<String>();
    if content.chars().count() > MAX_EXCERPT_CHARS {
        excerpt.push_str("...");
    }
    excerpt
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

fn function_arguments(payload: ToolPayload) -> Result<String, FunctionCallError> {
    match payload {
        ToolPayload::Function { arguments } => Ok(arguments),
        _ => Err(FunctionCallError::RespondToModel(
            "collab handler received unsupported payload".to_string(),
        )),
    }
}

pub fn function_arguments_from_payload(payload: ToolPayload) -> Result<String, FunctionCallError> {
    function_arguments(payload)
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

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pending_message_result_takes_precedence_over_final_status() {
        let started = Instant::now();
        let target_path = AgentPath::try_from("/root/worker").expect("target path");
        let message = InterAgentCommunication::new(
            target_path.clone(),
            AgentPath::root(),
            Vec::new(),
            "child finished".to_string(),
            InterAgentOperation::ChildCompletion,
        );

        let result = build_pending_message_result(
            "worker",
            target_path.as_str(),
            AgentStatus::Completed(Some("final status".to_string())),
            message,
            started,
            60_000,
            60_000,
            1_800_000,
        );

        assert_eq!(result.reason, WaitAgentReason::MailboxMessage);
        assert_eq!(result.current_timeout_ms, 60_000);
        assert_eq!(
            result.status,
            AgentStatus::Completed(Some("final status".to_string()))
        );
        assert_eq!(
            result.message_operation.as_deref(),
            Some("child_completion")
        );
        assert_eq!(result.message_excerpt.as_deref(), Some("child finished"));
    }

    #[test]
    fn wait_lifecycle_timeout_uses_current_window_not_hard_cap() {
        assert_eq!(
            wait_lifecycle_timeout_ms(Duration::from_millis(120_000)),
            120_000
        );
    }

    #[test]
    fn spawn_agent_args_reject_legacy_fork_context() {
        let err = spawn_agent_request_from_arguments(
            r#"{"message":"hi","task_name":"worker","fork_context":true}"#,
        )
        .expect_err("legacy fork_context should be rejected");
        assert!(format!("{err:?}").contains("fork_context is not supported"));
    }
}
