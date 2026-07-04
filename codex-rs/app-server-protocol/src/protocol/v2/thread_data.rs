use super::CodexErrorInfo;
use super::ThreadItem;
use super::ThreadStatus;
use super::ThreadTokenUsage;
use super::TurnStatus;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::protocol::SessionSource as CoreSessionSource;
use protocol::protocol::SubAgentSource as CoreSubAgentSource;
use protocol::protocol::ThreadContextUsage as CoreThreadContextUsage;
use protocol::protocol::ThreadContextUsageCategoryBreakdown as CoreThreadContextUsageCategoryBreakdown;
use protocol::protocol::ThreadContextUsageLoadedSkills as CoreThreadContextUsageLoadedSkills;
use protocol::protocol::ThreadContextUsageSkill as CoreThreadContextUsageSkill;
use protocol::protocol::ThreadSkill as CoreThreadSkill;
use protocol::protocol::ThreadSkillKind as CoreThreadSkillKind;
use protocol::protocol::ThreadSource as CoreThreadSource;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
#[derive(Default)]
pub enum SessionSource {
    Cli,
    #[serde(rename = "vscode")]
    #[ts(rename = "vscode")]
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case", export_to = "v2/")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct GitInfo {
    pub sha: Option<String>,
    pub branch: Option<String>,
    pub origin_url: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase", export_to = "v2/")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextUsageCategoryBreakdown {
    #[ts(type = "number")]
    pub compact: i64,
    #[ts(type = "number")]
    pub skills_metadata: i64,
    #[ts(type = "number")]
    pub concrete_skills: i64,
    #[ts(type = "number")]
    pub tools_metadata: i64,
    #[ts(type = "number")]
    pub tool_calls: i64,
    #[ts(type = "number")]
    pub user_messages: i64,
    #[ts(type = "number")]
    pub llm_messages: i64,
    #[ts(type = "number")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextUsageSkill {
    pub name: String,
    pub path: String,
    pub kind: ThreadSkillKind,
    #[ts(type = "number")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextUsageLoadedSkills {
    #[ts(type = "number")]
    pub loaded_count: u32,
    #[ts(type = "number | null")]
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ThreadContextUsage {
    #[ts(type = "number")]
    pub total_bytes: i64,
    #[ts(type = "number | null")]
    pub budget_used_percent: Option<i64>,
    pub categories: ThreadContextUsageCategoryBreakdown,
    pub loaded_skills: ThreadContextUsageLoadedSkills,
}

impl From<CoreThreadContextUsage> for ThreadContextUsage {
    fn from(value: CoreThreadContextUsage) -> Self {
        Self {
            total_bytes: value.total_bytes,
            budget_used_percent: value.budget_used_percent,
            categories: value.categories.into(),
            loaded_skills: value.loaded_skills.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
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
    #[ts(type = "number")]
    pub created_at: i64,
    /// Unix timestamp (in seconds) when the thread was last updated.
    #[ts(type = "number")]
    pub updated_at: i64,
    /// Current runtime status for the thread.
    pub status: ThreadStatus,
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
    /// Only populated on `thread/start`, `thread/resume`, `thread/rollback`, `thread/fork`, and
    /// `thread/read` (when `includeTurns` is true) responses.
    /// `thread/start` only includes initial injected context display items.
    /// For all other responses and notifications returning a Thread,
    /// the turns field will be an empty list.
    pub turns: Vec<Turn>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
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
    #[ts(type = "number | null")]
    pub started_at: Option<i64>,
    /// Unix timestamp (in seconds) when the turn completed.
    #[ts(type = "number | null")]
    pub completed_at: Option<i64>,
    /// Duration between turn start and completion in milliseconds, if known.
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub enum TurnItemsView {
    /// `items` was not loaded for this turn. The field is intentionally empty.
    NotLoaded,
    /// `items` contains only a display summary for this turn.
    Summary,
    /// `items` contains every ThreadItem available from persisted app-server history for this turn.
    #[default]
    Full,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
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
