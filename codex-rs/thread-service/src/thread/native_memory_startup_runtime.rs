use std::sync::Arc;

use config_service::Config;
use protocol::ThreadId;
use protocol::error::Result as CodexResult;
use thread_service_api::ThreadServiceFuture;

use super::ThreadService;

pub trait NativeMemoryStartupConfigRuntime: Send + Sync {
    fn live_thread_memory_startup_config<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<Arc<Config>>>;
}

impl NativeMemoryStartupConfigRuntime for ThreadService {
    fn live_thread_memory_startup_config<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> ThreadServiceFuture<'a, CodexResult<Arc<Config>>> {
        Box::pin(ThreadService::live_thread_config(self, thread_id))
    }
}
