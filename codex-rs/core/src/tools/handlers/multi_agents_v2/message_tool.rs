//! Shared argument parsing and dispatch for the v2 text-only follow-up task tool.

use super::*;
use crate::agent::AgentMode;
use crate::tools::context::FunctionToolOutput;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_features::Feature;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Input for the MultiAgentV2 `followup_task` tool.
pub(crate) struct FollowupTaskArgs {
    pub(crate) target: String,
    pub(crate) message: String,
}

fn message_content(message: String) -> Result<String, FunctionCallError> {
    if message.trim().is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "Empty message can't be sent to an agent".to_string(),
        ));
    }
    Ok(message)
}

/// Handles the MultiAgentV2 plain-text follow-up task flow.
pub(crate) async fn handle_message_string_tool(
    invocation: ToolInvocation,
    target: String,
    message: String,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let prompt = message_content(message)?;
    let ToolInvocation {
        session,
        turn,
        call_id,
        ..
    } = invocation;
    let receiver_thread_id = resolve_agent_target(&session, &turn, &target).await?;
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
            "Tasks can't be assigned to the root agent".to_string(),
        ));
    }
    let receiver_agent_path = receiver_agent.agent_path.clone().ok_or_else(|| {
        FunctionCallError::RespondToModel("target agent is missing an agent_path".to_string())
    })?;
    let receiver_agent_path_string = receiver_agent_path.to_string();
    session
        .send_event(
            &turn,
            CollabAgentInteractionBeginEvent {
                call_id: call_id.clone(),
                started_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: turn
                    .session_source
                    .get_agent_path()
                    .unwrap_or_else(AgentPath::root)
                    .to_string(),
                receiver_thread_id,
                receiver_agent_path: receiver_agent_path_string.clone(),
                prompt: prompt.clone(),
            }
            .into(),
        )
        .await;
    let sender_agent_path = turn
        .session_source
        .get_agent_path()
        .unwrap_or_else(AgentPath::root);
    let receiver_is_direct_child = receiver_agent_path
        .as_str()
        .rsplit_once('/')
        .is_some_and(|(parent, _)| parent == sender_agent_path.as_str());
    let receiver_will_send_completion = receiver_agent.agent_mode != AgentMode::Management
        && session
            .services
            .agent_control
            .agent_thread_enabled(receiver_thread_id, Feature::MultiAgentV2)
            .await;
    if receiver_is_direct_child && receiver_will_send_completion {
        session
            .mark_direct_child_completion_pending(receiver_thread_id)
            .await;
    }

    let communication = InterAgentCommunication::new(
        turn.session_source
            .get_agent_path()
            .unwrap_or_else(AgentPath::root),
        receiver_agent_path.clone(),
        Vec::new(),
        prompt.clone(),
        InterAgentOperation::FollowupTask,
    )
    .with_thread_ids(session.conversation_id, receiver_thread_id);
    let result = session
        .services
        .agent_control
        .send_inter_agent_communication(receiver_thread_id, communication.with_trigger_turn(true))
        .await
        .map_err(|err| collab_agent_error(receiver_thread_id, err));
    let status = session
        .services
        .agent_control
        .get_status(receiver_thread_id)
        .await;
    session
        .send_event(
            &turn,
            CollabAgentInteractionEndEvent {
                call_id,
                completed_at_ms: now_unix_timestamp_ms(),
                sender_thread_id: session.conversation_id,
                sender_agent_path: turn
                    .session_source
                    .get_agent_path()
                    .unwrap_or_else(AgentPath::root)
                    .to_string(),
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
        if receiver_is_direct_child && receiver_will_send_completion {
            if session
                .mark_direct_child_completion_received(receiver_thread_id)
                .await
            {
                session
                    .maybe_notify_parent_of_final_status_for_current_source()
                    .await;
            }
        }
        return Err(err);
    }
    Ok(FunctionToolOutput::from_text(String::new(), Some(true)))
}
