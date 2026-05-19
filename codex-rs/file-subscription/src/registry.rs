use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use codex_core::ThreadManager;
use codex_core::UnifiedExecProcessManager;
use codex_file_watcher::FileWatcher;
use codex_file_watcher::FileWatcherEvent;
use codex_file_watcher::FileWatcherSubscriber;
use codex_file_watcher::Receiver;
use codex_file_watcher::WatchPath;
use codex_file_watcher::WatchRegistration;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::warn;

const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(200);

struct DebouncedReceiver {
    rx: Receiver,
    interval: Duration,
    changed_paths: HashSet<PathBuf>,
    next_allowance: Option<Instant>,
}

impl DebouncedReceiver {
    fn new(rx: Receiver, interval: Duration) -> Self {
        Self {
            rx,
            interval,
            changed_paths: HashSet::new(),
            next_allowance: None,
        }
    }

    async fn recv(&mut self) -> Option<FileWatcherEvent> {
        while self.changed_paths.is_empty() {
            self.changed_paths.extend(self.rx.recv().await?.paths);
        }
        let next_allowance = *self
            .next_allowance
            .get_or_insert_with(|| Instant::now() + self.interval);
        loop {
            tokio::select! {
                event = self.rx.recv() => self.changed_paths.extend(event?.paths),
                _ = tokio::time::sleep_until(next_allowance) => break,
            }
        }
        Some(FileWatcherEvent {
            paths: self.changed_paths.drain().collect(),
        })
    }
}

struct SubscriptionEntry {
    _cancel_tx: oneshot::Sender<()>,
    _subscriber: Option<FileWatcherSubscriber>,
    _registration: Option<WatchRegistration>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct SubscriptionKey {
    thread_id: ThreadId,
    subscription_id: String,
}

/// Session-scoped registry of active file subscriptions.
///
/// Each subscription spawns a background task that watches the given path
/// and injects `Op::UserInput` turns into the owning thread when changes
/// are detected. Subscriptions are automatically cancelled when their thread
/// stops via `cancel_all_for_thread`.
pub(crate) struct FsSubscriptionRegistry {
    file_watcher: Arc<FileWatcher>,
    thread_manager: Weak<ThreadManager>,
    state: AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
}

impl FsSubscriptionRegistry {
    pub(crate) fn new(file_watcher: Arc<FileWatcher>, thread_manager: Weak<ThreadManager>) -> Self {
        Self {
            file_watcher,
            thread_manager,
            state: AsyncMutex::new(HashMap::new()),
        }
    }

    async fn send_text_to_thread(
        thread_manager: &Weak<ThreadManager>,
        thread_id: ThreadId,
        text: String,
    ) -> Result<(), String> {
        let Some(thread_manager) = thread_manager.upgrade() else {
            return Err("thread manager unavailable".to_string());
        };
        let thread = thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|err| err.to_string())?;
        let _ = thread
            .submit(Op::UserInput {
                items: vec![UserInput::Text {
                    text,
                    text_elements: vec![],
                }],
                environments: None,
                final_output_json_schema: None,
                responsesapi_client_metadata: None,
            })
            .await;
        Ok(())
    }

    fn subscription_entry(
        cancel_tx: oneshot::Sender<()>,
        subscriber: Option<FileWatcherSubscriber>,
        registration: Option<WatchRegistration>,
    ) -> SubscriptionEntry {
        SubscriptionEntry {
            _cancel_tx: cancel_tx,
            _subscriber: subscriber,
            _registration: registration,
        }
    }

