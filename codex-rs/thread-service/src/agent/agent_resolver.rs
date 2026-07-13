use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use protocol::ThreadId;
use std::sync::Arc;
use tool_service_api::FunctionCallError;

/// Resolves a single tool-facing agent target to a thread id.
pub(crate) async fn resolve_agent_target(
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    target: &str,
) -> Result<ThreadId, FunctionCallError> {
    session.register_session_root_for_turn(turn);
    if let Ok(thread_id) = ThreadId::from_string(target) {
        return session
            .resolve_agent_thread_id_for_turn(turn, thread_id)
            .await
            .map_err(|err| match err {
                protocol::error::CodexErr::UnsupportedOperation(message) => {
                    FunctionCallError::RespondToModel(message)
                }
                other => FunctionCallError::RespondToModel(other.to_string()),
            });
    }

    session
        .resolve_agent_reference_for_turn(turn, target)
        .await
        .map_err(|err| match err {
            protocol::error::CodexErr::UnsupportedOperation(message) => {
                FunctionCallError::RespondToModel(message)
            }
            other => FunctionCallError::RespondToModel(other.to_string()),
        })
}
