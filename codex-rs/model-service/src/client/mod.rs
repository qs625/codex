//! Session- and turn-scoped helpers for talking to model provider APIs.
//!
//! `ModelClient` is intended to live for the lifetime of a Codex session and holds the stable
//! configuration and state needed to talk to a provider (auth, provider selection, conversation id,
//! and transport fallback state).
//!
//! Per-turn settings (model selection, reasoning controls, telemetry context, and turn metadata)
//! are passed explicitly to streaming and unary methods so that the turn lifetime is visible at the
//! call site.
//!
//! A [`ModelClientSession`] is created per turn and is used to stream one or more Responses API
//! requests during that turn. It caches a Responses WebSocket connection (opened lazily) and stores
//! per-turn state such as the `x-codex-turn-state` token used for sticky routing.
//!
//! WebSocket prewarm is a v2-only `response.create` with `generate=false`; it waits for completion
//! so the next request can reuse the same connection and `previous_response_id`.
//!
//! Turn execution performs prewarm as a best-effort step before the first stream request so the
//! subsequent request can reuse the same connection.
//!
//! ## Retry-Budget Tradeoff
//!
//! WebSocket prewarm is treated as the first websocket connection attempt for a turn. If it
//! fails, normal stream retry/fallback logic handles recovery on the same turn.

pub mod attestation;
mod chat_completions;
mod identity;
mod model_client;
mod prompt;
mod realtime;
mod turn_session;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use codex_auth_types::AuthMode;
use transport_client_types::RequestTelemetry;
use transport_client_types::TransportError;
use codex_otel::current_span_w3c_trace_context;
use model_service_api::ApiError;
use model_service_api::AuthProvider;
use model_service_api::ChatCompletionsPath as ApiChatCompletionsPath;
use model_service_api::ChatCompletionsRuntimeRequest;
use model_service_api::CompactInputRuntimeRequest;
use model_service_api::CompactionInput as ApiCompactionInput;
use model_service_api::Compression;
use model_service_api::MemorySummarizeInput as ApiMemorySummarizeInput;
use model_service_api::MemorySummarizeOutput as ApiMemorySummarizeOutput;
use model_service_api::MemorySummarizeRuntimeRequest;
use model_service_api::Provider as ApiProvider;
use model_service_api::RawMemory as ApiRawMemory;
use model_service_api::RealtimeCallRuntimeRequest;
use model_service_api::RealtimeSessionConfig as ApiRealtimeSessionConfig;
use model_service_api::RealtimeWebsocketClientRuntime;
use model_service_api::Reasoning;
use model_service_api::ResponseCreateWsRequest;
use model_service_api::ResponseStream as ApiResponseStream;
use model_service_api::ResponsesApiRequest;
use model_service_api::ResponsesOptions as ApiResponsesOptions;
use model_service_api::ResponsesStreamRuntimeRequest;
use model_service_api::ResponsesWebsocketConnectRequest;
use model_service_api::ResponsesWebsocketConnectionRuntime;
use model_service_api::ResponsesWebsocketConnectorRuntime;
use model_service_api::ResponsesWsRequest;
use model_service_api::SharedAuthProvider;
use model_service_api::SseEventTelemetry;
use model_service_api::SseTelemetry;
use model_service_api::WebsocketEventTelemetry;
use model_service_api::WebsocketTelemetry;
use model_service_api::X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER;
use model_service_api::auth_header_telemetry;
use model_service_api::build_session_headers;
use model_service_api::create_text_param_for_request;
use model_service_api::response_create_client_metadata;
use session_telemetry_api::SharedSessionTelemetry;
use session_telemetry_api::SseEventTelemetry as SessionSseEventTelemetry;
use session_telemetry_api::WebsocketEventTelemetry as SessionWebsocketEventTelemetry;

use futures::StreamExt;
use http::HeaderMap as ApiHeaderMap;
use http::HeaderValue;
use http::StatusCode as HttpStatusCode;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::config_types::Verbosity as VerbosityConfig;
use protocol::models::ResponseItem;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use protocol::protocol::InternalSessionSource;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::W3cTraceContext;
use rollout_trace_api::CompactionTraceContext;
use rollout_trace_api::InferenceTraceAttempt;
use rollout_trace_api::InferenceTraceContext;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;
use tokio_util::sync::CancellationToken;
use tool_service_api::create_tools_json_for_responses_api;
use tracing::debug;
use tracing::instrument;
use tracing::trace;
use tracing::warn;

