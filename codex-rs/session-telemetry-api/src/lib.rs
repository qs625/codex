use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_auth_types::AuthEnvTelemetryMetadata;
use codex_auth_types::TelemetryAuthMode;
use metrics_api::MetricsSink;
use metrics_api::RuntimeMetricsSummary;
use metrics_api::ToolDecisionSource;
use protocol::ThreadId;
use protocol::config_types::ReasoningSummary;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AskForApproval;
use protocol::protocol::ModelVerification;
use protocol::protocol::ReviewDecision;
use protocol::protocol::RateLimitSnapshot;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionSource;
use protocol::protocol::TokenUsage;
use protocol::user_input::UserInput;
use serde_json::Value;
use tracing::Span;

/// Shared session telemetry handle used by runtime crates that should not depend on the concrete
/// OTEL implementation.
pub type SharedSessionTelemetry = Arc<dyn SessionTelemetry>;

/// Shared factory that creates session-scoped telemetry handles.
pub type SharedSessionTelemetryFactory = Arc<dyn SessionTelemetryFactory>;

/// Drop guard returned by session telemetry timers.
pub type SessionTelemetryTimer = Box<dyn SessionTelemetryTimerHandle>;

/// Transport-neutral summary of a single SSE poll used by telemetry sinks.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEventTelemetry {
    pub kind: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
}

impl SseEventTelemetry {
    pub fn succeeded(kind: impl Into<String>) -> Self {
        Self {
            kind: Some(kind.into()),
            success: true,
            error_message: None,
        }
    }

    pub fn failed(kind: Option<String>, error_message: impl Into<String>) -> Self {
        Self {
            kind,
            success: false,
            error_message: Some(error_message.into()),
        }
    }
}

/// Transport-neutral summary of a single Responses WebSocket poll used by telemetry sinks.
#[derive(Debug, Clone, PartialEq)]
pub struct WebsocketEventTelemetry {
    pub kind: Option<String>,
    pub success: bool,
    pub error_message: Option<String>,
    /// Parsed JSON payload for text events. Telemetry uses this to extract
    /// Responses API timing metrics without depending on tungstenite.
    pub payload: Option<Value>,
}

impl WebsocketEventTelemetry {
    pub fn succeeded(kind: Option<String>, payload: Option<Value>) -> Self {
        Self {
            kind,
            success: true,
            error_message: None,
            payload,
        }
    }

    pub fn failed(
        kind: Option<String>,
        error_message: impl Into<String>,
        payload: Option<Value>,
    ) -> Self {
        Self {
            kind,
            success: false,
            error_message: Some(error_message.into()),
            payload,
        }
    }
}

#[derive(Debug)]
pub enum ResponseEvent {
    Created,
    OutputItemDone(ResponseItem),
    OutputItemAdded(ResponseItem),
    /// Emitted when the server includes `OpenAI-Model` on the stream response.
    /// This can differ from the requested model when backend safety routing applies.
    ServerModel(String),
    /// Emitted when the server recommends additional account verification.
    ModelVerifications(Vec<ModelVerification>),
    /// Emitted when `X-Reasoning-Included: true` is present on the response,
    /// meaning the server already accounted for past reasoning tokens and the
    /// client should not re-estimate them.
    ServerReasoningIncluded(bool),
    Completed {
        response_id: String,
        token_usage: Option<TokenUsage>,
        /// Did the model affirmatively end its turn? Some providers do not set this,
        /// so we rely on fallback logic when this is `None`.
        end_turn: Option<bool>,
    },
    OutputTextDelta(String),
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
    ReasoningSummaryPartAdded {
        summary_index: i64,
    },
    RateLimits(RateLimitSnapshot),
    ModelsEtag(String),
}

/// Input used by composition roots or session runtimes to create a session telemetry handle.
#[derive(Debug, Clone)]
pub struct SessionTelemetryCreateParams {
    pub conversation_id: ThreadId,
    pub model: String,
    pub slug: String,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub auth_mode: Option<TelemetryAuthMode>,
    pub auth_env: AuthEnvTelemetryMetadata,
    pub originator: String,
    pub log_user_prompts: bool,
    pub terminal_type: String,
    pub session_source: SessionSource,
    pub metrics_service_name: Option<String>,
}

/// Creates session-scoped telemetry handles.
///
/// Implementations own exporter/runtime-specific setup. Consumers should request a handle through
/// this trait instead of depending on the concrete OTEL crate.
pub trait SessionTelemetryFactory: Send + Sync {
    fn create(&self, params: SessionTelemetryCreateParams) -> SharedSessionTelemetry;
}

