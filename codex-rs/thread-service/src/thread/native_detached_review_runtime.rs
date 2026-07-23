use config_service::Config;
use protocol::ThreadId;
use protocol::error::Result as CodexResult;
use protocol::protocol::W3cTraceContext;
use rollout_api::ForkSnapshot;
use thread_service_api::ThreadServiceFuture;
use thread_store_api::ReadThreadParams;
use thread_store_api::StoredThread;

use super::ThreadService;

pub trait NativeDetachedReviewRuntime: Send + Sync {
    fn fork_detached_review_thread<'a>(
        &'a self,
        parent_thread_id: ThreadId,
        config: Config,
        trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<ThreadId>>;

    fn read_detached_review_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<StoredThread>>;
}

impl NativeDetachedReviewRuntime for ThreadService {
    fn fork_detached_review_thread<'a>(
        &'a self,
        parent_thread_id: ThreadId,
        config: Config,
        trace: Option<W3cTraceContext>,
    ) -> ThreadServiceFuture<'a, CodexResult<ThreadId>> {
        Box::pin(async move {
            let new_thread = ThreadService::fork_live_thread_from_current_history(
                self,
                parent_thread_id,
                ForkSnapshot::Interrupted,
                config,
                /*thread_source*/ None,
                /*persist_extended_history*/ false,
                trace,
            )
            .await?;
            Ok(new_thread.thread_id)
        })
    }

    fn read_detached_review_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<StoredThread>> {
        Box::pin(async move {
            ThreadService::read_thread(
                self,
                ReadThreadParams {
                    thread_id,
                    include_archived: true,
                    include_history: false,
                },
            )
            .await
        })
    }
}
