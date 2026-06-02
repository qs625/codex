use codex_protocol::ThreadId;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::MutexGuard;

#[derive(Default)]
pub struct ActiveEventSubscriptionTracker {
    counts_by_thread_id: Mutex<HashMap<ThreadId, usize>>,
}

impl ActiveEventSubscriptionTracker {
    pub fn set_active_count(&self, thread_id: ThreadId, active_count: usize) {
        let mut counts_by_thread_id = self.counts_by_thread_id();
        if active_count == 0 {
            counts_by_thread_id.remove(&thread_id);
        } else {
            counts_by_thread_id.insert(thread_id, active_count);
        }
    }

    pub fn active_count(&self, thread_id: ThreadId) -> usize {
        self.counts_by_thread_id()
            .get(&thread_id)
            .copied()
            .unwrap_or(0)
    }

    fn counts_by_thread_id(&self) -> MutexGuard<'_, HashMap<ThreadId, usize>> {
        match self.counts_by_thread_id.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
