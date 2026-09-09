use crate::LauncherError;
use crate::Result;
use crate::control::ControlPaths;
use crate::control::StateLock;
use crate::control::read_json_if_exists;
use crate::control::write_json_atomic;
use crate::io_error;
use crate::json_error;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

#[cfg(unix)]
use std::ffi::CString;

pub const MIGRATION_SCHEMA_VERSION: u32 = 1;
pub const LEGACY_LAUNCHER_SCHEMA_VERSIONS: [u32; 2] = [1, 2];
pub const LEGACY_STATE_CHILDREN: [&str; 6] = [
    "state.json",
    "transaction.json",
    "failure-evidence.json",
    "ready.json",
    ".operation-lock",
    "artifacts",
];

const MIGRATION_JOURNAL_FILE: &str = "migration.json";
const MIGRATION_PAYLOAD_DIR: &str = "state";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Moving,
    Complete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationItemStatus {
    Planned,
    MovedDurable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationItem {
    pub child: String,
    pub status: MigrationItemStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationJournal {
    pub schema_version: u32,
    pub migration_id: String,
    pub source_schema_version: u32,
    pub status: MigrationStatus,
    pub items: Vec<MigrationItem>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MigrationOutcome {
    pub migrated: bool,
    pub migration_id: Option<String>,
    pub moved_children: Vec<String>,
}

pub fn migrate_legacy_state(paths: &ControlPaths) -> Result<MigrationOutcome> {
    let _lock = StateLock::acquire(paths)?;
    migrate_legacy_state_locked(paths)
}

fn migrate_legacy_state_locked(paths: &ControlPaths) -> Result<MigrationOutcome> {
    paths.ensure()?;
    if let Some((journal_path, mut journal)) = find_incomplete_journal(paths)? {
        resume_journal(paths, &journal_path, &mut journal)?;
        return Ok(MigrationOutcome {
            migrated: true,
            migration_id: Some(journal.migration_id.clone()),
            moved_children: journal
                .items
                .iter()
                .map(|item| item.child.clone())
                .collect(),
        });
    }

    let legacy_state_path = paths.root.join("state.json");
    let Some(value) = read_json_if_exists::<serde_json::Value>(&legacy_state_path)? else {
        return Ok(MigrationOutcome::default());
    };
    let source_schema_version = read_legacy_schema(&value)?;
    let migration_id = format!("launcher-state-v{source_schema_version}");
    ensure_or_create_real_directory(&paths.legacy)?;
    let migration_dir = paths.legacy.join(&migration_id);
    let payload_dir = migration_dir.join(MIGRATION_PAYLOAD_DIR);
    let journal_path = migration_dir.join(MIGRATION_JOURNAL_FILE);
    create_new_real_directory(&migration_dir)?;
    create_new_real_directory(&payload_dir)?;

    let mut items = Vec::new();
    for child in LEGACY_STATE_CHILDREN {
        if path_exists(&paths.root.join(child))? {
            items.push(MigrationItem {
                child: child.to_string(),
                status: MigrationItemStatus::Planned,
            });
        }
    }
    let mut journal = MigrationJournal {
        schema_version: MIGRATION_SCHEMA_VERSION,
        migration_id,
        source_schema_version,
        status: MigrationStatus::Moving,
        items,
    };
    validate_journal(&journal)?;
    write_json_atomic(&journal_path, &journal)?;
    resume_journal(paths, &journal_path, &mut journal)?;
    Ok(MigrationOutcome {
        migrated: true,
        migration_id: Some(journal.migration_id.clone()),
        moved_children: journal
            .items
            .iter()
            .map(|item| item.child.clone())
            .collect(),
    })
}

fn find_incomplete_journal(
    paths: &ControlPaths,
) -> Result<Option<(PathBuf, MigrationJournal)>> {
    let metadata = match std::fs::symlink_metadata(&paths.legacy) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(io_error(
                format!("inspect {}", paths.legacy.display()),
                error,
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "legacy migration root is not a real directory: {}",
            paths.legacy.display()
        )));
    }
    let mut incomplete = Vec::new();
    for entry in std::fs::read_dir(&paths.legacy)
        .map_err(|error| io_error(format!("read {}", paths.legacy.display()), error))?
    {
        let entry =
            entry.map_err(|error| io_error(format!("read {}", paths.legacy.display()), error))?;
        let metadata = entry.metadata().map_err(|error| {
            io_error(format!("inspect {}", entry.path().display()), error)
        })?;
        if entry.file_type().map_err(|error| {
            io_error(format!("inspect {}", entry.path().display()), error)
        })?.is_symlink()
            || !metadata.is_dir()
        {
            return Err(LauncherError::Conflict(format!(
                "legacy migration entry is not a real directory: {}",
                entry.path().display()
            )));
        }
        let journal_path = entry.path().join(MIGRATION_JOURNAL_FILE);
        let Some(journal) = load_journal(&journal_path)? else {
            continue;
        };
        if journal.status == MigrationStatus::Moving {
            incomplete.push((journal_path, journal));
        }
    }
    match incomplete.len() {
        0 => Ok(None),
        1 => Ok(incomplete.pop()),
        count => Err(LauncherError::Conflict(format!(
            "found {count} incomplete legacy migrations"
        ))),
    }
}

fn resume_journal(
    paths: &ControlPaths,
    journal_path: &Path,
    journal: &mut MigrationJournal,
) -> Result<()> {
    validate_journal(journal)?;
    let migration_dir = journal_path.parent().ok_or_else(|| {
        LauncherError::Conflict("migration journal has no parent".to_string())
    })?;
    let payload_dir = migration_dir.join(MIGRATION_PAYLOAD_DIR);
    ensure_real_directory(migration_dir)?;
    ensure_real_directory(&payload_dir)?;
    for index in 0..journal.items.len() {
        let child = journal.items[index].child.clone();
        let source = paths.root.join(&child);
        let destination = payload_dir.join(&child);
        reconcile_item(
            &source,
            &destination,
            journal.items[index].status,
        )?;
        if journal.items[index].status != MigrationItemStatus::MovedDurable {
            journal.items[index].status = MigrationItemStatus::MovedDurable;
            write_json_atomic(journal_path, journal)?;
        }
    }
    journal.status = MigrationStatus::Complete;
    write_json_atomic(journal_path, journal)
}

fn reconcile_item(
    source: &Path,
    destination: &Path,
    status: MigrationItemStatus,
) -> Result<()> {
    let source_exists = path_exists(source)?;
    let destination_exists = path_exists(destination)?;
    match (source_exists, destination_exists) {
        (true, false) => {
            rename_no_replace(source, destination)?;
            sync_parent(source)?;
            if source.parent() != destination.parent() {
                sync_parent(destination)?;
            }
            Ok(())
        }
        (false, true) => Ok(()),
        (true, true) => Err(LauncherError::Conflict(format!(
            "legacy migration collision: source {} and destination {} both exist",
            source.display(),
            destination.display()
        ))),
        (false, false) => Err(LauncherError::Conflict(format!(
            "legacy migration lost {} while item was {status:?}",
            source.display()
        ))),
    }
}

fn read_legacy_schema(value: &serde_json::Value) -> Result<u32> {
    let schema = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            LauncherError::Conflict(
                "legacy launcher state has no valid schemaVersion".to_string(),
            )
        })?;
    if LEGACY_LAUNCHER_SCHEMA_VERSIONS.contains(&schema) {
        return Ok(schema);
    }
    let highest_known = LEGACY_LAUNCHER_SCHEMA_VERSIONS
        .iter()
        .copied()
        .max()
        .unwrap_or_default();
    if schema > highest_known {
        return Err(LauncherError::Conflict(format!(
            "legacy launcher state schema {schema} is newer than supported schema \
             {highest_known}"
        )));
    }
    Err(LauncherError::Conflict(format!(
        "legacy launcher state schema {schema} is not explicitly supported"
    )))
}

