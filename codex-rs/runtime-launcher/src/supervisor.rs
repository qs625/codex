use crate::ArtifactRecord;
use crate::LauncherError;
use crate::Result;
use crate::artifact::snapshot_bundle_artifact;
use crate::state::LauncherPaths;
use crate::state::LauncherState;
use crate::state::OperationLock;
use crate::state::read_json_if_exists;
use crate::state::remove_file_if_exists;
use crate::transaction::TransactionRecord;
use crate::transaction::TransactionState;
use crate::transaction::TransactionType;
use crate::transaction::activate_transaction;
use crate::transaction::ActivationOutcome;
use crate::transaction::commit_transaction;
use crate::transaction::load_transaction;
use crate::transaction::rollback_transaction;
use crate::transaction::rollback_failure;
use crate::transaction::recovered_transaction_failure;
use crate::persist_recovered_activation_failure;
use crate::transaction::reconcile_transaction;
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::process::ExitStatus;
use std::time::Duration;
use std::time::Instant;

pub const EXIT_COORDINATED_RESTART: i32 = 75;
const READY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const EARLY_CRASH_WINDOW: Duration = Duration::from_secs(30);
const MAX_EARLY_CRASHES: u8 = 3;
const STABLE_HOST_EXECUTABLE: &str = "Contents/MacOS/Root Worker Runtime";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Exited(i32),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyMarker {
    transaction_id: String,
    build_id: String,
}

