use std::future::Future;

use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use state_api::ExternalGoalSet;

const UPDATE_GOAL_TOOL_NAME: &str = "update_goal";

/// Runtime lifecycle events that can affect goal accounting, scheduling, or
/// model-visible steering.
///
/// Callers report the session event they observed; this module owns the policy
/// for how that event maps to host side effects. The host still owns concrete
/// persistence, event emission, metrics, and turn scheduling.
pub enum GoalRuntimeEvent<'a, Turn: ?Sized> {
    TurnStarted {
        turn_context: &'a Turn,
        token_usage: TokenUsage,
    },
    ToolCompleted {
        turn_context: &'a Turn,
        tool_name: &'a str,
    },
    ToolCompletedGoal {
        turn_context: &'a Turn,
    },
    TurnFinished {
        turn_context: &'a Turn,
        turn_completed: bool,
    },
    MaybeContinueIfIdle,
    TaskAborted {
        turn_context: Option<&'a Turn>,
        reason: TurnAbortReason,
    },
    ExternalMutationStarting,
    ExternalSet {
        external_set: ExternalGoalSet,
    },
    ExternalClear,
    ThreadResumed,
}

/// Host interface used by `codex-agent-runtime` goal lifecycle policy.
///
/// Implementations own concrete runtime side effects: state database IO,
/// telemetry, typed event emission, model-visible steering injection, and turn
/// scheduling. The agent runtime owns only the event-to-effect ordering policy.
pub trait GoalRuntimeLifecycleHost: Send + Sync {
    type Turn: ?Sized + Send + Sync;

    fn mark_goal_turn_started(
        &self,
        turn_context: &Self::Turn,
        token_usage: TokenUsage,
    ) -> impl Future<Output = ()> + Send;

    fn account_goal_progress_after_tool(
        &self,
        turn_context: &Self::Turn,
        budget_limit_steering: BudgetLimitSteering,
        terminal_metric_emission: TerminalMetricEmission,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn finish_goal_turn(
        &self,
        turn_context: &Self::Turn,
        turn_completed: bool,
    ) -> impl Future<Output = ()> + Send;

    fn maybe_continue_goal_if_idle(&self) -> impl Future<Output = ()> + Send;

    fn handle_goal_task_abort(
        &self,
        turn_context: Option<&Self::Turn>,
        reason: TurnAbortReason,
    ) -> impl Future<Output = ()> + Send;

    fn account_goal_before_external_mutation(
        &self,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn apply_external_goal_status(
        &self,
        external_set: ExternalGoalSet,
    ) -> impl Future<Output = ()> + Send;

    fn clear_stopped_goal_runtime_state(&self) -> impl Future<Output = ()> + Send;

    fn restore_goal_runtime_after_resume(&self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[derive(Clone, Copy)]
pub enum BudgetLimitSteering {
    Allowed,
    Suppressed,
}

#[derive(Clone, Copy)]
pub enum TerminalMetricEmission {
    Emit,
    Suppress,
}

pub async fn apply_goal_runtime_event<H>(
    host: &H,
    event: GoalRuntimeEvent<'_, H::Turn>,
) -> anyhow::Result<()>
where
    H: GoalRuntimeLifecycleHost + ?Sized,
{
    match event {
        GoalRuntimeEvent::TurnStarted {
            turn_context,
            token_usage,
        } => {
            host.mark_goal_turn_started(turn_context, token_usage).await;
        }
        GoalRuntimeEvent::ToolCompleted {
            turn_context,
            tool_name,
        } => {
            if tool_name != UPDATE_GOAL_TOOL_NAME {
                host.account_goal_progress_after_tool(
                    turn_context,
                    BudgetLimitSteering::Allowed,
                    TerminalMetricEmission::Emit,
                )
                .await?;
            }
        }
        GoalRuntimeEvent::ToolCompletedGoal { turn_context } => {
            host.account_goal_progress_after_tool(
                turn_context,
                BudgetLimitSteering::Suppressed,
                TerminalMetricEmission::Suppress,
            )
            .await?;
        }
        GoalRuntimeEvent::TurnFinished {
            turn_context,
            turn_completed,
        } => {
            host.finish_goal_turn(turn_context, turn_completed).await;
        }
        GoalRuntimeEvent::MaybeContinueIfIdle => {
            host.maybe_continue_goal_if_idle().await;
        }
        GoalRuntimeEvent::TaskAborted {
            turn_context,
            reason,
        } => {
            host.handle_goal_task_abort(turn_context, reason).await;
        }
        GoalRuntimeEvent::ExternalMutationStarting => {
            if let Err(err) = host.account_goal_before_external_mutation().await {
                tracing::warn!(
                    "failed to account thread goal progress before external mutation: {err}"
                );
            }
        }
        GoalRuntimeEvent::ExternalSet { external_set } => {
            host.apply_external_goal_status(external_set).await;
        }
        GoalRuntimeEvent::ExternalClear => {
            host.clear_stopped_goal_runtime_state().await;
        }
        GoalRuntimeEvent::ThreadResumed => {
            host.restore_goal_runtime_after_resume().await?;
        }
    }
    Ok(())
}