fn load_journal(path: &Path) -> Result<Option<MigrationJournal>> {
    let Some(value) = read_json_if_exists::<serde_json::Value>(path)? else {
        return Ok(None);
    };
    let schema = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            LauncherError::Conflict(format!(
                "migration journal has no valid schemaVersion: {}",
                path.display()
            ))
        })?;
    if schema != MIGRATION_SCHEMA_VERSION {
        return Err(LauncherError::Conflict(format!(
            "unsupported migration journal schema {schema}: {}",
            path.display()
        )));
    }
    let journal = serde_json::from_value::<MigrationJournal>(value)
        .map_err(|error| json_error(format!("parse {}", path.display()), error))?;
    validate_journal(&journal)?;
    Ok(Some(journal))
}

fn validate_journal(journal: &MigrationJournal) -> Result<()> {
    if journal.schema_version != MIGRATION_SCHEMA_VERSION {
        return Err(LauncherError::Conflict(format!(
            "unsupported migration journal schema {}",
            journal.schema_version
        )));
    }
    if !LEGACY_LAUNCHER_SCHEMA_VERSIONS.contains(&journal.source_schema_version) {
        return Err(LauncherError::Conflict(format!(
            "migration journal references unsupported legacy schema {}",
            journal.source_schema_version
        )));
    }
    if journal.migration_id.is_empty()
        || !journal
            .migration_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(LauncherError::Conflict(
            "migration id contains unsupported characters".to_string(),
        ));
    }
    for item in &journal.items {
        if !LEGACY_STATE_CHILDREN.contains(&item.child.as_str()) {
            return Err(LauncherError::Conflict(format!(
                "migration journal child is not allowlisted: {}",
                item.child
            )));
        }
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "migration path is not a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn create_new_real_directory(path: &Path) -> Result<()> {
    std::fs::create_dir(path)
        .map_err(|error| io_error(format!("create new {}", path.display()), error))?;
    ensure_real_directory(path)?;
    sync_parent(path)
}

