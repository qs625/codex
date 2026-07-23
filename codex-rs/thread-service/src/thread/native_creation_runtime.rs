use codex_agent_runtime::AgentMetadata;
use codex_utils_absolute_path::AbsolutePathBuf;
use config_service::Config;
use protocol::error::Result as CodexResult;
use protocol::protocol::InitialHistory;
use protocol::protocol::SessionSource;
use protocol::protocol::ThreadSource;
use protocol::protocol::TurnEnvironmentSelection;
use protocol::protocol::W3cTraceContext;
use rollout_api::ForkSnapshot;
use thread_service_api::ThreadServiceFuture;

use super::NewThread;
use super::StartThreadOptions;
use super::ThreadService;

pub trait NativeThreadEnvironmentRuntime: Send + Sync {
    fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection>;

    fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()>;
}

pub trait NativeThreadCreationRuntime: Send + Sync {
    fn start_thread_with_options<'a>(
        &'a self,
        options: StartThreadOptions,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>>;

    fn resume_thread_with_history<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>>;

    fn resume_thread_with_history_and_source<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        session_source: SessionSource,
        agent_metadata: Option<AgentMetadata>,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>>;

    fn fork_thread_from_history<'a>(
        &'a self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>>;
}

impl NativeThreadEnvironmentRuntime for ThreadService {
    fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection> {
        ThreadService::default_environment_selections(self, cwd)
    }

    fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()> {
        ThreadService::validate_environment_selections(self, environments)
    }
}

impl NativeThreadCreationRuntime for ThreadService {
    fn start_thread_with_options<'a>(
        &'a self,
        options: StartThreadOptions,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>> {
        Box::pin(ThreadService::start_thread_with_options(self, options))
    }

    fn resume_thread_with_history<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>> {
        Box::pin(ThreadService::resume_thread_with_history(
            self,
            config,
            initial_history,
            persist_extended_history,
            parent_trace,
        ))
    }

    fn resume_thread_with_history_and_source<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        session_source: SessionSource,
        agent_metadata: Option<AgentMetadata>,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>> {
        Box::pin(
            ThreadService::resume_thread_with_history_source_and_agent_metadata(
                self,
                config,
                initial_history,
                session_source,
                agent_metadata,
                parent_trace,
            ),
        )
    }

    fn fork_thread_from_history<'a>(
        &'a self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<NewThread>> {
        Box::pin(ThreadService::fork_thread_from_history(
            self,
            snapshot,
            config,
            history,
            thread_source,
            persist_extended_history,
            parent_trace,
        ))
    }
}
