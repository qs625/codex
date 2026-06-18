use std::future::Future;
use std::pin::Pin;

use codex_protocol::ThreadId;
use codex_protocol::event_command::EventCommandEvent;
use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
use codex_protocol::subscriptions::PersistedSubscription;

pub type SubscriptionRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Host-provided thread operations needed by event subscriptions.
///
/// Implementations own the bridge from subscription runtime events to the
/// concrete thread manager/session machinery used by the embedding host.
pub trait FileSubscriptionThreadRuntime: Send + Sync {
    fn update_active_subscription_count<'a>(
        &'a self,
        thread_id: ThreadId,
        active_count: usize,
    ) -> SubscriptionRuntimeFuture<'a, ()>;

    fn append_event_driven_tool<'a>(
        &'a self,
        thread_id: ThreadId,
        trigger: EventDrivenToolTrigger,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>>;

    fn append_event_command_event<'a>(
        &'a self,
        thread_id: ThreadId,
        event: EventCommandEvent,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>>;

    fn persist_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
        subscriptions: Vec<PersistedSubscription>,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>>;

    fn load_persisted_subscriptions<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> SubscriptionRuntimeFuture<'a, Result<Vec<PersistedSubscription>, String>>;
}

#[cfg(test)]
pub struct UnavailableFileSubscriptionThreadRuntime;

#[cfg(test)]
impl FileSubscriptionThreadRuntime for UnavailableFileSubscriptionThreadRuntime {
    fn update_active_subscription_count<'a>(
        &'a self,
        _thread_id: ThreadId,
        _active_count: usize,
    ) -> SubscriptionRuntimeFuture<'a, ()> {
        Box::pin(async {})
    }

    fn append_event_driven_tool<'a>(
        &'a self,
        _thread_id: ThreadId,
        _trigger: EventDrivenToolTrigger,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async { Err("thread manager unavailable".to_string()) })
    }

    fn append_event_command_event<'a>(
        &'a self,
        _thread_id: ThreadId,
        _event: EventCommandEvent,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async { Err("thread manager unavailable".to_string()) })
    }

    fn persist_subscriptions<'a>(
        &'a self,
        _thread_id: ThreadId,
        _subscriptions: Vec<PersistedSubscription>,
    ) -> SubscriptionRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async { Err("thread manager unavailable".to_string()) })
    }

    fn load_persisted_subscriptions<'a>(
        &'a self,
        _thread_id: ThreadId,
    ) -> SubscriptionRuntimeFuture<'a, Result<Vec<PersistedSubscription>, String>> {
        Box::pin(async { Err("thread manager unavailable".to_string()) })
    }
}
