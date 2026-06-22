use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rand::Rng;
use rand::rng;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

mod output;
pub mod output_decoding;
pub use output::CommandOutputBuffer;
pub use output::CommandOutputHandles;
pub use output::CommandOutputRuntime;
pub use output::DEFAULT_COMMAND_OUTPUT_DELTA_MAX_BYTES;
pub use output::HeadTailBuffer;
pub use output::collect_output_until_deadline;
pub use output::resolve_aggregated_output;
pub use output::split_valid_utf8_prefix;
pub use output::split_valid_utf8_prefix_with_max;
pub use output_decoding::bytes_to_string_smart;

pub const MIN_YIELD_TIME_MS: u64 = 250;
pub const MAX_YIELD_TIME_MS: u64 = 30_000;
pub const DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub const DEFAULT_COMMAND_OUTPUT_MAX_BYTES: usize = 1024 * 1024; // 1 MiB
pub const DEFAULT_COMMAND_OUTPUT_MAX_TOKENS: usize = DEFAULT_COMMAND_OUTPUT_MAX_BYTES / 4;
pub const DEFAULT_MAX_COMPLETED_COMMAND_PROCESSES: usize = 256;
pub const WAIT_BACKOFF_MULTIPLIER: u32 = 2;
const MIN_PROCESS_ID: i32 = 1_000;
const PROCESS_ID_SPAN: i32 = 99_000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessState {
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

#[derive(Debug)]
pub struct WriteStdinRequest<'a> {
    pub process_id: i32,
    pub input: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WriteStdinOutput {
    pub process_id: i32,
    pub call_id: String,
    pub bytes_written: usize,
}

#[derive(Debug)]
pub struct CommandWaitRequest {
    pub process_id: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandNotificationFilter {
    Output,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandNotificationKind {
    Output,
    Exit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandWaitStatus {
    Running,
    Completed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommandWaitOutput {
    pub process_id: i32,
    pub status: CommandWaitStatus,
    pub notification: Option<CommandNotificationKind>,
    pub exit_code: Option<i32>,
    pub wall_time: Duration,
    pub wait_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct WaitBackoffState {
    current_window: Duration,
    initial_window: Duration,
    max_window: Duration,
}

impl WaitBackoffState {
    pub fn new(initial_window: Duration, max_window: Duration) -> Self {
        let initial_window = initial_window.min(max_window);
        Self {
            current_window: initial_window,
            initial_window,
            max_window,
        }
    }

    pub fn current_window(&self) -> Duration {
        self.current_window
    }

    pub fn advance_after_timeout(&mut self) {
        self.current_window = self
            .current_window
            .saturating_mul(WAIT_BACKOFF_MULTIPLIER)
            .min(self.max_window);
    }

    pub fn reset_after_event(&mut self) {
        self.current_window = self.initial_window;
    }
}

#[derive(Default)]
pub struct CommandNotificationState {
    inner: Mutex<CommandNotificationSnapshot>,
    notify: Notify,
    background_session_active: AtomicBool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CommandNotificationSnapshot {
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

pub fn clamp_yield_time(yield_time_ms: u64) -> u64 {
    yield_time_ms.clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
}

pub fn resolve_max_tokens(max_tokens: Option<usize>) -> usize {
    max_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

pub fn generate_chunk_id() -> String {
    let mut rng = rng();
    (0..6)
        .map(|_| format!("{:x}", rng.random_range(0..16)))
        .collect()
}

#[derive(Debug)]
pub struct CommandProcessIdAllocator {
    reserved_process_ids: HashSet<i32>,
    completed_processes: HashMap<i32, CompletedCommandProcess>,
    max_completed_processes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedCommandProcess {
    pub exit_code: Option<i32>,
    completed_at: Instant,
}

impl CommandProcessIdAllocator {
    pub fn new(max_completed_processes: usize) -> Self {
        Self {
            max_completed_processes,
            ..Self::default()
        }
    }

    pub fn reserve_next(&mut self, deterministic: bool) -> i32 {
        loop {
            let process_id = if deterministic {
                self.next_deterministic_process_id()
            } else {
                random_process_id()
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

fn random_process_id() -> i32 {
    rng().random_range(MIN_PROCESS_ID..MIN_PROCESS_ID + PROCESS_ID_SPAN)
}

#[cfg(test)]
mod tests;
