use crate::error_code::invalid_request;
use crate::outgoing_message::ConnectionId;
use crate::outgoing_message::OutgoingMessageSender;
use app_server_protocol::FsChangedNotification;
use app_server_protocol::FsUnwatchParams;
use app_server_protocol::FsUnwatchResponse;
use app_server_protocol::FsWatchParams;
use app_server_protocol::FsWatchResponse;
use app_server_protocol::JSONRPCErrorError;
use app_server_protocol::ServerNotification;
use codex_file_watcher::FileWatcher;
use codex_file_watcher::FileWatcherEvent;
use codex_file_watcher::FileWatcherSubscriber;
use codex_file_watcher::Receiver;
use codex_file_watcher::WatchPath;
use codex_file_watcher::WatchRegistration;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::warn;

const FS_CHANGED_NOTIFICATION_DEBOUNCE: Duration = Duration::from_millis(200);

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

struct FsWatchRequest {
    key: WatchKey,
    watch_id: String,
    root: AbsolutePathBuf,
}

struct PreparedWatch {
    request: FsWatchRequest,
    rx: Receiver,
    entry: WatchEntry,
    terminate_rx: oneshot::Receiver<oneshot::Sender<()>>,
}

struct WatchTask {
    connection_id: ConnectionId,
    watch_id: String,
    root: AbsolutePathBuf,
    outgoing: Arc<OutgoingMessageSender>,
    rx: DebouncedReceiver,
    terminate_rx: oneshot::Receiver<oneshot::Sender<()>>,
}

struct WatchNotification {
    connection_id: ConnectionId,
    notification: FsChangedNotification,
}

struct WatchTermination {
    done_rx: oneshot::Receiver<()>,
    _subscriber: FileWatcherSubscriber,
    _registration: WatchRegistration,
}

#[derive(Clone)]
pub(crate) struct FsWatchManager {
    outgoing: Arc<OutgoingMessageSender>,
    file_watcher: Arc<FileWatcher>,
    state: Arc<AsyncMutex<FsWatchState>>,
}

#[derive(Default)]
struct FsWatchState {
    entries: HashMap<WatchKey, WatchEntry>,
}

struct WatchEntry {
    terminate_tx: oneshot::Sender<oneshot::Sender<()>>,
    _subscriber: FileWatcherSubscriber,
    _registration: WatchRegistration,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct WatchKey {
    connection_id: ConnectionId,
    watch_id: String,
}

impl FsWatchManager {
    pub(crate) fn new(outgoing: Arc<OutgoingMessageSender>) -> Self {
        let file_watcher = match FileWatcher::new() {
            Ok(file_watcher) => Arc::new(file_watcher),
            Err(err) => {
                warn!("filesystem watch manager falling back to noop core watcher: {err}");
                Arc::new(FileWatcher::noop())
            }
        };
        Self::new_with_file_watcher(outgoing, file_watcher)
    }

    pub(crate) fn new_with_file_watcher(
        outgoing: Arc<OutgoingMessageSender>,
        file_watcher: Arc<FileWatcher>,
    ) -> Self {
        Self {
            outgoing,
            file_watcher,
            state: Arc::new(AsyncMutex::new(FsWatchState::default())),
        }
    }

    pub(crate) fn file_watcher(&self) -> Arc<FileWatcher> {
        Arc::clone(&self.file_watcher)
    }

    pub(crate) async fn watch(
        &self,
        connection_id: ConnectionId,
        params: FsWatchParams,
    ) -> Result<FsWatchResponse, JSONRPCErrorError> {
        let prepared = self.prepare_watch(connection_id, params);
        let response_path = prepared.request.root.clone();
        let terminate_rx = prepared.terminate_rx;
        self.insert_watch_entry(&prepared.request, prepared.entry)
            .await?;
        self.spawn_watch_task(prepared.request, prepared.rx, terminate_rx);

        Ok(FsWatchResponse {
            path: response_path,
        })
    }

