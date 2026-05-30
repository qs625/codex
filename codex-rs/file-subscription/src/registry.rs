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
use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
use codex_protocol::subscriptions::PersistedSubscription;
use codex_thread_store::ThreadMetadataPatch;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::warn;

use crate::SubscriptionActivityObserver;
use crate::tools::schedule::CompiledSchedule;

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
    persisted: PersistedSubscription,
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
    state: Arc<AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>>,
    activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
}

impl FsSubscriptionRegistry {
    pub(crate) fn new(
        file_watcher: Arc<FileWatcher>,
        thread_manager: Weak<ThreadManager>,
        activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
    ) -> Self {
        Self {
            file_watcher,
            thread_manager,
            state: Arc::new(AsyncMutex::new(HashMap::new())),
            activity_observer,
        }
    }

    async fn active_subscription_count(&self, thread_id: ThreadId) -> usize {
        self.state
            .lock()
            .await
            .keys()
            .filter(|key| key.thread_id == thread_id)
            .count()
    }

    async fn notify_active_subscription_count(&self, thread_id: ThreadId) {
        let Some(observer) = self.activity_observer.as_ref() else {
            return;
        };
        let active_count = self.active_subscription_count(thread_id).await;
        observer.active_subscription_count_changed(thread_id, active_count);
    }

