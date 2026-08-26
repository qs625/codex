use std::ffi::OsStr;
use std::fs::FileTimes;
use std::fs::OpenOptions;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::Utc;
use codex_git_info::GitSha;
use protocol::ThreadId;
use protocol::protocol::AskForApproval;
use protocol::protocol::GitInfo;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionSource;
use rollout::ThreadItem;
use rollout_api::ARCHIVED_SESSIONS_SUBDIR;
use state::ThreadMetadata;

use crate::StoredThread;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;

const MAX_FLAT_SEGMENT_MANIFEST_ENTRIES: usize = 1024;

pub(super) fn scoped_rollout_path(
    root: PathBuf,
    rollout_path: &Path,
    root_name: &str,
) -> ThreadStoreResult<PathBuf> {
    let canonical_root =
        std::fs::canonicalize(&root).map_err(|err| ThreadStoreError::Internal {
            message: format!(
                "failed to resolve {root_name} directory `{}`: {err}",
                root.display()
            ),
        })?;
    let canonical_rollout_path =
        std::fs::canonicalize(rollout_path).map_err(|_| ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout path `{}` must be in {root_name} directory",
                rollout_path.display()
            ),
        })?;
    if canonical_rollout_path.starts_with(&canonical_root) {
        Ok(canonical_rollout_path)
    } else {
        Err(ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout path `{}` must be in {root_name} directory",
                rollout_path.display()
            ),
        })
    }
}

pub(super) fn rollout_path_is_archived(codex_home: &Path, path: &Path) -> bool {
    path.starts_with(codex_home.join(ARCHIVED_SESSIONS_SUBDIR))
        || path
            .components()
            .any(|component| component.as_os_str() == OsStr::new(ARCHIVED_SESSIONS_SUBDIR))
}

pub(super) fn matching_rollout_file_name(
    rollout_path: &Path,
    thread_id: ThreadId,
    display_path: &Path,
) -> ThreadStoreResult<std::ffi::OsString> {
    let container_path = movable_rollout_path(rollout_path);
    let Some(file_name) = container_path.file_name().map(OsStr::to_owned) else {
        return Err(ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout path `{}` missing file name",
                display_path.display()
            ),
        });
    };
    let file_name_text = file_name.to_string_lossy();
    let required_file_suffix = format!("{thread_id}.jsonl");
    let required_container_suffix = thread_id.to_string();
    if file_name_text.ends_with(required_file_suffix.as_str())
        || file_name_text.ends_with(required_container_suffix.as_str())
    {
        Ok(file_name)
    } else {
        Err(ThreadStoreError::InvalidRequest {
            message: format!(
                "rollout path `{}` does not match thread id {thread_id}",
                display_path.display()
            ),
        })
    }
}

pub(super) fn move_rollout_collection(
    rollout_path: &Path,
    destination_dir: &Path,
    thread_id: ThreadId,
) -> ThreadStoreResult<PathBuf> {
    let movable_path = movable_rollout_path(rollout_path);
    let file_name = matching_rollout_file_name(rollout_path, thread_id, rollout_path)?;
    let input_is_directory_container = rollout_path.is_dir();
    if rollout_path_is_directory_layout(rollout_path) {
        let moved_container_path = destination_dir.join(&file_name);
        std::fs::rename(&movable_path, &moved_container_path).map_err(|err| {
            ThreadStoreError::Internal {
                message: format!("failed to move rollout collection: {err}"),
            }
        })?;
        if input_is_directory_container {
            return Ok(head_path_for_directory_container(
                moved_container_path.as_path(),
            ));
        }
        return Ok(rollout::rollout_path_after_moving_container(
            rollout_path,
            moved_container_path.as_path(),
        ));
    }

    let related_paths = related_flat_rollout_paths(movable_path.as_path())?;
    for source_path in related_paths {
        if !source_path.exists() {
            continue;
        }
        let Some(source_file_name) = source_path.file_name() else {
            continue;
        };
        std::fs::rename(&source_path, destination_dir.join(source_file_name)).map_err(|err| {
            ThreadStoreError::Internal {
                message: format!("failed to move rollout collection: {err}"),
            }
        })?;
    }

    let moved_file_name = rollout_path
        .file_name()
        .map(OsStr::to_owned)
        .unwrap_or(file_name);
    let moved_path = destination_dir.join(moved_file_name);
    Ok(moved_path)
}

