use base64::Engine;
use transport_client_types::RequestTelemetry;
use protocol::config_types::ReasoningSummary as ReasoningSummaryConfig;
use protocol::config_types::Verbosity as VerbosityConfig;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use protocol::protocol::RealtimeAudioFrame;
use protocol::protocol::RealtimeEvent;
use protocol::protocol::RealtimeOutputModality;
use protocol::protocol::RealtimeVoice;
use protocol::protocol::SessionSource;
use protocol::protocol::W3cTraceContext;
use futures::Stream;
use http::HeaderMap;
pub use session_telemetry_api::ResponseEvent;
pub use session_telemetry_api::SseEventTelemetry;
pub use session_telemetry_api::WebsocketEventTelemetry;
use http::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Result as JsonResult;
use serde_json::Value;
use serde_json::json;
use serde_json::to_value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::OnceLock;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

mod api_bridge;
mod arc_monitor;
mod error;
pub mod rate_limits;
mod response_debug_context;

pub use api_bridge::map_api_error;
pub use arc_monitor::ArcMonitorChatMessage;
pub use arc_monitor::ArcMonitorEvidence;
pub use arc_monitor::ArcMonitorMetadata;
pub use arc_monitor::ArcMonitorPolicies;
pub use arc_monitor::ArcMonitorRequest;
pub use arc_monitor::ArcMonitorResult;
pub use arc_monitor::ArcMonitorResultOutcome;
pub use arc_monitor::ArcMonitorRiskLevel;
pub use arc_monitor::build_arc_monitor_request;
pub use error::ApiError;
pub use error::extract_response_debug_context_from_api_error;
pub use error::telemetry_api_error_message;
pub use rate_limits::RateLimitError;
pub use rate_limits::parse_all_rate_limits;
pub use rate_limits::parse_default_rate_limit;
pub use rate_limits::parse_promo_message;
pub use rate_limits::parse_rate_limit_event;
pub use rate_limits::parse_rate_limit_for_limit;
pub use response_debug_context::ResponseDebugContext;
pub use response_debug_context::extract_response_debug_context;
pub use response_debug_context::telemetry_transport_error_message;

pub const WS_REQUEST_HEADER_TRACEPARENT_CLIENT_METADATA_KEY: &str = "ws_request_header_traceparent";
pub const WS_REQUEST_HEADER_TRACESTATE_CLIENT_METADATA_KEY: &str = "ws_request_header_tracestate";
pub const X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER: &str =
    "x-responsesapi-include-timing-metrics";

pub fn decoded_realtime_audio_samples_per_channel(frame: &RealtimeAudioFrame) -> Option<u32> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&frame.data)
        .ok()?;
    let channels = usize::from(frame.num_channels.max(1));
    let samples = bytes.len().checked_div(2)?.checked_div(channels)?;
    u32::try_from(samples).ok()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Compression {
    #[default]
    None,
    Zstd,
}

