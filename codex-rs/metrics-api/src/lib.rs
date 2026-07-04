use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use codex_utils_string::sanitize_metric_tag_value;
use serde::Deserialize;
use serde::Serialize;

pub const ORIGINATOR_TAG: &str = "originator";
pub const MULTI_AGENT_NICKNAME_POOL_RESET_METRIC: &str = "codex.multi_agent.nickname_pool_reset";
pub const CURATED_PLUGINS_STARTUP_SYNC_METRIC: &str = "codex.plugins.startup_sync";
pub const CURATED_PLUGINS_STARTUP_SYNC_FINAL_METRIC: &str = "codex.plugins.startup_sync.final";
pub const THREAD_SKILLS_ENABLED_TOTAL_METRIC: &str = "codex.thread.skills.enabled_total";
pub const THREAD_SKILLS_KEPT_TOTAL_METRIC: &str = "codex.thread.skills.kept_total";
pub const THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC: &str =
    "codex.thread.skills.description_truncated_chars";
pub const THREAD_SKILLS_TRUNCATED_METRIC: &str = "codex.thread.skills.truncated";
pub const TOOL_CALL_COUNT_METRIC: &str = "codex.tool.call";
pub const TOOL_CALL_DURATION_METRIC: &str = "codex.tool.call.duration_ms";
pub const TOOL_CALL_UNIFIED_EXEC_METRIC: &str = "codex.tool.unified_exec";
pub const PROCESS_START_METRIC: &str = "codex.process.start";
pub const API_CALL_COUNT_METRIC: &str = "codex.api_request";
pub const API_CALL_DURATION_METRIC: &str = "codex.api_request.duration_ms";
pub const SSE_EVENT_COUNT_METRIC: &str = "codex.sse_event";
pub const SSE_EVENT_DURATION_METRIC: &str = "codex.sse_event.duration_ms";
pub const WEBSOCKET_REQUEST_COUNT_METRIC: &str = "codex.websocket.request";
pub const WEBSOCKET_REQUEST_DURATION_METRIC: &str = "codex.websocket.request.duration_ms";
pub const WEBSOCKET_EVENT_COUNT_METRIC: &str = "codex.websocket.event";
pub const WEBSOCKET_EVENT_DURATION_METRIC: &str = "codex.websocket.event.duration_ms";
pub const RESPONSES_API_OVERHEAD_DURATION_METRIC: &str = "codex.responses_api_overhead.duration_ms";
pub const RESPONSES_API_INFERENCE_TIME_DURATION_METRIC: &str =
    "codex.responses_api_inference_time.duration_ms";
pub const RESPONSES_API_ENGINE_IAPI_TTFT_DURATION_METRIC: &str =
    "codex.responses_api_engine_iapi_ttft.duration_ms";
pub const RESPONSES_API_ENGINE_SERVICE_TTFT_DURATION_METRIC: &str =
    "codex.responses_api_engine_service_ttft.duration_ms";
pub const RESPONSES_API_ENGINE_IAPI_TBT_DURATION_METRIC: &str =
    "codex.responses_api_engine_iapi_tbt.duration_ms";
pub const RESPONSES_API_ENGINE_SERVICE_TBT_DURATION_METRIC: &str =
    "codex.responses_api_engine_service_tbt.duration_ms";
pub const TURN_E2E_DURATION_METRIC: &str = "codex.turn.e2e_duration_ms";
pub const TURN_TTFT_DURATION_METRIC: &str = "codex.turn.ttft.duration_ms";
pub const TURN_TTFM_DURATION_METRIC: &str = "codex.turn.ttfm.duration_ms";
pub const TURN_NETWORK_PROXY_METRIC: &str = "codex.turn.network_proxy";
pub const TURN_MEMORY_METRIC: &str = "codex.turn.memory";
pub const TURN_TOOL_CALL_METRIC: &str = "codex.turn.tool.call";
pub const TURN_TOKEN_USAGE_METRIC: &str = "codex.turn.token_usage";
pub const GOAL_CREATED_METRIC: &str = "codex.goal.created";
pub const GOAL_COMPLETED_METRIC: &str = "codex.goal.completed";
pub const GOAL_BUDGET_LIMITED_METRIC: &str = "codex.goal.budget_limited";
pub const GOAL_TOKEN_COUNT_METRIC: &str = "codex.goal.token_count";
pub const GOAL_DURATION_SECONDS_METRIC: &str = "codex.goal.duration_s";
pub const PROFILE_USAGE_METRIC: &str = "codex.profile.usage";
pub const HOOK_RUN_METRIC: &str = "codex.hooks.run";
pub const HOOK_RUN_DURATION_METRIC: &str = "codex.hooks.run.duration_ms";
/// Duration for coarse startup phases, tagged by low-cardinality phase and status.
pub const STARTUP_PHASE_DURATION_METRIC: &str = "codex.startup.phase.duration_ms";
/// Total runtime of a startup prewarm attempt until it completes, tagged by final status.
pub const STARTUP_PREWARM_DURATION_METRIC: &str = "codex.startup_prewarm.duration_ms";
/// Age of the startup prewarm attempt when the first real turn resolves it, tagged by outcome.
pub const STARTUP_PREWARM_AGE_AT_FIRST_TURN_METRIC: &str =
    "codex.startup_prewarm.age_at_first_turn_ms";
