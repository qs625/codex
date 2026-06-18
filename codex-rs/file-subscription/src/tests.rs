use std::sync::Arc;

use crate::extension::FsSubscriptionExtension;
use crate::extension::ThreadSubscriptionState;
use crate::registry::FsSubscriptionRegistry;
use crate::runtime::UnavailableFileSubscriptionThreadRuntime;
use codex_extension_api::ExtensionData;
use codex_extension_api::ToolContributor;
use codex_file_watcher::FileWatcher;

fn make_thread_store(registry: Arc<FsSubscriptionRegistry>) -> ExtensionData {
    let thread_store = ExtensionData::new("thread");
    thread_store.insert(ThreadSubscriptionState {
        thread_id: codex_protocol::ThreadId::new(),
        registry,
    });
    thread_store
}

fn unavailable_runtime() -> Arc<UnavailableFileSubscriptionThreadRuntime> {
    Arc::new(UnavailableFileSubscriptionThreadRuntime)
}

#[test]
fn tools_include_file_and_timer_subscriptions_without_exec_manager() {
    let extension =
        FsSubscriptionExtension::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);
    let registry = Arc::new(FsSubscriptionRegistry::new(
        Arc::new(FileWatcher::noop()),
        unavailable_runtime(),
        None,
    ));
    let thread_store = make_thread_store(registry);

    let tool_names = extension
        .tools(&ExtensionData::new("session"), &thread_store)
        .into_iter()
        .map(|tool| tool.tool_name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec!["schedule_subscribe", "schedule_unsubscribe",]
    );
}

#[test]
fn old_process_exit_tools_are_not_contributed_with_exec_manager_handle() {
    let extension =
        FsSubscriptionExtension::new(Arc::new(FileWatcher::noop()), unavailable_runtime(), None);
    let registry = Arc::new(FsSubscriptionRegistry::new(
        Arc::new(FileWatcher::noop()),
        unavailable_runtime(),
        None,
    ));
    let session_store = ExtensionData::new("session");
    let thread_store = make_thread_store(registry);

    let tool_names = extension
        .tools(&session_store, &thread_store)
        .into_iter()
        .map(|tool| tool.tool_name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        tool_names,
        vec!["schedule_subscribe", "schedule_unsubscribe",]
    );
}
