use crate::ArtifactRecord;
use crate::LauncherError;
use crate::PreparedArtifactRequest;
use crate::Result;
use crate::artifact::copy_snapshot;
use crate::artifact::candidate_store;
use crate::artifact::candidate_copying_path;
use crate::artifact::candidate_temp_store;
use crate::artifact::current_store;
use crate::artifact::current_snapshot_path;
use crate::artifact::rebase_artifact;
use crate::artifact::validate_identifier;
use crate::artifact::validate_artifact_snapshot;
use crate::artifact::validate_artifact_snapshot_at;
use crate::io_error;
use crate::platform::BundleSigner;
use crate::platform::PlatformBundleSigner;
use crate::state::LauncherPaths;
use crate::state::read_json_if_exists;
use crate::state::write_json_atomic;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationChanges {
    #[serde(default)]
    pub main: bool,
    #[serde(default)]
    pub preload: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotActivationRequest {
    #[serde(flatten)]
    pub artifact: PreparedArtifactRequest,
}

impl HotActivationRequest {
    pub(crate) fn validate(&self) -> Result<()> {
        self.artifact.validate_common()?;
        if self.artifact.changes.main || self.artifact.changes.preload {
            return Err(LauncherError::InvalidRequest(
                "hot activation cannot change main or preload".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionType {
    Full,
    Hot,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Prepared,
    Launching,
    RollingBack,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionRecord {
    pub schema_version: u32,
    pub transaction_id: String,
    pub transaction_type: TransactionType,
    pub state: TransactionState,
    pub reason: String,
    pub candidate: ArtifactRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<ArtifactRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TransactionFailure>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub commit_cleanup_requested: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionFailure {
    pub failed: crate::ArtifactIdentity,
    pub fallback: crate::ArtifactIdentity,
    pub reason: String,
}

#[derive(Debug)]
pub(crate) struct RecoveredActivationFailure {
    pub failed: ArtifactRecord,
    pub fallback: ArtifactRecord,
    pub mode: TransactionType,
    pub reason: String,
    pub error: LauncherError,
}

#[derive(Debug)]
pub(crate) enum ActivationOutcome {
    Activated {
        current: ArtifactRecord,
        previous: ArtifactRecord,
    },
    RecoveredFailure(RecoveredActivationFailure),
}

impl TransactionRecord {
    pub(crate) fn full_prepared(
        request: PreparedArtifactRequest,
        candidate: ArtifactRecord,
    ) -> Self {
        Self {
            schema_version: crate::state::SCHEMA_VERSION,
            transaction_id: request.transaction_id,
            transaction_type: TransactionType::Full,
            state: TransactionState::Prepared,
            reason: request.reason,
            candidate,
            previous: None,
            failure: None,
            commit_cleanup_requested: false,
        }
    }

    pub(crate) fn hot_prepared(
        request: HotActivationRequest,
        candidate: ArtifactRecord,
    ) -> Self {
        Self {
            schema_version: crate::state::SCHEMA_VERSION,
            transaction_id: request.artifact.transaction_id,
            transaction_type: TransactionType::Hot,
            state: TransactionState::Prepared,
            reason: request.artifact.reason,
            candidate,
            previous: None,
            failure: None,
            commit_cleanup_requested: false,
        }
    }

    pub(crate) fn require(&self, kind: TransactionType, transaction_id: &str) -> Result<()> {
        if self.transaction_type != kind || self.transaction_id != transaction_id {
            return Err(LauncherError::Conflict(format!(
                "transaction {} does not match requested operation",
                self.transaction_id
            )));
        }
        Ok(())
    }
}

pub(crate) fn load_transaction(paths: &LauncherPaths) -> Result<Option<TransactionRecord>> {
    let transaction: Option<TransactionRecord> = read_json_if_exists(&paths.transaction)?;
    if let Some(transaction) = &transaction
        && transaction.schema_version != crate::state::SCHEMA_VERSION
    {
        return Err(LauncherError::Conflict(format!(
            "unsupported transaction schema {}",
            transaction.schema_version
        )));
    }
    if let Some(transaction) = &transaction
        && transaction.commit_cleanup_requested
        && transaction.state != TransactionState::Launching
    {
        return Err(LauncherError::Conflict(
            "commit cleanup intent requires a launching transaction".to_string(),
        ));
    }
    Ok(transaction)
}

pub(crate) fn require_no_transaction(paths: &LauncherPaths) -> Result<()> {
    if let Some(transaction) = load_transaction(paths)? {
        return Err(LauncherError::Conflict(format!(
            "transaction {} is already active",
            transaction.transaction_id
        )));
    }
    Ok(())
}

pub(crate) fn reconcile_transaction(
    paths: &LauncherPaths,
) -> Result<Option<RecoveredActivationFailure>> {
    reconcile_transaction_with_signer(paths, &PlatformBundleSigner)
}

fn reconcile_transaction_with_signer(
    paths: &LauncherPaths,
    signer: &dyn BundleSigner,
) -> Result<Option<RecoveredActivationFailure>> {
    let Some(mut transaction) = load_transaction(paths)? else {
        return Ok(None);
    };
    validate_identifier("transactionId", &transaction.transaction_id)?;
    if transaction.state == TransactionState::Prepared {
        validate_candidate_path(paths, &transaction)?;
        let store = candidate_store(paths)?;
        let temp_store = candidate_temp_store(paths)?;
        let removed_copying = remove_controlled_directory_if_exists(
            &candidate_copying_path(&temp_store, &transaction.transaction_id),
            &temp_store,
        )?;
        let (swap_new, swap_old) = swap_paths(
            &transaction.candidate.app_bundle_path,
            &transaction.transaction_id,
        );
        let contents = validate_app_bundle_contents(&transaction.candidate.app_bundle_path)?;
        let removed_swap_new = remove_controlled_directory_if_exists(&swap_new, &contents)?;
        let removed_swap_old = remove_controlled_directory_if_exists(&swap_old, &contents)?;
        let removed_swap_copying = remove_swap_temp_if_exists(
            &transaction.candidate.app_bundle_path,
            &transaction.transaction_id,
        )?;
        if transaction.transaction_type == TransactionType::Hot {
            let removed_candidate =
                remove_controlled_directory_if_exists(
                    &transaction.candidate.artifact_root,
                    &store,
                )?;
            if removed_candidate {
                sync_directory(&store)?;
            }
            crate::state::remove_file_if_exists(&paths.transaction)?;
        }
        if removed_copying {
            sync_directory(&temp_store)?;
        }
        if removed_swap_new || removed_swap_old || removed_swap_copying {
            sync_directory(&transaction.candidate.app_bundle_path.join("Contents"))?;
        }
        return Ok(None);
    }
    let candidate_was_promoted = reconcile_candidate_promotion(paths, &mut transaction)?;
    if transaction.commit_cleanup_requested && !candidate_was_promoted {
        return Err(LauncherError::Conflict(
            "commit cleanup intent requires a promoted current snapshot".to_string(),
        ));
    }
    if candidate_was_promoted
        && transaction.commit_cleanup_requested
        && reconcile_committed_cleanup(paths, &transaction)?
    {
        return Ok(None);
    }
    if !candidate_was_promoted {
        preflight_candidate_promotion(paths, &transaction)?;
    }
    validate_transaction_snapshot_aliases(paths, &transaction)?;
    let app_bundle = transaction.candidate.app_bundle_path.clone();
    validate_snapshot_record(paths, &transaction.candidate)?;
    let contents = validate_app_bundle_contents(&app_bundle)?;
    let resources = contents.join("Resources");
    let (swap_new, swap_old) = swap_paths(&app_bundle, &transaction.transaction_id);
    match transaction.state {
        TransactionState::Prepared => Ok(None),
        TransactionState::Launching => {
            if real_directory_exists(&resources, "live resources")?
                && !real_directory_exists(&swap_old, "old swap snapshot")?
                && real_directory_exists(&swap_new, "new swap snapshot")?
            {
                validate_controlled_snapshot_at(
                    &transaction.candidate,
                    &swap_new,
                    &contents,
                )?;
                std::fs::rename(&resources, &swap_old)
                    .map_err(|err| io_error("resume activation backup", err))?;
            }
            if !real_directory_exists(&resources, "live resources")?
                && real_directory_exists(&swap_old, "old swap snapshot")?
                && real_directory_exists(&swap_new, "new swap snapshot")?
            {
                validate_controlled_snapshot_at(
                    &transaction.candidate,
                    &swap_new,
                    &contents,
                )?;
                std::fs::rename(&swap_new, &resources)
                    .map_err(|err| io_error("finish interrupted activation", err))?;
            }
            if real_directory_exists(&resources, "live resources")?
                && real_directory_exists(&swap_old, "old swap snapshot")?
                && !real_directory_exists(&swap_new, "new swap snapshot")?
            {
                remove_controlled_directory_if_exists(&swap_old, &contents)?;
            }
            if real_directory_exists(&resources, "live resources")?
                && !real_directory_exists(&swap_old, "old swap snapshot")?
                && !real_directory_exists(&swap_new, "new swap snapshot")?
            {
                let state = crate::LauncherState::load(paths)?;
                if state.current.as_ref().map(|artifact| &artifact.identity)
                    == Some(&transaction.candidate.identity)
                {
                    return Ok(None);
                }
                seal_swap_temp_namespace(&app_bundle, &transaction.transaction_id)?;
                if let Err(candidate_error) = signer.sign_and_verify(&app_bundle) {
                    let failure = recovered_failure(&transaction, candidate_error)?;
                    begin_rollback(
                        paths,
                        &mut transaction,
                        transaction_failure(&failure),
                    )?;
                    return match restore_previous_resources(
                        paths,
                        &mut transaction,
                        crate::LauncherState::load(paths)?,
                        signer,
                    ) {
                        Ok(_) => Ok(Some(failure)),
                        Err(restore_error) => Err(combined_signing_error(
                            &failure.error,
                            &restore_error,
                        )),
                    };
                }
                promote_candidate_snapshot(paths, &mut transaction)?;
                let mut state = crate::LauncherState::load(paths)?;
                state.current = Some(transaction.candidate.clone());
                state.previous = transaction.previous.clone();
                state.save(paths)?;
                sync_directory(&contents)?;
                return Ok(None);
            }
            Err(LauncherError::Conflict(
                "cannot reconcile launching resource paths".to_string(),
            ))
        }
        TransactionState::RollingBack => {
            let state = crate::LauncherState::load(paths)?;
            restore_previous_resources(
                paths,
                &mut transaction,
                state,
                signer,
            )?;
            Ok(transaction.failure.as_ref().map(|failure| {
                recovered_failure_from_record(&transaction, failure)
            }).transpose()?)
        }
    }
}

pub(crate) fn activate_transaction(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    current: &ArtifactRecord,
) -> Result<ActivationOutcome> {
    activate_transaction_with_signer(paths, transaction, current, &PlatformBundleSigner)
}

fn activate_transaction_with_signer(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    current: &ArtifactRecord,
    signer: &dyn BundleSigner,
) -> Result<ActivationOutcome> {
    let app_bundle = transaction.candidate.app_bundle_path.clone();
    if current.app_bundle_path != app_bundle {
        return Err(LauncherError::Conflict(
            "candidate and current artifacts belong to different app bundles".to_string(),
        ));
    }
    validate_current_root(paths, current)?;
    validate_candidate_root(paths, transaction)?;
    if std::fs::canonicalize(&current.artifact_root)
        .map_err(|error| io_error("canonicalize current snapshot", error))?
        == std::fs::canonicalize(&transaction.candidate.artifact_root)
            .map_err(|error| io_error("canonicalize candidate snapshot", error))?
    {
        return Err(LauncherError::Conflict(
            "candidate and current snapshots must not alias".to_string(),
        ));
    }
    preflight_candidate_promotion(paths, transaction)?;
    let contents = validate_app_bundle_contents(&app_bundle)?;
    let resources = contents.join("Resources");
    let (swap_new, swap_old) = swap_paths(&app_bundle, &transaction.transaction_id);
    if real_directory_exists(&swap_new, "new swap snapshot")?
        || real_directory_exists(&swap_old, "old swap snapshot")?
    {
        return Err(LauncherError::Conflict(format!(
            "transaction swap paths already exist for {}",
            transaction.transaction_id
        )));
    }
    install_swap_snapshot(
        &transaction.candidate,
        &swap_new,
        &transaction.transaction_id,
    )?;
    transaction.state = TransactionState::Launching;
    transaction.previous = Some(current.clone());
    write_json_atomic(&paths.transaction, transaction)?;
    std::fs::rename(&resources, &swap_old)
        .map_err(|err| io_error(format!("backup {}", resources.display()), err))?;
    validate_controlled_snapshot_at(
        &transaction.candidate,
        &swap_new,
        &contents,
    )?;
    if let Err(error) = std::fs::rename(&swap_new, &resources) {
        let _ = std::fs::rename(&swap_old, &resources);
        return Err(io_error(
            format!("activate {}", transaction.candidate.identity.build_id),
            error,
        ));
    }
    remove_controlled_directory_if_exists(&swap_old, &contents)?;
    seal_swap_temp_namespace(&app_bundle, &transaction.transaction_id)?;
    sync_directory(&contents)?;
    if let Err(candidate_error) = signer.sign_and_verify(&app_bundle) {
        let failure = recovered_failure(transaction, candidate_error)?;
        begin_rollback(paths, transaction, transaction_failure(&failure))?;
        return match restore_previous_resources(
            paths,
            transaction,
            crate::LauncherState::load(paths)?,
            signer,
        ) {
            Ok(_) => Ok(ActivationOutcome::RecoveredFailure(failure)),
            Err(restore_error) => Err(combined_signing_error(
                &failure.error,
                &restore_error,
            )),
        };
    }
    promote_candidate_snapshot(paths, transaction)?;
    Ok(ActivationOutcome::Activated {
        current: transaction.candidate.clone(),
        previous: current.clone(),
    })
}

pub(crate) fn abort_prepared_full(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
) -> Result<()> {
    if transaction.transaction_type != TransactionType::Full
        || transaction.state != TransactionState::Prepared
    {
        return Err(LauncherError::Conflict(
            "prepared full candidate path does not match its transaction".to_string(),
        ));
    }
    let expected = &transaction.candidate.artifact_root;
    validate_candidate_path(paths, transaction)?;
    let store = candidate_store(paths)?;
    let temp_store = candidate_temp_store(paths)?;
    let copying = candidate_copying_path(&temp_store, &transaction.transaction_id);
    let (swap_new, swap_old) = swap_paths(
        &transaction.candidate.app_bundle_path,
        &transaction.transaction_id,
    );
    let contents = validate_app_bundle_contents(&transaction.candidate.app_bundle_path)?;
    let removed_candidate = remove_controlled_directory_if_exists(expected, &store)?;
    let removed_copying = remove_controlled_directory_if_exists(&copying, &temp_store)?;
    let removed_swap_new = remove_controlled_directory_if_exists(&swap_new, &contents)?;
    let removed_swap_old = remove_controlled_directory_if_exists(&swap_old, &contents)?;
    let removed_swap_copying = remove_swap_temp_if_exists(
        &transaction.candidate.app_bundle_path,
        &transaction.transaction_id,
    )?;
    if removed_candidate || removed_copying {
        sync_directory(&temp_store)?;
        if removed_candidate {
            sync_directory(expected.parent().ok_or_else(|| {
                LauncherError::Conflict("candidate snapshot has no parent directory".to_string())
            })?)?;
        }
    }
    if removed_swap_new || removed_swap_old || removed_swap_copying {
        sync_directory(&transaction.candidate.app_bundle_path.join("Contents"))?;
    }
    crate::state::remove_file_if_exists(&paths.transaction)
}

fn recovered_failure(
    transaction: &TransactionRecord,
    error: LauncherError,
) -> Result<RecoveredActivationFailure> {
    let fallback = transaction.previous.clone().ok_or_else(|| {
        LauncherError::Conflict("transaction has no previous artifact".to_string())
    })?;
    Ok(RecoveredActivationFailure {
        failed: transaction.candidate.clone(),
        fallback,
        mode: transaction.transaction_type,
        reason: error.to_string(),
        error,
    })
}

fn transaction_failure(failure: &RecoveredActivationFailure) -> TransactionFailure {
    TransactionFailure {
        failed: failure.failed.identity.clone(),
        fallback: failure.fallback.identity.clone(),
        reason: failure.reason.clone(),
    }
}

fn recovered_failure_from_record(
    transaction: &TransactionRecord,
    failure: &TransactionFailure,
) -> Result<RecoveredActivationFailure> {
    let fallback = transaction.previous.clone().ok_or_else(|| {
        LauncherError::Conflict("transaction has no previous artifact".to_string())
    })?;
    if transaction.candidate.identity != failure.failed || fallback.identity != failure.fallback {
        return Err(LauncherError::Conflict(
            "transaction failure identities do not match transaction artifacts".to_string(),
        ));
    }
    Ok(RecoveredActivationFailure {
        failed: transaction.candidate.clone(),
        fallback,
        mode: transaction.transaction_type,
        reason: failure.reason.clone(),
        error: LauncherError::Launch(failure.reason.clone()),
    })
}

fn combined_signing_error(
    candidate_error: &LauncherError,
    restore_error: &LauncherError,
) -> LauncherError {
    LauncherError::Launch(format!(
        "candidate signing failed: {candidate_error}; previous restore signing failed: {restore_error}"
    ))
}

fn remove_controlled_directory_if_exists(path: &Path, intended_parent: &Path) -> Result<bool> {
    require_real_directory(intended_parent, "controlled directory parent")?;
    let canonical_parent = std::fs::canonicalize(intended_parent)
        .map_err(|error| io_error(format!("canonicalize {}", intended_parent.display()), error))?;
    let path_parent = path.parent().ok_or_else(|| {
        LauncherError::Conflict("controlled directory has no parent".to_string())
    })?;
    require_real_directory(path_parent, "controlled path parent")?;
    let canonical_path_parent = std::fs::canonicalize(path_parent)
        .map_err(|error| io_error(format!("canonicalize {}", path_parent.display()), error))?;
    if canonical_path_parent != canonical_parent || path == intended_parent {
        return Err(LauncherError::Conflict(format!(
            "controlled directory is outside its intended parent: {}",
            path.display()
        )));
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(format!("inspect {}", path.display()), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "controlled transaction path is not a real directory: {}",
            path.display()
        )));
    }
    std::fs::remove_dir_all(path)
        .map_err(|error| io_error(format!("remove {}", path.display()), error))?;
    Ok(true)
}

fn require_real_directory(path: &Path, description: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "{description} is not a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn real_directory_exists(path: &Path, description: &str) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(LauncherError::Conflict(format!(
                "{description} is not a real directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

fn validate_app_bundle_contents(app_bundle: &Path) -> Result<std::path::PathBuf> {
    require_real_directory(app_bundle, "app bundle")?;
    let canonical_app = std::fs::canonicalize(app_bundle)
        .map_err(|error| io_error(format!("canonicalize {}", app_bundle.display()), error))?;
    let contents = app_bundle.join("Contents");
    require_real_directory(&contents, "app bundle Contents")?;
    let canonical_contents = std::fs::canonicalize(&contents)
        .map_err(|error| io_error(format!("canonicalize {}", contents.display()), error))?;
    if canonical_contents != canonical_app.join("Contents") {
        return Err(LauncherError::Conflict(format!(
            "app bundle Contents escapes its bundle: {}",
            contents.display()
        )));
    }
    Ok(canonical_contents)
}

fn ensure_real_directory(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(LauncherError::Conflict(format!(
                "temporary namespace is not a real directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(path)
            .map_err(|error| io_error(format!("create {}", path.display()), error)),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

fn remove_empty_directory(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(format!("inspect {}", path.display()), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "temporary namespace is not a real directory: {}",
            path.display()
        )));
    }
    std::fs::remove_dir(path)
        .map_err(|error| io_error(format!("remove empty {}", path.display()), error))
}

fn remove_directory_if_empty(path: &Path) -> Result<bool> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(format!("inspect {}", path.display()), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "temporary namespace is not a real directory: {}",
            path.display()
        )));
    }
    if std::fs::read_dir(path)
        .map_err(|error| io_error(format!("read {}", path.display()), error))?
        .next()
        .is_some()
    {
        return Ok(false);
    }
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(format!("remove empty {}", path.display()), error)),
    }
}

fn remove_swap_temp_if_exists(app_bundle: &Path, transaction_id: &str) -> Result<bool> {
    let root = swap_temp_root(app_bundle);
    match std::fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(format!("inspect {}", root.display()), error)),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(LauncherError::Conflict(format!(
                "swap temporary namespace is not a real directory: {}",
                root.display()
            )));
        }
        Ok(_) => {}
    }
    let _removed =
        remove_controlled_directory_if_exists(&swap_copying_path(app_bundle, transaction_id), &root)?;
    if !remove_directory_if_empty(&root)? {
        return Err(LauncherError::Conflict(format!(
            "swap temporary namespace contains an unrelated entry: {}",
            root.display()
        )));
    }
    Ok(true)
}

