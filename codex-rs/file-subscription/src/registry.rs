use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use codex_core::ThreadManager;
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
    _subscriber: FileWatcherSubscriber,
    _registration: WatchRegistration,
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

    /// Creates a new file subscription and spawns a background watcher task.
    pub(crate) async fn subscribe(
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
                let Some(thread_manager) = thread_manager.upgrade() else {
                    break;
                };
                match thread_manager.get_thread(thread_id).await {
                    Ok(thread) => {
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
                    }
                    Err(err) => {
                        warn!(
                            "file subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                        );
                        break;
                    }
                }
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id,
            },
            SubscriptionEntry {
                _cancel_tx: cancel_tx,
                _subscriber: subscriber,
                _registration: registration,
            },
        );
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