fn ensure_or_create_real_directory(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => ensure_real_directory(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_new_real_directory(path)
        }
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

fn path_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

fn sync_parent(path: &Path) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        LauncherError::Conflict(format!("{} has no parent directory", path.display()))
    })?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(format!("sync {}", parent.display()), error))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source_c = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        LauncherError::Conflict(format!("source path contains NUL: {}", source.display()))
    })?;
    let destination_c =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            LauncherError::Conflict(format!(
                "destination path contains NUL: {}",
                destination.display()
            ))
        })?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source_c.as_ptr(),
            libc::AT_FDCWD,
            destination_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io_error(
            format!(
                "move {} to {} without replacement",
                source.display(),
                destination.display()
            ),
            std::io::Error::last_os_error(),
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn rename_no_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source_c = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        LauncherError::Conflict(format!("source path contains NUL: {}", source.display()))
    })?;
    let destination_c =
        CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
            LauncherError::Conflict(format!(
                "destination path contains NUL: {}",
                destination.display()
            ))
        })?;
    let result = unsafe {
        libc::renamex_np(
            source_c.as_ptr(),
            destination_c.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io_error(
            format!(
                "move {} to {} without replacement",
                source.display(),
                destination.display()
            ),
            std::io::Error::last_os_error(),
        ))
    }
}

