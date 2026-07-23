use super::*;
use codex_agent_runtime::AgentMetadata;
use thread_service_api::ThreadLifecycleRuntime;

pub(crate) trait ThreadProcessorMetadataRuntime: Send + Sync {
    fn update_thread_metadata<'a>(
        &'a self,
        thread_id: ThreadId,
        patch: StoreThreadMetadataPatch,
        include_archived: bool,
    ) -> futures::future::BoxFuture<'a, CodexResult<StoredThread>>;
}

impl ThreadProcessorMetadataRuntime for ThreadService {
    fn update_thread_metadata<'a>(
        &'a self,
        thread_id: ThreadId,
        patch: StoreThreadMetadataPatch,
        include_archived: bool,
    ) -> futures::future::BoxFuture<'a, CodexResult<StoredThread>> {
        Box::pin(ThreadService::update_thread_metadata(
            self,
            thread_id,
            patch,
            include_archived,
        ))
    }
}

pub(crate) trait ThreadProcessorCreatedThread: AppServerLiveThreadHandle {
    fn record_startup_phase(
        &self,
        phase: &'static str,
        duration: Duration,
        status: Option<&'static str>,
    );
}

impl ThreadProcessorCreatedThread for thread_service::CodexThread {
    fn record_startup_phase(
        &self,
        phase: &'static str,
        duration: Duration,
        status: Option<&'static str>,
    ) {
        self.session_telemetry()
            .record_startup_phase(phase, duration, status);
    }
}

pub(crate) struct ThreadProcessorNewThread {
    pub(crate) thread_id: ThreadId,
    pub(crate) thread: Arc<dyn ThreadProcessorCreatedThread>,
    pub(crate) session_configured: SessionConfiguredEvent,
}

pub(crate) fn thread_processor_new_thread(new_thread: NewThread) -> ThreadProcessorNewThread {
    let NewThread {
        thread_id,
        thread,
        session_configured,
    } = new_thread;
    let thread: Arc<dyn ThreadProcessorCreatedThread> = thread;
    ThreadProcessorNewThread {
        thread_id,
        thread,
        session_configured,
    }
}

pub(crate) trait ThreadProcessorThreadRuntime: ThreadLifecycleRuntime {
    fn default_environment_selections(
        &self,
        cwd: &AbsolutePathBuf,
    ) -> Vec<TurnEnvironmentSelection>;

    fn validate_environment_selections(
        &self,
        environments: &[TurnEnvironmentSelection],
    ) -> CodexResult<()>;

    fn start_thread_with_options<'a>(
        &'a self,
        options: StartThreadOptions,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>>;

    fn resume_thread_with_history<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>>;

    fn resume_thread_with_history_and_source<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        session_source: protocol::protocol::SessionSource,
        agent_metadata: Option<AgentMetadata>,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>>;

    fn fork_thread_from_history<'a>(
        &'a self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<protocol::protocol::ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>>;
}

impl ThreadProcessorThreadRuntime for ThreadService {
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

    fn start_thread_with_options<'a>(
        &'a self,
        options: StartThreadOptions,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>> {
        Box::pin(async move {
            ThreadService::start_thread_with_options(self, options)
                .await
                .map(thread_processor_new_thread)
        })
    }

    fn resume_thread_with_history<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>> {
        Box::pin(async move {
            ThreadService::resume_thread_with_history(
                self,
                config,
                initial_history,
                persist_extended_history,
                parent_trace,
            )
            .await
            .map(thread_processor_new_thread)
        })
    }

    fn resume_thread_with_history_and_source<'a>(
        &'a self,
        config: Config,
        initial_history: InitialHistory,
        session_source: protocol::protocol::SessionSource,
        agent_metadata: Option<AgentMetadata>,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>> {
        Box::pin(async move {
            ThreadService::resume_thread_with_history_source_and_agent_metadata(
                self,
                config,
                initial_history,
                session_source,
                agent_metadata,
                parent_trace,
            )
            .await
            .map(thread_processor_new_thread)
        })
    }

    fn fork_thread_from_history<'a>(
        &'a self,
        snapshot: ForkSnapshot,
        config: Config,
        history: InitialHistory,
        thread_source: Option<protocol::protocol::ThreadSource>,
        persist_extended_history: bool,
        parent_trace: Option<W3cTraceContext>,
    ) -> futures::future::BoxFuture<'a, CodexResult<ThreadProcessorNewThread>> {
        Box::pin(async move {
            ThreadService::fork_thread_from_history(
                self,
                snapshot,
                config,
                history,
                thread_source,
                persist_extended_history,
                parent_trace,
            )
            .await
            .map(thread_processor_new_thread)
        })
    }
}
