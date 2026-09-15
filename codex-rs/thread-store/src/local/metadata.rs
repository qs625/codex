use chrono::DateTime;
use chrono::Utc;
use protocol::ThreadId;
use protocol::protocol::AskForApproval;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionSource;
use rollout::find_thread_name_by_id;
use rollout::read_session_meta_line;
use state::ThreadMetadata;

use super::LocalThreadStore;
use super::helpers::distinct_thread_metadata_title;
use super::helpers::git_info_from_parts;
use super::helpers::set_thread_name_from_title;
use crate::StoredThread;

pub(super) struct ThreadMetadataOverlay {
    title: Option<String>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    agent_path: Option<String>,
}

impl ThreadMetadataOverlay {
    pub(super) fn from_metadata(metadata: &ThreadMetadata) -> Self {
        Self {
            title: distinct_thread_metadata_title(metadata),
            agent_nickname: metadata.agent_nickname.clone(),
            agent_role: metadata.agent_role.clone(),
            agent_path: metadata.agent_path.clone(),
        }
    }

    pub(super) fn legacy_title(title: String) -> Self {
        Self {
            title: Some(title),
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
        }
    }

    pub(super) fn apply_to_thread(self, thread: &mut StoredThread) {
        if let Some(title) = self.title {
            set_thread_name_from_title(thread, title);
        }
        if thread.agent_nickname.is_none() {
            thread.agent_nickname = self.agent_nickname;
        }
        if thread.agent_role.is_none() {
            thread.agent_role = self.agent_role;
        }
        if thread.agent_path.is_none() {
            thread.agent_path = self.agent_path;
        }
    }
}

pub(super) async fn read_sqlite_metadata(
    store: &LocalThreadStore,
    thread_id: ThreadId,
) -> Option<ThreadMetadata> {
    let runtime = store.state_db().await?;
    runtime.get_thread(thread_id).await.ok().flatten()
}

pub(super) async fn stored_thread_from_sqlite_metadata(
    store: &LocalThreadStore,
    metadata: ThreadMetadata,
) -> StoredThread {
    let name = match distinct_thread_metadata_title(&metadata) {
        Some(title) => Some(title),
        None => find_thread_name_by_id(store.config.codex_home.as_path(), &metadata.id)
            .await
            .ok()
            .flatten()
            .filter(|title| !title.trim().is_empty()),
    };
    let session_meta = read_session_meta_line(metadata.rollout_path.as_path())
        .await
        .ok()
        .map(|meta_line| meta_line.meta);
    let forked_from_id = session_meta.as_ref().and_then(|meta| meta.forked_from_id);
    let preview = metadata
        .preview
        .clone()
        .or_else(|| metadata.first_user_message.clone())
        .unwrap_or_default();
    let skills = rollout::state_db::get_thread_skills(
        store.state_db().await.as_deref(),
        metadata.id,
        "thread_store.read_thread",
    )
    .await
    .unwrap_or_default();
    StoredThread {
        thread_id: metadata.id,
        rollout_path: Some(metadata.rollout_path),
        forked_from_id,
        preview,
        name,
        model_provider: if metadata.model_provider.is_empty() {
            store.config.default_model_provider_id.clone()
        } else {
            metadata.model_provider
        },
        model: metadata.model,
        reasoning_effort: metadata.reasoning_effort,
        created_at: metadata.created_at,
        updated_at: metadata.updated_at,
        archived_at: metadata.archived_at,
        cwd: metadata.cwd,
        cli_version: metadata.cli_version,
        source: parse_session_source(&metadata.source),
        thread_source: metadata.thread_source,
        agent_nickname: metadata.agent_nickname,
        agent_role: metadata.agent_role,
        agent_path: metadata.agent_path,
        git_info: git_info_from_parts(
            metadata.git_sha,
            metadata.git_branch,
            metadata.git_origin_url,
        ),
        approval_mode: parse_or_default(&metadata.approval_mode, AskForApproval::OnRequest),
        sandbox_policy: parse_or_default(
            &metadata.sandbox_policy,
            SandboxPolicy::new_read_only_policy(),
        ),
        token_usage: None,
        first_user_message: metadata.first_user_message,
        thread_status: metadata.thread_status,
        skills,
        history: None,
    }
}

pub(super) fn apply_sqlite_metadata_overlay_to_rollout_thread(
    thread: &mut StoredThread,
    metadata: ThreadMetadata,
) {
    if thread.agent_nickname.is_none() {
        thread.agent_nickname = metadata.agent_nickname;
    }
    if thread.agent_role.is_none() {
        thread.agent_role = metadata.agent_role;
    }
    if thread.agent_path.is_none() {
        thread.agent_path = metadata.agent_path;
    }

    let existing_git_info = thread.git_info.take();
    let (fallback_sha, fallback_branch, fallback_origin_url) = match existing_git_info {
        Some(info) => (
            info.commit_hash.map(|sha| sha.0),
            info.branch,
            info.repository_url,
        ),
        None => (None, None, None),
    };
    thread.git_info = git_info_from_parts(
        metadata.git_sha.or(fallback_sha),
        metadata.git_branch.or(fallback_branch),
        metadata.git_origin_url.or(fallback_origin_url),
    );
}

pub(super) fn prefer_rollout_summary_with_sqlite_metadata(
    mut sqlite_thread: StoredThread,
    mut rollout_thread: StoredThread,
) -> StoredThread {
    if sqlite_thread.name.is_some() {
        rollout_thread.name = sqlite_thread.name.take();
    }
    rollout_thread.git_info = sqlite_thread.git_info;
    rollout_thread.thread_status = sqlite_thread.thread_status;
    if rollout_thread.skills.is_empty() {
        rollout_thread.skills = sqlite_thread.skills;
    }
    rollout_thread
}

fn parse_session_source(source: &str) -> SessionSource {
    serde_json::from_str(source)
        .or_else(|_| serde_json::from_value(serde_json::Value::String(source.to_string())))
        .unwrap_or(SessionSource::Unknown)
}

fn parse_or_default<T>(value: &str, default: T) -> T
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value)
        .or_else(|_| serde_json::from_value(serde_json::Value::String(value.to_string())))
        .unwrap_or(default)
}

pub(super) fn parse_rfc3339_non_optional(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}
