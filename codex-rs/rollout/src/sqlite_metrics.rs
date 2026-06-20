use std::sync::Arc;
use std::time::Duration;

use codex_metrics_api::MetricsSink;
use codex_metrics_api::ORIGINATOR_TAG;
use codex_metrics_api::bounded_originator_tag_value;
use codex_state::DbTelemetry;
use codex_state::DbTelemetryHandle;

struct MetricsDbTelemetry {
    metrics: Arc<dyn MetricsSink>,
    originator: &'static str,
}

impl DbTelemetry for MetricsDbTelemetry {
    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        let tags = with_originator(tags, self.originator);
        self.metrics.counter(name, inc, &tags);
    }

    fn record_duration(&self, name: &str, duration: Duration, tags: &[(&str, &str)]) {
        let tags = with_originator(tags, self.originator);
        self.metrics.record_duration(name, duration, &tags);
    }
}

pub(crate) fn recorder(metrics: Arc<dyn MetricsSink>, originator: &str) -> DbTelemetryHandle {
    Arc::new(MetricsDbTelemetry {
        metrics,
        originator: bounded_originator_tag_value(originator),
    })
}

fn with_originator<'a>(
    tags: &[(&'a str, &'a str)],
    originator: &'static str,
) -> Vec<(&'a str, &'a str)> {
    let mut tags = tags.to_vec();
    tags.push((ORIGINATOR_TAG, originator));
    tags
}
