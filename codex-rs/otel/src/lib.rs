pub(crate) mod config;
mod events;
pub(crate) mod metrics;
pub(crate) mod provider;

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
pub use crate::metrics::runtime_metrics::RuntimeMetricTotals;
pub use crate::metrics::runtime_metrics::RuntimeMetricsSummary;
pub use crate::metrics::timer::Timer;
pub use crate::metrics::*;
pub use crate::provider::OtelProvider;
pub use codex_auth_types::AuthEnvTelemetryMetadata;
pub use codex_auth_types::TelemetryAuthMode;
pub use codex_metrics_api::ToolDecisionSource;
pub use codex_trace_context::context_from_w3c_trace_context;
pub use codex_trace_context::current_span_trace_id;
pub use codex_trace_context::current_span_w3c_trace_context;
pub use codex_trace_context::set_parent_from_context;
pub use codex_trace_context::set_parent_from_w3c_trace_context;
pub use codex_trace_context::span_w3c_trace_context;
pub use codex_trace_context::traceparent_context_from_env;
pub use codex_trace_context::validate_tracestate_entries;
pub use codex_trace_context::validate_tracestate_member;
pub use codex_utils_string::sanitize_metric_tag_value;

impl codex_metrics_api::MetricsSink for SessionTelemetry {
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
