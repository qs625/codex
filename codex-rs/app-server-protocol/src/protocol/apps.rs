pub use codex_connectors_api::AppBranding;
pub use codex_connectors_api::AppInfo;
pub use codex_connectors_api::AppMetadata;
pub use codex_connectors_api::AppReview;
pub use codex_connectors_api::AppScreenshot;
pub use codex_connectors_api::AppSummary;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
/// EXPERIMENTAL - list available apps/connectors.
pub struct AppsListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional page size; defaults to a reasonable server-side value.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
    /// Optional thread id used to evaluate app feature gating from that thread's config.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_id: Option<String>,
    /// When true, bypass app caches and fetch the latest data from sources.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force_refetch: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
/// EXPERIMENTAL - app list response.
pub struct AppsListResponse {
    pub data: Vec<AppInfo>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// If None, there are no more items to return.
    pub next_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
/// EXPERIMENTAL - notification emitted when the app list changes.
pub struct AppListUpdatedNotification {
    pub data: Vec<AppInfo>,
}
