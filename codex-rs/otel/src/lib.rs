pub(crate) mod config;
mod events;
pub(crate) mod metrics;
pub(crate) mod provider;
mod trace_context;

mod otlp;
mod targets;

pub use crate::config::OtelExporter;
pub use crate::config::OtelHttpProtocol;
pub use crate::config::OtelSettings;
pub use crate::config::OtelTlsConfig;
pub use crate::config::StatsigMetricsSettings;
pub use crate::config::validate_span_attributes;
pub use crate::events::session_telemetry::SessionTelemetry;
pub use crate::events::session_telemetry::SessionTelemetryMetadata;
use crate::metrics::Result as MetricsResult;
pub use crate::metrics::timer::Timer;
pub use crate::metrics::*;
pub use crate::provider::OtelProvider;
pub use codex_auth_types::AuthEnvTelemetryMetadata;
pub use codex_auth_types::TelemetryAuthMode;
pub use codex_utils_string::sanitize_metric_tag_value;
pub use metrics_api::RuntimeMetricTotals;
pub use metrics_api::RuntimeMetricsSummary;
pub use metrics_api::ToolDecisionSource;
pub use session_telemetry_api::SessionTelemetryCreateParams;
pub use session_telemetry_api::SharedSessionTelemetry;
pub use session_telemetry_api::SharedSessionTelemetryFactory;
pub use trace_context::context_from_w3c_trace_context;
pub use trace_context::current_span_trace_id;
pub use trace_context::current_span_w3c_trace_context;
pub use trace_context::set_parent_from_context;
pub use trace_context::set_parent_from_w3c_trace_context;
pub use trace_context::span_w3c_trace_context;
pub use trace_context::traceparent_context_from_env;
pub use trace_context::validate_tracestate_entries;
pub use trace_context::validate_tracestate_member;

impl metrics_api::MetricsSink for SessionTelemetry {
    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        SessionTelemetry::counter(self, name, inc, tags);
    }

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        SessionTelemetry::histogram(self, name, value, tags);
    }

    fn record_duration(&self, name: &str, duration: std::time::Duration, tags: &[(&str, &str)]) {
        SessionTelemetry::record_duration(self, name, duration, tags);
    }
}

pub struct OtelSessionTelemetryFactory;

impl session_telemetry_api::SessionTelemetryFactory for OtelSessionTelemetryFactory {
    fn create(
        &self,
        params: session_telemetry_api::SessionTelemetryCreateParams,
    ) -> session_telemetry_api::SharedSessionTelemetry {
        let mut telemetry = SessionTelemetry::new(
            params.conversation_id,
            params.model.as_str(),
            params.slug.as_str(),
            params.account_id,
            params.account_email,
            params.auth_mode,
            params.originator,
            params.log_user_prompts,
            params.terminal_type,
            params.session_source,
        )
        .with_auth_env(params.auth_env);
        if let Some(service_name) = params.metrics_service_name.as_deref() {
            telemetry = telemetry.with_metrics_service_name(service_name);
        }
        std::sync::Arc::new(telemetry)
    }
}

impl session_telemetry_api::SessionTelemetry for SessionTelemetry {
    fn with_model(&self, model: &str, slug: &str) -> session_telemetry_api::SharedSessionTelemetry {
        std::sync::Arc::new(SessionTelemetry::with_model(self.clone(), model, slug))
    }

    fn record_startup_phase(
        &self,
        phase: &'static str,
        duration: std::time::Duration,
        status: Option<&'static str>,
    ) {
        SessionTelemetry::record_startup_phase(self, phase, duration, status);
    }

    fn record_turn_ttft(&self, duration: std::time::Duration) {
        SessionTelemetry::record_turn_ttft(self, duration);
    }

    fn start_timer(
        &self,
        name: &str,
        tags: &[(&str, &str)],
    ) -> Option<session_telemetry_api::SessionTelemetryTimer> {
        SessionTelemetry::start_timer(self, name, tags)
            .ok()
            .map(|timer| Box::new(timer) as session_telemetry_api::SessionTelemetryTimer)
    }

    fn runtime_metrics_summary(&self) -> Option<RuntimeMetricsSummary> {
        SessionTelemetry::runtime_metrics_summary(self)
    }

    fn record_responses(
        &self,
        handle_responses_span: &tracing::Span,
        event: &session_telemetry_api::ResponseEvent,
    ) {
        SessionTelemetry::record_responses(self, handle_responses_span, event);
    }

