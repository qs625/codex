use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use codex_file_watcher::FileWatcher;
#[cfg(unix)]
use codex_utils_pty::process_group::kill_process_group;
#[cfg(unix)]
use codex_utils_pty::process_group::terminate_process_group;
use protocol::ThreadId;
use protocol::event_command::EventCommandEvent;
use protocol::event_command::EventCommandEventKind;
use protocol::event_driven_tool::EventDrivenToolTrigger;
use protocol::subscriptions::PersistedSubscription;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::warn;

use crate::SubscriptionActivityObserver;
use crate::event_command_stdin::EventCommandRuntime;
use crate::runtime::FileSubscriptionThreadRuntime;
use crate::tools::schedule::CompiledSchedule;

const MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
const EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(50);
const EVENT_COMMAND_TERM_GRACE_PERIOD: Duration = Duration::from_millis(250);

struct SubscriptionEntry {
    _cancel_tx: oneshot::Sender<()>,
    persisted: PersistedSubscription,
    #[cfg_attr(not(test), allow(dead_code))]
    event_command_runtime: Option<EventCommandRuntime>,
}

struct EventCommandRun {
    thread_runtime: Arc<dyn FileSubscriptionThreadRuntime>,
    thread_id: ThreadId,
    subscription_id: String,
    command: String,
    cwd: Option<String>,
    label: Option<String>,
    cancel_rx: oneshot::Receiver<()>,
    runtime: EventCommandRuntime,
}

enum EventCommandOutputRead {
    Chunk { chunk: String, truncated: bool },
    Eof,
    Failed(String),
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
    _file_watcher: Arc<FileWatcher>,
    thread_runtime: Arc<dyn FileSubscriptionThreadRuntime>,
    state: Arc<AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>>,
    activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
}

impl FsSubscriptionRegistry {
    pub(crate) fn new(
        file_watcher: Arc<FileWatcher>,
        thread_runtime: Arc<dyn FileSubscriptionThreadRuntime>,
        activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
    ) -> Self {
        Self {
            _file_watcher: file_watcher,
            thread_runtime,
            state: Arc::new(AsyncMutex::new(HashMap::new())),
            activity_observer,
        }
    }

    async fn active_subscription_count_for_state(
        state: &AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
        thread_id: ThreadId,
    ) -> usize {
        state
            .lock()
            .await
            .keys()
            .filter(|key| key.thread_id == thread_id)
            .count()
    }

    async fn notify_active_subscription_count(&self, thread_id: ThreadId) {
        Self::notify_active_subscription_count_for(
            self.thread_runtime.as_ref(),
            &self.state,
            self.activity_observer.as_ref(),
            thread_id,
        )
        .await;
    }

    async fn notify_active_subscription_count_for(
        thread_runtime: &dyn FileSubscriptionThreadRuntime,
        state: &AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
        activity_observer: Option<&Arc<dyn SubscriptionActivityObserver>>,
        thread_id: ThreadId,
    ) {
        let active_count = Self::active_subscription_count_for_state(state, thread_id).await;
        thread_runtime
            .update_active_subscription_count(thread_id, active_count)
            .await;
        if let Some(observer) = activity_observer {
            observer.active_subscription_count_changed(thread_id, active_count);
        }
    }

    async fn send_trigger_to_thread(
        thread_runtime: &dyn FileSubscriptionThreadRuntime,
        thread_id: ThreadId,
        trigger: EventDrivenToolTrigger,
    ) -> Result<(), String> {
        thread_runtime
            .append_event_driven_tool(thread_id, trigger)
            .await
    }

    fn subscription_entry(
        cancel_tx: oneshot::Sender<()>,
        persisted: PersistedSubscription,
    ) -> SubscriptionEntry {
        SubscriptionEntry {
            _cancel_tx: cancel_tx,
            persisted,
            event_command_runtime: None,
        }
    }

