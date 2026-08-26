use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use rollout_api::ARCHIVED_SESSIONS_SUBDIR;
use uuid::Uuid;

use super::LocalThreadStoreConfig;

pub(super) fn test_config(codex_home: &Path) -> LocalThreadStoreConfig {
    LocalThreadStoreConfig {
        codex_home: codex_home.to_path_buf(),
        sqlite_home: codex_home.to_path_buf(),
        default_model_provider_id: "test-provider".to_string(),
    }
}

pub(super) fn write_session_file(root: &Path, ts: &str, uuid: Uuid) -> std::io::Result<PathBuf> {
    write_session_file_with(
        root,
        root.join("sessions/2025/01/03"),
        ts,
        uuid,
        "Hello from user",
        Some("test-provider"),
    )
}

pub(super) fn write_archived_session_file(
    root: &Path,
    ts: &str,
    uuid: Uuid,
) -> std::io::Result<PathBuf> {
    write_session_file_with(
        root,
        root.join(ARCHIVED_SESSIONS_SUBDIR),
        ts,
        uuid,
        "Archived user message",
        Some("test-provider"),
    )
}

pub(super) fn write_directory_session_file(
    root: &Path,
    parent_dir: PathBuf,
    ts: &str,
    uuid: Uuid,
    first_user_message: &str,
) -> std::io::Result<PathBuf> {
    let container = parent_dir.join(format!("rollout-{ts}-{uuid}"));
    fs::create_dir_all(&container)?;
    let base_path = container.join("rollout.jsonl");
    write_session_jsonl(
        root,
        &base_path,
        ts,
        uuid,
        first_user_message,
        Some("test-provider"),
        /*forked_from_id*/ None,
    )?;
    let compact_path = container.join("compact-000001.jsonl");
    write_session_jsonl(
        root,
        &compact_path,
        ts,
        uuid,
        first_user_message,
        Some("test-provider"),
        /*forked_from_id*/ None,
    )?;
    fs::write(
        container.join("segments.json"),
        serde_json::json!({
            "version": 1,
            "thread_id": uuid,
            "head": "compact-000001.jsonl",
            "segments": ["rollout.jsonl", "compact-000001.jsonl"],
            "updated_at": "2025-01-03T12:00:00Z"
        })
        .to_string(),
    )?;
    Ok(base_path)
}

pub(super) fn write_flat_segmented_session_file(
    root: &Path,
    day_dir: PathBuf,
    ts: &str,
    uuid: Uuid,
    first_user_message: &str,
) -> std::io::Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(&day_dir)?;
    let base_path = day_dir.join(format!("rollout-{ts}-{uuid}.jsonl"));
    write_session_jsonl(
        root,
        &base_path,
        ts,
        uuid,
        first_user_message,
        Some("test-provider"),
        /*forked_from_id*/ None,
    )?;
    let base_stem = base_path.file_stem().expect("base stem").to_string_lossy();
    let compact_path = day_dir.join(format!("{base_stem}.compact-000001.jsonl"));
    write_session_jsonl(
        root,
        &compact_path,
        ts,
        uuid,
        first_user_message,
        Some("test-provider"),
        /*forked_from_id*/ None,
    )?;
    fs::write(
        day_dir.join(format!("{base_stem}.segments.json")),
        serde_json::json!({
            "version": 1,
            "thread_id": uuid,
            "head": compact_path.file_name().expect("compact file name").to_string_lossy(),
            "segments": [
                base_path.file_name().expect("base file name").to_string_lossy(),
                compact_path.file_name().expect("compact file name").to_string_lossy()
            ],
            "updated_at": "2025-01-03T12:00:00Z"
        })
        .to_string(),
    )?;
    Ok((base_path, compact_path))
}

pub(super) fn write_session_file_with(
    root: &Path,
    day_dir: PathBuf,
    ts: &str,
    uuid: Uuid,
    first_user_message: &str,
    model_provider: Option<&str>,
) -> std::io::Result<PathBuf> {
    write_session_file_with_fork(
        root,
        day_dir,
        ts,
        uuid,
        first_user_message,
        model_provider,
        /*forked_from_id*/ None,
    )
}

pub(super) fn write_session_file_with_fork(
    root: &Path,
    day_dir: PathBuf,
    ts: &str,
    uuid: Uuid,
    first_user_message: &str,
    model_provider: Option<&str>,
    forked_from_id: Option<Uuid>,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(&day_dir)?;
    let path = day_dir.join(format!("rollout-{ts}-{uuid}.jsonl"));
    write_session_jsonl(
        root,
        &path,
        ts,
        uuid,
        first_user_message,
        model_provider,
        forked_from_id,
    )?;
    Ok(path)
}

fn write_session_jsonl(
    root: &Path,
    path: &Path,
    ts: &str,
    uuid: Uuid,
    first_user_message: &str,
    model_provider: Option<&str>,
    forked_from_id: Option<Uuid>,
) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    let meta = serde_json::json!({
        "timestamp": ts,
        "type": "session_meta",
        "payload": {
            "id": uuid,
            "forked_from_id": forked_from_id,
            "timestamp": ts,
            "cwd": root,
            "originator": "test_originator",
            "cli_version": "test_version",
            "source": "cli",
            "model_provider": model_provider,
            "git": {
                "commit_hash": "abcdef",
                "branch": "main",
                "repository_url": "https://example.com/repo.git"
            }
        },
    });
    writeln!(file, "{meta}")?;
    let user_event = serde_json::json!({
        "timestamp": ts,
        "type": "event_msg",
        "payload": {
            "type": "user_message",
            "message": first_user_message,
            "kind": "plain",
        },
    });
    writeln!(file, "{user_event}")?;
    Ok(())
}
