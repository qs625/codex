//! OTEL configuration TOML and effective settings types.

use std::collections::BTreeMap;
use std::collections::HashMap;

use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

pub const DEFAULT_OTEL_ENVIRONMENT: &str = "dev";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OtelHttpProtocol {
    /// Binary payload
    Binary,
    /// JSON payload
    Json,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(rename_all = "kebab-case")]
pub struct OtelTlsConfig {
    pub ca_certificate: Option<AbsolutePathBuf>,
    pub client_certificate: Option<AbsolutePathBuf>,
    pub client_private_key: Option<AbsolutePathBuf>,
}

/// Which OTEL exporter to use.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
#[serde(rename_all = "kebab-case")]
pub enum OtelExporterKind {
    None,
    Statsig,
    OtlpHttp {
        endpoint: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        protocol: OtelHttpProtocol,
        #[serde(default)]
        tls: Option<OtelTlsConfig>,
    },
    OtlpGrpc {
        endpoint: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        tls: Option<OtelTlsConfig>,
    },
}

/// OTEL settings loaded from config.toml. Fields are optional so we can apply defaults.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct OtelConfigToml {
    /// Log user prompt in traces
    pub log_user_prompt: Option<bool>,

    /// Mark traces with environment (dev, staging, prod, test). Defaults to dev.
    pub environment: Option<String>,

    /// Optional log exporter
    pub exporter: Option<OtelExporterKind>,

    /// Optional trace exporter
    pub trace_exporter: Option<OtelExporterKind>,

    /// Optional metrics exporter
    pub metrics_exporter: Option<OtelExporterKind>,

    /// Attributes to add to every exported trace span.
    pub span_attributes: Option<BTreeMap<String, String>>,

    /// Semicolon-separated `key:value` fields to upsert into W3C tracestate members.
    pub tracestate: Option<BTreeMap<String, BTreeMap<String, String>>>,
}

/// Effective OTEL settings after defaults are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct OtelConfig {
    pub log_user_prompt: bool,
    pub environment: String,
    pub exporter: OtelExporterKind,
    pub trace_exporter: OtelExporterKind,
    pub metrics_exporter: OtelExporterKind,
    pub span_attributes: BTreeMap<String, String>,
    pub tracestate: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for OtelConfig {
    fn default() -> Self {
        OtelConfig {
            log_user_prompt: false,
            environment: DEFAULT_OTEL_ENVIRONMENT.to_owned(),
            exporter: OtelExporterKind::None,
            trace_exporter: OtelExporterKind::None,
            metrics_exporter: OtelExporterKind::Statsig,
            span_attributes: BTreeMap::new(),
            tracestate: BTreeMap::new(),
        }
    }
}

/// Validates configured span attributes before they are attached to exported spans.
pub fn validate_otel_span_attributes(attributes: &BTreeMap<String, String>) -> std::io::Result<()> {
    if attributes.keys().any(String::is_empty) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "configured span attribute key must not be empty",
        ));
    }

    Ok(())
}

/// Validates configured tracestate members before they are propagated in W3C trace context.
pub fn validate_otel_tracestate_entries(
    entries: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    for (member_key, fields) in entries {
        validate_otel_tracestate_member(member_key, fields)?;
    }
    Ok(())
}

/// Validates one configured tracestate member and its encoded field value.
pub fn validate_otel_tracestate_member(
    member_key: &str,
    fields: &BTreeMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let value = encode_tracestate_member_fields(member_key, fields)?;
    if !is_w3c_tracestate_member_key(member_key) {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate member key {member_key}"
        )));
    }
    if !is_w3c_tracestate_member_value(&value) {
        return Err(invalid_tracestate_config(format!(
            "invalid configured tracestate value for {member_key}"
        )));
    }
    Ok(())
}