fn seal_swap_temp_namespace(app_bundle: &Path, transaction_id: &str) -> Result<()> {
    remove_swap_temp_if_exists(app_bundle, transaction_id)?;
    validate_app_bundle_contents(app_bundle)?;
    Ok(())
}

fn validate_controlled_snapshot_at(
    artifact: &ArtifactRecord,
    root: &Path,
    intended_parent: &Path,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| io_error(format!("inspect {}", root.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "controlled snapshot is not a real directory: {}",
            root.display()
        )));
    }
    let parent_metadata = std::fs::symlink_metadata(intended_parent)
        .map_err(|error| io_error(format!("inspect {}", intended_parent.display()), error))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "controlled snapshot parent is not a real directory: {}",
            intended_parent.display()
        )));
    }
    let path_parent = root.parent().ok_or_else(|| {
        LauncherError::Conflict("controlled snapshot has no parent directory".to_string())
    })?;
    let canonical_parent = std::fs::canonicalize(intended_parent)
        .map_err(|error| io_error(format!("canonicalize {}", intended_parent.display()), error))?;
    let canonical_path_parent = std::fs::canonicalize(path_parent)
        .map_err(|error| io_error(format!("canonicalize {}", path_parent.display()), error))?;
    let file_name = root.file_name().ok_or_else(|| {
        LauncherError::Conflict("controlled snapshot has no leaf name".to_string())
    })?;
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| io_error(format!("canonicalize {}", root.display()), error))?;
    if canonical_path_parent != canonical_parent
        || canonical_root != canonical_parent.join(file_name)
        || root == intended_parent
    {
        return Err(LauncherError::Conflict(format!(
            "controlled snapshot is outside its exact namespace path: {}",
            root.display()
        )));
    }
    validate_artifact_snapshot_at(artifact, root)
}

