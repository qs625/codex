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

pub use extension::FsSubscriptionExtension;

/// Installs the event subscription extension into the extension registry.
///
/// The extension exposes file, timer, and process-exit subscription tools to
/// the model. When a subscribed event fires, the runtime automatically injects
/// a new user turn into the owning thread so the model can observe and respond
/// to the change.
pub fn install(
    registry: &mut ExtensionRegistryBuilder<Config>,
    file_watcher: Arc<FileWatcher>,
    thread_manager: Weak<ThreadManager>,
) {
    let extension = Arc::new(FsSubscriptionExtension::new(file_watcher, thread_manager));
    registry.thread_lifecycle_contributor(extension.clone());
    registry.tool_contributor(extension);
}
