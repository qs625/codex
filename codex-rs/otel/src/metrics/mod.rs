mod client;
mod config;
mod error;
pub(crate) mod names;
mod process;
pub(crate) mod runtime_metrics;
pub(crate) mod tags;
pub(crate) mod timer;
pub(crate) mod validation;

use crate::config::StatsigMetricsSettings;
pub use crate::metrics::client::MetricsClient;
pub use crate::metrics::config::MetricsConfig;
pub use crate::metrics::config::MetricsExporter;
pub use crate::metrics::error::MetricsError;
pub use crate::metrics::error::Result;
pub use crate::metrics::process::record_process_start_once;
pub use codex_metrics_api::CURATED_PLUGINS_STARTUP_SYNC_FINAL_METRIC;
pub use codex_metrics_api::CURATED_PLUGINS_STARTUP_SYNC_METRIC;
pub use codex_metrics_api::MULTI_AGENT_NICKNAME_POOL_RESET_METRIC;
pub use codex_metrics_api::THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC;
pub use codex_metrics_api::THREAD_SKILLS_ENABLED_TOTAL_METRIC;
pub use codex_metrics_api::THREAD_SKILLS_KEPT_TOTAL_METRIC;
pub use codex_metrics_api::THREAD_SKILLS_TRUNCATED_METRIC;
pub use names::*;
use std::sync::Arc;
use std::sync::OnceLock;
pub use tags::ORIGINATOR_TAG;
pub use tags::SessionMetricTagValues;
pub use tags::bounded_originator_tag_value;

static GLOBAL_METRICS: OnceLock<MetricsClient> = OnceLock::new();
static GLOBAL_STATSIG_METRICS_SETTINGS: OnceLock<StatsigMetricsSettings> = OnceLock::new();

pub(crate) fn install_global(metrics: MetricsClient) {
    let _ = codex_metrics_api::install_global_metrics(Arc::new(metrics.clone()));
    let _ = GLOBAL_METRICS.set(metrics);
}

pub fn global() -> Option<MetricsClient> {
    GLOBAL_METRICS.get().cloned()
}

pub(crate) fn install_global_statsig_settings(settings: StatsigMetricsSettings) {
    let _ = GLOBAL_STATSIG_METRICS_SETTINGS.set(settings);
}

pub(crate) fn global_statsig_settings() -> Option<StatsigMetricsSettings> {
    GLOBAL_STATSIG_METRICS_SETTINGS.get().cloned()
}
