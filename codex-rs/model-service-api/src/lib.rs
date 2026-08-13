mod request_auth;
mod provider_auth;
mod provider_config;
mod provider_info;
mod provider_transport;
mod provider_runtime;
mod transport_runtime;
mod wire;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use transport_client_types::Request;
use transport_client_types::Response;
use codex_auth_types::RequestAuthSnapshot;
use futures::Stream;
use http::HeaderMap;
use protocol::config_types::CollaborationModeMask;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::ServiceTier;
use protocol::config_types::Verbosity;
use protocol::config_types::Personality;
use protocol::models::BaseInstructions;
use protocol::models::ResponseItem;
use protocol::error::CodexErr;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelsResponse;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::ModelVerification;
use protocol::protocol::RateLimitSnapshot;
use protocol::protocol::RealtimeOutputModality;
use protocol::protocol::SessionSource;
use protocol::protocol::RealtimeVoice;
use protocol::protocol::TokenUsage;
use protocol::SessionId;
use protocol::ThreadId;
use rollout_trace_api::InferenceTraceContext;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use session_telemetry_api::SharedSessionTelemetry;
use tool_types::ToolSpec;

pub use provider_transport::*;
pub use request_auth::BearerAuthProvider;
pub use request_auth::auth_provider_from_auth_snapshot;
pub use request_auth::unauthenticated_auth_provider;
pub use transport_runtime::*;
pub use wire::*;
pub use provider_auth::ModelProviderAuthFuture;
pub use provider_auth::ModelProviderAuthManager;
pub use provider_auth::ModelProviderAuthRecoveryError;
pub use provider_auth::ModelProviderUnauthorizedRecovery;
pub use provider_auth::ModelProviderUnauthorizedRecoveryStepResult;
pub use provider_auth::SharedModelProviderAuthManager;
pub use provider_config::model_provider_info_to_api_provider;
pub use provider_info::AMAZON_BEDROCK_DEFAULT_BASE_URL;
pub use provider_info::AMAZON_BEDROCK_GPT_5_4_MODEL_ID;
pub use provider_info::AMAZON_BEDROCK_PROVIDER_ID;
pub use provider_info::CHATGPT_CODEX_BASE_URL;
pub use provider_info::DEFAULT_LMSTUDIO_PORT;
pub use provider_info::DEFAULT_OLLAMA_PORT;
pub use provider_info::DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS;
pub use provider_info::LEGACY_OLLAMA_CHAT_PROVIDER_ID;
pub use provider_info::LMSTUDIO_OSS_PROVIDER_ID;
pub use provider_info::ModelOptionToml;
pub use provider_info::ModelProviderAwsAuthInfo;
pub use provider_info::ModelProviderInfo;
pub use provider_info::OLLAMA_CHAT_PROVIDER_REMOVED_ERROR;
pub use provider_info::OLLAMA_OSS_PROVIDER_ID;
pub use provider_info::OPENAI_PROVIDER_ID;
pub use provider_info::WireApi;
pub use provider_info::built_in_model_providers;
pub use provider_info::create_oss_provider_with_base_url;
pub use provider_info::deserialize_model_providers;
pub use provider_info::merge_configured_model_providers;
pub use provider_info::validate_model_providers;
pub use provider_info::validate_oss_provider;
pub use provider_runtime::DEFAULT_APPROVAL_REVIEW_PREFERRED_MODEL;
pub use provider_runtime::ModelProvider;
pub use provider_runtime::ModelProviderFactory;
pub use provider_runtime::ModelProviderFuture;
pub use provider_runtime::ProviderAccountError;
pub use provider_runtime::ProviderAccountResult;
pub use provider_runtime::ProviderAccountState;
pub use provider_runtime::ProviderCapabilities;
pub use provider_runtime::SharedModelProvider;
pub use provider_runtime::SharedModelProviderFactory;
pub use provider_runtime::resolve_provider_auth;

/// Legacy notice keys kept for config compatibility with older migration prompts.
pub const HIDE_GPT5_1_MIGRATION_PROMPT_CONFIG: &str = "hide_gpt5_1_migration_prompt";
/// Legacy notice key kept for config compatibility with older migration prompts.
pub const HIDE_GPT_5_1_CODEX_MAX_MIGRATION_PROMPT_CONFIG: &str =
    "hide_gpt-5.1-codex-max_migration_prompt";

