use goal_service_api::GoalServiceApi;
use protocol::models::ThreadGoalUpdateEventSource;
use protocol::protocol::EventMsg;
use protocol::protocol::ThreadGoal;
use protocol::protocol::ThreadGoalStatus;
use protocol::protocol::ThreadGoalUpdatedEvent;
use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::validate_thread_goal_objective;
use state_api::ExternalGoalPreviousStatus;
use state_api::ExternalGoalSet;
use state_api::ThreadGoalUpdate;
use state_api::protocol_goal_from_state;
use state_api::state_goal_status_from_protocol;
use state_api::thread_goal_update_response_item;
use state_api::validate_thread_goal_budget;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;

#[derive(Clone, Default)]
pub struct GoalService;

impl GoalServiceApi for GoalService {
    fn get_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        Box::pin(async move {
            let state_db = session.require_persisted_state_db().await?;
            state_db
                .get_thread_goal(session.conversation_id())
                .await
                .map(|goal| goal.map(protocol_goal_from_state))
                .map_err(|err| err.to_string())
        })
    }

    fn create_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async move {
            validate_thread_goal_budget(token_budget).map_err(|err| err.to_string())?;
            let objective = objective.trim();
            validate_thread_goal_objective(objective).map_err(|err| err.to_string())?;

            let state_db = session.require_persisted_state_db().await?;
            session.prepare_external_goal_mutation().await?;

            let goal = state_db
                .insert_thread_goal(
                    session.conversation_id(),
                    objective,
                    state_api::ThreadGoalStatus::Active,
                    token_budget,
                )
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| {
                    format!(
                        "cannot create a new goal because thread {} already has a goal",
                        session.conversation_id()
                    )
                })?;

            session
                .apply_external_goal_set(ExternalGoalSet {
                    goal: goal.clone(),
                    previous_status: ExternalGoalPreviousStatus::NewGoal,
                })
                .await?;

            let goal = protocol_goal_from_state(goal);
            emit_goal_update(session, turn, goal.clone(), None).await;
            Ok(goal)
        })
    }

    fn complete_thread_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async move {
            let state_db = session.require_persisted_state_db().await?;
            session.prepare_external_goal_mutation().await?;

            let existing_goal = state_db
                .get_thread_goal(session.conversation_id())
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| {
                    format!(
                        "cannot update goal for thread {}: no goal exists",
                        session.conversation_id()
                    )
                })?;
            let previous_status = existing_goal.status;

            let goal = state_db
                .update_thread_goal(
                    session.conversation_id(),
                    ThreadGoalUpdate {
                        objective: None,
                        status: Some(state_goal_status_from_protocol(ThreadGoalStatus::Complete)),
                        token_budget: None,
                        expected_goal_id: Some(existing_goal.goal_id.clone()),
                    },
                )
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| {
                    format!(
                        "cannot update goal for thread {}: no goal exists",
                        session.conversation_id()
                    )
                })?;

            session
                .apply_external_goal_set(ExternalGoalSet {
                    goal: goal.clone(),
                    previous_status: (&existing_goal).into(),
                })
                .await?;

            let goal = protocol_goal_from_state(goal);
            emit_goal_update(session, turn, goal.clone(), Some(previous_status)).await;
            Ok(goal)
        })
    }

    fn begin_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        token_usage: TokenUsage,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.begin_turn_goal_accounting(turn, token_usage).await })
    }

    fn account_non_goal_tool_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if tool_name == "update_goal" {
                Ok(())
            } else {
                session.account_goal_tool_completed(turn, tool_name).await
            }
        })
    }

    fn account_goal_mutation_completed<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.account_goal_mutation_completed(turn).await })
    }

    fn finish_turn_goal_accounting<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        turn_completed: bool,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            session
                .finish_turn_goal_accounting(turn, turn_completed)
                .await
        })
    }

    fn handle_goal_turn_abort<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        turn: Option<&'a dyn ThreadTurnCapability>,
        reason: TurnAbortReason,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.handle_goal_turn_abort(turn, reason).await })
    }

    fn maybe_continue_active_goal<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.maybe_continue_active_goal().await })
    }

    fn prepare_external_goal_mutation<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.prepare_external_goal_mutation().await })
    }

    fn apply_external_goal_set<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
        external_set: ExternalGoalSet,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.apply_external_goal_set(external_set).await })
    }

    fn apply_external_goal_clear<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.apply_external_goal_clear().await })
    }

    fn restore_goal_runtime_after_resume<'a>(
        &'a self,
        session: &'a dyn ThreadSessionCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move { session.restore_goal_runtime_after_resume().await })
    }
}

async fn emit_goal_update(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadTurnCapability,
    goal: ThreadGoal,
    previous_status: Option<state_api::ThreadGoalStatus>,
) {
    session
        .emit_event(
            turn,
            EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
                thread_id: session.conversation_id(),
                turn_id: Some(turn.runtime_turn_id_str().to_string()),
                goal: goal.clone(),
            }),
        )
        .await;
    session
        .record_model_items_and_emit_display_events(
            turn,
            vec![thread_goal_update_response_item(
                goal,
                previous_status,
                ThreadGoalUpdateEventSource::ModelTool,
            )],
        )
        .await;
}