fn movable_rollout_path(rollout_path: &Path) -> PathBuf {
    let container_path = rollout::rollout_container_path(rollout_path);
    if container_path != rollout_path {
        return container_path;
    }
    let Some(file_stem) = rollout_path.file_stem().and_then(|stem| stem.to_str()) else {
        return rollout_path.to_path_buf();
    };
    let Some((base_stem, _compact_suffix)) = file_stem.split_once(".compact-") else {
        return rollout_path.to_path_buf();
    };
    rollout_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{base_stem}.jsonl"))
}

fn rollout_path_is_directory_layout(rollout_path: &Path) -> bool {
    rollout_path.is_dir() || rollout::rollout_container_path(rollout_path) != rollout_path
}

fn head_path_for_directory_container(container: &Path) -> PathBuf {
    let manifest_path = rollout::segment_manifest_path_for_rollout(container);
    if let Ok(text) = std::fs::read_to_string(&manifest_path)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(head) = value.get("head").and_then(serde_json::Value::as_str)
        && is_safe_segment_file_name(head)
    {
        return container.join(head);
    }
    container.join("rollout.jsonl")
}

fn related_flat_rollout_paths(base_path: &Path) -> ThreadStoreResult<Vec<PathBuf>> {
    let mut paths = vec![base_path.to_path_buf()];
    let Some(base_file_name) = base_path.file_name().and_then(OsStr::to_str) else {
        return Ok(paths);
    };
    let Some(base_stem) = base_path.file_stem().and_then(OsStr::to_str) else {
        return Ok(paths);
    };
    let manifest_path = rollout::segment_manifest_path_for_rollout(base_path);
    if let Ok(text) = std::fs::read_to_string(&manifest_path)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(segments) = value.get("segments").and_then(serde_json::Value::as_array)
    {
        if segments.len() > MAX_FLAT_SEGMENT_MANIFEST_ENTRIES {
            return Err(ThreadStoreError::InvalidRequest {
                message: format!(
                    "rollout segment manifest `{}` has too many entries",
                    manifest_path.display()
                ),
            });
        }
        let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new(""));
        for segment in segments {
            let Some(segment) = segment.as_str() else {
                continue;
            };
            if !is_safe_segment_file_name(segment) {
                continue;
            }
            if !segment_belongs_to_flat_rollout(segment, base_file_name, base_stem) {
                continue;
            }
            let segment_path = base_dir.join(segment);
            if !paths.iter().any(|path| path == &segment_path) {
                paths.push(segment_path);
            }
        }
    }
    if manifest_path.exists() {
        paths.push(manifest_path);
    }
    Ok(paths)
}

fn is_safe_segment_file_name(segment: &str) -> bool {
    let path = Path::new(segment);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path.file_name().and_then(OsStr::to_str) == Some(segment)
}

fn segment_belongs_to_flat_rollout(segment: &str, base_file_name: &str, base_stem: &str) -> bool {
    segment == base_file_name
        || segment
            .strip_prefix(base_stem)
            .is_some_and(|suffix| suffix.starts_with(".compact-") && suffix.ends_with(".jsonl"))
}

pub(super) fn touch_modified_time(path: &Path) -> std::io::Result<()> {
    let times = FileTimes::new().set_modified(SystemTime::now());
    OpenOptions::new().append(true).open(path)?.set_times(times)
}