    async fn send_trigger_to_thread(
        thread_manager: &Weak<ThreadManager>,
        thread_id: ThreadId,
        trigger: EventDrivenToolTrigger,
    ) -> Result<(), String> {
        let Some(thread_manager) = thread_manager.upgrade() else {
            return Err("thread manager unavailable".to_string());
        };
        let thread = thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|err| err.to_string())?;
        let _ = thread.append_message(trigger.to_response_item()).await;
        Ok(())
    }

    fn subscription_entry(
        cancel_tx: oneshot::Sender<()>,
        subscriber: Option<FileWatcherSubscriber>,
        registration: Option<WatchRegistration>,
        persisted: PersistedSubscription,
    ) -> SubscriptionEntry {
        SubscriptionEntry {
            _cancel_tx: cancel_tx,
            _subscriber: subscriber,
            _registration: registration,
            persisted,
        }
    }

    async fn persist_thread_subscriptions(
        thread_manager: &Weak<ThreadManager>,
        state: &AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
        thread_id: ThreadId,
    ) -> Result<(), String> {
        let subscriptions = {
            let state = state.lock().await;
            let mut subscriptions = state
                .iter()
                .filter(|(key, _)| key.thread_id == thread_id)
                .map(|(_, entry)| entry.persisted.clone())
                .collect::<Vec<_>>();
            subscriptions.sort_by(|left, right| subscription_id(left).cmp(subscription_id(right)));
            subscriptions
        };
        let Some(thread_manager) = thread_manager.upgrade() else {
            return Err("thread manager unavailable".to_string());
        };
        let thread = thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|err| err.to_string())?;
        thread
            .update_thread_metadata(
                ThreadMetadataPatch {
                    subscriptions: Some(subscriptions),
                    ..Default::default()
                },
                /*include_archived*/ true,
            )
            .await
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    async fn remove_subscription_and_persist(
        thread_manager: &Weak<ThreadManager>,
        state: &AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
        thread_id: ThreadId,
        subscription_id: &str,
    ) -> Result<bool, String> {
        let removed = state
            .lock()
            .await
            .remove(&SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.to_string(),
            })
            .is_some();
        if removed {
            Self::persist_thread_subscriptions(thread_manager, state, thread_id).await?;
        }
        Ok(removed)
    }

    async fn subscription_snapshot_from_history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<PersistedSubscription>, String> {
        let Some(thread_manager) = self.thread_manager.upgrade() else {
            return Err("thread manager unavailable".to_string());
        };
        let thread = thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|err| err.to_string())?;
        let stored = thread
            .read_thread(
                /*include_archived*/ true, /*include_history*/ true,
            )
            .await
            .map_err(|err| err.to_string())?;
        let subscriptions = stored
            .history
            .and_then(|history| {
                history.items.iter().rev().find_map(|item| match item {
                    codex_protocol::protocol::RolloutItem::SessionMeta(meta_line) => {
                        meta_line.meta.subscriptions.clone()
                    }
                    _ => None,
                })
            })
            .unwrap_or_default();
        Ok(subscriptions)
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
        self.subscribe_file_with_persistence(
            thread_id,
            path,
            recursive,
            label,
            subscription_id,
            /*persist_after*/ true,
        )
        .await;
    }

    async fn subscribe_file_with_persistence(
        &self,
        thread_id: ThreadId,
        path: PathBuf,
        recursive: bool,
        label: Option<String>,
        subscription_id: String,
        persist_after: bool,
    ) {
        let (subscriber, rx) = self.file_watcher.add_subscriber();
        let registration = subscriber.register_paths(vec![WatchPath {
            path: path.clone(),
            recursive,
        }]);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_manager = self.thread_manager.clone();
        let sub_id_for_log = subscription_id.clone();
        let persisted = PersistedSubscription::Fs {
            subscription_id: subscription_id.clone(),
            path: path.display().to_string(),
            recursive,
            label: label.clone(),
        };

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
                let trigger = EventDrivenToolTrigger {
                    tool: "fs_subscribe".to_string(),
                    title: "File watch triggered".to_string(),
                    text,
                };
                if let Err(err) =
                    Self::send_trigger_to_thread(&thread_manager, thread_id, trigger).await
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
                subscription_id: subscription_id.clone(),
            },
            Self::subscription_entry(cancel_tx, Some(subscriber), Some(registration), persisted),
        );
        self.notify_active_subscription_count(thread_id).await;
        if persist_after
            && let Err(err) =
                Self::persist_thread_subscriptions(&self.thread_manager, &self.state, thread_id)
                    .await
        {
            warn!("failed to persist file subscription {subscription_id}: {err}");
        }
    }

    pub(crate) async fn subscribe_schedule(
        &self,
        thread_id: ThreadId,
        schedule_spec: codex_protocol::subscriptions::ScheduleSpec,
        schedule: CompiledSchedule,
        label: Option<String>,
        subscription_id: String,
    ) {
        self.subscribe_schedule_with_persistence(
            thread_id,
            schedule_spec,
            schedule,
            label,
            subscription_id,
            /*persist_after*/ true,
        )
        .await;
    }

    async fn subscribe_schedule_with_persistence(
        &self,
        thread_id: ThreadId,
        schedule_spec: codex_protocol::subscriptions::ScheduleSpec,
        schedule: CompiledSchedule,
        label: Option<String>,
        subscription_id: String,
        persist_after: bool,
    ) {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_manager = self.thread_manager.clone();
        let state = Arc::clone(&self.state);
        let activity_observer = self.activity_observer.clone();
        let sub_id_for_log = subscription_id.clone();
        let persisted = PersistedSubscription::Schedule {
            subscription_id: subscription_id.clone(),
            schedule: schedule_spec,
            label: label.clone(),
        };

        tokio::spawn(async move {
            tokio::pin!(cancel_rx);
            loop {
                let now = chrono::Utc::now();
                let next_fire_at = match schedule.next_fire_at(now) {
                    Ok(next_fire_at) => next_fire_at,
                    Err(err) => {
                        warn!("schedule subscription {sub_id_for_log}: {err}");
                        break;
                    }
                };
                let delay = match (next_fire_at - now).to_std() {
                    Ok(delay) => delay,
                    Err(_) => Duration::from_secs(0),
                };
                tokio::select! {
                    biased;
                    _ = &mut cancel_rx => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                let label_part = label
                    .as_deref()
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default();
                let text = format!(
                    "[Schedule subscription{label_part}] Trigger fired: {}",
                    schedule.summary()
                );
                let trigger = EventDrivenToolTrigger {
                    tool: "schedule_subscribe".to_string(),
                    title: "Schedule triggered".to_string(),
                    text,
                };
                if let Err(err) =
                    Self::send_trigger_to_thread(&thread_manager, thread_id, trigger).await
                {
                    if err != "thread manager unavailable" {
                        warn!(
                            "schedule subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                        );
                    }
                    break;
                }
                if schedule.is_one_shot() {
                    if let Err(err) = Self::remove_subscription_and_persist(
                        &thread_manager,
                        &state,
                        thread_id,
                        &sub_id_for_log,
                    )
                    .await
                    {
                        warn!(
                            "failed to persist completed schedule subscription {sub_id_for_log}: {err}"
                        );
                    } else if let Some(observer) = activity_observer.as_ref() {
                        let active_count = state
                            .lock()
                            .await
                            .keys()
                            .filter(|key| key.thread_id == thread_id)
                            .count();
                        observer.active_subscription_count_changed(thread_id, active_count);
                    }
                    break;
                }
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.clone(),
            },
            Self::subscription_entry(cancel_tx, None, None, persisted),
        );
        self.notify_active_subscription_count(thread_id).await;
        if persist_after
            && let Err(err) =
                Self::persist_thread_subscriptions(&self.thread_manager, &self.state, thread_id)
                    .await
        {
            warn!("failed to persist schedule subscription {subscription_id}: {err}");
        }
    }

    pub(crate) async fn subscribe_process_exit(
        &self,
        thread_id: ThreadId,
        process_id: i32,
        label: Option<String>,
        subscription_id: String,
        unified_exec_manager: Arc<UnifiedExecProcessManager>,
    ) -> bool {
        self.subscribe_process_exit_with_persistence(
            thread_id,
            process_id,
            label,
            subscription_id,
            unified_exec_manager,
            /*persist_after*/ true,
        )
        .await
    }

    async fn subscribe_process_exit_with_persistence(
        &self,
        thread_id: ThreadId,
        process_id: i32,
        label: Option<String>,
        subscription_id: String,
        unified_exec_manager: Arc<UnifiedExecProcessManager>,
        persist_after: bool,
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
        let state = Arc::clone(&self.state);
        let activity_observer = self.activity_observer.clone();
        let persisted = PersistedSubscription::ProcessExit {
            subscription_id: subscription_id.clone(),
            session_id: process_id,
            label: label.clone(),
        };

        tokio::spawn(async move {
            tokio::pin!(cancel_rx);
            let (exit_code, retained_output) = tokio::select! {
                biased;
                _ = &mut cancel_rx => return,
                result = process_exit.wait_with_retained_output() => result,
            };
            let label_part = label
                .as_deref()
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            let mut text = match exit_code {
                Some(exit_code) => format!(
                    "[Process exit subscription{label_part}] Session {process_id} exited with code {exit_code}"
                ),
                None => {
                    format!("[Process exit subscription{label_part}] Session {process_id} exited")
                }
            };
            let retained_output = retained_output.trim();
            if !retained_output.is_empty() {
                text.push_str("\nCaptured output:\n");
                text.push_str(retained_output);
            }
            let trigger = EventDrivenToolTrigger {
                tool: "process_exit_subscribe".to_string(),
                title: "Process exited".to_string(),
                text,
            };
            if let Err(err) =
                Self::send_trigger_to_thread(&thread_manager, thread_id, trigger).await
                && err != "thread manager unavailable"
            {
                warn!(
                    "process exit subscription {sub_id_for_log}: thread {thread_id} unavailable: {err}"
                );
            }
            if let Err(err) = Self::remove_subscription_and_persist(
                &thread_manager,
                &state,
                thread_id,
                &sub_id_for_log,
            )
            .await
            {
                warn!(
                    "failed to persist completed process exit subscription {sub_id_for_log}: {err}"
                );
            } else if let Some(observer) = activity_observer.as_ref() {
                let active_count = state
                    .lock()
                    .await
                    .keys()
                    .filter(|key| key.thread_id == thread_id)
                    .count();
                observer.active_subscription_count_changed(thread_id, active_count);
            }
        });

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.clone(),
            },
            Self::subscription_entry(cancel_tx, None, None, persisted),
        );
        self.notify_active_subscription_count(thread_id).await;
        if persist_after
            && let Err(err) =
                Self::persist_thread_subscriptions(&self.thread_manager, &self.state, thread_id)
                    .await
        {
            warn!("failed to persist process exit subscription {subscription_id}: {err}");
        }
        true
    }

    /// Cancels a specific subscription. Returns `true` if the subscription existed.
    pub(crate) async fn unsubscribe(&self, thread_id: ThreadId, subscription_id: &str) -> bool {
        match Self::remove_subscription_and_persist(
            &self.thread_manager,
            &self.state,
            thread_id,
            subscription_id,
        )
        .await
        {
            Ok(unsubscribed) => {
                if unsubscribed {
                    self.notify_active_subscription_count(thread_id).await;
                }
                unsubscribed
            }
            Err(err) => {
                warn!("failed to persist subscription removal {subscription_id}: {err}");
                false
            }
        }
    }

    /// Cancels all subscriptions belonging to the given thread.
    pub(crate) async fn cancel_all_for_thread(&self, thread_id: ThreadId) {
        // Dropping entries cancels their background tasks.
        self.state
            .lock()
            .await
            .extract_if(|key, _| key.thread_id == thread_id)
            .count();
        self.notify_active_subscription_count(thread_id).await;
    }

    pub(crate) async fn restore_thread_subscriptions(
        &self,
        thread_id: ThreadId,
        unified_exec_manager: Option<Arc<UnifiedExecProcessManager>>,
    ) {
        let subscriptions = match self.subscription_snapshot_from_history(thread_id).await {
            Ok(subscriptions) => subscriptions,
            Err(err) => {
                warn!("failed to load persisted subscriptions for {thread_id}: {err}");
                return;
            }
        };
        let mut changed = false;
        for subscription in subscriptions {
            match subscription {
                PersistedSubscription::Fs {
                    subscription_id,
                    path,
                    recursive,
                    label,
                } => {
                    self.subscribe_file_with_persistence(
                        thread_id,
                        PathBuf::from(path),
                        recursive,
                        label,
                        subscription_id,
                        /*persist_after*/ false,
                    )
                    .await;
                }
                PersistedSubscription::Schedule {
                    subscription_id,
                    schedule,
                    label,
                } => match crate::tools::schedule::CompiledSchedule::compile(schedule.clone()) {
                    Ok(compiled) => {
                        self.subscribe_schedule_with_persistence(
                            thread_id,
                            schedule,
                            compiled,
                            label,
                            subscription_id,
                            /*persist_after*/ false,
                        )
                        .await;
                    }
                    Err(err) => {
                        changed = true;
                        let text = format!(
                            "[Schedule subscription restore] Failed to restore subscription {subscription_id}: {err}"
                        );
                        let _ = Self::send_trigger_to_thread(
                            &self.thread_manager,
                            thread_id,
                            EventDrivenToolTrigger {
                                tool: "schedule_subscribe".to_string(),
                                title: "Schedule restore failed".to_string(),
                                text,
                            },
                        )
                        .await;
                    }
                },
                PersistedSubscription::ProcessExit {
                    subscription_id,
                    session_id,
                    label,
                } => {
                    let restored = if let Some(unified_exec_manager) = unified_exec_manager.clone()
                    {
                        self.subscribe_process_exit_with_persistence(
                            thread_id,
                            session_id,
                            label.clone(),
                            subscription_id.clone(),
                            unified_exec_manager,
                            /*persist_after*/ false,
                        )
                        .await
                    } else {
                        false
                    };
                    if !restored {
                        changed = true;
                        let label_part = label
                            .as_deref()
                            .map(|value| format!(" ({value})"))
                            .unwrap_or_default();
                        let text = format!(
                            "[Process exit subscription restore{label_part}] Could not restore session {session_id} after restart because the original exec session is no longer available."
                        );
                        let _ = Self::send_trigger_to_thread(
                            &self.thread_manager,
                            thread_id,
                            EventDrivenToolTrigger {
                                tool: "process_exit_subscribe".to_string(),
                                title: "Process exit restore failed".to_string(),
                                text,
                            },
                        )
                        .await;
                    }
                }
            }
        }
        if changed
            && let Err(err) =
                Self::persist_thread_subscriptions(&self.thread_manager, &self.state, thread_id)
                    .await
        {
            warn!("failed to persist restored subscription snapshot for {thread_id}: {err}");
        }
    }
}

fn subscription_id(subscription: &PersistedSubscription) -> &str {
    match subscription {
        PersistedSubscription::Fs {
            subscription_id, ..
        }
        | PersistedSubscription::Schedule {
            subscription_id, ..
        }
        | PersistedSubscription::ProcessExit {
            subscription_id, ..
        } => subscription_id.as_str(),
    }
}