/// Shared handle for the global model service.
pub type SharedModelServiceApi = Arc<dyn ModelServiceApi>;

/// Shared handle for a generic HTTP client owned by the model/network domain.
pub type SharedHttpClientApi = Arc<dyn HttpClientApi>;

/// Shared handle for a resolved model client.
pub type SharedModelClientApi = Arc<dyn ModelClientApi>;

/// Owned handle for a turn-scoped model client.
pub type OwnedModelTurnClientApi = Box<dyn ModelTurnClientApi>;

/// Boxed future returned by object-safe model service APIs.
pub type ModelFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed future returned by the object-safe model catalog manager API.
pub type ModelsManagerFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed stream returned by model response APIs.
pub type ModelResponseStream =
    Pin<Box<dyn Stream<Item = Result<ModelResponseEvent, ModelRequestError>> + Send + 'static>>;

/// Strategy for refreshing model catalog data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogRefresh {
    /// Always refresh from the remote source.
    Online,
    /// Only use cached catalog data.
    Offline,
    /// Refresh from the remote source only if the cache is missing.
    #[default]
    OnlineIfUncached,
}

/// High-level policy for picking a model before building a client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelectionPolicy {
    /// Explicit model requested by the caller, if any.
    pub requested_model: Option<String>,
    /// Optional provider hint used to disambiguate multiple deployments.
    pub provider_hint: Option<String>,
    /// Whether the service may fall back to a configured default model.
    pub allow_default_fallback: bool,
    /// How the service should refresh the backing catalog before resolving.
    pub refresh: ModelCatalogRefresh,
}

/// Request for listing models from the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListModelsRequest {
    /// Whether hidden models should be included in the result.
    pub include_hidden: bool,
    /// How the service should refresh the backing catalog before listing.
    pub refresh: ModelCatalogRefresh,
}

#[derive(Debug, Clone, Default)]
pub struct ModelsManagerConfig {
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub tool_output_token_limit: Option<usize>,
    pub base_instructions: Option<String>,
    pub personality_enabled: bool,
    pub model_supports_reasoning_summaries: Option<bool>,
    pub model_catalog: Option<ModelsResponse>,
    pub model_metadata_overrides: Vec<ModelMetadataOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelMetadataOverride {
    pub model: String,
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    pub auto_compact_token_limit: Option<i64>,
}

/// Strategy for refreshing available models inside the model domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshStrategy {
    Online,
    Offline,
    OnlineIfUncached,
}

impl RefreshStrategy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::OnlineIfUncached => "online_if_uncached",
        }
    }
}

impl fmt::Display for RefreshStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ModelCatalogRefresh> for RefreshStrategy {
    fn from(value: ModelCatalogRefresh) -> Self {
        match value {
            ModelCatalogRefresh::Online => Self::Online,
            ModelCatalogRefresh::Offline => Self::Offline,
            ModelCatalogRefresh::OnlineIfUncached => Self::OnlineIfUncached,
        }
    }
}

impl From<RefreshStrategy> for ModelCatalogRefresh {
    fn from(value: RefreshStrategy) -> Self {
        match value {
            RefreshStrategy::Online => ModelCatalogRefresh::Online,
            RefreshStrategy::Offline => ModelCatalogRefresh::Offline,
            RefreshStrategy::OnlineIfUncached => ModelCatalogRefresh::OnlineIfUncached,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TryListModelsError;

impl fmt::Display for TryListModelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("model catalog is currently locked")
    }
}

impl std::error::Error for TryListModelsError {}

/// Request for resolving the default model under the current policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveDefaultModelRequest {
    /// Optional caller-provided model selection policy.
    pub selection: ModelSelectionPolicy,
}

