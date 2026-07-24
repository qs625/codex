use super::*;
use codex_agent_runtime::AgentMetadata;
use protocol::AgentPath;

pub(super) fn xcode_26_4_mcp_elicitations_auto_deny(
    client_name: Option<&str>,
    client_version: Option<&str>,
) -> bool {
    // Xcode 26.4 shipped before app-server MCP elicitation requests were
    // client-visible. Keep elicitations auto-denied for that client line.
    // TODO: Remove this compatibility hack once Xcode 26.4 ages out.
    client_name == Some("Xcode")
        && client_version.is_some_and(|version| version.starts_with("26.4"))
}

pub(super) const THREAD_TURNS_DEFAULT_LIMIT: usize = 25;
pub(super) const THREAD_TURNS_MAX_LIMIT: usize = 100;

pub(super) fn thread_backwards_cursor_for_sort_key(
    thread: &StoredThread,
    sort_key: StoreThreadSortKey,
    sort_direction: SortDirection,
) -> Option<String> {
    let timestamp = match sort_key {
        StoreThreadSortKey::CreatedAt => thread.created_at,
        StoreThreadSortKey::UpdatedAt => thread.updated_at,
    };
    // The state DB stores unique millisecond timestamps. Offset the reverse cursor by one
    // millisecond so the opposite-direction query includes the page anchor.
    let timestamp = match sort_direction {
        SortDirection::Asc => timestamp.checked_add_signed(ChronoDuration::milliseconds(1))?,
        SortDirection::Desc => timestamp.checked_sub_signed(ChronoDuration::milliseconds(1))?,
    };
    Some(timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
}

pub(super) struct ThreadTurnsPage {
    pub(super) turns: Vec<Turn>,
    pub(super) next_cursor: Option<String>,
    pub(super) backwards_cursor: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadTurnsCursor {
    turn_id: String,
    include_anchor: bool,
}

pub(super) fn paginate_thread_turns(
    turns: Vec<Turn>,
    cursor: Option<&str>,
    limit: Option<u32>,
    sort_direction: SortDirection,
) -> Result<ThreadTurnsPage, JSONRPCErrorError> {
    if turns.is_empty() {
        return Ok(ThreadTurnsPage {
            turns: Vec::new(),
            next_cursor: None,
            backwards_cursor: None,
        });
    }

    let anchor = cursor.map(parse_thread_turns_cursor).transpose()?;
    let page_size = limit
        .map(|value| value as usize)
        .unwrap_or(THREAD_TURNS_DEFAULT_LIMIT)
        .clamp(1, THREAD_TURNS_MAX_LIMIT);

    let anchor_index = anchor
        .as_ref()
        .and_then(|anchor| turns.iter().position(|turn| turn.id == anchor.turn_id));
    if anchor.is_some() && anchor_index.is_none() {
        return Err(invalid_request(
            "invalid cursor: anchor turn is no longer present",
        ));
    }

    let mut keyed_turns: Vec<_> = turns.into_iter().enumerate().collect();
    match sort_direction {
        SortDirection::Asc => {
            if let (Some(anchor), Some(anchor_index)) = (anchor.as_ref(), anchor_index) {
                keyed_turns.retain(|(index, _)| {
                    if anchor.include_anchor {
                        *index >= anchor_index
                    } else {
                        *index > anchor_index
                    }
                });
            }
        }
        SortDirection::Desc => {
            keyed_turns.reverse();
            if let (Some(anchor), Some(anchor_index)) = (anchor.as_ref(), anchor_index) {
                keyed_turns.retain(|(index, _)| {
                    if anchor.include_anchor {
                        *index <= anchor_index
                    } else {
                        *index < anchor_index
                    }
                });
            }
        }
    }

    let more_turns_available = keyed_turns.len() > page_size;
    keyed_turns.truncate(page_size);
    let backwards_cursor = keyed_turns
        .first()
        .map(|(_, turn)| serialize_thread_turns_cursor(&turn.id, /*include_anchor*/ true))
        .transpose()?;
    let next_cursor = if more_turns_available {
        keyed_turns
            .last()
            .map(|(_, turn)| serialize_thread_turns_cursor(&turn.id, /*include_anchor*/ false))
            .transpose()?
    } else {
        None
    };
    let turns = keyed_turns.into_iter().map(|(_, turn)| turn).collect();

    Ok(ThreadTurnsPage {
        turns,
        next_cursor,
        backwards_cursor,
    })
}

pub(super) fn serialize_thread_turns_cursor(
    turn_id: &str,
    include_anchor: bool,
) -> Result<String, JSONRPCErrorError> {
    serde_json::to_string(&ThreadTurnsCursor {
        turn_id: turn_id.to_string(),
        include_anchor,
    })
    .map_err(|err| internal_error(format!("failed to serialize cursor: {err}")))
}

pub(super) fn parse_thread_turns_cursor(cursor: &str) -> Result<ThreadTurnsCursor, JSONRPCErrorError> {
    serde_json::from_str(cursor).map_err(|_| invalid_request(format!("invalid cursor: {cursor}")))
}

pub(super) fn reconstruct_thread_turns_for_turns_list(
    items: &[RolloutItem],
    loaded_status: ThreadLifecycleStatus,
    has_live_running_thread: bool,
    active_turn: Option<Turn>,
) -> Vec<Turn> {
    let has_live_in_progress_turn = has_live_running_thread
        || active_turn
            .as_ref()
            .is_some_and(|turn| matches!(turn.status, TurnStatus::InProgress));
    let mut turns = build_api_turns_from_rollout_items(items);
    normalize_thread_turns_status(&mut turns, loaded_status, has_live_in_progress_turn);
    if let Some(active_turn) = active_turn {
        merge_turn_history_with_active_turn(&mut turns, active_turn);
    }
    turns
}

pub(super) async fn read_thread_history_items(
    thread_store: &dyn ThreadStore,
    thread_id: ThreadId,
) -> Result<Vec<RolloutItem>, ThreadStoreError> {
    let stored_thread = thread_store
        .read_thread(StoreReadThreadParams {
            thread_id,
            include_archived: true,
            include_history: true,
        })
        .await?;
    let history = stored_thread
        .history
        .ok_or_else(|| ThreadStoreError::Internal {
            message: format!("thread store did not return history for thread {thread_id}"),
        })?;
    Ok(history.items)
}

pub(super) fn normalize_thread_turns_status(
    turns: &mut [Turn],
    loaded_status: ThreadLifecycleStatus,
    has_live_in_progress_turn: bool,
) {
    let status = resolve_thread_status(loaded_status, has_live_in_progress_turn);
    if matches!(status, ThreadLifecycleStatus::Active { .. }) {
        return;
    }
    for turn in turns {
        if matches!(turn.status, TurnStatus::InProgress) {
            turn.status = TurnStatus::Interrupted;
        }
    }
}

pub(super) fn apply_persisted_thread_lifecycle_status(
    thread: &mut Thread,
    items: &[RolloutItem],
) {
    if persisted_shutdown_agent_status_from_rollout_items(items).is_some() {
        thread.lifecycle_status =
            super::ops::thread_lifecycle_status_from_agent_status(&AgentStatus::Shutdown);
    }
}

fn persisted_shutdown_agent_status_from_rollout_items(
    items: &[RolloutItem],
) -> Option<AgentStatus> {
    items.iter().rev().find_map(|item| match item {
        RolloutItem::EventMsg(event) => codex_agent_runtime::agent_status_from_event(event)
            .filter(|status| matches!(status, AgentStatus::Shutdown)),
        _ => None,
    })
}

pub(super) enum ThreadReadViewError {
    InvalidRequest(String),
    Unsupported(&'static str),
    Internal(String),
}

pub(super) fn thread_read_view_error(err: ThreadReadViewError) -> JSONRPCErrorError {
    match err {
        ThreadReadViewError::InvalidRequest(message) => invalid_request(message),
        ThreadReadViewError::Unsupported(operation) => {
            unsupported_thread_store_operation(operation)
        }
        ThreadReadViewError::Internal(message) => internal_error(message),
    }
}

pub(super) fn unsupported_thread_store_operation(operation: &'static str) -> JSONRPCErrorError {
    method_not_found(format!("{operation} is not supported yet"))
}

pub(super) fn thread_store_list_error(err: ThreadStoreError) -> JSONRPCErrorError {
    match err {
        ThreadStoreError::InvalidRequest { message } => invalid_request(message),
        ThreadStoreError::Unsupported { operation } => {
            unsupported_thread_store_operation(operation)
        }
        err => internal_error(format!("failed to list threads: {err}")),
    }
}

pub(super) fn thread_store_resume_read_error(err: ThreadStoreError) -> JSONRPCErrorError {
    match err {
        ThreadStoreError::InvalidRequest { message } => invalid_request(message),
        ThreadStoreError::Unsupported { operation } => {
            unsupported_thread_store_operation(operation)
        }
        ThreadStoreError::ThreadNotFound { thread_id } => {
            invalid_request(format!("no rollout found for thread id {thread_id}"))
        }
        err => internal_error(format!("failed to read thread: {err}")),
    }
}

pub(super) fn thread_turns_list_history_load_error(
    thread_id: ThreadId,
    err: ThreadStoreError,
) -> ThreadReadViewError {
    match err {
        ThreadStoreError::InvalidRequest { message }
            if message.starts_with("failed to resolve rollout path `") =>
        {
            ThreadReadViewError::InvalidRequest(format!(
                "thread {thread_id} is not materialized yet; thread/turns/list is unavailable before first user message"
            ))
        }
        ThreadStoreError::InvalidRequest { message } => {
            ThreadReadViewError::InvalidRequest(message)
        }
        ThreadStoreError::Unsupported { operation } => ThreadReadViewError::Unsupported(operation),
        err => ThreadReadViewError::Internal(format!(
            "failed to load thread history for thread {thread_id}: {err}"
        )),
    }
}

pub(super) fn thread_read_history_load_error(
    thread_id: ThreadId,
    err: ThreadStoreError,
) -> ThreadReadViewError {
    match err {
        ThreadStoreError::InvalidRequest { message }
            if message.starts_with("failed to resolve rollout path `") =>
        {
            ThreadReadViewError::InvalidRequest(format!(
                "thread {thread_id} is not materialized yet; includeTurns is unavailable before first user message"
            ))
        }
        ThreadStoreError::ThreadNotFound {
            thread_id: missing_thread_id,
        } if missing_thread_id == thread_id => ThreadReadViewError::InvalidRequest(format!(
            "thread {thread_id} is not materialized yet; includeTurns is unavailable before first user message"
        )),
        ThreadStoreError::InvalidRequest { message } => {
            ThreadReadViewError::InvalidRequest(message)
        }
        ThreadStoreError::Unsupported { operation } => ThreadReadViewError::Unsupported(operation),
        err => ThreadReadViewError::Internal(format!(
            "failed to load thread history for thread {thread_id}: {err}"
        )),
    }
}

pub(super) fn core_thread_write_error(operation: &str, err: CodexErr) -> JSONRPCErrorError {
    match err {
        CodexErr::ThreadNotFound(thread_id) => {
            invalid_request(format!("thread not found: {thread_id}"))
        }
        CodexErr::InvalidRequest(message) => invalid_request(message),
        CodexErr::UnsupportedOperation(message) => method_not_found(message),
        err => internal_error(format!("failed to {operation}: {err}")),
    }
}

pub(super) fn thread_store_archive_error(operation: &str, err: ThreadStoreError) -> JSONRPCErrorError {
    match err {
        ThreadStoreError::InvalidRequest { message } => invalid_request(message),
        ThreadStoreError::Unsupported {
            operation: unsupported_operation,
        } => unsupported_thread_store_operation(unsupported_operation),
        err => internal_error(format!("failed to {operation} thread: {err}")),
    }
}

pub(super) fn set_thread_name_from_title(thread: &mut Thread, title: String) {
    if title.trim().is_empty() || thread.preview.trim() == title.trim() {
        return;
    }
    thread.name = Some(title);
}

pub(super) fn apply_thread_usage_from_rollout_items(thread: &mut Thread, rollout_items: &[RolloutItem]) {
    thread.token_usage =
        super::token_usage_replay::latest_thread_token_usage_from_rollout_items(rollout_items);
    thread.context_usage =
        super::context_usage_replay::latest_nonzero_thread_context_usage_from_rollout_items(
            rollout_items,
        )
        .map(Into::into);
}

pub(super) fn stored_thread_session_source_with_agent_metadata(
    thread: &StoredThread,
) -> protocol::protocol::SessionSource {
    with_thread_spawn_agent_metadata(
        thread.source.clone(),
        thread.agent_nickname.clone(),
        thread.agent_role.clone(),
        thread.agent_path.clone(),
    )
}

pub(super) fn stored_thread_root_agent_metadata(thread: &StoredThread) -> Option<AgentMetadata> {
    if thread.source.is_non_root_agent() {
        return None;
    }
    let agent_path = thread.agent_path.as_ref().and_then(|path| {
        AgentPath::try_from(path.as_str())
            .map_err(|err| {
                warn!("stored root thread agent_path is invalid and will be ignored: {err}");
            })
            .ok()
    });
    if agent_path.is_none() {
        return None;
    }
    Some(AgentMetadata {
        agent_path,
        agent_role: thread.agent_role.clone(),
        ..Default::default()
    })
}

pub(crate) fn thread_from_stored_thread(
    thread: StoredThread,
    fallback_provider: &str,
    fallback_cwd: &AbsolutePathBuf,
) -> (Thread, Option<thread_store::StoredThreadHistory>) {
    let stored_agent_path = thread.agent_path.clone();
    let stored_agent_role = thread.agent_role.clone();
    let source = stored_thread_session_source_with_agent_metadata(&thread);
    let path = thread.rollout_path;
    let git_info = thread.git_info.map(|info| ApiGitInfo {
        sha: info.commit_hash.map(|sha| sha.0),
        branch: info.branch,
        origin_url: info.repository_url,
    });
    let cwd = AbsolutePathBuf::relative_to_current_dir(path_utils::normalize_for_native_workdir(
        thread.cwd,
    ))
    .unwrap_or_else(|err| {
        warn!("failed to normalize thread cwd while reading stored thread: {err}");
        fallback_cwd.clone()
    });
    let history = thread.history;
    let thread_id = thread.thread_id.to_string();
    let mut thread = Thread {
        id: thread_id.clone(),
        session_id: thread_id,
        forked_from_id: thread.forked_from_id.map(|id| id.to_string()),
        preview: thread.preview,
        ephemeral: false,
        model_provider: if thread.model_provider.is_empty() {
            fallback_provider.to_string()
        } else {
            thread.model_provider
        },
        created_at: thread.created_at.timestamp(),
        updated_at: thread.updated_at.timestamp(),
        lifecycle_status: ThreadLifecycleStatus::NotLoaded,
        path,
        cwd,
        cli_version: thread.cli_version,
        agent_nickname: source.get_nickname(),
        agent_role: stored_agent_role.or_else(|| source.get_agent_role()),
        agent_path: stored_agent_path.or_else(|| source.get_agent_path().map(Into::into)),
        source: source.into(),
        thread_source: thread.thread_source.map(Into::into),
        git_info,
        name: thread.name,
        skills: thread.skills.into_iter().map(Into::into).collect(),
        token_usage: None,
        context_usage: None,
        turns: Vec::new(),
    };
    if let Some(history) = history.as_ref() {
        apply_thread_usage_from_rollout_items(&mut thread, history.items.as_slice());
        apply_persisted_thread_lifecycle_status(&mut thread, history.items.as_slice());
    }
    (thread, history)
}

pub(super) async fn sync_active_event_subscriptions(
    active_event_subscriptions: &ActiveEventSubscriptionTracker,
    thread_watch_manager: &ThreadWatchManager,
    thread_id: ThreadId,
    active_count: usize,
) {
    active_event_subscriptions.set_active_count(thread_id, active_count);
    let thread_id_str = thread_id.to_string();
    thread_watch_manager
        .note_active_event_subscriptions(thread_id_str.as_str(), active_count)
        .await;
}

pub(super) fn persisted_subscription_count(thread: &StoredThread) -> usize {
    thread
        .history
        .as_ref()
        .and_then(|history| {
            history.items.iter().rev().find_map(|item| match item {
                RolloutItem::SessionMeta(meta_line) => Some(
                    meta_line
                        .meta
                        .subscriptions
                        .as_ref()
                        .map_or(0, std::vec::Vec::len),
                ),
                _ => None,
            })
        })
        .unwrap_or_default()
}

pub(super) fn persisted_subscription_count_from_rollout(path: Option<&Path>) -> usize {
    let Some(path) = path else {
        return 0;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return 0;
    };
    contents
        .lines()
        .rev()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            let item_type = value.get("type")?.as_str()?;
            if item_type != "session_meta" {
                return None;
            }
            Some(
                value
                    .get("payload")
                    .and_then(|payload| payload.get("subscriptions"))
                    .and_then(|subscriptions| subscriptions.as_array())
                    .map_or(0, std::vec::Vec::len),
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(super) fn summary_from_stored_thread(
    thread: StoredThread,
    fallback_provider: &str,
) -> ConversationSummary {
    let path = thread.rollout_path.unwrap_or_default();
    let source = with_thread_spawn_agent_metadata(
        thread.source,
        thread.agent_nickname.clone(),
        thread.agent_role.clone(),
        thread.agent_path.clone(),
    );
    let git_info = thread.git_info.map(|git| ConversationGitInfo {
        sha: git.commit_hash.map(|sha| sha.0),
        branch: git.branch,
        origin_url: git.repository_url,
    });
    ConversationSummary {
        conversation_id: thread.thread_id,
        path,
        preview: thread.preview,
        // Preserve millisecond precision from the thread store so thread/list cursors
        // round-trip the same ordering key used by pagination queries.
        timestamp: Some(
            thread
                .created_at
                .to_rfc3339_opts(SecondsFormat::Millis, true),
        ),
        updated_at: Some(
            thread
                .updated_at
                .to_rfc3339_opts(SecondsFormat::Millis, true),
        ),
        model_provider: if thread.model_provider.is_empty() {
            fallback_provider.to_string()
        } else {
            thread.model_provider
        },
        cwd: thread.cwd,
        cli_version: thread.cli_version,
        source,
        git_info,
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(super) fn summary_from_state_db_metadata(
    conversation_id: ThreadId,
    path: PathBuf,
    first_user_message: Option<String>,
    preview: Option<String>,
    timestamp: String,
    updated_at: String,
    model_provider: String,
    cwd: PathBuf,
    cli_version: String,
    source: String,
    _thread_source: Option<protocol::protocol::ThreadSource>,
    agent_nickname: Option<String>,
    agent_role: Option<String>,
    agent_path: Option<String>,
    git_sha: Option<String>,
    git_branch: Option<String>,
    git_origin_url: Option<String>,
) -> ConversationSummary {
    let preview = preview.or(first_user_message).unwrap_or_default();
    let source = serde_json::from_str(&source)
        .or_else(|_| serde_json::from_value(serde_json::Value::String(source.clone())))
        .unwrap_or(protocol::protocol::SessionSource::Unknown);
    let source = with_thread_spawn_agent_metadata(source, agent_nickname, agent_role, agent_path);
    let git_info = if git_sha.is_none() && git_branch.is_none() && git_origin_url.is_none() {
        None
    } else {
        Some(ConversationGitInfo {
            sha: git_sha,
            branch: git_branch,
            origin_url: git_origin_url,
        })
    };
    ConversationSummary {
        conversation_id,
        path,
        preview,
        timestamp: Some(timestamp),
        updated_at: Some(updated_at),
        model_provider,
        cwd,
        cli_version,
        source,
        git_info,
    }
}

#[cfg(test)]
pub(super) fn summary_from_thread_metadata(metadata: &ThreadMetadata) -> ConversationSummary {
    summary_from_state_db_metadata(
        metadata.id,
        metadata.rollout_path.clone(),
        metadata.first_user_message.clone(),
        metadata.preview.clone(),
        metadata
            .created_at
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        metadata
            .updated_at
            .to_rfc3339_opts(SecondsFormat::Secs, true),
        metadata.model_provider.clone(),
        metadata.cwd.clone(),
        metadata.cli_version.clone(),
        metadata.source.clone(),
        metadata.thread_source,
        metadata.agent_nickname.clone(),
        metadata.agent_role.clone(),
        metadata.agent_path.clone(),
        metadata.git_sha.clone(),
        metadata.git_branch.clone(),
        metadata.git_origin_url.clone(),
    )
}

pub(super) fn preview_from_rollout_items(items: &[RolloutItem]) -> String {
    items
        .iter()
        .find_map(|item| match item {
            RolloutItem::EventMsg(protocol::protocol::EventMsg::UserMessage(user)) => {
                Some(user.message.clone())
            }
            _ => None,
        })
        .map(|preview| match preview.find(USER_MESSAGE_BEGIN) {
            Some(idx) => preview[idx + USER_MESSAGE_BEGIN.len()..].trim().to_string(),
            None => preview,
        })
        .unwrap_or_default()
}

pub(super) fn requested_permissions_trust_project(overrides: &ConfigOverrides, cwd: &Path) -> bool {
    if matches!(
        overrides.sandbox_mode,
        Some(
            protocol::config_types::SandboxMode::WorkspaceWrite
                | protocol::config_types::SandboxMode::DangerFullAccess
        )
    ) {
        return true;
    }

    if matches!(
        overrides.default_permissions.as_deref(),
        Some(
            BUILT_IN_PERMISSION_PROFILE_WORKSPACE | BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS
        )
    ) {
        return true;
    }

    overrides
        .permission_profile
        .as_ref()
        .is_some_and(|profile| permission_profile_trusts_project(profile, cwd))
}

pub(super) fn permission_profile_trusts_project(
    profile: &protocol::models::PermissionProfile,
    cwd: &Path,
) -> bool {
    match profile {
        protocol::models::PermissionProfile::Disabled
        | protocol::models::PermissionProfile::External { .. } => true,
        protocol::models::PermissionProfile::Managed { .. } => profile
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(cwd, cwd),
    }
}

pub(super) fn build_thread_from_snapshot(
    thread_id: ThreadId,
    session_id: String,
    config_snapshot: &ThreadConfigSnapshot,
    path: Option<PathBuf>,
) -> Thread {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    Thread {
        id: thread_id.to_string(),
        session_id,
        forked_from_id: None,
        preview: String::new(),
        ephemeral: config_snapshot.ephemeral,
        model_provider: config_snapshot.model_provider_id.clone(),
        created_at: now,
        updated_at: now,
        lifecycle_status: ThreadLifecycleStatus::NotLoaded,
        path,
        cwd: config_snapshot.cwd.clone(),
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        agent_nickname: config_snapshot.session_source.get_nickname(),
        agent_role: config_snapshot
            .root_agent_role
            .clone()
            .or_else(|| config_snapshot.session_source.get_agent_role()),
        agent_path: config_snapshot.root_agent_path.clone().or_else(|| {
            config_snapshot
                .session_source
                .get_agent_path()
                .map(Into::into)
        }),
        source: config_snapshot.session_source.clone().into(),
        thread_source: config_snapshot.thread_source.map(Into::into),
        git_info: None,
        name: None,
        skills: Vec::new(),
        token_usage: None,
        context_usage: None,
        turns: Vec::new(),
    }
}

pub(super) fn build_thread_from_live_snapshot(
    thread_id: ThreadId,
    live_snapshot: &LiveThreadSnapshot,
) -> Thread {
    build_thread_from_snapshot(
        thread_id,
        live_snapshot.info.session_id.to_string(),
        &live_snapshot.config_snapshot,
        live_snapshot.info.rollout_path.clone(),
    )
}
