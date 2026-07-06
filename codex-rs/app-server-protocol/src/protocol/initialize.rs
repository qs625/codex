use codex_utils_absolute_path::AbsolutePathBuf;
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
use ts_rs::TS;

/// Client-declared capabilities negotiated during initialize.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeCapabilities {
    /// Opt into receiving experimental API methods and fields.
    #[serde(default)]
    pub experimental_api: bool,
    /// Opt into `attestation/generate` requests for upstream `x-oai-attestation`.
    #[serde(default)]
    pub request_attestation: bool,
    /// Exact notification method names that should be suppressed for this
    /// connection (for example `thread/started`).
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub opt_out_notification_methods: Option<Vec<String>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<InitializeCapabilities>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub user_agent: String,
    /// Absolute path to the server's $CODEX_HOME directory.
    pub codex_home: AbsolutePathBuf,
    /// Platform family for the running app-server target, for example
    /// `"unix"` or `"windows"`.
    pub platform_family: String,
    /// Operating system for the running app-server target, for example
    /// `"macos"` or `"linux"` or `"windows"`.
    pub platform_os: String,
}