    fn prepare_watch(&self, connection_id: ConnectionId, params: FsWatchParams) -> PreparedWatch {
        let request = FsWatchRequest::from_connection_params(connection_id, params);
        let (subscriber, rx) = self.file_watcher.add_subscriber();
        let registration = subscriber.register_paths(vec![request.watch_path()]);
        let (terminate_tx, terminate_rx) = oneshot::channel();
        PreparedWatch {
            request,
            rx,
            entry: WatchEntry {
                terminate_tx,
                _subscriber: subscriber,
                _registration: registration,
            },
            terminate_rx,
        }
    }

    async fn insert_watch_entry(
        &self,
        request: &FsWatchRequest,
        entry: WatchEntry,
    ) -> Result<(), JSONRPCErrorError> {
        match self.state.lock().await.entries.entry(request.key.clone()) {
            Entry::Occupied(_) => {
                return Err(invalid_request(format!(
                    "watchId already exists: {}",
                    request.watch_id
                )));
            }
            Entry::Vacant(vacant) => {
                vacant.insert(entry);
            }
        }
        Ok(())
    }

    fn spawn_watch_task(
        &self,
        request: FsWatchRequest,
        rx: Receiver,
        terminate_rx: oneshot::Receiver<oneshot::Sender<()>>,
    ) {
        let task = WatchTask {
            connection_id: request.key.connection_id,
            watch_id: request.watch_id,
            root: request.root,
            outgoing: Arc::clone(&self.outgoing),
            rx: DebouncedReceiver::new(rx, FS_CHANGED_NOTIFICATION_DEBOUNCE),
            terminate_rx,
        };
        tokio::spawn(async move {
            task.run().await;
        });
    }

    pub(crate) async fn unwatch(
        &self,
        connection_id: ConnectionId,
        params: FsUnwatchParams,
    ) -> Result<FsUnwatchResponse, JSONRPCErrorError> {
        if let Some(termination) = self
            .remove_watch_entry(WatchKey {
                connection_id,
                watch_id: params.watch_id,
            })
            .await
        {
            termination.wait().await;
        }
        Ok(FsUnwatchResponse {})
    }

    pub(crate) async fn connection_closed(&self, connection_id: ConnectionId) {
        let mut state = self.state.lock().await;
        state
            .entries
            .extract_if(|key, _| key.connection_id == connection_id)
            .count();
    }

    async fn remove_watch_entry(&self, watch_key: WatchKey) -> Option<WatchTermination> {
        self.state
            .lock()
            .await
            .entries
            .remove(&watch_key)
            .map(WatchTermination::from_entry)
    }
}

impl FsWatchRequest {
    fn from_connection_params(connection_id: ConnectionId, params: FsWatchParams) -> Self {
        let watch_id = params.watch_id;
        Self {
            key: WatchKey {
                connection_id,
                watch_id: watch_id.clone(),
            },
            watch_id,
            root: params.path,
        }
    }

    fn watch_path(&self) -> WatchPath {
        WatchPath {
            path: self.root.to_path_buf(),
            recursive: false,
        }
    }
}

impl WatchTask {
    async fn run(self) {
        let WatchTask {
            connection_id,
            watch_id,
            root,
            outgoing,
            mut rx,
            terminate_rx,
        } = self;
        tokio::pin!(terminate_rx);
        loop {
            let event = tokio::select! {
                biased;
                _ = &mut terminate_rx => break,
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            if let Some(notification) =
                watch_notification_for_event(connection_id, &watch_id, &root, event)
            {
                notification.send(&outgoing).await;
            }
        }
    }
}

impl WatchNotification {
    async fn send(self, outgoing: &OutgoingMessageSender) {
        outgoing
            .send_server_notification_to_connection_and_wait(
                self.connection_id,
                ServerNotification::FsChanged(self.notification),
            )
            .await;
    }
}

impl WatchTermination {
    fn from_entry(entry: WatchEntry) -> Self {
        let WatchEntry {
            terminate_tx,
            _subscriber,
            _registration,
        } = entry;
        let (done_tx, done_rx) = oneshot::channel();
        let _ = terminate_tx.send(done_tx);
        Self {
            done_rx,
            _subscriber,
            _registration,
        }
    }

