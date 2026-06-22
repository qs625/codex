//! Core support for persisted thread goals.
//!
//! This module bridges core sessions and the state-db goal table. It validates
//! goal mutations, converts between state and protocol shapes, emits goal-update
//! events, and owns helper hooks used by goal lifecycle behavior.

use crate::pending_input::PendingInputItem;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::state::ActiveTurn;
use crate::state::TurnState;
use crate::state_db_bridge::insert_thread_metadata_if_absent;
use crate::tasks::RegularTask;
use anyhow::Context;
use chrono::DateTime;
use chrono::Utc;
use codex_agent_runtime::ThreadPostTurnInputs;
use codex_agent_runtime::ThreadPostTurnState;
use codex_agent_runtime::goal_budget_limit_steering_item;
use codex_agent_runtime::goal_continuation_input_item;
use codex_agent_runtime::goal_objective_updated_steering_item;
use codex_agent_runtime::select_thread_post_turn_state;
use codex_agent_runtime::should_ignore_goal_for_mode;
use codex_features::Feature;
use codex_metrics_api::GOAL_BUDGET_LIMITED_METRIC;
use codex_metrics_api::GOAL_COMPLETED_METRIC;
use codex_metrics_api::GOAL_CREATED_METRIC;
use codex_metrics_api::GOAL_DURATION_SECONDS_METRIC;
use codex_metrics_api::GOAL_TOKEN_COUNT_METRIC;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ThreadGoalUpdateEventSource;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_protocol::protocol::ThreadGoalUpdatedEvent;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::validate_thread_goal_objective;
use codex_state_api::ExternalGoalPreviousStatus;
use codex_state_api::ExternalGoalSet;
use codex_state_api::SharedStateDbRuntime;
use codex_state_api::ThreadGoalTurnAccountingSnapshot as GoalTurnAccountingSnapshot;
use codex_state_api::protocol_goal_from_state;
use codex_state_api::state_goal_status_from_protocol;
use codex_state_api::thread_goal_update_response_item;
use codex_state_api::validate_thread_goal_budget;
use codex_tool_planning::UPDATE_GOAL_TOOL_NAME;
use futures::future::BoxFuture;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct SetGoalRequest {
    pub(crate) objective: Option<String>,
    pub(crate) status: Option<ThreadGoalStatus>,
    pub(crate) token_budget: Option<Option<i64>>,
}

pub(crate) struct CreateGoalRequest {
    pub(crate) objective: String,
    pub(crate) token_budget: Option<i64>,
}

#[derive(Clone, Copy)]
enum BudgetLimitSteering {
    Allowed,
    Suppressed,
}

#[derive(Clone, Copy)]
enum TerminalMetricEmission {
    Emit,
    Suppress,
}

/// Runtime lifecycle events that can affect goal accounting, scheduling, or
/// model-visible steering.
///
/// Callers report the session event they observed; this module owns the policy
/// for how that event changes goal runtime state.
pub(crate) enum GoalRuntimeEvent<'a> {
    TurnStarted {
        turn_context: &'a TurnContext,
        token_usage: TokenUsage,
    },
    ToolCompleted {
        turn_context: &'a TurnContext,
        tool_name: &'a str,
    },
    ToolCompletedGoal {
        turn_context: &'a TurnContext,
    },
    TurnFinished {
        turn_context: &'a TurnContext,
        turn_completed: bool,
    },
    MaybeContinueIfIdle,
    TaskAborted {
        turn_context: Option<&'a TurnContext>,
        reason: TurnAbortReason,
    },
    ExternalMutationStarting,
    ExternalSet {
        external_set: ExternalGoalSet,
    },
    ExternalClear,
    ThreadResumed,
}

struct GoalContinuationCandidate {
    goal_id: String,
    items: Vec<ResponseInputItem>,
}

