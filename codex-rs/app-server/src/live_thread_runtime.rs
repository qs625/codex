use std::collections::HashMap;
use std::sync::Arc;

use codex_features::Feature;
use futures::future::BoxFuture;
use protocol::ThreadId;
use protocol::error::Result as CodexResult;
use protocol::models::ResponseItem;
use protocol::protocol::AgentStatus;
use protocol::protocol::Event;
use protocol::protocol::Op;
use protocol::protocol::SessionSource;
use protocol::protocol::ThreadContextUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::W3cTraceContext;
use protocol::user_input::UserInput;
use skill_service_api::SkillWatchPath;
use state_api::ExternalGoalSet;
use thread_service::NativeThreadSteerRuntime;
use thread_service::SteerInputError;
use thread_service_api::AppServerClientInfo;
use thread_service_api::CodexThreadTurnContextOverrides;
use thread_service_api::LiveThreadCommandRuntime;
use thread_service_api::LiveThreadConfigRefreshSnapshot;
use thread_service_api::LiveThreadConversationInjectionRuntime;
use thread_service_api::LiveThreadElicitationRuntime;
use thread_service_api::LiveThreadFeedbackRuntime;
use thread_service_api::LiveThreadGoalRuntime;
use thread_service_api::LiveThreadHandle;
use thread_service_api::LiveThreadHistoryRuntime;
use thread_service_api::LiveThreadInfo;
use thread_service_api::LiveThreadInspectionRuntime;
use thread_service_api::LiveThreadListenerHandle;
use thread_service_api::LiveThreadListenerRuntime;
use thread_service_api::LiveThreadSkillWatchRuntime;
use thread_service_api::LiveThreadSnapshot;
use thread_service_api::LiveThreadTurnRuntime;
use thread_service_api::LiveThreadUsageRuntime;
use thread_service_api::ThreadConfigSnapshot;
use thread_store_api::StoredThread;
use thread_store_api::StoredThreadHistory;
use thread_store_api::ThreadStoreResult;

/// Object-safe live thread surface needed by memory consolidation.
pub(crate) trait AppServerMemoryConsolidationThreadHandle: Send + Sync {
    fn submit_op(&self, op: Op) -> BoxFuture<'_, CodexResult<String>>;

    fn agent_status(&self) -> BoxFuture<'_, AgentStatus>;

    fn wait_until_terminated(&self) -> BoxFuture<'_, ()>;

    fn token_usage_info(&self) -> BoxFuture<'_, Option<TokenUsageInfo>>;

    fn shutdown_and_wait(&self) -> BoxFuture<'_, CodexResult<()>>;
}

impl<T> AppServerMemoryConsolidationThreadHandle for T
where
    T: LiveThreadHandle + ?Sized,
{
    fn submit_op(&self, op: Op) -> BoxFuture<'_, CodexResult<String>> {
        Box::pin(LiveThreadHandle::submit_thread_op(self, op))
    }

    fn agent_status(&self) -> BoxFuture<'_, AgentStatus> {
        Box::pin(LiveThreadHandle::agent_status(self))
    }

    fn wait_until_terminated(&self) -> BoxFuture<'_, ()> {
        Box::pin(LiveThreadHandle::wait_until_terminated(self))
    }

    fn token_usage_info(&self) -> BoxFuture<'_, Option<TokenUsageInfo>> {
        Box::pin(LiveThreadHandle::token_usage_info(self))
    }

    fn shutdown_and_wait(&self) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadHandle::shutdown_and_wait(self))
    }
}

/// Object-safe live thread surface consumed by app-server listener/event-stream code.
pub(crate) trait AppServerLiveThreadListenerHandle: Send + Sync {
    fn next_event(&self) -> BoxFuture<'_, CodexResult<Event>>;

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThread>>;
}

impl<T> AppServerLiveThreadListenerHandle for T
where
    T: LiveThreadListenerHandle + ?Sized,
{
    fn next_event(&self) -> BoxFuture<'_, CodexResult<Event>> {
        Box::pin(LiveThreadListenerHandle::next_event(self))
    }

    fn read_thread(
        &self,
        include_archived: bool,
        include_history: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThread>> {
        Box::pin(LiveThreadListenerHandle::read_thread(
            self,
            include_archived,
            include_history,
        ))
    }

}

pub(crate) trait AppServerLiveThreadListenerRuntime: Send + Sync {
    fn live_thread_listener_handle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Arc<dyn AppServerLiveThreadListenerHandle>>>;
}

impl<T> AppServerLiveThreadListenerRuntime for T
where
    T: LiveThreadListenerRuntime + Send + Sync,
{
    fn live_thread_listener_handle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Arc<dyn AppServerLiveThreadListenerHandle>>> {
        Box::pin(async move {
            let thread =
                LiveThreadListenerRuntime::live_thread_listener_handle(self, thread_id).await?;
            let thread: Arc<dyn AppServerLiveThreadListenerHandle> = thread;
            Ok(thread)
        })
    }
}

