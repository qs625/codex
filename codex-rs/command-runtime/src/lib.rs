use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rand::Rng;
use rand::rng;
use tokio::sync::Mutex;
use tokio::sync::Notify;

pub mod output_decoding;
pub use output_decoding::bytes_to_string_smart;

pub const MIN_YIELD_TIME_MS: u64 = 250;
pub const MAX_YIELD_TIME_MS: u64 = 30_000;
pub const DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub const DEFAULT_COMMAND_OUTPUT_MAX_BYTES: usize = 1024 * 1024; // 1 MiB
pub const DEFAULT_COMMAND_OUTPUT_MAX_TOKENS: usize = DEFAULT_COMMAND_OUTPUT_MAX_BYTES / 4;
pub const WAIT_BACKOFF_MULTIPLIER: u32 = 2;

/// A capped buffer that preserves a stable prefix ("head") and suffix ("tail"),
/// dropping the middle once it exceeds the configured maximum. The buffer is
/// symmetric meaning 50% of the capacity is allocated to the head and 50% is
/// allocated to the tail.
#[derive(Debug)]
pub struct HeadTailBuffer {
    max_bytes: usize,
    head_budget: usize,
    tail_budget: usize,
    head: VecDeque<Vec<u8>>,
    tail: VecDeque<Vec<u8>>,
    head_bytes: usize,
    tail_bytes: usize,
    omitted_bytes: usize,
}

impl Default for HeadTailBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_COMMAND_OUTPUT_MAX_BYTES)
    }
}

impl HeadTailBuffer {
    /// Create a new buffer that retains at most `max_bytes` of output.
    ///
    /// The retained output is split across a prefix ("head") and suffix ("tail")
    /// budget, dropping bytes from the middle once the limit is exceeded.
    pub fn new(max_bytes: usize) -> Self {
        let head_budget = max_bytes / 2;
        let tail_budget = max_bytes.saturating_sub(head_budget);
        Self {
            max_bytes,
            head_budget,
            tail_budget,
            head: VecDeque::new(),
            tail: VecDeque::new(),
            head_bytes: 0,
            tail_bytes: 0,
            omitted_bytes: 0,
        }
    }

    /// Total bytes currently retained by the buffer (head + tail).
    pub fn retained_bytes(&self) -> usize {
        self.head_bytes.saturating_add(self.tail_bytes)
    }

    /// Total bytes that were dropped from the middle due to the size cap.
    pub fn omitted_bytes(&self) -> usize {
        self.omitted_bytes
    }

    /// Append a chunk of bytes to the buffer.
    ///
    /// Bytes are first added to the head until the head budget is full; any
    /// remaining bytes are added to the tail, with older tail bytes being
    /// dropped to preserve the tail budget.
    pub fn push_chunk(&mut self, chunk: Vec<u8>) {
        if self.max_bytes == 0 {
            self.omitted_bytes = self.omitted_bytes.saturating_add(chunk.len());
            return;
        }

        if self.head_bytes < self.head_budget {
            let remaining_head = self.head_budget.saturating_sub(self.head_bytes);
            if chunk.len() <= remaining_head {
                self.head_bytes = self.head_bytes.saturating_add(chunk.len());
                self.head.push_back(chunk);
                return;
            }

            let (head_part, tail_part) = chunk.split_at(remaining_head);
            if !head_part.is_empty() {
                self.head_bytes = self.head_bytes.saturating_add(head_part.len());
                self.head.push_back(head_part.to_vec());
            }
            self.push_to_tail(tail_part.to_vec());
            return;
        }

        self.push_to_tail(chunk);
    }

    /// Snapshot the retained output as a list of chunks.
    ///
    /// The returned chunks are ordered as: head chunks first, then tail chunks.
    /// Omitted bytes are not represented in the snapshot.
    pub fn snapshot_chunks(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        out.extend(self.head.iter().cloned());
        out.extend(self.tail.iter().cloned());
        out
    }

    /// Return the retained output as a single byte vector.
    ///
    /// The output is formed by concatenating head chunks, then tail chunks.
    /// Omitted bytes are not represented in the returned value.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.retained_bytes());
        for chunk in self.head.iter() {
            out.extend_from_slice(chunk);
        }
        for chunk in self.tail.iter() {
            out.extend_from_slice(chunk);
        }
        out
    }

    /// Drain all retained chunks from the buffer and reset its state.
    ///
    /// The drained chunks are returned in head-then-tail order. Omitted bytes
    /// are discarded along with the retained content.
    pub fn drain_chunks(&mut self) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = self.head.drain(..).collect();
        out.extend(self.tail.drain(..));
        self.head_bytes = 0;
        self.tail_bytes = 0;
        self.omitted_bytes = 0;
        out
    }

    fn push_to_tail(&mut self, chunk: Vec<u8>) {
        if self.tail_budget == 0 {
            self.omitted_bytes = self.omitted_bytes.saturating_add(chunk.len());
            return;
        }

        if chunk.len() >= self.tail_budget {
            let start = chunk.len().saturating_sub(self.tail_budget);
            let kept = chunk[start..].to_vec();
            let dropped = chunk.len().saturating_sub(kept.len());
            self.omitted_bytes = self
                .omitted_bytes
                .saturating_add(self.tail_bytes)
                .saturating_add(dropped);
            self.tail.clear();
            self.tail_bytes = kept.len();
            self.tail.push_back(kept);
            return;
        }

        self.tail_bytes = self.tail_bytes.saturating_add(chunk.len());
        self.tail.push_back(chunk);
        self.trim_tail_to_budget();
    }

    fn trim_tail_to_budget(&mut self) {
        let mut excess = self.tail_bytes.saturating_sub(self.tail_budget);
        while excess > 0 {
            match self.tail.front_mut() {
                Some(front) if excess >= front.len() => {
                    excess -= front.len();
                    self.tail_bytes = self.tail_bytes.saturating_sub(front.len());
                    self.omitted_bytes = self.omitted_bytes.saturating_add(front.len());
                    self.tail.pop_front();
                }
                Some(front) => {
                    front.drain(..excess);
                    self.tail_bytes = self.tail_bytes.saturating_sub(excess);
                    self.omitted_bytes = self.omitted_bytes.saturating_add(excess);
                    break;
                }
                None => break,
            }
        }
    }
}

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

#[cfg(test)]
mod tests;