/// Session-scoped telemetry facade.
///
/// Implementations should be best-effort: telemetry failures must not affect normal runtime
/// behavior. The trait includes structured session events in addition to the metrics sink
/// operations inherited from [`MetricsSink`].
#[allow(clippy::too_many_arguments)]
pub trait SessionTelemetry: MetricsSink + std::fmt::Debug + Send + Sync {
    fn with_model(&self, model: &str, slug: &str) -> SharedSessionTelemetry;

    fn record_startup_phase(
        &self,
        phase: &'static str,
        duration: Duration,
        status: Option<&'static str>,
    );

    fn record_turn_ttft(&self, duration: Duration);

    fn start_timer(&self, name: &str, tags: &[(&str, &str)]) -> Option<SessionTelemetryTimer>;

    fn runtime_metrics_summary(&self) -> Option<RuntimeMetricsSummary>;

    fn record_responses(&self, handle_responses_span: &Span, event: &ResponseEvent);

    fn conversation_starts(
        &self,
        provider_name: &str,
        reasoning_effort: Option<ReasoningEffort>,
        reasoning_summary: ReasoningSummary,
        context_window: Option<i64>,
        auto_compact_token_limit: Option<i64>,
        approval_policy: AskForApproval,
        sandbox_policy: SandboxPolicy,
        mcp_servers: &[&str],
        active_profile: Option<&str>,
    );

    fn record_api_request(
        &self,
        attempt: u64,
        status: Option<u16>,
        error: Option<&str>,
        duration: Duration,
        auth_header_attached: bool,
        auth_header_name: Option<&str>,
        retry_after_unauthorized: bool,
        recovery_mode: Option<&str>,
        recovery_phase: Option<&str>,
        endpoint: &str,
        request_id: Option<&str>,
        cf_ray: Option<&str>,
        auth_error: Option<&str>,
        auth_error_code: Option<&str>,
    );

    fn record_websocket_connect(
        &self,
        duration: Duration,
        status: Option<u16>,
        error: Option<&str>,
        auth_header_attached: bool,
        auth_header_name: Option<&str>,
        retry_after_unauthorized: bool,
        recovery_mode: Option<&str>,
        recovery_phase: Option<&str>,
        endpoint: &str,
        connection_reused: bool,
        request_id: Option<&str>,
        cf_ray: Option<&str>,
        auth_error: Option<&str>,
        auth_error_code: Option<&str>,
    );

    fn record_websocket_request(
        &self,
        duration: Duration,
        error: Option<&str>,
        connection_reused: bool,
    );

    fn record_auth_recovery(
        &self,
        mode: &str,
        step: &str,
        outcome: &str,
        request_id: Option<&str>,
        cf_ray: Option<&str>,
        auth_error: Option<&str>,
        auth_error_code: Option<&str>,
        recovery_reason: Option<&str>,
        auth_state_changed: Option<bool>,
    );

    fn record_websocket_event(&self, event: Option<&WebsocketEventTelemetry>, duration: Duration);

    fn log_sse_event(&self, event: Option<&SseEventTelemetry>, duration: Duration);

    fn see_event_completed_failed(&self, error: &dyn std::fmt::Display);

    fn sse_event_completed(
        &self,
        input_token_count: i64,
        output_token_count: i64,
        cached_token_count: Option<i64>,
        reasoning_token_count: Option<i64>,
        tool_token_count: i64,
    );

    fn user_prompt(&self, items: &[UserInput]);

    fn tool_decision(
        &self,
        tool_name: &str,
        call_id: &str,
        decision: &ReviewDecision,
        source: ToolDecisionSource,
    );

    fn tool_result_with_tags(
        &self,
        tool_name: &str,
        call_id: &str,
        arguments: &str,
        duration: Duration,
        success: bool,
        output: &str,
        extra_tags: &[(&str, &str)],
        extra_trace_fields: &[(&str, &str)],
    );
}

/// Timer handle used by [`SessionTelemetry::start_timer`].
pub trait SessionTelemetryTimerHandle: Send + Sync {
    fn record(&self, additional_tags: &[(&str, &str)]);
}

/// Lightweight disabled telemetry factory for tests and sample paths that do not need OTEL.
pub struct DisabledSessionTelemetryFactory;

impl SessionTelemetryFactory for DisabledSessionTelemetryFactory {
    fn create(&self, _params: SessionTelemetryCreateParams) -> SharedSessionTelemetry {
        Arc::new(DisabledSessionTelemetry)
    }
}

#[derive(Debug)]
struct DisabledSessionTelemetry;

impl MetricsSink for DisabledSessionTelemetry {
    fn counter(&self, _metric: &str, _inc: i64, _tags: &[(&str, &str)]) {}

    fn histogram(&self, _metric: &str, _value: i64, _tags: &[(&str, &str)]) {}

    fn record_duration(&self, _metric: &str, _duration: Duration, _tags: &[(&str, &str)]) {}
}

impl SessionTelemetry for DisabledSessionTelemetry {
    fn with_model(&self, _model: &str, _slug: &str) -> SharedSessionTelemetry {
        Arc::new(Self)
    }

