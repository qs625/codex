use std::collections::HashMap;

use protocol::ThreadId;
use protocol::error::Result as CodexResult;
use protocol::user_input::UserInput;
use thread_service_api::ThreadServiceFuture;

use crate::SteerInputError;

use super::ThreadService;

pub trait NativeThreadSteerRuntime: Send + Sync {
    fn steer_live_thread_input<'a>(
        &'a self,
        thread_id: ThreadId,
        input: Vec<UserInput>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> ThreadServiceFuture<'a, CodexResult<Result<String, SteerInputError>>>;
}

impl NativeThreadSteerRuntime for ThreadService {
    fn steer_live_thread_input<'a>(
        &'a self,
        thread_id: ThreadId,
        input: Vec<UserInput>,
        expected_turn_id: Option<String>,
        responsesapi_client_metadata: Option<HashMap<String, String>>,
    ) -> ThreadServiceFuture<'a, CodexResult<Result<String, SteerInputError>>> {
        Box::pin(async move {
            ThreadService::steer_thread_input(
                self,
                thread_id,
                input,
                expected_turn_id.as_deref(),
                responsesapi_client_metadata,
            )
            .await
        })
    }
}
