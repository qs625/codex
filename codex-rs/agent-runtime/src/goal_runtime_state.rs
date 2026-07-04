use anyhow::Context;
use state_api::SharedStateDbRuntime;
use state_api::ThreadGoalAccountingSnapshot as GoalAccountingSnapshot;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::SemaphorePermit;

/// Mutable runtime state for thread-goal scheduling and accounting.
///
/// This owns the locks and cached state shared by goal accounting,
/// continuation scheduling, and budget-limit steering. Runtime host code still
/// decides when to read/write persisted goal state and when to inject model
/// context.
pub struct GoalRuntimeState {
    pub state_db: Mutex<Option<SharedStateDbRuntime>>,
    pub budget_limit_reported_goal_id: Mutex<Option<String>>,
    accounting_lock: Semaphore,
    pub accounting: Mutex<GoalAccountingSnapshot>,
    continuation_turn_id: Mutex<Option<String>>,
    pub continuation_lock: Semaphore,
}

impl GoalRuntimeState {
    pub fn new() -> Self {
        Self {
            state_db: Mutex::new(None),
            budget_limit_reported_goal_id: Mutex::new(None),
            accounting_lock: Semaphore::new(/*permits*/ 1),
            accounting: Mutex::new(GoalAccountingSnapshot::new()),
            continuation_turn_id: Mutex::new(None),
            continuation_lock: Semaphore::new(/*permits*/ 1),
        }
    }

    pub async fn accounting_permit(&self) -> anyhow::Result<SemaphorePermit<'_>> {
        self.accounting_lock
            .acquire()
            .await
            .context("goal accounting semaphore closed")
    }

    pub async fn mark_continuation_turn_started(&self, turn_id: String) {
        *self.continuation_turn_id.lock().await = Some(turn_id);
    }

    pub async fn take_continuation_turn(&self, turn_id: &str) -> bool {
        let mut continuation_turn_id = self.continuation_turn_id.lock().await;
        if continuation_turn_id.as_deref() == Some(turn_id) {
            *continuation_turn_id = None;
            true
        } else {
            false
        }
    }
}

impl Default for GoalRuntimeState {
    fn default() -> Self {
        Self::new()
    }
}
