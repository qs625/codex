use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use thread_service_api::ThreadPollEvent;
use thread_service_api::ThreadPollEventResult;
use thread_service_api::ThreadPollEventTimeoutMetadata;
use tokio::sync::Mutex;
use tokio::sync::watch;

#[derive(Clone, Debug, Default)]
pub(crate) struct ThreadWaitEventSnapshot {
    pub(crate) seq: u64,
    pub(crate) source: Option<ThreadWaitSource>,
    pub(crate) events: Vec<ThreadPollEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadWaitSource {
    UserInput,
    InterAgent,
    ChildCompletion,
    QueuedInput,
    AsyncInput,
    CommandOutput,
    CommandExit,
}

#[derive(Clone)]
pub(crate) struct ThreadWaitState {
    events: watch::Sender<ThreadWaitEventSnapshot>,
    backoff: Arc<Mutex<ThreadWaitBackoffState>>,
}

pub(crate) struct ThreadWaitWatcher {
    rx: watch::Receiver<ThreadWaitEventSnapshot>,
    baseline_seq: u64,
}

pub(crate) enum ThreadWaitOutcome {
    Event {
        snapshot: ThreadWaitEventSnapshot,
        waited_ms: i64,
    },
    Timeout {
        waited_ms: i64,
    },
}

#[derive(Debug, Default)]
pub(crate) struct ThreadWaitBackoffState {
    current_window: Option<Duration>,
}

const THREAD_WAIT_BACKOFF_MULTIPLIER: u32 = 2;

impl Default for ThreadWaitState {
    fn default() -> Self {
        let (events, _rx) = watch::channel(ThreadWaitEventSnapshot::default());
        Self {
            events,
            backoff: Arc::new(Mutex::new(ThreadWaitBackoffState::default())),
        }
    }
}

impl ThreadWaitState {
    pub(crate) fn subscribe(&self) -> watch::Receiver<ThreadWaitEventSnapshot> {
        self.events.subscribe()
    }

    pub(crate) fn begin_wait(&self) -> ThreadWaitWatcher {
        let mut rx = self.subscribe();
        let baseline_seq = rx.borrow_and_update().seq;
        ThreadWaitWatcher { rx, baseline_seq }
    }

    pub(crate) fn note_event(&self, source: ThreadWaitSource) {
        self.note_event_with_events(source, Vec::new());
    }

    pub(crate) fn note_event_with_events(
        &self,
        source: ThreadWaitSource,
        events: Vec<ThreadPollEvent>,
    ) {
        let current = self.events.borrow().clone();
        self.events.send_replace(ThreadWaitEventSnapshot {
            seq: current.seq + 1,
            source: Some(source),
            events,
        });
    }

