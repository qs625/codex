use rollout::find_archived_thread_path_by_id_str;
use rollout::read_thread_item_from_rollout;
use rollout::rollout_date_parts;

use super::LocalThreadStore;
use super::helpers::matching_rollout_file_name;
use super::helpers::move_rollout_collection;
use super::helpers::scoped_rollout_path;
use super::helpers::stored_thread_from_rollout_item;
use super::helpers::touch_modified_time;
use crate::ArchiveThreadParams;
use crate::StoredThread;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

pub(super) async fn unarchive_thread(
    store: &LocalThreadStore,
    params: ArchiveThreadParams,
) -> ThreadStoreResult<StoredThread> {
    let thread_id = params.thread_id;
    let state_db_ctx = store.state_db().await;
    let archived_path = find_archived_thread_path_by_id_str(
        store.config.codex_home.as_path(),
        &thread_id.to_string(),
        state_db_ctx.as_deref(),
    )
    .await
    .map_err(|err| ThreadStoreError::InvalidRequest {
        message: format!("failed to locate archived thread id {thread_id}: {err}"),
    })?
    .ok_or_else(|| ThreadStoreError::InvalidRequest {
        message: format!("no archived rollout found for thread id {thread_id}"),
    })?;

    let canonical_archived_path = scoped_rollout_path(
        store
            .config
            .codex_home
            .join(rollout_api::ARCHIVED_SESSIONS_SUBDIR),
        archived_path.as_path(),
        "archived",
    )?;
    let file_name = matching_rollout_file_name(
        canonical_archived_path.as_path(),
        thread_id,
        archived_path.as_path(),
    )?;
    let Some((year, month, day)) = rollout_date_parts(&file_name) else {
        return Err(ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout path `{}` missing filename timestamp",
                archived_path.display()
            ),
        });
    };

    let dest_dir = store
        .config
        .codex_home
        .join(rollout_api::SESSIONS_SUBDIR)
        .join(year)
        .join(month)
        .join(day);
    std::fs::create_dir_all(&dest_dir).map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to unarchive thread: {err}"),
    })?;
    let restored_path = move_rollout_collection(
        canonical_archived_path.as_path(),
        dest_dir.as_path(),
        thread_id,
    )?;
    touch_modified_time(restored_path.as_path()).map_err(|err| ThreadStoreError::Internal {
        message: format!("failed to update unarchived thread timestamp: {err}"),
    })?;

    if let Some(ctx) = state_db_ctx {
        let _ = ctx
            .mark_unarchived(thread_id, restored_path.as_path())
            .await;
    }

    let item = read_thread_item_from_rollout(restored_path.clone())
        .await
        .ok_or_else(|| ThreadStoreError::Internal {
            message: format!(
                "failed to read unarchived thread {}",
                restored_path.display()
            ),
        })?;
    stored_thread_from_rollout_item(
        item,
        /*archived*/ false,
        store.config.default_model_provider_id.as_str(),
    )
    .ok_or_else(|| ThreadStoreError::Internal {
        message: format!(
            "failed to read unarchived thread id from {}",
            restored_path.display()
        ),
    })
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
    use crate::ThreadStore;
    use crate::local::LocalThreadStore;
    use crate::local::test_support::test_config;
    use crate::local::test_support::write_archived_session_file;
    use crate::local::test_support::write_directory_session_file;
    use crate::local::test_support::write_flat_segmented_session_file;

    #[tokio::test]
    async fn unarchive_thread_restores_rollout_and_returns_updated_thread() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(203);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let archived_path = write_archived_session_file(home.path(), "2025-01-03T13-00-00", uuid)
            .expect("archived session file");

        let thread = store
            .unarchive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("unarchive thread");

        assert!(!archived_path.exists());
        let restored_path = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_path.file_name().expect("file name"));
        assert!(restored_path.exists());
        assert_eq!(thread.thread_id, thread_id);
        assert_eq!(thread.rollout_path, Some(restored_path));
        assert_eq!(thread.archived_at, None);
        assert_eq!(thread.preview, "Archived user message");
        assert_eq!(
            thread.first_user_message.as_deref(),
            Some("Archived user message")
        );
    }

    #[tokio::test]
    async fn unarchive_thread_restores_directory_layout_container() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(302);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let archived_base = write_directory_session_file(
            home.path(),
            home.path().join(rollout_api::ARCHIVED_SESSIONS_SUBDIR),
            "2025-01-03T13-00-00",
            uuid,
            "Archived directory user message",
        )
        .expect("archived directory session");
        let archived_container = archived_base.parent().expect("container").to_path_buf();

        let thread = store
            .unarchive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("unarchive thread");

        assert!(!archived_container.exists());
        let restored_container = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_container.file_name().expect("container name"));
        assert!(restored_container.join("rollout.jsonl").exists());
        assert!(restored_container.join("compact-000001.jsonl").exists());
        assert!(restored_container.join("segments.json").exists());
        assert_eq!(thread.thread_id, thread_id);
        assert_eq!(
            thread.rollout_path,
            Some(restored_container.join("compact-000001.jsonl"))
        );
        assert_eq!(thread.preview, "Archived directory user message");
    }

    #[tokio::test]
    async fn unarchive_thread_restores_legacy_flat_segmented_collection() {
        let home = TempDir::new().expect("temp dir");
        let store = LocalThreadStore::new(test_config(home.path()), /*state_db*/ None);
        let uuid = Uuid::from_u128(304);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let (archived_base, archived_head) = write_flat_segmented_session_file(
            home.path(),
            home.path().join(rollout_api::ARCHIVED_SESSIONS_SUBDIR),
            "2025-01-03T13-00-00",
            uuid,
            "Archived legacy segmented user message",
        )
        .expect("archived legacy segmented session");
        let archived_manifest = rollout::segment_manifest_path_for_rollout(archived_base.as_path());

        let thread = store
            .unarchive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("unarchive thread");

        assert!(!archived_base.exists());
        assert!(!archived_head.exists());
        assert!(!archived_manifest.exists());
        let restored_base = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_base.file_name().expect("base name"));
        let restored_head = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_head.file_name().expect("head name"));
        let restored_manifest = rollout::segment_manifest_path_for_rollout(restored_base.as_path());
        assert!(restored_base.exists());
        assert!(restored_head.exists());
        assert!(restored_manifest.exists());
        assert_eq!(thread.thread_id, thread_id);
        assert_eq!(thread.rollout_path, Some(restored_head));
        assert_eq!(thread.preview, "Archived legacy segmented user message");
    }

    #[tokio::test]
    async fn unarchive_thread_accepts_sqlite_directory_container_path() {
        let home = TempDir::new().expect("temp dir");
        let config = test_config(home.path());
        let uuid = Uuid::from_u128(305);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let archived_base = write_directory_session_file(
            home.path(),
            home.path().join(rollout_api::ARCHIVED_SESSIONS_SUBDIR),
            "2025-01-03T13-00-00",
            uuid,
            "Archived directory from sqlite",
        )
        .expect("archived directory session");
        let archived_container = archived_base.parent().expect("container").to_path_buf();
        let runtime = state::StateRuntime::init(
            home.path().to_path_buf(),
            config.default_model_provider_id.clone(),
        )
        .await
        .expect("state db should initialize");
        let mut builder = state::ThreadMetadataBuilder::new(
            thread_id,
            archived_container.clone(),
            Utc::now(),
            SessionSource::Cli,
        );
        builder.model_provider = Some(config.default_model_provider_id.clone());
        builder.cwd = home.path().to_path_buf();
        builder.cli_version = Some("test_version".to_string());
        let mut metadata = builder.build(config.default_model_provider_id.as_str());
        metadata.archived_at = Some(metadata.updated_at);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("state db upsert should succeed");
        let store = LocalThreadStore::new(config, Some(runtime.clone()));

        let thread = store
            .unarchive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("unarchive thread");

        let restored_container = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_container.file_name().expect("container name"));
        let restored_head = restored_container.join("compact-000001.jsonl");
        assert_eq!(thread.rollout_path, Some(restored_head.clone()));
        let updated = runtime
            .get_thread(thread_id)
            .await
            .expect("state db read should succeed")
            .expect("thread metadata should exist");
        assert_eq!(updated.rollout_path, restored_head);
        assert_eq!(updated.archived_at, None);
    }

    #[tokio::test]
    async fn unarchive_thread_updates_sqlite_metadata_when_present() {
        let home = TempDir::new().expect("temp dir");
        let config = test_config(home.path());
        let uuid = Uuid::from_u128(204);
        let thread_id = ThreadId::from_string(&uuid.to_string()).expect("valid thread id");
        let archived_path = write_archived_session_file(home.path(), "2025-01-03T13-00-00", uuid)
            .expect("archived session file");
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
            archived_path.clone(),
            Utc::now(),
            SessionSource::Cli,
        );
        builder.model_provider = Some(config.default_model_provider_id.clone());
        builder.cwd = home.path().to_path_buf();
        builder.cli_version = Some("test_version".to_string());
        let mut metadata = builder.build(config.default_model_provider_id.as_str());
        metadata.archived_at = Some(metadata.updated_at);
        runtime
            .upsert_thread(&metadata)
            .await
            .expect("state db upsert should succeed");

        store
            .unarchive_thread(ArchiveThreadParams { thread_id })
            .await
            .expect("unarchive thread");

        let restored_path = home
            .path()
            .join("sessions/2025/01/03")
            .join(archived_path.file_name().expect("file name"));
        let updated = runtime
            .get_thread(thread_id)
            .await
            .expect("state db read should succeed")
            .expect("thread metadata should exist");
        assert_eq!(updated.rollout_path, restored_path);
        assert_eq!(updated.archived_at, None);
    }
}
