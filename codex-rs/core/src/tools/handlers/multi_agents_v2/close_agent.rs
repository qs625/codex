use super::*;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_close_agent_tool_v2;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    type Output = CloseAgentResult;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("close_agent")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_close_agent_tool_v2())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation,
    ) -> crate::tools::registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move { handle_close_agent(invocation).await })
    }
}

async fn handle_close_agent(
    invocation: ToolInvocation,
) -> Result<CloseAgentResult, FunctionCallError> {
    let ToolInvocation {
        session,
        turn,
        payload,
        call_id,
        ..
    } = invocation;
    let arguments = function_arguments(payload)?;
    let args: CloseAgentArgs = parse_arguments(&arguments)?;
    let agent_id = resolve_agent_target(&session, &turn, &args.target).await?;
    let receiver_agent = session
        .services
        .agent_control
        .get_agent_metadata(agent_id)
        .unwrap_or_default();
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let sender_agent_path = turn
        .session_source
        .get_agent_path()
        .unwrap_or_else(AgentPath::root);
    let receiver_is_direct_child = receiver_agent_path
        .as_str()
        .rsplit_once('/')
        .is_some_and(|(parent, _)| parent == sender_agent_path.as_str());
    if receiver_agent
        .agent_path
        .as_ref()
        .is_some_and(AgentPath::is_root)
    {
        return Err(FunctionCallError::RespondToModel(
            "root is not a spawned agent".to_string(),
        ));
    }
    session
        .send_event(
            &turn,
            CollabCloseBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: turn
                    .session_source
                    .get_agent_path()
                    .unwrap_or_else(AgentPath::root)
                    .to_string(),
                receiver_thread_id: agent_id,
                receiver_agent_path: receiver_agent_path.to_string(),
            }
            .into(),
        )
        .await;
    let status = match session
        .services
        .agent_control
        .subscribe_status(agent_id)
        .await
    {
        Ok(mut status_rx) => status_rx.borrow_and_update().clone(),
        Err(err) => {
            let status = session.services.agent_control.get_status(agent_id).await;
            session
                .send_event(
                    &turn,
                    CollabCloseEndEvent {
                        call_id: call_id.clone(),
                        completed_at_ms: now_unix_timestamp_ms(),
                        sender_thread_id: session.conversation_id,
                        sender_agent_path: turn
                            .session_source
                            .get_agent_path()
                            .unwrap_or_else(AgentPath::root)
                            .to_string(),
                        receiver_thread_id: agent_id,
                        receiver_agent_path: receiver_agent_path.to_string(),
                        receiver_agent_nickname: receiver_agent.agent_nickname.clone(),
                        receiver_agent_role: receiver_agent.agent_role.clone(),
                        status,
                    }
                    .into(),
                )
                .await;
            return Err(collab_agent_error(agent_id, err));
        }
    };
    let result = session
        .services
        .agent_control
        .close_agent(agent_id)
        .await
        .map_err(|err| collab_agent_error(agent_id, err))
        .map(|_| ());
    session
        .send_event(
            &turn,
            CollabCloseEndEvent {
                call_id,
                completed_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: turn
                    .session_source
                    .get_agent_path()
                    .unwrap_or_else(AgentPath::root)
                    .to_string(),
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
        && session
            .clear_direct_child_completion_pending(agent_id)
            .await
    {
        session
            .maybe_notify_parent_of_final_status_for_current_source()
            .await;
    }

    Ok(CloseAgentResult {
        previous_status: status,
    })
}

impl ToolHandler for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseAgentArgs {
    target: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CloseAgentResult {
    pub(crate) previous_status: AgentStatus,
}

impl ToolOutput for CloseAgentResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "close_agent")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "close_agent")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "close_agent")
    }
}