pub(super) fn stored_thread_from_rollout_item(
    item: ThreadItem,
    archived: bool,
    default_provider: &str,
) -> Option<StoredThread> {
    let thread_id = item
        .thread_id
        .or_else(|| thread_id_from_rollout_path(item.path.as_path()))?;
    let created_at = parse_rfc3339(item.created_at.as_deref()).unwrap_or_else(Utc::now);
    let updated_at = parse_rfc3339(item.updated_at.as_deref()).unwrap_or(created_at);
    let archived_at = archived.then_some(updated_at);
    let git_info = git_info_from_parts(
        item.git_sha.clone(),
        item.git_branch.clone(),
        item.git_origin_url.clone(),
    );
    let source = item.source.unwrap_or(SessionSource::Unknown);
    let preview = item
        .preview
        .clone()
        .or_else(|| item.first_user_message.clone())
        .unwrap_or_default();

    Some(StoredThread {
        thread_id,
        rollout_path: Some(item.path),
        forked_from_id: None,
        preview,
        name: None,
        model_provider: item
            .model_provider
            .filter(|provider| !provider.is_empty())
            .unwrap_or_else(|| default_provider.to_string()),
        model: None,
        reasoning_effort: None,
        created_at,
        updated_at,
        archived_at,
        cwd: item.cwd.unwrap_or_default(),
        cli_version: item.cli_version.unwrap_or_default(),
        source,
        thread_source: item.thread_source,
        agent_nickname: item.agent_nickname,
        agent_role: item.agent_role,
        agent_path: item.agent_path,
        git_info,
        approval_mode: AskForApproval::OnRequest,
        sandbox_policy: SandboxPolicy::new_read_only_policy(),
        token_usage: None,
        first_user_message: item.first_user_message,
        skills: Vec::new(),
        history: None,
    })
}

pub(super) fn distinct_thread_metadata_title(metadata: &ThreadMetadata) -> Option<String> {
    let title = metadata.title.trim();
    if title.is_empty() || metadata.first_user_message.as_deref().map(str::trim) == Some(title) {
        None
    } else {
        Some(title.to_string())
    }
}

pub(super) fn set_thread_name_from_title(thread: &mut StoredThread, title: String) {
    if title.trim().is_empty() || thread.preview.trim() == title.trim() {
        return;
    }
    thread.name = Some(title);
}

fn parse_rfc3339(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub(super) fn git_info_from_parts(
    sha: Option<String>,
    branch: Option<String>,
    origin_url: Option<String>,
) -> Option<GitInfo> {
    if sha.is_none() && branch.is_none() && origin_url.is_none() {
        return None;
    }
    Some(GitInfo {
        commit_hash: sha.as_deref().map(GitSha::new),
        branch,
        repository_url: origin_url,
    })
}

fn thread_id_from_rollout_path(path: &Path) -> Option<ThreadId> {
    thread_id_from_rollout_name(path.file_name()?.to_str()?).or_else(|| {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .and_then(thread_id_from_rollout_name)
    })
}

fn thread_id_from_rollout_name(name: &str) -> Option<ThreadId> {
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    if stem.len() < 37 {
        return None;
    }
    let uuid_start = stem.len().saturating_sub(36);
    if !stem[..uuid_start].ends_with('-') {
        return None;
    }
    ThreadId::from_string(&stem[uuid_start..]).ok()
}

#[cfg(test)]
mod tests {
    use super::is_safe_segment_file_name;

    #[test]
    fn safe_segment_file_name_rejects_paths_outside_collection() {
        assert!(is_safe_segment_file_name("rollout.jsonl"));
        assert!(is_safe_segment_file_name("compact-000001.jsonl"));
        assert!(!is_safe_segment_file_name("../outside.jsonl"));
        assert!(!is_safe_segment_file_name("/tmp/outside.jsonl"));
        assert!(!is_safe_segment_file_name("nested/segment.jsonl"));
    }

    #[test]
    fn segment_belongs_to_flat_rollout_rejects_sibling_rollouts() {
        assert!(super::segment_belongs_to_flat_rollout(
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001",
        ));
        assert!(super::segment_belongs_to_flat_rollout(
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001.compact-000001.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001",
        ));
        assert!(!super::segment_belongs_to_flat_rollout(
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000002.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001.jsonl",
            "rollout-2025-01-03T12-00-00-00000000-0000-0000-0000-000000000001",
        ));
    }
}
