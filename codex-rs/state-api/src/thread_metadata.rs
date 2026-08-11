use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AskForApproval;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionSource;
use protocol::protocol::ThreadSource;
use protocol::subscriptions::PersistedSubscription;
use serde::Serialize;
use std::path::PathBuf;

/// The sort key to use when listing threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    /// Sort by the thread's creation timestamp.
    CreatedAt,
    /// Sort by the thread's last update timestamp.
    UpdatedAt,
}

/// Sort direction to use when listing threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// A pagination anchor used for keyset pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// The timestamp component of the anchor.
    pub ts: DateTime<Utc>,
}

/// A single page of thread metadata results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadsPage {
    /// The thread metadata items in this page.
    pub items: Vec<ThreadMetadata>,
    /// The next anchor to use for pagination, if any.
    pub next_anchor: Option<Anchor>,
    /// The number of rows scanned to produce this page.
    pub num_scanned_rows: usize,
}

/// The outcome of extracting metadata from a rollout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionOutcome {
    /// The extracted thread metadata.
    pub metadata: ThreadMetadata,
    /// The explicit thread memory mode from rollout metadata, if present.
    pub memory_mode: Option<String>,
    /// The number of rollout lines that failed to parse.
    pub parse_errors: usize,
}

/// Canonical thread metadata derived from rollout files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetadata {
    /// The thread identifier.
    pub id: ThreadId,
    /// The absolute rollout path on disk.
    pub rollout_path: PathBuf,
    /// The creation timestamp.
    pub created_at: DateTime<Utc>,
    /// The last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// The session source (stringified enum).
    pub source: String,
    /// Optional analytics source classification for this thread.
    pub thread_source: Option<ThreadSource>,
    /// Optional random unique nickname assigned to an AgentControl-spawned sub-agent.
    pub agent_nickname: Option<String>,
    /// Optional role (agent_role) assigned to an AgentControl-spawned sub-agent.
    pub agent_role: Option<String>,
    /// Optional canonical agent path assigned to an AgentControl-spawned sub-agent.
    pub agent_path: Option<String>,
    /// The model provider identifier.
    pub model_provider: String,
    /// The latest observed model for the thread.
    pub model: Option<String>,
    /// The latest observed reasoning effort for the thread.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// The working directory for the thread.
    pub cwd: PathBuf,
    /// Version of the CLI that created the thread.
    pub cli_version: String,
    /// A best-effort thread title.
    pub title: String,
    /// Best available user-facing preview for discovery and list display.
    pub preview: Option<String>,
    /// The sandbox policy (stringified enum).
    pub sandbox_policy: String,
    /// The approval mode (stringified enum).
    pub approval_mode: String,
    /// The last observed token usage.
    pub tokens_used: i64,
    /// First user message observed for this thread, if any.
    pub first_user_message: Option<String>,
    /// The archive timestamp, if the thread is archived.
    pub archived_at: Option<DateTime<Utc>>,
    /// The git commit SHA, if known.
    pub git_sha: Option<String>,
    /// The git branch name, if known.
    pub git_branch: Option<String>,
    /// The git origin URL, if known.
    pub git_origin_url: Option<String>,
    /// Persisted event subscriptions for runtime restore, if a current-state snapshot exists.
    pub subscriptions: Option<Vec<PersistedSubscription>>,
}

/// Builder data required to construct [`ThreadMetadata`] without parsing filenames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMetadataBuilder {
    /// The thread identifier.
    pub id: ThreadId,
    /// The absolute rollout path on disk.
    pub rollout_path: PathBuf,
    /// The creation timestamp.
    pub created_at: DateTime<Utc>,
    /// The last update timestamp, if known.
    pub updated_at: Option<DateTime<Utc>>,
    /// The session source.
    pub source: SessionSource,
    /// Optional analytics source classification for this thread.
    pub thread_source: Option<ThreadSource>,
    /// Optional random unique nickname assigned to the session.
    pub agent_nickname: Option<String>,
    /// Optional role (agent_role) assigned to the session.
    pub agent_role: Option<String>,
    /// Optional canonical agent path assigned to the session.
    pub agent_path: Option<String>,
    /// The model provider identifier, if known.
    pub model_provider: Option<String>,
    /// The working directory for the thread.
    pub cwd: PathBuf,
    /// Version of the CLI that created the thread.
    pub cli_version: Option<String>,
    /// The sandbox policy.
    pub sandbox_policy: SandboxPolicy,
    /// The approval mode.
    pub approval_mode: AskForApproval,
    /// The archive timestamp, if the thread is archived.
    pub archived_at: Option<DateTime<Utc>>,
    /// The git commit SHA, if known.
    pub git_sha: Option<String>,
    /// The git branch name, if known.
    pub git_branch: Option<String>,
    /// The git origin URL, if known.
    pub git_origin_url: Option<String>,
}

impl ThreadMetadataBuilder {
    /// Create a new builder with required fields and sensible defaults.
    pub fn new(
        id: ThreadId,
        rollout_path: PathBuf,
        created_at: DateTime<Utc>,
        source: SessionSource,
    ) -> Self {
        Self {
            id,
            rollout_path,
            created_at,
            updated_at: None,
            source,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            model_provider: None,
            cwd: PathBuf::new(),
            cli_version: None,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            approval_mode: AskForApproval::OnRequest,
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
        }
    }

