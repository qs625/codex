use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::DEFAULT_COMMAND_OUTPUT_MAX_BYTES;

pub const DEFAULT_COMMAND_OUTPUT_DELTA_MAX_BYTES: usize = 8192;
const DEFAULT_COMMAND_OUTPUT_BROADCAST_CAPACITY: usize = 64;

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
    /// The retained output is split across a prefix ("head") and suffix
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

pub type CommandOutputBuffer = Arc<Mutex<HeadTailBuffer>>;

/// Shared command output state used by initial exec and stdin follow-up waits.
#[derive(Clone)]
pub struct CommandOutputHandles {
    pub output_buffer: CommandOutputBuffer,
    pub output_notify: Arc<Notify>,
    pub output_closed: Arc<AtomicBool>,
    pub output_closed_notify: Arc<Notify>,
    pub cancellation_token: CancellationToken,
}

#[derive(Clone)]
pub struct CommandOutputRuntime {
    output_tx: broadcast::Sender<Vec<u8>>,
    output_buffer: CommandOutputBuffer,
    output_notify: Arc<Notify>,
    output_closed: Arc<AtomicBool>,
    output_closed_notify: Arc<Notify>,
    cancellation_token: CancellationToken,
    output_drained: Arc<Notify>,
}

impl Default for CommandOutputRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandOutputRuntime {
    pub fn new() -> Self {
        let output_buffer = Arc::new(Mutex::new(HeadTailBuffer::default()));
        let output_notify = Arc::new(Notify::new());
        let output_closed = Arc::new(AtomicBool::new(false));
        let output_closed_notify = Arc::new(Notify::new());
        let cancellation_token = CancellationToken::new();
        let output_drained = Arc::new(Notify::new());
        let (output_tx, _) = broadcast::channel(DEFAULT_COMMAND_OUTPUT_BROADCAST_CAPACITY);

        Self {
            output_tx,
            output_buffer,
            output_notify,
            output_closed,
            output_closed_notify,
            cancellation_token,
            output_drained,
        }
    }

    pub fn handles(&self) -> CommandOutputHandles {
        CommandOutputHandles {
            output_buffer: Arc::clone(&self.output_buffer),
            output_notify: Arc::clone(&self.output_notify),
            output_closed: Arc::clone(&self.output_closed),
            output_closed_notify: Arc::clone(&self.output_closed_notify),
            cancellation_token: self.cancellation_token.clone(),
        }
    }

    pub fn receiver(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    pub fn output_drained_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.output_drained)
    }

    pub fn close_output(&self) {
        self.output_closed.store(true, Ordering::Release);
        self.output_closed_notify.notify_waiters();
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub async fn snapshot_chunks(&self) -> Vec<Vec<u8>> {
        let guard = self.output_buffer.lock().await;
        guard.snapshot_chunks()
    }

    pub async fn push_chunk(&self, chunk: Vec<u8>) {
        let mut guard = self.output_buffer.lock().await;
        guard.push_chunk(chunk.clone());
        drop(guard);
        let _ = self.output_tx.send(chunk);
        self.output_notify.notify_waiters();
    }

    pub async fn pump_broadcast_receiver(self, mut receiver: broadcast::Receiver<Vec<u8>>) {
        loop {
            match receiver.recv().await {
                Ok(chunk) => self.push_chunk(chunk).await,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    self.close_output();
                    break;
                }
            };
        }
    }
}

