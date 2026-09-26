use std::collections::HashMap;
use std::sync::Arc;

use codex_features::Feature;
use codex_utils_absolute_path::AbsolutePathBuf;
use config_service::Config;
use futures::future::BoxFuture;
use protocol::ThreadId;
use protocol::error::Result as CodexResult;
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
use thread_service_api::LiveThreadClientRecoveryRuntime;
use thread_service_api::LiveThreadCommandRuntime;
use thread_service_api::LiveThreadConfigRefreshSnapshot;
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
use thread_service_api::LiveThreadTerminalRuntime;
use thread_service_api::LiveThreadTurnRuntime;
use thread_service_api::LiveThreadUsageRuntime;
use thread_service_api::ThreadAgentDirectoryRuntime;
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
}

impl<T> AppServerLiveThreadListenerHandle for T
where
    T: LiveThreadListenerHandle + ?Sized,
{
    fn next_event(&self) -> BoxFuture<'_, CodexResult<Event>> {
        Box::pin(LiveThreadListenerHandle::next_event(self))
    }
}

macro_rules! delegate_app_server_live_thread_runtime {
    (
        impl $target_trait:ident for $source_trait:path {
            $(
                fn $target_method:ident => $source_method:ident(
                    $($arg:ident : $arg_ty:ty),* $(,)?
                ) -> $ret:ty;
            )*
        }
    ) => {
        impl<T> $target_trait for T
        where
            T: $source_trait + Send + Sync,
        {
            $(
                fn $target_method(
                    &self,
                    $($arg: $arg_ty),*
                ) -> BoxFuture<'_, $ret> {
                    Box::pin(<T as $source_trait>::$source_method(self $(, $arg)*))
                }
            )*
        }
    };
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

    fn read_live_thread(
        &self,
        thread_id: ThreadId,
        include_archived: bool,
        include_history: bool,
    ) -> BoxFuture<'_, ThreadStoreResult<StoredThread>>;
}

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadHistoryRuntime for LiveThreadHistoryRuntime {
        fn live_thread_history => live_thread_history(
            thread_id: ThreadId,
            include_archived: bool,
        ) -> ThreadStoreResult<StoredThreadHistory>;

        fn read_live_thread => read_live_thread(
            thread_id: ThreadId,
            include_archived: bool,
            include_history: bool,
        ) -> ThreadStoreResult<StoredThread>;
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

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadUsageRuntime for LiveThreadUsageRuntime {
        fn thread_token_usage_info => thread_token_usage_info(
            thread_id: ThreadId,
        ) -> CodexResult<Option<TokenUsageInfo>>;

        fn thread_context_usage => thread_context_usage(
            thread_id: ThreadId,
        ) -> CodexResult<ThreadContextUsage>;
    }
}

