use std::sync::Arc;
use std::sync::Weak;

use codex_thread_runtime::NewThread;
use codex_thread_runtime::StartThreadOptions;
use codex_thread_runtime::ThreadService;
use codex_thread_runtime::config::Config;
use codex_extension_api::AgentSpawnFuture;
use codex_extension_api::AgentSpawner;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_file_watcher::FileWatcher;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result as CodexResult;
use codex_protocol::event_command::EventCommandEvent;
use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use codex_protocol::subscriptions::PersistedSubscription;
use codex_thread_api::ActiveEventSubscriptionTracker;
use codex_thread_api::LiveThreadRegistry;
use codex_thread_store_api::ReadThreadParams;
use codex_thread_store_api::ThreadMetadataPatch;
use futures::future::BoxFuture;

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

/// Thread capabilities needed by file subscription extensions.
///
/// Implementations own the concrete thread lookup, metadata persistence, and
/// parent final-status notification behavior. The extension runtime should
/// depend on this narrow host instead of the full thread manager.
pub(crate) trait FileSubscriptionThreadHost: Send + Sync {
    fn update_active_subscription_count<'a>(
        &'a self,
        thread_id: ThreadId,
        active_count: usize,
    ) -> BoxFuture<'a, ()>;

    fn append_subscription_item<'a>(
        &'a self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> BoxFuture<'a, Result<(), String>>;

    fn persist_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
        subscriptions: Vec<PersistedSubscription>,
    ) -> BoxFuture<'a, Result<(), String>>;

    fn load_persisted_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> BoxFuture<'a, Result<Vec<PersistedSubscription>, String>>;
}

/// Thread capability used by the guardian extension to spawn review agents.
///
/// Implementations are expected to fork from the source thread, apply the
/// provided start options, and return the newly created runtime thread result.
pub(crate) trait GuardianAgentSpawnHost: Send + Sync {
    fn spawn_guardian_agent<'a>(
        &'a self,
        forked_from_thread_id: ThreadId,
        options: StartThreadOptions,
    ) -> BoxFuture<'a, CodexResult<NewThread>>;
}

impl GuardianAgentSpawnHost for ThreadService {
    fn spawn_guardian_agent<'a>(
        &'a self,
        forked_from_thread_id: ThreadId,
        options: StartThreadOptions,
    ) -> BoxFuture<'a, CodexResult<NewThread>> {
        Box::pin(ThreadService::spawn_subagent(
            self,
            forked_from_thread_id,
            options,
        ))
    }
}

impl FileSubscriptionThreadHost for ThreadService {
    fn update_active_subscription_count<'a>(
        &'a self,
        thread_id: ThreadId,
        active_count: usize,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let active_event_subscriptions: Arc<ActiveEventSubscriptionTracker> =
                self.active_event_subscriptions();
            let previous_count = active_event_subscriptions.active_count(thread_id);
            active_event_subscriptions.set_active_count(thread_id, active_count);
            if previous_count > 0 && active_count == 0 {
                self.maybe_notify_parent_of_final_status(thread_id).await;
            }
        })
    }

    fn append_subscription_item<'a>(
        &'a self,
        thread_id: ThreadId,
        item: ResponseItem,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let _ = self
                .append_thread_conversation_item(thread_id, item)
                .await
                .map_err(|err| err.to_string())?;
            Ok(())
        })
    }

    fn persist_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
        subscriptions: Vec<PersistedSubscription>,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.update_thread_metadata(
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
    ) -> BoxFuture<'a, Result<Vec<PersistedSubscription>, String>> {
        Box::pin(async move {
            let stored = self
                .read_thread(ReadThreadParams {
                    thread_id,
                    include_archived: true,
                    include_history: true,
                })
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

struct CoreFileSubscriptionThreadRuntime {
    host: Weak<dyn FileSubscriptionThreadHost>,
}

impl CoreFileSubscriptionThreadRuntime {
    fn new(host: Weak<dyn FileSubscriptionThreadHost>) -> Self {
        Self { host }
    }

    fn upgrade_host(&self) -> Result<Arc<dyn FileSubscriptionThreadHost>, String> {
        self.host
            .upgrade()
            .ok_or_else(|| "file subscription thread host unavailable".to_string())
    }
}

impl codex_file_subscription::FileSubscriptionThreadRuntime for CoreFileSubscriptionThreadRuntime {
    fn update_active_subscription_count<'a>(
        &'a self,
        thread_id: ThreadId,
        active_count: usize,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, ()> {
        Box::pin(async move {
            let Some(host) = self.host.upgrade() else {
                return;
            };
            host.update_active_subscription_count(thread_id, active_count)
                .await;
        })
    }

    fn append_event_driven_tool<'a>(
        &'a self,
        thread_id: ThreadId,
        trigger: EventDrivenToolTrigger,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.upgrade_host()?
                .append_subscription_item(
                    thread_id,
                    ResponseItem::EventDrivenTool { id: None, trigger },
                )
                .await
        })
    }

    fn append_event_command_event<'a>(
        &'a self,
        thread_id: ThreadId,
        event: EventCommandEvent,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.upgrade_host()?
                .append_subscription_item(
                    thread_id,
                    ResponseItem::EventCommandEvent { id: None, event },
                )
                .await
        })
    }

    fn persist_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
        subscriptions: Vec<PersistedSubscription>,
    ) -> codex_file_subscription::SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.upgrade_host()?
                .persist_subscriptions(thread_id, subscriptions)
                .await
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
            self.upgrade_host()?
                .load_persisted_subscriptions(thread_id)
                .await
        })
    }
}

pub(crate) fn thread_extensions<S>(
    guardian_agent_spawner: S,
    file_watcher: Arc<FileWatcher>,
    file_subscription_host: Weak<dyn FileSubscriptionThreadHost>,
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
        Arc::new(CoreFileSubscriptionThreadRuntime::new(
            file_subscription_host,
        )),
        Some(Arc::new(ThreadSubscriptionActivityObserver {
            thread_watch_manager,
        })),
    );
    Arc::new(builder.build())
}

pub(crate) fn guardian_agent_spawner(
    host: Weak<dyn GuardianAgentSpawnHost>,
) -> impl AgentSpawner<StartThreadOptions, Spawned = NewThread, Error = CodexErr> {
    move |forked_from_thread_id: ThreadId,
          options: StartThreadOptions|
          -> AgentSpawnFuture<'static, NewThread, CodexErr> {
        let host = host.clone();
        Box::pin(async move {
            let host = host.upgrade().ok_or_else(|| {
                CodexErr::UnsupportedOperation("guardian agent spawn host dropped".to_string())
            })?;
            host.spawn_guardian_agent(forked_from_thread_id, options)
                .await
        })
    }
}
