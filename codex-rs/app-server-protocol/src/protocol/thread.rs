use super::ActivePermissionProfile;
use super::ApprovalsReviewer;
use super::AskForApproval;
use super::PermissionProfile;
use super::PermissionProfileSelectionParams;
use super::SandboxMode;
use super::SandboxPolicy;
use super::Thread;
use super::ThreadContextUsage;
use super::ThreadItem;
use super::ThreadSkill;
use super::ThreadSource;
use super::Turn;
use super::TurnEnvironmentParams;
use super::TurnItemsView;
use super::shared::camel_case_enum_from_core;
use codex_experimental_api_macros::ExperimentalApi;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::config_types::Personality;
use protocol::models::ResponseItem;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::ThreadGoalStatus as CoreThreadGoalStatus;
use protocol::protocol::TokenUsage as CoreTokenUsage;
use protocol::protocol::TokenUsageInfo as CoreTokenUsageInfo;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadStartSource {
    Startup,
    Clear,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct DynamicToolSpec {
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub namespace: Option<String>,
    pub name: String,
    pub description: String,
    pub input_schema: JsonValue,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defer_loading: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicToolSpecDe {
    namespace: Option<String>,
    name: String,
    description: String,
    input_schema: JsonValue,
    defer_loading: Option<bool>,
    expose_to_context: Option<bool>,
}

impl<'de> Deserialize<'de> for DynamicToolSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let DynamicToolSpecDe {
            namespace,
            name,
            description,
            input_schema,
            defer_loading,
            expose_to_context,
        } = DynamicToolSpecDe::deserialize(deserializer)?;

        Ok(Self {
            namespace,
            name,
            description,
            input_schema,
            defer_loading: defer_loading
                .unwrap_or_else(|| expose_to_context.map(|visible| !visible).unwrap_or(false)),
        })
    }
}