pub(crate) trait AppServerLiveThreadHistoryRuntime: Send + Sync {
    fn live_thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThreadHistory>>;
}

impl<T> AppServerLiveThreadHistoryRuntime for T
where
    T: LiveThreadHistoryRuntime + Send + Sync,
{
    fn live_thread_history(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThreadHistory>> {
        Box::pin(LiveThreadHistoryRuntime::live_thread_history(
            self,
            thread_id,
            include_archived,
        ))
    }
}

pub(crate) trait AppServerLiveThreadUsageRuntime: Send + Sync {
    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<TokenUsageInfo>>>;

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadContextUsage>>;
}

impl<T> AppServerLiveThreadUsageRuntime for T
where
    T: LiveThreadUsageRuntime + Send + Sync,
{
    fn thread_token_usage_info(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<TokenUsageInfo>>> {
        Box::pin(LiveThreadUsageRuntime::thread_token_usage_info(
            self, thread_id,
        ))
    }

    fn thread_context_usage(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadContextUsage>> {
        Box::pin(LiveThreadUsageRuntime::thread_context_usage(
            self, thread_id,
        ))
    }
}

pub(crate) trait AppServerLiveThreadSkillWatchRuntime: Send + Sync {
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<SkillWatchPath>>>;
}

impl<T> AppServerLiveThreadSkillWatchRuntime for T
where
    T: LiveThreadSkillWatchRuntime + Send + Sync,
{
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<SkillWatchPath>>> {
        Box::pin(LiveThreadSkillWatchRuntime::thread_skill_watch_paths(
            self, thread_id,
        ))
    }
}

pub(crate) trait AppServerLiveThreadInspectionRuntime: Send + Sync {
    fn list_live_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>>;

    fn is_live_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool>;

    fn live_thread_info(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<LiveThreadInfo>>;

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadSnapshot>>;

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadConfigSnapshot>>;

    fn live_thread_config_refresh_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadConfigRefreshSnapshot>>;

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> BoxFuture<'_, CodexResult<bool>>;
}

impl<T> AppServerLiveThreadInspectionRuntime for T
where
    T: LiveThreadInspectionRuntime + Send + Sync,
{
    fn list_live_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>> {
        Box::pin(LiveThreadInspectionRuntime::list_live_thread_ids(self))
    }

    fn is_live_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
        Box::pin(LiveThreadInspectionRuntime::is_live_thread_loaded(
            self, thread_id,
        ))
    }

    fn live_thread_info(&self, thread_id: ThreadId) -> BoxFuture<'_, CodexResult<LiveThreadInfo>> {
        Box::pin(LiveThreadInspectionRuntime::live_thread_info(
            self, thread_id,
        ))
    }

    fn live_thread_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadSnapshot>> {
        Box::pin(LiveThreadInspectionRuntime::live_thread_snapshot(
            self, thread_id,
        ))
    }

    fn live_thread_config_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<ThreadConfigSnapshot>> {
        Box::pin(LiveThreadInspectionRuntime::live_thread_config_snapshot(
            self, thread_id,
        ))
    }

    fn live_thread_config_refresh_snapshot(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<LiveThreadConfigRefreshSnapshot>> {
        Box::pin(LiveThreadInspectionRuntime::live_thread_config_refresh_snapshot(self, thread_id))
    }

    fn live_thread_feature_enabled(
        &self,
        thread_id: ThreadId,
        feature: Feature,
    ) -> BoxFuture<'_, CodexResult<bool>> {
        Box::pin(LiveThreadInspectionRuntime::live_thread_feature_enabled(
            self, thread_id, feature,
        ))
    }
}

pub(crate) trait AppServerLiveThreadFeedbackRuntime: Send + Sync {
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<ThreadId>>>;

    fn thread_guardian_trunk_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<std::path::PathBuf>>>;

    fn session_source(&self) -> SessionSource;
}

impl<T> AppServerLiveThreadFeedbackRuntime for T
where
    T: LiveThreadFeedbackRuntime + Send + Sync,
{
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<ThreadId>>> {
        Box::pin(LiveThreadFeedbackRuntime::list_agent_subtree_thread_ids(
            self, thread_id,
        ))
    }

    fn thread_guardian_trunk_rollout_path(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Option<std::path::PathBuf>>> {
        Box::pin(LiveThreadFeedbackRuntime::thread_guardian_trunk_rollout_path(self, thread_id))
    }

    fn session_source(&self) -> SessionSource {
        LiveThreadFeedbackRuntime::session_source(self)
    }
}

pub(crate) trait AppServerLiveThreadGoalRuntime: Send + Sync {
    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn apply_thread_goal_resume_runtime_effects(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn continue_thread_active_goal_if_idle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>>;
}

impl<T> AppServerLiveThreadGoalRuntime for T
where
    T: LiveThreadGoalRuntime + Send + Sync,
{
    fn prepare_thread_external_goal_mutation(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadGoalRuntime::prepare_thread_external_goal_mutation(self, thread_id))
    }

    fn apply_thread_external_goal_set(
        &self,
        thread_id: ThreadId,
        external_set: ExternalGoalSet,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadGoalRuntime::apply_thread_external_goal_set(
            self,
            thread_id,
            external_set,
        ))
    }

    fn apply_thread_external_goal_clear(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadGoalRuntime::apply_thread_external_goal_clear(
            self, thread_id,
        ))
    }

    fn apply_thread_goal_resume_runtime_effects(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadGoalRuntime::apply_thread_goal_resume_runtime_effects(
            self, thread_id,
        ))
    }

    fn continue_thread_active_goal_if_idle(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(LiveThreadGoalRuntime::continue_thread_active_goal_if_idle(
            self, thread_id,
        ))
    }
}

