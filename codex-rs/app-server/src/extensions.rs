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
use codex_protocol::event_command::EventCommandEvent;
use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::subscriptions::PersistedSubscription;
use codex_thread_api::LiveThreadRegistry;
use codex_thread_store::ThreadMetadataPatch;

use crate::thread_status::ThreadWatchManager;

struct ThreadSubscriptionActivityObserver {
    thread_watch_manager: ThreadWatchManager,
}

impl codex_file_subscription::SubscriptionActivityObserver for ThreadSubscriptionActivityObserver {
    fn active_subscription_count_changed(&self, thread_id: ThreadId, active_count: usize) {
        let thread_watch_manager = self.thread_watch_manager.clone();
        let thread_id = thread_id.to_string();
        tokio::spawn(async move {
            thread_watch_manager
                .note_active_event_subscriptions(&thread_id, active_count)
                .await;
        });
    }
}

struct CoreFileSubscriptionThreadRuntime {
    thread_manager: Weak<ThreadManager>,
}

impl CoreFileSubscriptionThreadRuntime {
    fn new(thread_manager: Weak<ThreadManager>) -> Self {
        Self { thread_manager }
    }

    fn upgrade_thread_manager(&self) -> Result<Arc<ThreadManager>, String> {
        self.thread_manager
            .upgrade()
            .ok_or_else(|| "thread manager unavailable".to_string())
    }
}

impl codex_file_subscription::FileSubscriptionThreadRuntime for CoreFileSubscriptionThreadRuntime {
    fn update_active_subscription_count<'a>(
        &'a self,
        thread_id: ThreadId,
        active_count: usize,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, ()> {
        Box::pin(async move {
            let Some(thread_manager) = self.thread_manager.upgrade() else {
                return;
            };
            let active_event_subscriptions: Arc<ActiveEventSubscriptionTracker> =
                thread_manager.active_event_subscriptions();
            let previous_count = active_event_subscriptions.active_count(thread_id);
            active_event_subscriptions.set_active_count(thread_id, active_count);
            if previous_count > 0 && active_count == 0 {
                thread_manager
                    .maybe_notify_parent_of_final_status(thread_id)
                    .await;
            }
        })
    }

    fn append_event_driven_tool<'a>(
        &'a self,
        thread_id: ThreadId,
        trigger: EventDrivenToolTrigger,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let thread_manager = self.upgrade_thread_manager()?;
            let _ = thread_manager
                .append_thread_conversation_item(
                    thread_id,
                    ResponseItem::EventDrivenTool { id: None, trigger },
                )
                .await
                .map_err(|err| err.to_string())?;
            Ok(())
        })
    }

    fn append_event_command_event<'a>(
        &'a self,
        thread_id: ThreadId,
        event: EventCommandEvent,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let thread_manager = self.upgrade_thread_manager()?;
            let _ = thread_manager
                .append_thread_conversation_item(
                    thread_id,
                    ResponseItem::EventCommandEvent { id: None, event },
                )
                .await
                .map_err(|err| err.to_string())?;
            Ok(())
        })
    }

    fn persist_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
        subscriptions: Vec<PersistedSubscription>,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let thread_manager = self.upgrade_thread_manager()?;
            thread_manager
                .update_thread_metadata(
                    thread_id,
                    ThreadMetadataPatch {
                        subscriptions: Some(subscriptions),
                        ..Default::default()
                    },
                    /*include_archived*/ true,
                )
                .await
                .map(|_| ())
                .map_err(|err| err.to_string())
        })
    }

    fn load_persisted_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<
        'a,
        Result<Vec<PersistedSubscription>, String>,
    > {
        Box::pin(async move {
            let thread_manager = self.upgrade_thread_manager()?;
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
            Ok(stored
                .history
                .and_then(|history| {
                    history.items.iter().rev().find_map(|item| match item {
                        RolloutItem::SessionMeta(meta_line) => meta_line.meta.subscriptions.clone(),
                        _ => None,
                    })
                })
                .unwrap_or_default())
        })
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
    let mut builder = ExtensionRegistryBuilder::<Config>::new();
    codex_guardian::install(&mut builder, guardian_agent_spawner);
    codex_file_subscription::install(
        &mut builder,
        file_watcher,
        Arc::new(CoreFileSubscriptionThreadRuntime::new(thread_manager)),
        Some(Arc::new(ThreadSubscriptionActivityObserver {
            thread_watch_manager,
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