fn validate_snapshot_record(paths: &LauncherPaths, artifact: &ArtifactRecord) -> Result<()> {
    validate_identifier("transactionId", &artifact.identity.transaction_id)?;
    validate_identifier("buildId", &artifact.identity.build_id)?;
    let metadata = std::fs::symlink_metadata(&artifact.artifact_root)
        .map_err(|error| io_error(format!("inspect {}", artifact.artifact_root.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "artifact snapshot is not a real directory: {}",
            artifact.artifact_root.display()
        )));
    }
    let canonical = std::fs::canonicalize(&artifact.artifact_root)
        .map_err(|error| io_error("canonicalize artifact snapshot", error))?;
    let candidates = candidate_store(paths)?;
    let current = current_store(paths)?;
    let expected_current = current_snapshot_path(paths, &artifact.identity)?;
    let valid = canonical == expected_current
        || canonical
            == candidates.join(&artifact.identity.transaction_id);
    if !valid || canonical == candidates || canonical == current {
        return Err(LauncherError::Conflict(format!(
            "artifact snapshot is outside its exact namespace path: {}",
            artifact.artifact_root.display()
        )));
    }
    Ok(())
}

fn validate_candidate_root(paths: &LauncherPaths, transaction: &TransactionRecord) -> Result<()> {
    validate_candidate_path(paths, transaction)?;
    validate_snapshot_record(paths, &transaction.candidate)
}

