mod extension;
mod registry;
mod schema;
#[cfg(test)]
mod tests;
mod tools;

use std::sync::Arc;
use std::sync::Weak;

use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_file_watcher::FileWatcher;
use codex_protocol::ThreadId;

pub use extension::FsSubscriptionExtension;

pub trait SubscriptionActivityObserver: Send + Sync {
    fn active_subscription_count_changed(&self, thread_id: ThreadId, active_count: usize);
}

/// Installs the event subscription extension into the extension registry.
///
/// The extension exposes file, schedule, and process-exit subscription tools to
/// the model. When a subscribed event fires, the runtime automatically injects
/// a new user turn into the owning thread so the model can observe and respond
/// to the change.
pub fn install(
    registry: &mut ExtensionRegistryBuilder<Config>,
    file_watcher: Arc<FileWatcher>,
    thread_manager: Weak<ThreadManager>,
    activity_observer: Option<Arc<dyn SubscriptionActivityObserver>>,
) {
    let extension = Arc::new(FsSubscriptionExtension::new(
        file_watcher,
        thread_manager,
        activity_observer,
    ));
    registry.thread_lifecycle_contributor(extension.clone());
    registry.tool_contributor(extension);
}