    fn event_command_subscription_entry(
        cancel_tx: oneshot::Sender<()>,
        persisted: PersistedSubscription,
        event_command_runtime: EventCommandRuntime,
    ) -> SubscriptionEntry {
        SubscriptionEntry {
            _cancel_tx: cancel_tx,
            persisted,
            event_command_runtime: Some(event_command_runtime),
        }
    }

    async fn persist_thread_subscriptions(
        thread_runtime: &dyn FileSubscriptionThreadRuntime,
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
        thread_runtime
            .persist_subscriptions(thread_id, subscriptions)
            .await
    }

    async fn remove_subscription_and_persist(
        thread_runtime: &dyn FileSubscriptionThreadRuntime,
        state: &AsyncMutex<HashMap<SubscriptionKey, SubscriptionEntry>>,
        thread_id: ThreadId,
        subscription_id: &str,
    ) -> Result<bool, String> {
        let key = SubscriptionKey {
            thread_id,
            subscription_id: subscription_id.to_string(),
        };
        let removed_entry = state.lock().await.remove(&key);
        let Some(entry) = removed_entry else {
            return Ok(false);
        };

        if let Err(err) = Self::persist_thread_subscriptions(thread_runtime, state, thread_id).await
        {
            state.lock().await.insert(key, entry);
            return Err(err);
        }

        Ok(true)
    }

