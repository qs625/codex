use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::str::FromStr;
use std::sync::OnceLock;
use std::sync::RwLock;

pub use codex_config_types::validate_otel_tracestate_entries as validate_tracestate_entries;
pub use codex_config_types::validate_otel_tracestate_member as validate_tracestate_member;
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::trace::TraceState;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use protocol::protocol::W3cTraceContext;
use tracing::Span;
use tracing::debug;
use tracing::warn;
use tracing_opentelemetry::OpenTelemetrySpanExt;

const TRACEPARENT_ENV_VAR: &str = "TRACEPARENT";
const TRACESTATE_ENV_VAR: &str = "TRACESTATE";
static TRACEPARENT_CONTEXT: OnceLock<Option<Context>> = OnceLock::new();

// Trace context propagation can happen outside the provider object, so configured
// tracestate lives beside the process-global tracer provider.
static TRACESTATE_ENTRIES: OnceLock<RwLock<BTreeMap<String, BTreeMap<String, String>>>> =
    OnceLock::new();

pub fn current_span_w3c_trace_context() -> Option<W3cTraceContext> {
    span_w3c_trace_context(&Span::current())
}

pub fn span_w3c_trace_context(span: &Span) -> Option<W3cTraceContext> {
    let context = span.context();
    if !context.span().span_context().is_valid() {
        return None;
    }

    let mut headers = HashMap::new();
    TraceContextPropagator::new().inject_context(&context, &mut headers);
    let tracestate = headers.remove("tracestate");
    let configured_tracestate_guard = tracestate_entries()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    Some(W3cTraceContext {
        traceparent: headers.remove("traceparent"),
        tracestate: merge_tracestate_entries(tracestate.as_deref(), &configured_tracestate_guard),
    })
}

pub fn set_tracestate_entries(
    entries: BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    validate_tracestate_entries(&entries)?;
    let mut guard = tracestate_entries()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = entries;
    Ok(())
}

pub fn current_span_trace_id() -> Option<String> {
    let context = Span::current().context();
    let span = context.span();
    let span_context = span.span_context();
    if !span_context.is_valid() {
        return None;
    }

    Some(span_context.trace_id().to_string())
}

pub fn context_from_w3c_trace_context(trace: &W3cTraceContext) -> Option<Context> {
    context_from_trace_headers(trace.traceparent.as_deref(), trace.tracestate.as_deref())
}

pub fn set_parent_from_w3c_trace_context(span: &Span, trace: &W3cTraceContext) -> bool {
    if let Some(context) = context_from_w3c_trace_context(trace) {
        set_parent_from_context(span, context);
        true
    } else {
        false
    }
}

pub fn set_parent_from_context(span: &Span, context: Context) {
    let _ = span.set_parent(context);
}

pub fn traceparent_context_from_env() -> Option<Context> {
    TRACEPARENT_CONTEXT
        .get_or_init(load_traceparent_context)
        .clone()
}

fn context_from_trace_headers(
    traceparent: Option<&str>,
    tracestate: Option<&str>,
) -> Option<Context> {
    let traceparent = traceparent?;
    let mut headers = HashMap::new();
    headers.insert("traceparent".to_string(), traceparent.to_string());
    if let Some(tracestate) = tracestate {
        headers.insert("tracestate".to_string(), tracestate.to_string());
    }

    let context = TraceContextPropagator::new().extract(&headers);
    if !context.span().span_context().is_valid() {
        return None;
    }
    Some(context)
}

fn load_traceparent_context() -> Option<Context> {
    let traceparent = env::var(TRACEPARENT_ENV_VAR).ok()?;
    let tracestate = env::var(TRACESTATE_ENV_VAR).ok();

    match context_from_trace_headers(Some(&traceparent), tracestate.as_deref()) {
        Some(context) => {
            debug!("TRACEPARENT detected; continuing trace from parent context");
            Some(context)
        }
        None => {
            warn!("TRACEPARENT is set but invalid; ignoring trace context");
            None
        }
    }
}

fn tracestate_entries() -> &'static RwLock<BTreeMap<String, BTreeMap<String, String>>> {
    TRACESTATE_ENTRIES.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn merge_tracestate_entries(
    tracestate: Option<&str>,
    configured_entries: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<String> {
    let mut trace_state = tracestate
        .and_then(|tracestate| match TraceState::from_str(tracestate) {
            Ok(trace_state) => Some(trace_state),
            Err(err) => {
                warn!("ignoring invalid tracestate while propagating trace context: {err}");
                None
            }
        })
        .unwrap_or_default();

    for (key, fields) in configured_entries.iter().rev() {
        let value = merge_tracestate_member_fields(trace_state.get(key), fields);
        trace_state = match trace_state.insert(key.clone(), value) {
            Ok(trace_state) => trace_state,
            Err(err) => {
                warn!("ignoring configured tracestate while propagating trace context: {err}");
                break;
            }
        };
    }

    let tracestate = trace_state.header();
    (!tracestate.is_empty()).then_some(tracestate)
}

fn merge_tracestate_member_fields(
    existing: Option<&str>,
    configured_fields: &BTreeMap<String, String>,
) -> String {
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(existing) = existing {
        for field in existing.split(';').filter(|field| !field.is_empty()) {
            if let Some((field_key, _)) = field.split_once(':') {
                if let Some(value) = configured_fields.get(field_key) {
                    if seen.insert(field_key) {
                        fields.push(format!("{field_key}:{value}"));
                    }
                    continue;
                }
                seen.insert(field_key);
            }
            fields.push(field.to_string());
        }
    }

    fields.extend(
        configured_fields
            .iter()
            .filter(|(field_key, _)| !seen.contains(field_key.as_str()))
            .map(|(field_key, value)| format!("{field_key}:{value}")),
    );
    fields.join(";")
}
