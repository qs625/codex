pub use codex_auth_types::AuthEnvTelemetry;
pub use codex_auth_types::AuthEnvTelemetryInput;
use model_service_api::ModelProviderInfo;

pub fn collect_auth_env_telemetry(
    provider: &ModelProviderInfo,
    codex_api_key_env_enabled: bool,
) -> AuthEnvTelemetry {
    codex_auth_types::collect_auth_env_telemetry(AuthEnvTelemetryInput {
        provider_env_key: provider.env_key.as_deref(),
        codex_api_key_env_enabled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use model_service_api::WireApi;
    use pretty_assertions::assert_eq;

    #[test]
    fn collect_auth_env_telemetry_buckets_provider_env_key_name() {
        let provider = ModelProviderInfo {
            name: "Custom".to_string(),
            base_url: None,
            env_key: Some("sk-should-not-leak".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            aws: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };

        let telemetry =
            collect_auth_env_telemetry(&provider, /*codex_api_key_env_enabled*/ false);

        assert_eq!(
            telemetry.provider_env_key_name,
            Some("configured".to_string())
        );
    }
}
