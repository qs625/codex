use chrono::Utc;
use rollout::find_thread_path_by_id_str;
use rollout_api::ARCHIVED_SESSIONS_SUBDIR;
use rollout_api::SESSIONS_SUBDIR;

use super::LocalThreadStore;
use super::helpers::move_rollout_collection;
use super::helpers::scoped_rollout_path;
use crate::ArchiveThreadParams;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

pub(super) async fn archive_thread(
    store: &LocalThreadStore,
    params: ArchiveThreadParams,
) -> ThreadStoreResult<()> {
    let thread_id = params.thread_id;
    let state_db_ctx = store.state_db().await;
    let rollout_path = find_thread_path_by_id_str(
        store.config.codex_home.as_path(),
        &thread_id.to_string(),
        state_db_ctx.as_deref(),
    )
    .await
    .map_err(|err| ThreadStoreError::InvalidRequest {
        message: format!("failed to locate thread id {thread_id}: {err}"),
    })?
    .ok_or_else(|| ThreadStoreError::InvalidRequest {
        message: format!("no rollout found for thread id {thread_id}"),
    })?;

    let canonical_rollout_path = scoped_rollout_path(
        store.config.codex_home.join(SESSIONS_SUBDIR),
        rollout_path.as_path(),
        "sessions",
    )?;
    let archive_folder = store.config.codex_home.join(ARCHIVED_SESSIONS_SUBDIR);
    std::fs::create_dir_all(&archive_folder).map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to archive thread: {err}"),
    })?;
    let archived_path = move_rollout_collection(
        canonical_rollout_path.as_path(),
        archive_folder.as_path(),
        thread_id,
    )?;

    if let Some(ctx) = state_db_ctx {
        let _ = ctx
            .mark_archived(thread_id, archived_path.as_path(), Utc::now())
            .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use protocol::ThreadId;
    use protocol::protocol::SessionSource;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::ListThreadsParams;
    use crate::ThreadSortKey;
    use crate::ThreadStore;
    use crate::local::LocalThreadStore;
    use crate::local::test_support::test_config;
    use crate::local::test_support::write_directory_session_file;
    use crate::local::test_support::write_flat_segmented_session_file;
    use crate::local::test_support::write_session_file;

    #[tokio::test]
    async fn archive_thread_moves_rollout_to_archived_collection() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(201);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let active_path =
            write_session_file(home.path(), "2025-01-03T12-00-00", uuid).expect("session file");

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("archive thread");

        assert!(!active_path.exists());
        let archived_path = home
            .path()
            .join(ARCHIVED_SESSIONS_SUBDIR)
            .join(active_path.file_name().expect("file name"));
        assert!(archived_path.exists());

        let archived = store
            .list_threads(ListThreadsParams {
                page_size: 10,
                cursor: None,
                sort_key: ThreadSortKey::CreatedAt,
                sort_direction: crate::SortDirection::Desc,
                allowed_sources: Vec::new(),
                model_providers: None,
                cwd_filters: None,
                archived: true,
                search_term: None,
                use_state_db_only: false,
            })
            .await
            .expect("archived listing");
        assert_eq!(archived.items.len(), 1);
        assert_eq!(archived.items[0].thread_id, thread_id);
        assert_eq!(archived.items[0].rollout_path, Some(archived_path));
        assert_eq!(
            archived.items[0].archived_at,
            Some(archived.items[0].updated_at)
        );
    }

    #[tokio::test]
    async fn archive_thread_moves_directory_layout_container() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(301);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let active_base = write_directory_session_file(
            home.path(),
            home.path().join("sessions/2025/01/03"),
            "2025-01-03T12-00-00",
            uuid,
            "Directory user message",
        )
        .expect("directory session");
        let active_container = active_base.parent().expect("container").to_path_buf();

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("archive thread");

        assert!(!active_container.exists());
        let archived_container = home
            .path()
            .join(ARCHIVED_SESSIONS_SUBDIR)
            .join(active_container.file_name().expect("container name"));
        assert!(archived_container.join("rollout.jsonl").exists());
        assert!(archived_container.join("compact-000001.jsonl").exists());
        assert!(archived_container.join("segments.json").exists());

        let archived = store
            .list_threads(ListThreadsParams {
                page_size: 10,
                cursor: None,
                sort_key: ThreadSortKey::CreatedAt,
                sort_direction: crate::SortDirection::Desc,
                allowed_sources: Vec::new(),
                model_providers: None,
                cwd_filters: None,
                archived: true,
                search_term: None,
                use_state_db_only: false,
            })
            .await
            .expect("archived listing");
        assert_eq!(archived.items.len(), 1);
        assert_eq!(archived.items[0].thread_id, thread_id);
        assert_eq!(
            archived.items[0].rollout_path,
            Some(archived_container.join("compact-000001.jsonl"))
        );
    }

    #[tokio::test]
    async fn archive_thread_moves_legacy_flat_segmented_collection() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(303);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let (active_base, active_head) = write_flat_segmented_session_file(
            home.path(),
            home.path().join("sessions/2025/01/03"),
            "2025-01-03T12-00-00",
            uuid,
            "Legacy segmented user message",
        )
        .expect("legacy segmented session");
        let manifest_path = rollout::segment_manifest_path_for_rollout(active_base.as_path());

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("archive thread");

        assert!(!active_base.exists());
        assert!(!active_head.exists());
        assert!(!manifest_path.exists());
        let archived_base = home
            .path()
            .join(ARCHIVED_SESSIONS_SUBDIR)
            .join(active_base.file_name().expect("base name"));
        let archived_head = home
            .path()
            .join(ARCHIVED_SESSIONS_SUBDIR)
            .join(active_head.file_name().expect("head name"));
        let archived_manifest = rollout::segment_manifest_path_for_rollout(archived_base.as_path());
        assert!(archived_base.exists());
        assert!(archived_head.exists());
        assert!(archived_manifest.exists());

        let archived = store
            .list_threads(ListThreadsParams {
                page_size: 10,
                cursor: None,
                sort_key: ThreadSortKey::CreatedAt,
                sort_direction: crate::SortDirection::Desc,
                allowed_sources: Vec::new(),
                model_providers: None,
                cwd_filters: None,
                archived: true,
                search_term: None,
                use_state_db_only: false,
            })
            .await
            .expect("archived listing");
        assert_eq!(archived.items.len(), 1);
        assert_eq!(archived.items[0].thread_id, thread_id);
        assert_eq!(archived.items[0].rollout_path, Some(archived_head));
    }

    #[tokio::test]
    async fn archive_thread_ignores_flat_manifest_sibling_rollout_segments() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(304);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let (active_base, active_head) = write_flat_segmented_session_file(
            home.path(),
            home.path().join("sessions/2025/01/03"),
            "2025-01-03T12-00-00",
            uuid,
            "Legacy segmented user message",
        )
        .expect("legacy segmented session");
        let sibling_uuid = Uuid::from_u128(305);
        let sibling_path = write_session_file(home.path(), "2025-01-03T12-00-01", sibling_uuid)
            .expect("sibling session file");
        let manifest_path = rollout::segment_manifest_path_for_rollout(active_base.as_path());
        std::fs::write(
            manifest_path.as_path(),
            serde_json::json!({
                "version": 1,
                "thread_id": thread_id,
                "head": active_head.file_name().expect("head file name").to_string_lossy(),
                "segments": [
                    active_base.file_name().expect("base file name").to_string_lossy(),
                    active_head.file_name().expect("head file name").to_string_lossy(),
                    sibling_path.file_name().expect("sibling file name").to_string_lossy()
                ],
                "updated_at": "2025-01-03T12:00:00Z"
            })
            .to_string(),
        )
        .expect("rewrite manifest");

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("archive thread");

        assert!(
            sibling_path.exists(),
            "sibling rollout must not move with a corrupt manifest"
        );
        assert!(
            !home
                .path()
                .join(ARCHIVED_SESSIONS_SUBDIR)
                .join(sibling_path.file_name().expect("sibling name"))
                .exists(),
            "archive destination must not receive sibling rollout"
        );
    }

    #[tokio::test]
    async fn archive_thread_rejects_oversized_flat_segment_manifest_without_partial_move() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(306);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let (active_base, active_head) = write_flat_segmented_session_file(
            home.path(),
            home.path().join("sessions/2025/01/03"),
            "2025-01-03T12-00-00",
            uuid,
            "Legacy segmented user message",
        )
        .expect("legacy segmented session");
        let manifest_path = rollout::segment_manifest_path_for_rollout(active_base.as_path());
        let base_stem = active_base
            .file_stem()
            .expect("base stem")
            .to_string_lossy();
        let segments = (0..=1024)
            .map(|idx| format!("{base_stem}.compact-{idx:06}.jsonl"))
            .collect::<Vec<_>>();
        std::fs::write(
            manifest_path.as_path(),
            serde_json::json!({
                "version": 1,
                "thread_id": thread_id,
                "head": segments.last().expect("head"),
                "segments": segments,
                "updated_at": "2025-01-03T12:00:00Z"
            })
            .to_string(),
        )
        .expect("rewrite manifest");

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect_err("oversized manifest should fail closed");

        assert!(active_base.exists());
        assert!(active_head.exists());
        assert!(manifest_path.exists());
    }

    #[tokio::test]
    async fn archive_thread_updates_sqlite_metadata_when_present() {
        let home = TempDir::new().expect("temp dir");
        let config = test_config(home.path());
        let uuid = Uuid::from_u128(202);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let active_path =
            write_session_file(home.path(), "2025-01-03T12-00-00", uuid).expect("session file");
        let runtime = state::StateRuntime::init(
            home.path().to_path_buf(),
            config.default_model_provider_id.clone(),
        )
        .await
        .expect("state db should initialize");
        let store = LocalThreadStore::new(config.clone(), Some(runtime.clone()));
        runtime
            .mark_backfill_complete(/*last_watermark*/ None)
            .await
            .expect("backfill should be complete");
        let mut builder = state::ThreadMetadataBuilder::new(
            thread_id,
            active_path.clone(),
            Utc::now(),
            SessionSource::Cli,
        );
        builder.model_provider = Some(config.default_model_provider_id.clone());
        builder.cwd = home.path().to_path_buf();
        builder.cli_version = Some("test_version".to_string());
        let metadata = builder.build(config.default_model_provider_id.as_str());
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("state db upsert should succeed");

        store
            .archive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("archive thread");

        let archived_path = home
            .path()
            .join(ARCHIVED_SESSIONS_SUBDIR)
            .join(active_path.file_name().expect("file name"));
        let updated = runtime
            .get_thread(thread_id)
            .await
            .expect("state db read should succeed")
            .expect("thread metadata should exist");
        assert_eq!(updated.rollout_path, archived_path);
        assert!(updated.archived_at.is_some());
    }
}
