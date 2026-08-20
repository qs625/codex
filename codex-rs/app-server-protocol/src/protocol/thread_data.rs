use super::CodexErrorInfo;
use super::ThreadItem;
use super::ThreadLifecycleStatus;
use super::ThreadTokenUsage;
use super::TurnStatus;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::protocol::SessionSource as CoreSessionSource;
use protocol::protocol::SubAgentSource as CoreSubAgentSource;
use protocol::protocol::ThreadContextUsage as CoreThreadContextUsage;
use protocol::protocol::ThreadContextUsageCategoryBreakdown as CoreThreadContextUsageCategoryBreakdown;
use protocol::protocol::ThreadContextUsageLoadedSkills as CoreThreadContextUsageLoadedSkills;
use protocol::protocol::ThreadContextUsageSkill as CoreThreadContextUsageSkill;
use protocol::protocol::ThreadContextUsageToolBreakdown as CoreThreadContextUsageToolBreakdown;
use protocol::protocol::ThreadContextUsageToolBucket as CoreThreadContextUsageToolBucket;
use protocol::protocol::ThreadSkill as CoreThreadSkill;
use protocol::protocol::ThreadSkillKind as CoreThreadSkillKind;
use protocol::protocol::ThreadSource as CoreThreadSource;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
#[derive(Default)]
pub enum SessionSource {
    Cli,
    #[serde(rename = "vscode")]
    #[cfg_attr(feature = "schema-export", ts(rename = "vscode"))]
    #[default]
    VsCode,
    Exec,
    AppServer,
    Custom(String),
    SubAgent(CoreSubAgentSource),
    #[serde(other)]
    Unknown,
}

impl From<CoreSessionSource> for SessionSource {
    fn from(value: CoreSessionSource) -> Self {
        match value {
            CoreSessionSource::Cli => SessionSource::Cli,
            CoreSessionSource::VSCode => SessionSource::VsCode,
            CoreSessionSource::Exec => SessionSource::Exec,
            CoreSessionSource::Mcp => SessionSource::AppServer,
            CoreSessionSource::Custom(source) => SessionSource::Custom(source),
            // We do not want to render those at the app-server level.
            CoreSessionSource::Internal(_) => SessionSource::Unknown,
            CoreSessionSource::SubAgent(sub) => SessionSource::SubAgent(sub),
            CoreSessionSource::Unknown => SessionSource::Unknown,
        }
    }
}

impl From<SessionSource> for CoreSessionSource {
    fn from(value: SessionSource) -> Self {
        match value {
            SessionSource::Cli => CoreSessionSource::Cli,
            SessionSource::VsCode => CoreSessionSource::VSCode,
            SessionSource::Exec => CoreSessionSource::Exec,
            SessionSource::AppServer => CoreSessionSource::Mcp,
            SessionSource::Custom(source) => CoreSessionSource::Custom(source),
            SessionSource::SubAgent(sub) => CoreSessionSource::SubAgent(sub),
            SessionSource::Unknown => CoreSessionSource::Unknown,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "snake_case"))]
pub enum ThreadSource {
    User,
    Subagent,
    MemoryConsolidation,
}

impl From<CoreThreadSource> for ThreadSource {
    fn from(value: CoreThreadSource) -> Self {
        match value {
            CoreThreadSource::User => ThreadSource::User,
            CoreThreadSource::Subagent => ThreadSource::Subagent,
            CoreThreadSource::MemoryConsolidation => ThreadSource::MemoryConsolidation,
        }
    }
}