    async fn subscription_snapshot_from_history(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<PersistedSubscription>, String> {
        self.thread_runtime
            .load_persisted_subscriptions(thread_id)
            .await
    }

    pub(crate) async fn active_subscriptions(
        &self,
        thread_id: ThreadId,
    ) -> Vec<PersistedSubscription> {
        let state = self.state.lock().await;
        let mut subscriptions = state
            .iter()
            .filter(|(key, _)| key.thread_id == thread_id)
            .map(|(_, entry)| entry.persisted.clone())
            .collect::<Vec<_>>();
        subscriptions.sort_by(|left, right| subscription_id(left).cmp(subscription_id(right)));
        subscriptions
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn subscribe_event_command(
        &self,
        thread_id: ThreadId,
        command: String,
        cwd: Option<String>,
        label: Option<String>,
        subscription_id: String,
    ) {
        self.subscribe_event_command_with_persistence(
            thread_id,
            command,
            cwd,
            label,
            subscription_id,
            /*persist_after*/ true,
        )
        .await;
    }

    async fn subscribe_event_command_with_persistence(
        &self,
        thread_id: ThreadId,
        command: String,
        cwd: Option<String>,
        label: Option<String>,
        subscription_id: String,
        persist_after: bool,
    ) {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_runtime = Arc::clone(&self.thread_runtime);
        let sub_id_for_log = subscription_id.clone();
        let state = Arc::clone(&self.state);
        let activity_observer = self.activity_observer.clone();
        let event_command_runtime = EventCommandRuntime::new();
        let persisted = PersistedSubscription::EventCommand {
            subscription_id: subscription_id.clone(),
            command: command.clone(),
            cwd: cwd.clone(),
            label: label.clone(),
        };

        self.state.lock().await.insert(
            SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.clone(),
            },
            Self::event_command_subscription_entry(
                cancel_tx,
                persisted,
                event_command_runtime.clone(),
            ),
        );
        self.notify_active_subscription_count(thread_id).await;
        if persist_after
            && let Err(err) = Self::persist_thread_subscriptions(
                self.thread_runtime.as_ref(),
                &self.state,
                thread_id,
            )
            .await
        {
            warn!("failed to persist event command subscription {subscription_id}: {err}");
        }

        tokio::spawn(async move {
            if let Err(err) = run_event_command(EventCommandRun {
                thread_runtime: Arc::clone(&thread_runtime),
                thread_id,
                subscription_id: sub_id_for_log.clone(),
                command: command.clone(),
                cwd: cwd.clone(),
                label: label.clone(),
                cancel_rx,
                runtime: event_command_runtime,
            })
            .await
                && err != "thread manager unavailable"
            {
                warn!("event command {sub_id_for_log}: thread {thread_id} unavailable: {err}");
            }
            if let Err(err) = Self::remove_subscription_and_persist(
                thread_runtime.as_ref(),
                &state,
                thread_id,
                &sub_id_for_log,
            )
            .await
            {
                warn!("failed to persist completed event command {sub_id_for_log}: {err}");
            } else {
                Self::notify_active_subscription_count_for(
                    thread_runtime.as_ref(),
                    &state,
                    activity_observer.as_ref(),
                    thread_id,
                )
                .await;
            }
        });
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn write_event_command_stdin(
        &self,
        thread_id: ThreadId,
        subscription_id: &str,
        chars: &str,
    ) -> Result<(), String> {
        if chars.is_empty() {
            return Err("chars must not be empty".to_string());
        }
        let runtime = {
            let state = self.state.lock().await;
            let Some(entry) = state.get(&SubscriptionKey {
                thread_id,
                subscription_id: subscription_id.to_string(),
            }) else {
                return Err(format!(
                    "event command subscription not found: {subscription_id}"
                ));
            };
            match &entry.persisted {
                PersistedSubscription::EventCommand { .. } => {}
                PersistedSubscription::Schedule { .. }
                | PersistedSubscription::Fs { .. }
                | PersistedSubscription::ProcessExit { .. } => {
                    return Err(format!(
                        "subscription is not an event command: {subscription_id}"
                    ));
                }
            }
            entry
                .event_command_runtime
                .clone()
                .ok_or_else(|| format!("event command stdin unavailable: {subscription_id}"))?
        };

        runtime.write_stdin(subscription_id, chars).await
    }

    pub(crate) async fn subscribe_schedule(
        &self,
        thread_id: ThreadId,
        schedule_spec: protocol::subscriptions::ScheduleSpec,
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
        schedule_spec: protocol::subscriptions::ScheduleSpec,
        schedule: CompiledSchedule,
        label: Option<String>,
        subscription_id: String,
        persist_after: bool,
    ) {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let thread_runtime = Arc::clone(&self.thread_runtime);
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
                    Self::send_trigger_to_thread(thread_runtime.as_ref(), thread_id, trigger).await
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
                        thread_runtime.as_ref(),
                        &state,
                        thread_id,
                        &sub_id_for_log,
                    )
                    .await
                    {
                        warn!(
                            "failed to persist completed schedule subscription {sub_id_for_log}: {err}"
                        );
                    } else {
                        Self::notify_active_subscription_count_for(
                            thread_runtime.as_ref(),
                            &state,
                            activity_observer.as_ref(),
                            thread_id,
                        )
                        .await;
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
            Self::subscription_entry(cancel_tx, persisted),
        );
        self.notify_active_subscription_count(thread_id).await;
        if persist_after
            && let Err(err) = Self::persist_thread_subscriptions(
                self.thread_runtime.as_ref(),
                &self.state,
                thread_id,
            )
            .await
        {
            warn!("failed to persist schedule subscription {subscription_id}: {err}");
        }
    }

    /// Cancels a specific subscription. Returns `true` if the subscription existed.
    pub(crate) async fn unsubscribe(&self, thread_id: ThreadId, subscription_id: &str) -> bool {
        match Self::remove_subscription_and_persist(
            self.thread_runtime.as_ref(),
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

    pub(crate) async fn restore_thread_subscriptions(&self, thread_id: ThreadId) {
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
                PersistedSubscription::EventCommand {
                    subscription_id,
                    command,
                    cwd,
                    label,
                } => {
                    self.subscribe_event_command_with_persistence(
                        thread_id,
                        command,
                        cwd,
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
                            self.thread_runtime.as_ref(),
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
                PersistedSubscription::Fs {
                    subscription_id, ..
                }
                | PersistedSubscription::ProcessExit {
                    subscription_id, ..
                } => {
                    changed = true;
                    warn!(
                        "ignoring legacy subscription {subscription_id} for {thread_id} during restore"
                    );
                }
            }
        }
        if changed
            && let Err(err) = Self::persist_thread_subscriptions(
                self.thread_runtime.as_ref(),
                &self.state,
                thread_id,
            )
            .await
        {
            warn!("failed to persist restored subscription snapshot for {thread_id}: {err}");
        }
    }
}

fn subscription_id(subscription: &PersistedSubscription) -> &str {
    match subscription {
        PersistedSubscription::EventCommand {
            subscription_id, ..
        }
        | PersistedSubscription::Schedule {
            subscription_id, ..
        }
        | PersistedSubscription::Fs {
            subscription_id, ..
        }
        | PersistedSubscription::ProcessExit {
            subscription_id, ..
        } => subscription_id.as_str(),
    }
}

async fn run_event_command(run: EventCommandRun) -> Result<(), String> {
    let EventCommandRun {
        thread_runtime,
        thread_id,
        subscription_id,
        command,
        cwd,
        label,
        cancel_rx,
        runtime,
    } = run;
    let mut child = match shell_command(&command, cwd.as_deref())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            send_event_command_event(
                thread_runtime.as_ref(),
                thread_id,
                EventCommandEvent {
                    subscription_id,
                    kind: EventCommandEventKind::FailedToStart,
                    label,
                    command,
                    cwd,
                    line: None,
                    sequence: None,
                    exit_code: None,
                    signal: None,
                    message: Some(err.to_string()),
                    truncated: false,
                    created_at: chrono::Utc::now().timestamp(),
                },
            )
            .await?;
            return Ok(());
        }
    };
    let process_group_id = child.id();
    runtime.set_stdin(child.stdin.take()).await;

    let Some(stdout) = child.stdout.take() else {
        send_event_command_event(
            thread_runtime.as_ref(),
            thread_id,
            EventCommandEvent {
                subscription_id,
                kind: EventCommandEventKind::FailedToStart,
                label,
                command,
                cwd,
                line: None,
                sequence: None,
                exit_code: None,
                signal: None,
                message: Some("event command stdout unavailable".to_string()),
                truncated: false,
                created_at: chrono::Utc::now().timestamp(),
            },
        )
        .await?;
        return Ok(());
    };
    let (mut output_rx, output_task) = spawn_event_command_stdout_task(BufReader::new(stdout));
    tokio::pin!(cancel_rx);
    let mut sequence = 0_u32;
    let mut stdout_closed = false;

    loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                output_task.abort();
                terminate_event_command_process_tree(&mut child, process_group_id).await;
                send_event_command_event(
                    thread_runtime.as_ref(),
                    thread_id,
                    EventCommandEvent {
                        subscription_id,
                        kind: EventCommandEventKind::Cancelled,
                        label,
                        command,
                        cwd,
                        line: None,
                        sequence: None,
                        exit_code: None,
                        signal: None,
                        message: Some("EventCommand cancelled".to_string()),
                        truncated: false,
                        created_at: chrono::Utc::now().timestamp(),
                    },
                ).await?;
                return Ok(());
            }
            status = child.wait() => {
                let status = status.map_err(|err| err.to_string())?;
                let stdout_drain_deadline =
                    tokio::time::Instant::now() + EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT;
                while let Some(drain_timeout) =
                    stdout_drain_deadline.checked_duration_since(tokio::time::Instant::now())
                {
                    if drain_timeout.is_zero() {
                        break;
                    }
                    let Some(output) =
                        recv_event_command_output_after_exit(&mut output_rx, drain_timeout).await
                    else {
                        break;
                    };
                    match output {
                        EventCommandOutputRead::Chunk { chunk, truncated } => {
                            sequence = sequence.saturating_add(1);
                            send_event_command_output_event(
                                thread_runtime.as_ref(),
                                thread_id,
                                &subscription_id,
                                &label,
                                &command,
                                &cwd,
                                sequence,
                                chunk,
                                truncated,
                            ).await?;
                        }
                        EventCommandOutputRead::Eof => break,
                        EventCommandOutputRead::Failed(err) => {
                            output_task.abort();
                            return Err(err);
                        }
                    }
                }
                output_task.abort();
                send_event_command_event(
                    thread_runtime.as_ref(),
                    thread_id,
                    EventCommandEvent {
                        subscription_id,
                        kind: EventCommandEventKind::Exited,
                        label,
                        command,
                        cwd,
                        line: None,
                        sequence: None,
                        exit_code: status.code(),
                        signal: None,
                        message: Some(format!("EventCommand exited with status {status}")),
                        truncated: false,
                        created_at: chrono::Utc::now().timestamp(),
                    },
                ).await?;
                return Ok(());
            }
            output = output_rx.recv(), if !stdout_closed => {
                match output {
                    Some(EventCommandOutputRead::Chunk { chunk, truncated }) => {
                        sequence = sequence.saturating_add(1);
                        send_event_command_output_event(
                            thread_runtime.as_ref(),
                            thread_id,
                            &subscription_id,
                            &label,
                            &command,
                            &cwd,
                            sequence,
                            chunk,
                            truncated,
                        ).await?;
                    }
                    Some(EventCommandOutputRead::Eof) | None => {
                        stdout_closed = true;
                    }
                    Some(EventCommandOutputRead::Failed(err)) => {
                        output_task.abort();
                        return Err(err);
                    }
                }
            }
        }
    }
}