use codex_auth_types::AuthEnvTelemetry;
use codex_auth_types::AuthEnvTelemetryInput;
use codex_auth_types::collect_auth_env_telemetry;
use codex_feedback_api::FeedbackRequestTags;
use codex_feedback_api::emit_feedback_request_tags_with_auth_env;
#[cfg(test)]
use model_service_api::DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS;
use model_service_api::ModelProviderAuthRecoveryError;
use model_service_api::ModelProviderInfo;
use model_service_api::ModelProviderUnauthorizedRecovery;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelProvider;
use model_service_api::SharedModelProviderAuthManager;
use model_service_api::SharedModelProviderFactory;
use model_service_api::WireApi;
use model_service_api::extract_response_debug_context;
use model_service_api::extract_response_debug_context_from_api_error;
use model_service_api::map_api_error;
use model_service_api::telemetry_api_error_message;
use model_service_api::telemetry_transport_error_message;
use protocol::error::CodexErr;
use protocol::error::Result;

pub use attestation::AttestationContext;
pub use attestation::AttestationProvider;
pub use attestation::GenerateAttestationFuture;
pub use attestation::X_OAI_ATTESTATION_HEADER;
pub use prompt::Prompt;
pub use prompt::PromptBuildParams;
pub use prompt::REVIEW_EXIT_INTERRUPTED_TMPL;
pub use prompt::REVIEW_EXIT_SUCCESS_TMPL;
pub use prompt::REVIEW_PROMPT;
pub use prompt::ResponseEvent;
pub use prompt::ResponseStream;
pub use prompt::build_prompt;

macro_rules! feedback_tags {
    ($( $key:ident = $value:expr ),+ $(,)?) => {
        ::tracing::info!(
            target: "feedback_tags",
            $( $key = ::tracing::field::debug(&$value) ),+
        );
    };
}

mod headers;
mod stream;
mod telemetry;
mod websocket;

use headers::build_responses_headers;
use headers::parent_thread_id_header_value;
use headers::parse_turn_metadata_header;
use headers::sideband_websocket_auth_headers;
use headers::stamp_ws_stream_request_start_ms;
use headers::subagent_header_value;
use stream::LastResponse;
use stream::map_response_stream;
use telemetry::ApiTelemetry;
use telemetry::AuthRequestTelemetryContext;
use telemetry::PendingUnauthorizedRetry;
use telemetry::api_error_http_status;
use telemetry::handle_unauthorized;
use websocket::WebsocketConnectParams;
use websocket::WebsocketSession;
use websocket::WebsocketStreamOutcome;

pub const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";
pub const X_CODEX_INSTALLATION_ID_HEADER: &str = "x-codex-installation-id";
pub const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";
pub const X_CODEX_TURN_METADATA_HEADER: &str = "x-codex-turn-metadata";
pub const X_CODEX_PARENT_THREAD_ID_HEADER: &str = "x-codex-parent-thread-id";
pub const X_CODEX_WINDOW_ID_HEADER: &str = "x-codex-window-id";
pub const X_OPENAI_MEMGEN_REQUEST_HEADER: &str = "x-openai-memgen-request";
pub const X_OPENAI_SUBAGENT_HEADER: &str = "x-openai-subagent";
const X_CODEX_WS_STREAM_REQUEST_START_MS_CLIENT_METADATA_KEY: &str =
    "x-codex-ws-stream-request-start-ms";
const RESPONSES_WEBSOCKETS_V2_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const RESPONSES_ENDPOINT: &str = "/responses";
const CHAT_COMPLETIONS_ENDPOINT: &str = "/chat/completions";
const RESPONSES_COMPACT_ENDPOINT: &str = "/responses/compact";
const MEMORIES_SUMMARIZE_ENDPOINT: &str = "/memories/trace_summarize";
#[cfg(test)]
pub(crate) const WEBSOCKET_CONNECT_TIMEOUT: Duration =
    Duration::from_millis(DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS);

pub struct CompactConversationRequestSettings {
    pub effort: Option<ReasoningEffortConfig>,
    pub summary: ReasoningSummaryConfig,
    pub service_tier: Option<String>,
}

/// Session-scoped state shared by all [`ModelClient`] clones.
///
/// This is intentionally kept minimal so `ModelClient` does not need to hold a full `Config`. Most
/// configuration is per turn and is passed explicitly to streaming/unary methods.
struct ModelClientState {
    session_id: SessionId,
    thread_id: ThreadId,
    window_generation: AtomicU64,
    installation_id: String,
    api_runtime_factory: SharedApiRuntimeFactory,
    model_provider_factory: SharedModelProviderFactory,
    provider: SharedModelProvider,
    auth_env_telemetry: AuthEnvTelemetry,
    session_source: SessionSource,
    model_verbosity: Option<VerbosityConfig>,
    chat_completions_max_tokens_by_model: HashMap<String, u64>,
    enable_request_compression: bool,
    include_timing_metrics: bool,
    beta_features_header: Option<String>,
    include_attestation: bool,
    attestation_provider: Option<Arc<dyn AttestationProvider>>,
    disable_websockets: AtomicBool,
    cached_websocket_session: StdMutex<WebsocketSession>,
}