/// Request for constructing a resolved model client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateModelClientRequest {
    /// Model selection policy used before a client is created.
    pub selection: ModelSelectionPolicy,
    /// Stable installation identifier propagated to outbound requests.
    pub installation_id: String,
    /// Optional turn/session-scoped session identifier propagated to requests.
    pub session_id: SessionId,
    /// Thread identifier propagated to requests.
    pub thread_id: ThreadId,
    /// Source of the session that owns this model client.
    pub session_source: SessionSource,
    /// Preferred reasoning effort applied by default when the caller omits one.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Preferred service tier applied by default when the caller omits one.
    pub service_tier: Option<ServiceTier>,
    /// Preferred verbosity applied by default when the caller omits one.
    pub verbosity: Option<Verbosity>,
    /// Chat Completions max_tokens overrides keyed by model slug.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub chat_completions_max_tokens_by_model: HashMap<String, u64>,
    /// Whether request compression should be enabled by default.
    pub enable_request_compression: bool,
    /// Whether timing metrics should be requested by default.
    pub include_timing_metrics: bool,
    /// Optional beta-features header value applied by default.
    pub beta_features_header: Option<String>,
}

/// Request for executing one provider-scoped HTTP call through the model service.
#[derive(Debug, Clone)]
pub struct ProviderHttpRequest {
    /// Provider selection policy used before the HTTP request is executed.
    pub selection: ModelSelectionPolicy,
    /// Optional caller-supplied auth snapshot used instead of the provider-owned auth manager.
    pub auth: Option<RequestAuthSnapshot>,
    /// Transport-neutral outbound HTTP request.
    pub request: Request,
}

/// Generic HTTP client contract exposed by the model/network domain.
///
/// This is intentionally provider-agnostic. Callers should use it for signed
/// upload URLs, bundle downloads, GitHub/archive fetches, and other HTTP
/// requests that should reuse Codex's shared client configuration without
/// going through provider selection or provider-owned auth.
pub trait HttpClientApi: Send + Sync {
    /// Execute one transport-neutral HTTP request with the shared default
    /// client configuration.
    fn execute(&self, request: Request) -> ModelFuture<'_, Result<Response, ModelRequestError>>;
}

/// Business-facing description of a resolved model client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelClientDescriptor {
    /// Concrete model selected for this client.
    pub model: String,
    /// Optional provider selected for this client.
    pub provider: Option<String>,
    /// Effective default reasoning effort bound to this client.
    pub default_reasoning_effort: Option<ReasoningEffort>,
    /// Effective default service tier bound to this client.
    pub default_service_tier: Option<ServiceTier>,
    /// Effective default verbosity bound to this client.
    pub default_verbosity: Option<Verbosity>,
}

/// Request for a Responses-style model invocation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResponsesModelRequest {
    /// Model-visible input items for this request.
    pub input: Vec<ResponseItem>,
    /// Model-visible tools available during this request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// Whether parallel tool calls are allowed for this request.
    pub parallel_tool_calls: bool,
    /// Base instructions applied to the request.
    pub base_instructions: BaseInstructions,
    /// Optional personality override for this request.
    pub personality: Option<Personality>,
    /// Optional structured output schema forwarded to the model.
    pub output_schema: Option<Value>,
    /// Whether the model should strictly validate the output schema.
    pub output_schema_strict: bool,
    /// Optional override for the model slug.
    pub model: Option<String>,
    /// Optional override for reasoning effort.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Reasoning summary mode used for this request.
    pub reasoning_summary: ReasoningSummary,
    /// Optional override for service tier.
    pub service_tier: Option<ServiceTier>,
    /// Optional override for verbosity.
    pub verbosity: Option<Verbosity>,
    /// Optional turn metadata header forwarded to the model backend.
    pub turn_metadata_header: Option<String>,
}

/// Request for opening a realtime session or call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeModelRequest {
    /// SDP offer used when creating a realtime WebRTC call.
    pub sdp: String,
    /// Optional prompt forwarded when creating the realtime session.
    pub prompt: Option<String>,
    /// Optional session identifier forwarded to the realtime backend.
    pub realtime_session_id: Option<String>,
    /// Desired output modality.
    pub output_modality: Option<RealtimeOutputModality>,
    /// Desired voice, when audio output is requested.
    pub voice: Option<RealtimeVoice>,
    /// Optional model override.
    pub model: Option<String>,
}

