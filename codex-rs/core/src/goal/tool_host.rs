use std::sync::Arc;

use crate::goal::CreateGoalRequest;
use crate::goal::GoalRuntimeEvent;
use crate::goal::SetGoalRequest;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::CoreToolDomainHost;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_tool_runtime_api::GoalToolHost;

impl GoalToolHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    async fn get_thread_goal(&self, session: &Self::Session) -> Result<Option<ThreadGoal>, String> {
        session.get_thread_goal().await.map_err(format_goal_error)
    }

    async fn create_thread_goal(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        objective: String,
        token_budget: Option<i64>,
    ) -> Result<ThreadGoal, String> {
        session
            .create_thread_goal(
                turn.as_ref(),
                CreateGoalRequest {
                    objective,
                    token_budget,
                },
            )
            .await
            .map_err(format_goal_error)
    }

    async fn complete_thread_goal(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> Result<ThreadGoal, String> {
        session
            .goal_runtime_apply(GoalRuntimeEvent::ToolCompletedGoal {
                turn_context: turn.as_ref(),
            })
            .await
            .map_err(format_goal_error)?;
        session
            .set_thread_goal(
                turn.as_ref(),
                SetGoalRequest {
                    objective: None,
                    status: Some(ThreadGoalStatus::Complete),
                    token_budget: None,
                },
            )
            .await
            .map_err(format_goal_error)
    }
}

fn format_goal_error(err: anyhow::Error) -> String {
    let mut message = err.to_string();
    for cause in err.chain().skip(1) {
        message.push_str(": ");
        message.push_str(&cause.to_string());
    }
    message
}
