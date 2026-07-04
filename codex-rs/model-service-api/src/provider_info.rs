//! Registry of model providers supported by Codex.
//!
//! Providers can be defined in two places:
//!   1. Built-in defaults compiled into the binary so Codex works out-of-the-box.
//!   2. User-defined entries inside `~/.codex/config.toml` under the `model_providers`
//!      key. These override or extend the defaults at runtime.

use protocol::config_types::ModelProviderAuthInfo;
use protocol::error::CodexErr;
use protocol::error::EnvVarError;
use protocol::error::Result as CodexResult;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_STREAM_MAX_RETRIES: u64 = 5;
const DEFAULT_REQUEST_MAX_RETRIES: u64 = 4;
pub const DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS: u64 = 15_000;
/// Hard cap for user-configured `stream_max_retries`.
const MAX_STREAM_MAX_RETRIES: u64 = 100;
/// Hard cap for user-configured `request_max_retries`.
const MAX_REQUEST_MAX_RETRIES: u64 = 100;

const OPENAI_PROVIDER_NAME: &str = "OpenAI";
pub const OPENAI_PROVIDER_ID: &str = "openai";
pub const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const AMAZON_BEDROCK_PROVIDER_NAME: &str = "Amazon Bedrock";
pub const AMAZON_BEDROCK_PROVIDER_ID: &str = "amazon-bedrock";
pub const AMAZON_BEDROCK_GPT_5_4_MODEL_ID: &str = "openai.gpt-5.4";
pub const AMAZON_BEDROCK_DEFAULT_BASE_URL: &str =
    "https://bedrock-mantle.us-east-1.api.aws/openai/v1";
const AMAZON_BEDROCK_MANTLE_CLIENT_AGENT_HEADER: &str = "x-amzn-mantle-client-agent";
const AMAZON_BEDROCK_MANTLE_CLIENT_AGENT_VALUE: &str = "codex";
const CHAT_WIRE_API_REMOVED_ERROR: &str = "`wire_api = \"chat\"` is no longer supported.\nHow to fix: set `wire_api = \"responses\"` in your provider config.\nMore info: https://github.com/openai/codex/discussions/7782";
pub const LEGACY_OLLAMA_CHAT_PROVIDER_ID: &str = "ollama-chat";
pub const OLLAMA_CHAT_PROVIDER_REMOVED_ERROR: &str = "`ollama-chat` is no longer supported.\nHow to fix: replace `ollama-chat` with `ollama` in `model_provider`, `oss_provider`, or `--local-provider`.\nMore info: https://github.com/openai/codex/discussions/7782";

/// Wire protocol that the provider speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WireApi {
    /// The Responses API exposed by OpenAI at `/v1/responses`.
    #[default]
    Responses,
    /// OpenAI-compatible Chat Completions API at `<base_url>/chat/completions`.
    #[serde(rename = "chat_completions")]
    #[schemars(rename = "chat_completions")]
    ChatCompletions,
    /// Azure-compatible Chat Completions API where `base_url` is the full endpoint.
    #[serde(rename = "azure_chat_completions")]
    #[schemars(rename = "azure_chat_completions")]
    AzureChatCompletions,
}

impl fmt::Display for WireApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat_completions",
            Self::AzureChatCompletions => "azure_chat_completions",
        };
        f.write_str(value)
    }
}

impl<'de> Deserialize<'de> for WireApi {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "responses" => Ok(Self::Responses),
            "chat_completions" => Ok(Self::ChatCompletions),
            "azure_chat_completions" => Ok(Self::AzureChatCompletions),
            "chat" => Err(serde::de::Error::custom(CHAT_WIRE_API_REMOVED_ERROR)),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["responses", "chat_completions", "azure_chat_completions"],
            )),
        }
    }
}

