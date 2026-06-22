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
