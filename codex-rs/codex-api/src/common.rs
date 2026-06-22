use futures::Stream;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::sync::mpsc;

pub use codex_api_types::CompactionInput;
pub use codex_api_types::MemorySummarizeInput;
pub use codex_api_types::MemorySummarizeOutput;
pub use codex_api_types::OpenAiVerbosity;
pub use codex_api_types::RawMemory;
pub use codex_api_types::RawMemoryMetadata;
pub use codex_api_types::Reasoning;
pub use codex_api_types::ResponseCreateWsRequest;
pub use codex_api_types::ResponseEvent;
pub use codex_api_types::ResponseProcessedWsRequest;
pub use codex_api_types::ResponseStream;
pub use codex_api_types::ResponsesApiRequest;
pub use codex_api_types::ResponsesOptions;
pub use codex_api_types::ResponsesWsRequest;
pub use codex_api_types::TextControls;
pub use codex_api_types::WS_REQUEST_HEADER_TRACEPARENT_CLIENT_METADATA_KEY;
pub use codex_api_types::WS_REQUEST_HEADER_TRACESTATE_CLIENT_METADATA_KEY;
pub use codex_api_types::create_text_param_for_request;
pub use codex_api_types::response_create_client_metadata;

struct ReceiverResponseStream {
    rx_event: mpsc::Receiver<Result<ResponseEvent, codex_api_types::ApiError>>,
}

impl Stream for ReceiverResponseStream {
    type Item = Result<ResponseEvent, codex_api_types::ApiError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx_event.poll_recv(cx)
    }
}

pub(crate) fn response_stream_from_receiver(
    rx_event: mpsc::Receiver<Result<ResponseEvent, codex_api_types::ApiError>>,
    upstream_request_id: Option<String>,
) -> ResponseStream {
    ResponseStream::new(ReceiverResponseStream { rx_event }, upstream_request_id)
}
