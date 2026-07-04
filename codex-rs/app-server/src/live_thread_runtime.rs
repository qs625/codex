use std::sync::Arc;

use futures::future::BoxFuture;
use protocol::ThreadId;
use protocol::error::Result as CodexResult;
use protocol::protocol::AgentStatus;
use protocol::protocol::Event;
use protocol::protocol::Op;
use protocol::protocol::SessionConfiguredEvent;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::W3cTraceContext;
use skill_service_api::SkillWatchPath;
use thread_service_api::AppServerClientInfo;
use thread_service_api::LiveThreadHandle;
use thread_service_api::LiveThreadInfo;
use thread_service_api::LiveThreadRegistry;
use thread_service_api::LiveThreadSnapshot;
use thread_service_api::ThreadConfigSnapshot;
use thread_service_api::ThreadRuntimeStatus;
use thread_store_api::StoredThread;
use thread_store_api::StoredThreadHistory;
use thread_store_api::ThreadStoreResult;

/// Object-safe live thread surface consumed by app-server listener/display code.
///
/// This keeps app-server orchestration depending on capability traits instead
/// of concrete `codex-core` thread types. Runtime owner crates can implement
/// the underlying `LiveThreadHandle`; this facade only boxes the futures needed
/// at the app-server boundary.
pub(crate) trait AppServerLiveThreadHandle: Send + Sync {
    fn session_configured(&self) -> SessionConfiguredEvent;

    fn next_event(&self) -> BoxFuture<'_, CodexResult<Event>>;

    fn submit_op(&self, op: Op) -> BoxFuture<'_, CodexResult<String>>;

    fn agent_status(&self) -> BoxFuture<'_, AgentStatus>;

    fn runtime_thread_status(&self) -> BoxFuture<'_, ThreadRuntimeStatus>;

    fn config_snapshot(&self) -> BoxFuture<'_, ThreadConfigSnapshot>;

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThread>>;

    fn token_usage_info(&self) -> BoxFuture<'_, Option<TokenUsageInfo>>;

    fn thread_context_usage(&self) -> BoxFuture<'_, ThreadContextUsage>;

    fn apply_goal_resume_runtime_effects(&self) -> BoxFuture<'_, CodexResult<()>>;

    fn continue_active_goal_if_idle(&self) -> BoxFuture<'_, CodexResult<()>>;

    fn shutdown_and_wait(&self) -> BoxFuture<'_, CodexResult<()>>;

    fn wait_until_terminated(&self) -> BoxFuture<'_, ()>;
}

impl<T> AppServerLiveThreadHandle for T
where
    T: LiveThreadHandle + ?Sized,
{
    fn session_configured(&self) -> SessionConfiguredEvent {
        LiveThreadHandle::session_configured(self)
    }

    fn next_event(&self) -> BoxFuture<'_, CodexResult<Event>> {
        Box::pin(LiveThreadHandle::next_event(self))
    }

    fn submit_op(&self, op: Op) -> BoxFuture<'_, CodexResult<String>> {
        Box::pin(LiveThreadHandle::submit_thread_op(self, op))
    }

    fn agent_status(&self) -> BoxFuture<'_, AgentStatus> {
        Box::pin(LiveThreadHandle::agent_status(self))
    }

    fn runtime_thread_status(&self) -> BoxFuture<'_, ThreadRuntimeStatus> {
        Box::pin(LiveThreadHandle::runtime_thread_status(self))
    }

    fn config_snapshot(&self) -> BoxFuture<'_, ThreadConfigSnapshot> {
        Box::pin(LiveThreadHandle::config_snapshot(self))
    }

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThread>> {
        Box::pin(LiveThreadHandle::read_thread(
            self,
            include_archived,
            include_history,
        ))
    }

    fn token_usage_info(&self) -> BoxFuture<'_, Option<TokenUsageInfo>> {
        Box::pin(LiveThreadHandle::token_usage_info(self))
    }

    fn thread_context_usage(&self) -> BoxFuture<'_, ThreadContextUsage> {
        Box::pin(LiveThreadHandle::thread_context_usage(self))
    }

    fn apply_goal_resume_runtime_effects(&self) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadHandle::apply_goal_resume_runtime_effects(self))
    }

    fn continue_active_goal_if_idle(&self) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadHandle::continue_active_goal_if_idle(self))
    }

    fn shutdown_and_wait(&self) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadHandle::shutdown_and_wait(self))
    }

    fn wait_until_terminated(&self) -> BoxFuture<'_, ()> {
        Box::pin(LiveThreadHandle::wait_until_terminated(self))
    }
}