    async fn wait(self) {
        // Wait for the oneshot to be destroyed by the task to ensure that no notifications
        // are sent after the unwatch response.
        let _ = self.done_rx.await;
    }
}

fn watch_notification_for_event(
    connection_id: ConnectionId,
    watch_id: &str,
    root: &AbsolutePathBuf,
    event: FileWatcherEvent,
) -> Option<WatchNotification> {
    let changed_paths = changed_paths_for_event(root, event);
    if changed_paths.is_empty() {
        return None;
    }
    Some(WatchNotification {
        connection_id,
        notification: FsChangedNotification {
            watch_id: watch_id.to_string(),
            changed_paths,
        },
    })
}

fn changed_paths_for_event(
    root: &AbsolutePathBuf,
    event: FileWatcherEvent,
) -> Vec<AbsolutePathBuf> {
    let mut changed_paths = event
        .paths
        .into_iter()
        .map(|path| root.join(path))
        .collect::<Vec<_>>();
    changed_paths.sort_by(|left, right| left.as_path().cmp(right.as_path()));
    changed_paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn absolute_path(path: PathBuf) -> AbsolutePathBuf {
        assert!(
            path.is_absolute(),
            "path must be absolute: {}",
            path.display()
        );
        AbsolutePathBuf::try_from(path).expect("path should be absolute")
    }

    fn manager_with_noop_watcher() -> FsWatchManager {
        const OUTGOING_BUFFER: usize = 1;
        let (tx, _rx) = mpsc::channel(OUTGOING_BUFFER);
        FsWatchManager::new_with_file_watcher(
            Arc::new(OutgoingMessageSender::new(
                tx,
                codex_analytics::AnalyticsEventsClient::disabled(),
            )),
            Arc::new(FileWatcher::noop()),
        )
    }

    #[tokio::test]
    async fn watch_uses_client_id_and_tracks_the_owner_scoped_entry() {
        let temp_dir = TempDir::new().expect("temp dir");
        let head_path = temp_dir.path().join("HEAD");
        std::fs::write(&head_path, "ref: refs/heads/main\n").expect("write HEAD");

        let manager = manager_with_noop_watcher();
        let path = absolute_path(head_path);
        let watch_id = "watch-head".to_string();
        let response = manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: watch_id.clone(),
                    path: path.clone(),
                },
            )
            .await
            .expect("watch should succeed");

        assert_eq!(response.path, path);

        let state = manager.state.lock().await;
        assert_eq!(
            state.entries.keys().cloned().collect::<HashSet<_>>(),
            HashSet::from([WatchKey {
                connection_id: ConnectionId(1),
                watch_id,
            }])
        );
    }

