/// Post-turn state selected by the thread scheduler after a local turn ends.
///
/// This is a runtime contract shared by goal continuation, parent completion
/// delivery, and thread status projection. It intentionally carries only the
/// stable scheduling outcome; the code that observes child/command/goal state
/// remains in the runtime owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadPostTurnState {
    ThreadActive,
    ThreadIdle(ThreadIdleReason),
    GoContextContinuation { goal_id: String },
    ThreadCompletion,
}

/// Reason a thread is idle instead of complete after a local turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadIdleReason {
    WaitCommand,
    WaitChild,
}

/// Snapshot of runtime facts needed to select the next post-turn scheduler state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadPostTurnInputs {
    pub has_pending_turn_input: bool,
    pub active_goal_id: Option<String>,
    pub has_active_direct_child: bool,
    pub has_wait_command: bool,
}

/// Selects the canonical post-turn state from already-collected runtime facts.
///
/// The ordering intentionally mirrors the thread lifecycle contract:
/// pending input keeps the thread active, an active goal continuation runs
/// before child/command idling, direct-child waiting is driven only by whether
/// a direct child thread is locally active, and command waiting is considered
/// only after child waiting.
pub fn select_thread_post_turn_state(inputs: ThreadPostTurnInputs) -> ThreadPostTurnState {
    if inputs.has_pending_turn_input {
        return ThreadPostTurnState::ThreadActive;
    }
    if let Some(goal_id) = inputs.active_goal_id {
        return ThreadPostTurnState::GoContextContinuation { goal_id };
    }
    if inputs.has_active_direct_child {
        return ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitChild);
    }
    if inputs.has_wait_command {
        return ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitCommand);
    }
    ThreadPostTurnState::ThreadCompletion
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_input_takes_precedence_over_goal_and_idle_reasons() {
        assert_eq!(
            select_thread_post_turn_state(ThreadPostTurnInputs {
                has_pending_turn_input: true,
                active_goal_id: Some("goal-1".to_string()),
                has_active_direct_child: true,
                has_wait_command: true,
            }),
            ThreadPostTurnState::ThreadActive
        );
    }

    #[test]
    fn active_goal_continuation_takes_precedence_over_child_and_command_waits() {
        assert_eq!(
            select_thread_post_turn_state(ThreadPostTurnInputs {
                active_goal_id: Some("goal-1".to_string()),
                has_active_direct_child: true,
                has_wait_command: true,
                ..ThreadPostTurnInputs::default()
            }),
            ThreadPostTurnState::GoContextContinuation {
                goal_id: "goal-1".to_string()
            }
        );
    }

    #[test]
    fn child_wait_takes_precedence_over_command_wait() {
        assert_eq!(
            select_thread_post_turn_state(ThreadPostTurnInputs {
                has_active_direct_child: true,
                has_wait_command: true,
                ..ThreadPostTurnInputs::default()
            }),
            ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitChild)
        );
    }

    #[test]
    fn command_wait_is_selected_when_no_child_is_incomplete() {
        assert_eq!(
            select_thread_post_turn_state(ThreadPostTurnInputs {
                has_wait_command: true,
                ..ThreadPostTurnInputs::default()
            }),
            ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitCommand)
        );
    }

    #[test]
    fn no_post_turn_work_completes_thread() {
        assert_eq!(
            select_thread_post_turn_state(ThreadPostTurnInputs::default()),
            ThreadPostTurnState::ThreadCompletion
        );
    }
}
