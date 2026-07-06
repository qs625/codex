#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct FeedbackUploadParams {
    pub classification: String,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub reason: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_id: Option<String>,
    pub include_logs: bool,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub extra_log_files: Option<Vec<PathBuf>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub tags: Option<BTreeMap<String, String>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct FeedbackUploadResponse {
    pub thread_id: String,
}