#[derive(Default)]
pub struct ResponsesOptions {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub session_source: Option<SessionSource>,
    pub extra_headers: HeaderMap,
    pub compression: Compression,
    pub turn_state: Option<Arc<OnceLock<String>>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ChatCompletionsPath {
    AppendChatCompletions,
    FullEndpoint,
}

impl ChatCompletionsPath {
    pub fn as_path(self) -> &'static str {
        match self {
            Self::AppendChatCompletions => "chat/completions",
            Self::FullEndpoint => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeEventParser {
    V1,
    RealtimeV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealtimeSessionMode {
    Conversational,
    Transcription,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeSessionConfig {
    pub instructions: String,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub event_parser: RealtimeEventParser,
    pub session_mode: RealtimeSessionMode,
    pub output_modality: RealtimeOutputModality,
    pub voice: RealtimeVoice,
}

const REALTIME_AUDIO_SAMPLE_RATE: u32 = 24_000;
const REALTIME_V2_OUTPUT_MODALITY_AUDIO: &str = "audio";
const REALTIME_V2_OUTPUT_MODALITY_TEXT: &str = "text";
const REALTIME_V2_TOOL_CHOICE: &str = "auto";
const REALTIME_V2_BACKGROUND_AGENT_TOOL_NAME: &str = "background_agent";
const REALTIME_V2_BACKGROUND_AGENT_TOOL_DESCRIPTION: &str = "Send a user request to the background agent. Use this as the default action. Do not rephrase the user's ask or rewrite it in your own words; pass along the user's own words. If the background agent is idle, this starts a new task and returns the final result to the user. If the background agent is already working on a task, this sends the request as guidance to steer that previous task. If the user asks to do something next, later, after this, or once current work finishes, call this tool so the work is actually queued instead of merely promising to do it later.";
const REALTIME_V2_SILENCE_TOOL_NAME: &str = "remain_silent";
const REALTIME_V2_SILENCE_TOOL_DESCRIPTION: &str = "Call this when the best response is to say nothing. Use it instead of speaking after hidden system/control messages, after background agent updates in silent modes, or whenever acknowledging aloud would be distracting. This tool has no user-visible effect.";
const REALTIME_V2_INPUT_TRANSCRIPTION_MODEL: &str = "gpt-4o-mini-transcribe";
const REALTIME_V2_TRANSCRIPTION_SESSION_MODEL: &str = "gpt-realtime-whisper";

#[derive(Debug, Clone, Serialize)]
struct SessionUpdateSessionWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(rename = "type")]
    r#type: SessionTypeWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_modalities: Option<Vec<String>>,
    audio: SessionAudioWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<SessionFunctionToolWire>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionTypeWire {
    Quicksilver,
    Realtime,
    Transcription,
}

#[derive(Debug, Clone, Serialize)]
struct SessionAudioWire {
    input: SessionAudioInputWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<SessionAudioOutputWire>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionAudioInputWire {
    format: SessionAudioFormatWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    noise_reduction: Option<SessionNoiseReductionWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<SessionInputAudioTranscriptionWire>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_detection: Option<SessionTurnDetectionWire>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionInputAudioTranscriptionWire {
    model: String,
}

#[derive(Debug, Clone, Serialize)]
struct SessionAudioFormatWire {
    #[serde(rename = "type")]
    r#type: AudioFormatTypeWire,
    rate: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum AudioFormatTypeWire {
    #[serde(rename = "audio/pcm")]
    AudioPcm,
}

#[derive(Debug, Clone, Serialize)]
struct SessionAudioOutputWire {
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<SessionAudioOutputFormatWire>,
    voice: RealtimeVoice,
}

#[derive(Debug, Clone, Serialize)]
struct SessionNoiseReductionWire {
    #[serde(rename = "type")]
    r#type: NoiseReductionTypeWire,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum NoiseReductionTypeWire {
    NearField,
}

#[derive(Debug, Clone, Serialize)]
struct SessionTurnDetectionWire {
    #[serde(rename = "type")]
    r#type: TurnDetectionTypeWire,
    interrupt_response: bool,
    create_response: bool,
    silence_duration_ms: u32,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TurnDetectionTypeWire {
    ServerVad,
}

#[derive(Debug, Clone, Serialize)]
struct SessionAudioOutputFormatWire {
    #[serde(rename = "type")]
    r#type: AudioFormatTypeWire,
    rate: u32,
}

#[derive(Debug, Clone, Serialize)]
struct SessionFunctionToolWire {
    #[serde(rename = "type")]
    r#type: SessionToolTypeWire,
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionToolTypeWire {
    Function,
}

fn normalized_session_mode(
    event_parser: RealtimeEventParser,
    session_mode: RealtimeSessionMode,
) -> RealtimeSessionMode {
    match event_parser {
        RealtimeEventParser::V1 => RealtimeSessionMode::Conversational,
        RealtimeEventParser::RealtimeV2 => session_mode,
    }
}

fn session_accepts_top_level_model(
    event_parser: RealtimeEventParser,
    session_mode: RealtimeSessionMode,
) -> bool {
    match normalized_session_mode(event_parser, session_mode) {
        RealtimeSessionMode::Conversational => true,
        RealtimeSessionMode::Transcription => false,
    }
}

fn session_update_session_wire(
    event_parser: RealtimeEventParser,
    instructions: String,
    session_mode: RealtimeSessionMode,
    output_modality: RealtimeOutputModality,
    voice: RealtimeVoice,
) -> SessionUpdateSessionWire {
    let session_mode = normalized_session_mode(event_parser, session_mode);
    match event_parser {
        RealtimeEventParser::V1 => SessionUpdateSessionWire {
            id: None,
            r#type: SessionTypeWire::Quicksilver,
            model: None,
            instructions: Some(instructions),
            output_modalities: None,
            audio: SessionAudioWire {
                input: SessionAudioInputWire {
                    format: SessionAudioFormatWire {
                        r#type: AudioFormatTypeWire::AudioPcm,
                        rate: REALTIME_AUDIO_SAMPLE_RATE,
                    },
                    noise_reduction: None,
                    transcription: None,
                    turn_detection: None,
                },
                output: Some(SessionAudioOutputWire {
                    format: None,
                    voice,
                }),
            },
            tools: None,
            tool_choice: None,
        },
        RealtimeEventParser::RealtimeV2 => match session_mode {
            RealtimeSessionMode::Conversational => SessionUpdateSessionWire {
                id: None,
                r#type: SessionTypeWire::Realtime,
                model: None,
                instructions: Some(instructions),
                output_modalities: Some(vec![output_modality_value(output_modality).to_string()]),
                audio: SessionAudioWire {
                    input: SessionAudioInputWire {
                        format: SessionAudioFormatWire {
                            r#type: AudioFormatTypeWire::AudioPcm,
                            rate: REALTIME_AUDIO_SAMPLE_RATE,
                        },
                        noise_reduction: Some(SessionNoiseReductionWire {
                            r#type: NoiseReductionTypeWire::NearField,
                        }),
                        transcription: Some(SessionInputAudioTranscriptionWire {
                            model: REALTIME_V2_INPUT_TRANSCRIPTION_MODEL.to_string(),
                        }),
                        turn_detection: Some(SessionTurnDetectionWire {
                            r#type: TurnDetectionTypeWire::ServerVad,
                            interrupt_response: true,
                            create_response: true,
                            silence_duration_ms: 500,
                        }),
                    },
                    output: Some(SessionAudioOutputWire {
                        format: Some(SessionAudioOutputFormatWire {
                            r#type: AudioFormatTypeWire::AudioPcm,
                            rate: REALTIME_AUDIO_SAMPLE_RATE,
                        }),
                        voice,
                    }),
                },
                tools: Some(vec![
                    SessionFunctionToolWire {
                        r#type: SessionToolTypeWire::Function,
                        name: REALTIME_V2_BACKGROUND_AGENT_TOOL_NAME.to_string(),
                        description: REALTIME_V2_BACKGROUND_AGENT_TOOL_DESCRIPTION.to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {
                                "prompt": {
                                    "type": "string",
                                    "description": "The user request to delegate to the background agent."
                                }
                            },
                            "required": ["prompt"],
                            "additionalProperties": false
                        }),
                    },
                    SessionFunctionToolWire {
                        r#type: SessionToolTypeWire::Function,
                        name: REALTIME_V2_SILENCE_TOOL_NAME.to_string(),
                        description: REALTIME_V2_SILENCE_TOOL_DESCRIPTION.to_string(),
                        parameters: json!({
                            "type": "object",
                            "properties": {},
                            "additionalProperties": false
                        }),
                    },
                ]),
                tool_choice: Some(REALTIME_V2_TOOL_CHOICE.to_string()),
            },
            RealtimeSessionMode::Transcription => SessionUpdateSessionWire {
                id: None,
                r#type: SessionTypeWire::Transcription,
                model: None,
                instructions: None,
                output_modalities: None,
                audio: SessionAudioWire {
                    input: SessionAudioInputWire {
                        format: SessionAudioFormatWire {
                            r#type: AudioFormatTypeWire::AudioPcm,
                            rate: REALTIME_AUDIO_SAMPLE_RATE,
                        },
                        noise_reduction: None,
                        transcription: Some(SessionInputAudioTranscriptionWire {
                            model: REALTIME_V2_TRANSCRIPTION_SESSION_MODEL.to_string(),
                        }),
                        turn_detection: None,
                    },
                    output: None,
                },
                tools: None,
                tool_choice: None,
            },
        },
    }
}

fn output_modality_value(output_modality: RealtimeOutputModality) -> &'static str {
    match output_modality {
        RealtimeOutputModality::Text => REALTIME_V2_OUTPUT_MODALITY_TEXT,
        RealtimeOutputModality::Audio => REALTIME_V2_OUTPUT_MODALITY_AUDIO,
    }
}

pub fn session_update_session_json(config: RealtimeSessionConfig) -> JsonResult<Value> {
    let RealtimeSessionConfig {
        instructions,
        model,
        session_id,
        event_parser,
        session_mode,
        output_modality,
        voice,
    } = config;
    let mut session = session_update_session_wire(
        event_parser,
        instructions,
        session_mode,
        output_modality,
        voice,
    );
    session.id = session_id;
    if session_accepts_top_level_model(event_parser, session_mode) {
        session.model = model;
    }
    to_value(session)
}

/// Answer from creating a WebRTC Realtime call.
///
/// `sdp` configures the peer connection. `call_id` is parsed from the response `Location` header
/// and is later used by the server-side sideband WebSocket to join this exact call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeCallResponse {
    pub sdp: String,
    pub call_id: String,
}

/// Close frame information captured by a handshake probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsesWebsocketClose {
    /// WebSocket close code returned by the server.
    pub code: String,
    /// Human-readable close reason returned by the server.
    pub reason: String,
}

/// Result of a handshake-only Responses WebSocket probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsesWebsocketProbe {
    /// Redacted by callers before displaying or serializing support reports.
    pub url: String,
    /// HTTP status returned by the successful WebSocket upgrade.
    pub status: StatusCode,
    /// Whether the server reported reasoning support in the upgrade response.
    pub reasoning_included: bool,
    /// Whether the server returned a model catalog ETag in the upgrade response.
    pub models_etag_present: bool,
    /// Whether the server returned a server-selected model in the upgrade response.
    pub server_model_present: bool,
    /// Close frame received immediately after upgrade, when one arrives quickly.
    pub immediate_close: Option<ResponsesWebsocketClose>,
}

/// Generic telemetry for Responses SSE transport.
pub trait SseTelemetry: Send + Sync {
    fn on_sse_poll(&self, event: Option<&SseEventTelemetry>, duration: Duration);
}

/// Telemetry for Responses WebSocket transport.
pub trait WebsocketTelemetry: Send + Sync {
    fn on_ws_request(&self, duration: Duration, error: Option<&ApiError>, connection_reused: bool);

    fn on_ws_event(&self, event: Option<&WebsocketEventTelemetry>, duration: Duration);
}

pub type ApiRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type CompactionOutput = Vec<ResponseItem>;

type ApiResponseStreamItem = Result<ResponseEvent, ApiError>;

/// Stream of Responses API events plus the upstream request id captured from the transport.
///
/// Implementations construct this from their concrete transport stream. Consumers should not depend
/// on the transport channel type.
pub struct ResponseStream {
    inner: Pin<Box<dyn Stream<Item = ApiResponseStreamItem> + Send>>,
    upstream_request_id: Option<String>,
}

impl ResponseStream {
    pub fn new<S>(stream: S, upstream_request_id: Option<String>) -> Self
    where
        S: Stream<Item = ApiResponseStreamItem> + Send + 'static,
    {
        Self {
            inner: Box::pin(stream),
            upstream_request_id,
        }
    }

    pub fn upstream_request_id(&self) -> Option<&str> {
        self.upstream_request_id.as_deref()
    }
}

impl Stream for ResponseStream {
    type Item = ApiResponseStreamItem;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Unpin for ResponseStream {}

/// Runtime capability for an established Responses WebSocket connection.
///
/// Concrete implementations own transport details such as tungstenite streams, background pump
/// tasks, and request serialization. Consumers use this trait to send Responses API frames and read
/// the typed event stream without depending on the concrete websocket runtime crate.
pub trait ResponsesWebsocketConnectionRuntime: Send + Sync {
    fn is_closed(&self) -> ApiRuntimeFuture<'_, bool>;

    fn send_response_processed(
        &self,
        response_id: String,
    ) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;

    fn stream_request(
        &self,
        request: ResponsesWsRequest,
        connection_reused: bool,
    ) -> ApiRuntimeFuture<'_, Result<ResponseStream, ApiError>>;
}

/// Request for opening a Responses WebSocket connection through a configured runtime client.
pub struct ResponsesWebsocketConnectRequest {
    pub extra_headers: HeaderMap,
    pub default_headers: HeaderMap,
    pub turn_state: Option<Arc<OnceLock<String>>>,
    pub telemetry: Option<Arc<dyn WebsocketTelemetry>>,
}

/// Runtime capability for opening a Responses WebSocket connection.
///
/// Implementations own URL construction, auth header application, TLS/custom-CA policy, and
/// concrete WebSocket transport setup. Consumers should depend on this trait instead of the
/// concrete WebSocket client method shape.
pub trait ResponsesWebsocketConnectorRuntime: Send + Sync {
    fn connect(
        &self,
        request: ResponsesWebsocketConnectRequest,
    ) -> ApiRuntimeFuture<'_, Result<Box<dyn ResponsesWebsocketConnectionRuntime>, ApiError>>;
}

/// Request for opening a realtime WebSocket connection through a configured runtime client.
pub struct RealtimeWebsocketConnectRuntimeRequest {
    pub session_config: RealtimeSessionConfig,
    pub extra_headers: HeaderMap,
    pub default_headers: HeaderMap,
}

/// Request for joining an existing WebRTC realtime call through its sideband WebSocket.
pub struct RealtimeWebrtcSidebandConnectRuntimeRequest {
    pub session_config: RealtimeSessionConfig,
    pub call_id: String,
    pub extra_headers: HeaderMap,
    pub default_headers: HeaderMap,
}

/// Runtime capability for opening realtime WebSocket connections.
///
/// Implementations own URL construction, custom CA/TLS setup, retries, and concrete WebSocket
/// transport details. Consumers provide typed session configuration and headers, then use
/// transport-neutral connection handles.
pub trait RealtimeWebsocketClientRuntime: Send + Sync {
    fn connect(
        &self,
        request: RealtimeWebsocketConnectRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<Box<dyn RealtimeWebsocketConnectionRuntime>, ApiError>>;

    fn connect_webrtc_sideband(
        &self,
        request: RealtimeWebrtcSidebandConnectRuntimeRequest,
    ) -> ApiRuntimeFuture<'_, Result<Box<dyn RealtimeWebsocketConnectionRuntime>, ApiError>>;
}

/// Runtime capability for an established realtime WebSocket connection.
///
/// Implementations expose cheap cloneable writer/event handles while keeping concrete transport
/// channels, pump tasks, and protocol framing inside the API runtime implementation crate.
pub trait RealtimeWebsocketConnectionRuntime: Send + Sync {
    fn writer(&self) -> Arc<dyn RealtimeWebsocketWriterRuntime>;

    fn events(&self) -> Arc<dyn RealtimeWebsocketEventsRuntime>;
}

/// Runtime capability for sending realtime WebSocket messages.
///
/// Implementations encode version-specific payloads and own concrete WebSocket send behavior.
pub trait RealtimeWebsocketWriterRuntime: Send + Sync {
    fn send_audio_frame(
        &self,
        frame: RealtimeAudioFrame,
    ) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;

    fn send_conversation_item_create(
        &self,
        text: String,
    ) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;

    fn send_conversation_function_call_output(
        &self,
        call_id: String,
        output_text: String,
    ) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;

    fn send_response_create(&self) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;

    fn send_payload(&self, payload: String) -> ApiRuntimeFuture<'_, Result<(), ApiError>>;
}

/// Runtime capability for receiving typed realtime WebSocket events.
///
/// Implementations own parsing, transcript state, and transport close/error classification.
pub trait RealtimeWebsocketEventsRuntime: Send + Sync {
    fn next_event(&self) -> ApiRuntimeFuture<'_, Result<Option<RealtimeEvent>, ApiError>>;
}

/// Request for executing the compaction endpoint through a configured runtime client.
pub struct CompactInputRuntimeRequest<'a> {
    pub input: &'a CompactionInput<'a>,
    pub extra_headers: HeaderMap,
    pub request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

/// Request for executing the memory summarize endpoint through a configured runtime client.
pub struct MemorySummarizeRuntimeRequest<'a> {
    pub input: &'a MemorySummarizeInput,
    pub extra_headers: HeaderMap,
    pub request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

/// Request for executing a Chat Completions-compatible endpoint through a configured runtime
/// client.
pub struct ChatCompletionsRuntimeRequest {
    pub request: ResponsesApiRequest,
    pub extra_headers: HeaderMap,
    pub path: ChatCompletionsPath,
    pub request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

/// Request for creating a realtime WebRTC call through a configured runtime client.
pub struct RealtimeCallRuntimeRequest {
    pub sdp: String,
    pub session_config: RealtimeSessionConfig,
    pub extra_headers: HeaderMap,
    pub request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

/// Request for streaming a Responses API request through a configured runtime client.
pub struct ResponsesStreamRuntimeRequest {
    pub request: ResponsesApiRequest,
    pub options: ResponsesOptions,
    pub request_telemetry: Option<Arc<dyn RequestTelemetry>>,
    pub sse_telemetry: Option<Arc<dyn SseTelemetry>>,
}

/// Request for executing ARC monitor HTTP checks through a configured runtime client.
pub struct ArcMonitorRuntimeRequest {
    pub url: String,
    pub body: Value,
    pub bearer_token: Option<String>,
    pub auth_headers: HeaderMap,
    pub timeout: Duration,
}

/// Transport-neutral ARC monitor HTTP response.
pub struct ArcMonitorRuntimeResponse {
    pub status: StatusCode,
    pub body_text: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffortConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummaryConfig>,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextFormatType {
    #[default]
    JsonSchema,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
pub struct TextFormat {
    /// Format type used by the OpenAI text controls.
    pub r#type: TextFormatType,
    /// When true, the server is expected to strictly validate responses.
    pub strict: bool,
    /// JSON schema for the desired output.
    pub schema: Value,
    /// Friendly name for the format, used in telemetry/debugging.
    pub name: String,
}

/// Controls the `text` field for the Responses API, combining verbosity and
/// optional JSON schema output formatting.
#[derive(Debug, Serialize, Default, Clone, PartialEq)]
pub struct TextControls {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<OpenAiVerbosity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
}

#[derive(Debug, Serialize, Default, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OpenAiVerbosity {
    Low,
    #[default]
    Medium,
    High,
}

impl From<VerbosityConfig> for OpenAiVerbosity {
    fn from(v: VerbosityConfig) -> Self {
        match v {
            VerbosityConfig::Low => OpenAiVerbosity::Low,
            VerbosityConfig::Medium => OpenAiVerbosity::Medium,
            VerbosityConfig::High => OpenAiVerbosity::High,
        }
    }
}

/// Canonical input payload for the compaction endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CompactionInput<'a> {
    pub model: &'a str,
    pub input: &'a [ResponseItem],
    #[serde(skip_serializing_if = "str::is_empty")]
    pub instructions: &'a str,
    pub tools: Vec<Value>,
    pub parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
}

/// Canonical input payload for the memory summarize endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct MemorySummarizeInput {
    pub model: String,
    #[serde(rename = "traces")]
    pub raw_memories: Vec<RawMemory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMemory {
    pub id: String,
    pub metadata: RawMemoryMetadata,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RawMemoryMetadata {
    pub source_path: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MemorySummarizeOutput {
    #[serde(rename = "trace_summary", alias = "raw_memory")]
    pub raw_memory: String,
    pub memory_summary: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ResponsesApiRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<serde_json::Value>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,
    pub store: bool,
    pub stream: bool,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
    /// Chat Completions-only completion token cap. This is intentionally skipped for
    /// Responses API serialization and consumed by the chat-completions adapter.
    #[serde(skip)]
    pub chat_completions_max_tokens: Option<u64>,
}

impl From<&ResponsesApiRequest> for ResponseCreateWsRequest {
    fn from(request: &ResponsesApiRequest) -> Self {
        Self {
            model: request.model.clone(),
            instructions: request.instructions.clone(),
            previous_response_id: None,
            input: request.input.clone(),
            tools: request.tools.clone(),
            tool_choice: request.tool_choice.clone(),
            parallel_tool_calls: request.parallel_tool_calls,
            reasoning: request.reasoning.clone(),
            store: request.store,
            stream: request.stream,
            include: request.include.clone(),
            service_tier: request.service_tier.clone(),
            prompt_cache_key: request.prompt_cache_key.clone(),
            text: request.text.clone(),
            generate: None,
            client_metadata: request.client_metadata.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ResponseCreateWsRequest {
    pub model: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    pub input: Vec<ResponseItem>,
    pub tools: Vec<Value>,
    pub tool_choice: String,
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,
    pub store: bool,
    pub stream: bool,
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextControls>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct ResponseProcessedWsRequest {
    pub response_id: String,
}

pub fn response_create_client_metadata(
    client_metadata: Option<HashMap<String, String>>,
    trace: Option<&W3cTraceContext>,
) -> Option<HashMap<String, String>> {
    let mut client_metadata = client_metadata.unwrap_or_default();

    if let Some(traceparent) = trace.and_then(|trace| trace.traceparent.as_deref()) {
        client_metadata.insert(
            WS_REQUEST_HEADER_TRACEPARENT_CLIENT_METADATA_KEY.to_string(),
            traceparent.to_string(),
        );
    }
    if let Some(tracestate) = trace.and_then(|trace| trace.tracestate.as_deref()) {
        client_metadata.insert(
            WS_REQUEST_HEADER_TRACESTATE_CLIENT_METADATA_KEY.to_string(),
            tracestate.to_string(),
        );
    }

    (!client_metadata.is_empty()).then_some(client_metadata)
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum ResponsesWsRequest {
    #[serde(rename = "response.create")]
    ResponseCreate(ResponseCreateWsRequest),
    #[serde(rename = "response.processed")]
    ResponseProcessed(ResponseProcessedWsRequest),
}

pub fn create_text_param_for_request(
    verbosity: Option<VerbosityConfig>,
    output_schema: &Option<Value>,
    output_schema_strict: bool,
) -> Option<TextControls> {
    if verbosity.is_none() && output_schema.is_none() {
        return None;
    }

    Some(TextControls {
        verbosity: verbosity.map(std::convert::Into::into),
        format: output_schema.as_ref().map(|schema| TextFormat {
            r#type: TextFormatType::JsonSchema,
            strict: output_schema_strict,
            schema: schema.clone(),
            name: "codex_output_schema".to_string(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::config_types::Verbosity;
    use pretty_assertions::assert_eq;

    #[test]
    fn create_text_param_returns_none_without_controls() {
        assert_eq!(
            create_text_param_for_request(
                /*verbosity*/ None, &None, /*output_schema_strict*/ false
            ),
            None
        );
    }

    #[test]
    fn create_text_param_sets_verbosity() {
        assert_eq!(
            create_text_param_for_request(
                Some(Verbosity::Low),
                &None,
                /*output_schema_strict*/ false
            ),
            Some(TextControls {
                verbosity: Some(OpenAiVerbosity::Low),
                format: None,
            })
        );
    }

    #[test]
    fn create_text_param_sets_json_schema_format() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            }
        });

        assert_eq!(
            create_text_param_for_request(
                /*verbosity*/ None,
                &Some(schema.clone()),
                /*output_schema_strict*/ true
            ),
            Some(TextControls {
                verbosity: None,
                format: Some(TextFormat {
                    r#type: TextFormatType::JsonSchema,
                    strict: true,
                    schema,
                    name: "codex_output_schema".to_string(),
                }),
            })
        );
    }
}