pub(crate) trait AppServerLiveThreadSkillWatchRuntime: Send + Sync {
    fn thread_skill_watch_paths(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<SkillWatchPath>>>;
}

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadSkillWatchRuntime for LiveThreadSkillWatchRuntime {
        fn thread_skill_watch_paths => thread_skill_watch_paths(
            thread_id: ThreadId,
        ) -> CodexResult<Vec<SkillWatchPath>>;
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

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadInspectionRuntime for LiveThreadInspectionRuntime {
        fn list_live_thread_ids => list_live_thread_ids() -> Vec<ThreadId>;

        fn is_live_thread_loaded => is_live_thread_loaded(
            thread_id: ThreadId,
        ) -> bool;

        fn live_thread_info => live_thread_info(
            thread_id: ThreadId,
        ) -> CodexResult<LiveThreadInfo>;

        fn live_thread_snapshot => live_thread_snapshot(
            thread_id: ThreadId,
        ) -> CodexResult<LiveThreadSnapshot>;

        fn live_thread_config_snapshot => live_thread_config_snapshot(
            thread_id: ThreadId,
        ) -> CodexResult<ThreadConfigSnapshot>;

        fn live_thread_config_refresh_snapshot => live_thread_config_refresh_snapshot(
            thread_id: ThreadId,
        ) -> CodexResult<LiveThreadConfigRefreshSnapshot>;

        fn live_thread_feature_enabled => live_thread_feature_enabled(
            thread_id: ThreadId,
            feature: Feature,
        ) -> CodexResult<bool>;
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
    T: LiveThreadFeedbackRuntime + ThreadAgentDirectoryRuntime + Send + Sync,
{
    fn list_agent_subtree_thread_ids(
        &self,
        thread_id: ThreadId,
    ) -> BoxFuture<'_, CodexResult<Vec<ThreadId>>> {
        Box::pin(ThreadAgentDirectoryRuntime::list_agent_subtree_thread_ids(
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

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadGoalRuntime for LiveThreadGoalRuntime {
        fn prepare_thread_external_goal_mutation => prepare_thread_external_goal_mutation(
            thread_id: ThreadId,
        ) -> CodexResult<()>;

        fn apply_thread_external_goal_set => apply_thread_external_goal_set(
            thread_id: ThreadId,
            external_set: ExternalGoalSet,
        ) -> CodexResult<()>;

        fn apply_thread_external_goal_clear => apply_thread_external_goal_clear(
            thread_id: ThreadId,
        ) -> CodexResult<()>;

        fn apply_thread_goal_resume_runtime_effects => apply_thread_goal_resume_runtime_effects(
            thread_id: ThreadId,
        ) -> CodexResult<()>;

        fn continue_thread_active_goal_if_idle => continue_thread_active_goal_if_idle(
            thread_id: ThreadId,
        ) -> CodexResult<()>;
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

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadElicitationRuntime for LiveThreadElicitationRuntime {
        fn increment_thread_out_of_band_elicitation_count => increment_thread_out_of_band_elicitation_count(
            thread_id: ThreadId,
        ) -> CodexResult<u64>;

        fn decrement_thread_out_of_band_elicitation_count => decrement_thread_out_of_band_elicitation_count(
            thread_id: ThreadId,
        ) -> CodexResult<u64>;
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

pub(crate) trait AppServerLiveThreadTerminalRuntime: Send + Sync {
    fn update_live_thread_preferred_terminal_size(
        &self,
        thread_id: ThreadId,
        size: thread_service_api::PreferredTerminalSize,
    ) -> BoxFuture<'_, CodexResult<()>>;
}

pub(crate) trait AppServerLiveThreadClientRecoveryRuntime: Send + Sync {
    fn record_live_thread_client_recovery(
        &self,
        thread_id: ThreadId,
        event: protocol::protocol::ClientRecoveryEvent,
    ) -> BoxFuture<'_, CodexResult<bool>>;
}

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadClientRecoveryRuntime for LiveThreadClientRecoveryRuntime {
        fn record_live_thread_client_recovery => record_live_thread_client_recovery(
            thread_id: ThreadId,
            event: protocol::protocol::ClientRecoveryEvent,
        ) -> CodexResult<bool>;
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

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadSteerRuntime for NativeThreadSteerRuntime {
        fn steer_live_thread_input => steer_live_thread_input(
            thread_id: ThreadId,
            input: Vec<UserInput>,
            expected_turn_id: Option<String>,
            responsesapi_client_metadata: Option<HashMap<String, String>>,
        ) -> CodexResult<Result<String, SteerInputError>>;
    }
}

pub(crate) trait AppServerLiveThreadTurnRuntime: Send + Sync {
    fn validate_live_thread_turn_context_overrides(
        &self,
        thread_id: ThreadId,
        overrides: CodexThreadTurnContextOverrides,
    ) -> BoxFuture<'_, CodexResult<()>>;

    fn apply_live_thread_persisted_resume_metadata(
        &self,
        thread_id: ThreadId,
        resume_config: Config,
        cwd: AbsolutePathBuf,
        root_agent_path: Option<String>,
        root_agent_role: Option<String>,
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

    fn apply_live_thread_persisted_resume_metadata(
        &self,
        thread_id: ThreadId,
        resume_config: Config,
        cwd: AbsolutePathBuf,
        root_agent_path: Option<String>,
        root_agent_role: Option<String>,
    ) -> BoxFuture<'_, CodexResult<()>> {
        Box::pin(
            LiveThreadTurnRuntime::apply_live_thread_persisted_resume_metadata(
                self,
                thread_id,
                Arc::new(resume_config),
                cwd,
                root_agent_path,
                root_agent_role,
            ),
        )
    }
}

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadCommandRuntime for LiveThreadCommandRuntime {
        fn submit_live_thread_op => submit_live_thread_op(
            thread_id: ThreadId,
            op: Op,
        ) -> CodexResult<String>;

        fn submit_live_thread_op_with_trace => submit_live_thread_op_with_trace(
            thread_id: ThreadId,
            op: Op,
            trace: Option<W3cTraceContext>,
        ) -> CodexResult<String>;

        fn set_live_thread_app_server_client_info => set_live_thread_app_server_client_info(
            thread_id: ThreadId,
            info: AppServerClientInfo,
        ) -> CodexResult<()>;
    }
}

delegate_app_server_live_thread_runtime! {
    impl AppServerLiveThreadTerminalRuntime for LiveThreadTerminalRuntime {
        fn update_live_thread_preferred_terminal_size => update_live_thread_preferred_terminal_size(
            thread_id: ThreadId,
            size: thread_service_api::PreferredTerminalSize,
        ) -> CodexResult<()>;
    }
}
