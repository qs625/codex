use super::*;
use crate::agent::status::is_final;
use crate::pending_input::PendingInputItem;
use crate::session::session::Session;
use codex_tools::create_wait_agent_tool_v2;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_protocol::AgentPath;
use codex_protocol::ThreadId;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::CollabAgentRef;
use codex_protocol::protocol::CollabAgentStatusEntry;
use codex_protocol::protocol::CollabWaitingBeginEvent;
use codex_protocol::protocol::CollabWaitingEndEvent;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_tools::ToolSpec;
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

pub(crate) struct Handler;

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for Handler {
    type Output = WaitAgentResult;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("wait_agent")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_wait_agent_tool_v2())
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        handle_wait_agent(invocation).await
    }
}

impl ToolHandler for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitAgentArgs {
    target: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WaitAgentResult {
    target: String,
    agent_name: String,
    reason: WaitAgentReason,
    timed_out: bool,
    status: AgentStatus,
    message_operation: Option<String>,
    message_author: Option<String>,
    message_excerpt: Option<String>,
    waited_ms: i64,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WaitAgentReason {
    PendingMessage,
    MailboxMessage,
    StatusUpdate,
    FinalStatus,
    Timeout,
}

impl ToolOutput for WaitAgentResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "wait_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "wait_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "wait_agent")
    }
}