impl std::fmt::Debug for ModelClientState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelClientState")
            .field("session_id", &self.session_id)
            .field("thread_id", &self.thread_id)
            .field("window_generation", &self.window_generation)
            .field("installation_id", &self.installation_id)
            .field("api_runtime_factory", &"<api runtime factory>")
            .field("model_provider_factory", &"<model provider factory>")
            .field("provider", &self.provider)
            .field("auth_env_telemetry", &self.auth_env_telemetry)
            .field("session_source", &self.session_source)
            .field("model_verbosity", &self.model_verbosity)
            .field(
                "chat_completions_max_tokens_by_model",
                &self.chat_completions_max_tokens_by_model,
            )
            .field(
                "enable_request_compression",
                &self.enable_request_compression,
            )
            .field("include_timing_metrics", &self.include_timing_metrics)
            .field("beta_features_header", &self.beta_features_header)
            .field("include_attestation", &self.include_attestation)
            .field("attestation_provider", &self.attestation_provider.is_some())
            .field("disable_websockets", &self.disable_websockets)
            .field("cached_websocket_session", &self.cached_websocket_session)
            .finish()
    }
}

/// Resolved API client setup for a single request attempt.
///
/// Keeping this as a single bundle ensures prewarm and normal request paths
/// share the same auth/provider setup flow.
struct CurrentClientSetup {
    auth: Option<codex_auth_types::RequestAuthSnapshot>,
    api_provider: ApiProvider,
    api_auth: SharedAuthProvider,
}

#[derive(Clone, Copy)]
struct RequestRouteTelemetry {
    endpoint: &'static str,
}

impl RequestRouteTelemetry {
    fn for_endpoint(endpoint: &'static str) -> Self {
        Self { endpoint }
    }
}

/// A session-scoped client for model-provider API calls.
///
/// This holds configuration and state that should be shared across turns within a Codex session
/// (auth, provider selection, thread id, and transport fallback state).
///
/// WebSocket fallback is session-scoped: once a turn activates the HTTP fallback, subsequent turns
/// will also use HTTP for the remainder of the session.
///
/// Turn-scoped settings (model selection, reasoning controls, telemetry context, and turn
/// metadata) are passed explicitly to the relevant methods to keep turn lifetime visible at the
/// call site.
#[derive(Debug, Clone)]
pub struct ModelClient {
    state: Arc<ModelClientState>,
}

/// A turn-scoped streaming session created from a [`ModelClient`].
///
/// The session establishes a Responses WebSocket connection lazily and reuses it across multiple
/// requests within the turn. It also caches per-turn state:
///
/// - The last full request, so subsequent calls can reuse incremental websocket request payloads
///   only when the current request is an incremental extension of the previous one.
/// - The `x-codex-turn-state` sticky-routing token, which must be replayed for all requests within
///   the same turn.
///
/// Create a fresh `ModelClientSession` for each Codex turn. Reusing it across turns would replay
/// the previous turn's sticky-routing token into the next turn, which violates the client/server
/// contract and can cause routing bugs.
pub struct ModelClientSession {
    client: ModelClient,
    websocket_session: WebsocketSession,
    /// Turn state for sticky routing.
    ///
    /// This is an `OnceLock` that stores the turn state value received from the server
    /// on turn start via the `x-codex-turn-state` response header. Once set, this value
    /// should be sent back to the server in the `x-codex-turn-state` request header for
    /// all subsequent requests within the same turn to maintain sticky routing.
    ///
    /// This is a contract between the client and server: we receive it at turn start,
    /// keep sending it unchanged between turn requests (e.g., for retries, incremental
    /// appends, or continuation requests), and must not send it between different turns.
    turn_state: Arc<OnceLock<String>>,
}

type ApiWebSocketConnection = Box<dyn ResponsesWebsocketConnectionRuntime>;

/// Result of opening a WebRTC Realtime call.
///
/// The SDP answer goes back to the client. The call id and auth headers stay on the server so the
/// ordinary Realtime WebSocket machinery can join the same in-progress call as a sideband
/// controller.
pub struct RealtimeWebrtcCallStart {
    pub sdp: String,
    pub call_id: String,
    pub sideband_headers: ApiHeaderMap,
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