    /// Creates a new file subscription and spawns a background watcher task.
    pub(crate) async fn subscribe_file(
        &self,
        thread_id: ThreadId,
        path: PathBuf,
        recursive: bool,
        label: Option<String>,
        subscription_id: String,
    ) {
        let (subscriber, rx) = self.file_watcher.add_subscriber();
        let registration = subscriber.register_paths(vec![WatchPath {
            path: path.clone(),
            recursive,
        }]);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_manager = self.thread_manager.clone();
        let sub_id_for_log = subscription_id.clone();

        tokio::spawn(async move {
            let mut debounced = DebouncedReceiver::new(rx, DEBOUNCE_INTERVAL);
            tokio::pin!(cancel_rx);
            loop {
                let event = tokio::select! {
                    biased;
                    _ = &mut cancel_rx => break,
                    event = debounced.recv() => match event {
                        Some(e) => e,
                        None => break,
                    },
                };
                let mut changed_paths: Vec<PathBuf> = event.paths.into_iter().collect();
                changed_paths.sort();
                if changed_paths.is_empty() {
                    continue;
                }
                let paths_str = changed_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let label_part = label
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default();
                let text = format!("[File subscription{label_part}] File changed: {paths_str}");
                if let Err(err) = Self::send_text_to_thread(&thread_manager, thread_id, text).await
                {
                    if err != "thread manager unavailable" {
                        warn!(
                            "file subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                        );
                    }
                    break;
                }
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id,
            },
            Self::subscription_entry(cancel_tx, Some(subscriber), Some(registration)),
        );
    }

    pub(crate) async fn subscribe_timer(
        &self,
        thread_id: ThreadId,
        interval: Duration,
        label: Option<String>,
        subscription_id: String,
    ) {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_manager = self.thread_manager.clone();
        let sub_id_for_log = subscription_id.clone();

        tokio::spawn(async move {
            tokio::pin!(cancel_rx);
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                tokio::select! {
                    biased;
                    _ = &mut cancel_rx => break,
                    _ = ticker.tick() => {}
                }
                let label_part = label
                    .as_deref()
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default();
                let text = format!(
                    "[Timer subscription{label_part}] Interval elapsed: {} ms",
                    interval.as_millis()
                );
                if let Err(err) = Self::send_text_to_thread(&thread_manager, thread_id, text).await
                {
                    if err != "thread manager unavailable" {
                        warn!(
                            "timer subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                        );
                    }
                    break;
                }
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id,
            },
            Self::subscription_entry(cancel_tx, None, None),
        );
    }

    pub(crate) async fn subscribe_process_exit(
        &self,
        thread_id: ThreadId,
        process_id: i32,
        label: Option<String>,
        subscription_id: String,
        unified_exec_manager: Arc<UnifiedExecProcessManager>,
    ) -> bool {
        let Some(process_exit) = unified_exec_manager
            .subscribe_process_exit(process_id)
            .await
        else {
            return false;
        };

        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_manager = self.thread_manager.clone();
        let sub_id_for_log = subscription_id.clone();

        tokio::spawn(async move {
            tokio::pin!(cancel_rx);
            let exit_code = tokio::select! {
                biased;
                _ = &mut cancel_rx => return,
                exit_code = process_exit.wait() => exit_code,
            };
            let label_part = label
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            let text = match exit_code {
                Some(exit_code) => format!(
                    "[Process exit subscription{label_part}] Session {process_id} exited with code {exit_code}"
                ),
                None => {
                    format!("[Process exit subscription{label_part}] Session {process_id} exited")
                }
            };
            if let Err(err) = Self::send_text_to_thread(&thread_manager, thread_id, text).await
                && err != "thread manager unavailable"
            {
                warn!(
                    "process exit subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                );
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id,
            },
            Self::subscription_entry(cancel_tx, None, None),
        );
        true
    }

    /// Cancels a specific subscription. Returns `true` if the subscription existed.
    pub(crate) async fn unsubscribe(&self, thread_id: ThreadId, subscription_id: &str) -> bool {
        // Dropping the entry cancels the background task via cancel_tx.
        self.state
            .lock()
            .await
            .remove(&SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.to_string(),
            })
            .is_some()
    }

    /// Cancels all subscriptions belonging to the given thread.
    pub(crate) async fn cancel_all_for_thread(&self, thread_id: ThreadId) {
        // Dropping entries cancels their background tasks.
        self.state
            .lock()
            .await
            .extract_if(|key, _| key.thread_id == thread_id)
            .count();
    }
}