pub(crate) async fn handle_wait_agent(
    invocation: ToolInvocation,
) -> Result<WaitAgentResult, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        payload,
        call_id,
        ..
    } = invocation;
    let arguments = function_arguments(payload)?;
    let args: WaitAgentArgs = parse_arguments(&arguments)?;
    let receiver_thread_id = resolve_agent_target(&session, &turn, &args.target).await?;
    let receiver_agent = session
        .services
        .agent_control
        .get_agent_metadata(receiver_thread_id)
        .unwrap_or_default();
    if receiver_agent
        .agent_path
        .as_ref()
        .is_some_and(AgentPath::is_root)
    {
        return Err(FunctionCallError::RespondToModel(
            "root is not a spawned agent".to_string(),
        ));
    }
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let sender_agent_path = turn
        .session_source
        .get_agent_path()
        .unwrap_or_else(AgentPath::root);
    let initial_timeout_ms = turn.config.multi_agent_v2.default_wait_timeout_ms;
    let hard_cap_timeout_ms = turn.config.multi_agent_v2.max_wait_timeout_ms;
    let current_timeout = session
        .wait_agent_current_window(
            session.conversation_id,
            receiver_thread_id,
            initial_timeout_ms,
            hard_cap_timeout_ms,
        )
        .await;
    let current_timeout_ms = duration_to_ms(current_timeout);
    let receiver_agents = vec![CollabAgentRef {
        thread_id: receiver_thread_id,
        agent_path: Some(receiver_agent_path.to_string()),
        agent_nickname: receiver_agent.agent_nickname.clone(),
        agent_role: receiver_agent.agent_role.clone(),
    }];

    session
        .send_event(
            &turn,
            CollabWaitingBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: sender_agent_path.to_string(),
                receiver_thread_ids: vec![receiver_thread_id],
                receiver_agents,
                timeout_ms: wait_lifecycle_timeout_ms(current_timeout),
            }
            .into(),
        )
        .await;

    let started = Instant::now();
    let mailbox_seq_rx = session.subscribe_mailbox_seq();
    let snapshot_status = session
        .services
        .agent_control
        .get_status(receiver_thread_id)
        .await;
    let agent_name = receiver_agent_path.to_string();

    let result = if let Some(message) =
        find_matching_pending_message(&session, receiver_thread_id, &receiver_agent_path).await
    {
        build_result(
            args.target,
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
            args.target,
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
        let mut status_rx = session
            .services
            .agent_control
            .subscribe_status(receiver_thread_id)
            .await
            .map_err(|err| collab_agent_error(receiver_thread_id, err))?;
        let initial_status = status_rx.borrow_and_update().clone();
        if let Some(message) =
            find_matching_pending_message(&session, receiver_thread_id, &receiver_agent_path).await
        {
            build_result(
                args.target,
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
                args.target,
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
                &session,
                args.target,
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
        session
            .advance_wait_agent_backoff(session.conversation_id, receiver_thread_id)
            .await;
    } else {
        session
            .reset_wait_agent_backoff(session.conversation_id, receiver_thread_id)
            .await;
    }

    let mut statuses = HashMap::new();
    statuses.insert(receiver_thread_id, result.status.clone());
    session
        .send_event(
            &turn,
            CollabWaitingEndEvent {
                call_id,
                completed_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
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

async fn wait_for_update(
    session: &std::sync::Arc<Session>,
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
) -> WaitAgentResult {
    let current_timeout_ms = duration_to_ms(current_timeout);
    if let Some(result) = pending_message_result(
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
            let status = session
                .services
                .agent_control
                .get_status(receiver_thread_id)
                .await;
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
                let status = session
                    .services
                    .agent_control
                    .get_status(receiver_thread_id)
                    .await;
                if let Some(result) = pending_message_result_with_fallback_status(
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
                let status = session
                    .services
                    .agent_control
                    .get_status(receiver_thread_id)
                    .await;
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

async fn pending_message_result(
    session: &std::sync::Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    status_rx: &mut tokio::sync::watch::Receiver<AgentStatus>,
    target: &str,
    agent_name: &str,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> Option<WaitAgentResult> {
    let message =
        find_matching_pending_message(session, receiver_thread_id, receiver_agent_path).await?;
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

async fn pending_message_result_with_fallback_status(
    session: &std::sync::Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
    fallback_status: AgentStatus,
    target: &str,
    agent_name: &str,
    started: Instant,
    initial_timeout_ms: i64,
    current_timeout_ms: i64,
    hard_cap_timeout_ms: i64,
) -> Option<WaitAgentResult> {
    let message =
        find_matching_pending_message(session, receiver_thread_id, receiver_agent_path).await?;
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

async fn find_matching_pending_message(
    session: &std::sync::Arc<Session>,
    receiver_thread_id: ThreadId,
    receiver_agent_path: &AgentPath,
) -> Option<InterAgentCommunication> {
    session
        .find_pending_input(|item| {
            matching_communication(item, receiver_thread_id, receiver_agent_path)
        })
        .await
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
    .then(|| communication.clone())
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
) -> WaitAgentResult {
    WaitAgentResult {
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
) -> WaitAgentResult {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::tests::make_session_and_context;
    use codex_protocol::ThreadId;
    use pretty_assertions::assert_eq;

    #[test]
    fn matching_communication_uses_typed_author_or_sender_thread_id() {
        let target_thread_id = ThreadId::new();
        let target_path = AgentPath::try_from("/root/worker").expect("target path");
        let parent_path = AgentPath::root();
        let by_author = InterAgentCommunication::new(
            target_path.clone(),
            parent_path.clone(),
            Vec::new(),
            "done".to_string(),
            InterAgentOperation::ChildCompletion,
        );
        let by_thread_id = InterAgentCommunication::new(
            AgentPath::try_from("/root/renamed_worker").expect("renamed path"),
            parent_path.clone(),
            Vec::new(),
            "also done".to_string(),
            InterAgentOperation::ChildCompletion,
        )
        .with_thread_ids(target_thread_id, ThreadId::new());
        let unrelated = InterAgentCommunication::new(
            AgentPath::try_from("/root/other").expect("other path"),
            parent_path,
            Vec::new(),
            "ignore".to_string(),
            InterAgentOperation::Unknown,
        );

        assert_eq!(
            matching_communication(
                &PendingInputItem::from(by_author.clone()),
                target_thread_id,
                &target_path,
            ),
            Some(by_author)
        );
        assert_eq!(
            matching_communication(
                &PendingInputItem::from(by_thread_id.clone()),
                target_thread_id,
                &target_path,
            ),
            Some(by_thread_id)
        );
        assert_eq!(
            matching_communication(
                &PendingInputItem::from(unrelated),
                target_thread_id,
                &target_path,
            ),
            None
        );
    }

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

    #[tokio::test]
    async fn wait_agent_backoff_advances_per_target_and_resets_after_event() {
        let (session, _) = make_session_and_context().await;
        let sender_thread_id = session.conversation_id;
        let receiver_thread_id = ThreadId::new();

        assert_eq!(
            session
                .wait_agent_current_window(
                    sender_thread_id,
                    receiver_thread_id,
                    /*initial_timeout_ms*/ 10,
                    /*hard_cap_timeout_ms*/ 25,
                )
                .await,
            Duration::from_millis(10)
        );

        session
            .advance_wait_agent_backoff(sender_thread_id, receiver_thread_id)
            .await;
        assert_eq!(
            session
                .wait_agent_current_window(
                    sender_thread_id,
                    receiver_thread_id,
                    /*initial_timeout_ms*/ 10,
                    /*hard_cap_timeout_ms*/ 25,
                )
                .await,
            Duration::from_millis(20)
        );

        session
            .advance_wait_agent_backoff(sender_thread_id, receiver_thread_id)
            .await;
        assert_eq!(
            session
                .wait_agent_current_window(
                    sender_thread_id,
                    receiver_thread_id,
                    /*initial_timeout_ms*/ 10,
                    /*hard_cap_timeout_ms*/ 25,
                )
                .await,
            Duration::from_millis(25)
        );

        session
            .reset_wait_agent_backoff(sender_thread_id, receiver_thread_id)
            .await;
        assert_eq!(
            session
                .wait_agent_current_window(
                    sender_thread_id,
                    receiver_thread_id,
                    /*initial_timeout_ms*/ 10,
                    /*hard_cap_timeout_ms*/ 25,
                )
                .await,
            Duration::from_millis(10)
        );
    }

    #[tokio::test]
    async fn wait_for_update_ignores_unmatched_mailbox_event_until_current_window_expires() {
        let (session, _) = make_session_and_context().await;
        let session = std::sync::Arc::new(session);
        let receiver_thread_id = ThreadId::new();
        let receiver_agent_path = AgentPath::try_from("/root/target").expect("target path");
        let mailbox_seq_rx = session.subscribe_mailbox_seq();
        let (_status_tx, status_rx) = tokio::sync::watch::channel(AgentStatus::Running);
        let unrelated_session = std::sync::Arc::clone(&session);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            unrelated_session.enqueue_mailbox_communication(InterAgentCommunication::new(
                AgentPath::try_from("/root/other").expect("other path"),
                AgentPath::root(),
                Vec::new(),
                "unrelated".to_string(),
                InterAgentOperation::SendMessage,
            ));
        });

        let result = wait_for_update(
            &session,
            "target".to_string(),
            receiver_agent_path.to_string(),
            receiver_thread_id,
            receiver_agent_path,
            mailbox_seq_rx,
            status_rx,
            Instant::now(),
            /*initial_timeout_ms*/ 30,
            Duration::from_millis(30),
            /*hard_cap_timeout_ms*/ 300,
        )
        .await;

        assert_eq!(result.reason, WaitAgentReason::Timeout);
        assert_eq!(result.current_timeout_ms, 30);
        assert!(
            result.waited_ms >= 20,
            "unmatched mailbox event should not end the wait early: {result:?}"
        );
    }
}
