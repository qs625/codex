use std::future::Future;

use codex_protocol::ThreadId;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::validate_thread_goal_objective;
use codex_state_api::SharedStateDbRuntime;
use codex_state_api::ThreadGoalAccountingMode;
use codex_state_api::protocol_goal_from_state;
use codex_state_api::state_goal_status_from_protocol;
use codex_state_api::validate_thread_goal_budget;

use crate::TerminalMetricEmission;
use crate::create_thread_goal_mutation_plan;
use crate::set_thread_goal_mutation_plan;

pub struct SetGoalRequest {
    pub objective: Option<String>,
    pub status: Option<ThreadGoalStatus>,
    pub token_budget: Option<Option<i64>>,
}

pub struct CreateGoalRequest {
    pub objective: String,
    pub token_budget: Option<i64>,
}

/// Host interface for persisted thread-goal mutations.
///
/// Implementations own concrete session effects: feature gating, state-db
/// discovery, accounting side effects, metrics, event emission, history writes,
/// and parent final-status notification. The agent runtime owns mutation
/// validation, state-db mutation ordering, and mutation-plan effects.
pub trait ThreadGoalMutationHost: Send + Sync {
    type Turn: ?Sized + Send + Sync;

    fn goals_enabled(&self) -> bool;

    fn thread_id(&self) -> ThreadId;

    fn turn_id(&self, turn_context: &Self::Turn) -> String;

    fn require_state_db_for_thread_goals(
        &self,
    ) -> impl Future<Output = anyhow::Result<SharedStateDbRuntime>> + Send;

    fn account_goal_wall_clock_usage(
        &self,
        state_db: &SharedStateDbRuntime,
        mode: ThreadGoalAccountingMode,
        terminal_metric_emission: TerminalMetricEmission,
    ) -> impl Future<Output = anyhow::Result<Option<ThreadGoal>>> + Send;

    fn emit_goal_created_metric(&self);

    fn emit_goal_terminal_metrics_if_status_changed(
        &self,
        previous_status: Option<codex_state_api::ThreadGoalStatus>,
        goal: &codex_state_api::ThreadGoal,
    );

    fn reset_budget_limit_reported_goal(&self) -> impl Future<Output = ()> + Send;

    fn current_token_usage(&self) -> impl Future<Output = TokenUsage> + Send;

    fn mark_active_goal_accounting(
        &self,
        goal_id: String,
        turn_id: Option<String>,
        token_usage: TokenUsage,
    ) -> impl Future<Output = ()> + Send;

    fn clear_active_goal_accounting(
        &self,
        turn_context: &Self::Turn,
    ) -> impl Future<Output = ()> + Send;

    fn emit_thread_goal_updated(
        &self,
        turn_context: &Self::Turn,
        goal: ThreadGoal,
    ) -> impl Future<Output = ()> + Send;

    fn record_thread_goal_update_item(
        &self,
        turn_context: &Self::Turn,
        goal: ThreadGoal,
        previous_status: Option<codex_state_api::ThreadGoalStatus>,
    ) -> impl Future<Output = ()> + Send;

    fn maybe_notify_parent_of_final_status(&self) -> impl Future<Output = ()> + Send;
}