/// Serializable representation of a provider definition.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModelProviderInfo {
    /// Friendly display name.
    #[serde(default)]
    pub name: String,
    /// Base URL for the provider's OpenAI-compatible API.
    pub base_url: Option<String>,
    /// Environment variable that stores the user's API key for this provider.
    pub env_key: Option<String>,

    /// Optional instructions to help the user get a valid value for the
    /// variable and set it.
    pub env_key_instructions: Option<String>,
    /// Value to use with `Authorization: Bearer <token>` header. Use of this
    /// config is discouraged in favor of `env_key` for security reasons, but
    /// this may be necessary when using this programmatically.
    pub experimental_bearer_token: Option<String>,
    /// Command-backed bearer-token configuration for this provider.
    pub auth: Option<ModelProviderAuthInfo>,
    /// AWS SigV4 auth configuration for this provider.
    pub aws: Option<ModelProviderAwsAuthInfo>,
    /// Which wire protocol this provider expects.
    #[serde(default)]
    pub wire_api: WireApi,
    /// Optional query parameters to append to the base URL.
    pub query_params: Option<HashMap<String, String>>,
    /// Additional HTTP headers to include in requests to this provider where
    /// the (key, value) pairs are the header name and value.
    pub http_headers: Option<HashMap<String, String>>,
    /// Optional HTTP headers to include in requests to this provider where the
    /// (key, value) pairs are the header name and _environment variable_ whose
    /// value should be used. If the environment variable is not set, or the
    /// value is empty, the header will not be included in the request.
    pub env_http_headers: Option<HashMap<String, String>>,
    /// Maximum number of times to retry a failed HTTP request to this provider.
    pub request_max_retries: Option<u64>,
    /// Number of times to retry reconnecting a dropped streaming response before failing.
    pub stream_max_retries: Option<u64>,
    /// Idle timeout (in milliseconds) to wait for activity on a streaming response before treating
    /// the connection as lost.
    pub stream_idle_timeout_ms: Option<u64>,
    /// Maximum time (in milliseconds) to wait for a websocket connection attempt before treating
    /// it as failed.
    pub websocket_connect_timeout_ms: Option<u64>,
    /// Does this provider require an OpenAI API Key or ChatGPT login token? If true,
    /// user is presented with login screen on first run, and login preference and token/key
    /// are stored in auth.json. If false (which is the default), login screen is skipped,
    /// and API key (if needed) comes from the "env_key" environment variable.
    #[serde(default)]
    pub requires_openai_auth: bool,
    /// Whether this provider supports the Responses API WebSocket transport.
    #[serde(default)]
    pub supports_websockets: bool,
}

/// AWS SigV4 auth configuration for a model provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModelProviderAwsAuthInfo {
    /// AWS profile name to use. When unset, the AWS SDK default chain decides.
    pub profile: Option<String>,
    /// AWS region to use for provider-specific endpoints.
    pub region: Option<String>,
}

/// User-visible model picker entry backed by a configured provider.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModelOptionToml {
    /// Upstream model identifier sent to the provider.
    pub model: String,
    /// Provider ID to use for this model option.
    pub provider: String,
    /// Optional provider base URL. When present, Codex can synthesize the provider entry.
    pub base_url: Option<String>,
    /// Optional ModelHub-style query parameter key appended as `ak`.
    pub ak: Option<String>,
    /// Optional environment variable containing the provider API key.
    pub env_key: Option<String>,
    /// Optional help text for configuring `env_key`.
    pub env_key_instructions: Option<String>,
    /// Optional bearer token literal.
    pub experimental_bearer_token: Option<String>,
    /// Optional wire protocol override for the synthesized provider.
    pub wire_api: Option<WireApi>,
    /// Optional query parameters appended to the provider URL.
    pub query_params: Option<HashMap<String, String>>,
    /// Optional literal HTTP headers sent to the provider.
    pub http_headers: Option<HashMap<String, String>>,
    /// Optional HTTP headers read from environment variables.
    pub env_http_headers: Option<HashMap<String, String>>,
    /// Optional provider request retry override.
    pub request_max_retries: Option<u64>,
    /// Optional provider stream retry override.
    pub stream_max_retries: Option<u64>,
    /// Optional stream idle timeout override in milliseconds.
    pub stream_idle_timeout_ms: Option<u64>,
    /// Optional websocket connect timeout override in milliseconds.
    pub websocket_connect_timeout_ms: Option<u64>,
    /// Size of the model context window, in tokens.
    pub context_window: Option<i64>,
    /// Maximum context window supported by the model, in tokens.
    pub max_context_window: Option<i64>,
    /// Token usage threshold triggering auto-compaction for this model.
    pub auto_compact_token_limit: Option<i64>,
    /// Chat Completions `max_tokens` value for this model option.
    pub max_tokens: Option<u64>,
    /// Additional Chat Completions request body fields for this model option.
    pub extra_body: Option<HashMap<String, JsonValue>>,
}

impl ModelOptionToml {
    pub fn has_inline_provider_config(&self) -> bool {
        self.base_url.is_some()
    }