pub const THREAD_STARTED_METRIC: &str = "codex.thread.started";

/// Resolved Statsig metrics settings that a helper process can use to
/// recreate the built-in metrics exporter configuration without receiving
/// generic exporter credentials in-process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsigMetricsSettings {
    pub environment: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecisionSource {
    AutomatedReviewer,
    Config,
    User,
}

impl std::fmt::Display for ToolDecisionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::AutomatedReviewer => "automated_reviewer",
            Self::Config => "config",
            Self::User => "user",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeMetricTotals {
    pub count: u64,
    pub duration_ms: u64,
}

impl RuntimeMetricTotals {
    pub fn is_empty(self) -> bool {
        self.count == 0 && self.duration_ms == 0
    }

    pub fn merge(&mut self, other: Self) {
        self.count = self.count.saturating_add(other.count);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeMetricsSummary {
    pub tool_calls: RuntimeMetricTotals,
    pub api_calls: RuntimeMetricTotals,
    pub streaming_events: RuntimeMetricTotals,
    pub websocket_calls: RuntimeMetricTotals,
    pub websocket_events: RuntimeMetricTotals,
    pub responses_api_overhead_ms: u64,
    pub responses_api_inference_time_ms: u64,
    pub responses_api_engine_iapi_ttft_ms: u64,
    pub responses_api_engine_service_ttft_ms: u64,
    pub responses_api_engine_iapi_tbt_ms: u64,
    pub responses_api_engine_service_tbt_ms: u64,
    pub turn_ttft_ms: u64,
    pub turn_ttfm_ms: u64,
}

impl RuntimeMetricsSummary {
    pub fn is_empty(self) -> bool {
        self.tool_calls.is_empty()
            && self.api_calls.is_empty()
            && self.streaming_events.is_empty()
            && self.websocket_calls.is_empty()
            && self.websocket_events.is_empty()
            && self.responses_api_overhead_ms == 0
            && self.responses_api_inference_time_ms == 0
            && self.responses_api_engine_iapi_ttft_ms == 0
            && self.responses_api_engine_service_ttft_ms == 0
            && self.responses_api_engine_iapi_tbt_ms == 0
            && self.responses_api_engine_service_tbt_ms == 0
            && self.turn_ttft_ms == 0
            && self.turn_ttfm_ms == 0
    }

    pub fn merge(&mut self, other: Self) {
        self.tool_calls.merge(other.tool_calls);
        self.api_calls.merge(other.api_calls);
        self.streaming_events.merge(other.streaming_events);
        self.websocket_calls.merge(other.websocket_calls);
        self.websocket_events.merge(other.websocket_events);
        if other.responses_api_overhead_ms > 0 {
            self.responses_api_overhead_ms = other.responses_api_overhead_ms;
        }
        if other.responses_api_inference_time_ms > 0 {
            self.responses_api_inference_time_ms = other.responses_api_inference_time_ms;
        }
        if other.responses_api_engine_iapi_ttft_ms > 0 {
            self.responses_api_engine_iapi_ttft_ms = other.responses_api_engine_iapi_ttft_ms;
        }
        if other.responses_api_engine_service_ttft_ms > 0 {
            self.responses_api_engine_service_ttft_ms = other.responses_api_engine_service_ttft_ms;
        }
        if other.responses_api_engine_iapi_tbt_ms > 0 {
            self.responses_api_engine_iapi_tbt_ms = other.responses_api_engine_iapi_tbt_ms;
        }
        if other.responses_api_engine_service_tbt_ms > 0 {
            self.responses_api_engine_service_tbt_ms = other.responses_api_engine_service_tbt_ms;
        }
        if other.turn_ttft_ms > 0 {
            self.turn_ttft_ms = other.turn_ttft_ms;
        }
        if other.turn_ttfm_ms > 0 {
            self.turn_ttfm_ms = other.turn_ttfm_ms;
        }
    }

    pub fn responses_api_summary(&self) -> RuntimeMetricsSummary {
        Self {
            responses_api_overhead_ms: self.responses_api_overhead_ms,
            responses_api_inference_time_ms: self.responses_api_inference_time_ms,
            responses_api_engine_iapi_ttft_ms: self.responses_api_engine_iapi_ttft_ms,
            responses_api_engine_service_ttft_ms: self.responses_api_engine_service_ttft_ms,
            responses_api_engine_iapi_tbt_ms: self.responses_api_engine_iapi_tbt_ms,
            responses_api_engine_service_tbt_ms: self.responses_api_engine_service_tbt_ms,
            ..RuntimeMetricsSummary::default()
        }
    }
}

const OTHER_ORIGINATOR_TAG_VALUE: &str = "other";
const KNOWN_ORIGINATOR_TAG_VALUES: &[&str] = &[
    "codex_desktop",
    "app-server",
    "codex_mcp_server",
    "codex_cli_rs",
    "codex-tui",
    "codex_vscode",
    "none",
    "codex_exec",
    "codex-cli",
    "codex_sdk_ts",
    "codex-app-server-sdk",
];

/// Return a known low-cardinality originator tag value, or `other`.
pub fn bounded_originator_tag_value(originator: &str) -> &'static str {
    let sanitized = sanitize_metric_tag_value(originator);
    KNOWN_ORIGINATOR_TAG_VALUES
        .iter()
        .copied()
        .find(|known| *known == sanitized.as_str())
        .unwrap_or(OTHER_ORIGINATOR_TAG_VALUE)
}

/// Lightweight metrics sink shared by crates that should not depend on the
/// concrete OTEL runtime.
///
/// Implementations are expected to handle exporter-specific validation and
/// errors internally. Callers use this trait for best-effort telemetry only.
pub trait MetricsSink: Send + Sync {
    fn counter(&self, metric: &str, inc: i64, tags: &[(&str, &str)]);

