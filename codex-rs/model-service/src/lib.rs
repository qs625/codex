pub mod amazon_bedrock;
mod adapters;
mod api_runtime_factory;
mod cache;
mod client;
mod chat_completions_client;
mod collaboration_mode_presets;
mod compact_client;
mod deps;
mod endpoint_session;
pub mod manager;
mod memories_client;
pub mod model_info;
mod models_client;
mod provider_auth_support;
mod provider_models_endpoint;
mod provider_runtime;
mod realtime_call_client;
mod realtime_websocket;
mod request_headers;
mod responses_client;
mod responses_requests;
mod responses_sse;
mod responses_websocket_client;
mod service_api;
pub mod test_support;
mod transport_telemetry;

pub use api_runtime_factory::DefaultApiRuntimeFactory;
pub use client::AttestationContext;
pub use client::AttestationProvider;
pub use client::CompactConversationRequestSettings;
pub use client::GenerateAttestationFuture;
pub use client::ModelClient;
pub use client::ModelClientSession;
pub use client::Prompt;
pub use client::PromptBuildParams;
pub use client::REVIEW_EXIT_INTERRUPTED_TMPL;
pub use client::REVIEW_EXIT_SUCCESS_TMPL;
pub use client::REVIEW_PROMPT;
pub use client::ResponseEvent as LegacyResponseEvent;
pub use client::ResponseStream;
pub use client::X_OAI_ATTESTATION_HEADER;
pub use client::build_prompt;
pub use chat_completions_client::ChatCompletionsClient;
pub use collaboration_mode_presets::builtin_collaboration_mode_presets;
pub use compact_client::CompactClient;
pub use memories_client::MemoriesClient;
pub use model_service_api::BearerAuthProvider;
pub use model_service_api::BearerAuthProvider as CoreAuthProvider;
pub use model_service_api::DEFAULT_APPROVAL_REVIEW_PREFERRED_MODEL;
pub use model_service_api::ModelProvider;
pub use model_service_api::ModelProviderFactory;
pub use model_service_api::ModelProviderFuture;
pub use model_service_api::ProviderAccountError;
pub use model_service_api::ProviderAccountResult;
pub use model_service_api::ProviderAccountState;
pub use model_service_api::ProviderCapabilities;
pub use model_service_api::SharedModelProvider;
pub use model_service_api::SharedModelProviderAuthManager;
pub use model_service_api::SharedModelProviderFactory;
pub use model_service_api::resolve_provider_auth;
pub use model_service_api::unauthenticated_auth_provider;
pub use models_client::ModelsClient;
pub use protocol::account::ProviderAccount;
pub use provider_auth_support::auth_provider_from_auth;
pub use provider_runtime::DefaultModelProviderFactory;
pub use provider_runtime::create_model_provider;
pub use provider_runtime::model_provider_info_to_api_provider;
pub use realtime_call_client::RealtimeCallClient;
pub use realtime_websocket::RealtimeWebsocketClient;
pub use realtime_websocket::RealtimeWebsocketConnection;
pub use realtime_websocket::RealtimeWebsocketEvents;
pub use realtime_websocket::RealtimeWebsocketWriter;
pub use realtime_websocket::session_update_session_json;
pub use responses_client::ResponsesClient;
pub use responses_websocket_client::ResponsesWebsocketClient;
pub use responses_websocket_client::ResponsesWebsocketConnection;
pub use deps::ModelService;
pub use deps::ModelServiceDeps;
pub use deps::ModelServiceRuntimeDeps;
pub use deps::default_http_client;

use std::fmt;
use std::sync::Arc;

use codex_realtime::build_realtime_api_provider;
use codex_realtime::default_realtime_voice;
use codex_realtime::realtime_api_key;
use codex_realtime::realtime_request_headers;
use futures::StreamExt;
use model_service_api::ModelClientApi;
use model_service_api::ModelClientDescriptor;
use model_service_api::ModelMemorySummary;
use model_service_api::ModelMetadataOverride;
use model_service_api::ModelProviderInfo;
use model_service_api::ModelRequestError;
use model_service_api::ModelResponseEvent;
use model_service_api::ModelResponseStream;
use model_service_api::ModelTurnClientApi;
use model_service_api::ModelsManagerConfig;
use model_service_api::OwnedModelTurnClientApi;
use model_service_api::PrepareRealtimeTransportRequest;
use model_service_api::PreparedRealtimeTransport;
use model_service_api::RawMemory;
use model_service_api::RawMemoryMetadata;
use model_service_api::RealtimeCallHandle;
use model_service_api::RealtimeEventParser;
use model_service_api::RealtimeModelRequest;
use model_service_api::RealtimeSessionConfig;
use model_service_api::RealtimeSessionMode;
use model_service_api::RealtimeWebrtcCallHandle;
use model_service_api::RealtimeWebrtcCallRequest;
use model_service_api::ResponsesModelRequest;
use model_service_api::SharedApiRuntimeFactory;
use model_service_api::SharedModelsManager;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelPreset;
use protocol::openai_models::ModelsResponse;
use protocol::protocol::RealtimeOutputModality;
use rollout_trace_api::CompactionTraceContext;
use rollout_trace_api::InferenceTraceContext;
use session_telemetry_api::DisabledSessionTelemetryFactory;
use session_telemetry_api::SessionTelemetryCreateParams;
use session_telemetry_api::SessionTelemetryFactory;
use session_telemetry_api::SharedSessionTelemetry;

/// Load the bundled model catalog shipped with `model-service`.
pub fn bundled_models_response() -> std::result::Result<ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Convert the client version string to a whole version string.
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}
