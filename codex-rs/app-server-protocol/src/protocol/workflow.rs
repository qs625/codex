#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "lowercase"))]
pub enum WorkflowSource {
    Home,
    Project,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowInputSpec {
    #[serde(rename = "type")]
    pub input_type: String,
    pub description: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: WorkflowSource,
    pub path: String,
    pub entry: String,
    pub version: Option<String>,
    pub when_to_use: Vec<String>,
    pub inputs: BTreeMap<String, WorkflowInputSpec>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowDetails {
    #[serde(flatten)]
    pub summary: WorkflowSummary,
    pub instructions: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowDiagnostic {
    pub source: WorkflowSource,
    pub path: String,
    pub message: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow: WorkflowSummary,
    pub status: WorkflowRunStatus,
    pub runner_status: String,
    #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
    pub inputs: JsonValue,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub created_at: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub updated_at: i64,
    pub revision: u64,
    pub message: String,
    pub abort_reason: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
    pub output: Option<JsonValue>,
    pub error: Option<String>,
    pub snapshot_path: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "snake_case"))]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowListParams {
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowListResponse {
    pub workflows: Vec<WorkflowSummary>,
    pub diagnostics: Vec<WorkflowDiagnostic>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowDescribeParams {
    pub workflow: String,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowDescribeResponse {
    pub workflow: WorkflowDetails,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowStartParams {
    pub workflow: String,
    #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
    pub inputs: JsonValue,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowStartResponse {
    pub run: WorkflowRun,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowStatusParams {
    pub run_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowStatusResponse {
    pub run: WorkflowRun,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowResumeParams {
    pub run_id: String,
    #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub inputs: Option<JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowResumeResponse {
    pub run: WorkflowRun,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowAbortParams {
    pub run_id: String,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub reason: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowAbortResponse {
    pub run: WorkflowRun,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct WorkflowRunUpdatedNotification {
    pub run: WorkflowRun,
}