    pub(crate) async fn current_window(
        &self,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> Duration {
        self.backoff
            .lock()
            .await
            .current_window(
                duration_from_config_ms(initial_timeout_ms),
                duration_from_config_ms(hard_cap_timeout_ms),
            )
    }

    pub(crate) async fn timeout_metadata(
        &self,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) -> ThreadPollEventTimeoutMetadata {
        let current_timeout_ms = self
            .current_window(initial_timeout_ms, hard_cap_timeout_ms)
            .await
            .as_millis() as i64;
        ThreadPollEventTimeoutMetadata {
            initial_timeout_ms,
            current_timeout_ms,
            hard_cap_timeout_ms,
        }
    }

    pub(crate) async fn reset_after_event(&self) {
        self.backoff.lock().await.reset_after_event();
    }

    pub(crate) async fn advance_after_timeout(
        &self,
        initial_timeout_ms: i64,
        hard_cap_timeout_ms: i64,
    ) {
        self.backoff.lock().await.advance_after_timeout(
            duration_from_config_ms(initial_timeout_ms),
            duration_from_config_ms(hard_cap_timeout_ms),
        );
    }

    pub(crate) async fn wait(
        &self,
        watcher: ThreadWaitWatcher,
        metadata: &ThreadPollEventTimeoutMetadata,
    ) -> ThreadWaitOutcome {
        let current_timeout = Duration::from_millis(metadata.current_timeout_ms as u64);
        let started = Instant::now();
        let wake_snapshot = tokio::time::timeout(current_timeout, watcher.wait()).await;
        match wake_snapshot {
            Ok(Some(snapshot)) => {
                self.reset_after_event().await;
                ThreadWaitOutcome::Event {
                    snapshot,
                    waited_ms: started.elapsed().as_millis() as i64,
                }
            }
            Ok(None) | Err(_) => {
                self.advance_after_timeout(metadata.initial_timeout_ms, metadata.hard_cap_timeout_ms)
                    .await;
                ThreadWaitOutcome::Timeout {
                    waited_ms: started.elapsed().as_millis() as i64,
                }
            }
        }
    }
}

impl ThreadWaitWatcher {
    async fn wait(mut self) -> Option<ThreadWaitEventSnapshot> {
        loop {
            if self.rx.changed().await.is_err() {
                return None;
            }
            let snapshot = self.rx.borrow_and_update().clone();
            if snapshot.seq > self.baseline_seq {
                return Some(snapshot);
            }
        }
    }
}

impl ThreadWaitBackoffState {
    pub(crate) fn current_window(
        &mut self,
        initial_window: Duration,
        max_window: Duration,
    ) -> Duration {
        let current_window = self
            .current_window
            .unwrap_or(initial_window)
            .clamp(initial_window, max_window);
        self.current_window = Some(current_window);
        current_window
    }

    pub(crate) fn advance_after_timeout(&mut self, initial_window: Duration, max_window: Duration) {
        let current_window = self.current_window(initial_window, max_window);
        self.current_window = Some(
            current_window
                .saturating_mul(THREAD_WAIT_BACKOFF_MULTIPLIER)
                .min(max_window),
        );
    }

    pub(crate) fn reset_after_event(&mut self) {
        self.current_window = None;
    }
}

impl ThreadWaitSource {
    pub(crate) fn source_hint(self) -> String {
        match self {
            ThreadWaitSource::UserInput => "user_input",
            ThreadWaitSource::InterAgent => "inter_agent",
            ThreadWaitSource::ChildCompletion => "child_completion",
            ThreadWaitSource::QueuedInput => "queued_input",
            ThreadWaitSource::AsyncInput => "async_input",
            ThreadWaitSource::CommandOutput => "command_output",
            ThreadWaitSource::CommandExit => "command_exit",
        }
        .to_string()
    }
}

pub(crate) fn poll_event_result(
    source_hint: Option<String>,
    event: Option<ThreadPollEvent>,
    events: Vec<ThreadPollEvent>,
    waited_ms: i64,
    metadata: ThreadPollEventTimeoutMetadata,
) -> ThreadPollEventResult {
    ThreadPollEventResult {
        timed_out: false,
        source_hint,
        event,
        events,
        waited_ms,
        initial_timeout_ms: metadata.initial_timeout_ms,
        current_timeout_ms: metadata.current_timeout_ms,
        hard_cap_timeout_ms: metadata.hard_cap_timeout_ms,
    }
}

pub(crate) fn poll_event_timeout_result(
    waited_ms: i64,
    metadata: ThreadPollEventTimeoutMetadata,
) -> ThreadPollEventResult {
    ThreadPollEventResult {
        timed_out: true,
        source_hint: None,
        event: None,
        events: Vec::new(),
        waited_ms,
        initial_timeout_ms: metadata.initial_timeout_ms,
        current_timeout_ms: metadata.current_timeout_ms,
        hard_cap_timeout_ms: metadata.hard_cap_timeout_ms,
    }
}

fn duration_from_config_ms(value: i64) -> Duration {
    Duration::from_millis(value.max(0) as u64)
}
