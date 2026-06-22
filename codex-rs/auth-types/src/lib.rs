use schemars::JsonSchema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::Error as SerdeError;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use strum_macros::Display;
use ts_rs::TS;

use codex_config_types::AuthCredentialsStoreMode;

/// Authentication mode for OpenAI-backed providers.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// OpenAI API key provided by the caller and stored by Codex.
    ApiKey,
    /// ChatGPT OAuth managed by Codex (tokens persisted and refreshed by Codex).
    Chatgpt,
    /// [UNSTABLE] FOR OPENAI INTERNAL USE ONLY - DO NOT USE.
    ///
    /// ChatGPT auth tokens are supplied by an external host app and are only
    /// stored in memory. Token refresh must be handled by the external host app.
    #[serde(rename = "chatgptAuthTokens")]
    #[ts(rename = "chatgptAuthTokens")]
    #[strum(serialize = "chatgptAuthTokens")]
    ChatgptAuthTokens,
    /// Programmatic Codex auth backed by a registered Agent Identity.
    #[serde(rename = "agentIdentity")]
    #[ts(rename = "agentIdentity")]
    #[strum(serialize = "agentIdentity")]
    AgentIdentity,
}

/// Authentication mode normalized for telemetry tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum TelemetryAuthMode {
    ApiKey,
    Chatgpt,
}

impl From<AuthMode> for TelemetryAuthMode {
    fn from(mode: AuthMode) -> Self {
        match mode {
            AuthMode::ApiKey => Self::ApiKey,
            AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens | AuthMode::AgentIdentity => {
                Self::Chatgpt
            }
        }
    }
}

/// Model/API request authentication captured without depending on the login runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestAuthSnapshot {
    Bearer(BearerRequestAuthSnapshot),
    AgentIdentity(AgentIdentityRequestAuthSnapshot),
}

impl RequestAuthSnapshot {
    pub fn auth_mode(&self) -> AuthMode {
        match self {
            Self::Bearer(auth) => auth.auth_mode,
            Self::AgentIdentity(_) => AuthMode::AgentIdentity,
        }
    }

    pub fn uses_codex_backend(&self) -> bool {
        !matches!(self.auth_mode(), AuthMode::ApiKey)
    }

    pub fn is_chatgpt_auth(&self) -> bool {
        matches!(
            self.auth_mode(),
            AuthMode::Chatgpt | AuthMode::ChatgptAuthTokens
        )
    }

    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::Bearer(auth) => auth.account_id.as_deref(),
            Self::AgentIdentity(auth) => Some(auth.account_id.as_str()),
        }
    }

    pub fn chatgpt_user_id(&self) -> Option<&str> {
        match self {
            Self::Bearer(auth) => auth.chatgpt_user_id.as_deref(),
            Self::AgentIdentity(auth) => Some(auth.chatgpt_user_id.as_str()),
        }
    }

    pub fn is_workspace_account(&self) -> bool {
        match self {
            Self::Bearer(auth) => auth.is_workspace_account,
            Self::AgentIdentity(auth) => auth.is_workspace_account,
        }
    }
}

/// Bearer-token request auth plus optional ChatGPT routing metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BearerRequestAuthSnapshot {
    pub auth_mode: AuthMode,
    pub token: Option<String>,
    pub account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
    pub is_workspace_account: bool,
    pub is_fedramp_account: bool,
}

/// Agent Identity request auth plus data needed to sign each request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentityRequestAuthSnapshot {
    pub agent_runtime_id: String,
    pub private_key_pkcs8_base64: String,
    pub task_id: String,
    pub account_id: String,
    pub chatgpt_user_id: String,
    pub is_workspace_account: bool,
    pub is_fedramp_account: bool,
}

/// Boxed future returned by object-safe authentication runtime APIs.
pub type AuthRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Authentication details used for session-scoped telemetry and audit metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthTelemetrySnapshot {
    pub auth_mode: Option<AuthMode>,
    pub account_id: Option<String>,
    pub account_email: Option<String>,
    pub uses_enterprise_default_service_tier: bool,
}