    fn conversation_starts(
        &self,
        provider_name: &str,
        reasoning_effort: Option<protocol::openai_models::ReasoningEffort>,
        reasoning_summary: protocol::config_types::ReasoningSummary,
        context_window: Option<i64>,
        auto_compact_token_limit: Option<i64>,
        approval_policy: protocol::protocol::AskForApproval,
        sandbox_policy: protocol::protocol::SandboxPolicy,
        mcp_servers: &[&str],
        active_profile: Option<&str>,
    ) {
        SessionTelemetry::conversation_starts(
            self,
            provider_name,
            reasoning_effort,
            reasoning_summary,
            context_window,
            auto_compact_token_limit,
            approval_policy,
            sandbox_policy,
            mcp_servers.to_vec(),
            active_profile.map(str::to_string),
        );
    }

    fn record_api_request(
        &self,
        attempt: u64,
        status: Option<u16>,
        error: Option<&str>,
        duration: std::time::Duration,
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
    ) {
        SessionTelemetry::record_api_request(
            self,
            attempt,
            status,
            error,
            duration,
            auth_header_attached,
            auth_header_name,
            retry_after_unauthorized,
            recovery_mode,
            recovery_phase,
            endpoint,
            request_id,
            cf_ray,
            auth_error,
            auth_error_code,
        );
    }

    fn record_websocket_connect(
        &self,
        duration: std::time::Duration,
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
    ) {
        SessionTelemetry::record_websocket_connect(
            self,
            duration,
            status,
            error,
            auth_header_attached,
            auth_header_name,
            retry_after_unauthorized,
            recovery_mode,
            recovery_phase,
            endpoint,
            connection_reused,
            request_id,
            cf_ray,
            auth_error,
            auth_error_code,
        );
    }

    fn record_websocket_request(
        &self,
        duration: std::time::Duration,
        error: Option<&str>,
        connection_reused: bool,
    ) {
        SessionTelemetry::record_websocket_request(self, duration, error, connection_reused);
    }

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
    ) {
        SessionTelemetry::record_auth_recovery(
            self,
            mode,
            step,
            outcome,
            request_id,
            cf_ray,
            auth_error,
            auth_error_code,
            recovery_reason,
            auth_state_changed,
        );
    }

    fn record_websocket_event(
        &self,
        event: Option<&session_telemetry_api::WebsocketEventTelemetry>,
        duration: std::time::Duration,
    ) {
        SessionTelemetry::record_websocket_event(self, event, duration);
    }

    fn log_sse_event(
        &self,
        event: Option<&session_telemetry_api::SseEventTelemetry>,
        duration: std::time::Duration,
    ) {
        SessionTelemetry::log_sse_event(self, event, duration);
    }

    fn see_event_completed_failed(&self, error: &dyn std::fmt::Display) {
        SessionTelemetry::see_event_completed_failed(self, &error.to_string());
    }

    fn sse_event_completed(
        &self,
        input_token_count: i64,
        output_token_count: i64,
        cached_token_count: Option<i64>,
        reasoning_token_count: Option<i64>,
        tool_token_count: i64,
    ) {
        SessionTelemetry::sse_event_completed(
            self,
            input_token_count,
            output_token_count,
            cached_token_count,
            reasoning_token_count,
            tool_token_count,
        );
    }

    fn user_prompt(&self, items: &[protocol::user_input::UserInput]) {
        SessionTelemetry::user_prompt(self, items);
    }

    fn tool_decision(
        &self,
        tool_name: &str,
        call_id: &str,
        decision: &protocol::protocol::ReviewDecision,
        source: metrics_api::ToolDecisionSource,
    ) {
        SessionTelemetry::tool_decision(self, tool_name, call_id, decision, source);
    }

    fn tool_result_with_tags(
        &self,
        tool_name: &str,
        call_id: &str,
        arguments: &str,
        duration: std::time::Duration,
        success: bool,
        output: &str,
        extra_tags: &[(&str, &str)],
        extra_trace_fields: &[(&str, &str)],
    ) {
        SessionTelemetry::tool_result_with_tags(
            self,
            tool_name,
            call_id,
            arguments,
            duration,
            success,
            output,
            extra_tags,
            extra_trace_fields,
        );
    }
}

impl session_telemetry_api::SessionTelemetryTimerHandle for Timer {
    fn record(&self, additional_tags: &[(&str, &str)]) {
        let _ = Timer::record(self, additional_tags);
    }
}

/// Start a metrics timer using the globally installed metrics client.
pub fn start_global_timer(name: &str, tags: &[(&str, &str)]) -> MetricsResult<Timer> {
    let Some(metrics) = crate::metrics::global() else {
        return Err(MetricsError::ExporterDisabled);
    };
    metrics.start_timer(name, tags)
}

/// Returns the resolved Statsig metrics settings for the globally installed
/// OTEL metrics client, if the active metrics exporter is Statsig.
pub fn global_statsig_metrics_settings() -> Option<StatsigMetricsSettings> {
    crate::metrics::global_statsig_settings()
}