pub(crate) trait AppServerLiveThreadElicitationRuntime: Send + Sync {
    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>>;

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>>;
}

impl<T> AppServerLiveThreadElicitationRuntime for T
where
    T: LiveThreadElicitationRuntime + Send + Sync,
{
    fn increment_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>> {
        Box::pin(
            LiveThreadElicitationRuntime::increment_thread_out_of_band_elicitation_count(
                self, thread_id,
            ),
        )
    }

    fn decrement_thread_out_of_band_elicitation_count(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<u64>> {
        Box::pin(
            LiveThreadElicitationRuntime::decrement_thread_out_of_band_elicitation_count(
                self, thread_id,
            ),
        )
    }
}

pub(crate) trait AppServerLiveThreadCommandRuntime: Send + Sync {
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> BoxFuture<'_, CodexResult<String>>;

    fn submit_live_thread_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> BoxFuture<'_, CodexResult<String>>;

    fn set_live_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: AppServerClientInfo,
    ) -> BoxFuture<'_, CodexResult<()>>;
}

pub(crate) trait AppServerLiveThreadConversationInjectionRuntime: Send + Sync {
    fn inject_live_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> BoxFuture<'_, CodexResult<()>>;
}

impl<T> AppServerLiveThreadConversationInjectionRuntime for T
where
    T: LiveThreadConversationInjectionRuntime + Send + Sync,
{
    fn inject_live_thread_conversation_items(
        &self,
        thread_id: ThreadId,
        items: Vec<ResponseItem>,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(
            LiveThreadConversationInjectionRuntime::inject_live_thread_conversation_items(
                self, thread_id, items,
            ),
        )
    }
}

pub(crate) trait AppServerLiveThreadSteerRuntime: Send + Sync {
    fn steer_live_thread_input(
        &self,
        thread_id: ThreadId,
        input: Vec<UserInput>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> BoxFuture<'_, CodexResult<Result<String, SteerInputError>>>;
}

impl<T> AppServerLiveThreadSteerRuntime for T
where
    T: NativeThreadSteerRuntime + Send + Sync,
{
    fn steer_live_thread_input(
        &self,
        thread_id: ThreadId,
        input: Vec<UserInput>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> BoxFuture<'_, CodexResult<Result<String, SteerInputError>>> {
        Box::pin(NativeThreadSteerRuntime::steer_live_thread_input(
            self,
            thread_id,
            input,
            expected_turn_id,
            responsesapi_client_metadata,
        ))
    }
}

pub(crate) trait AppServerLiveThreadTurnRuntime: Send + Sync {
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: CodexThreadTurnContextOverrides,
    ) -> BoxFuture<'_, CodexResult<()>>;
}

impl<T> AppServerLiveThreadTurnRuntime for T
where
    T: LiveThreadTurnRuntime + Send + Sync,
{
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: CodexThreadTurnContextOverrides,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(
            LiveThreadTurnRuntime::validate_live_thread_turn_context_overrides(
                self, thread_id, overrides,
            ),
        )
    }
}

impl<T> AppServerLiveThreadCommandRuntime for T
where
    T: LiveThreadCommandRuntime + Send + Sync,
{
    fn submit_live_thread_op(
        &self,
        thread_id: ThreadId,
        op: Op,
    ) -> BoxFuture<'_, CodexResult<String>> {
        Box::pin(LiveThreadCommandRuntime::submit_live_thread_op(
            self, thread_id, op,
        ))
    }

    fn submit_live_thread_op_with_trace(
        &self,
        thread_id: ThreadId,
        op: Op,
        trace: Option<W3cTraceContext>,
    ) -> BoxFuture<'_, CodexResult<String>> {
        Box::pin(LiveThreadCommandRuntime::submit_live_thread_op_with_trace(
            self, thread_id, op, trace,
        ))
    }

    fn set_live_thread_app_server_client_info(
        &self,
        thread_id: ThreadId,
        info: AppServerClientInfo,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(
            LiveThreadCommandRuntime::set_live_thread_app_server_client_info(self, thread_id, info),
        )
    }
}