/// Runtime authentication boundary for crates that should not depend on the
/// concrete login runtime.
///
/// Implementations own token refresh, auth storage, and host-specific login
/// behavior. Consumers should use lightweight request or telemetry snapshots
/// instead of matching `CodexAuth` variants.
pub trait AuthRuntime: std::fmt::Debug + Send + Sync {
    /// Returns current request auth, refreshing if the implementation supports it.
    fn auth(&self) -> AuthRuntimeFuture<'_, Option<RequestAuthSnapshot>>;

    /// Returns cached request auth without attempting refresh.
    fn auth_cached(&self) -> Option<RequestAuthSnapshot>;

    /// Returns cached telemetry metadata without attempting refresh.
    fn telemetry_snapshot(&self) -> AuthTelemetrySnapshot {
        let auth = self.auth_cached();
        AuthTelemetrySnapshot {
            auth_mode: auth.as_ref().map(RequestAuthSnapshot::auth_mode),
            account_id: auth.and_then(|auth| auth.account_id().map(ToOwned::to_owned)),
            account_email: None,
            uses_enterprise_default_service_tier: false,
        }
    }

    /// Returns whether CODEX_API_KEY environment auth is enabled.
    fn codex_api_key_env_enabled(&self) -> bool;

    /// Returns whether the cached auth can access Codex backend-only APIs.
    fn current_auth_uses_codex_backend(&self) -> bool {
        self.auth_cached()
            .as_ref()
            .is_some_and(RequestAuthSnapshot::uses_codex_backend)
    }
}

pub type SharedAuthRuntime = Arc<dyn AuthRuntime>;

/// Backward-compatible shape for ChatGPT workspace login restrictions in config.toml.
#[derive(Serialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(untagged)]
pub enum ForcedChatgptWorkspaceIds {
    Single(String),
    Multiple(Vec<String>),
}

impl ForcedChatgptWorkspaceIds {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(value) => vec![value],
            Self::Multiple(values) => values,
        }
    }
}

impl<'de> Deserialize<'de> for ForcedChatgptWorkspaceIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Single(String),
            Multiple(Vec<String>),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Single(value) if value.contains(',') => Err(D::Error::custom(
                "forced_chatgpt_workspace_id must be a single workspace ID string or a TOML list \
of strings; comma-separated strings are not supported. Use \
`forced_chatgpt_workspace_id = [\"123e4567-e89b-42d3-a456-426614174000\", \
\"123e4567-e89b-42d3-a456-426614174001\"]` instead.",
            )),
            Repr::Single(value) => Ok(Self::Single(value)),
            Repr::Multiple(values) => Ok(Self::Multiple(values)),
        }
    }
}

/// Authentication environment metadata attached to session telemetry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthEnvTelemetryMetadata {
    pub openai_api_key_env_present: bool,
    pub codex_api_key_env_present: bool,
    pub codex_api_key_env_enabled: bool,
    pub provider_env_key_name: Option<String>,
    pub provider_env_key_present: Option<bool>,
    pub refresh_token_url_override_present: bool,
}

pub const OPENAI_API_KEY_ENV_VAR: &str = "OPENAI_API_KEY";
pub const CODEX_API_KEY_ENV_VAR: &str = "CODEX_API_KEY";
pub const REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR: &str = "CODEX_REFRESH_TOKEN_URL_OVERRIDE";

pub fn read_openai_api_key_from_env() -> Option<String> {
    read_non_empty_env_var(OPENAI_API_KEY_ENV_VAR)
}

fn read_non_empty_env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Inputs for collecting authentication environment telemetry without depending
/// on the login runtime or full model-provider implementation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthEnvTelemetryInput<'a> {
    pub provider_env_key: Option<&'a str>,
    pub codex_api_key_env_enabled: bool,
}