/// Object-safe live thread registry surface needed by app-server listeners.
pub(crate) trait AppServerLiveThreadRegistry: Send + Sync {
    fn list_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>>;

    fn is_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool>;

    fn live_thread_handle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Arc<dyn AppServerLiveThreadHandle>>>;

    fn live_thread_info(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<LiveThreadInfo>>;

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadSnapshot>>;

    fn send_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> BoxFuture<'_, CodexResult<String>>;

    fn thread_agent_status(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<AgentStatus>>;

    fn set_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: AppServerClientInfo,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThreadHistory>>;

    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<TokenUsageInfo>>>;

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadContextUsage>>;

    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>>;

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>>;

    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<SkillWatchPath>>>;

    fn remove_loaded_thread(&self, thread_id: ThreadId) -> BoxFuture<'_, bool>;
}

impl<T> AppServerLiveThreadRegistry for T
where
    T: LiveThreadRegistry + Send + Sync,
{
    fn list_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>> {
        Box::pin(LiveThreadRegistry::list_thread_ids(self))
    }

    fn is_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
        Box::pin(LiveThreadRegistry::is_thread_loaded(self, thread_id))
    }

    fn live_thread_handle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Arc<dyn AppServerLiveThreadHandle>>> {
        Box::pin(async move {
            let thread = LiveThreadRegistry::live_thread_handle(self, thread_id).await?;
            let thread: Arc<dyn AppServerLiveThreadHandle> = thread;
            Ok(thread)
        })
    }

    fn live_thread_info(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<LiveThreadInfo>> {
        Box::pin(LiveThreadRegistry::live_thread_info(self, thread_id))
    }

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadSnapshot>> {
        Box::pin(LiveThreadRegistry::live_thread_snapshot(self, thread_id))
    }

    fn send_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> BoxFuture<'_, CodexResult<String>> {
        Box::pin(LiveThreadRegistry::send_op_with_trace(
            self, thread_id, op, trace,
        ))
    }

    fn thread_agent_status(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<AgentStatus>> {
        Box::pin(LiveThreadRegistry::thread_agent_status(self, thread_id))
    }

    fn set_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: AppServerClientInfo,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadRegistry::set_thread_app_server_client_info(
            self, thread_id, info,
        ))
    }

    fn thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThreadHistory>> {
        Box::pin(LiveThreadRegistry::thread_history(
            self,
            thread_id,
            include_archived,
        ))
    }

    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<TokenUsageInfo>>> {
        Box::pin(LiveThreadRegistry::thread_token_usage_info(self, thread_id))
    }

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadContextUsage>> {
        Box::pin(LiveThreadRegistry::thread_context_usage(self, thread_id))
    }

    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>> {
        Box::pin(
            LiveThreadRegistry::increment_thread_out_of_band_elicitation_count(self, thread_id),
        )
    }

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>> {
        Box::pin(
            LiveThreadRegistry::decrement_thread_out_of_band_elicitation_count(self, thread_id),
        )
    }

    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<SkillWatchPath>>> {
        Box::pin(LiveThreadRegistry::thread_skill_watch_paths(
            self, thread_id,
        ))
    }

    fn remove_loaded_thread(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
        Box::pin(LiveThreadRegistry::remove_loaded_thread(self, thread_id))
    }
}
