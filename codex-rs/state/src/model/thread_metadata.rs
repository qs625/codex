use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use protocol::openai_models::ReasoningEffort;
use protocol::subscriptions::PersistedSubscription;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use std::path::PathBuf;

pub use state_api::Anchor;
pub use state_api::BackfillStats;
pub use state_api::ExtractionOutcome;
pub use state_api::SortDirection;
pub use state_api::SortKey;
pub use state_api::ThreadMetadata;
pub use state_api::ThreadMetadataBuilder;
pub use state_api::ThreadsPage;

#[derive(Debug)]
pub(crate) struct ThreadRow {
    id: String,
    rollout_path: String,
    created_at: i64,
    updated_at: i64,
    source: String,
    thread_source: Option<String>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    agent_path: Option<String>,
    model_provider: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    cwd: String,
    cli_version: String,
    title: String,
    preview: String,
    sandbox_policy: String,
    approval_mode: String,
    tokens_used: i64,
    first_user_message: String,
    archived_at: Option<i64>,
    git_sha: Option<String>,
    git_branch: Option<String>,
    git_origin_url: Option<String>,
    subscriptions: Option<String>,
}

impl ThreadRow {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            id: row.try_get("id")?,
            rollout_path: row.try_get("rollout_path")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            source: row.try_get("source")?,
            thread_source: row.try_get("thread_source")?,
            agent_nickname: row.try_get("agent_nickname")?,
            agent_role: row.try_get("agent_role")?,
            agent_path: row.try_get("agent_path")?,
            model_provider: row.try_get("model_provider")?,
            model: row.try_get("model")?,
            reasoning_effort: row.try_get("reasoning_effort")?,
            cwd: row.try_get("cwd")?,
            cli_version: row.try_get("cli_version")?,
            title: row.try_get("title")?,
            preview: row.try_get("preview")?,
            sandbox_policy: row.try_get("sandbox_policy")?,
            approval_mode: row.try_get("approval_mode")?,
            tokens_used: row.try_get("tokens_used")?,
            first_user_message: row.try_get("first_user_message")?,
            archived_at: row.try_get("archived_at")?,
            git_sha: row.try_get("git_sha")?,
            git_branch: row.try_get("git_branch")?,
            git_origin_url: row.try_get("git_origin_url")?,
            subscriptions: row.try_get("subscriptions")?,
        })
    }
}

impl TryFrom<ThreadRow> for ThreadMetadata {
    type Error = anyhow::Error;

    fn try_from(row: ThreadRow) -> std::result::Result<Self, Self::Error> {
        let ThreadRow {
            id,
            rollout_path,
            created_at,
            updated_at,
            source,
            thread_source,
            agent_nickname,
            agent_role,
            agent_path,
            model_provider,
            model,
            reasoning_effort,
            cwd,
            cli_version,
            title,
            preview,
            sandbox_policy,
            approval_mode,
            tokens_used,
            first_user_message,
            archived_at,
            git_sha,
            git_branch,
            git_origin_url,
            subscriptions,
        } = row;
        let thread_source = thread_source
            .map(|thread_source| thread_source.parse())
            .transpose()
            .map_err(anyhow::Error::msg)?;
        let subscriptions = subscriptions
            .map(|subscriptions| serde_json::from_str::<Vec<PersistedSubscription>>(&subscriptions))
            .transpose()?;
        Ok(Self {
            id: ThreadId::try_from(id)?,
            rollout_path: PathBuf::from(rollout_path),
            created_at: epoch_millis_to_datetime(created_at)?,
            updated_at: epoch_millis_to_datetime(updated_at)?,
            source,
            thread_source,
            agent_nickname,
            agent_role,
            agent_path,
            model_provider,
            model,
            reasoning_effort: reasoning_effort
                .and_then(|value| value.parse::<ReasoningEffort>().ok()),
            cwd: PathBuf::from(cwd),
            cli_version,
            title,
            preview: (!preview.is_empty()).then_some(preview),
            sandbox_policy,
            approval_mode,
            tokens_used,
            first_user_message: (!first_user_message.is_empty()).then_some(first_user_message),
            archived_at: archived_at.map(epoch_seconds_to_datetime).transpose()?,
            git_sha,
            git_branch,
            git_origin_url,
            subscriptions,
        })
    }
}