    /// Build canonical thread metadata, filling missing values from defaults.
    pub fn build(&self, default_provider: &str) -> ThreadMetadata {
        let source = enum_to_string(&self.source);
        let sandbox_policy = enum_to_string(&self.sandbox_policy);
        let approval_mode = enum_to_string(&self.approval_mode);
        let created_at = canonicalize_datetime(self.created_at);
        let updated_at = self
            .updated_at
            .map(canonicalize_datetime)
            .unwrap_or(created_at);
        ThreadMetadata {
            id: self.id,
            rollout_path: self.rollout_path.clone(),
            created_at,
            updated_at,
            source,
            thread_source: self.thread_source,
            agent_nickname: self.agent_nickname.clone(),
            agent_role: self.agent_role.clone(),
            agent_path: self
                .agent_path
                .clone()
                .or_else(|| self.source.get_agent_path().map(Into::into)),
            model_provider: self
                .model_provider
                .clone()
                .unwrap_or_else(|| default_provider.to_string()),
            model: None,
            reasoning_effort: None,
            cwd: self.cwd.clone(),
            cli_version: self.cli_version.clone().unwrap_or_default(),
            title: String::new(),
            preview: None,
            sandbox_policy,
            approval_mode,
            tokens_used: 0,
            first_user_message: None,
            archived_at: self.archived_at.map(canonicalize_datetime),
            git_sha: self.git_sha.clone(),
            git_branch: self.git_branch.clone(),
            git_origin_url: self.git_origin_url.clone(),
            subscriptions: None,
        }
    }
}

impl ThreadMetadata {
    /// Preserve existing non-null Git fields when rollout-derived metadata is reconciled.
    pub fn prefer_existing_git_info(&mut self, existing: &Self) {
        if existing.git_sha.is_some() {
            self.git_sha = existing.git_sha.clone();
        }
        if existing.git_branch.is_some() {
            self.git_branch = existing.git_branch.clone();
        }
        if existing.git_origin_url.is_some() {
            self.git_origin_url = existing.git_origin_url.clone();
        }
        if existing.subscriptions.is_some() {
            self.subscriptions = existing.subscriptions.clone();
        }
    }

    /// Return the list of field names that differ between `self` and `other`.
    pub fn diff_fields(&self, other: &Self) -> Vec<&'static str> {
        let mut diffs = Vec::new();
        if self.id != other.id {
            diffs.push("id");
        }
        if self.rollout_path != other.rollout_path {
            diffs.push("rollout_path");
        }
        if self.created_at != other.created_at {
            diffs.push("created_at");
        }
        if self.updated_at != other.updated_at {
            diffs.push("updated_at");
        }
        if self.source != other.source {
            diffs.push("source");
        }
        if self.agent_nickname != other.agent_nickname {
            diffs.push("agent_nickname");
        }
        if self.agent_role != other.agent_role {
            diffs.push("agent_role");
        }
        if self.agent_path != other.agent_path {
            diffs.push("agent_path");
        }
        if self.model_provider != other.model_provider {
            diffs.push("model_provider");
        }
        if self.model != other.model {
            diffs.push("model");
        }
        if self.reasoning_effort != other.reasoning_effort {
            diffs.push("reasoning_effort");
        }
        if self.cwd != other.cwd {
            diffs.push("cwd");
        }
        if self.cli_version != other.cli_version {
            diffs.push("cli_version");
        }
        if self.title != other.title {
            diffs.push("title");
        }
        if self.preview != other.preview {
            diffs.push("preview");
        }
        if self.sandbox_policy != other.sandbox_policy {
            diffs.push("sandbox_policy");
        }
        if self.approval_mode != other.approval_mode {
            diffs.push("approval_mode");
        }
        if self.tokens_used != other.tokens_used {
            diffs.push("tokens_used");
        }
        if self.first_user_message != other.first_user_message {
            diffs.push("first_user_message");
        }
        if self.archived_at != other.archived_at {
            diffs.push("archived_at");
        }
        if self.git_sha != other.git_sha {
            diffs.push("git_sha");
        }
        if self.git_branch != other.git_branch {
            diffs.push("git_branch");
        }
        if self.git_origin_url != other.git_origin_url {
            diffs.push("git_origin_url");
        }
        if self.subscriptions != other.subscriptions {
            diffs.push("subscriptions");
        }
        diffs
    }
}

/// Statistics about a backfill operation.
#[derive(Debug, Clone)]
pub struct BackfillStats {
    /// The number of rollout files scanned.
    pub scanned: usize,
    /// The number of rows upserted successfully.
    pub upserted: usize,
    /// The number of rows that failed to upsert.
    pub failed: usize,
}

fn canonicalize_datetime(dt: DateTime<Utc>) -> DateTime<Utc> {
    epoch_millis_to_datetime(datetime_to_epoch_millis(dt)).unwrap_or(dt)
}

fn datetime_to_epoch_millis(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

fn epoch_millis_to_datetime(value: i64) -> Result<DateTime<Utc>> {
    const MIN_EPOCH_MILLIS: i64 = 1_577_836_800_000;
    let millis = if value < MIN_EPOCH_MILLIS {
        value.saturating_mul(1000)
    } else {
        value
    };
    DateTime::<Utc>::from_timestamp_millis(millis)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp millis: {value}"))
}

fn enum_to_string<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(value)) => value,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}