#[cfg(windows)]
fn rename_no_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }
    let mut source_wide = source.as_os_str().encode_wide().collect::<Vec<_>>();
    source_wide.push(0);
    let mut destination_wide = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    destination_wide.push(0);
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            0,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(io_error(
            format!(
                "move {} to {} without replacement",
                source.display(),
                destination.display()
            ),
            std::io::Error::last_os_error(),
        ))
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
fn rename_no_replace(source: &Path, destination: &Path) -> Result<()> {
    Err(LauncherError::Conflict(format!(
        "atomic no-replace rename is unsupported on this platform: {} to {}",
        source.display(),
        destination.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_legacy_state(paths: &ControlPaths, schema: u32) {
        paths.ensure().expect("paths");
        std::fs::write(
            paths.root.join("state.json"),
            format!(r#"{{"schemaVersion":{schema}}}"#),
        )
        .expect("legacy state");
    }

    fn planned_journal(child: &str) -> MigrationJournal {
        MigrationJournal {
            schema_version: MIGRATION_SCHEMA_VERSION,
            migration_id: "test".to_string(),
            source_schema_version: 2,
            status: MigrationStatus::Moving,
            items: vec![MigrationItem {
                child: child.to_string(),
                status: MigrationItemStatus::Planned,
            }],
        }
    }

    #[test]
    fn migration_moves_only_allowlisted_children() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        write_legacy_state(&paths, 2);
        for child in [
            "transaction.json",
            "failure-evidence.json",
            "ready.json",
            "unknown",
            "source",
            "config",
        ] {
            std::fs::write(paths.root.join(child), child).expect("child");
        }
        let outcome = migrate_legacy_state(&paths).expect("migrate");
        assert!(outcome.migrated);
        for child in [
            "state.json",
            "transaction.json",
            "failure-evidence.json",
            "ready.json",
        ] {
            assert!(!paths.root.join(child).exists());
            assert!(
                paths
                    .legacy
                    .join("launcher-state-v2/state")
                    .join(child)
                    .exists()
            );
        }
        for child in ["unknown", "source", "config"] {
            assert!(paths.root.join(child).exists());
        }
    }

    #[test]
    fn planned_item_recovers_source_destination_crash_combinations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");

        std::fs::write(&source, b"source").expect("source");
        reconcile_item(&source, &destination, MigrationItemStatus::Planned)
            .expect("source only");
        assert!(!source.exists());
        assert!(destination.exists());

        reconcile_item(&source, &destination, MigrationItemStatus::Planned)
            .expect("destination only");

        std::fs::write(&source, b"collision").expect("collision");
        let both = reconcile_item(
            &source,
            &destination,
            MigrationItemStatus::Planned,
        )
        .expect_err("both exist");
        assert!(both.to_string().contains("collision"));

        std::fs::remove_file(&source).expect("remove source");
        std::fs::remove_file(&destination).expect("remove destination");
        let neither = reconcile_item(
            &source,
            &destination,
            MigrationItemStatus::Planned,
        )
        .expect_err("neither exists");
        assert!(neither.to_string().contains("lost"));
    }

    #[test]
    fn moved_durable_item_accepts_only_destination_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::write(&destination, b"destination").expect("destination");
        reconcile_item(
            &source,
            &destination,
            MigrationItemStatus::MovedDurable,
        )
        .expect("durable destination");

        std::fs::write(&source, b"source").expect("source");
        let both = reconcile_item(
            &source,
            &destination,
            MigrationItemStatus::MovedDurable,
        )
        .expect_err("both");
        assert!(both.to_string().contains("collision"));
    }

    #[test]
    fn incomplete_journal_resumes_after_state_json_was_moved() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let migration_dir = paths.legacy.join("test");
        let payload_dir = migration_dir.join(MIGRATION_PAYLOAD_DIR);
        create_new_real_directory(&paths.legacy).expect("legacy");
        create_new_real_directory(&migration_dir).expect("migration");
        create_new_real_directory(&payload_dir).expect("payload");
        let journal_path = migration_dir.join(MIGRATION_JOURNAL_FILE);
        let journal = planned_journal("state.json");
        write_json_atomic(&journal_path, &journal).expect("journal");
        std::fs::write(payload_dir.join("state.json"), br#"{"schemaVersion":2}"#)
            .expect("moved state");

        let outcome = migrate_legacy_state(&paths).expect("resume");
        assert!(outcome.migrated);
        let completed = load_journal(&journal_path)
            .expect("load journal")
            .expect("journal");
        assert_eq!(completed.status, MigrationStatus::Complete);
        assert_eq!(
            completed.items[0].status,
            MigrationItemStatus::MovedDurable
        );
    }

    #[test]
    fn newer_or_unlisted_legacy_schema_fails_closed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        write_legacy_state(&paths, 3);
        let newer = migrate_legacy_state(&paths).expect_err("newer schema");
        assert!(newer.to_string().contains("newer than supported"));

        std::fs::write(paths.root.join("state.json"), br#"{"schemaVersion":0}"#)
            .expect("schema zero");
        let unlisted = migrate_legacy_state(&paths).expect_err("unlisted schema");
        assert!(unlisted.to_string().contains("not explicitly supported"));
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_preexisting_symlink_components() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        write_legacy_state(&paths, 2);
        std::fs::create_dir_all(&paths.legacy).expect("legacy");
        std::os::unix::fs::symlink(
            &outside,
            paths.legacy.join("launcher-state-v2"),
        )
        .expect("migration symlink");

        let error = migrate_legacy_state(&paths).expect_err("symlink rejected");
        assert!(
            error.to_string().contains("create new")
                || error.to_string().contains("not a real directory")
        );
        assert!(std::fs::read_dir(&outside).expect("outside listing").next().is_none());
        assert!(paths.root.join("state.json").exists());
    }
}
