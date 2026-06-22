use codex_api_provider::Provider;
use codex_api_provider::SharedAuthProvider;
use codex_api_types::ApiError;
use codex_api_types::ApiRuntimeFuture;
use codex_api_types::ArcMonitorRuntimeRequest;
use codex_api_types::ArcMonitorRuntimeResponse;
use codex_api_types::ChatCompletionsRuntimeRequest;
use codex_api_types::CompactInputRuntimeRequest;
use codex_api_types::CompactionOutput;
use codex_api_types::MemorySummarizeOutput;
use codex_api_types::MemorySummarizeRuntimeRequest;
use codex_api_types::RealtimeCallResponse;
use codex_api_types::RealtimeCallRuntimeRequest;
use codex_api_types::RealtimeWebsocketClientRuntime;
use codex_api_types::ResponseStream;
use codex_api_types::ResponsesStreamRuntimeRequest;
use codex_api_types::ResponsesWebsocketConnectorRuntime;
use std::sync::Arc;

/// Shared handle for API runtime factories injected by composition roots.
pub type SharedApiRuntimeFactory = Arc<dyn ApiRuntimeFactory>;

/// Creates runtime clients for concrete Codex API transports.
///
/// Implementations own concrete HTTP/WebSocket transport construction, custom CA policy, and any
/// runtime task setup. Consumers should request endpoint runtimes through this trait instead of
/// depending on the heavy `codex-api` implementation crate.
pub trait ApiRuntimeFactory: Send + Sync {
    fn responses_websocket_connector(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesWebsocketConnectorRuntime>;

    fn compact_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn CompactClientRuntime>;

    fn memories_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn MemoriesClientRuntime>;

    fn chat_completions_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ChatCompletionsClientRuntime>;

    fn realtime_call_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn RealtimeCallClientRuntime>;

    fn realtime_websocket_client(
        &self,
        provider: Provider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime>;

    fn responses_client(
        &self,
        provider: Provider,
        auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesClientRuntime>;

    fn arc_monitor_client(&self) -> Box<dyn ArcMonitorClientRuntime>;
}

/// Runtime capability for the compaction endpoint.
///
/// Implementations own concrete HTTP transport setup and request execution. Consumers pass typed
/// payloads, headers, and optional telemetry without depending on the concrete API client crate.
pub trait CompactClientRuntime: Send + Sync {
    fn compact_input<'a>(
        &'a self,
        request: CompactInputRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<CompactionOutput, ApiError>>;
}

/// Runtime capability for the memory summarize endpoint.
///
/// Implementations own concrete HTTP transport setup and request execution. Consumers pass typed
/// payloads, headers, and optional telemetry without depending on the concrete API client crate.
pub trait MemoriesClientRuntime: Send + Sync {
    fn summarize_input<'a>(
        &'a self,
        request: MemorySummarizeRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<Vec<MemorySummarizeOutput>, ApiError>>;
}

/// Runtime capability for Chat Completions-compatible endpoints.
///
/// Implementations own concrete HTTP transport setup, request adaptation, and response parsing.
/// Consumers pass typed Responses-shaped requests and receive a transport-neutral event stream.
pub trait ChatCompletionsClientRuntime: Send + Sync {
    fn create(
        &self,
        request: ChatCompletionsRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>>;
}

/// Runtime capability for creating realtime WebRTC calls.
///
/// Implementations own concrete HTTP transport setup and endpoint-specific request encoding.
pub trait RealtimeCallClientRuntime: Send + Sync {
    fn create(
        &self,
        request: RealtimeCallRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<RealtimeCallResponse, ApiError>>;
}

/// Runtime capability for streaming Responses API requests over HTTP/SSE.
///
/// Implementations own concrete HTTP transport setup, request encoding, compression, SSE parsing,
/// and background stream task setup. Consumers pass typed Responses request/options and receive a
/// transport-neutral event stream.
pub trait ResponsesClientRuntime: Send + Sync {
    fn stream_request(
        &self,
        request: ResponsesStreamRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>>;
}

/// Runtime capability for ARC monitor HTTP checks.
///
/// Implementations own concrete HTTP client construction and request execution. Consumers pass a
/// JSON body and already selected auth material without depending on the concrete reqwest runtime.
pub trait ArcMonitorClientRuntime: Send + Sync {
    fn send(
        &self,
        request: ArcMonitorRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ArcMonitorRuntimeResponse, ApiError>>;
}

/// API runtime factory used by tests or sample paths that should not perform network I/O.
pub struct DisabledApiRuntimeFactory;

impl ApiRuntimeFactory for DisabledApiRuntimeFactory {
    fn responses_websocket_connector(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesWebsocketConnectorRuntime> {
        Box::new(DisabledResponsesWebsocketConnectorRuntime)
    }

    fn compact_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn CompactClientRuntime> {
        Box::new(DisabledCompactClientRuntime)
    }

    fn memories_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn MemoriesClientRuntime> {
        Box::new(DisabledMemoriesClientRuntime)
    }

    fn chat_completions_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn ChatCompletionsClientRuntime> {
        Box::new(DisabledChatCompletionsClientRuntime)
    }

    fn realtime_call_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn RealtimeCallClientRuntime> {
        Box::new(DisabledRealtimeCallClientRuntime)
    }

    fn realtime_websocket_client(
        &self,
        _provider: Provider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime> {
        Box::new(DisabledRealtimeWebsocketClientRuntime)
    }

    fn responses_client(
        &self,
        _provider: Provider,
        _auth: SharedAuthProvider,
    ) -> Box<dyn ResponsesClientRuntime> {
        Box::new(DisabledResponsesClientRuntime)
    }

    fn arc_monitor_client(&self) -> Box<dyn ArcMonitorClientRuntime> {
        Box::new(DisabledArcMonitorClientRuntime)
    }
}

struct DisabledResponsesWebsocketConnectorRuntime;

impl ResponsesWebsocketConnectorRuntime for DisabledResponsesWebsocketConnectorRuntime {
    fn connect(
        &self,
        _request: codex_api_types::ResponsesWebsocketConnectRequest,
    ) -> codex_api_types::ApiRuntimeFuture<
        '_,
        Result<Box<dyn codex_api_types::ResponsesWebsocketConnectionRuntime>, ApiError>,
    > {
        Box::pin(async {
            Err(ApiError::Stream(
                "Responses WebSocket runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledCompactClientRuntime;

impl CompactClientRuntime for DisabledCompactClientRuntime {
    fn compact_input<'a>(
        &'a self,
        _request: CompactInputRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<CompactionOutput, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "Compact endpoint runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledMemoriesClientRuntime;

impl MemoriesClientRuntime for DisabledMemoriesClientRuntime {
    fn summarize_input<'a>(
        &'a self,
        _request: MemorySummarizeRuntimeRequest<'a>,
    ) -> ApiRuntimeFuture<'a, Result<Vec<MemorySummarizeOutput>, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "Memories endpoint runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledChatCompletionsClientRuntime;

impl ChatCompletionsClientRuntime for DisabledChatCompletionsClientRuntime {
    fn create(
        &self,
        _request: ChatCompletionsRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "Chat Completions endpoint runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledRealtimeCallClientRuntime;

impl RealtimeCallClientRuntime for DisabledRealtimeCallClientRuntime {
    fn create(
        &self,
        _request: RealtimeCallRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<RealtimeCallResponse, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "Realtime call endpoint runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledRealtimeWebsocketClientRuntime;

impl RealtimeWebsocketClientRuntime for DisabledRealtimeWebsocketClientRuntime {
    fn connect(
        &self,
        _request: codex_api_types::RealtimeWebsocketConnectRuntimeRequest,
    ) -> ApiRuntimeFuture<
        '_,
        Result<Box<dyn codex_api_types::RealtimeWebsocketConnectionRuntime>, ApiError>,
    > {
        Box::pin(async {
            Err(ApiError::Stream(
                "Realtime WebSocket runtime is not configured".to_string(),
            ))
        })
    }

    fn connect_webrtc_sideband(
        &self,
        _request: codex_api_types::RealtimeWebrtcSidebandConnectRuntimeRequest,
    ) -> ApiRuntimeFuture<
        '_,
        Result<Box<dyn codex_api_types::RealtimeWebsocketConnectionRuntime>, ApiError>,
    > {
        Box::pin(async {
            Err(ApiError::Stream(
                "Realtime WebSocket runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledResponsesClientRuntime;

impl ResponsesClientRuntime for DisabledResponsesClientRuntime {
    fn stream_request(
        &self,
        _request: ResponsesStreamRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "Responses endpoint runtime is not configured".to_string(),
            ))
        })
    }
}

struct DisabledArcMonitorClientRuntime;

impl ArcMonitorClientRuntime for DisabledArcMonitorClientRuntime {
    fn send(
        &self,
        _request: ArcMonitorRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<ArcMonitorRuntimeResponse, ApiError>> {
        Box::pin(async {
            Err(ApiError::Stream(
                "ARC monitor runtime is not configured".to_string(),
            ))
        })
    }
}
