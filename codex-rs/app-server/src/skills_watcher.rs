use std::sync::Arc;
use std::time::Duration;

use crate::outgoing_message::OutgoingMessageSender;
use app_server_protocol::ServerNotification;
use app_server_protocol::SkillsChangedNotification;
use codex_file_watcher::FileWatcher;
use codex_file_watcher::FileWatcherSubscriber;
use codex_file_watcher::Receiver;
use codex_file_watcher::ThrottledWatchReceiver;
use codex_file_watcher::WatchPath;
use codex_file_watcher::WatchRegistration;
use skill_service_api::SharedSkillServiceApi;
use skill_service_api::SkillWatchPath;
use tracing::warn;

#[cfg(not(test))]
const WATCHER_THROTTLE_INTERVAL: Duration = Duration::from_secs(10);
#[cfg(test)]
const WATCHER_THROTTLE_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct SkillsWatcher {
    subscriber: FileWatcherSubscriber,
}

impl SkillsWatcher {
    pub(crate) fn new(
        skill_service: SharedSkillServiceApi,
        outgoing: Arc<OutgoingMessageSender>,
    ) -> Arc<Self> {
        let file_watcher = match FileWatcher::new() {
            Ok(file_watcher) => Arc::new(file_watcher),
            Err(err) => {
                warn!("failed to initialize skills file watcher: {err}");
                Arc::new(FileWatcher::noop())
            }
        };
        let (subscriber, rx) = file_watcher.add_subscriber();
        Self::spawn_event_loop(rx, skill_service, outgoing);
        Arc::new(Self { subscriber })
    }

    pub(crate) fn register_thread_skill_watch_paths(
        &self,
        paths: Vec<SkillWatchPath>,
    ) -> WatchRegistration {
        self.subscriber.register_paths(
            paths
                .into_iter()
                .map(|path| WatchPath {
                    path: path.path,
                    recursive: path.recursive,
                })
                .collect(),
        )
    }

    fn spawn_event_loop(
        rx: Receiver,
        skill_service: SharedSkillServiceApi,
        outgoing: Arc<OutgoingMessageSender>,
    ) {
        let mut rx = ThrottledWatchReceiver::new(rx, WATCHER_THROTTLE_INTERVAL);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            warn!("skills watcher listener skipped: no Tokio runtime available");
            return;
        };
        handle.spawn(async move {
            while rx.recv().await.is_some() {
                skill_service.clear_cache();
                outgoing
                    .send_server_notification(ServerNotification::SkillsChanged(
                        SkillsChangedNotification {},
                    ))
                    .await;
            }
        });
    }
}