impl Session {
    /// Applies runtime policy for a goal lifecycle event.
    ///
    /// Goal data methods validate and persist state; this dispatcher owns the
    /// cross-cutting runtime behavior: plan mode ignores continuations, turn
    /// starts capture the active goal and token baseline, tool completions
    /// account usage and may inject budget steering, completion accounting
    /// suppresses that steering, external mutations account best-effort before
    /// changing state, interrupts pause active goals, thread resumes restore
    /// runtime state for already-active goals, explicit maybe-continue events
    /// start idle goal continuation turns, and continuation turns with no counted
    /// autonomous activity suppress the next automatic continuation until
    /// user/tool/external activity resets it.
    pub(crate) fn goal_runtime_apply<'a>(
        self: &'a Arc<Self>,
        event: GoalRuntimeEvent<'a>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        match event {
            GoalRuntimeEvent::TurnStarted {
                turn_context,
                token_usage,
            } => Box::pin(async move {
                self.mark_thread_goal_turn_started(turn_context, token_usage)
                    .await;
                Ok(())
            }),
            GoalRuntimeEvent::ToolCompleted {
                turn_context,
                tool_name,
            } => Box::pin(async move {
                if tool_name != UPDATE_GOAL_TOOL_NAME {
                    self.account_thread_goal_progress(
                        turn_context,
                        BudgetLimitSteering::Allowed,
                        TerminalMetricEmission::Emit,
                    )
                    .await?;
                }
                Ok(())
            }),
            GoalRuntimeEvent::ToolCompletedGoal { turn_context } => Box::pin(async move {
                self.account_thread_goal_progress(
                    turn_context,
                    BudgetLimitSteering::Suppressed,
                    TerminalMetricEmission::Suppress,
                )
                .await?;
                Ok(())
            }),
            GoalRuntimeEvent::TurnFinished {
                turn_context,
                turn_completed,
            } => Box::pin(async move {
                self.finish_thread_goal_turn(turn_context, turn_completed)
                    .await;
                Ok(())
            }),
            GoalRuntimeEvent::MaybeContinueIfIdle => Box::pin(async move {
                self.maybe_continue_goal_if_idle_runtime().await;
                Ok(())
            }),
            GoalRuntimeEvent::TaskAborted {
                turn_context,
                reason,
            } => Box::pin(async move {
                self.handle_thread_goal_task_abort(turn_context, reason)
                    .await;
                Ok(())
            }),
            GoalRuntimeEvent::ExternalMutationStarting => Box::pin(async move {
                if let Err(err) = self.account_thread_goal_before_external_mutation().await {
                    tracing::warn!(
                        "failed to account thread goal progress before external mutation: {err}"
                    );
                }
                Ok(())
            }),
            GoalRuntimeEvent::ExternalSet { external_set } => Box::pin(async move {
                self.apply_external_thread_goal_status(external_set).await;
                Ok(())
            }),
            GoalRuntimeEvent::ExternalClear => Box::pin(async move {
                self.clear_stopped_thread_goal_runtime_state().await;
                Ok(())
            }),
            GoalRuntimeEvent::ThreadResumed => Box::pin(async move {
                self.restore_thread_goal_runtime_after_resume().await?;
                Ok(())
            }),
        }
    }

    pub(crate) async fn get_thread_goal(&self) -> anyhow::Result<Option<ThreadGoal>> {
        if !self.enabled(Feature::Goals) {
            anyhow::bail!("goals feature is disabled");
        }

        let state_db = self.require_state_db_for_thread_goals().await?;
        state_db
            .get_thread_goal(self.conversation_id)
            .await
            .map(|goal| goal.map(protocol_goal_from_state))
    }

    pub(crate) async fn set_thread_goal(
        &self,
        turn_context: &TurnContext,
        request: SetGoalRequest,
    ) -> anyhow::Result<ThreadGoal> {
        if !self.enabled(Feature::Goals) {
            anyhow::bail!("goals feature is disabled");
        }

        let SetGoalRequest {
            objective,
            status,
            token_budget,
        } = request;
        validate_thread_goal_budget(token_budget.flatten())?;
        let state_db = self.require_state_db_for_thread_goals().await?;
        let objective = objective.map(|objective| objective.trim().to_string());
        if let Some(objective) = objective.as_deref()
            && let Err(err) = validate_thread_goal_objective(objective)
        {
            anyhow::bail!("{err}");
        }

        self.account_thread_goal_wall_clock_usage(
            &state_db,
            codex_state_api::ThreadGoalAccountingMode::ActiveOnly,
            TerminalMetricEmission::Emit,
        )
        .await?;
        let mut replacing_goal = false;
        let previous_status;
        let goal = if let Some(objective) = objective.as_deref() {
            let existing_goal = state_db.get_thread_goal(self.conversation_id).await?;
            previous_status = existing_goal.as_ref().map(|goal| goal.status);
            if let Some(existing_goal) = existing_goal.as_ref() {
                state_db
                    .update_thread_goal(
                        self.conversation_id,
                        codex_state_api::ThreadGoalUpdate {
                            objective: Some(objective.to_string()),
                            status: status.map(state_goal_status_from_protocol),
                            token_budget,
                            expected_goal_id: Some(existing_goal.goal_id.clone()),
                        },
                    )
                    .await?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "cannot update goal for thread {}: no goal exists",
                            self.conversation_id
                        )
                    })?
            } else {
                replacing_goal = true;
                state_db
                    .replace_thread_goal(
                        self.conversation_id,
                        objective,
                        status
                            .map(state_goal_status_from_protocol)
                            .unwrap_or(codex_state_api::ThreadGoalStatus::Active),
                        token_budget.flatten(),
                    )
                    .await?
            }
        } else {
            let existing_goal = state_db.get_thread_goal(self.conversation_id).await?;
            previous_status = existing_goal.as_ref().map(|goal| goal.status);
            let expected_goal_id = existing_goal.map(|goal| goal.goal_id);
            let status = status.map(state_goal_status_from_protocol);
            state_db
                .update_thread_goal(
                    self.conversation_id,
                    codex_state_api::ThreadGoalUpdate {
                        objective: None,
                        status,
                        token_budget,
                        expected_goal_id,
                    },
                )
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot update goal for thread {}: no goal exists",
                        self.conversation_id
                    )
                })?
        };

        let goal_status = goal.status;
        let goal_id = goal.goal_id.clone();
        let previous_status_for_goal = if replacing_goal {
            None
        } else {
            previous_status
        };
        if replacing_goal {
            self.emit_goal_created_metric();
        }
        self.emit_goal_terminal_metrics_if_status_changed(previous_status_for_goal, &goal);
        let goal = protocol_goal_from_state(goal);
        *self.goal_runtime.budget_limit_reported_goal_id.lock().await = None;
        let newly_active_goal = goal_status == codex_state_api::ThreadGoalStatus::Active
            && (replacing_goal
                || previous_status
                    .is_some_and(|status| status != codex_state_api::ThreadGoalStatus::Active));
        if newly_active_goal {
            let current_token_usage = self.total_token_usage().await.unwrap_or_default();
            self.mark_active_goal_accounting(
                goal_id,
                Some(turn_context.sub_id.clone()),
                current_token_usage,
            )
            .await;
        } else if goal_status != codex_state_api::ThreadGoalStatus::Active {
            self.clear_active_goal_accounting(turn_context).await;
        }
        self.send_event(
            turn_context,
            EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
                thread_id: self.conversation_id,
                turn_id: Some(turn_context.sub_id.clone()),
                goal: goal.clone(),
            }),
        )
        .await;
        self.record_thread_goal_update_item(turn_context, goal.clone(), previous_status_for_goal)
            .await;
        if goal_status != codex_state_api::ThreadGoalStatus::Active {
            self.maybe_notify_parent_of_final_status_for_current_source()
                .await;
        }
        Ok(goal)
    }

    pub(crate) async fn create_thread_goal(
        &self,
        turn_context: &TurnContext,
        request: CreateGoalRequest,
    ) -> anyhow::Result<ThreadGoal> {
        if !self.enabled(Feature::Goals) {
            anyhow::bail!("goals feature is disabled");
        }

        let CreateGoalRequest {
            objective,
            token_budget,
        } = request;
        validate_thread_goal_budget(token_budget)?;
        let objective = objective.trim();
        validate_thread_goal_objective(objective).map_err(anyhow::Error::msg)?;

        let state_db = self.require_state_db_for_thread_goals().await?;
        self.account_thread_goal_wall_clock_usage(
            &state_db,
            codex_state_api::ThreadGoalAccountingMode::ActiveOnly,
            TerminalMetricEmission::Emit,
        )
        .await?;
        let goal = state_db
            .insert_thread_goal(
                self.conversation_id,
                objective,
                codex_state_api::ThreadGoalStatus::Active,
                token_budget,
            )
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot create a new goal because thread {} already has a goal",
                    self.conversation_id
                )
            })?;

        let goal_id = goal.goal_id.clone();
        self.emit_goal_created_metric();
        let goal = protocol_goal_from_state(goal);
        *self.goal_runtime.budget_limit_reported_goal_id.lock().await = None;

        let current_token_usage = self.total_token_usage().await.unwrap_or_default();
        self.mark_active_goal_accounting(
            goal_id,
            Some(turn_context.sub_id.clone()),
            current_token_usage,
        )
        .await;

        self.send_event(
            turn_context,
            EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
                thread_id: self.conversation_id,
                turn_id: Some(turn_context.sub_id.clone()),
                goal: goal.clone(),
            }),
        )
        .await;
        self.record_thread_goal_update_item(turn_context, goal.clone(), None)
            .await;
        Ok(goal)
    }

    async fn record_thread_goal_update_item(
        &self,
        turn_context: &TurnContext,
        goal: ThreadGoal,
        previous_status: Option<codex_state_api::ThreadGoalStatus>,
    ) {
        let item = thread_goal_update_response_item(
            goal,
            previous_status,
            ThreadGoalUpdateEventSource::ModelTool,
        );
        self.record_model_items_and_emit_display_events(turn_context, std::slice::from_ref(&item))
            .await;
    }

    async fn apply_external_thread_goal_status(self: &Arc<Self>, external_set: ExternalGoalSet) {
        let ExternalGoalSet {
            goal,
            previous_status,
        } = external_set;
        let previous_goal = match previous_status {
            ExternalGoalPreviousStatus::NewGoal => None,
            ExternalGoalPreviousStatus::Existing(goal) => Some(goal),
        };
        let replaced_existing_goal = previous_goal
            .as_ref()
            .is_some_and(|previous_goal| previous_goal.goal_id != goal.goal_id);
        if previous_goal.is_none() || replaced_existing_goal {
            self.emit_goal_created_metric();
        }
        let objective_changed = previous_goal
            .as_ref()
            .is_some_and(|previous_goal| previous_goal.objective != goal.objective);
        let previous_status = previous_goal
            .as_ref()
            .and_then(|previous_goal| (!replaced_existing_goal).then_some(previous_goal.status));
        self.emit_goal_terminal_metrics_if_status_changed(previous_status, &goal);
        let goal_for_steering = objective_changed.then(|| protocol_goal_from_state(goal.clone()));
        let goal_id = goal.goal_id;
        let status = goal.status;
        match status {
            codex_state_api::ThreadGoalStatus::Active => {
                let turn_id = self
                    .active_turn_context()
                    .await
                    .map(|turn_context| turn_context.sub_id.clone());
                let current_token_usage = self.total_token_usage().await.unwrap_or_default();
                self.mark_active_goal_accounting(goal_id, turn_id, current_token_usage)
                    .await;
                if let Some(goal) = goal_for_steering {
                    let item = goal_objective_updated_steering_item(&goal);
                    if self
                        .inject_hook_inspectable_items(vec![item])
                        .await
                        .is_err()
                    {
                        tracing::debug!(
                            "skipping objective-updated goal steering because no turn is active"
                        );
                    }
                }
                self.maybe_continue_goal_if_idle_runtime().await;
            }
            codex_state_api::ThreadGoalStatus::BudgetLimited => {
                if self.active_turn_context().await.is_none() {
                    self.clear_stopped_thread_goal_runtime_state().await;
                }
            }
            codex_state_api::ThreadGoalStatus::Paused
            | codex_state_api::ThreadGoalStatus::Complete => {
                self.clear_stopped_thread_goal_runtime_state().await;
            }
        }
        if status != codex_state_api::ThreadGoalStatus::Active {
            self.maybe_notify_parent_of_final_status_for_current_source()
                .await;
        }
    }

    async fn clear_stopped_thread_goal_runtime_state(&self) {
        *self.goal_runtime.budget_limit_reported_goal_id.lock().await = None;
        let mut accounting = self.goal_runtime.accounting.lock().await;
        if let Some(turn) = accounting.turn.as_mut() {
            turn.clear_active_goal();
        }
        accounting.wall_clock.clear_active_goal();
    }

    async fn clear_active_goal_accounting(&self, turn_context: &TurnContext) {
        let mut accounting = self.goal_runtime.accounting.lock().await;
        if let Some(turn) = accounting.turn.as_mut()
            && turn.turn_id == turn_context.sub_id
        {
            turn.clear_active_goal();
        }
        accounting.wall_clock.clear_active_goal();
    }

    async fn mark_active_goal_accounting(
        &self,
        goal_id: String,
        turn_id: Option<String>,
        token_usage: TokenUsage,
    ) {
        let mut accounting = self.goal_runtime.accounting.lock().await;
        if let Some(turn_id) = turn_id {
            match accounting.turn.as_mut() {
                Some(turn) if turn.turn_id == turn_id => {
                    turn.reset_baseline(token_usage);
                    turn.mark_active_goal(goal_id.clone());
                }
                _ => {
                    let mut turn = GoalTurnAccountingSnapshot::new(turn_id, token_usage);
                    turn.mark_active_goal(goal_id.clone());
                    accounting.turn = Some(turn);
                }
            }
        }
        accounting.wall_clock.mark_active_goal(goal_id);
    }

    fn emit_goal_created_metric(&self) {
        self.services
            .session_telemetry
            .counter(GOAL_CREATED_METRIC, /*inc*/ 1, &[]);
    }

    fn emit_goal_terminal_metrics_if_status_changed(
        &self,
        previous_status: Option<codex_state_api::ThreadGoalStatus>,
        goal: &codex_state_api::ThreadGoal,
    ) {
        if previous_status == Some(goal.status) {
            return;
        }

        let counter = match goal.status {
            codex_state_api::ThreadGoalStatus::BudgetLimited => GOAL_BUDGET_LIMITED_METRIC,
            codex_state_api::ThreadGoalStatus::Complete => GOAL_COMPLETED_METRIC,
            codex_state_api::ThreadGoalStatus::Active
            | codex_state_api::ThreadGoalStatus::Paused => {
                return;
            }
        };
        let status_tag = [("status", goal.status.as_str())];
        self.services
            .session_telemetry
            .counter(counter, /*inc*/ 1, &[]);
        self.services.session_telemetry.histogram(
            GOAL_TOKEN_COUNT_METRIC,
            goal.tokens_used,
            &status_tag,
        );
        self.services.session_telemetry.histogram(
            GOAL_DURATION_SECONDS_METRIC,
            goal.time_used_seconds,
            &status_tag,
        );
    }

    async fn current_goal_status_for_metrics(
        &self,
        state_db: &SharedStateDbRuntime,
        expected_goal_id: Option<&str>,
    ) -> anyhow::Result<Option<codex_state_api::ThreadGoalStatus>> {
        let goal = state_db.get_thread_goal(self.conversation_id).await?;
        Ok(goal.and_then(|goal| {
            expected_goal_id
                .is_none_or(|expected_goal_id| goal.goal_id == expected_goal_id)
                .then_some(goal.status)
        }))
    }

    async fn active_turn_context(&self) -> Option<Arc<TurnContext>> {
        let active = self.active_turn.lock().await;
        active
            .as_ref()
            .and_then(|active_turn| active_turn.tasks.values().next())
            .map(|task| Arc::clone(&task.turn_context))
    }

    async fn mark_thread_goal_turn_started(
        &self,
        turn_context: &TurnContext,
        token_usage: TokenUsage,
    ) {
        self.goal_runtime.accounting.lock().await.turn = Some(GoalTurnAccountingSnapshot::new(
            turn_context.sub_id.clone(),
            token_usage,
        ));

        if !self.enabled(Feature::Goals) {
            return;
        }
        if should_ignore_goal_for_mode(turn_context.collaboration_mode.mode) {
            self.clear_active_goal_accounting(turn_context).await;
            return;
        }
        let state_db = match self.state_db_for_thread_goals().await {
            Ok(Some(state_db)) => state_db,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!("failed to open state db at turn start: {err}");
                return;
            }
        };
        match state_db.get_thread_goal(self.conversation_id).await {
            Ok(Some(goal))
                if matches!(
                    goal.status,
                    codex_state_api::ThreadGoalStatus::Active
                        | codex_state_api::ThreadGoalStatus::BudgetLimited
                ) =>
            {
                let mut accounting = self.goal_runtime.accounting.lock().await;
                if let Some(turn) = accounting.turn.as_mut()
                    && turn.turn_id == turn_context.sub_id
                {
                    turn.mark_active_goal(goal.goal_id.clone());
                }
                accounting.wall_clock.mark_active_goal(goal.goal_id);
            }
            Ok(Some(_)) | Ok(None) => {
                self.goal_runtime
                    .accounting
                    .lock()
                    .await
                    .wall_clock
                    .clear_active_goal();
            }
            Err(err) => {
                tracing::warn!("failed to read thread goal at turn start: {err}");
            }
        }
    }

    async fn mark_thread_goal_continuation_turn_started(&self, turn_id: String) {
        self.goal_runtime
            .mark_continuation_turn_started(turn_id)
            .await;
    }

    async fn take_thread_goal_continuation_turn(&self, turn_id: &str) -> bool {
        self.goal_runtime.take_continuation_turn(turn_id).await
    }

    async fn clear_reserved_goal_continuation_turn(&self, turn_state: &Arc<Mutex<TurnState>>) {
        let mut active_turn_guard = self.active_turn.lock().await;
        if let Some(active_turn) = active_turn_guard.as_ref()
            && active_turn.tasks.is_empty()
            && Arc::ptr_eq(&active_turn.turn_state, turn_state)
        {
            *active_turn_guard = None;
        }
    }

    async fn finish_thread_goal_turn(
        self: &Arc<Self>,
        turn_context: &TurnContext,
        turn_completed: bool,
    ) {
        if turn_completed
            && let Err(err) = self
                .account_thread_goal_progress(
                    turn_context,
                    BudgetLimitSteering::Suppressed,
                    TerminalMetricEmission::Emit,
                )
                .await
        {
            tracing::warn!("failed to account thread goal progress at turn end: {err}");
        }

        self.take_thread_goal_continuation_turn(&turn_context.sub_id)
            .await;
        if turn_completed {
            let mut accounting = self.goal_runtime.accounting.lock().await;
            if accounting
                .turn
                .as_ref()
                .is_some_and(|turn| turn.turn_id == turn_context.sub_id)
            {
                accounting.turn = None;
            }
        }
    }

    async fn handle_thread_goal_task_abort(
        &self,
        turn_context: Option<&TurnContext>,
        reason: TurnAbortReason,
    ) {
        if let Some(turn_context) = turn_context {
            self.take_thread_goal_continuation_turn(&turn_context.sub_id)
                .await;
            if let Err(err) = self
                .account_thread_goal_progress(
                    turn_context,
                    BudgetLimitSteering::Suppressed,
                    TerminalMetricEmission::Emit,
                )
                .await
            {
                tracing::warn!("failed to account thread goal progress after abort: {err}");
            }
            let mut accounting = self.goal_runtime.accounting.lock().await;
            if accounting
                .turn
                .as_ref()
                .is_some_and(|turn| turn.turn_id == turn_context.sub_id)
            {
                accounting.turn = None;
            }
        }

        if reason == TurnAbortReason::Interrupted
            && let Err(err) = self.pause_active_thread_goal_for_interrupt().await
        {
            tracing::warn!("failed to pause active thread goal after interrupt: {err}");
        }
    }

    async fn account_thread_goal_progress(
        &self,
        turn_context: &TurnContext,
        budget_limit_steering: BudgetLimitSteering,
        terminal_metric_emission: TerminalMetricEmission,
    ) -> anyhow::Result<()> {
        if !self.enabled(Feature::Goals) {
            return Ok(());
        }
        if should_ignore_goal_for_mode(turn_context.collaboration_mode.mode) {
            return Ok(());
        }
        let Some(state_db) = self.state_db_for_thread_goals().await? else {
            return Ok(());
        };
        let _accounting_permit = self.goal_runtime.accounting_permit().await?;
        let current_token_usage = self.total_token_usage().await.unwrap_or_default();
        let (token_delta, expected_goal_id, time_delta_seconds) = {
            let accounting = self.goal_runtime.accounting.lock().await;
            let Some(turn) = accounting
                .turn
                .as_ref()
                .filter(|turn| turn.turn_id == turn_context.sub_id)
            else {
                return Ok(());
            };
            if !turn.active_this_turn() {
                return Ok(());
            }
            (
                turn.token_delta_since_last_accounting(&current_token_usage),
                turn.active_goal_id(),
                accounting.wall_clock.time_delta_since_last_accounting(),
            )
        };
        if time_delta_seconds == 0 && token_delta <= 0 {
            return Ok(());
        }
        let previous_status = self
            .current_goal_status_for_metrics(&state_db, expected_goal_id.as_deref())
            .await?;
        let outcome = state_db
            .account_thread_goal_usage(
                self.conversation_id,
                time_delta_seconds,
                token_delta,
                codex_state_api::ThreadGoalAccountingMode::ActiveOnly,
                expected_goal_id.as_deref(),
            )
            .await?;
        let budget_limit_was_already_reported = {
            let reported_goal_id = self.goal_runtime.budget_limit_reported_goal_id.lock().await;
            expected_goal_id
                .as_deref()
                .is_some_and(|goal_id| reported_goal_id.as_deref() == Some(goal_id))
        };
        let goal = match outcome {
            codex_state_api::ThreadGoalAccountingOutcome::Updated(goal) => {
                let clear_active_goal = match goal.status {
                    codex_state_api::ThreadGoalStatus::Active => false,
                    codex_state_api::ThreadGoalStatus::BudgetLimited => {
                        matches!(budget_limit_steering, BudgetLimitSteering::Suppressed)
                    }
                    codex_state_api::ThreadGoalStatus::Paused
                    | codex_state_api::ThreadGoalStatus::Complete => true,
                };
                {
                    let mut accounting = self.goal_runtime.accounting.lock().await;
                    if let Some(turn) = accounting
                        .turn
                        .as_mut()
                        .filter(|turn| turn.turn_id == turn_context.sub_id)
                    {
                        turn.mark_accounted(current_token_usage);
                        if clear_active_goal {
                            turn.clear_active_goal();
                        }
                    }
                    accounting.wall_clock.mark_accounted(time_delta_seconds);
                    if clear_active_goal {
                        accounting.wall_clock.clear_active_goal();
                    }
                }
                if matches!(terminal_metric_emission, TerminalMetricEmission::Emit) {
                    self.emit_goal_terminal_metrics_if_status_changed(previous_status, &goal);
                }
                goal
            }
            codex_state_api::ThreadGoalAccountingOutcome::Unchanged(_) => return Ok(()),
        };
        let should_steer_budget_limit =
            matches!(budget_limit_steering, BudgetLimitSteering::Allowed)
                && goal.status == codex_state_api::ThreadGoalStatus::BudgetLimited
                && !budget_limit_was_already_reported;
        let goal_status = goal.status;
        let goal_id = goal.goal_id.clone();
        if goal_status != codex_state_api::ThreadGoalStatus::BudgetLimited {
            *self.goal_runtime.budget_limit_reported_goal_id.lock().await = None;
        }
        let goal = protocol_goal_from_state(goal);
        self.send_event(
            turn_context,
            EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
                thread_id: self.conversation_id,
                turn_id: Some(turn_context.sub_id.clone()),
                goal: goal.clone(),
            }),
        )
        .await;
        if should_steer_budget_limit {
            let item = goal_budget_limit_steering_item(&goal);
            if self
                .inject_hook_inspectable_items(vec![item])
                .await
                .is_err()
            {
                tracing::debug!("skipping budget-limit goal steering because no turn is active");
            }
            *self.goal_runtime.budget_limit_reported_goal_id.lock().await = Some(goal_id);
        }
        Ok(())
    }

    async fn account_thread_goal_before_external_mutation(&self) -> anyhow::Result<()> {
        if let Some(turn_context) = self.active_turn_context().await {
            return self
                .account_thread_goal_progress(
                    turn_context.as_ref(),
                    BudgetLimitSteering::Suppressed,
                    TerminalMetricEmission::Emit,
                )
                .await;
        }

        let Some(state_db) = self.state_db_for_thread_goals().await? else {
            return Ok(());
        };
        self.account_thread_goal_wall_clock_usage(
            &state_db,
            codex_state_api::ThreadGoalAccountingMode::ActiveOnly,
            TerminalMetricEmission::Suppress,
        )
        .await?;
        Ok(())
    }

    async fn account_thread_goal_wall_clock_usage(
        &self,
        state_db: &SharedStateDbRuntime,
        mode: codex_state_api::ThreadGoalAccountingMode,
        terminal_metric_emission: TerminalMetricEmission,
    ) -> anyhow::Result<Option<ThreadGoal>> {
        let _accounting_permit = self.goal_runtime.accounting_permit().await?;
        let (time_delta_seconds, expected_goal_id) = {
            let accounting = self.goal_runtime.accounting.lock().await;
            (
                accounting.wall_clock.time_delta_since_last_accounting(),
                accounting.wall_clock.active_goal_id(),
            )
        };
        if time_delta_seconds == 0 {
            return Ok(None);
        }
        let previous_status = self
            .current_goal_status_for_metrics(state_db, expected_goal_id.as_deref())
            .await?;

        match state_db
            .account_thread_goal_usage(
                self.conversation_id,
                time_delta_seconds,
                /*token_delta*/ 0,
                mode,
                expected_goal_id.as_deref(),
            )
            .await?
        {
            codex_state_api::ThreadGoalAccountingOutcome::Updated(goal) => {
                if matches!(terminal_metric_emission, TerminalMetricEmission::Emit) {
                    self.emit_goal_terminal_metrics_if_status_changed(previous_status, &goal);
                }
                self.goal_runtime
                    .accounting
                    .lock()
                    .await
                    .wall_clock
                    .mark_accounted(time_delta_seconds);
                let goal = protocol_goal_from_state(goal);
                Ok(Some(goal))
            }
            codex_state_api::ThreadGoalAccountingOutcome::Unchanged(goal) => {
                {
                    let mut accounting = self.goal_runtime.accounting.lock().await;
                    accounting.wall_clock.reset_baseline();
                    accounting.wall_clock.clear_active_goal();
                }
                if let Some(goal) = goal {
                    let goal = protocol_goal_from_state(goal);
                    return Ok(Some(goal));
                }
                Ok(None)
            }
        }
    }

    async fn pause_active_thread_goal_for_interrupt(&self) -> anyhow::Result<()> {
        if should_ignore_goal_for_mode(self.collaboration_mode().await.mode) {
            return Ok(());
        }

        if !self.enabled(Feature::Goals) {
            return Ok(());
        }

        let _continuation_guard = self
            .goal_runtime
            .continuation_lock
            .acquire()
            .await
            .context("goal continuation semaphore closed")?;
        let Some(state_db) = self.state_db_for_thread_goals().await? else {
            return Ok(());
        };
        self.account_thread_goal_wall_clock_usage(
            &state_db,
            codex_state_api::ThreadGoalAccountingMode::ActiveStatusOnly,
            TerminalMetricEmission::Emit,
        )
        .await?;
        let Some(goal) = state_db
            .pause_active_thread_goal(self.conversation_id)
            .await?
        else {
            return Ok(());
        };
        let goal = protocol_goal_from_state(goal);
        *self.goal_runtime.budget_limit_reported_goal_id.lock().await = None;
        self.goal_runtime
            .accounting
            .lock()
            .await
            .wall_clock
            .clear_active_goal();
        self.send_event_raw(Event {
            id: uuid::Uuid::new_v4().to_string(),
            msg: EventMsg::ThreadGoalUpdated(ThreadGoalUpdatedEvent {
                thread_id: self.conversation_id,
                turn_id: None,
                goal,
            }),
        })
        .await;
        Ok(())
    }

    async fn restore_thread_goal_runtime_after_resume(&self) -> anyhow::Result<()> {
        if !self.enabled(Feature::Goals) {
            return Ok(());
        }
        if should_ignore_goal_for_mode(self.collaboration_mode().await.mode) {
            tracing::debug!(
                "skipping goal runtime restore while current collaboration mode ignores goals"
            );
            return Ok(());
        }

        let _continuation_guard = self
            .goal_runtime
            .continuation_lock
            .acquire()
            .await
            .context("goal continuation semaphore closed")?;
        let Some(state_db) = self.state_db_for_thread_goals().await? else {
            return Ok(());
        };
        let Some(goal) = state_db.get_thread_goal(self.conversation_id).await? else {
            self.clear_stopped_thread_goal_runtime_state().await;
            return Ok(());
        };
        match goal.status {
            codex_state_api::ThreadGoalStatus::Active => {
                self.goal_runtime
                    .accounting
                    .lock()
                    .await
                    .wall_clock
                    .mark_active_goal(goal.goal_id);
            }
            codex_state_api::ThreadGoalStatus::Paused
            | codex_state_api::ThreadGoalStatus::BudgetLimited
            | codex_state_api::ThreadGoalStatus::Complete => {
                self.clear_stopped_thread_goal_runtime_state().await;
            }
        }
        Ok(())
    }

    async fn maybe_continue_goal_if_idle_runtime(self: &Arc<Self>) {
        self.maybe_start_turn_for_pending_work().await;
        self.maybe_start_goal_continuation_turn().await;
    }

    async fn maybe_start_goal_continuation_turn(self: &Arc<Self>) {
        let Ok(_continuation_guard) = self.goal_runtime.continuation_lock.acquire().await else {
            tracing::warn!("goal continuation semaphore closed");
            return;
        };
        let Some(candidate) = self.goal_continuation_candidate_if_active().await else {
            return;
        };

        let turn_state = {
            let mut active_turn = self.active_turn.lock().await;
            if active_turn.is_some() {
                return;
            }
            let active_turn = active_turn.get_or_insert_with(ActiveTurn::default);
            Arc::clone(&active_turn.turn_state)
        };
        let goal_is_current = match self.state_db_for_thread_goals().await {
            Ok(Some(state_db)) => match state_db.get_thread_goal(self.conversation_id).await {
                Ok(Some(goal))
                    if goal.goal_id == candidate.goal_id
                        && goal.status == codex_state_api::ThreadGoalStatus::Active =>
                {
                    true
                }
                Ok(Some(_)) | Ok(None) => {
                    tracing::debug!(
                        "skipping active goal continuation because the goal changed before launch"
                    );
                    false
                }
                Err(err) => {
                    tracing::warn!("failed to re-read thread goal before continuation: {err}");
                    false
                }
            },
            Ok(None) => {
                tracing::debug!("skipping active goal continuation for ephemeral thread");
                false
            }
            Err(err) => {
                tracing::warn!("failed to open state db before goal continuation: {err}");
                false
            }
        };
        if !goal_is_current {
            self.clear_reserved_goal_continuation_turn(&turn_state)
                .await;
            return;
        }
        {
            let mut turn_state = turn_state.lock().await;
            for item in candidate.items {
                turn_state.push_pending_input(PendingInputItem::from(item));
            }
        }

        let turn_context = self
            .new_default_turn_with_sub_id(uuid::Uuid::new_v4().to_string())
            .await;
        self.maybe_emit_unknown_model_warning_for_turn(turn_context.as_ref())
            .await;
        let still_reserved = {
            let active_turn = self.active_turn.lock().await;
            active_turn.as_ref().is_some_and(|active_turn| {
                active_turn.tasks.is_empty() && Arc::ptr_eq(&active_turn.turn_state, &turn_state)
            })
        };
        if !still_reserved {
            self.clear_reserved_goal_continuation_turn(&turn_state)
                .await;
            return;
        }
        self.mark_thread_goal_continuation_turn_started(turn_context.sub_id.clone())
            .await;
        self.start_task(turn_context, Vec::new(), RegularTask::new())
            .await;
    }

    async fn goal_continuation_candidate_if_active(
        self: &Arc<Self>,
    ) -> Option<GoalContinuationCandidate> {
        let ThreadPostTurnState::GoContextContinuation { goal_id } =
            self.thread_post_turn_state().await
        else {
            return None;
        };
        let state_db = match self.state_db_for_thread_goals().await {
            Ok(Some(state_db)) => state_db,
            Ok(None) => {
                tracing::debug!("skipping active goal continuation for ephemeral thread");
                return None;
            }
            Err(err) => {
                tracing::warn!("failed to open state db for goal continuation: {err}");
                return None;
            }
        };
        let goal = match state_db.get_thread_goal(self.conversation_id).await {
            Ok(Some(goal)) => goal,
            Ok(None) => {
                tracing::debug!("skipping active goal continuation because no goal is set");
                return None;
            }
            Err(err) => {
                tracing::warn!("failed to read thread goal for continuation: {err}");
                return None;
            }
        };
        if goal.goal_id != goal_id || goal.status != codex_state_api::ThreadGoalStatus::Active {
            tracing::debug!(status = ?goal.status, "skipping inactive thread goal");
            return None;
        }
        if self.thread_post_turn_state().await
            != (ThreadPostTurnState::GoContextContinuation {
                goal_id: goal_id.clone(),
            })
        {
            tracing::debug!("skipping active goal continuation because pending work appeared");
            return None;
        }
        let goal = protocol_goal_from_state(goal);
        Some(GoalContinuationCandidate {
            goal_id,
            items: vec![goal_continuation_input_item(&goal)],
        })
    }

    pub(crate) async fn thread_post_turn_state(&self) -> ThreadPostTurnState {
        if self.active_turn.lock().await.is_some() || self.has_pending_turn_input().await {
            return select_thread_post_turn_state(ThreadPostTurnInputs {
                has_pending_turn_input: true,
                ..ThreadPostTurnInputs::default()
            });
        }
        if !self.enabled(Feature::Goals) {
            return self.thread_idle_or_completion().await;
        }
        if should_ignore_goal_for_mode(self.collaboration_mode().await.mode) {
            tracing::debug!("skipping active goal continuation while plan mode is active");
            return self.thread_idle_or_completion().await;
        }
        let state_db = match self.state_db_for_thread_goals().await {
            Ok(Some(state_db)) => state_db,
            Ok(None) => return self.thread_idle_or_completion().await,
            Err(err) => {
                tracing::warn!("failed to open state db for post-turn goal state: {err}");
                return self.thread_idle_or_completion().await;
            }
        };
        let goal = match state_db.get_thread_goal(self.conversation_id).await {
            Ok(Some(goal)) => goal,
            Ok(None) => return self.thread_idle_or_completion().await,
            Err(err) => {
                tracing::warn!("failed to read thread goal for post-turn state: {err}");
                return self.thread_idle_or_completion().await;
            }
        };
        match goal.status {
            codex_state_api::ThreadGoalStatus::Active => {
                select_thread_post_turn_state(ThreadPostTurnInputs {
                    active_goal_id: Some(goal.goal_id),
                    ..ThreadPostTurnInputs::default()
                })
            }
            codex_state_api::ThreadGoalStatus::Complete
            | codex_state_api::ThreadGoalStatus::Paused
            | codex_state_api::ThreadGoalStatus::BudgetLimited => {
                self.thread_idle_or_completion().await
            }
        }
    }

    async fn thread_idle_or_completion(&self) -> ThreadPostTurnState {
        if Box::pin(self.has_incomplete_direct_child()).await {
            return select_thread_post_turn_state(ThreadPostTurnInputs {
                has_incomplete_direct_child: true,
                ..ThreadPostTurnInputs::default()
            });
        }
        select_thread_post_turn_state(ThreadPostTurnInputs {
            has_wait_command: Box::pin(self.has_wait_command()).await,
            ..ThreadPostTurnInputs::default()
        })
    }
}

