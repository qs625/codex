use state_api::ExternalGoalPreviousStatus;
use state_api::ThreadGoal;
use state_api::ThreadGoalStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalGoalStatusAction {
    Active { objective_changed: bool },
    BudgetLimited { clear_if_no_active_turn: bool },
    Stopped { clear_runtime_state: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadGoalMutationPlan {
    pub emit_created_metric: bool,
    pub previous_status_for_terminal_metrics: Option<ThreadGoalStatus>,
    pub previous_status_for_display: Option<ThreadGoalStatus>,
    pub newly_active_goal_id: Option<String>,
    pub clear_active_accounting: bool,
    pub notify_parent_final_status: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalGoalMutationPlan {
    pub emit_created_metric: bool,
    pub previous_status_for_terminal_metrics: Option<ThreadGoalStatus>,
    pub status_action: ExternalGoalStatusAction,
    pub notify_parent_final_status: bool,
}

pub fn create_thread_goal_mutation_plan(goal: &ThreadGoal) -> ThreadGoalMutationPlan {
    ThreadGoalMutationPlan {
        emit_created_metric: true,
        previous_status_for_terminal_metrics: None,
        previous_status_for_display: None,
        newly_active_goal_id: Some(goal.goal_id.clone()),
        clear_active_accounting: false,
        notify_parent_final_status: false,
    }
}

pub fn set_thread_goal_mutation_plan(
    previous_status: Option<ThreadGoalStatus>,
    replacing_goal: bool,
    goal: &ThreadGoal,
) -> ThreadGoalMutationPlan {
    let previous_status_for_display = if replacing_goal {
        None
    } else {
        previous_status
    };
    let newly_active_goal_id = (goal.status == ThreadGoalStatus::Active
        && (replacing_goal
            || previous_status.is_some_and(|status| status != ThreadGoalStatus::Active)))
    .then(|| goal.goal_id.clone());

    ThreadGoalMutationPlan {
        emit_created_metric: replacing_goal,
        previous_status_for_terminal_metrics: previous_status_for_display,
        previous_status_for_display,
        newly_active_goal_id,
        clear_active_accounting: goal.status != ThreadGoalStatus::Active,
        notify_parent_final_status: goal.status != ThreadGoalStatus::Active,
    }
}

pub fn external_goal_mutation_plan(
    previous_status: ExternalGoalPreviousStatus,
    goal: &ThreadGoal,
) -> ExternalGoalMutationPlan {
    let previous_goal = match previous_status {
        ExternalGoalPreviousStatus::NewGoal => None,
        ExternalGoalPreviousStatus::Existing(goal) => Some(goal),
    };
    let replaced_existing_goal = previous_goal
        .as_ref()
        .is_some_and(|previous_goal| previous_goal.goal_id != goal.goal_id);
    let objective_changed = previous_goal
        .as_ref()
        .is_some_and(|previous_goal| previous_goal.objective != goal.objective);
    let previous_status_for_terminal_metrics = previous_goal
        .as_ref()
        .and_then(|previous_goal| (!replaced_existing_goal).then_some(previous_goal.status));
    let status_action = match goal.status {
        ThreadGoalStatus::Active => ExternalGoalStatusAction::Active { objective_changed },
        ThreadGoalStatus::BudgetLimited => ExternalGoalStatusAction::BudgetLimited {
            clear_if_no_active_turn: true,
        },
        ThreadGoalStatus::Paused | ThreadGoalStatus::Complete => {
            ExternalGoalStatusAction::Stopped {
                clear_runtime_state: true,
            }
        }
    };

    ExternalGoalMutationPlan {
        emit_created_metric: previous_goal.is_none() || replaced_existing_goal,
        previous_status_for_terminal_metrics,
        status_action,
        notify_parent_final_status: goal.status != ThreadGoalStatus::Active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use protocol::ThreadId;
    use state_api::ExternalGoalPreviousGoal;

    fn goal(goal_id: &str, objective: &str, status: ThreadGoalStatus) -> ThreadGoal {
        ThreadGoal {
            thread_id: ThreadId::new(),
            goal_id: goal_id.to_string(),
            objective: objective.to_string(),
            status,
            token_budget: Some(100),
            tokens_used: 10,
            time_used_seconds: 20,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn create_plan_marks_new_active_goal() {
        let goal = goal("goal-1", "ship it", ThreadGoalStatus::Active);

        assert_eq!(
            create_thread_goal_mutation_plan(&goal),
            ThreadGoalMutationPlan {
                emit_created_metric: true,
                previous_status_for_terminal_metrics: None,
                previous_status_for_display: None,
                newly_active_goal_id: Some("goal-1".to_string()),
                clear_active_accounting: false,
                notify_parent_final_status: false,
            }
        );
    }

    #[test]
    fn set_plan_treats_replacement_as_new_goal() {
        let goal = goal("goal-2", "new objective", ThreadGoalStatus::Active);

        assert_eq!(
            set_thread_goal_mutation_plan(Some(ThreadGoalStatus::Complete), true, &goal),
            ThreadGoalMutationPlan {
                emit_created_metric: true,
                previous_status_for_terminal_metrics: None,
                previous_status_for_display: None,
                newly_active_goal_id: Some("goal-2".to_string()),
                clear_active_accounting: false,
                notify_parent_final_status: false,
            }
        );
    }

    #[test]
    fn set_plan_clears_and_notifies_for_non_active_status() {
        let goal = goal("goal-1", "done", ThreadGoalStatus::Complete);

        assert_eq!(
            set_thread_goal_mutation_plan(Some(ThreadGoalStatus::Active), false, &goal),
            ThreadGoalMutationPlan {
                emit_created_metric: false,
                previous_status_for_terminal_metrics: Some(ThreadGoalStatus::Active),
                previous_status_for_display: Some(ThreadGoalStatus::Active),
                newly_active_goal_id: None,
                clear_active_accounting: true,
                notify_parent_final_status: true,
            }
        );
    }

    #[test]
    fn external_plan_detects_replacement_and_active_objective_change() {
        let goal = goal("goal-2", "new objective", ThreadGoalStatus::Active);

        assert_eq!(
            external_goal_mutation_plan(
                ExternalGoalPreviousStatus::Existing(ExternalGoalPreviousGoal {
                    goal_id: "goal-1".to_string(),
                    status: ThreadGoalStatus::Complete,
                    objective: "old objective".to_string(),
                }),
                &goal,
            ),
            ExternalGoalMutationPlan {
                emit_created_metric: true,
                previous_status_for_terminal_metrics: None,
                status_action: ExternalGoalStatusAction::Active {
                    objective_changed: true,
                },
                notify_parent_final_status: false,
            }
        );
    }

    #[test]
    fn external_plan_keeps_terminal_previous_status_for_same_goal() {
        let goal = goal("goal-1", "same objective", ThreadGoalStatus::Complete);

        assert_eq!(
            external_goal_mutation_plan(
                ExternalGoalPreviousStatus::Existing(ExternalGoalPreviousGoal {
                    goal_id: "goal-1".to_string(),
                    status: ThreadGoalStatus::Active,
                    objective: "same objective".to_string(),
                }),
                &goal,
            ),
            ExternalGoalMutationPlan {
                emit_created_metric: false,
                previous_status_for_terminal_metrics: Some(ThreadGoalStatus::Active),
                status_action: ExternalGoalStatusAction::Stopped {
                    clear_runtime_state: true,
                },
                notify_parent_final_status: true,
            }
        );
    }
}
