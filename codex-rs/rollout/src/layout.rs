use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use protocol::ThreadId;

pub(crate) const DIRECTORY_BASE_SEGMENT_FILE: &str = "rollout.jsonl";
pub(crate) const DIRECTORY_MANIFEST_FILE: &str = "segments.json";

pub(crate) fn new_session_base_path(
    day_dir: &Path,
    date_str: &str,
    conversation_id: ThreadId,
) -> PathBuf {
    day_dir
        .join(format!("rollout-{date_str}-{conversation_id}"))
        .join(DIRECTORY_BASE_SEGMENT_FILE)
}

pub(crate) fn directory_base_segment_path(container: &Path) -> PathBuf {
    container.join(DIRECTORY_BASE_SEGMENT_FILE)
}

pub fn rollout_container_path(path: &Path) -> PathBuf {
    if is_directory_layout_segment_path(path) {
        path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
    } else {
        path.to_path_buf()
    }
}

pub fn rollout_path_after_moving_container(
    original_path: &Path,
    moved_container: &Path,
) -> PathBuf {
    if is_directory_layout_segment_path(original_path) {
        moved_container.join(
            original_path
                .file_name()
                .unwrap_or_else(|| OsStr::new(DIRECTORY_BASE_SEGMENT_FILE)),
        )
    } else {
        moved_container.to_path_buf()
    }
}

pub(crate) fn segment_manifest_path_for_rollout_path(path: &Path) -> PathBuf {
    if path.is_dir() || looks_like_directory_container_path(path) {
        return path.join(DIRECTORY_MANIFEST_FILE);
    }
    if is_directory_layout_segment_path(path) {
        return path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(DIRECTORY_MANIFEST_FILE);
    }

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = base_rollout_stem(path)
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "rollout".to_string());
    parent.join(format!("{stem}.segments.json"))
}

pub(crate) fn next_segment_path(current_head: &Path, next_index: usize) -> Option<PathBuf> {
    let parent = current_head.parent()?;
    if is_directory_layout_segment_path(current_head) {
        return Some(parent.join(format!("compact-{next_index:06}.jsonl")));
    }
    let base_stem = base_rollout_stem(current_head)?;
    Some(parent.join(format!("{base_stem}.compact-{next_index:06}.jsonl")))
}

pub(crate) fn parse_timestamp_uuid_from_session_name(
    name: &str,
) -> Option<(time::OffsetDateTime, uuid::Uuid)> {
    let mut core = name
        .strip_suffix(".jsonl")
        .unwrap_or(name)
        .strip_prefix("rollout-")?;
    if let Some((base, _compact_suffix)) = core.split_once(".compact-") {
        core = base;
    }

    let (sep_idx, uuid) = core
        .match_indices('-')
        .rev()
        .find_map(|(i, _)| uuid::Uuid::parse_str(&core[i + 1..]).ok().map(|u| (i, u)))?;

    let ts_str = &core[..sep_idx];
    let format: &[time::format_description::FormatItem] =
        time::macros::format_description!("[year]-[month]-[day]T[hour]-[minute]-[second]");
    let ts = time::PrimitiveDateTime::parse(ts_str, format)
        .ok()?
        .assume_utc();
    Some((ts, uuid))
}

pub(crate) fn parse_timestamp_uuid_from_rollout_path(
    path: &Path,
) -> Option<(time::OffsetDateTime, uuid::Uuid)> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(parse_timestamp_uuid_from_session_name)
        .or_else(|| {
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .and_then(parse_timestamp_uuid_from_session_name)
        })
}

pub(crate) fn base_rollout_stem(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    match stem.find(".compact-") {
        Some(index) => Some(stem[..index].to_string()),
        None => Some(stem.to_string()),
    }
}

pub(crate) fn is_directory_layout_segment_path(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let is_directory_segment_name = file_name == DIRECTORY_BASE_SEGMENT_FILE
        || (file_name.starts_with("compact-") && file_name.ends_with(".jsonl"));
    is_directory_segment_name
        && path
            .parent()
            .is_some_and(|parent| looks_like_directory_container_path(parent))
}

fn looks_like_directory_container_path(path: &Path) -> bool {
    path.extension().is_none()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-"))
}