async fn recv_event_command_output_after_exit(
    output_rx: &mut mpsc::UnboundedReceiver<EventCommandOutputRead>,
    timeout: Duration,
) -> Option<EventCommandOutputRead> {
    tokio::time::timeout(timeout, output_rx.recv())
        .await
        .ok()
        .flatten()
}

fn spawn_event_command_stdout_task<R>(
    mut stdout_reader: BufReader<R>,
) -> (
    mpsc::UnboundedReceiver<EventCommandOutputRead>,
    tokio::task::JoinHandle<()>,
)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (output_tx, output_rx) = mpsc::unbounded_channel();
    let output_task = tokio::spawn(async move {
        loop {
            let event = match read_event_command_chunk(&mut stdout_reader).await {
                Ok(Some((chunk, truncated))) => EventCommandOutputRead::Chunk { chunk, truncated },
                Ok(None) => EventCommandOutputRead::Eof,
                Err(err) => EventCommandOutputRead::Failed(err),
            };
            let is_terminal = !matches!(event, EventCommandOutputRead::Chunk { .. });
            if output_tx.send(event).is_err() || is_terminal {
                return;
            }
        }
    });
    (output_rx, output_task)
}

async fn read_event_command_chunk<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Option<(String, bool)>, String> {
    let mut bytes = vec![0_u8; MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES];
    let read_len = reader
        .read(&mut bytes)
        .await
        .map_err(|err| err.to_string())?;
    if read_len == 0 {
        return Ok(None);
    }
    bytes.truncate(read_len);
    Ok(Some(truncate_event_command_chunk(
        String::from_utf8_lossy(&bytes).to_string(),
    )))
}