pub async fn collect_output_until_deadline(
    output_handles: &CommandOutputHandles,
    mut pause_state: Option<watch::Receiver<bool>>,
    mut deadline: Instant,
) -> Vec<u8> {
    const POST_EXIT_CLOSE_WAIT_CAP: Duration = Duration::from_millis(50);

    let mut collected: Vec<u8> = Vec::with_capacity(4096);
    let mut exit_signal_received = output_handles.cancellation_token.is_cancelled();
    let mut post_exit_deadline: Option<Instant> = None;
    loop {
        extend_deadlines_while_paused(&mut pause_state, &mut deadline, &mut post_exit_deadline)
            .await;
        let drained_chunks: Vec<Vec<u8>>;
        let mut wait_for_output = None;
        {
            let mut guard = output_handles.output_buffer.lock().await;
            drained_chunks = guard.drain_chunks();
            if drained_chunks.is_empty() {
                wait_for_output = Some(output_handles.output_notify.notified());
            }
        }

        if drained_chunks.is_empty() {
            exit_signal_received |= output_handles.cancellation_token.is_cancelled();
            if exit_signal_received && output_handles.output_closed.load(Ordering::Acquire) {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining == Duration::ZERO {
                break;
            }

            if exit_signal_received {
                let now = Instant::now();
                let close_wait_deadline = *post_exit_deadline
                    .get_or_insert_with(|| now + remaining.min(POST_EXIT_CLOSE_WAIT_CAP));
                let close_wait_remaining = close_wait_deadline.saturating_duration_since(now);
                if close_wait_remaining == Duration::ZERO {
                    break;
                }
                let notified =
                    wait_for_output.unwrap_or_else(|| output_handles.output_notify.notified());
                let closed = output_handles.output_closed_notify.notified();
                tokio::pin!(notified);
                tokio::pin!(closed);
                tokio::select! {
                    _ = &mut notified => {}
                    _ = &mut closed => {}
                    _ = tokio::time::sleep(close_wait_remaining) => break,
                    _ = wait_for_pause_change(pause_state.as_ref()) => {}
                }
                continue;
            }

            let notified =
                wait_for_output.unwrap_or_else(|| output_handles.output_notify.notified());
            tokio::pin!(notified);
            let exit_notified = output_handles.cancellation_token.cancelled();
            tokio::pin!(exit_notified);
            tokio::select! {
                _ = &mut notified => {}
                _ = &mut exit_notified => exit_signal_received = true,
                _ = tokio::time::sleep(remaining) => break,
                _ = wait_for_pause_change(pause_state.as_ref()) => {}
            }
            continue;
        }

        for chunk in drained_chunks {
            collected.extend_from_slice(&chunk);
        }

        exit_signal_received |= output_handles.cancellation_token.is_cancelled();
        if Instant::now() >= deadline {
            break;
        }
    }

    collected
}

async fn extend_deadlines_while_paused(
    pause_state: &mut Option<watch::Receiver<bool>>,
    deadline: &mut Instant,
    post_exit_deadline: &mut Option<Instant>,
) {
    let Some(receiver) = pause_state.as_mut() else {
        return;
    };
    if !*receiver.borrow() {
        return;
    }

    let paused_at = Instant::now();
    while *receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }

    let paused_for = paused_at.elapsed();
    *deadline += paused_for;
    if let Some(post_exit_deadline) = post_exit_deadline.as_mut() {
        *post_exit_deadline += paused_for;
    }
}

async fn wait_for_pause_change(pause_state: Option<&watch::Receiver<bool>>) {
    match pause_state {
        Some(pause_state) => {
            let mut receiver = pause_state.clone();
            let _ = receiver.changed().await;
        }
        None => std::future::pending::<()>().await,
    }
}

pub fn split_valid_utf8_prefix(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    split_valid_utf8_prefix_with_max(buffer, DEFAULT_COMMAND_OUTPUT_DELTA_MAX_BYTES)
}

pub fn split_valid_utf8_prefix_with_max(buffer: &mut Vec<u8>, max_bytes: usize) -> Option<Vec<u8>> {
    if buffer.is_empty() {
        return None;
    }

    let max_len = buffer.len().min(max_bytes);
    let mut split = max_len;
    while split > 0 {
        if std::str::from_utf8(&buffer[..split]).is_ok() {
            let prefix = buffer[..split].to_vec();
            buffer.drain(..split);
            return Some(prefix);
        }

        if max_len - split > 4 {
            break;
        }
        split -= 1;
    }

    // If no valid UTF-8 prefix was found, emit the first byte so the stream
    // keeps making progress and the transcript reflects all bytes.
    let byte = buffer.drain(..1).collect();
    Some(byte)
}

pub async fn resolve_aggregated_output(
    transcript: &CommandOutputBuffer,
    fallback: String,
) -> String {
    let guard = transcript.lock().await;
    if guard.retained_bytes() == 0 {
        return fallback;
    }

    String::from_utf8_lossy(&guard.to_bytes()).to_string()
}
