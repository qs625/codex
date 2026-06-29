use std::sync::Arc;

use codex_protocol::protocol::ThreadGoal;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ThreadTurnCapability;

/// Goal domain service API consumed by tool-service and composition roots.
pub trait GoalServiceApi: Send + Sync + 'static {
    /// Read the current thread goal.
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>>;

    /// Create a new active thread goal for the current turn.
    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Mark the current thread goal complete.
    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;
}

impl<Service> GoalServiceApi for Arc<Service>
where
    Service: GoalServiceApi,
{
    fn get_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        self.as_ref().get_thread_goal(capability)
    }

    fn create_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref()
            .create_thread_goal(capability, objective, token_budget)
    }

    fn complete_thread_goal<'a>(
        &'a self,
        capability: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref().complete_thread_goal(capability)
    }
}