    fn record_startup_phase(
        &self,
        _phase: &'static str,
        _duration: Duration,
        _status: Option<&'static str>,
    ) {
    }

    fn record_turn_ttft(&self, _duration: Duration) {}

    fn start_timer(&self, _name: &str, _tags: &[(&str, &str)]) -> Option<SessionTelemetryTimer> {
        None
    }

    fn runtime_metrics_summary(&self) -> Option<RuntimeMetricsSummary> {
        None
    }

    fn record_responses(&self, _handle_responses_span: &Span, _event: &ResponseEvent) {}

    fn conversation_starts(
        &self,
        _provider_name: &str,
        _reasoning_effort: Option<ReasoningEffort>,
        _reasoning_summary: ReasoningSummary,
        _context_window: Option<i64>,
        _auto_compact_token_limit: Option<i64>,
        _approval_policy: AskForApproval,
        _sandbox_policy: SandboxPolicy,
        _mcp_servers: &[&str],
        _active_profile: Option<&str>,
    ) {
    }

    fn record_api_request(
        &self,
        _attempt: u64,
        _status: Option<u16>,
        _error: Option<&str>,
        _duration: Duration,
        _auth_header_attached: bool,
        _auth_header_name: Option<&str>,
        _retry_after_unauthorized: bool,
        _recovery_mode: Option<&str>,
        _recovery_phase: Option<&str>,
        _endpoint: &str,
        _request_id: Option<&str>,
        _cf_ray: Option<&str>,
        _auth_error: Option<&str>,
        _auth_error_code: Option<&str>,
    ) {
    }

    fn record_websocket_connect(
        &self,
        _duration: Duration,
        _status: Option<u16>,
        _error: Option<&str>,
        _auth_header_attached: bool,
        _auth_header_name: Option<&str>,
        _retry_after_unauthorized: bool,
        _recovery_mode: Option<&str>,
        _recovery_phase: Option<&str>,
        _endpoint: &str,
        _connection_reused: bool,
        _request_id: Option<&str>,
        _cf_ray: Option<&str>,
        _auth_error: Option<&str>,
        _auth_error_code: Option<&str>,
    ) {
    }

    fn record_websocket_request(
        &self,
        _duration: Duration,
        _error: Option<&str>,
        _connection_reused: bool,
    ) {
    }

    fn record_auth_recovery(
        &self,
        _mode: &str,
        _step: &str,
        _outcome: &str,
        _request_id: Option<&str>,
        _cf_ray: Option<&str>,
        _auth_error: Option<&str>,
        _auth_error_code: Option<&str>,
        _recovery_reason: Option<&str>,
        _auth_state_changed: Option<bool>,
    ) {
    }

    fn record_websocket_event(
        &self,
        _event: Option<&WebsocketEventTelemetry>,
        _duration: Duration,
    ) {
    }

    fn log_sse_event(&self, _event: Option<&SseEventTelemetry>, _duration: Duration) {}

    fn see_event_completed_failed(&self, _error: &dyn std::fmt::Display) {}

    fn sse_event_completed(
        &self,
        _input_token_count: i64,
        _output_token_count: i64,
        _cached_token_count: Option<i64>,
        _reasoning_token_count: Option<i64>,
        _tool_token_count: i64,
    ) {
    }

    fn user_prompt(&self, _items: &[UserInput]) {}

    fn tool_decision(
        &self,
        _tool_name: &str,
        _call_id: &str,
        _decision: &ReviewDecision,
        _source: ToolDecisionSource,
    ) {
    }

    fn tool_result_with_tags(
        &self,
        _tool_name: &str,
        _call_id: &str,
        _arguments: &str,
        _duration: Duration,
        _success: bool,
        _output: &str,
        _extra_tags: &[(&str, &str)],
        _extra_trace_fields: &[(&str, &str)],
    ) {
    }
}

pub async fn log_tool_result_with_tags<F, Fut, E>(
    telemetry: &dyn SessionTelemetry,
    tool_name: &str,
    call_id: &str,
    arguments: &str,
    extra_tags: &[(&str, &str)],
    extra_trace_fields: &[(&str, &str)],
    f: F,
) -> Result<(String, bool), E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(String, bool), E>>,
    E: std::fmt::Display,
{
    let start = Instant::now();
    let result = f().await;
    let duration = start.elapsed();

    let (output, success) = match &result {
        Ok((preview, success)) => (Cow::Borrowed(preview.as_str()), *success),
        Err(error) => (Cow::Owned(error.to_string()), false),
    };

    telemetry.tool_result_with_tags(
        tool_name,
        call_id,
        arguments,
        duration,
        success,
        output.as_ref(),
        extra_tags,
        extra_trace_fields,
    );

    result
}
