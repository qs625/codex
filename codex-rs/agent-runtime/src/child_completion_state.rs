use protocol::ThreadId;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

/// Tracks child-completion delivery state from a parent thread's point of view.
///
/// This does not define whether a child thread is currently active. It only
/// tracks whether a previously-finished child still has a completion envelope
/// pending delivery to the parent, plus whether completion delivery is armed.
#[derive(Debug)]
pub struct ChildCompletionState {
    delivery_active: AtomicBool,
    pending_direct_child_completions: Mutex<HashMap<ThreadId, usize>>,
}

impl ChildCompletionState {
    pub fn new() -> Self {
        Self::with_delivery_active(true)
    }

    pub fn inactive() -> Self {
        Self::with_delivery_active(false)
    }

    fn with_delivery_active(delivery_active: bool) -> Self {
        Self {
            delivery_active: AtomicBool::new(delivery_active),
            pending_direct_child_completions: Mutex::new(HashMap::new()),
        }
    }

    pub fn mark_delivery_active(&self) {
        self.delivery_active.store(true, Ordering::SeqCst);
    }

    pub fn try_begin_delivery(&self) -> bool {
        self.delivery_active
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub async fn mark_pending(&self, child_thread_id: ThreadId) {
        let mut pending = self.pending_direct_child_completions.lock().await;
        *pending.entry(child_thread_id).or_default() += 1;
    }

    pub async fn mark_received(&self, child_thread_id: ThreadId) -> bool {
        let mut pending = self.pending_direct_child_completions.lock().await;
        remove_one_pending_completion(&mut pending, child_thread_id) && pending.is_empty()
    }

    pub async fn clear_pending(&self, child_thread_id: ThreadId) -> bool {
        let mut pending = self.pending_direct_child_completions.lock().await;
        let removed = pending.remove(&child_thread_id).is_some();
        removed && pending.is_empty()
    }

    pub async fn has_pending(&self) -> bool {
        !self
            .pending_direct_child_completions
            .lock()
            .await
            .is_empty()
    }

    pub async fn mark_received_many(&self, child_thread_ids: impl IntoIterator<Item = ThreadId>) {
        let mut pending = self.pending_direct_child_completions.lock().await;
        for child_thread_id in child_thread_ids {
            remove_one_pending_completion(&mut pending, child_thread_id);
        }
    }
}

impl Default for ChildCompletionState {
    fn default() -> Self {
        Self::new()
    }
}

fn remove_one_pending_completion(
    pending: &mut HashMap<ThreadId, usize>,
    child_thread_id: ThreadId,
) -> bool {
    let Some(count) = pending.get_mut(&child_thread_id) else {
        return false;
    };
    if *count > 1 {
        *count -= 1;
    } else {
        pending.remove(&child_thread_id);
    }
    true
}