    pub fn to_provider_info(&self) -> ModelProviderInfo {
        let query_params = match (self.ak.as_ref(), self.query_params.as_ref()) {
            (None, None) => None,
            (ak, query_params) => {
                let mut merged = query_params.cloned().unwrap_or_default();
                if let Some(ak) = ak {
                    merged.insert("ak".to_string(), ak.clone());
                }
                Some(merged)
            }
        };
        ModelProviderInfo {
            name: self.provider.clone(),
            base_url: self.base_url.clone(),
            env_key: self.env_key.clone(),
            env_key_instructions: self.env_key_instructions.clone(),
            experimental_bearer_token: self.experimental_bearer_token.clone(),
            wire_api: self.wire_api.unwrap_or(WireApi::AzureChatCompletions),
            query_params,
            http_headers: self.http_headers.clone(),
            env_http_headers: self.env_http_headers.clone(),
            request_max_retries: self.request_max_retries,
            stream_max_retries: self.stream_max_retries,
            stream_idle_timeout_ms: self.stream_idle_timeout_ms,
            websocket_connect_timeout_ms: self.websocket_connect_timeout_ms,
            ..Default::default()
        }
    }
}

impl ModelProviderInfo {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.aws.is_some() {
            if self.supports_websockets {
                // TODO(celia-oai): Support AWS SigV4 signing for WebSocket
                // upgrade requests before allowing AWS-authenticated providers
                // to enable Responses-over-WebSocket.
                return Err("provider aws cannot be combined with supports_websockets".to_string());
            }

            let mut conflicts = Vec::new();
            if self.env_key.is_some() {
                conflicts.push("env_key");
            }
            if self.experimental_bearer_token.is_some() {
                conflicts.push("experimental_bearer_token");
            }
            if self.auth.is_some() {
                conflicts.push("auth");
            }
            if self.requires_openai_auth {
                conflicts.push("requires_openai_auth");
            }

            if !conflicts.is_empty() {
                return Err(format!(
                    "provider aws cannot be combined with {}",
                    conflicts.join(", ")
                ));
            }
        }

        let Some(auth) = self.auth.as_ref() else {
            return Ok(());
        };

        if auth.command.trim().is_empty() {
            return Err("provider auth.command must not be empty".to_string());
        }

        let mut conflicts = Vec::new();
        if self.env_key.is_some() {
            conflicts.push("env_key");
        }
        if self.experimental_bearer_token.is_some() {
            conflicts.push("experimental_bearer_token");
        }
        if self.requires_openai_auth {
            conflicts.push("requires_openai_auth");
        }

