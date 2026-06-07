use std::sync::Arc;
use std::sync::Weak;

use codex_core::ActiveEventSubscriptionTracker;
use codex_core::NewThread;
use codex_core::StartThreadOptions;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_extension_api::AgentSpawnFuture;
use codex_extension_api::AgentSpawner;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_file_watcher::FileWatcher;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;

use crate::thread_status::ThreadWatchManager;

struct ThreadSubscriptionActivityObserver {
    thread_watch_manager: ThreadWatchManager,
    thread_manager: Weak<ThreadManager>,
    active_event_subscriptions: Arc<ActiveEventSubscriptionTracker>,
}

impl codex_file_subscription::SubscriptionActivityObserver for ThreadSubscriptionActivityObserver {
    fn active_subscription_count_changed(&self, thread_id: ThreadId, active_count: usize) {
        self.active_event_subscriptions
            .set_active_count(thread_id, active_count);
        if active_count == 0
            && let Some(thread_manager) = self.thread_manager.upgrade()
        {
            tokio::spawn(async move {
                thread_manager
                    .maybe_notify_parent_of_final_status(thread_id)
                    .await;
            });
        }
        let thread_watch_manager = self.thread_watch_manager.clone();
        let thread_id = thread_id.to_string();
        tokio::spawn(async move {
            thread_watch_manager
                .note_active_event_subscriptions(&thread_id, active_count)
                .await;
        });
    }
}

pub(crate) fn thread_extensions<S>(
    guardian_agent_spawner: S,
    file_watcher: Arc<FileWatcher>,
    thread_manager: Weak<ThreadManager>,
    thread_watch_manager: ThreadWatchManager,
) -> Arc<ExtensionRegistry<Config>>
where
    S: AgentSpawner<StartThreadOptions, Spawned = NewThread, Error = CodexErr> + 'static,
{
    let active_event_subscriptions = thread_manager
        .upgrade()
        .map(|thread_manager| thread_manager.active_event_subscriptions())
        .unwrap_or_default();
    let mut builder = ExtensionRegistryBuilder::<Config>::new();
    codex_guardian::install(&mut builder, guardian_agent_spawner);
    codex_file_subscription::install(
        &mut builder,
        file_watcher,
        thread_manager.clone(),
        Some(Arc::new(ThreadSubscriptionActivityObserver {
            thread_watch_manager,
            thread_manager,
            active_event_subscriptions,
        })),
    );
    Arc::new(builder.build())
}

pub(crate) fn guardian_agent_spawner(
    thread_manager: Weak<ThreadManager>,
) -> impl AgentSpawner<StartThreadOptions, Spawned = NewThread, Error = CodexErr> {
    move |forked_from_thread_id: ThreadId,
          options: StartThreadOptions|
          -> AgentSpawnFuture<'static, NewThread, CodexErr> {
        let thread_manager = thread_manager.clone();
        Box::pin(async move {
            let thread_manager = thread_manager.upgrade().ok_or_else(|| {
                CodexErr::UnsupportedOperation("thread manager dropped".to_string())
            })?;
            thread_manager
                .spawn_subagent(forked_from_thread_id, options)
                .await
        })
    }
}