fn truncate_event_command_chunk(chunk: String) -> (String, bool) {
    if chunk.len() <= MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES {
        return (chunk, false);
    }

    let mut end = MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES;
    while !chunk.is_char_boundary(end) {
        end -= 1;
    }
    (chunk[..end].to_string(), true)
}

fn shell_command(command: &str, cwd: Option<&str>) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell.arg("-c").arg(command);
    if let Some(cwd) = cwd {
        shell.current_dir(Path::new(cwd));
    }
    #[cfg(unix)]
    shell.process_group(0);
    shell
}

async fn terminate_event_command_process_tree(
    child: &mut tokio::process::Child,
    process_group_id: Option<u32>,
) {
    #[cfg(unix)]
    {
        let Some(process_group_id) = process_group_id else {
            let _ = child.kill().await;
            return;
        };
        let should_escalate = match terminate_process_group(process_group_id) {
            Ok(exists) => exists,
            Err(err) => {
                warn!("failed to terminate EventCommand process group {process_group_id}: {err}");
                false
            }
        };
        if should_escalate {
            tokio::time::sleep(EVENT_COMMAND_TERM_GRACE_PERIOD).await;
            if let Err(err) = kill_process_group(process_group_id) {
                warn!("failed to kill EventCommand process group {process_group_id}: {err}");
            }
        }
        let _ = child.wait().await;
    }
    #[cfg(not(unix))]
    {
        let _ = process_group_id;
        let _ = child.kill().await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_event_command_output_event(
    thread_runtime: &dyn FileSubscriptionThreadRuntime,
    thread_id: ThreadId,
    subscription_id: &str,
    label: &Option<String>,
    command: &str,
    cwd: &Option<String>,
    sequence: u32,
    chunk: String,
    truncated: bool,
) -> Result<(), String> {
    send_event_command_event(
        thread_runtime,
        thread_id,
        EventCommandEvent {
            subscription_id: subscription_id.to_string(),
            kind: EventCommandEventKind::Output,
            label: label.clone(),
            command: command.to_string(),
            cwd: cwd.clone(),
            line: Some(chunk),
            sequence: Some(sequence),
            exit_code: None,
            signal: None,
            message: None,
            truncated,
            created_at: chrono::Utc::now().timestamp(),
        },
    )
    .await
}

async fn send_event_command_event(
    thread_runtime: &dyn FileSubscriptionThreadRuntime,
    thread_id: ThreadId,
    event: EventCommandEvent,
) -> Result<(), String> {
    thread_runtime
        .append_event_command_event(thread_id, event)
        .await
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use codex_file_watcher::FileWatcher;
    use pretty_assertions::assert_eq;
    use protocol::ThreadId;
    use protocol::subscriptions::ScheduleSpec;
    use tokio::io::BufReader;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    use super::EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT;
    use super::EventCommandOutputRead;
    use super::FsSubscriptionRegistry;
    use super::MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES;
    use super::read_event_command_chunk;
    use super::recv_event_command_output_after_exit;
    #[cfg(unix)]
    use super::shell_command;
    #[cfg(unix)]
    use super::spawn_event_command_stdout_task;
    #[cfg(unix)]
    use super::terminate_event_command_process_tree;
    use super::truncate_event_command_chunk;
    use crate::runtime::UnavailableFileSubscriptionThreadRuntime;
    use crate::tools::schedule::CompiledSchedule;

    fn unavailable_runtime() -> Arc<UnavailableFileSubscriptionThreadRuntime> {
        Arc::new(UnavailableFileSubscriptionThreadRuntime)
    }

    #[tokio::test]
    async fn failed_subscription_removal_keeps_event_command_active() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("stdin-after-failed-remove.out");
        let registry =
            FsSubscriptionRegistry::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);
        let thread_id = ThreadId::new();
        let subscription_id = "sub-remove-fails".to_string();
        registry
            .subscribe_event_command(
                thread_id,
                "IFS= read -r line; printf '%s' \"$line\" > stdin-after-failed-remove.out"
                    .to_string(),
                Some(temp_dir.path().to_string_lossy().to_string()),
                None,
                subscription_id.clone(),
            )
            .await;

        let unsubscribed = registry.unsubscribe(thread_id, &subscription_id).await;

        assert!(!unsubscribed);
        registry
            .write_event_command_stdin(thread_id, &subscription_id, "still-active\n")
            .await
            .unwrap();
        let output = read_file_eventually(&output_path).await;
        assert_eq!(output, "still-active");
        registry.cancel_all_for_thread(thread_id).await;
    }

    #[tokio::test]
    async fn writes_event_command_stdin_by_subscription_id() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("stdin.out");
        let registry =
            FsSubscriptionRegistry::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);
        let thread_id = ThreadId::new();
        let subscription_id = "sub-stdin".to_string();
        registry
            .subscribe_event_command(
                thread_id,
                "IFS= read -r line; printf '%s' \"$line\" > stdin.out".to_string(),
                Some(temp_dir.path().to_string_lossy().to_string()),
                None,
                subscription_id.clone(),
            )
            .await;

        registry
            .write_event_command_stdin(thread_id, &subscription_id, "hello event command\n")
            .await
            .unwrap();

        let output = read_file_eventually(&output_path).await;
        assert_eq!(output, "hello event command");
    }

    #[tokio::test]
    async fn rejects_empty_event_command_stdin() {
        let registry =
            FsSubscriptionRegistry::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);

        let err = registry
            .write_event_command_stdin(ThreadId::new(), "sub-stdin", "")
            .await
            .expect_err("expected empty stdin to be rejected");

        assert_eq!(err, "chars must not be empty");
    }

    #[tokio::test]
    async fn rejects_unknown_event_command_stdin_subscription() {
        let registry =
            FsSubscriptionRegistry::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);

        let err = registry
            .write_event_command_stdin(ThreadId::new(), "missing-sub", "input\n")
            .await
            .expect_err("expected missing subscription to be rejected");

        assert_eq!(err, "event command subscription not found: missing-sub");
    }

    #[tokio::test]
    async fn rejects_non_event_command_stdin_subscription() {
        let registry =
            FsSubscriptionRegistry::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);
        let thread_id = ThreadId::new();
        let subscription_id = "schedule-sub".to_string();
        let schedule_spec = ScheduleSpec::EveryInterval {
            interval_ms: 60_000,
        };
        let schedule = CompiledSchedule::compile(schedule_spec.clone()).unwrap();
        registry
            .subscribe_schedule(
                thread_id,
                schedule_spec,
                schedule,
                None,
                subscription_id.clone(),
            )
            .await;

        let err = registry
            .write_event_command_stdin(thread_id, &subscription_id, "input\n")
            .await
            .expect_err("expected non-event-command subscription to be rejected");

        assert_eq!(err, "subscription is not an event command: schedule-sub");
    }

    #[test]
    fn truncates_event_command_output_chunks_on_char_boundaries() {
        let chunk = format!(
            "{}中",
            "a".repeat(MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES - "中".len() + 1)
        );

        let (truncated, was_truncated) = truncate_event_command_chunk(chunk);

        assert!(was_truncated);
        assert!(truncated.len() <= MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES);
        assert_eq!(
            truncated,
            "a".repeat(MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES - "中".len() + 1)
        );
    }

    #[tokio::test]
    async fn reads_event_command_chunks_without_buffering_unbounded_output() {
        let input = "a".repeat(MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES * 2);
        let mut reader = BufReader::new(input.as_bytes());

        let chunk = read_event_command_chunk(&mut reader).await.unwrap();
        let next_chunk = read_event_command_chunk(&mut reader).await.unwrap();
        let end = read_event_command_chunk(&mut reader).await.unwrap();

        assert_eq!(
            chunk,
            Some(("a".repeat(MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES), false))
        );
        assert_eq!(
            next_chunk,
            Some(("a".repeat(MAX_EVENT_COMMAND_OUTPUT_CHUNK_BYTES), false))
        );
        assert_eq!(end, None);
    }

    #[tokio::test]
    async fn reads_event_command_chunk_with_multiple_lines() {
        let mut reader = BufReader::new("first\nsecond\n".as_bytes());

        let chunk = read_event_command_chunk(&mut reader).await.unwrap();
        let end = read_event_command_chunk(&mut reader).await.unwrap();

        assert_eq!(chunk, Some(("first\nsecond\n".to_string(), false)));
        assert_eq!(end, None);
    }

    #[tokio::test]
    async fn reads_event_command_chunk_with_blank_lines_and_crlf_unmodified() {
        let mut reader = BufReader::new("\r\nfirst\r\n\r\nsecond\r\n".as_bytes());

        let chunk = read_event_command_chunk(&mut reader).await.unwrap();
        let end = read_event_command_chunk(&mut reader).await.unwrap();

        assert_eq!(
            chunk,
            Some(("\r\nfirst\r\n\r\nsecond\r\n".to_string(), false))
        );
        assert_eq!(end, None);
    }

    #[tokio::test]
    async fn receives_event_command_output_that_arrives_after_process_exit() {
        let (output_tx, mut output_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            output_tx
                .send(EventCommandOutputRead::Chunk {
                    chunk: "tail".to_string(),
                    truncated: false,
                })
                .unwrap();
        });

        let output = timeout(
            EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT * 2,
            recv_event_command_output_after_exit(
                &mut output_rx,
                EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT,
            ),
        )
        .await
        .expect("expected delayed output before timeout");

        assert!(matches!(
            output,
            Some(EventCommandOutputRead::Chunk { ref chunk, truncated: false }) if chunk == "tail"
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdout_reader_preserves_fast_command_tail_without_newline_after_exit() {
        let mut child = shell_command("printf 'tail'", None)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().expect("stdout should be piped");
        let (mut output_rx, output_task) = spawn_event_command_stdout_task(BufReader::new(stdout));

        let status = timeout(Duration::from_secs(1), child.wait())
            .await
            .expect("expected main process exit before timeout")
            .expect("child wait should succeed");
        assert!(status.success());

        let output = timeout(
            EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT * 2,
            recv_event_command_output_after_exit(
                &mut output_rx,
                EVENT_COMMAND_EXIT_STDOUT_DRAIN_TIMEOUT,
            ),
        )
        .await
        .expect("expected stdout drain before timeout");
        assert!(matches!(
            output,
            Some(EventCommandOutputRead::Chunk { ref chunk, truncated: false }) if chunk == "tail"
        ));

        output_task.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn waits_for_process_exit_without_waiting_for_stdout_eof() {
        let mut child = shell_command("printf 'ready\\n'; sleep 60 &", None)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let process_group_id = child.id();
        let stdout = child.stdout.take().expect("stdout should be piped");
        let (mut output_rx, output_task) = spawn_event_command_stdout_task(BufReader::new(stdout));

        let first_output = timeout(Duration::from_secs(1), output_rx.recv())
            .await
            .expect("expected stdout chunk before timeout")
            .expect("expected stdout event");
        assert!(matches!(
            first_output,
            EventCommandOutputRead::Chunk { ref chunk, truncated: false } if chunk == "ready\n"
        ));

        let status = timeout(Duration::from_secs(1), child.wait())
            .await
            .expect("expected main process exit before timeout")
            .expect("child wait should succeed");
        assert!(status.success());

        let pending_output = timeout(Duration::from_millis(100), output_rx.recv()).await;
        assert!(
            pending_output.is_err(),
            "stdout reader should stay blocked because a background child still holds the pipe"
        );

        output_task.abort();
        terminate_event_command_process_tree(&mut child, process_group_id).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminating_event_command_kills_background_children() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pid_path = temp_dir.path().join("child.pid");
        let command = format!("sleep 60 & echo $! > {}; wait", pid_path.display());
        let mut child = shell_command(&command, Some(temp_dir.path().to_str().unwrap()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let process_group_id = child.id();

        let pid = read_pid_file(&pid_path).await;
        assert!(process_exists(pid));

        terminate_event_command_process_tree(&mut child, process_group_id).await;

        for _ in 0..10 {
            if !process_exists(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("background EventCommand child process {pid} was still alive after cancellation");
    }

    #[cfg(unix)]
    async fn read_pid_file(path: &Path) -> i32 {
        for _ in 0..20 {
            if let Ok(pid) = std::fs::read_to_string(path) {
                return pid.trim().parse::<i32>().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("child pid file was not written");
    }

    async fn read_file_eventually(path: &Path) -> String {
        for _ in 0..20 {
            if let Ok(output) = std::fs::read_to_string(path) {
                return output;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("output file was not written");
    }

    #[cfg(unix)]
    fn process_exists(pid: i32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .is_ok_and(|status| status.success())
    }
}