    fn histogram(&self, metric: &str, value: i64, tags: &[(&str, &str)]);

    fn record_duration(&self, metric: &str, duration: Duration, tags: &[(&str, &str)]);
}

static GLOBAL_METRICS: OnceLock<Arc<dyn MetricsSink>> = OnceLock::new();

pub fn install_global_metrics(metrics: Arc<dyn MetricsSink>) -> bool {
    GLOBAL_METRICS.set(metrics).is_ok()
}

pub fn global_metrics() -> Option<Arc<dyn MetricsSink>> {
    GLOBAL_METRICS.get().cloned()
}

pub fn record_global_counter(metric: &str, inc: i64, tags: &[(&str, &str)]) {
    if let Some(metrics) = global_metrics() {
        metrics.counter(metric, inc, tags);
    }
}

pub fn record_global_histogram(metric: &str, value: i64, tags: &[(&str, &str)]) {
    if let Some(metrics) = global_metrics() {
        metrics.histogram(metric, value, tags);
    }
}

pub fn record_global_duration(metric: &str, duration: Duration, tags: &[(&str, &str)]) {
    if let Some(metrics) = global_metrics() {
        metrics.record_duration(metric, duration, tags);
    }
}

pub fn start_global_timer(metric: &str, tags: &[(&str, &str)]) -> Option<GlobalDurationTimer> {
    global_metrics().map(|metrics| GlobalDurationTimer {
        metrics,
        metric: metric.to_string(),
        tags: tags
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
        start: Instant::now(),
    })
}

pub struct GlobalDurationTimer {
    metrics: Arc<dyn MetricsSink>,
    metric: String,
    tags: Vec<(String, String)>,
    start: Instant,
}

impl Drop for GlobalDurationTimer {
    fn drop(&mut self) {
        self.record(&[]);
    }
}

impl GlobalDurationTimer {
    pub fn record(&self, additional_tags: &[(&str, &str)]) {
        let mut tags = Vec::with_capacity(self.tags.len() + additional_tags.len());
        tags.extend(additional_tags);
        tags.extend(
            self.tags
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        self.metrics
            .record_duration(&self.metric, self.start.elapsed(), &tags);
    }
}