/// Request for streaming one turn through a turn-scoped model client.
#[derive(Clone)]
pub struct TurnModelRequest {
    /// Business-facing Responses payload for this turn.
    pub request: ResponsesModelRequest,
    /// Fully resolved model metadata for the request model.
    pub model_info: ModelInfo,
    /// Session telemetry handle for the owning thread/session.
    pub session_telemetry: SharedSessionTelemetry,
    /// Optional turn metadata header forwarded to the model backend.
    pub turn_metadata_header: Option<String>,
    /// Rollout/inference trace context for the request.
    pub inference_trace: InferenceTraceContext,
}

impl fmt::Debug for TurnModelRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnModelRequest")
            .field("request", &self.request)
            .field("model_info", &self.model_info.slug)
            .field("session_telemetry", &"<session telemetry>")
            .field("turn_metadata_header", &self.turn_metadata_header)
            .field("inference_trace", &self.inference_trace)
            .finish()
    }
}

/// Request for compacting conversation state through the model service.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactModelRequest {
    /// Conversation history or synthetic input items to compact.
    pub input: Vec<ResponseItem>,
    /// Base instructions applied to the compaction request.
    pub base_instructions: BaseInstructions,
    /// Optional structured output schema forwarded to the model.
    pub output_schema: Option<Value>,
    /// Whether the model should strictly validate the output schema.
    pub output_schema_strict: bool,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional reasoning effort override.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Reasoning summary mode used for the compaction request.
    pub reasoning_summary: ReasoningSummary,
    /// Optional service tier override.
    pub service_tier: Option<ServiceTier>,
}

/// Request for summarizing memories through the model service.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemorySummarizeModelRequest {
    /// Raw memories that should be summarized.
    pub raw_memories: Vec<ModelRawMemory>,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional reasoning effort override.
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// High-level result of creating a realtime call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealtimeCallHandle {
    /// SDP answer returned by the backend, when WebRTC is used.
    pub sdp: Option<String>,
    /// Stable identifier for the created realtime call or session.
    pub call_id: String,
}

/// Prepared transport details for starting a realtime session.
#[derive(Debug, Clone)]
pub struct PreparedRealtimeTransport {
    pub api_provider: Provider,
    pub extra_headers: Option<HeaderMap>,
}

/// Request for preparing realtime transport headers and provider selection.
#[derive(Debug, Clone)]
pub struct PrepareRealtimeTransportRequest {
    pub requested_realtime_session_id: Option<String>,
    pub websocket_base_url: Option<String>,
    pub include_api_key_header: bool,
}

/// Request for creating a WebRTC realtime call while preserving sideband auth.
#[derive(Debug, Clone)]
pub struct RealtimeWebrtcCallRequest {
    pub sdp: String,
    pub api_provider: Provider,
    pub session_config: RealtimeSessionConfig,
    pub extra_headers: HeaderMap,
}

/// Result of a WebRTC realtime call plus sideband join material.
#[derive(Debug, Clone)]
pub struct RealtimeWebrtcCallHandle {
    pub sdp: String,
    pub call_id: String,
    pub sideband_headers: HeaderMap,
}

/// High-level result of a compact operation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCompactResult {
    /// Replacement response items returned by compaction.
    pub items: Vec<ResponseItem>,
}

/// High-level result of a memory summarization operation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelMemorySummary {
    pub raw_memory: String,
    pub memory_summary: String,
}

/// Business-facing raw memory input.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelRawMemory {
    pub id: String,
    pub source_path: String,
    pub items: Vec<Value>,
}

/// Stream event returned from a model response request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelResponseEvent {
    Created,
    ItemDone { item: ResponseItem },
    ItemAdded { item: ResponseItem },
    ServerModel { model: String },
    ModelVerifications { verifications: Vec<ModelVerification> },
    ServerReasoningIncluded { included: bool },
    Completed {
        response_id: String,
        token_usage: Option<TokenUsage>,
        end_turn: Option<bool>,
    },
    OutputTextDelta { delta: String },
    ToolCallInputDelta {
        item_id: String,
        call_id: Option<String>,
        delta: String,
    },
    ReasoningSummaryDelta {
        delta: String,
        summary_index: i64,
    },
    ReasoningContentDelta {
        delta: String,
        content_index: i64,
    },
    ReasoningSummaryPartAdded { summary_index: i64 },
    RateLimits { snapshot: RateLimitSnapshot },
    ModelsEtag { etag: String },
}

