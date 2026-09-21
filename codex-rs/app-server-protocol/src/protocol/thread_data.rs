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
use schemars::JsonSchema;
#[cfg(feature = "schema-export")]
use schemars::r#gen::SchemaGenerator;
#[cfg(feature = "schema-export")]
use schemars::schema::Schema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de;
use serde::ser::SerializeStruct;
#[cfg(feature = "schema-export")]
use std::borrow::Cow;
use std::fmt;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
use ts_rs::TS;
#[cfg(feature = "schema-export")]
use ts_rs::TypeVisitor;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
#[derive(Default)]
pub enum ThreadOrigin {
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

/// Legacy API name retained for compatibility. New app-server protocol code
/// should use [`ThreadOrigin`] for Morpheus thread provenance.
pub type SessionSource = ThreadOrigin;

impl From<CoreSessionSource> for ThreadOrigin {
    fn from(value: CoreSessionSource) -> Self {
        match value {
            CoreSessionSource::Cli => ThreadOrigin::Cli,
            CoreSessionSource::VSCode => ThreadOrigin::VsCode,
            CoreSessionSource::Exec => ThreadOrigin::Exec,
            CoreSessionSource::Mcp => ThreadOrigin::AppServer,
            CoreSessionSource::Custom(source) => ThreadOrigin::Custom(source),
            // We do not want to render those at the app-server level.
            CoreSessionSource::Internal(_) => ThreadOrigin::Unknown,
            CoreSessionSource::SubAgent(sub) => ThreadOrigin::SubAgent(sub),
            CoreSessionSource::Unknown => ThreadOrigin::Unknown,
        }
    }
}

impl From<ThreadOrigin> for CoreSessionSource {
    fn from(value: ThreadOrigin) -> Self {
        match value {
            ThreadOrigin::Cli => CoreSessionSource::Cli,
            ThreadOrigin::VsCode => CoreSessionSource::VSCode,
            ThreadOrigin::Exec => CoreSessionSource::Exec,
            ThreadOrigin::AppServer => CoreSessionSource::Mcp,
            ThreadOrigin::Custom(source) => CoreSessionSource::Custom(source),
            ThreadOrigin::SubAgent(sub) => CoreSessionSource::SubAgent(sub),
            ThreadOrigin::Unknown => CoreSessionSource::Unknown,
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ThreadStats {
    /// Total number of context compactions observed in the persisted thread history.
    pub compaction_count: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub id: String,
    /// Legacy runtime attachment/thread-tree id. New clients should use
    /// `threadId`/`id` for Morpheus thread identity; this compatibility field
    /// only remains for older renderers and runtime attachment plumbing.
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
    pub created_at: i64,
    /// Unix timestamp (in seconds) when the thread was last updated.
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
    pub source: ThreadOrigin,
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
    pub skills: Vec<ThreadSkill>,
    /// Restored aggregate thread token usage, when available.
    pub token_usage: Option<ThreadTokenUsage>,
    /// Restored aggregate thread context usage, when available.
    pub context_usage: Option<ThreadContextUsage>,
    /// Bounded thread-level historical statistics restored from persisted history.
    pub stats: Option<ThreadStats>,
    /// Populated only on responses that explicitly include display history, such as
    /// `thread/resume`, `thread/rollback`, `thread/fork`, and
    /// `thread/read` (when `includeTurns` is true).
    /// For `thread/start`, `thread/started`, and other metadata-only Thread payloads,
    /// the turns field will be an empty list.
    pub turns: Vec<Turn>,
    /// Current active subscription display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    pub active_subscription_items: Option<Vec<ThreadItem>>,
    /// Current command display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    pub active_command_items: Option<Vec<ThreadItem>>,
}

#[cfg(feature = "schema-export")]
#[derive(JsonSchema, TS)]
#[schemars(rename = "Thread")]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
#[allow(dead_code)]
struct ThreadExport {
    id: String,
    /// Canonical Morpheus thread identity. Written alongside legacy `id`.
    thread_id: String,
    /// Legacy runtime attachment/thread-tree id. New clients should use
    /// `runtimeSessionId`; this compatibility field remains for older renderers
    /// and runtime attachment plumbing.
    session_id: String,
    /// Canonical runtime attachment/thread-tree id. Written alongside legacy
    /// `sessionId`.
    runtime_session_id: String,
    /// Source thread id when this thread was created by forking another thread.
    forked_from_id: Option<String>,
    /// Usually the first user message in the thread, if available.
    preview: String,
    /// Whether the thread is ephemeral and should not be materialized on disk.
    ephemeral: bool,
    /// Model provider used for this thread (for example, 'openai').
    model_provider: String,
    /// Unix timestamp (in seconds) when the thread was created.
    #[ts(type = "number")]
    created_at: i64,
    /// Unix timestamp (in seconds) when the thread was last updated.
    #[ts(type = "number")]
    updated_at: i64,
    /// Current runtime lifecycle status for the thread.
    lifecycle_status: ThreadLifecycleStatus,
    /// [UNSTABLE] Path to the thread on disk.
    path: Option<PathBuf>,
    /// Working directory captured for the thread.
    cwd: AbsolutePathBuf,
    /// Version of the CLI that created the thread.
    cli_version: String,
    /// Origin of the thread (CLI, VSCode, codex exec, codex app-server, etc.).
    source: ThreadOrigin,
    /// Optional analytics source classification for this thread.
    thread_source: Option<ThreadSource>,
    /// Optional random unique nickname assigned to an AgentControl-spawned sub-agent.
    agent_nickname: Option<String>,
    /// Optional role (agent_role) assigned to an AgentControl-spawned sub-agent.
    agent_role: Option<String>,
    /// Optional canonical agent path assigned to this thread.
    agent_path: Option<String>,
    /// Optional Git metadata captured when the thread was created.
    git_info: Option<GitInfo>,
    /// Optional user-facing thread title.
    name: Option<String>,
    /// Aggregate thread-level skill usage observed so far.
    #[serde(default)]
    skills: Vec<ThreadSkill>,
    /// Restored aggregate thread token usage, when available.
    token_usage: Option<ThreadTokenUsage>,
    /// Restored aggregate thread context usage, when available.
    context_usage: Option<ThreadContextUsage>,
    /// Bounded thread-level historical statistics restored from persisted history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    stats: Option<ThreadStats>,
    /// Populated only on responses that explicitly include display history, such as
    /// `thread/resume`, `thread/rollback`, `thread/fork`, and
    /// `thread/read` (when `includeTurns` is true).
    /// For `thread/start`, `thread/started`, and other metadata-only Thread payloads,
    /// the turns field will be an empty list.
    turns: Vec<Turn>,
    /// Current active subscription display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    active_subscription_items: Option<Vec<ThreadItem>>,
    /// Current command display facts restored from persisted activity events.
    /// These are intentionally kept out of `turns`, which represents ordinary
    /// conversation history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    active_command_items: Option<Vec<ThreadItem>>,
}

#[cfg(feature = "schema-export")]
impl JsonSchema for Thread {
    fn schema_name() -> String {
        "Thread".to_string()
    }

    fn schema_id() -> Cow<'static, str> {
        Cow::Borrowed("app_server_protocol::protocol::thread_data::Thread")
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        ThreadExport::json_schema(generator)
    }
}

#[cfg(feature = "schema-export")]
impl TS for Thread {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn ident() -> String {
        "Thread".to_string()
    }

    fn name() -> String {
        "Thread".to_string()
    }

    fn decl() -> String {
        format!("type Thread = {};", ThreadExport::inline())
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn inline() -> String {
        Self::name()
    }

    fn inline_flattened() -> String {
        ThreadExport::inline_flattened()
    }

    fn visit_dependencies(v: &mut impl TypeVisitor)
    where
        Self: 'static,
    {
        ThreadExport::visit_dependencies(v);
    }

    fn output_path() -> Option<PathBuf> {
        Some(PathBuf::from("Thread.ts"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWire {
    id: Option<String>,
    thread_id: Option<String>,
    session_id: Option<String>,
    runtime_session_id: Option<String>,
    forked_from_id: Option<String>,
    preview: String,
    ephemeral: bool,
    model_provider: String,
    created_at: i64,
    updated_at: i64,
    lifecycle_status: ThreadLifecycleStatus,
    path: Option<PathBuf>,
    cwd: AbsolutePathBuf,
    cli_version: String,
    source: ThreadOrigin,
    thread_source: Option<ThreadSource>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    agent_path: Option<String>,
    git_info: Option<GitInfo>,
    name: Option<String>,
    #[serde(default)]
    skills: Vec<ThreadSkill>,
    token_usage: Option<ThreadTokenUsage>,
    context_usage: Option<ThreadContextUsage>,
    #[serde(default)]
    stats: Option<ThreadStats>,
    turns: Vec<Turn>,
    #[serde(default)]
    active_subscription_items: Option<Vec<ThreadItem>>,
    #[serde(default)]
    active_command_items: Option<Vec<ThreadItem>>,
}

impl<'de> Deserialize<'de> for Thread {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ThreadWire::deserialize(deserializer)?;
        let id = reconcile_identity_field(wire.id, wire.thread_id, "id", "threadId")?;
        let session_id = reconcile_identity_field(
            wire.session_id,
            wire.runtime_session_id,
            "sessionId",
            "runtimeSessionId",
        )?;

        Ok(Self {
            id,
            session_id,
            forked_from_id: wire.forked_from_id,
            preview: wire.preview,
            ephemeral: wire.ephemeral,
            model_provider: wire.model_provider,
            created_at: wire.created_at,
            updated_at: wire.updated_at,
            lifecycle_status: wire.lifecycle_status,
            path: wire.path,
            cwd: wire.cwd,
            cli_version: wire.cli_version,
            source: wire.source,
            thread_source: wire.thread_source,
            agent_nickname: wire.agent_nickname,
            agent_role: wire.agent_role,
            agent_path: wire.agent_path,
            git_info: wire.git_info,
            name: wire.name,
            skills: wire.skills,
            token_usage: wire.token_usage,
            context_usage: wire.context_usage,
            stats: wire.stats,
            turns: wire.turns,
            active_subscription_items: wire.active_subscription_items,
            active_command_items: wire.active_command_items,
        })
    }
}

fn reconcile_identity_field<E>(
    legacy: Option<String>,
    canonical: Option<String>,
    legacy_name: &'static str,
    canonical_name: &'static str,
) -> Result<String, E>
where
    E: de::Error,
{
    match (legacy, canonical) {
        (Some(legacy), Some(canonical)) if legacy == canonical => Ok(canonical),
        (Some(legacy), Some(canonical)) => Err(E::custom(format!(
            "conflicting {legacy_name} and {canonical_name} values: {legacy:?} != {canonical:?}"
        ))),
        (Some(legacy), None) => Ok(legacy),
        (None, Some(canonical)) => Ok(canonical),
        (None, None) => Err(E::missing_field(canonical_name)),
    }
}

impl Serialize for Thread {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("Thread", 36)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("threadId", &self.id)?;
        state.serialize_field("sessionId", &self.session_id)?;
        state.serialize_field("runtimeSessionId", &self.session_id)?;
        state.serialize_field("forkedFromId", &self.forked_from_id)?;
        state.serialize_field("preview", &self.preview)?;
        state.serialize_field("ephemeral", &self.ephemeral)?;
        state.serialize_field("modelProvider", &self.model_provider)?;
        state.serialize_field("createdAt", &self.created_at)?;
        state.serialize_field("updatedAt", &self.updated_at)?;
        state.serialize_field("lifecycleStatus", &self.lifecycle_status)?;
        state.serialize_field("path", &self.path)?;
        state.serialize_field("cwd", &self.cwd)?;
        state.serialize_field("cliVersion", &self.cli_version)?;
        state.serialize_field("source", &self.source)?;
        state.serialize_field("threadSource", &self.thread_source)?;
        state.serialize_field("agentNickname", &self.agent_nickname)?;
        state.serialize_field("agentRole", &self.agent_role)?;
        state.serialize_field("agentPath", &self.agent_path)?;
        state.serialize_field("gitInfo", &self.git_info)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("skills", &self.skills)?;
        state.serialize_field("tokenUsage", &self.token_usage)?;
        state.serialize_field("contextUsage", &self.context_usage)?;
        if let Some(stats) = &self.stats {
            state.serialize_field("stats", stats)?;
        }
        state.serialize_field("turns", &self.turns)?;
        if let Some(items) = &self.active_subscription_items {
            state.serialize_field("activeSubscriptionItems", items)?;
        }
        if let Some(items) = &self.active_command_items {
            state.serialize_field("activeCommandItems", items)?;
        }
        state.end()
    }
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
