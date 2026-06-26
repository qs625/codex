use std::sync::Arc;

use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_thread_api::GoalApi;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ThreadCapability;

use crate::goal::CreateGoalRequest;
use crate::goal::SetGoalRequest;
use crate::session::turn_context::TurnContext;

#[derive(Clone, Default)]
pub struct GoalService;

impl GoalApi for GoalService {
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        let session = session_from_capability(capability);
        Box::pin(async move { session.get_thread_goal().await.map_err(format_goal_error) })
    }

    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            session
                .create_thread_goal(
                    turn,
                    CreateGoalRequest {
                        objective,
                        token_budget,
                    },
                )
                .await
                .map_err(format_goal_error)
        })
    }

    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            session
                .set_thread_goal(
                    turn,
                    SetGoalRequest {
                        objective: None,
                        status: Some(ThreadGoalStatus::Complete),
                        token_budget: None,
                    },
                )
                .await
                .map_err(format_goal_error)
        })
    }
}

fn session_from_capability(capability: &dyn ThreadCapability) -> Arc<crate::session::session::Session> {
    turn_context_from_capability(capability).session_arc()
}

fn turn_context_from_capability(capability: &dyn ThreadCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("thread goal capability must be backed by TurnContext")
}

fn format_goal_error(err: anyhow::Error) -> String {
    let mut message = err.to_string();
    for cause in err.chain().skip(1) {
        message.push_str(": ");
        message.push_str(&cause.to_string());
    }
    message
}