impl Session {
    async fn state_db_for_thread_goals(&self) -> anyhow::Result<Option<SharedStateDbRuntime>> {
        let config = self.get_config().await;
        if config.ephemeral {
            return Ok(None);
        }

        self.try_ensure_rollout_materialized()
            .await
            .context("failed to materialize rollout before opening state db for thread goals")?;

        let state_db: SharedStateDbRuntime = if let Some(state_db) = self.state_db() {
            state_db
        } else if let Some(state_db) = self.goal_runtime.state_db.lock().await.clone() {
            state_db
        } else {
            anyhow::bail!("thread goals require a local persisted thread with a state database");
        };

        let thread_metadata_present = state_db
            .get_thread(self.conversation_id)
            .await
            .context("failed to read thread metadata before reconciling thread goals")?
            .is_some();
        if !thread_metadata_present {
            let rollout_path = self
                .current_rollout_path()
                .await
                .context("failed to locate rollout before reconciling thread goals")?
                .ok_or_else(|| {
                    anyhow::anyhow!("thread goals require materialized thread metadata")
                })?;
            let metadata = self
                .thread_metadata_for_goal_state_db(rollout_path.as_path(), &config)
                .await;
            insert_thread_metadata_if_absent(
                state_db.as_ref(),
                metadata,
                "thread_goals_metadata_bootstrap",
            )
            .await
            .context("failed to insert thread metadata before reconciling thread goals")?;
            let thread_metadata_present = state_db
                .get_thread(self.conversation_id)
                .await
                .context("failed to read thread metadata after reconciling thread goals")?
                .is_some();
            if !thread_metadata_present {
                anyhow::bail!("thread metadata is unavailable after reconciling thread goals");
            }
        }

        *self.goal_runtime.state_db.lock().await = Some(state_db.clone());
        Ok(Some(state_db))
    }