struct ChildOutcome {
    status: ExitStatus,
    ready: bool,
    full_transaction_committed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchErrorDisposition {
    NoTransaction,
    CommitCleanupPending,
    RolledBack,
}

pub(crate) fn run(paths: &LauncherPaths, app_bundle: &Path) -> Result<RunOutcome> {
    paths.ensure()?;
    {
        let _lock = OperationLock::acquire(paths)?;
        reconcile_for_run(paths)?;
        let mut state = LauncherState::load(paths)?;
        if state.current.is_none() {
            state.current = Some(snapshot_bundle_artifact(paths, app_bundle)?);
            state.save(paths)?;
        }
    }
    let mut rollback_attempted = false;
    let mut early_crashes = 0_u8;

    loop {
        let mut state = LauncherState::load(paths)?;
        let current = state.current.clone().ok_or_else(|| {
            LauncherError::Conflict("launcher has no current artifact".to_string())
        })?;
        let outcome = match launch_once(paths, &current) {
            Ok(outcome) => outcome,
            Err(error) => {
                let _lock = OperationLock::acquire(paths)?;
                match handle_candidate_launch_error(paths, &current, &error)? {
                    LaunchErrorDisposition::NoTransaction
                    | LaunchErrorDisposition::CommitCleanupPending => return Err(error),
                    LaunchErrorDisposition::RolledBack => {
                        rollback_attempted = true;
                        early_crashes = 0;
                        continue;
                    }
                }
            }
        };
        let code = outcome.status.code().unwrap_or(1);
        let _lock = OperationLock::acquire(paths)?;
        if reconcile_for_run(paths)? {
            rollback_attempted = true;
            early_crashes = 0;
            continue;
        }
        state = LauncherState::load(paths)?;
        let launching = matching_full_transaction(paths, &current)?;

        if code == EXIT_COORDINATED_RESTART && launching.is_none() {
            let mut transaction = load_transaction(paths)?.ok_or_else(|| {
                LauncherError::Conflict(
                    "exit 75 requires one prepared full transaction".to_string(),
                )
            })?;
            if transaction.transaction_type != TransactionType::Full
                || transaction.state != TransactionState::Prepared
            {
                return Err(LauncherError::Conflict(
                    "exit 75 requires one prepared full transaction".to_string(),
                ));
            }
            let (activated, previous) =
                match activate_transaction(paths, &mut transaction, &current)? {
                    ActivationOutcome::Activated { current, previous } => (current, previous),
                    ActivationOutcome::RecoveredFailure(failure) => {
                        persist_recovered_activation_failure(paths, &failure)?;
                        rollback_attempted = true;
                        early_crashes = 0;
                        continue;
                    }
                };
            state.current = Some(activated);
            state.previous = Some(previous);
            state.save(paths)?;
            early_crashes = 0;
            continue;
        }

        if let Some(mut transaction) = launching {
            if !outcome.ready {
                let reason = format!("candidate exited with code {code} before readiness");
                let failure = rollback_failure(&transaction, &reason)?;
                rollback_transaction(paths, &mut transaction, state, failure)?;
                let recovered = recovered_transaction_failure(&transaction)?;
                persist_recovered_activation_failure(paths, &recovered)?;
                rollback_attempted = true;
                early_crashes = 0;
                continue;
            }
            if !outcome.full_transaction_committed && !outcome.status.success() {
                early_crashes = early_crashes.saturating_add(1);
                if early_crashes >= MAX_EARLY_CRASHES {
                    let reason = format!(
                        "candidate crashed {} times within {} seconds of readiness",
                        early_crashes,
                        EARLY_CRASH_WINDOW.as_secs()
                    );
                    let failure = rollback_failure(&transaction, reason)?;
                    rollback_transaction(paths, &mut transaction, state, failure)?;
                    let recovered = recovered_transaction_failure(&transaction)?;
                    persist_recovered_activation_failure(paths, &recovered)?;
                    rollback_attempted = true;
                    early_crashes = 0;
                    continue;
                }
                continue;
            }
            if outcome.status.success() && !outcome.full_transaction_committed {
                commit_transaction(paths, &transaction, state)?;
            }
        }

        if rollback_attempted && !outcome.status.success() {
            return Ok(RunOutcome::Exited(code));
        }
        return Ok(RunOutcome::Exited(code));
    }
}

fn reconcile_for_run(paths: &LauncherPaths) -> Result<bool> {
    let Some(failure) = reconcile_transaction(paths)? else {
        return Ok(false);
    };
    persist_recovered_activation_failure(paths, &failure)?;
    Ok(true)
}

fn matching_full_transaction(
    paths: &LauncherPaths,
    current: &ArtifactRecord,
) -> Result<Option<TransactionRecord>> {
    Ok(load_transaction(paths)?.filter(|transaction| {
        transaction.transaction_type == TransactionType::Full
            && transaction.state == TransactionState::Launching
            && transaction.candidate.identity == current.identity
    }))
}

fn handle_candidate_launch_error(
    paths: &LauncherPaths,
    current: &ArtifactRecord,
    error: &LauncherError,
) -> Result<LaunchErrorDisposition> {
    let Some(mut transaction) = matching_full_transaction(paths, current)? else {
        return Ok(LaunchErrorDisposition::NoTransaction);
    };
    if transaction.commit_cleanup_requested {
        return Ok(LaunchErrorDisposition::CommitCleanupPending);
    }
    let state = LauncherState::load(paths)?;
    let failure = rollback_failure(
        &transaction,
        format!("candidate launch failed: {error}"),
    )?;
    rollback_transaction(paths, &mut transaction, state, failure)?;
    let recovered = recovered_transaction_failure(&transaction)?;
    persist_recovered_activation_failure(paths, &recovered)?;
    Ok(LaunchErrorDisposition::RolledBack)
}

fn launch_once(paths: &LauncherPaths, artifact: &ArtifactRecord) -> Result<ChildOutcome> {
    if !artifact.entrypoint.is_file() {
        return Err(LauncherError::Launch(format!(
            "resource entrypoint does not exist: {}",
            artifact.entrypoint.display()
        )));
    }
    let host = artifact.app_bundle_path.join(STABLE_HOST_EXECUTABLE);
    if !host.is_file() {
        return Err(LauncherError::Launch(format!(
            "stable host executable does not exist: {}",
            host.display()
        )));
    }
    remove_file_if_exists(&paths.ready)?;
    let mut command = std::process::Command::new(&host);
    command
        .env("MORPHEUS_LAUNCHER_READY_PATH", &paths.ready)
        .env(
            "MORPHEUS_LAUNCHER_FAILURE_EVIDENCE",
            &paths.failure_evidence,
        )
        .env("MORPHEUS_RUNTIME_LAUNCHER_HOME", &paths.root);
    if matching_full_transaction(paths, artifact)?.is_some() {
        command
            .env(
            "MORPHEUS_LAUNCH_TRANSACTION_ID",
            &artifact.identity.transaction_id,
        )
            .env("MORPHEUS_LAUNCH_BUILD_ID", &artifact.identity.build_id);
    }
    let mut child = command
        .spawn()
        .map_err(|err| crate::io_error(format!("launch {}", host.display()), err))?;
    let mut ready_at = None;
    let mut committed = false;
    loop {
        if ready_at.is_none() && ready_marker_matches(paths, artifact)? {
            ready_at = Some(Instant::now());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|err| crate::io_error("wait for runtime", err))?
        {
            if ready_at.is_none() && ready_marker_matches(paths, artifact)? {
                ready_at = Some(Instant::now());
            }
            if !committed
                && ready_at.is_some_and(|instant| instant.elapsed() >= EARLY_CRASH_WINDOW)
            {
                let _lock = OperationLock::acquire(paths)?;
                if let Some(transaction) = matching_full_transaction(paths, artifact)? {
                    let state = LauncherState::load(paths)?;
                    commit_transaction(paths, &transaction, state)?;
                    committed = true;
                }
            }
            return Ok(ChildOutcome {
                status,
                ready: ready_at.is_some(),
                full_transaction_committed: committed,
            });
        }
        if !committed
            && ready_at.is_some_and(|instant| instant.elapsed() >= EARLY_CRASH_WINDOW)
        {
            let _lock = OperationLock::acquire(paths)?;
            if let Some(transaction) = matching_full_transaction(paths, artifact)? {
                let state = LauncherState::load(paths)?;
                commit_transaction(paths, &transaction, state)?;
                committed = true;
            }
        }
        std::thread::sleep(READY_POLL_INTERVAL);
    }
}

fn ready_marker_matches(paths: &LauncherPaths, artifact: &ArtifactRecord) -> Result<bool> {
    let Some(marker) = read_json_if_exists::<ReadyMarker>(&paths.ready)? else {
        return Ok(false);
    };
    Ok(marker.transaction_id == artifact.identity.transaction_id
        && marker.build_id == artifact.identity.build_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactIdentity;
    use crate::ArtifactManifest;
    use crate::ArtifactManifestEntry;
    use crate::artifact::current_snapshot_path;
    use sha2::Digest;
    use sha2::Sha256;
    use std::io::Read;
    use std::path::PathBuf;

    #[test]
    fn launch_error_preserves_durable_commit_intent_for_startup_reconcile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("state");
        let app_bundle = temp.path().join("Morpheus.app");
        let resources = app_bundle.join("Contents/Resources");
        std::fs::create_dir_all(&resources).expect("resources");
        write_snapshot(&resources, "new");

        let candidate_identity = ArtifactIdentity {
            transaction_id: "tx".to_string(),
            build_id: "new".to_string(),
            source_commit: "commit".to_string(),
        };
        let previous_identity = ArtifactIdentity {
            transaction_id: "installed".to_string(),
            build_id: "old".to_string(),
            source_commit: "commit".to_string(),
        };
        let candidate_root =
            current_snapshot_path(&paths, &candidate_identity).expect("candidate path");
        let previous_root =
            current_snapshot_path(&paths, &previous_identity).expect("previous path");
        std::fs::create_dir_all(&candidate_root).expect("candidate root");
        std::fs::create_dir_all(&previous_root).expect("previous root");
        write_snapshot(&candidate_root, "new");
        write_snapshot(&previous_root, "old");
        let candidate = record(&candidate_identity, &app_bundle, &candidate_root);
        let previous = record(&previous_identity, &app_bundle, &previous_root);
        LauncherState {
            schema_version: 1,
            current: Some(candidate.clone()),
            previous: Some(previous.clone()),
        }
        .save(&paths)
        .expect("state");
        let transaction = TransactionRecord {
            schema_version: 1,
            transaction_id: "tx".to_string(),
            transaction_type: TransactionType::Full,
            state: TransactionState::Launching,
            reason: String::new(),
            candidate: candidate.clone(),
            previous: Some(previous),
            failure: None,
            commit_cleanup_requested: false,
        };
        let state = LauncherState::load(&paths).expect("state");
        let commit_error = crate::transaction::commit_transaction_with_previous_cleanup(
            &paths,
            &transaction,
            state,
            |_paths, previous| {
                std::fs::remove_file(
                    previous
                        .artifact_root
                        .join(".morpheus-runtime-manifest.json"),
                )
                .expect("leave partial previous");
                Err(LauncherError::Launch(
                    "post-intent cleanup failed".to_string(),
                ))
            },
        )
        .expect_err("injected cleanup failure");

        let disposition = handle_candidate_launch_error(
            &paths,
            &candidate,
            &commit_error,
        )
        .expect("handle launch error");

        assert_eq!(
            disposition,
            LaunchErrorDisposition::CommitCleanupPending
        );
        let durable = load_transaction(&paths)
            .expect("transaction")
            .expect("durable transaction");
        assert_eq!(durable.state, TransactionState::Launching);
        assert!(durable.commit_cleanup_requested);
        assert!(durable.failure.is_none());
        assert!(matches!(
            std::fs::symlink_metadata(&paths.failure_evidence),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ));
        assert!(reconcile_transaction(&paths).expect("startup reconcile").is_none());
        assert!(load_transaction(&paths).expect("transaction").is_none());
        let state = LauncherState::load(&paths).expect("state");
        assert_eq!(state.current.expect("current").identity, candidate_identity);
        assert!(state.previous.is_none());
    }

    fn record(
        identity: &ArtifactIdentity,
        app_bundle: &Path,
        root: &Path,
    ) -> ArtifactRecord {
        ArtifactRecord {
            identity: identity.clone(),
            artifact_root: root.to_path_buf(),
            app_bundle_path: app_bundle.to_path_buf(),
            entrypoint: root.join("app.asar"),
            installed_at_unix_ms: 1,
        }
    }

    fn write_snapshot(root: &Path, build_id: &str) {
        for (relative, contents) in [
            ("app.asar", b"runtime".as_slice()),
            ("bin/app-server", b"server".as_slice()),
            ("default-config/compact/COMPACT.md", b"compact".as_slice()),
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("parent");
            std::fs::write(path, contents).expect("artifact");
        }
        let artifacts = [
            "app.asar",
            "bin/app-server",
            "default-config/compact/COMPACT.md",
        ]
        .into_iter()
        .map(|relative| ArtifactManifestEntry {
            relative_path: PathBuf::from(relative),
            sha256: sha256_file(&root.join(relative)),
        })
        .collect();
        let manifest = ArtifactManifest {
            schema_version: 1,
            build_id: build_id.to_string(),
            source_commit: "commit".to_string(),
            entrypoint: PathBuf::from("app.asar"),
            artifacts,
        };
        std::fs::write(
            root.join(".morpheus-runtime-manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest"),
        )
        .expect("manifest");
    }

    fn sha256_file(path: &Path) -> String {
        let mut file = std::fs::File::open(path).expect("artifact");
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = file.read(&mut buffer).expect("read");
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        format!("{:x}", digest.finalize())
    }
}