/// Error returned while resolving catalog data or constructing a model client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelServiceError {
    pub message: String,
}

impl ModelServiceError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelServiceError {}

/// Error returned while executing a request through a resolved model client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequestError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ModelRequestErrorKind>,
}

/// Machine-readable request error category preserved across model-service adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRequestErrorKind {
    ContextWindowExceeded,
}

impl ModelRequestError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: None,
        }
    }

    pub fn context_window_exceeded() -> Self {
        Self {
            message: "context window exceeded".to_string(),
            kind: Some(ModelRequestErrorKind::ContextWindowExceeded),
        }
    }

    pub fn from_codex_err(err: CodexErr) -> Self {
        if matches!(err, CodexErr::ContextWindowExceeded) {
            Self::context_window_exceeded()
        } else {
            Self::new(err.to_string())
        }
    }

    pub fn into_codex_err(self) -> CodexErr {
        match self.kind {
            Some(ModelRequestErrorKind::ContextWindowExceeded) => CodexErr::ContextWindowExceeded,
            None => CodexErr::Stream(self.message, None),
        }
    }
}

impl fmt::Display for ModelRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ModelRequestError {}

/// Runtime model catalog manager used internally by the model domain.
///
/// Implementations own cache, auth, remote refresh, and provider-specific
/// behavior. Owner crates should depend on this contract through
/// `model-service-api`.
pub trait ModelsManager: fmt::Debug + Send + Sync {
    fn list_models(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, Vec<ModelPreset>>;

    fn raw_model_catalog(
        &self,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'_, ModelsResponse>;

    fn try_list_models(&self) -> Result<Vec<ModelPreset>, TryListModelsError>;

    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask>;

    fn get_default_model<'a>(
        &'a self,
        model: &'a Option<String>,
        refresh_strategy: RefreshStrategy,
    ) -> ModelsManagerFuture<'a, String>;

    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
        config: &'a ModelsManagerConfig,
    ) -> ModelsManagerFuture<'a, ModelInfo>;

    fn refresh_if_new_etag(&self, etag: String) -> ModelsManagerFuture<'_, ()>;
}

/// Shared model manager handle used across runtime services.
pub type SharedModelsManager = Arc<dyn ModelsManager>;

/// Global model service API.
///
/// Implementations own model catalog discovery, provider resolution, default
/// model selection, and creation of resolved model clients. Callers should use
/// this API instead of directly coordinating manager/provider/client layers.
pub trait ModelServiceApi: Send + Sync {
    /// List currently available models.
    fn list_models(
        &self,
        request: ListModelsRequest,
    ) -> ModelFuture<'_, Result<Vec<ModelPreset>, ModelServiceError>>;

    /// Return the active raw model catalog.
    fn raw_model_catalog(
        &self,
        refresh: ModelCatalogRefresh,
    ) -> ModelFuture<'_, Result<ModelsResponse, ModelServiceError>>;

    /// List collaboration mode presets derived from the active model catalog.
    fn list_collaboration_modes(&self) -> Vec<CollaborationModeMask>;

    /// Look up model metadata for one concrete model slug.
    fn get_model_info<'a>(
        &'a self,
        model: &'a str,
    ) -> ModelFuture<'a, Result<ModelInfo, ModelServiceError>>;

    /// Resolve the model that should be treated as the effective default.
    fn resolve_default_model(
        &self,
        request: ResolveDefaultModelRequest,
    ) -> ModelFuture<'_, Result<Option<ModelPreset>, ModelServiceError>>;

    /// Build a client bound to a resolved model/provider/auth configuration.
    fn create_client(
        &self,
        request: CreateModelClientRequest,
    ) -> ModelFuture<'_, Result<SharedModelClientApi, ModelServiceError>>;

    /// Execute one provider-scoped HTTP request after provider resolution and auth selection.
    fn execute_provider_http(
        &self,
        request: ProviderHttpRequest,
    ) -> ModelFuture<'_, Result<Response, ModelRequestError>>;

    /// Return a shared generic HTTP client that is not bound to any concrete
    /// provider/backend selection.
    fn http_client(&self) -> SharedHttpClientApi;

    /// Refresh cached catalog state when the backend exposes a newer ETag.
    fn refresh_if_new_etag(
        &self,
        etag: String,
    ) -> ModelFuture<'_, Result<(), ModelServiceError>>;
}