fn validate_candidate_path(paths: &LauncherPaths, transaction: &TransactionRecord) -> Result<()> {
    let store = candidate_store(paths)?;
    let expected = store.join(&transaction.transaction_id);
    if transaction.candidate.artifact_root != expected {
        return Err(LauncherError::Conflict(format!(
            "candidate snapshot path does not match transaction {}",
            transaction.transaction_id
        )));
    }
    Ok(())
}

fn validate_current_root(paths: &LauncherPaths, artifact: &ArtifactRecord) -> Result<()> {
    let expected = current_snapshot_path(paths, &artifact.identity)?;
    if artifact.artifact_root != expected {
        return Err(LauncherError::Conflict(format!(
            "current snapshot path does not match artifact {}",
            artifact.identity.recovery_identity()
        )));
    }
    validate_snapshot_record(paths, artifact)?;
    validate_artifact_snapshot_at(artifact, &artifact.artifact_root)
}

fn validate_transaction_snapshot_aliases(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
) -> Result<()> {
    validate_snapshot_record(paths, &transaction.candidate)?;
    let Some(previous) = transaction.previous.as_ref() else {
        return Ok(());
    };
    validate_current_root(paths, previous)?;
    let candidate = std::fs::canonicalize(&transaction.candidate.artifact_root)
        .map_err(|error| io_error("canonicalize transaction candidate", error))?;
    let previous = std::fs::canonicalize(&previous.artifact_root)
        .map_err(|error| io_error("canonicalize transaction previous", error))?;
    if candidate == previous {
        return Err(LauncherError::Conflict(
            "candidate and previous snapshots must not alias".to_string(),
        ));
    }
    Ok(())
}

fn reconcile_candidate_promotion(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
) -> Result<bool> {
    let candidate = candidate_store(paths)?.join(&transaction.transaction_id);
    let destination = current_snapshot_path(paths, &transaction.candidate.identity)?;
    if transaction.candidate.artifact_root == destination {
        if !real_directory_exists(&destination, "promoted current snapshot")? {
            return Err(LauncherError::Conflict(
                "promoted transaction snapshot is missing".to_string(),
            ));
        }
        validate_artifact_snapshot_at(&transaction.candidate, &destination)?;
        return Ok(true);
    }
    if transaction.candidate.artifact_root != candidate {
        return Err(LauncherError::Conflict(
            "transaction snapshot path is outside candidate/current namespaces".to_string(),
        ));
    }
    if real_directory_exists(&candidate, "transaction candidate snapshot")? {
        return Ok(false);
    }
    if !real_directory_exists(&destination, "promoted current snapshot")? {
        return Err(LauncherError::Conflict(
            "transaction candidate snapshot is missing".to_string(),
        ));
    }
    validate_artifact_snapshot_at(&transaction.candidate, &destination)?;
    transaction.candidate = rebase_artifact(&transaction.candidate, destination);
    write_json_atomic(&paths.transaction, transaction)?;
    Ok(true)
}

fn promote_candidate_snapshot(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
) -> Result<()> {
    let destination = current_snapshot_path(paths, &transaction.candidate.identity)?;
    let source = transaction.candidate.artifact_root.clone();
    if source == destination {
        validate_current_root(paths, &transaction.candidate)?;
        return Ok(());
    }
    let source_exists = real_directory_exists(&source, "candidate snapshot")?;
    let destination_exists = real_directory_exists(&destination, "current snapshot destination")?;
    if source_exists && !destination_exists {
        let source_parent = source
            .parent()
            .ok_or_else(|| LauncherError::Conflict("candidate snapshot has no parent".to_string()))?
            .to_path_buf();
        std::fs::rename(&source, &destination)
            .map_err(|error| io_error(format!("promote {}", source.display()), error))?;
        sync_directory(&source_parent)?;
        sync_directory(
            destination
                .parent()
                .ok_or_else(|| LauncherError::Conflict("current snapshot has no parent".to_string()))?,
        )?;
    } else if source_exists || !destination_exists {
        return Err(LauncherError::Conflict(
            "candidate promotion paths are inconsistent".to_string(),
        ));
    }
    transaction.candidate = rebase_artifact(&transaction.candidate, destination);
    validate_snapshot_record(paths, &transaction.candidate)?;
    write_json_atomic(&paths.transaction, transaction)
}