// === Threads, Turns, and Items ===
// Thread APIs
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(
    Serialize, Deserialize, Debug, Clone, PartialEq, Default, ExperimentalApi,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadStartParams {
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model_provider: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub service_tier: Option<Option<String>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
    /// Replace the thread's runtime workspace roots. Relative paths are
    /// resolved against the effective cwd for the thread.
    #[experimental("thread/start.runtimeWorkspaceRoots")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub runtime_workspace_roots: Option<Vec<PathBuf>>,
    #[experimental(nested)]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approval_policy: Option<AskForApproval>,
    /// Override where approval requests are routed for review on this thread
    /// and subsequent turns.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sandbox: Option<SandboxMode>,
    /// Named profile id for this thread. Cannot be combined with `sandbox`.
    #[experimental("thread/start.permissions")]
    #[cfg_attr(feature = "schema-export", schemars(with = "Option<String>"))]
    #[cfg_attr(feature = "schema-export", ts(type = "string | null"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub permissions: Option<PermissionProfileSelectionParams>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub config: Option<HashMap<String, JsonValue>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub service_name: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub base_instructions: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub developer_instructions: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub personality: Option<Personality>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub ephemeral: Option<bool>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub session_start_source: Option<ThreadStartSource>,
    /// Optional client-supplied analytics source classification for this thread.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_source: Option<ThreadSource>,
    /// Optional sticky environments for this thread.
    ///
    /// Omitted selects the default environment when environment access is
    /// enabled. Empty disables environment access for turns that do not
    /// provide a turn override. Non-empty selects the first environment as the
    /// current turn environment.
    #[experimental("thread/start.environments")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub environments: Option<Vec<TurnEnvironmentParams>>,
    #[experimental("thread/start.dynamicTools")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub dynamic_tools: Option<Vec<DynamicToolSpec>>,
    /// Test-only experimental field used to validate experimental gating and
    /// schema filtering behavior in a stable way.
    #[experimental("thread/start.mockExperimentalField")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub mock_experimental_field: Option<String>,
    /// Deprecated and ignored by app-server. Kept only so older clients can
    /// continue sending the field while rollout persistence always uses the
    /// limited history policy.
    #[experimental("thread/start.persistFullHistory")]
    #[serde(default)]
    pub persist_extended_history: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MockExperimentalMethodParams {
    /// Test-only payload field.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub value: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MockExperimentalMethodResponse {
    /// Echoes the input `value`.
    pub echoed: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadStartResponse {
    pub thread: Thread,
    pub model: String,
    pub model_provider: String,
    pub service_tier: Option<String>,
    pub cwd: AbsolutePathBuf,
    /// Thread-scoped runtime workspace roots used to materialize
    /// `:workspace_roots`.
    #[experimental("thread/start.runtimeWorkspaceRoots")]
    #[serde(default)]
    pub runtime_workspace_roots: Vec<AbsolutePathBuf>,
    /// Instruction source files currently loaded for this thread.
    #[serde(default)]
    pub instruction_sources: Vec<AbsolutePathBuf>,
    #[experimental(nested)]
    pub approval_policy: AskForApproval,
    /// Reviewer currently used for approval requests on this thread.
    pub approvals_reviewer: ApprovalsReviewer,
    /// Legacy sandbox policy retained for compatibility. Experimental clients
    /// should prefer `permissionProfile` when they need exact runtime
    /// permissions.
    pub sandbox: SandboxPolicy,
    /// Full active permissions for this thread. `activePermissionProfile`
    /// carries display/provenance metadata for this runtime profile.
    #[experimental("thread/start.permissionProfile")]
    #[serde(default)]
    pub permission_profile: Option<PermissionProfile>,
    /// Named or implicit built-in profile that produced the active
    /// permissions, when known.
    #[experimental("thread/start.activePermissionProfile")]
    #[serde(default)]
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(
    Serialize, Deserialize, Debug, Default, Clone, PartialEq, ExperimentalApi,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
/// There are three ways to resume a thread:
/// 1. By thread_id: load the thread from disk by thread_id and resume it.
/// 2. By history: instantiate the thread from memory and resume it.
/// 3. By path: load the thread from disk by path and resume it.
///
/// The precedence is: history > path > thread_id.
/// If using history or path, the thread_id param will be ignored.
///
/// Prefer using thread_id whenever possible.
pub struct ThreadResumeParams {
    pub thread_id: String,

    /// [UNSTABLE] FOR CODEX CLOUD - DO NOT USE.
    /// If specified, the thread will be resumed with the provided history
    /// instead of loaded from disk.
    #[experimental("thread/resume.history")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub history: Option<Vec<ResponseItem>>,

    /// [UNSTABLE] Specify the rollout path to resume from.
    /// If specified, the thread_id param will be ignored.
    #[experimental("thread/resume.path")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub path: Option<PathBuf>,

    /// Configuration overrides for the resumed thread, if any.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model_provider: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub service_tier: Option<Option<String>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
    /// Replace the thread's runtime workspace roots. Relative paths are
    /// resolved against the effective cwd for the thread.
    #[experimental("thread/resume.runtimeWorkspaceRoots")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub runtime_workspace_roots: Option<Vec<PathBuf>>,
    #[experimental(nested)]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approval_policy: Option<AskForApproval>,
    /// Override where approval requests are routed for review on this thread
    /// and subsequent turns.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sandbox: Option<SandboxMode>,
    /// Named profile id for the resumed thread. Cannot be combined with
    /// `sandbox`.
    #[experimental("thread/resume.permissions")]
    #[cfg_attr(feature = "schema-export", schemars(with = "Option<String>"))]
    #[cfg_attr(feature = "schema-export", ts(type = "string | null"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub permissions: Option<PermissionProfileSelectionParams>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub base_instructions: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub developer_instructions: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub personality: Option<Personality>,
    /// When true, return only thread metadata and live-resume state without
    /// populating `thread.turns`. This is useful when the client plans to call
    /// `thread/turns/list` immediately after resuming.
    #[experimental("thread/resume.excludeTurns")]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_turns: bool,
    /// Deprecated and ignored by app-server. Kept only so older clients can
    /// continue sending the field while rollout persistence always uses the
    /// limited history policy.
    #[experimental("thread/resume.persistFullHistory")]
    #[serde(default)]
    pub persist_extended_history: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadResumeResponse {
    pub thread: Thread,
    pub model: String,
    pub model_provider: String,
    pub service_tier: Option<String>,
    pub cwd: AbsolutePathBuf,
    /// Thread-scoped runtime workspace roots used to materialize
    /// `:workspace_roots`.
    #[experimental("thread/resume.runtimeWorkspaceRoots")]
    #[serde(default)]
    pub runtime_workspace_roots: Vec<AbsolutePathBuf>,
    /// Instruction source files currently loaded for this thread.
    #[serde(default)]
    pub instruction_sources: Vec<AbsolutePathBuf>,
    #[experimental(nested)]
    pub approval_policy: AskForApproval,
    /// Reviewer currently used for approval requests on this thread.
    pub approvals_reviewer: ApprovalsReviewer,
    /// Legacy sandbox policy retained for compatibility. Experimental clients
    /// should prefer `permissionProfile` when they need exact runtime
    /// permissions.
    pub sandbox: SandboxPolicy,
    /// Full active permissions for this thread. `activePermissionProfile`
    /// carries display/provenance metadata for this runtime profile.
    #[experimental("thread/resume.permissionProfile")]
    #[serde(default)]
    pub permission_profile: Option<PermissionProfile>,
    /// Named or implicit built-in profile that produced the active
    /// permissions, when known.
    #[experimental("thread/resume.activePermissionProfile")]
    #[serde(default)]
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(
    Serialize, Deserialize, Debug, Default, Clone, PartialEq, ExperimentalApi,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
/// There are two ways to fork a thread:
/// 1. By thread_id: load the thread from disk by thread_id and fork it into a new thread.
/// 2. By path: load the thread from disk by path and fork it into a new thread.
///
/// If using path, the thread_id param will be ignored.
///
/// Prefer using thread_id whenever possible.
pub struct ThreadForkParams {
    pub thread_id: String,

    /// [UNSTABLE] Specify the rollout path to fork from.
    /// If specified, the thread_id param will be ignored.
    #[experimental("thread/fork.path")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub path: Option<PathBuf>,

    /// Configuration overrides for the forked thread, if any.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model_provider: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub service_tier: Option<Option<String>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<String>,
    /// Replace the thread's runtime workspace roots. Relative paths are
    /// resolved against the effective cwd for the thread.
    #[experimental("thread/fork.runtimeWorkspaceRoots")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub runtime_workspace_roots: Option<Vec<PathBuf>>,
    #[experimental(nested)]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approval_policy: Option<AskForApproval>,
    /// Override where approval requests are routed for review on this thread
    /// and subsequent turns.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sandbox: Option<SandboxMode>,
    /// Named profile id for the forked thread. Cannot be combined with
    /// `sandbox`.
    #[experimental("thread/fork.permissions")]
    #[cfg_attr(feature = "schema-export", schemars(with = "Option<String>"))]
    #[cfg_attr(feature = "schema-export", ts(type = "string | null"))]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub permissions: Option<PermissionProfileSelectionParams>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub config: Option<HashMap<String, serde_json::Value>>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub base_instructions: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub developer_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ephemeral: bool,
    /// Optional client-supplied analytics source classification for this forked thread.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_source: Option<ThreadSource>,
    /// When true, return only thread metadata and live fork state without
    /// populating `thread.turns`. This is useful when the client plans to call
    /// `thread/turns/list` immediately after forking.
    #[experimental("thread/fork.excludeTurns")]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_turns: bool,
    /// Deprecated and ignored by app-server. Kept only so older clients can
    /// continue sending the field while rollout persistence always uses the
    /// limited history policy.
    #[experimental("thread/fork.persistFullHistory")]
    #[serde(default)]
    pub persist_extended_history: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadForkResponse {
    pub thread: Thread,
    pub model: String,
    pub model_provider: String,
    pub service_tier: Option<String>,
    pub cwd: AbsolutePathBuf,
    /// Thread-scoped runtime workspace roots used to materialize
    /// `:workspace_roots`.
    #[experimental("thread/fork.runtimeWorkspaceRoots")]
    #[serde(default)]
    pub runtime_workspace_roots: Vec<AbsolutePathBuf>,
    /// Instruction source files currently loaded for this thread.
    #[serde(default)]
    pub instruction_sources: Vec<AbsolutePathBuf>,
    #[experimental(nested)]
    pub approval_policy: AskForApproval,
    /// Reviewer currently used for approval requests on this thread.
    pub approvals_reviewer: ApprovalsReviewer,
    /// Legacy sandbox policy retained for compatibility. Experimental clients
    /// should prefer `permissionProfile` when they need exact runtime
    /// permissions.
    pub sandbox: SandboxPolicy,
    /// Full active permissions for this thread. `activePermissionProfile`
    /// carries display/provenance metadata for this runtime profile.
    #[experimental("thread/fork.permissionProfile")]
    #[serde(default)]
    pub permission_profile: Option<PermissionProfile>,
    /// Named or implicit built-in profile that produced the active
    /// permissions, when known.
    #[experimental("thread/fork.activePermissionProfile")]
    #[serde(default)]
    pub active_permission_profile: Option<ActivePermissionProfile>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadArchiveParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadArchiveResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadUnsubscribeParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadUnsubscribeResponse {
    pub status: ThreadUnsubscribeStatus,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadUnsubscribeStatus {
    NotLoaded,
    NotSubscribed,
    Unsubscribed,
}

/// Parameters for `thread/increment_elicitation`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadIncrementElicitationParams {
    /// Thread whose out-of-band elicitation counter should be incremented.
    pub thread_id: String,
}

/// Response for `thread/increment_elicitation`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadIncrementElicitationResponse {
    /// Current out-of-band elicitation count after the increment.
    pub count: u64,
    /// Whether timeout accounting is paused after applying the increment.
    pub paused: bool,
}

/// Parameters for `thread/decrement_elicitation`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadDecrementElicitationParams {
    /// Thread whose out-of-band elicitation counter should be decremented.
    pub thread_id: String,
}

/// Response for `thread/decrement_elicitation`.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadDecrementElicitationResponse {
    /// Current out-of-band elicitation count after the decrement.
    pub count: u64,
    /// Whether timeout accounting remains paused after applying the decrement.
    pub paused: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadSetNameParams {
    pub thread_id: String,
    pub name: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadUnarchiveParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadSetNameResponse {}

camel_case_enum_from_core! {
    pub enum ThreadGoalStatus from CoreThreadGoalStatus {
        Active,
        Paused,
        BudgetLimited,
        Complete,
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoal {
    pub thread_id: String,
    pub objective: String,
    pub status: ThreadGoalStatus,
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub token_budget: Option<i64>,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub tokens_used: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub time_used_seconds: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub created_at: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub updated_at: i64,
}

impl From<protocol::protocol::ThreadGoal> for ThreadGoal {
    fn from(value: protocol::protocol::ThreadGoal) -> Self {
        Self {
            thread_id: value.thread_id.to_string(),
            objective: value.objective,
            status: value.status.into(),
            token_budget: value.token_budget,
            tokens_used: value.tokens_used,
            time_used_seconds: value.time_used_seconds,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalSetParams {
    pub thread_id: String,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub objective: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub status: Option<ThreadGoalStatus>,
    #[serde(
        default,
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable, type = "number | null"))]
    pub token_budget: Option<Option<i64>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalSetResponse {
    pub goal: ThreadGoal,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalGetParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalGetResponse {
    pub goal: Option<ThreadGoal>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalClearParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalClearResponse {
    pub cleared: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadMetadataUpdateParams {
    pub thread_id: String,
    /// Patch the stored Git metadata for this thread.
    /// Omit a field to leave it unchanged, set it to `null` to clear it, or
    /// provide a string to replace the stored value.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub git_info: Option<ThreadMetadataGitInfoUpdateParams>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadMetadataGitInfoUpdateParams {
    /// Omit to leave the stored commit unchanged, set to `null` to clear it,
    /// or provide a non-empty string to replace it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable, type = "string | null"))]
    pub sha: Option<Option<String>>,
    /// Omit to leave the stored branch unchanged, set to `null` to clear it,
    /// or provide a non-empty string to replace it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable, type = "string | null"))]
    pub branch: Option<Option<String>>,
    /// Omit to leave the stored origin URL unchanged, set to `null` to clear it,
    /// or provide a non-empty string to replace it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::protocol::serde_helpers::serialize_double_option",
        deserialize_with = "crate::protocol::serde_helpers::deserialize_double_option"
    )]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable, type = "string | null"))]
    pub origin_url: Option<Option<String>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadMetadataUpdateResponse {
    pub thread: Thread,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "lowercase"))]
pub enum ThreadMemoryMode {
    Enabled,
    Disabled,
}

impl ThreadMemoryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn to_core(self) -> protocol::protocol::ThreadMemoryMode {
        match self {
            Self::Enabled => protocol::protocol::ThreadMemoryMode::Enabled,
            Self::Disabled => protocol::protocol::ThreadMemoryMode::Disabled,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadMemoryModeSetParams {
    pub thread_id: String,
    pub mode: ThreadMemoryMode,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadMemoryModeSetResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MemoryResetResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadUnarchiveResponse {
    pub thread: Thread,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadCompactStartParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadCompactStartResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadShellCommandParams {
    pub thread_id: String,
    /// Shell command string evaluated by the thread's configured shell.
    /// Unlike `command/exec`, this intentionally preserves shell syntax
    /// such as pipes, redirects, and quoting. This runs unsandboxed with full
    /// access rather than inheriting the thread sandbox policy.
    pub command: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadShellCommandResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadApproveGuardianDeniedActionParams {
    pub thread_id: String,
    /// Serialized `protocol::protocol::GuardianAssessmentEvent`.
    pub event: JsonValue,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadApproveGuardianDeniedActionResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadBackgroundTerminalsCleanParams {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadBackgroundTerminalsCleanResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadRollbackParams {
    pub thread_id: String,
    /// The number of turns to drop from the end of the thread. Must be >= 1.
    ///
    /// This only modifies the thread's history and does not revert local file changes
    /// that have been made by the agent. Clients are responsible for reverting these changes.
    pub num_turns: u32,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadRollbackResponse {
    /// The updated thread after applying the rollback, with `turns` populated.
    ///
    /// The ThreadItems stored in each Turn are lossy since we explicitly do not
    /// persist all agent interactions, such as command executions. This is the same
    /// behavior as `thread/resume`.
    pub thread: Thread,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional page size; defaults to a reasonable server-side value.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
    /// Optional sort key; defaults to created_at.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sort_key: Option<ThreadSortKey>,
    /// Optional sort direction; defaults to descending (newest first).
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sort_direction: Option<SortDirection>,
    /// Optional provider filter; when set, only sessions recorded under these
    /// providers are returned. When present but empty, includes all providers.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub model_providers: Option<Vec<String>>,
    /// Optional source filter; when set, only sessions from these source kinds
    /// are returned. When omitted or empty, defaults to interactive sources.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub source_kinds: Option<Vec<ThreadSourceKind>>,
    /// Optional archived filter; when set to true, only archived threads are returned.
    /// If false or null, only non-archived threads are returned.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub archived: Option<bool>,
    /// Optional cwd filter or filters; when set, only threads whose session cwd
    /// exactly matches one of these paths are returned.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable, type = "string | Array<string> | null"))]
    pub cwd: Option<ThreadListCwdFilter>,
    /// If true, return from the state DB without scanning JSONL rollouts to
    /// repair thread metadata. Omitted or false preserves scan-and-repair
    /// behavior.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_state_db_only: bool,
    /// Optional substring filter for the extracted thread title.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub search_term: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum ThreadListCwdFilter {
    One(String),
    Many(Vec<String>),
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadSourceKind {
    Cli,
    #[serde(rename = "vscode")]
    #[cfg_attr(feature = "schema-export", ts(rename = "vscode"))]
    VsCode,
    Exec,
    AppServer,
    SubAgent,
    SubAgentReview,
    SubAgentCompact,
    SubAgentThreadSpawn,
    SubAgentOther,
    Unknown,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadSortKey {
    CreatedAt,
    UpdatedAt,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum SortDirection {
    Asc,
    Desc,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadListResponse {
    pub data: Vec<Thread>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// if None, there are no more items to return.
    pub next_cursor: Option<String>,
    /// Opaque cursor to pass as `cursor` when reversing `sortDirection`.
    /// This is only populated when the page contains at least one thread.
    /// Use it with the opposite `sortDirection`; for timestamp sorts it anchors
    /// at the start of the page timestamp so same-second updates are not skipped.
    pub backwards_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadLoadedListParams {
    /// Opaque pagination cursor returned by a previous call.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional page size; defaults to no limit.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadLoadedListResponse {
    /// Thread ids for sessions currently loaded in memory.
    pub data: Vec<String>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// if None, there are no more items to return.
    pub next_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadStatus {
    NotLoaded,
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Idle {
        reason: ThreadIdleReason,
    },
    Complete,
    SystemError,
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Active {
        active_flags: Vec<ThreadActiveFlag>,
    },
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadIdleReason {
    WaitCommand,
    WaitChild,
    WaitEventSubscription,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadActiveFlag {
    Running,
    WaitingOnApproval,
    WaitingOnUserInput,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadReadParams {
    pub thread_id: String,
    /// When true, include turns and their items from rollout history.
    #[serde(default)]
    pub include_turns: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadReadResponse {
    pub thread: Thread,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadInjectItemsParams {
    pub thread_id: String,
    /// Raw Responses API items to append to the thread's model-visible history.
    pub items: Vec<JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadInjectItemsResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTurnsListParams {
    pub thread_id: String,
    /// Opaque cursor to pass to the next call to continue after the last turn.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional turn page size.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
    /// Optional turn pagination direction; defaults to descending.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sort_direction: Option<SortDirection>,
    /// How much item detail to include for each returned turn; defaults to summary.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub items_view: Option<TurnItemsView>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTurnsListResponse {
    pub data: Vec<Turn>,
    /// Opaque cursor to pass to the next call to continue after the last turn.
    /// if None, there are no more turns to return.
    pub next_cursor: Option<String>,
    /// Opaque cursor to pass as `cursor` when reversing `sortDirection`.
    /// This is only populated when the page contains at least one turn.
    /// Use it with the opposite `sortDirection` to include the anchor turn again
    /// and catch updates to that turn.
    pub backwards_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTurnsItemsListParams {
    pub thread_id: String,
    pub turn_id: String,
    /// Opaque cursor to pass to the next call to continue after the last item.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional item page size.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
    /// Optional item pagination direction; defaults to ascending.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub sort_direction: Option<SortDirection>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTurnsItemsListResponse {
    pub data: Vec<ThreadItem>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// if None, there are no more items to return.
    pub next_cursor: Option<String>,
    /// Opaque cursor to pass as `cursor` when reversing `sortDirection`.
    /// This is only populated when the page contains at least one item.
    pub backwards_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTokenUsageUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: ThreadTokenUsage,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: ThreadTokenUsage,
    pub context_usage: ThreadContextUsage,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadTokenUsage {
    pub total: TokenUsageBreakdown,
    pub last: TokenUsageBreakdown,
    // TODO(aibrahim): make this not optional
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub model_context_window: Option<i64>,
}

impl From<CoreTokenUsageInfo> for ThreadTokenUsage {
    fn from(value: CoreTokenUsageInfo) -> Self {
        Self {
            total: value.total_token_usage.into(),
            last: value.last_token_usage.into(),
            model_context_window: value.model_context_window,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TokenUsageBreakdown {
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub total_tokens: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub input_tokens: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub cached_input_tokens: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub output_tokens: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub reasoning_output_tokens: i64,
}

impl From<CoreTokenUsage> for TokenUsageBreakdown {
    fn from(value: CoreTokenUsage) -> Self {
        Self {
            total_tokens: value.total_tokens,
            input_tokens: value.input_tokens,
            cached_input_tokens: value.cached_input_tokens,
            output_tokens: value.output_tokens,
            reasoning_output_tokens: value.reasoning_output_tokens,
        }
    }
}

// Thread/Turn lifecycle notifications and item progress events
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadStartedNotification {
    pub thread: Thread,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadStatusChangedNotification {
    pub thread_id: String,
    pub status: ThreadStatus,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadArchivedNotification {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadUnarchivedNotification {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadClosedNotification {
    pub thread_id: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadSkillsUpdatedNotification {
    pub thread_id: String,
    pub skills: Vec<ThreadSkill>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadNameUpdatedNotification {
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub thread_name: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalUpdatedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub goal: ThreadGoal,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadGoalClearedNotification {
    pub thread_id: String,
}

/// Deprecated: Use `ContextCompaction` item type instead.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ContextCompactedNotification {
    pub thread_id: String,
    pub turn_id: String,
}
