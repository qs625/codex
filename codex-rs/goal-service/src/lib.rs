use codex_protocol::protocol::ThreadGoal;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ThreadTurnCapability;
use goal_service_api::GoalServiceApi;

#[derive(Clone, Default)]
pub struct GoalService;

impl GoalServiceApi for GoalService {
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        Box::pin(async move { capability.get_thread_goal().await })
    }

    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async move { capability.create_thread_goal(objective, token_budget).await })
    }

    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async move { capability.complete_thread_goal().await })
    }
}
