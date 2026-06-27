use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use rand::Rng;
use rand::rng;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::CommandNotificationKind;

pub(crate) const DEFAULT_MAX_COMPLETED_COMMAND_PROCESSES: usize = 256;
const MIN_PROCESS_ID: i32 = 1_000;
const PROCESS_ID_SPAN: i32 = 99_000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProcessState {
    pub has_exited: bool,
    pub exit_code: Option<i32>,
    pub failure_message: Option<String>,
}

impl ProcessState {
    pub fn exited(&self, exit_code: Option<i32>) -> Self {
        Self {
            has_exited: true,
            exit_code,
            failure_message: self.failure_message.clone(),
        }
    }

    pub fn failed(&self, message: String) -> Self {
        Self {
            has_exited: true,
            exit_code: self.exit_code,
            failure_message: Some(message),
        }
    }
}

#[derive(Default)]
pub(crate) struct CommandNotificationState {
    inner: Mutex<CommandNotificationSnapshot>,
    notify: Notify,
    background_session_active: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommandNotificationSnapshot {
    sequence: u64,
    kind: Option<CommandNotificationKind>,
}

impl CommandNotificationState {
    pub fn activate_background_session(&self) {
        self.background_session_active
            .store(true, Ordering::Relaxed);
    }

    pub fn is_background_session_active(&self) -> bool {
        self.background_session_active.load(Ordering::Relaxed)
    }

    pub async fn snapshot(&self) -> CommandNotificationSnapshot {
        *self.inner.lock().await
    }

    pub async fn notify(&self, kind: CommandNotificationKind) {
        {
            let mut guard = self.inner.lock().await;
            guard.sequence += 1;
            guard.kind = Some(kind);
        }
        self.notify.notify_waiters();
    }

    pub async fn wait_after(
        &self,
        snapshot: CommandNotificationSnapshot,
    ) -> CommandNotificationKind {
        loop {
            let notified = self.notify.notified();
            {
                let guard = self.inner.lock().await;
                if guard.sequence > snapshot.sequence
                    && let Some(kind) = guard.kind
                {
                    return kind;
                }
            }
            notified.await;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandProcessPruneMeta {
    pub process_id: i32,
    pub last_used: Instant,
    pub has_exited: bool,
}

pub(crate) fn command_process_id_to_prune(meta: &[CommandProcessPruneMeta]) -> Option<i32> {
    if meta.is_empty() {
        return None;
    }

    let mut by_recency = meta.to_vec();
    by_recency.sort_by_key(|entry| std::cmp::Reverse(entry.last_used));
    let protected: HashSet<i32> = by_recency
        .iter()
        .take(8)
        .map(|entry| entry.process_id)
        .collect();

    let mut lru = meta.to_vec();
    lru.sort_by_key(|entry| entry.last_used);

    if let Some(entry) = lru
        .iter()
        .find(|entry| !protected.contains(&entry.process_id) && entry.has_exited)
    {
        return Some(entry.process_id);
    }

    lru.into_iter()
        .find(|entry| !protected.contains(&entry.process_id))
        .map(|entry| entry.process_id)
}

#[derive(Debug)]
pub(crate) struct CommandProcessIdAllocator {
    reserved_process_ids: HashSet<i32>,
    completed_processes: HashMap<i32, CompletedCommandProcess>,
    max_completed_processes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletedCommandProcess {
    pub exit_code: Option<i32>,
    completed_at: Instant,
}

impl CommandProcessIdAllocator {
    pub fn reserve_next(&mut self, deterministic: bool) -> i32 {
        loop {
            let process_id = if deterministic {
                self.next_deterministic_process_id()
            } else {
                rng().random_range(MIN_PROCESS_ID..MIN_PROCESS_ID + PROCESS_ID_SPAN)
            };

            if self.reserved_process_ids.contains(&process_id)
                || self.completed_processes.contains_key(&process_id)
            {
                continue;
            }

            self.reserved_process_ids.insert(process_id);
            return process_id;
        }
    }

    pub fn release_reservation(&mut self, process_id: i32) {
        self.reserved_process_ids.remove(&process_id);
    }

    pub fn clear_reservations(&mut self) {
        self.reserved_process_ids.clear();
    }

    pub fn mark_completed(&mut self, process_id: i32, exit_code: Option<i32>) {
        self.completed_processes.insert(
            process_id,
            CompletedCommandProcess {
                exit_code,
                completed_at: Instant::now(),
            },
        );
        self.prune_completed_processes();
    }

    pub fn completed_process(&self, process_id: i32) -> Option<&CompletedCommandProcess> {
        self.completed_processes.get(&process_id)
    }

    fn next_deterministic_process_id(&self) -> i32 {
        self.reserved_process_ids
            .iter()
            .chain(self.completed_processes.keys())
            .copied()
            .max()
            .map(|m| std::cmp::max(m, MIN_PROCESS_ID - 1) + 1)
            .unwrap_or(MIN_PROCESS_ID)
    }

    fn prune_completed_processes(&mut self) {
        while self.completed_processes.len() > self.max_completed_processes {
            let Some(process_id) = self
                .completed_processes
                .iter()
                .min_by_key(|(_, entry)| entry.completed_at)
                .map(|(process_id, _)| *process_id)
            else {
                return;
            };
            self.completed_processes.remove(&process_id);
        }
    }
}

impl Default for CommandProcessIdAllocator {
    fn default() -> Self {
        Self {
            reserved_process_ids: HashSet::new(),
            completed_processes: HashMap::new(),
            max_completed_processes: DEFAULT_MAX_COMPLETED_COMMAND_PROCESSES,
        }
    }
}
