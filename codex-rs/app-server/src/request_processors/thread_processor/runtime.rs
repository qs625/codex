use super::*;

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

pub(crate) trait ThreadProcessorCreatedThread: Send + Sync {
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
