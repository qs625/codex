use std::sync::Arc;
use std::sync::Weak;

use codex_core::ThreadManager;
use codex_core::UnifiedExecManagerHandle;
use codex_core::config::Config;
use codex_extension_api::ExtensionData;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::ToolContributor;
use codex_file_watcher::FileWatcher;
use codex_protocol::ThreadId;

use crate::SubscriptionActivityObserver;
use crate::registry::FsSubscriptionRegistry;
use crate::tools;

/// Per-thread state stored in the thread extension store.
pub(crate) struct ThreadSubscriptionState {
    pub(crate) thread_id: ThreadId,
    pub(crate) registry: Arc<FsSubscriptionRegistry>,
}

/// Extension that provides event subscription tools to the model and manages
/// their lifecycle alongside the owning thread.
pub struct FsSubscriptionExtension {
    registry: Arc<FsSubscriptionRegistry>,
}

impl FsSubscriptionExtension {
    pub(crate) fn new(
        file_watcher: Arc<FileWatcher>,
        thread_manager: Weak<ThreadManager>,
        activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
    ) -> Self {
        Self {
            registry: Arc::new(FsSubscriptionRegistry::new(
                file_watcher,
                thread_manager,
                activity_observer,
            )),
        }
    }
}

impl ThreadLifecycleContributor<Config> for FsSubscriptionExtension {
    fn on_thread_start(&self, input: ThreadStartInput<'_, Config>) {
        if let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) {
            input.thread_store.insert(ThreadSubscriptionState {
                thread_id,
                registry: Arc::clone(&self.registry),
            });
        }
    }

    fn on_thread_stop(&self, input: ThreadStopInput<'_>) {
        if let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) {
            let registry = Arc::clone(&self.registry);
            tokio::spawn(async move {
                registry.cancel_all_for_thread(thread_id).await;
            });
        }
    }

    fn on_thread_resume(&self, input: codex_extension_api::ThreadResumeInput<'_>) {
        let Ok(thread_id) = ThreadId::from_string(input.thread_store.level_id()) else {
            return;
        };
        let registry = Arc::clone(&self.registry);
        let unified_exec_manager = input
            .session_store
            .get::<UnifiedExecManagerHandle>()
            .and_then(|handle| handle.upgrade());
        tokio::spawn(async move {
            registry
                .restore_thread_subscriptions(thread_id, unified_exec_manager)
                .await;
        });
    }
}

impl ToolContributor for FsSubscriptionExtension {
    fn tools(
        &self,
        session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn codex_extension_api::ExtensionToolExecutor>> {
        let Some(state) = thread_store.get::<ThreadSubscriptionState>() else {
            return Vec::new();
        };
        let unified_exec_manager = session_store
            .get::<UnifiedExecManagerHandle>()
            .and_then(|handle| handle.upgrade());
        tools::subscription_tools(
            state.thread_id,
            Arc::clone(&state.registry),
            unified_exec_manager,
        )
    }
}