pub(crate) fn anchor_from_item(item: &ThreadMetadata, sort_key: SortKey) -> Option<Anchor> {
    let ts = match sort_key {
        SortKey::CreatedAt => item.created_at,
        SortKey::UpdatedAt => item.updated_at,
    };
    Some(Anchor { ts })
}

pub(crate) fn datetime_to_epoch_millis(dt: DateTime<Utc>) -> i64 {
    dt.timestamp_millis()
}

pub(crate) fn datetime_to_epoch_seconds(dt: DateTime<Utc>) -> i64 {
    dt.timestamp()
}

pub(crate) fn epoch_millis_to_datetime(value: i64) -> Result<DateTime<Utc>> {
    // Values older than 2020 if interpreted as milliseconds are legacy second-precision rows.
    // Convert them in memory so old state DBs keep ordering correctly after new writes use ms.
    const MIN_EPOCH_MILLIS: i64 = 1_577_836_800_000;
    let millis = if value < MIN_EPOCH_MILLIS {
        value.saturating_mul(1000)
    } else {
        value
    };
    DateTime::<Utc>::from_timestamp_millis(millis)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp millis: {value}"))
}

pub(crate) fn epoch_seconds_to_datetime(value: i64) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(value, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp seconds: {value}"))
}

#[cfg(test)]
mod tests {
    use super::ThreadMetadata;
    use super::ThreadRow;
    use chrono::DateTime;
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use protocol::ThreadId;
    use protocol::openai_models::ReasoningEffort;
    use std::path::PathBuf;

    fn thread_row(reasoning_effort: Option<&str>) -> ThreadRow {
        ThreadRow {
            id: "00000000-0000-0000-0000-000000000123".to_string(),
            rollout_path: "/tmp/rollout-123.jsonl".to_string(),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_100,
            source: "cli".to_string(),
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            model_provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            reasoning_effort: reasoning_effort.map(str::to_string),
            cwd: "/tmp/workspace".to_string(),
            cli_version: "0.0.0".to_string(),
            title: String::new(),
            preview: String::new(),
            sandbox_policy: "read-only".to_string(),
            approval_mode: "on-request".to_string(),
            tokens_used: 1,
            first_user_message: String::new(),
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
            subscriptions: None,
        }
    }

    fn expected_thread_metadata(reasoning_effort: Option<ReasoningEffort>) -> ThreadMetadata {
        ThreadMetadata {
            id: ThreadId::from_string("00000000-0000-0000-0000-000000000123")
                .expect("valid thread id"),
            rollout_path: PathBuf::from("/tmp/rollout-123.jsonl"),
            created_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            updated_at: DateTime::<Utc>::from_timestamp(1_700_000_100, 0).expect("timestamp"),
            source: "cli".to_string(),
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            model_provider: "openai".to_string(),
            model: Some("gpt-5".to_string()),
            reasoning_effort,
            cwd: PathBuf::from("/tmp/workspace"),
            cli_version: "0.0.0".to_string(),
            title: String::new(),
            preview: None,
            sandbox_policy: "read-only".to_string(),
            approval_mode: "on-request".to_string(),
            tokens_used: 1,
            first_user_message: None,
            archived_at: None,
            git_sha: None,
            git_branch: None,
            git_origin_url: None,
            subscriptions: None,
        }
    }

    #[test]
    fn thread_row_parses_reasoning_effort() {
        let metadata = ThreadMetadata::try_from(thread_row(Some("high")))
            .expect("thread metadata should parse");

        assert_eq!(
            metadata,
            expected_thread_metadata(Some(ReasoningEffort::High))
        );
    }

    #[test]
    fn thread_row_ignores_unknown_reasoning_effort_values() {
        let metadata = ThreadMetadata::try_from(thread_row(Some("future")))
            .expect("thread metadata should parse");

        assert_eq!(
            metadata,
            expected_thread_metadata(/*reasoning_effort*/ None)
        );
    }
}