        if conflicts.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "provider auth cannot be combined with {}",
                conflicts.join(", ")
            ))
        }
    }

    /// If `env_key` is Some, returns the API key for this provider if present
    /// (and non-empty) in the environment. If `env_key` is required but
    /// cannot be found, returns an error.
    pub fn api_key(&self) -> CodexResult<Option<String>> {
        match &self.env_key {
            Some(env_key) => {
                let api_key = std::env::var(env_key)
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| {
                        CodexErr::EnvVar(EnvVarError {
                            var: env_key.clone(),
                            instructions: self.env_key_instructions.clone(),
                        })
                    })?;
                Ok(Some(api_key))
            }
            None => Ok(None),
        }
    }

    /// Effective maximum number of request retries for this provider.
    pub fn request_max_retries(&self) -> u64 {
        self.request_max_retries
            .unwrap_or(DEFAULT_REQUEST_MAX_RETRIES)
            .min(MAX_REQUEST_MAX_RETRIES)
    }

    /// Effective maximum number of stream reconnection attempts for this provider.
    pub fn stream_max_retries(&self) -> u64 {
        self.stream_max_retries
            .unwrap_or(DEFAULT_STREAM_MAX_RETRIES)
            .min(MAX_STREAM_MAX_RETRIES)
    }

    /// Effective idle timeout for streaming responses.
    pub fn stream_idle_timeout(&self) -> Duration {
        self.stream_idle_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(DEFAULT_STREAM_IDLE_TIMEOUT_MS))
    }

    /// Effective timeout for websocket connect attempts.
    pub fn websocket_connect_timeout(&self) -> Duration {
        self.websocket_connect_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS))
    }

    pub fn create_openai_provider(base_url: Option<String>) -> ModelProviderInfo {
        ModelProviderInfo {
            name: OPENAI_PROVIDER_NAME.into(),
            base_url,
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            aws: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: Some(
                [("version".to_string(), env!("CARGO_PKG_VERSION").to_string())]
                    .into_iter()
                    .collect(),
            ),
            env_http_headers: Some(
                [
                    (
                        "OpenAI-Organization".to_string(),
                        "OPENAI_ORGANIZATION".to_string(),
                    ),
                    ("OpenAI-Project".to_string(), "OPENAI_PROJECT".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            // Use global defaults for retry/timeout unless overridden in config.toml.
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: true,
            supports_websockets: true,
        }
    }

    pub fn create_amazon_bedrock_provider(
        aws: Option<ModelProviderAwsAuthInfo>,
    ) -> ModelProviderInfo {
        ModelProviderInfo {
            name: AMAZON_BEDROCK_PROVIDER_NAME.into(),
            base_url: Some(AMAZON_BEDROCK_DEFAULT_BASE_URL.into()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            aws: Some(aws.unwrap_or(ModelProviderAwsAuthInfo {
                profile: None,
                region: None,
            })),
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: Some(HashMap::from([(
                AMAZON_BEDROCK_MANTLE_CLIENT_AGENT_HEADER.to_string(),
                AMAZON_BEDROCK_MANTLE_CLIENT_AGENT_VALUE.to_string(),
            )])),
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        }
    }

    pub fn is_openai(&self) -> bool {
        self.name == OPENAI_PROVIDER_NAME
    }

    pub fn is_amazon_bedrock(&self) -> bool {
        self.name == AMAZON_BEDROCK_PROVIDER_NAME
    }

    pub fn supports_remote_compaction(&self) -> bool {
        self.is_openai() || is_azure_responses_provider(&self.name, self.base_url.as_deref())
    }

    pub fn has_command_auth(&self) -> bool {
        self.auth.is_some()
    }
}

fn is_azure_responses_provider(name: &str, base_url: Option<&str>) -> bool {
    if name.eq_ignore_ascii_case("azure") {
        true
    } else if let Some(base_url) = base_url {
        matches_azure_responses_base_url(base_url)
    } else {
        false
    }
}

fn matches_azure_responses_base_url(base_url: &str) -> bool {
    let base_url = base_url.to_ascii_lowercase();
    const AZURE_MARKERS: [&str; 6] = [
        "openai.azure.",
        "cognitiveservices.azure.",
        "aoai.azure.",
        "azure-api.",
        "azurefd.",
        "windows.net/openai",
    ];
    AZURE_MARKERS.iter().any(|marker| base_url.contains(marker))
}

pub const DEFAULT_LMSTUDIO_PORT: u16 = 1234;
pub const DEFAULT_OLLAMA_PORT: u16 = 11434;

pub const LMSTUDIO_OSS_PROVIDER_ID: &str = "lmstudio";
pub const OLLAMA_OSS_PROVIDER_ID: &str = "ollama";

/// Built-in default provider list.
pub fn built_in_model_providers(
    openai_base_url: Option<String>,
) -> HashMap<String, ModelProviderInfo> {
    use ModelProviderInfo as P;
    let openai_provider = P::create_openai_provider(openai_base_url);
    let amazon_bedrock_provider = P::create_amazon_bedrock_provider(/*aws*/ None);

    // We do not want to be in the business of adjucating which third-party
    // providers are bundled with Codex CLI, so we only include the OpenAI and
    // open source ("oss") providers by default. Users are encouraged to add to
    // `model_providers` in config.toml to add their own providers.
    [
        (OPENAI_PROVIDER_ID, openai_provider),
        (AMAZON_BEDROCK_PROVIDER_ID, amazon_bedrock_provider),
        (
            OLLAMA_OSS_PROVIDER_ID,
            create_oss_provider(DEFAULT_OLLAMA_PORT, WireApi::Responses),
        ),
        (
            LMSTUDIO_OSS_PROVIDER_ID,
            create_oss_provider(DEFAULT_LMSTUDIO_PORT, WireApi::Responses),
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// Merge configured providers into the built-in provider catalog.
///
/// Configured providers extend the built-in set. Built-in providers are not
/// generally overridable, but the built-in Amazon Bedrock provider allows the
/// user to set `aws.profile` and `aws.region`.
pub fn merge_configured_model_providers(
    mut model_providers: HashMap<String, ModelProviderInfo>,
    configured_model_providers: HashMap<String, ModelProviderInfo>,
) -> Result<HashMap<String, ModelProviderInfo>, String> {
    for (key, mut provider) in configured_model_providers {
        if key == AMAZON_BEDROCK_PROVIDER_ID {
            let aws_override = provider.aws.take();
            if provider != ModelProviderInfo::default() {
                return Err(format!(
                    "model_providers.{AMAZON_BEDROCK_PROVIDER_ID} only supports changing \
`aws.profile` and `aws.region`; other non-default provider fields are not supported"
                ));
            }

            if let Some(aws_override) = aws_override
                && let Some(built_in_provider) = model_providers.get_mut(AMAZON_BEDROCK_PROVIDER_ID)
                && let Some(built_in_aws) = built_in_provider.aws.as_mut()
            {
                if let Some(profile) = aws_override.profile {
                    built_in_aws.profile = Some(profile);
                }
                if let Some(region) = aws_override.region {
                    built_in_aws.region = Some(region);
                }
            }
        } else {
            model_providers.entry(key).or_insert(provider);
        }
    }

    Ok(model_providers)
}

pub fn validate_reserved_model_provider_ids(
    model_providers: &HashMap<String, ModelProviderInfo>,
) -> Result<(), String> {
    let mut conflicts = model_providers
        .keys()
        .filter(|key| {
            key.as_str() != AMAZON_BEDROCK_PROVIDER_ID
                && matches!(
                    key.as_str(),
                    OPENAI_PROVIDER_ID | OLLAMA_OSS_PROVIDER_ID | LMSTUDIO_OSS_PROVIDER_ID
                )
        })
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>();
    conflicts.sort_unstable();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "model_providers contains reserved built-in provider IDs: {}. \
Built-in providers cannot be overridden. Rename your custom provider (for example, `openai-custom`).",
            conflicts.join(", ")
        ))
    }
}

pub fn validate_model_providers(
    model_providers: &HashMap<String, ModelProviderInfo>,
) -> Result<(), String> {
    validate_reserved_model_provider_ids(model_providers)?;
    for (key, provider) in model_providers {
        if key == AMAZON_BEDROCK_PROVIDER_ID {
            continue;
        }
        if provider.aws.is_some() {
            return Err(format!(
                "model_providers.{key}: provider aws is only supported for `{AMAZON_BEDROCK_PROVIDER_ID}`"
            ));
        }
        if provider.name.trim().is_empty() {
            return Err(format!(
                "model_providers.{key}: provider name must not be empty"
            ));
        }
        provider
            .validate()
            .map_err(|message| format!("model_providers.{key}: {message}"))?;
    }
    Ok(())
}

pub fn deserialize_model_providers<'de, D>(
    deserializer: D,
) -> Result<HashMap<String, ModelProviderInfo>, D::Error>
where
    D: Deserializer<'de>,
{
    let model_providers = HashMap::<String, ModelProviderInfo>::deserialize(deserializer)?;
    validate_model_providers(&model_providers).map_err(serde::de::Error::custom)?;
    Ok(model_providers)
}

pub fn validate_oss_provider(provider: &str) -> std::io::Result<()> {
    match provider {
        LMSTUDIO_OSS_PROVIDER_ID | OLLAMA_OSS_PROVIDER_ID => Ok(()),
        LEGACY_OLLAMA_CHAT_PROVIDER_ID => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            OLLAMA_CHAT_PROVIDER_REMOVED_ERROR,
        )),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "Invalid OSS provider '{provider}'. Must be one of: {LMSTUDIO_OSS_PROVIDER_ID}, {OLLAMA_OSS_PROVIDER_ID}"
            ),
        )),
    }
}

pub fn create_oss_provider(default_provider_port: u16, wire_api: WireApi) -> ModelProviderInfo {
    // These CODEX_OSS_ environment variables are experimental: we may
    // switch to reading values from config.toml instead.
    let default_codex_oss_base_url = format!(
        "http://localhost:{codex_oss_port}/v1",
        codex_oss_port = std::env::var("CODEX_OSS_PORT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(default_provider_port)
    );

    let codex_oss_base_url = std::env::var("CODEX_OSS_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(default_codex_oss_base_url);
    create_oss_provider_with_base_url(&codex_oss_base_url, wire_api)
}

pub fn create_oss_provider_with_base_url(base_url: &str, wire_api: WireApi) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "gpt-oss".into(),
        base_url: Some(base_url.into()),
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        wire_api,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        websocket_connect_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    }
}

#[cfg(test)]
#[path = "model_provider_info_tests.rs"]
mod tests;
