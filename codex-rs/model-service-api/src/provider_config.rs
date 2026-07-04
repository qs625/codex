use std::collections::HashMap;
use std::time::Duration;

use crate::CHATGPT_CODEX_BASE_URL;
use crate::ModelProviderInfo;
use crate::Provider;
use crate::RetryConfig;
use codex_auth_types::AuthMode;
use http::HeaderMap;
use http::header::HeaderName;
use http::header::HeaderValue;

/// Adapts serializable provider metadata into API-client provider configuration.
pub fn model_provider_info_to_api_provider(
    provider: &ModelProviderInfo,
    auth_mode: Option<AuthMode>,
) -> Provider {
    let default_base_url = if matches!(
        auth_mode,
        Some(AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens | AuthMode::AgentIdentity)
    ) {
        CHATGPT_CODEX_BASE_URL
    } else {
        "https://api.openai.com/v1"
    };
    let base_url = provider
        .base_url
        .clone()
        .unwrap_or_else(|| default_base_url.to_string());

    let headers = build_header_map(provider);
    let retry = RetryConfig {
        max_attempts: provider.request_max_retries(),
        base_delay: Duration::from_millis(200),
        retry_429: false,
        retry_5xx: true,
        retry_transport: true,
    };

    Provider {
        name: provider.name.clone(),
        base_url,
        query_params: provider.query_params.clone(),
        headers,
        retry,
        stream_idle_timeout: provider.stream_idle_timeout(),
    }
}

fn build_header_map(provider: &ModelProviderInfo) -> HeaderMap {
    let capacity = provider.http_headers.as_ref().map_or(0, HashMap::len)
        + provider.env_http_headers.as_ref().map_or(0, HashMap::len);
    let mut headers = HeaderMap::with_capacity(capacity);
    if let Some(extra) = &provider.http_headers {
        for (k, v) in extra {
            if let (Ok(name), Ok(value)) = (HeaderName::try_from(k), HeaderValue::try_from(v)) {
                headers.insert(name, value);
            }
        }
    }

    if let Some(env_headers) = &provider.env_http_headers {
        for (header, env_var) in env_headers {
            if let Ok(val) = std::env::var(env_var)
                && !val.trim().is_empty()
                && let (Ok(name), Ok(value)) =
                    (HeaderName::try_from(header), HeaderValue::try_from(val))
            {
                headers.insert(name, value);
            }
        }
    }

    headers
}