pub async fn set_thread_goal<H>(
    host: &H,
    turn_context: &H::Turn,
    request: SetGoalRequest,
) -> anyhow::Result<ThreadGoal>
where
    H: ThreadGoalMutationHost + ?Sized,
{
    if !host.goals_enabled() {
        anyhow::bail!("goals feature is disabled");
    }

    let SetGoalRequest {
        objective,
        status,
        token_budget,
    } = request;
    validate_thread_goal_budget(token_budget.flatten())?;
    let state_db = host.require_state_db_for_thread_goals().await?;
    let objective = objective.map(|objective| objective.trim().to_string());
    if let Some(objective) = objective.as_deref()
        && let Err(err) = validate_thread_goal_objective(objective)
    {
        anyhow::bail!("{err}");
    }

    host.account_goal_wall_clock_usage(
        &state_db,
        ThreadGoalAccountingMode::ActiveOnly,
        TerminalMetricEmission::Emit,
    )
    .await?;

    let thread_id = host.thread_id();
    let mut replacing_goal = false;
    let previous_status;
    let goal = if let Some(objective) = objective.as_deref() {
        let existing_goal = state_db.get_thread_goal(thread_id).await?;
        previous_status = existing_goal.as_ref().map(|goal| goal.status);
        if let Some(existing_goal) = existing_goal.as_ref() {
            state_db
                .update_thread_goal(
                    thread_id,
                    codex_state_api::ThreadGoalUpdate {
                        objective: Some(objective.to_string()),
                        status: status.map(state_goal_status_from_protocol),
                        token_budget,
                        expected_goal_id: Some(existing_goal.goal_id.clone()),
                    },
                )
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!("cannot update goal for thread {thread_id}: no goal exists")
                })?
        } else {
            replacing_goal = true;
            state_db
                .replace_thread_goal(
                    thread_id,
                    objective,
                    status
                        .map(state_goal_status_from_protocol)
                        .unwrap_or(codex_state_api::ThreadGoalStatus::Active),
                    token_budget.flatten(),
                )
                .await?
        }
    } else {
        let existing_goal = state_db.get_thread_goal(thread_id).await?;
        previous_status = existing_goal.as_ref().map(|goal| goal.status);
        let expected_goal_id = existing_goal.map(|goal| goal.goal_id);
        let status = status.map(state_goal_status_from_protocol);
        state_db
            .update_thread_goal(
                thread_id,
                codex_state_api::ThreadGoalUpdate {
                    objective: None,
                    status,
                    token_budget,
                    expected_goal_id,
                },
            )
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("cannot update goal for thread {thread_id}: no goal exists")
            })?
    };

    let plan = set_thread_goal_mutation_plan(previous_status, replacing_goal, &goal);
    if plan.emit_created_metric {
        host.emit_goal_created_metric();
    }
    host.emit_goal_terminal_metrics_if_status_changed(
        plan.previous_status_for_terminal_metrics,
        &goal,
    );
    let goal = protocol_goal_from_state(goal);
    host.reset_budget_limit_reported_goal().await;
    if let Some(goal_id) = plan.newly_active_goal_id {
        let current_token_usage = host.current_token_usage().await;
        host.mark_active_goal_accounting(
            goal_id,
            Some(host.turn_id(turn_context)),
            current_token_usage,
        )
        .await;
    } else if plan.clear_active_accounting {
        host.clear_active_goal_accounting(turn_context).await;
    }
    host.emit_thread_goal_updated(turn_context, goal.clone())
        .await;
    host.record_thread_goal_update_item(
        turn_context,
        goal.clone(),
        plan.previous_status_for_display,
    )
    .await;
    if plan.notify_parent_final_status {
        host.maybe_notify_parent_of_final_status().await;
    }
    Ok(goal)
}

pub async fn create_thread_goal<H>(
    host: &H,
    turn_context: &H::Turn,
    request: CreateGoalRequest,
) -> anyhow::Result<ThreadGoal>
where
    H: ThreadGoalMutationHost + ?Sized,
{
    if !host.goals_enabled() {
        anyhow::bail!("goals feature is disabled");
    }

    let CreateGoalRequest {
        objective,
        token_budget,
    } = request;
    validate_thread_goal_budget(token_budget)?;
    let objective = objective.trim();
    validate_thread_goal_objective(objective).map_err(anyhow::Error::msg)?;

    let state_db = host.require_state_db_for_thread_goals().await?;
    host.account_goal_wall_clock_usage(
        &state_db,
        ThreadGoalAccountingMode::ActiveOnly,
        TerminalMetricEmission::Emit,
    )
    .await?;
    let thread_id = host.thread_id();
    let goal = state_db
        .insert_thread_goal(
            thread_id,
            objective,
            codex_state_api::ThreadGoalStatus::Active,
            token_budget,
        )
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot create a new goal because thread {thread_id} already has a goal"
            )
        })?;

    let plan = create_thread_goal_mutation_plan(&goal);
    if plan.emit_created_metric {
        host.emit_goal_created_metric();
    }
    let goal = protocol_goal_from_state(goal);
    host.reset_budget_limit_reported_goal().await;
    let current_token_usage = host.current_token_usage().await;
    host.mark_active_goal_accounting(
        plan.newly_active_goal_id
            .expect("created thread goals should be active"),
        Some(host.turn_id(turn_context)),
        current_token_usage,
    )
    .await;

    host.emit_thread_goal_updated(turn_context, goal.clone())
        .await;
    host.record_thread_goal_update_item(turn_context, goal.clone(), None)
        .await;
    Ok(goal)
}
