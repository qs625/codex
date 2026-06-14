use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename_all = "lowercase", export_to = "v2/")]
pub enum WorkflowSource {
    Home,
    Project,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowInputSpec {
    #[serde(rename = "type")]
    pub input_type: String,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowDetails {
    #[serde(flatten)]
    pub summary: WorkflowSummary,
    pub readme: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowDiagnostic {
    pub source: WorkflowSource,
    pub path: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowRun {
    pub run_id: String,
    pub workflow: WorkflowSummary,
    pub status: WorkflowRunStatus,
    pub runner_status: String,
    #[ts(type = "unknown")]
    pub inputs: JsonValue,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub revision: u64,
    pub message: String,
    pub abort_reason: Option<String>,
    #[ts(type = "unknown")]
    pub output: Option<JsonValue>,
    pub error: Option<String>,
    pub snapshot_path: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
pub enum WorkflowRunStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowListParams {
    #[ts(optional = nullable)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowListResponse {
    pub workflows: Vec<WorkflowSummary>,
    pub diagnostics: Vec<WorkflowDiagnostic>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowDescribeParams {
    pub workflow: String,
    #[ts(optional = nullable)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowDescribeResponse {
    pub workflow: WorkflowDetails,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowStartParams {
    pub workflow: String,
    #[ts(type = "unknown")]
    pub inputs: JsonValue,
    #[ts(optional = nullable)]
    pub cwd: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowStartResponse {
    pub run: WorkflowRun,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowStatusParams {
    pub run_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowStatusResponse {
    pub run: WorkflowRun,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowResumeParams {
    pub run_id: String,
    #[ts(type = "unknown")]
    #[ts(optional = nullable)]
    pub inputs: Option<JsonValue>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowResumeResponse {
    pub run: WorkflowRun,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowAbortParams {
    pub run_id: String,
    #[ts(optional = nullable)]
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowAbortResponse {
    pub run: WorkflowRun,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
pub struct WorkflowRunUpdatedNotification {
    pub run: WorkflowRun,
}