fn encode_tracestate_member_fields(
    member_key: &str,
    fields: &BTreeMap<String, String>,
) -> Result<String, Box<dyn std::error::Error>> {
    // Configured fields are encoded into one opaque tracestate member value.
    // Validate both the field grammar and the final header value so malformed
    // config cannot produce propagated trace context that downstream W3C
    // extractors reject.
    let mut encoded = Vec::with_capacity(fields.len());
    for (field_key, value) in fields {
        if !is_configured_tracestate_field_key(field_key) {
            return Err(invalid_tracestate_config(format!(
                "invalid configured tracestate field key {member_key}.{field_key}"
            )));
        }
        if !is_configured_tracestate_field_value(value) {
            return Err(invalid_tracestate_config(format!(
                "invalid configured tracestate value for {member_key}.{field_key}"
            )));
        }
        encoded.push(format!("{field_key}:{value}"));
    }
    Ok(encoded.join(";"))
}

fn is_configured_tracestate_field_key(field_key: &str) -> bool {
    !field_key.is_empty()
        && field_key
            .bytes()
            .all(|byte| matches!(byte, b'!'..=b'~') && !matches!(byte, b':' | b';' | b',' | b'='))
}

fn is_configured_tracestate_field_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| is_tracestate_member_value_byte(byte) && byte != b';')
}

fn is_w3c_tracestate_member_key(key: &str) -> bool {
    if key.is_empty() || key.len() > 256 {
        return false;
    }

    let mut vendor_separator_index = None;
    for (index, byte) in key.bytes().enumerate() {
        if !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'-' | b'*' | b'/' | b'@'))
        {
            return false;
        }

        if index == 0 && !(byte.is_ascii_lowercase() || byte.is_ascii_digit()) {
            return false;
        }

        if byte == b'@' {
            if vendor_separator_index.is_some() || index + 14 < key.len() {
                return false;
            }
            vendor_separator_index = Some(index);
            continue;
        }

        if let Some(separator_index) = vendor_separator_index
            && index == separator_index + 1
            && !(byte.is_ascii_lowercase() || byte.is_ascii_digit())
        {
            return false;
        }
    }

    true
}

fn is_w3c_tracestate_member_value(value: &str) -> bool {
    value.len() <= 256
        && (value.is_empty()
            || (value.bytes().all(is_tracestate_member_value_byte)
                && value.as_bytes().last().is_some_and(|byte| *byte != b' ')))
}

fn is_tracestate_member_value_byte(byte: u8) -> bool {
    matches!(byte, b' '..=b'~') && !matches!(byte, b',' | b'=')
}

fn invalid_tracestate_config(message: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn span_attributes_reject_empty_key() {
        let err =
            validate_otel_span_attributes(&BTreeMap::from([("".to_string(), "value".to_string())]))
                .expect_err("empty attribute key should fail");

        assert_eq!(
            err.to_string(),
            "configured span attribute key must not be empty"
        );
    }

    #[test]
    fn tracestate_accepts_configured_fields() {
        let entries = BTreeMap::from([(
            "example@vendor".to_string(),
            BTreeMap::from([
                ("alpha".to_string(), "one".to_string()),
                ("beta".to_string(), "two".to_string()),
            ]),
        )]);

        validate_otel_tracestate_entries(&entries).expect("valid tracestate");
    }

    #[test]
    fn tracestate_rejects_invalid_member_key() {
        let err = validate_otel_tracestate_member(
            "BadKey",
            &BTreeMap::from([("alpha".to_string(), "one".to_string())]),
        )
        .expect_err("uppercase member key should fail");

        assert_eq!(
            err.to_string(),
            "invalid configured tracestate member key BadKey"
        );
    }

    #[test]
    fn tracestate_rejects_invalid_field_value() {
        let err = validate_otel_tracestate_member(
            "example",
            &BTreeMap::from([("alpha".to_string(), "bad,value".to_string())]),
        )
        .expect_err("comma in field value should fail");

        assert_eq!(
            err.to_string(),
            "invalid configured tracestate value for example.alpha"
        );
    }
}
