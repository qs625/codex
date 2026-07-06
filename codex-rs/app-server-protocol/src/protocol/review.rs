use super::Turn;
use super::shared::camel_case_enum_from_core;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

camel_case_enum_from_core!(
    pub enum ReviewDelivery from protocol::protocol::ReviewDelivery {
        Inline, Detached
    }
);

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ReviewStartParams {
    pub thread_id: String,
    pub target: ReviewTarget,

    /// Where to run the review: inline (default) on the current thread or
    /// detached on a new thread (returned in `reviewThreadId`).
    #[serde(default)]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub delivery: Option<ReviewDelivery>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ReviewStartResponse {
    pub turn: Turn,
    /// Identifies the thread where the review runs.
    ///
    /// For inline reviews, this is the original thread id.
    /// For detached reviews, this is the id of the new review thread.
    pub review_thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
pub enum ReviewTarget {
    /// Review the working tree: staged, unstaged, and untracked files.
    UncommittedChanges,

    /// Review changes between the current branch and the given base branch.
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    BaseBranch { branch: String },

    /// Review the changes introduced by a specific commit.
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Commit {
        sha: String,
        /// Optional human-readable label (e.g., commit subject) for UIs.
        title: Option<String>,
    },

    /// Arbitrary instructions, equivalent to the old free-form prompt.
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Custom { instructions: String },
}