impl From<ThreadSource> for CoreThreadSource {
    fn from(value: ThreadSource) -> Self {
        match value {
            ThreadSource::User => CoreThreadSource::User,
            ThreadSource::Subagent => CoreThreadSource::Subagent,
            ThreadSource::MemoryConsolidation => CoreThreadSource::MemoryConsolidation,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct GitInfo {
    pub sha: Option<String>,
    pub branch: Option<String>,
    pub origin_url: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadSkillKind {
    Explicit,
    Implicit,
    All,
}

impl From<CoreThreadSkillKind> for ThreadSkillKind {
    fn from(value: CoreThreadSkillKind) -> Self {
        match value {
            CoreThreadSkillKind::Explicit => Self::Explicit,
            CoreThreadSkillKind::Implicit => Self::Implicit,
            CoreThreadSkillKind::All => Self::All,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadSkill {
    pub name: String,
    pub path: String,
    pub kind: ThreadSkillKind,
}

impl From<CoreThreadSkill> for ThreadSkill {
    fn from(value: CoreThreadSkill) -> Self {
        Self {
            name: value.name,
            path: value.path,
            kind: value.kind.into(),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageCategoryBreakdown {
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub compact: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub skills_metadata: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub concrete_skills: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub tools_metadata: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub tool_calls: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub user_messages: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub llm_messages: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub reasoning: i64,
}

impl From<CoreThreadContextUsageCategoryBreakdown> for ThreadContextUsageCategoryBreakdown {
    fn from(value: CoreThreadContextUsageCategoryBreakdown) -> Self {
        Self {
            compact: value.compact,
            skills_metadata: value.skills_metadata,
            concrete_skills: value.concrete_skills,
            tools_metadata: value.tools_metadata,
            tool_calls: value.tool_calls,
            user_messages: value.user_messages,
            llm_messages: value.llm_messages,
            reasoning: value.reasoning,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageSkill {
    pub name: String,
    pub path: String,
    pub kind: ThreadSkillKind,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub load_count: u32,
}

impl From<CoreThreadContextUsageSkill> for ThreadContextUsageSkill {
    fn from(value: CoreThreadContextUsageSkill) -> Self {
        Self {
            name: value.name,
            path: value.path,
            kind: value.kind.into(),
            load_count: value.load_count,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageLoadedSkills {
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub loaded_count: u32,
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub total_count: Option<u32>,
    pub skills: Vec<ThreadContextUsageSkill>,
}

impl From<CoreThreadContextUsageLoadedSkills> for ThreadContextUsageLoadedSkills {
    fn from(value: CoreThreadContextUsageLoadedSkills) -> Self {
        Self {
            loaded_count: value.loaded_count,
            total_count: value.total_count,
            skills: value.skills.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Default, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageToolBucket {
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub input: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub output: i64,
}

impl From<CoreThreadContextUsageToolBucket> for ThreadContextUsageToolBucket {
    fn from(value: CoreThreadContextUsageToolBucket) -> Self {
        Self {
            input: value.input,
            output: value.output,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Default, Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsageToolBreakdown {
    pub apply_patch: ThreadContextUsageToolBucket,
    pub file_operations: ThreadContextUsageToolBucket,
    pub commands: ThreadContextUsageToolBucket,
    pub inter_agent: ThreadContextUsageToolBucket,
    pub search_media: ThreadContextUsageToolBucket,
    pub other_tools: ThreadContextUsageToolBucket,
}

impl From<CoreThreadContextUsageToolBreakdown> for ThreadContextUsageToolBreakdown {
    fn from(value: CoreThreadContextUsageToolBreakdown) -> Self {
        Self {
            apply_patch: value.apply_patch.into(),
            file_operations: value.file_operations.into(),
            commands: value.commands.into(),
            inter_agent: value.inter_agent.into(),
            search_media: value.search_media.into(),
            other_tools: value.other_tools.into(),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadContextUsage {
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub total_bytes: i64,
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub budget_used_percent: Option<i64>,
    pub categories: ThreadContextUsageCategoryBreakdown,
    pub loaded_skills: ThreadContextUsageLoadedSkills,
    #[serde(default)]
    pub tool_breakdown: ThreadContextUsageToolBreakdown,
}

impl From<CoreThreadContextUsage> for ThreadContextUsage {
    fn from(value: CoreThreadContextUsage) -> Self {
        Self {
            total_bytes: value.total_bytes,
            budget_used_percent: value.budget_used_percent,
            categories: value.categories.into(),
            loaded_skills: value.loaded_skills.into(),
            tool_breakdown: value.tool_breakdown.into(),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct Thread {
    pub id: String,
    /// Session id shared by threads that belong to the same session tree.
    pub session_id: String,
    /// Source thread id when this thread was created by forking another thread.
    pub forked_from_id: Option<String>,
    /// Usually the first user message in the thread, if available.
    pub preview: String,
    /// Whether the thread is ephemeral and should not be materialized on disk.
    pub ephemeral: bool,
    /// Model provider used for this thread (for example, 'openai').
    pub model_provider: String,
    /// Unix timestamp (in seconds) when the thread was created.
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub created_at: i64,
    /// Unix timestamp (in seconds) when the thread was last updated.
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub updated_at: i64,
    /// Current runtime lifecycle status for the thread.
    pub lifecycle_status: ThreadLifecycleStatus,
    /// [UNSTABLE] Path to the thread on disk.
    pub path: Option<PathBuf>,
    /// Working directory captured for the thread.
    pub cwd: AbsolutePathBuf,
    /// Version of the CLI that created the thread.
    pub cli_version: String,
    /// Origin of the thread (CLI, VSCode, codex exec, codex app-server, etc.).
    pub source: SessionSource,
    /// Optional analytics source classification for this thread.
    pub thread_source: Option<ThreadSource>,
    /// Optional random unique nickname assigned to an AgentControl-spawned sub-agent.
    pub agent_nickname: Option<String>,
    /// Optional role (agent_role) assigned to an AgentControl-spawned sub-agent.
    pub agent_role: Option<String>,
    /// Optional canonical agent path assigned to this thread.
    pub agent_path: Option<String>,
    /// Optional Git metadata captured when the thread was created.
    pub git_info: Option<GitInfo>,
    /// Optional user-facing thread title.
    pub name: Option<String>,
    /// Aggregate thread-level skill usage observed so far.
    #[serde(default)]
    pub skills: Vec<ThreadSkill>,
    /// Restored aggregate thread token usage, when available.
    pub token_usage: Option<ThreadTokenUsage>,
    /// Restored aggregate thread context usage, when available.
    pub context_usage: Option<ThreadContextUsage>,
    /// Populated only on responses that explicitly include display history, such as
    /// `thread/resume`, `thread/rollback`, `thread/fork`, and
    /// `thread/read` (when `includeTurns` is true).
    /// For `thread/start`, `thread/started`, and other metadata-only Thread payloads,
    /// the turns field will be an empty list.
    pub turns: Vec<Turn>,
    /// Current active subscription display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub active_subscription_items: Option<Vec<ThreadItem>>,
    /// Current command display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub active_command_items: Option<Vec<ThreadItem>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct Turn {
    pub id: String,
    /// Thread items currently included in this turn payload.
    pub items: Vec<ThreadItem>,
    /// Describes how much of `items` has been loaded for this turn.
    #[serde(default)]
    pub items_view: TurnItemsView,
    pub status: TurnStatus,
    /// Only populated when the Turn's status is failed.
    pub error: Option<TurnError>,
    /// Unix timestamp (in seconds) when the turn started.
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub started_at: Option<i64>,
    /// Unix timestamp (in seconds) when the turn completed.
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub completed_at: Option<i64>,
    /// Duration between turn start and completion in milliseconds, if known.
    #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
    pub duration_ms: Option<i64>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum TurnItemsView {
    /// `items` was not loaded for this turn. The field is intentionally empty.
    NotLoaded,
    /// `items` contains only a display summary for this turn.
    Summary,
    /// `items` contains every ThreadItem available from persisted app-server history for this turn.
    #[default]
    Full,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct TurnError {
    pub message: String,
    pub codex_error_info: Option<CodexErrorInfo>,
    #[serde(default)]
    pub additional_details: Option<String>,
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TurnError {}
