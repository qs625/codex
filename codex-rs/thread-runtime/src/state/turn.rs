//! Turn-scoped state and active turn metadata scaffolding.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use codex_extension_api::ExtensionData;
use crate::TaskKind;
use crate::TurnState;

use crate::session::turn_context::TurnContext;
use crate::tasks::AnySessionTask;

/// Metadata about the currently running turn.
pub(crate) struct ActiveTurn {
    pub(crate) tasks: ActiveTasks,
    pub(crate) turn_state: Arc<Mutex<TurnState>>,
}

impl Default for ActiveTurn {
    fn default() -> Self {
        Self {
            tasks: ActiveTasks::default(),
            turn_state: Arc::new(Mutex::new(TurnState::default())),
        }
    }
}

pub(crate) struct RunningTask {
    pub(crate) done: Arc<Notify>,
    pub(crate) kind: TaskKind,
    pub(crate) task: Arc<dyn AnySessionTask>,
    pub(crate) cancellation_token: CancellationToken,
    pub(crate) handle: AbortOnDropHandle<()>,
    pub(crate) turn_context: Arc<TurnContext>,
    pub(crate) turn_extension_data: Arc<ExtensionData>,
    // Timer recorded when the task drops to capture the full turn duration.
    // Boxed so turn state does not expose the concrete telemetry timer type.
    pub(crate) _timer: Option<codex_session_telemetry_api::SessionTelemetryTimer>,
}

pub(crate) struct RemovedTask {
    pub(crate) records_turn_token_usage_on_span: bool,
    pub(crate) active_turn_is_empty: bool,
}

#[derive(Default)]
pub(crate) struct ActiveTasks {
    order: Vec<String>,
    tasks: HashMap<String, RunningTask>,
}

impl ActiveTasks {
    pub(crate) fn insert(&mut self, sub_id: String, task: RunningTask) {
        if !self.tasks.contains_key(&sub_id) {
            self.order.push(sub_id.clone());
        }
        self.tasks.insert(sub_id, task);
    }

    pub(crate) fn swap_remove(&mut self, sub_id: &str) -> Option<RunningTask> {
        let task = self.tasks.remove(sub_id)?;
        if let Some(index) = self.order.iter().position(|id| id == sub_id) {
            self.order.swap_remove(index);
        }
        Some(task)
    }

    pub(crate) fn drain(&mut self) -> Vec<RunningTask> {
        let order = std::mem::take(&mut self.order);
        let mut tasks = std::mem::take(&mut self.tasks);
        order
            .into_iter()
            .filter_map(|sub_id| tasks.remove(&sub_id))
            .collect()
    }

    pub(crate) fn get(&self, sub_id: &str) -> Option<&RunningTask> {
        self.tasks.get(sub_id)
    }

    pub(crate) fn first(&self) -> Option<(&String, &RunningTask)> {
        let sub_id = self.order.first()?;
        let task = self.tasks.get(sub_id)?;
        Some((sub_id, task))
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &RunningTask> {
        self.order
            .iter()
            .filter_map(|sub_id| self.tasks.get(sub_id))
    }

    pub(crate) fn contains_key(&self, sub_id: &str) -> bool {
        self.tasks.contains_key(sub_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

impl ActiveTurn {
    pub(crate) fn add_task(&mut self, task: RunningTask) {
        let sub_id = task.turn_context.sub_id.clone();
        self.tasks.insert(sub_id, task);
    }

    pub(crate) fn remove_task(&mut self, sub_id: &str) -> Option<RemovedTask> {
        let task = self.tasks.swap_remove(sub_id)?;
        let records_turn_token_usage_on_span = task.task.records_turn_token_usage_on_span();
        task.handle.detach();
        Some(RemovedTask {
            records_turn_token_usage_on_span,
            active_turn_is_empty: self.tasks.is_empty(),
        })
    }

    pub(crate) fn drain_tasks(&mut self) -> Vec<RunningTask> {
        self.tasks.drain()
    }
}

impl ActiveTurn {
    /// Clear any pending approvals and input buffered for the current turn.
    pub(crate) async fn clear_pending(&self) {
        let mut ts = self.turn_state.lock().await;
        ts.clear_pending();
    }
}