fn preflight_candidate_promotion(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
) -> Result<()> {
    let destination = current_snapshot_path(paths, &transaction.candidate.identity)?;
    if transaction.candidate.artifact_root == destination {
        return Err(LauncherError::Conflict(
            "candidate snapshot must remain in the candidate namespace before activation"
                .to_string(),
        ));
    }
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => Err(LauncherError::Conflict(format!(
            "current snapshot identity already exists: {}",
            transaction.candidate.identity.recovery_identity()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(format!("inspect {}", destination.display()), error)),
    }
}

fn reconcile_committed_cleanup(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
) -> Result<bool> {
    if transaction.state != TransactionState::Launching
        || !transaction.commit_cleanup_requested
    {
        return Ok(false);
    }
    let mut state = crate::LauncherState::load(paths)?;
    let Some(current) = state.current.as_ref() else {
        return Ok(false);
    };
    if current.identity != transaction.candidate.identity {
        return Ok(false);
    }
    validate_current_root(paths, current)?;
    if current.artifact_root != transaction.candidate.artifact_root {
        return Err(LauncherError::Conflict(
            "state current and promoted transaction snapshot paths differ".to_string(),
        ));
    }
    let contents = validate_app_bundle_contents(&transaction.candidate.app_bundle_path)?;
    validate_artifact_snapshot_at(&transaction.candidate, &contents.join("Resources"))?;
    let Some(previous) = transaction.previous.as_ref() else {
        return Err(LauncherError::Conflict(
            "promoted launching transaction has no previous artifact".to_string(),
        ));
    };
    let expected_previous = current_snapshot_path(paths, &previous.identity)?;
    if previous.artifact_root != expected_previous {
        return Err(LauncherError::Conflict(
            "promoted transaction previous snapshot path is invalid".to_string(),
        ));
    }
    cleanup_previous_snapshot_with_intent(paths, previous)?;
    state.previous = None;
    state.save(paths)?;
    crate::state::remove_file_if_exists(&paths.transaction)?;
    Ok(true)
}

pub(crate) fn commit_transaction(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
    state: crate::LauncherState,
) -> Result<crate::LauncherState> {
    commit_transaction_with_previous_cleanup(
        paths,
        transaction,
        state,
        cleanup_previous_snapshot_with_intent,
    )
}

pub(crate) fn commit_transaction_with_previous_cleanup<F>(
    paths: &LauncherPaths,
    transaction: &TransactionRecord,
    mut state: crate::LauncherState,
    cleanup_previous: F,
) -> Result<crate::LauncherState>
where
    F: FnOnce(&LauncherPaths, &ArtifactRecord) -> Result<()>,
{
    if let Some(previous) = &transaction.previous {
        validate_current_root(paths, previous)?;
    }
    validate_current_root(paths, &transaction.candidate)?;
    let contents = validate_app_bundle_contents(&transaction.candidate.app_bundle_path)?;
    validate_artifact_snapshot_at(&transaction.candidate, &contents.join("Resources"))?;
    let mut durable = transaction.clone();
    durable.commit_cleanup_requested = true;
    write_json_atomic(&paths.transaction, &durable)?;
    if let Some(previous) = &durable.previous {
        cleanup_previous(paths, previous)?;
    }
    state.previous = None;
    state.save(paths)?;
    crate::state::remove_file_if_exists(&paths.transaction)?;
    Ok(state)
}

fn cleanup_previous_snapshot_with_intent(
    paths: &LauncherPaths,
    previous: &ArtifactRecord,
) -> Result<()> {
    validate_identifier("transactionId", &previous.identity.transaction_id)?;
    validate_identifier("buildId", &previous.identity.build_id)?;
    let store = current_store(paths)?;
    let expected = current_snapshot_path(paths, &previous.identity)?;
    if previous.artifact_root != expected {
        return Err(LauncherError::Conflict(format!(
            "previous snapshot path does not match artifact {}",
            previous.identity.recovery_identity()
        )));
    }
    if real_directory_exists(&expected, "previous snapshot cleanup target")? {
        remove_controlled_directory_if_exists(&expected, &store)?;
        sync_directory(&store)?;
    }
    Ok(())
}

pub(crate) fn rollback_transaction(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    state: crate::LauncherState,
    failure: TransactionFailure,
) -> Result<crate::LauncherState> {
    rollback_transaction_with_signer(paths, transaction, state, failure, &PlatformBundleSigner)
}

pub(crate) fn begin_rollback(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    failure: TransactionFailure,
) -> Result<()> {
    if transaction.commit_cleanup_requested {
        return Err(LauncherError::Conflict(
            "commit cleanup intent cannot transition to rollback".to_string(),
        ));
    }
    if failure.failed != transaction.candidate.identity
        || transaction.previous.as_ref().map(|artifact| &artifact.identity)
            != Some(&failure.fallback)
    {
        return Err(LauncherError::Conflict(
            "rollback failure identities do not match transaction artifacts".to_string(),
        ));
    }
    if transaction.state == TransactionState::RollingBack {
        if transaction.failure.as_ref() == Some(&failure) {
            return Ok(());
        }
        return Err(LauncherError::Conflict(
            "rolling-back transaction already has a different failure fact".to_string(),
        ));
    }
    transaction.state = TransactionState::RollingBack;
    transaction.failure = Some(failure);
    write_json_atomic(&paths.transaction, transaction)
}

pub(crate) fn rollback_failure(
    transaction: &TransactionRecord,
    reason: impl Into<String>,
) -> Result<TransactionFailure> {
    let fallback = transaction.previous.as_ref().ok_or_else(|| {
        LauncherError::Conflict("transaction has no previous artifact".to_string())
    })?;
    Ok(TransactionFailure {
        failed: transaction.candidate.identity.clone(),
        fallback: fallback.identity.clone(),
        reason: reason.into(),
    })
}

pub(crate) fn recovered_transaction_failure(
    transaction: &TransactionRecord,
) -> Result<RecoveredActivationFailure> {
    let failure = transaction.failure.as_ref().ok_or_else(|| {
        LauncherError::Conflict("rolling-back transaction has no failure fact".to_string())
    })?;
    recovered_failure_from_record(transaction, failure)
}

fn rollback_transaction_with_signer(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    state: crate::LauncherState,
    failure: TransactionFailure,
    signer: &dyn BundleSigner,
) -> Result<crate::LauncherState> {
    begin_rollback(paths, transaction, failure)?;
    restore_previous_resources(paths, transaction, state, signer)
}

fn restore_previous_resources(
    paths: &LauncherPaths,
    transaction: &mut TransactionRecord,
    mut state: crate::LauncherState,
    signer: &dyn BundleSigner,
) -> Result<crate::LauncherState> {
    let previous = transaction.previous.clone().ok_or_else(|| {
        LauncherError::Conflict("transaction has no previous artifact".to_string())
    })?;
    let contents = validate_app_bundle_contents(&transaction.candidate.app_bundle_path)?;
    let resources = contents.join("Resources");
    let (swap_new, swap_old) = swap_paths(
        &transaction.candidate.app_bundle_path,
        &transaction.transaction_id,
    );
    if !real_directory_exists(&swap_new, "rollback swap snapshot")? {
        install_swap_snapshot(&previous, &swap_new, &transaction.transaction_id)?;
    } else if validate_controlled_snapshot_at(
            &previous,
            &swap_new,
            &contents,
        )
    .is_err()
    {
        remove_controlled_directory_if_exists(
            &swap_new,
            &contents,
        )?;
        install_swap_snapshot(&previous, &swap_new, &transaction.transaction_id)?;
    }
    if real_directory_exists(&resources, "live resources")?
        && !real_directory_exists(&swap_old, "failed resource backup")?
    {
        std::fs::rename(&resources, &swap_old)
            .map_err(|err| io_error(format!("move failed {}", resources.display()), err))?;
    }
    if !real_directory_exists(&resources, "live resources")?
        && real_directory_exists(&swap_old, "failed resource backup")?
        && real_directory_exists(&swap_new, "rollback swap snapshot")?
    {
        validate_controlled_snapshot_at(
            &previous,
            &swap_new,
            &contents,
        )?;
        std::fs::rename(&swap_new, &resources)
            .map_err(|err| io_error("restore previous resources snapshot", err))?;
    }
    if real_directory_exists(&resources, "live resources")?
        && real_directory_exists(&swap_old, "failed resource backup")?
        && !real_directory_exists(&swap_new, "rollback swap snapshot")?
    {
        remove_controlled_directory_if_exists(&swap_old, &contents)?;
    }
    if !real_directory_exists(&resources, "live resources")?
        || real_directory_exists(&swap_new, "rollback swap snapshot")?
        || real_directory_exists(&swap_old, "failed resource backup")?
    {
        return Err(LauncherError::Conflict(
            "cannot complete rollback resource swap".to_string(),
        ));
    }
    seal_swap_temp_namespace(
        &transaction.candidate.app_bundle_path,
        &transaction.transaction_id,
    )?;
    sync_directory(&contents)?;
    signer.sign_and_verify(&transaction.candidate.app_bundle_path)?;
    state.current = Some(previous);
    state.previous = None;
    state.save(paths)?;
    if transaction.failure.is_none() {
        crate::state::remove_file_if_exists(&paths.transaction)?;
    }
    Ok(state)
}

fn install_swap_snapshot(
    artifact: &ArtifactRecord,
    swap_new: &Path,
    transaction_id: &str,
) -> Result<()> {
    validate_artifact_snapshot(artifact)?;
    let temp_root = swap_temp_root(&artifact.app_bundle_path);
    ensure_real_directory(&temp_root)?;
    let temp = swap_copying_path(&artifact.app_bundle_path, transaction_id);
    if temp == swap_new {
        return Err(LauncherError::Conflict(
            "swap temporary and published paths must differ".to_string(),
        ));
    }
    remove_controlled_directory_if_exists(&temp, &temp_root)?;
    if std::fs::read_dir(&temp_root)
        .map_err(|error| io_error(format!("read {}", temp_root.display()), error))?
        .next()
        .is_some()
    {
        return Err(LauncherError::Conflict(format!(
            "swap temporary namespace contains an unrelated entry: {}",
            temp_root.display()
        )));
    }
    copy_snapshot(&artifact.artifact_root, &temp)?;
    validate_controlled_snapshot_at(artifact, &temp, &temp_root)?;
    std::fs::rename(&temp, swap_new)
        .map_err(|error| io_error(format!("publish {}", swap_new.display()), error))?;
    remove_empty_directory(&temp_root)?;
    let contents = swap_new
        .parent()
        .ok_or_else(|| LauncherError::Conflict("swap path has no parent".to_string()))?;
    validate_controlled_snapshot_at(artifact, swap_new, contents)?;
    sync_directory(contents)
}

fn swap_copying_path(app_bundle: &Path, transaction_id: &str) -> std::path::PathBuf {
    swap_temp_root(app_bundle).join(transaction_id)
}

fn swap_temp_root(app_bundle: &Path) -> std::path::PathBuf {
    app_bundle.join("Contents/.MorpheusLauncherTemp")
}

fn swap_paths(app_bundle: &Path, transaction_id: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let contents = app_bundle.join("Contents");
    (
        contents.join(format!(".MorpheusSwap-{transaction_id}-new")),
        contents.join(format!(".MorpheusSwap-{transaction_id}-old")),
    )
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|err| io_error(format!("sync {}", path.display()), err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactIdentity;
    use crate::LauncherState;
    use std::cell::Cell;

    struct FakeSigner {
        call: Cell<usize>,
        fail_candidate: bool,
        fail_restore: bool,
    }

    impl BundleSigner for FakeSigner {
        fn sign_and_verify(&self, _path: &Path) -> Result<()> {
            let call = self.call.get() + 1;
            self.call.set(call);
            if (call == 1 && self.fail_candidate) || (call == 2 && self.fail_restore) {
                return Err(LauncherError::Launch(format!("signing call {call} failed")));
            }
            Ok(())
        }
    }

    fn signer(fail_candidate: bool, fail_restore: bool) -> FakeSigner {
        FakeSigner {
            call: Cell::new(0),
            fail_candidate,
            fail_restore,
        }
    }

    #[test]
    fn load_rejects_commit_intent_outside_launching_state() {
        let fixture = activation_fixture();
        for state in [TransactionState::Prepared, TransactionState::RollingBack] {
            let mut transaction = fixture.transaction.borrow().clone();
            transaction.state = state;
            transaction.commit_cleanup_requested = true;
            write_json_atomic(&fixture.paths.transaction, &transaction).expect("transaction");

            assert!(load_transaction(&fixture.paths).is_err());
        }
    }

    #[test]
    fn activation_and_rollback_swap_only_three_resource_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_bundle = temp.path().join("Morpheus.app");
        let contents = app_bundle.join("Contents");
        let resources = contents.join("Resources");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("state");
        let candidate = candidate_store(&paths).expect("candidate store").join("tx");
        let old_identity = ArtifactIdentity {
            transaction_id: "installed".to_string(),
            build_id: "old".to_string(),
            source_commit: "commit".to_string(),
        };
        let current_snapshot =
            current_snapshot_path(&paths, &old_identity).expect("current snapshot");
        std::fs::create_dir_all(&resources).expect("resources");
        std::fs::create_dir_all(&candidate).expect("candidate");
        std::fs::create_dir_all(&current_snapshot).expect("current snapshot");
        std::fs::write(resources.join("old"), b"old").expect("old");
        std::fs::write(candidate.join("app.asar"), b"new").expect("new");
        std::fs::write(current_snapshot.join("old"), b"old").expect("snapshot old");
        let current = record("installed", "old", &app_bundle, &current_snapshot, "old");
        let mut state = LauncherState {
            schema_version: 1,
            current: Some(current.clone()),
            previous: None,
        };
        state.save(&paths).expect("save");
        let mut transaction = TransactionRecord {
            schema_version: 1,
            transaction_id: "tx".to_string(),
            transaction_type: TransactionType::Hot,
            state: TransactionState::Prepared,
            reason: String::new(),
            candidate: record("tx", "new", &app_bundle, &candidate, "app.asar"),
            previous: None,
            failure: None,
            commit_cleanup_requested: false,
        };
        write_json_atomic(&paths.transaction, &transaction).expect("transaction");
        let ActivationOutcome::Activated {
            current: activated,
            previous,
        } = activate_transaction_with_signer(
            &paths,
            &mut transaction,
            &current,
            &signer(false, false),
        )
        .expect("activate")
        else {
            panic!("activation unexpectedly recovered a signing failure");
        };
        state.current = Some(activated);
        state.previous = Some(previous);
        state.save(&paths).expect("save activated");
        assert!(resources.join("app.asar").is_file());
        assert!(current_snapshot.join("old").is_file());

        let failure = rollback_failure(&transaction, "test rollback").expect("failure");
        rollback_transaction_with_signer(
            &paths,
            &mut transaction,
            state,
            failure,
            &signer(false, false),
        )
        .expect("rollback");
        assert!(resources.join("old").is_file());
        assert!(!candidate.exists());
        assert!(current_snapshot.exists());
        assert_eq!(
            transaction.failure.as_ref().expect("durable failure").reason,
            "test rollback"
        );
    }

    #[test]
    fn reconcile_finishes_activation_after_first_rename() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_bundle = temp.path().join("Morpheus.app");
        let contents = app_bundle.join("Contents");
        let resources = contents.join("Resources");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("state");
        let candidate = candidate_store(&paths).expect("candidate store").join("tx");
        let old_identity = ArtifactIdentity {
            transaction_id: "installed".to_string(),
            build_id: "old".to_string(),
            source_commit: "commit".to_string(),
        };
        let previous = current_snapshot_path(&paths, &old_identity).expect("current snapshot");
        let (swap_new, swap_old) = swap_paths(&app_bundle, "tx");
        std::fs::create_dir_all(&candidate).expect("candidate");
        std::fs::create_dir_all(&previous).expect("previous");
        std::fs::create_dir_all(&swap_new).expect("swap new");
        std::fs::create_dir_all(&swap_old).expect("swap old");
        std::fs::write(swap_new.join("app.asar"), b"new").expect("new");
        ensure_snapshot(&swap_new, "new");
        std::fs::write(previous.join("old"), b"old").expect("old");
        let old = record("installed", "old", &app_bundle, &previous, "old");
        LauncherState {
            schema_version: 1,
            current: Some(old.clone()),
            previous: None,
        }
        .save(&paths)
        .expect("save");
        write_json_atomic(
            &paths.transaction,
            &TransactionRecord {
                schema_version: 1,
                transaction_id: "tx".to_string(),
                transaction_type: TransactionType::Full,
                state: TransactionState::Launching,
                reason: String::new(),
                candidate: record("tx", "new", &app_bundle, &candidate, "app.asar"),
                previous: Some(old),
                failure: None,
                commit_cleanup_requested: false,
            },
        )
        .expect("transaction");

        assert!(
            reconcile_transaction_with_signer(&paths, &signer(false, false))
                .expect("reconcile")
                .is_none()
        );
        assert!(resources.join("app.asar").is_file());
        let state = LauncherState::load(&paths).expect("state");
        assert_eq!(
            state.current.expect("current").identity.build_id,
            "new"
        );
    }

    #[test]
    fn candidate_signing_failure_restores_previous_resources() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow_mut();
        let outcome = activate_transaction_with_signer(
            &fixture.paths,
            &mut transaction,
            &fixture.current,
            &signer(true, false),
        )
        .expect("previous resources should be restored");
        let ActivationOutcome::RecoveredFailure(failure) = outcome else {
            panic!("candidate signing failure must be reported");
        };
        assert!(failure.error.to_string().contains("signing call 1 failed"));
        assert_eq!(failure.failed.identity.build_id, "new");
        assert_eq!(failure.fallback.identity.build_id, "old");
        assert!(fixture.resources.join("old").is_file());
        let durable = load_transaction(&fixture.paths)
            .expect("transaction")
            .expect("durable signing failure");
        assert!(durable.failure.is_some());
    }

    #[test]
    fn reconcile_signing_failure_restores_previous_resources() {
        let fixture = activation_fixture();
        {
            let mut transaction = fixture.transaction.borrow_mut();
            transaction.state = TransactionState::Launching;
            transaction.previous = Some(fixture.current.clone());
            std::fs::remove_dir_all(&fixture.resources).expect("remove old resources");
            copy_snapshot(&transaction.candidate.artifact_root, &fixture.resources)
                .expect("activate candidate");
            write_json_atomic(&fixture.paths.transaction, &*transaction)
                .expect("launching transaction");
        }

        let failure = reconcile_transaction_with_signer(&fixture.paths, &signer(true, false))
            .expect("restore previous")
            .expect("report recovered failure");
        assert!(failure.error.to_string().contains("signing call 1 failed"));
        assert!(fixture.resources.join("old").is_file());
        assert!(
            load_transaction(&fixture.paths)
                .expect("transaction")
                .expect("durable signing failure")
                .failure
                .is_some()
        );
    }

    #[test]
    fn activation_preflights_current_identity_collision_before_resource_swap() {
        let fixture = activation_fixture();
        let transaction = fixture.transaction.borrow();
        let collision =
            current_snapshot_path(&fixture.paths, &transaction.candidate.identity)
                .expect("collision path");
        copy_snapshot(&transaction.candidate.artifact_root, &collision)
            .expect("collision snapshot");
        let (swap_new, swap_old) =
            swap_paths(&transaction.candidate.app_bundle_path, &transaction.transaction_id);
        let mut attempted = transaction.clone();

        let error = activate_transaction_with_signer(
            &fixture.paths,
            &mut attempted,
            &fixture.current,
            &signer(false, false),
        )
        .expect_err("collision must fail before activation");

        assert!(error.to_string().contains("current snapshot identity already exists"));
        assert!(fixture.resources.join("old").is_file());
        assert!(!swap_new.exists());
        assert!(!swap_old.exists());
    }

    #[test]
    fn reconcile_preflights_current_identity_collision_before_resource_swap() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow_mut();
        transaction.state = TransactionState::Launching;
        transaction.previous = Some(fixture.current.clone());
        write_json_atomic(&fixture.paths.transaction, &*transaction).expect("transaction");
        let collision =
            current_snapshot_path(&fixture.paths, &transaction.candidate.identity)
                .expect("collision path");
        copy_snapshot(&transaction.candidate.artifact_root, &collision)
            .expect("collision snapshot");
        let (swap_new, swap_old) =
            swap_paths(&transaction.candidate.app_bundle_path, &transaction.transaction_id);
        copy_snapshot(&transaction.candidate.artifact_root, &swap_new).expect("swap snapshot");
        drop(transaction);
        let signer = signer(false, false);

        let error = reconcile_transaction_with_signer(&fixture.paths, &signer)
            .expect_err("collision must fail before reconcile mutates resources");

        assert!(error.to_string().contains("current snapshot identity already exists"));
        assert_eq!(signer.call.get(), 0);
        assert!(fixture.resources.join("old").is_file());
        assert!(!swap_old.exists());
    }

    #[cfg(unix)]
    #[test]
    fn reconcile_rejects_symlink_swap_root_before_moving_resources() {
        use std::os::unix::fs::symlink;

        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow_mut();
        transaction.state = TransactionState::Launching;
        transaction.previous = Some(fixture.current.clone());
        write_json_atomic(&fixture.paths.transaction, &*transaction).expect("transaction");
        let (swap_new, swap_old) =
            swap_paths(&transaction.candidate.app_bundle_path, &transaction.transaction_id);
        symlink(fixture.paths.root.join("missing-snapshot"), &swap_new).expect("swap symlink");
        drop(transaction);

        let error = reconcile_transaction_with_signer(&fixture.paths, &signer(false, false))
            .expect_err("symlink swap root must be rejected");

        assert!(error.to_string().contains("not a real directory"));
        assert!(fixture.resources.join("old").is_file());
        assert!(!swap_old.exists());
    }

    #[cfg(unix)]
    #[test]
    fn prepared_reconcile_rejects_symlink_contents_without_deleting_external_directory() {
        use std::os::unix::fs::symlink;

        let fixture = activation_fixture();
        fixture.transaction.borrow_mut().transaction_type = TransactionType::Hot;
        write_json_atomic(&fixture.paths.transaction, &*fixture.transaction.borrow())
            .expect("transaction");
        let contents = fixture
            .transaction
            .borrow()
            .candidate
            .app_bundle_path
            .join("Contents");
        std::fs::remove_dir_all(&contents).expect("remove contents");
        let external = fixture.paths.root.join("external-contents");
        let victim = external.join(".MorpheusSwap-tx-new");
        std::fs::create_dir_all(&victim).expect("victim");
        symlink(&external, &contents).expect("contents symlink");

        reconcile_transaction_with_signer(&fixture.paths, &signer(false, false))
            .expect_err("symlink Contents must be rejected");

        assert!(victim.exists());
        assert!(fixture.paths.transaction.exists());
    }

    #[test]
    fn rollback_removes_empty_temp_namespace_before_signing() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow_mut();
        transaction.state = TransactionState::RollingBack;
        transaction.previous = Some(fixture.current.clone());
        let failure = rollback_failure(&transaction, "test rollback").expect("failure");
        transaction.failure = Some(failure);
        write_json_atomic(&fixture.paths.transaction, &*transaction).expect("transaction");
        let (swap_new, _) =
            swap_paths(&transaction.candidate.app_bundle_path, &transaction.transaction_id);
        copy_snapshot(&fixture.current.artifact_root, &swap_new).expect("rollback swap");
        let temp_root = swap_temp_root(&transaction.candidate.app_bundle_path);
        std::fs::create_dir(&temp_root).expect("empty temp root");
        let signer = signer(false, false);

        restore_previous_resources(
            &fixture.paths,
            &mut transaction,
            LauncherState::load(&fixture.paths).expect("state"),
            &signer,
        )
        .expect("restore");

        assert_eq!(signer.call.get(), 1);
        assert!(!temp_root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn commit_keeps_cleanup_intent_when_previous_snapshot_is_unsafe() {
        use std::os::unix::fs::symlink;

        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow().clone();
        transaction.previous = Some(fixture.current.clone());
        write_json_atomic(&fixture.paths.transaction, &transaction).expect("transaction");
        std::fs::remove_dir_all(&fixture.current.artifact_root).expect("remove previous");
        symlink(
            fixture.paths.root.join("missing-previous"),
            &fixture.current.artifact_root,
        )
        .expect("unsafe previous");
        let state = LauncherState {
            schema_version: 1,
            current: Some(transaction.candidate.clone()),
            previous: Some(fixture.current.clone()),
        };
        state.save(&fixture.paths).expect("state");

        commit_transaction(&fixture.paths, &transaction, state)
            .expect_err("unsafe previous must keep cleanup intent");

        assert!(fixture.paths.transaction.exists());
        assert!(
            LauncherState::load(&fixture.paths)
                .expect("state")
                .previous
                .is_some()
        );
    }

    #[test]
    fn startup_reconcile_finishes_requested_commit_with_partial_previous_snapshot() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow().clone();
        transaction.state = TransactionState::Launching;
        transaction.previous = Some(fixture.current.clone());
        transaction.commit_cleanup_requested = true;
        std::fs::remove_dir_all(&fixture.resources).expect("remove resources");
        copy_snapshot(&transaction.candidate.artifact_root, &fixture.resources)
            .expect("active candidate resources");
        let promoted =
            current_snapshot_path(&fixture.paths, &transaction.candidate.identity)
                .expect("promoted path");
        std::fs::rename(&transaction.candidate.artifact_root, &promoted)
            .expect("promote candidate");
        transaction.candidate = rebase_artifact(&transaction.candidate, promoted);
        std::fs::remove_file(
            fixture
                .current
                .artifact_root
                .join(".morpheus-runtime-manifest.json"),
        )
        .expect("leave partial previous");
        let state = LauncherState {
            schema_version: 1,
            current: Some(transaction.candidate.clone()),
            previous: Some(fixture.current.clone()),
        };
        state.save(&fixture.paths).expect("state");
        write_json_atomic(&fixture.paths.transaction, &transaction).expect("transaction");

        assert!(
            reconcile_transaction_with_signer(&fixture.paths, &signer(false, false))
                .expect("startup reconcile")
                .is_none()
        );

        let reconciled = LauncherState::load(&fixture.paths).expect("state");
        assert_eq!(
            reconciled.current.expect("current").identity,
            transaction.candidate.identity
        );
        assert!(reconciled.previous.is_none());
        assert!(!fixture.paths.transaction.exists());
        assert!(fixture.resources.join("app.asar").is_file());
    }

    #[test]
    fn startup_reconcile_rejects_partial_previous_without_commit_intent() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow().clone();
        transaction.state = TransactionState::Launching;
        transaction.previous = Some(fixture.current.clone());
        std::fs::remove_dir_all(&fixture.resources).expect("remove resources");
        copy_snapshot(&transaction.candidate.artifact_root, &fixture.resources)
            .expect("active candidate resources");
        let promoted =
            current_snapshot_path(&fixture.paths, &transaction.candidate.identity)
                .expect("promoted path");
        std::fs::rename(&transaction.candidate.artifact_root, &promoted)
            .expect("promote candidate");
        transaction.candidate = rebase_artifact(&transaction.candidate, promoted);
        std::fs::remove_file(
            fixture
                .current
                .artifact_root
                .join(".morpheus-runtime-manifest.json"),
        )
        .expect("leave partial previous");
        LauncherState {
            schema_version: 1,
            current: Some(transaction.candidate.clone()),
            previous: Some(fixture.current.clone()),
        }
        .save(&fixture.paths)
        .expect("state");
        write_json_atomic(&fixture.paths.transaction, &transaction).expect("transaction");

        reconcile_transaction_with_signer(&fixture.paths, &signer(false, false))
            .expect_err("partial previous without intent must fail closed");

        assert!(fixture.paths.transaction.exists());
        assert!(
            LauncherState::load(&fixture.paths)
                .expect("state")
                .previous
                .is_some()
        );
    }

    #[test]
    fn startup_reconcile_does_not_commit_normal_promoted_launching_transaction() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow().clone();
        transaction.state = TransactionState::Launching;
        transaction.previous = Some(fixture.current.clone());
        std::fs::remove_dir_all(&fixture.resources).expect("remove resources");
        copy_snapshot(&transaction.candidate.artifact_root, &fixture.resources)
            .expect("active candidate resources");
        let promoted =
            current_snapshot_path(&fixture.paths, &transaction.candidate.identity)
                .expect("promoted path");
        std::fs::rename(&transaction.candidate.artifact_root, &promoted)
            .expect("promote candidate");
        transaction.candidate = rebase_artifact(&transaction.candidate, promoted);
        LauncherState {
            schema_version: 1,
            current: Some(transaction.candidate.clone()),
            previous: Some(fixture.current.clone()),
        }
        .save(&fixture.paths)
        .expect("state");
        write_json_atomic(&fixture.paths.transaction, &transaction).expect("transaction");
        let signer = signer(false, false);

        assert!(
            reconcile_transaction_with_signer(&fixture.paths, &signer)
                .expect("reconcile")
                .is_none()
        );

        assert_eq!(signer.call.get(), 0);
        assert!(fixture.paths.transaction.exists());
        assert!(
            LauncherState::load(&fixture.paths)
                .expect("state")
                .previous
                .is_some()
        );
    }

    #[test]
    fn maximum_transaction_id_keeps_swap_leaves_below_name_max() {
        let app_bundle = std::path::PathBuf::from("Morpheus.app");
        let transaction_id = "a".repeat(crate::artifact::MAX_IDENTIFIER_BYTES);
        let (swap_new, swap_old) = swap_paths(&app_bundle, &transaction_id);
        let swap_temp = swap_copying_path(&app_bundle, &transaction_id);
        for path in [swap_new, swap_old, swap_temp] {
            assert!(path.file_name().expect("leaf").len() < 255);
        }
    }

    #[test]
    fn prepared_reconcile_removes_only_transaction_copying_directory() {
        let fixture = activation_fixture();
        fixture.transaction.borrow_mut().transaction_type = TransactionType::Hot;
        write_json_atomic(&fixture.paths.transaction, &*fixture.transaction.borrow())
            .expect("hot transaction");
        let temp_store = candidate_temp_store(&fixture.paths).expect("candidate temp store");
        let copying = candidate_copying_path(&temp_store, "tx");
        let unrelated = copying.with_file_name("other.copying");
        std::fs::create_dir_all(&copying).expect("copying");
        std::fs::create_dir_all(&unrelated).expect("unrelated");

        assert!(
            reconcile_transaction_with_signer(&fixture.paths, &signer(false, false))
                .expect("reconcile")
                .is_none()
        );
        assert!(!copying.exists());
        assert!(unrelated.exists());
        assert!(!fixture.paths.transaction.exists());
        assert!(!fixture.transaction.borrow().candidate.artifact_root.exists());
    }

    #[test]
    fn abort_prepared_full_removes_candidate_and_copying_directory() {
        let fixture = activation_fixture();
        let transaction = fixture.transaction.borrow();
        let temp_store = candidate_temp_store(&fixture.paths).expect("candidate temp store");
        let copying = candidate_copying_path(&temp_store, "tx");
        std::fs::create_dir_all(&copying).expect("copying");

        abort_prepared_full(&fixture.paths, &transaction).expect("abort");
        assert!(!transaction.candidate.artifact_root.exists());
        assert!(!copying.exists());
        assert!(!fixture.paths.transaction.exists());
    }

    #[test]
    fn restore_signing_failure_returns_combined_error_and_keeps_rollback_durable() {
        let fixture = activation_fixture();
        let mut transaction = fixture.transaction.borrow_mut();
        let error = activate_transaction_with_signer(
            &fixture.paths,
            &mut transaction,
            &fixture.current,
            &signer(true, true),
        )
        .expect_err("both signing operations must fail");
        let message = error.to_string();
        assert!(message.contains("candidate signing failed"));
        assert!(message.contains("previous restore signing failed"));
        assert!(fixture.resources.join("old").is_file());
        assert_eq!(
            load_transaction(&fixture.paths)
                .expect("transaction")
                .expect("durable")
                .state,
            TransactionState::RollingBack
        );
    }

    struct ActivationFixture {
        paths: LauncherPaths,
        resources: std::path::PathBuf,
        current: ArtifactRecord,
        transaction: std::cell::RefCell<TransactionRecord>,
    }

    fn activation_fixture() -> ActivationFixture {
        let temp = tempfile::tempdir().expect("tempdir").keep();
        let app_bundle = temp.join("Morpheus.app");
        let resources = app_bundle.join("Contents/Resources");
        let paths = LauncherPaths::new(temp.join("state"));
        paths.ensure().expect("state");
        let candidate = candidate_store(&paths).expect("candidate store").join("tx");
        let old_identity = ArtifactIdentity {
            transaction_id: "installed".to_string(),
            build_id: "old".to_string(),
            source_commit: "commit".to_string(),
        };
        let current_snapshot =
            current_snapshot_path(&paths, &old_identity).expect("current snapshot");
        std::fs::create_dir_all(&resources).expect("resources");
        std::fs::create_dir_all(&candidate).expect("candidate");
        std::fs::create_dir_all(&current_snapshot).expect("current snapshot");
        std::fs::write(resources.join("old"), b"old").expect("old");
        std::fs::write(candidate.join("app.asar"), b"new").expect("new");
        std::fs::write(current_snapshot.join("old"), b"old").expect("snapshot old");
        let current = record("installed", "old", &app_bundle, &current_snapshot, "old");
        LauncherState {
            schema_version: 1,
            current: Some(current.clone()),
            previous: None,
        }
        .save(&paths)
        .expect("state");
        let transaction = TransactionRecord {
            schema_version: 1,
            transaction_id: "tx".to_string(),
            transaction_type: TransactionType::Full,
            state: TransactionState::Prepared,
            reason: String::new(),
            candidate: record("tx", "new", &app_bundle, &candidate, "app.asar"),
            previous: None,
            failure: None,
            commit_cleanup_requested: false,
        };
        write_json_atomic(&paths.transaction, &transaction).expect("transaction");
        ActivationFixture {
            paths,
            resources,
            current,
            transaction: std::cell::RefCell::new(transaction),
        }
    }

    fn record(
        transaction_id: &str,
        build_id: &str,
        app_bundle: &Path,
        root: &Path,
        _entrypoint: &str,
    ) -> ArtifactRecord {
        ensure_snapshot(root, build_id);
        ArtifactRecord {
            identity: ArtifactIdentity {
                transaction_id: transaction_id.to_string(),
                build_id: build_id.to_string(),
                source_commit: "commit".to_string(),
            },
            artifact_root: root.to_path_buf(),
            app_bundle_path: app_bundle.to_path_buf(),
            entrypoint: root.join("app.asar"),
            installed_at_unix_ms: 1,
        }
    }

    fn ensure_snapshot(root: &Path, build_id: &str) {
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::fs::create_dir_all(root.join("default-config/compact")).expect("compact");
        for (relative, contents) in [
            ("app.asar", b"runtime".as_slice()),
            ("bin/app-server", b"server".as_slice()),
            ("default-config/compact/COMPACT.md", b"compact".as_slice()),
        ] {
            let path = root.join(relative);
            if !path.exists() {
                std::fs::write(&path, contents).expect("snapshot artifact");
            }
        }
        let manifest = crate::ArtifactManifest {
            schema_version: 1,
            build_id: build_id.to_string(),
            source_commit: "commit".to_string(),
            entrypoint: std::path::PathBuf::from("app.asar"),
            artifacts: [
                "app.asar",
                "bin/app-server",
                "default-config/compact/COMPACT.md",
            ]
                .into_iter()
                .map(|relative| crate::ArtifactManifestEntry {
                    relative_path: std::path::PathBuf::from(relative),
                    sha256: {
                        use sha2::Digest;
                        format!(
                            "{:x}",
                            sha2::Sha256::digest(
                                std::fs::read(root.join(relative)).expect("artifact")
                            )
                        )
                    },
                })
                .collect(),
        };
        std::fs::write(
            root.join(".morpheus-runtime-manifest.json"),
            serde_json::to_vec(&manifest).expect("manifest"),
        )
        .expect("write manifest");
    }
}
