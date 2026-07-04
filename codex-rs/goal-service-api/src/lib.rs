use std::sync::Arc;

use protocol::protocol::ThreadGoal;
use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use state_api::ExternalGoalSet;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;

/// Goal domain service API consumed by tool-service and composition roots.
pub trait GoalServiceApi: Send + Sync + 'static {
    /// Read the current thread goal.
    fn get_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>>;

    /// Create a new active thread goal for the current turn.
    fn create_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Mark the current thread goal complete.
    fn complete_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>>;

    /// Capture goal accounting state for a newly started turn.
    fn begin_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        token_usage: TokenUsage,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Account goal progress after a non-goal tool completes.
    fn account_non_goal_tool_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Account goal progress after a goal-mutating tool completes, without
    /// emitting budget-limit steering or terminal metrics.
    fn account_goal_mutation_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Finalize goal accounting for one turn.
    fn finish_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        turn_completed: bool,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Handle turn abort side effects for active goal state.
    fn handle_goal_turn_abort<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: Option<&'a dyn ThreadTurnCapability>,
        reason: TurnAbortReason,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Continue the active goal when the thread is idle.
    fn maybe_continue_active_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Account active goal usage before an external goal mutation.
    fn prepare_external_goal_mutation<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Apply runtime side effects after an external goal upsert.
    fn apply_external_goal_set<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        external_set: ExternalGoalSet,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Clear runtime state after an external goal deletion.
    fn apply_external_goal_clear<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;

    /// Restore goal runtime state after resuming a thread.
    fn restore_goal_runtime_after_resume<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>>;
}

impl<Service> GoalServiceApi for Arc<Service>
where
    Service: GoalServiceApi,
{
    fn get_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        self.as_ref().get_thread_goal(session)
    }

    fn create_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref()
            .create_thread_goal(session, turn, objective, token_budget)
    }

    fn complete_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        self.as_ref().complete_thread_goal(session, turn)
    }

    fn begin_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        token_usage: TokenUsage,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref()
            .begin_turn_goal_accounting(session, turn, token_usage)
    }

    fn account_non_goal_tool_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref()
            .account_non_goal_tool_completed(session, turn, tool_name)
    }

    fn account_goal_mutation_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().account_goal_mutation_completed(session, turn)
    }

    fn finish_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        turn_completed: bool,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref()
            .finish_turn_goal_accounting(session, turn, turn_completed)
    }

    fn handle_goal_turn_abort<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: Option<&'a dyn ThreadTurnCapability>,
        reason: TurnAbortReason,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().handle_goal_turn_abort(session, turn, reason)
    }

    fn maybe_continue_active_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().maybe_continue_active_goal(session)
    }

    fn prepare_external_goal_mutation<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().prepare_external_goal_mutation(session)
    }

    fn apply_external_goal_set<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        external_set: ExternalGoalSet,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().apply_external_goal_set(session, external_set)
    }

    fn apply_external_goal_clear<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().apply_external_goal_clear(session)
    }

    fn restore_goal_runtime_after_resume<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        self.as_ref().restore_goal_runtime_after_resume(session)
    }
}