/// Resolved auth configuration required by login-aware runtime constructors.
///
/// Implementations should expose only auth-related values from an already-resolved
/// runtime config. The full login runtime consumes this trait, while callers can
/// implement it without depending on `codex-login`.
pub trait AuthManagerConfig {
    /// Returns the Codex home directory used for auth storage.
    fn codex_home(&self) -> PathBuf;

    /// Returns the CLI auth credential storage mode for auth loading.
    fn cli_auth_credentials_store_mode(&self) -> AuthCredentialsStoreMode;

    /// Returns the workspace IDs that ChatGPT auth should be restricted to, if any.
    fn forced_chatgpt_workspace_id(&self) -> Option<Vec<String>>;

    /// Returns the ChatGPT backend base URL used for first-party backend authorization.
    fn chatgpt_base_url(&self) -> String;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthEnvTelemetry {
    pub openai_api_key_env_present: bool,
    pub codex_api_key_env_present: bool,
    pub codex_api_key_env_enabled: bool,
    pub provider_env_key_name: Option<String>,
    pub provider_env_key_present: Option<bool>,
    pub refresh_token_url_override_present: bool,
}

impl AuthEnvTelemetry {
    pub fn to_otel_metadata(&self) -> AuthEnvTelemetryMetadata {
        AuthEnvTelemetryMetadata {
            openai_api_key_env_present: self.openai_api_key_env_present,
            codex_api_key_env_present: self.codex_api_key_env_present,
            codex_api_key_env_enabled: self.codex_api_key_env_enabled,
            provider_env_key_name: self.provider_env_key_name.clone(),
            provider_env_key_present: self.provider_env_key_present,
            refresh_token_url_override_present: self.refresh_token_url_override_present,
        }
    }
}

pub fn collect_auth_env_telemetry(input: AuthEnvTelemetryInput<'_>) -> AuthEnvTelemetry {
    AuthEnvTelemetry {
        openai_api_key_env_present: env_var_present(OPENAI_API_KEY_ENV_VAR),
        codex_api_key_env_present: env_var_present(CODEX_API_KEY_ENV_VAR),
        codex_api_key_env_enabled: input.codex_api_key_env_enabled,
        provider_env_key_name: input.provider_env_key.map(|_| "configured".to_string()),
        provider_env_key_present: input.provider_env_key.map(env_var_present),
        refresh_token_url_override_present: env_var_present(REFRESH_TOKEN_URL_OVERRIDE_ENV_VAR),
    }
}

fn env_var_present(name: &str) -> bool {
    match std::env::var(name) {
        Ok(value) => !value.trim().is_empty(),
        Err(std::env::VarError::NotUnicode(_)) => true,
        Err(std::env::VarError::NotPresent) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn collect_auth_env_telemetry_buckets_provider_env_key_name() {
        let telemetry = collect_auth_env_telemetry(AuthEnvTelemetryInput {
            provider_env_key: Some("sk-should-not-leak"),
            codex_api_key_env_enabled: false,
        });

        assert_eq!(
            telemetry.provider_env_key_name,
            Some("configured".to_string())
        );
    }

    #[test]
    fn read_non_empty_env_var_trims_blank_values() {
        const TEST_ENV_VAR: &str = "CODEX_AUTH_TYPES_EMPTY_ENV_TEST";
        let _guard = EnvVarGuard::set(TEST_ENV_VAR, "  ");

        assert_eq!(read_non_empty_env_var(TEST_ENV_VAR), None);
    }

    struct EnvVarGuard {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = std::env::var(name).ok();
            // SAFETY: This test crate does not concurrently read this private test variable.
            unsafe { std::env::set_var(name, value) };
            Self { name, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => {
                    // SAFETY: This test crate does not concurrently read this private test variable.
                    unsafe { std::env::set_var(self.name, value) };
                }
                None => {
                    // SAFETY: This test crate does not concurrently read this private test variable.
                    unsafe { std::env::remove_var(self.name) };
                }
            }
        }
    }
}
