use std::sync::Arc;
use std::sync::Weak;

use codex_core::ThreadManager;
use codex_core::UnifiedExecManagerHandle;
use codex_core::UnifiedExecProcessManager;
use codex_extension_api::ExtensionData;
use codex_extension_api::ToolContributor;
use codex_file_watcher::FileWatcher;
use pretty_assertions::assert_eq;

use crate::extension::FsSubscriptionExtension;
use crate::extension::ThreadSubscriptionState;
use crate::registry::FsSubscriptionRegistry;

fn make_thread_store(registry: Arc<FsSubscriptionRegistry>) -> ExtensionData {
    let thread_store = ExtensionData::new("thread");
    thread_store.insert(ThreadSubscriptionState {
        thread_id: codex_protocol::ThreadId::new(),
        registry,
    });
    thread_store
}

#[test]
fn tools_include_file_and_timer_subscriptions_without_exec_manager() {
    let extension =
        FsSubscriptionExtension::new(Arc::new(FileWatcher::noop()), Weak::<ThreadManager>::new());
    let registry = Arc::new(FsSubscriptionRegistry::new(
        Arc::new(FileWatcher::noop()),
        Weak::<ThreadManager>::new(),
    ));
    let thread_store = make_thread_store(registry);

    let tool_names = extension
        .tools(&ExtensionData::new("session"), &thread_store)
        .into_iter()
        .map(|tool| tool.tool_name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec![
            "fs_subscribe",
            "fs_unsubscribe",
            "timer_subscribe",
            "timer_unsubscribe",
        ]
    );
}

#[test]
fn process_exit_tools_are_contributed_with_exec_manager_handle() {
    let extension =
        FsSubscriptionExtension::new(Arc::new(FileWatcher::noop()), Weak::<ThreadManager>::new());
    let registry = Arc::new(FsSubscriptionRegistry::new(
        Arc::new(FileWatcher::noop()),
        Weak::<ThreadManager>::new(),
    ));
    let session_store = ExtensionData::new("session");
    let unified_exec_manager = Arc::new(UnifiedExecProcessManager::default());
    session_store.insert(UnifiedExecManagerHandle::new(Arc::downgrade(
        &unified_exec_manager,
    )));
    let thread_store = make_thread_store(registry);

    let tool_names = extension
        .tools(&session_store, &thread_store)
        .into_iter()
        .map(|tool| tool.tool_name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec![
            "fs_subscribe",
            "fs_unsubscribe",
            "timer_subscribe",
            "timer_unsubscribe",
            "process_exit_subscribe",
            "process_exit_unsubscribe",
        ]
    );
}