    #[tokio::test]
    async fn unwatch_is_scoped_to_the_connection_that_created_the_watch() {
        let temp_dir = TempDir::new().expect("temp dir");
        let head_path = temp_dir.path().join("HEAD");
        std::fs::write(&head_path, "ref: refs/heads/main\n").expect("write HEAD");

        let manager = manager_with_noop_watcher();
        manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: "watch-head".to_string(),
                    path: absolute_path(head_path),
                },
            )
            .await
            .expect("watch should succeed");
        let watch_key = WatchKey {
            connection_id: ConnectionId(1),
            watch_id: "watch-head".to_string(),
        };

        manager
            .unwatch(
                ConnectionId(2),
                FsUnwatchParams {
                    watch_id: "watch-head".to_string(),
                },
            )
            .await
            .expect("foreign unwatch should be a no-op");
        assert!(manager.state.lock().await.entries.contains_key(&watch_key));

        manager
            .unwatch(
                ConnectionId(1),
                FsUnwatchParams {
                    watch_id: "watch-head".to_string(),
                },
            )
            .await
            .expect("owner unwatch should succeed");
        assert!(!manager.state.lock().await.entries.contains_key(&watch_key));
    }

    #[tokio::test]
    async fn watch_rejects_duplicate_id_for_the_same_connection() {
        let temp_dir = TempDir::new().expect("temp dir");
        let head_path = temp_dir.path().join("HEAD");
        let fetch_head_path = temp_dir.path().join("FETCH_HEAD");
        std::fs::write(&head_path, "ref: refs/heads/main\n").expect("write HEAD");
        std::fs::write(&fetch_head_path, "old-fetch\n").expect("write FETCH_HEAD");

        let manager = manager_with_noop_watcher();
        manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: "watch-head".to_string(),
                    path: absolute_path(head_path),
                },
            )
            .await
            .expect("first watch should succeed");

        let error = manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: "watch-head".to_string(),
                    path: absolute_path(fetch_head_path),
                },
            )
            .await
            .expect_err("duplicate watch should fail");

        assert_eq!(error.message, "watchId already exists: watch-head");
        assert_eq!(manager.state.lock().await.entries.len(), 1);
    }

    #[test]
    fn watch_notification_projection_sorts_joined_paths_and_skips_empty_events() {
        let root = absolute_path(PathBuf::from("/tmp/repo/.git"));

        let notification = watch_notification_for_event(
            ConnectionId(7),
            "watch-git",
            &root,
            FileWatcherEvent {
                paths: vec![PathBuf::from("refs/heads/main"), PathBuf::from("HEAD")],
            },
        )
        .expect("changed paths should produce a notification");

        assert_eq!(notification.connection_id, ConnectionId(7));
        assert_eq!(notification.notification.watch_id, "watch-git");
        assert_eq!(
            notification.notification.changed_paths,
            vec![
                absolute_path(PathBuf::from("/tmp/repo/.git/HEAD")),
                absolute_path(PathBuf::from("/tmp/repo/.git/refs/heads/main")),
            ]
        );
        assert!(
            watch_notification_for_event(
                ConnectionId(7),
                "watch-git",
                &root,
                FileWatcherEvent { paths: Vec::new() },
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn connection_closed_removes_only_that_connections_watches() {
        let temp_dir = TempDir::new().expect("temp dir");
        let head_path = temp_dir.path().join("HEAD");
        let fetch_head_path = temp_dir.path().join("FETCH_HEAD");
        let packed_refs_path = temp_dir.path().join("packed-refs");
        std::fs::write(&head_path, "ref: refs/heads/main\n").expect("write HEAD");
        std::fs::write(&fetch_head_path, "old-fetch\n").expect("write FETCH_HEAD");
        std::fs::write(&packed_refs_path, "refs\n").expect("write packed-refs");

        let manager = manager_with_noop_watcher();
        let response = manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: "watch-head".to_string(),
                    path: absolute_path(head_path.clone()),
                },
            )
            .await
            .expect("first watch should succeed");
        manager
            .watch(
                ConnectionId(1),
                FsWatchParams {
                    watch_id: "watch-fetch-head".to_string(),
                    path: absolute_path(fetch_head_path),
                },
            )
            .await
            .expect("second watch should succeed");
        manager
            .watch(
                ConnectionId(2),
                FsWatchParams {
                    watch_id: "watch-packed-refs".to_string(),
                    path: absolute_path(packed_refs_path),
                },
            )
            .await
            .expect("third watch should succeed");

        manager.connection_closed(ConnectionId(1)).await;

        assert_eq!(
            manager
                .state
                .lock()
                .await
                .entries
                .keys()
                .cloned()
                .collect::<HashSet<_>>(),
            HashSet::from([WatchKey {
                connection_id: ConnectionId(2),
                watch_id: "watch-packed-refs".to_string(),
            }])
        );
        assert_eq!(response.path, absolute_path(head_path));
    }
}