/// Request API for a resolved model client.
///
/// Implementations encapsulate transport, auth, retry, and endpoint-specific
/// behavior behind business-facing request methods. Callers should not need to
/// know about runtime factories, providers, or low-level transport types.
pub trait ModelClientApi: Send + Sync {
    /// Describe the model/provider defaults captured by this client.
    fn descriptor(&self) -> &ModelClientDescriptor;

    /// Whether websocket transport is still enabled for this session-scoped client.
    fn responses_websocket_enabled(&self) -> bool;

    /// Update the compaction/window generation carried by this session-scoped client.
    fn set_window_generation(&self, window_generation: u64);

    /// Advance the compaction/window generation carried by this session-scoped client.
    fn advance_window_generation(&self);

    /// Create a turn-scoped client that owns websocket and sticky-routing state for one turn.
    fn create_turn_client(&self) -> ModelFuture<'_, Result<OwnedModelTurnClientApi, ModelServiceError>>;

    /// Prepare provider-specific realtime transport details.
    fn prepare_realtime_transport(
        &self,
        request: PrepareRealtimeTransportRequest,
    ) -> ModelFuture<'_, Result<PreparedRealtimeTransport, ModelRequestError>>;

    /// Build a realtime websocket runtime client for the chosen API provider.
    fn realtime_websocket_client(
        &self,
        api_provider: Provider,
    ) -> Box<dyn RealtimeWebsocketClientRuntime>;

    /// Create a WebRTC realtime call and return sideband join material.
    fn create_realtime_call_with_transport(
        &self,
        request: RealtimeWebrtcCallRequest,
    ) -> ModelFuture<'_, Result<RealtimeWebrtcCallHandle, ModelRequestError>>;

    /// Stream a Responses-style model request.
    fn stream_responses(
        &self,
        request: ResponsesModelRequest,
    ) -> ModelFuture<'_, Result<ModelResponseStream, ModelRequestError>>;

    /// Open a realtime session or call.
    fn create_realtime_call(
        &self,
        request: RealtimeModelRequest,
    ) -> ModelFuture<'_, Result<RealtimeCallHandle, ModelRequestError>>;

    /// Compact a business payload through the model service.
    fn compact(
        &self,
        request: CompactModelRequest,
    ) -> ModelFuture<'_, Result<ModelCompactResult, ModelRequestError>>;

    /// Summarize memories through the model service.
    fn summarize_memories(
        &self,
        request: MemorySummarizeModelRequest,
    ) -> ModelFuture<'_, Result<Vec<ModelMemorySummary>, ModelRequestError>>;
}

/// Turn-scoped model client API.
///
/// Implementations own websocket/session reuse, sticky routing, fallback
/// transport switches, and other per-turn request state. Callers should create
/// one instance per turn and discard it when that turn completes.
pub trait ModelTurnClientApi: Send {
    /// Return the provider identifier currently bound to this turn-scoped client.
    fn provider(&self) -> Option<&str>;

    /// Reset websocket-local state for the current turn.
    fn reset_websocket_session(&mut self);

    /// Mark the completed response as processed on the websocket transport.
    fn send_response_processed<'a>(&'a self, response_id: &'a str) -> ModelFuture<'a, ()>;

    /// Opportunistically prewarm the websocket transport for the current turn.
    fn prewarm_websocket(
        &mut self,
        request: TurnModelRequest,
    ) -> ModelFuture<'_, Result<(), ModelRequestError>>;

    /// Stream one turn-scoped request.
    fn stream_responses(
        &mut self,
        request: TurnModelRequest,
    ) -> ModelFuture<'_, Result<ModelResponseStream, ModelRequestError>>;

    /// Permanently switch this session over to fallback HTTP transport.
    fn try_switch_fallback_transport(
        &mut self,
        session_telemetry: SharedSessionTelemetry,
        model_info: ModelInfo,
    ) -> bool;
}
