use std::time::Duration;

use rand::Rng;
use rand::rng;

pub const MIN_YIELD_TIME_MS: u64 = 250;
pub const MAX_YIELD_TIME_MS: u64 = 30_000;
pub const DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub const DEFAULT_COMMAND_OUTPUT_MAX_BYTES: usize = 1024 * 1024; // 1 MiB
pub const WAIT_BACKOFF_MULTIPLIER: u32 = 2;

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