    async fn thread_metadata_for_goal_state_db(
        &self,
        rollout_path: &Path,
        config: &crate::config::Config,
    ) -> codex_state_api::ThreadMetadata {
        let thread_config = self.thread_config_snapshot().await;
        let updated_at = rollout_modified_at_utc(rollout_path)
            .await
            .unwrap_or_else(Utc::now);
        let mut builder = codex_state_api::ThreadMetadataBuilder::new(
            self.conversation_id,
            rollout_path.to_path_buf(),
            updated_at,
            thread_config.session_source.clone(),
        );
        builder.updated_at = Some(updated_at);
        builder.thread_source = thread_config.thread_source;
        builder.model_provider = Some(config.model_provider_id.clone());
        builder.cwd = thread_config.cwd.to_path_buf();
        builder.sandbox_policy = thread_config.sandbox_policy();
        builder.approval_mode = thread_config.approval_policy;

        let mut metadata = builder.build(config.model_provider_id.as_str());
        metadata.model = Some(thread_config.model);
        metadata.reasoning_effort = thread_config.reasoning_effort;
        metadata
    }

    async fn require_state_db_for_thread_goals(&self) -> anyhow::Result<SharedStateDbRuntime> {
        self.state_db_for_thread_goals().await?.ok_or_else(|| {
            anyhow::anyhow!("thread goals require a persisted thread; this thread is ephemeral")
        })
    }
}

async fn rollout_modified_at_utc(path: &Path) -> Option<DateTime<Utc>> {
    tokio::fs::metadata(path)
        .await
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(DateTime::<Utc>::from)
}
