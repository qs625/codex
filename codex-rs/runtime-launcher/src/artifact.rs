use crate::LauncherError;
use crate::Result;
use crate::io_error;
use crate::platform;
use crate::state::ArtifactIdentity;
use crate::state::ArtifactRecord;
use crate::state::LauncherPaths;
use crate::state::SCHEMA_VERSION;
use crate::state::unix_time_ms;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

const INSTALLED_MANIFEST: &str = ".morpheus-runtime-manifest.json";
const REQUIRED_RUNTIME_ARTIFACTS: [&str; 3] = [
    "app.asar",
    "bin/app-server",
    "default-config/compact/COMPACT.md",
];
pub(crate) const MAX_IDENTIFIER_BYTES: usize = 96;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedArtifactRequest {
    pub schema_version: u32,
    pub transaction_id: String,
    pub build_id: String,
    pub source_commit: String,
    pub prepared_root: PathBuf,
    pub app_bundle_path: PathBuf,
    pub reason: String,
    #[serde(default)]
    pub changes: crate::transaction::ActivationChanges,
}

impl PreparedArtifactRequest {
    pub(crate) fn validate_common(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(LauncherError::InvalidRequest(format!(
                "unsupported request schema {}",
                self.schema_version
            )));
        }
        validate_identifier("transactionId", &self.transaction_id)?;
        validate_identifier("buildId", &self.build_id)?;
        if self.source_commit.trim().is_empty() || self.source_commit.len() > 160 {
            return Err(LauncherError::InvalidRequest(
                "sourceCommit must contain 1 to 160 bytes".to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) fn identity(&self) -> ArtifactIdentity {
        ArtifactIdentity {
            transaction_id: self.transaction_id.clone(),
            build_id: self.build_id.clone(),
            source_commit: self.source_commit.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub build_id: String,
    pub source_commit: String,
    pub entrypoint: PathBuf,
    pub artifacts: Vec<ArtifactManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifestEntry {
    pub relative_path: PathBuf,
    pub sha256: String,
}

pub(crate) fn install_prepared_artifact(
    paths: &LauncherPaths,
    request: &PreparedArtifactRequest,
) -> Result<ArtifactRecord> {
    let planned = plan_prepared_artifact(paths, request)?;
    let source = canonical_directory(&request.prepared_root)?;
    let manifest = load_and_validate_manifest(&source, request)?;
    let app_bundle = planned.app_bundle_path.clone();
    canonical_directory(&app_bundle.join("Contents"))?;
    let artifacts = candidate_store(paths)?;
    let temp = candidate_temp_store(paths)?;
    let destination = planned.artifact_root.clone();
    publish_prepared_artifact(
        request,
        &source,
        &manifest,
        &app_bundle,
        &artifacts,
        &temp,
        &destination,
    )
}

pub(crate) fn plan_prepared_artifact(
    paths: &LauncherPaths,
    request: &PreparedArtifactRequest,
) -> Result<ArtifactRecord> {
    request.validate_common()?;
    platform::verify_app_bundle(&request.app_bundle_path)?;
    let source = canonical_directory(&request.prepared_root)?;
    let manifest = load_and_validate_manifest(&source, request)?;
    let app_bundle = canonical_directory(&request.app_bundle_path)?;
    let destination = candidate_store(paths)?.join(&request.transaction_id);
    Ok(ArtifactRecord {
        identity: request.identity(),
        entrypoint: destination.join(&manifest.entrypoint),
        artifact_root: destination,
        app_bundle_path: app_bundle,
        installed_at_unix_ms: unix_time_ms(),
    })
}

fn publish_prepared_artifact(
    request: &PreparedArtifactRequest,
    source: &Path,
    manifest: &ArtifactManifest,
    app_bundle: &Path,
    artifact_store: &Path,
    temp_store: &Path,
    destination: &Path,
) -> Result<ArtifactRecord> {
    let copying = candidate_copying_path(temp_store, &request.transaction_id);
    if real_snapshot_directory_exists(destination)? {
        validate_existing_staged_directory(destination, artifact_store)?;
        let installed = read_installed_manifest(&destination)?;
        if &installed != manifest {
            return Err(LauncherError::Conflict(format!(
                "staging identity differs for transaction {}",
                request.transaction_id
            )));
        }
        validate_resource_tree(destination, &installed, true)?;
        return Ok(ArtifactRecord {
            identity: request.identity(),
            entrypoint: destination.join(&installed.entrypoint),
            artifact_root: destination.to_path_buf(),
            app_bundle_path: app_bundle.to_path_buf(),
            installed_at_unix_ms: unix_time_ms(),
        });
    }
    if real_snapshot_directory_exists(&copying)? {
        return Err(LauncherError::Conflict(format!(
            "staging already exists for transaction {}",
            request.transaction_id
        )));
    }
    let publish_result = (|| {
        std::fs::create_dir(&copying)
            .map_err(|err| io_error(format!("create {}", copying.display()), err))?;
        for artifact in &manifest.artifacts {
            let source_path = source.join("resources").join(&artifact.relative_path);
            let target = copying.join(&artifact.relative_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| io_error(format!("create {}", parent.display()), err))?;
            }
            std::fs::copy(&source_path, &target)
                .map_err(|err| io_error(format!("copy {}", source_path.display()), err))?;
        }
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|err| crate::json_error("serialize installed runtime manifest", err))?;
        std::fs::write(copying.join(INSTALLED_MANIFEST), manifest_bytes)
            .map_err(|err| io_error("write installed runtime manifest", err))?;
        validate_resource_tree(&copying, manifest, true)?;
        sync_tree(&copying)?;
        std::fs::rename(&copying, &destination)
            .map_err(|err| io_error(format!("install {}", destination.display()), err))
    })();
    if let Err(error) = publish_result {
        let _ = remove_real_directory_if_exists(&copying, temp_store);
        return Err(error);
    }
    sync_directory(temp_store)?;
    sync_directory(artifact_store)?;
    Ok(ArtifactRecord {
        identity: request.identity(),
        entrypoint: destination.join(&manifest.entrypoint),
        artifact_root: destination.to_path_buf(),
        app_bundle_path: app_bundle.to_path_buf(),
        installed_at_unix_ms: unix_time_ms(),
    })
}

pub(crate) fn artifact_store(paths: &LauncherPaths) -> Result<PathBuf> {
    paths.ensure()?;
    reject_symlink(&paths.root)?;
    let store = paths.root.join("artifacts");
    match std::fs::symlink_metadata(&store) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(LauncherError::Conflict(format!(
                "artifact store is not a real directory: {}",
                store.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&store)
                .map_err(|error| io_error(format!("create {}", store.display()), error))?;
        }
        Err(error) => return Err(io_error(format!("inspect {}", store.display()), error)),
    }
    let store = canonical_directory(&store)?;
    ensure_namespace(&store.join("candidates"))?;
    ensure_namespace(&store.join("current"))?;
    let temp = store.join("temp");
    ensure_namespace(&temp)?;
    ensure_namespace(&temp.join("candidates"))?;
    ensure_namespace(&temp.join("current"))?;
    Ok(store)
}

pub(crate) fn candidate_store(paths: &LauncherPaths) -> Result<PathBuf> {
    Ok(artifact_store(paths)?.join("candidates"))
}

pub(crate) fn current_store(paths: &LauncherPaths) -> Result<PathBuf> {
    Ok(artifact_store(paths)?.join("current"))
}

pub(crate) fn candidate_temp_store(paths: &LauncherPaths) -> Result<PathBuf> {
    Ok(artifact_store(paths)?.join("temp/candidates"))
}

fn current_temp_store(paths: &LauncherPaths) -> Result<PathBuf> {
    Ok(artifact_store(paths)?.join("temp/current"))
}

fn ensure_namespace(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(LauncherError::Conflict(format!(
                "artifact namespace is not a real directory: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(path)
            .map_err(|error| io_error(format!("create {}", path.display()), error)),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

pub(crate) fn candidate_copying_path(temp_store: &Path, transaction_id: &str) -> PathBuf {
    temp_store.join(transaction_id)
}

pub(crate) fn artifact_identity_key(identity: &ArtifactIdentity) -> String {
    let mut digest = Sha256::new();
    for value in [
        identity.transaction_id.as_bytes(),
        identity.build_id.as_bytes(),
        identity.source_commit.as_bytes(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    format!("{:x}", digest.finalize())
}

pub(crate) fn current_snapshot_path(
    paths: &LauncherPaths,
    identity: &ArtifactIdentity,
) -> Result<PathBuf> {
    Ok(current_store(paths)?.join(artifact_identity_key(identity)))
}

fn validate_existing_staged_directory(path: &Path, expected_parent: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "staged candidate is not a real directory: {}",
            path.display()
        )));
    }
    let parent = canonical_directory(path.parent().ok_or_else(|| {
        LauncherError::Conflict("staged candidate has no parent directory".to_string())
    })?)?;
    if parent != expected_parent {
        return Err(LauncherError::Conflict(format!(
            "staged candidate escapes app bundle parent: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn rebase_artifact(artifact: &ArtifactRecord, root: PathBuf) -> ArtifactRecord {
    let relative_entrypoint = artifact
        .entrypoint
        .strip_prefix(&artifact.artifact_root)
        .unwrap_or(&artifact.entrypoint);
    let mut rebased = artifact.clone();
    rebased.entrypoint = root.join(relative_entrypoint);
    rebased.artifact_root = root;
    rebased
}

pub(crate) fn load_and_validate_manifest(
    prepared_root: &Path,
    request: &PreparedArtifactRequest,
) -> Result<ArtifactManifest> {
    let manifest_path = prepared_root.join("manifest.json");
    reject_symlink(&manifest_path)?;
    let bytes = std::fs::read(&manifest_path)
        .map_err(|err| io_error(format!("read {}", manifest_path.display()), err))?;
    let manifest: ArtifactManifest = serde_json::from_slice(&bytes)
        .map_err(|err| crate::json_error(format!("parse {}", manifest_path.display()), err))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(LauncherError::InvalidArtifact(format!(
            "unsupported manifest schema {}",
            manifest.schema_version
        )));
    }
    if manifest.build_id != request.build_id || manifest.source_commit != request.source_commit {
        return Err(LauncherError::InvalidArtifact(
            "manifest identity does not match request".to_string(),
        ));
    }
    validate_resource_tree(
        &canonical_directory(&prepared_root.join("resources"))?,
        &manifest,
        true,
    )?;
    Ok(manifest)
}

fn validate_resource_tree(
    resources: &Path,
    manifest: &ArtifactManifest,
    reject_undeclared: bool,
) -> Result<()> {
    validate_relative_path(&manifest.entrypoint)?;
    if manifest.artifacts.len() != REQUIRED_RUNTIME_ARTIFACTS.len() {
        return Err(LauncherError::InvalidArtifact(
            "manifest must contain exactly three runtime artifacts".to_string(),
        ));
    }
    let mut entrypoint_declared = false;
    let mut required_found = [false; REQUIRED_RUNTIME_ARTIFACTS.len()];
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.relative_path)?;
        entrypoint_declared |= artifact.relative_path == manifest.entrypoint;
        for (index, required) in REQUIRED_RUNTIME_ARTIFACTS.iter().enumerate() {
            required_found[index] |= artifact.relative_path == Path::new(required);
        }
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(LauncherError::InvalidArtifact(format!(
                "invalid sha256 for {}",
                artifact.relative_path.display()
            )));
        }
        reject_symlink_chain(resources, &artifact.relative_path)?;
        let actual = sha256_file(&resources.join(&artifact.relative_path))?;
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            return Err(LauncherError::InvalidArtifact(format!(
                "sha256 mismatch for {}",
                artifact.relative_path.display()
            )));
        }
    }
    let declared = manifest
        .artifacts
        .iter()
        .map(|artifact| artifact.relative_path.clone())
        .collect::<BTreeSet<_>>();
    if declared.len() != manifest.artifacts.len() {
        return Err(LauncherError::InvalidArtifact(
            "manifest contains duplicate relativePath entries".to_string(),
        ));
    }
    if !entrypoint_declared || !resources.join(&manifest.entrypoint).is_file() {
        return Err(LauncherError::InvalidArtifact(
            "entrypoint must be a declared artifact file".to_string(),
        ));
    }
    if let Some(missing) = REQUIRED_RUNTIME_ARTIFACTS
        .iter()
        .zip(required_found)
        .find_map(|(path, found)| (!found).then_some(path))
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "manifest must declare required runtime artifact {missing}"
        )));
    }
    let actual = collect_leaf_paths(resources)?;
    let installed_manifest = PathBuf::from(INSTALLED_MANIFEST);
    if reject_undeclared
        && actual
            .iter()
            .any(|path| path != &installed_manifest && !declared.contains(path))
    {
        return Err(LauncherError::InvalidArtifact(
            "resources contain an undeclared leaf artifact".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn discover_bundle_artifact(app_bundle: &Path) -> Result<ArtifactRecord> {
    platform::verify_app_bundle(app_bundle)?;
    let app_bundle = canonical_directory(app_bundle)?;
    let resources = canonical_directory(&app_bundle.join("Contents/Resources"))?;
    let manifest_path = resources.join(INSTALLED_MANIFEST);
    reject_symlink(&manifest_path)?;
    let bytes = std::fs::read(&manifest_path)
        .map_err(|err| io_error(format!("read {}", manifest_path.display()), err))?;
    let manifest: ArtifactManifest = serde_json::from_slice(&bytes)
        .map_err(|err| crate::json_error("parse installed runtime manifest", err))?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(LauncherError::InvalidArtifact(format!(
            "unsupported manifest schema {}",
            manifest.schema_version
        )));
    }
    validate_identifier("buildId", &manifest.build_id)?;
    validate_resource_tree(&resources, &manifest, false)?;
    Ok(ArtifactRecord {
        identity: ArtifactIdentity {
            transaction_id: "installed".to_string(),
            build_id: manifest.build_id,
            source_commit: manifest.source_commit,
        },
        entrypoint: resources.join(manifest.entrypoint),
        artifact_root: resources,
        app_bundle_path: app_bundle,
        installed_at_unix_ms: unix_time_ms(),
    })
}

pub(crate) fn snapshot_bundle_artifact(
    paths: &LauncherPaths,
    app_bundle: &Path,
) -> Result<ArtifactRecord> {
    let discovered = discover_bundle_artifact(app_bundle)?;
    let store = current_store(paths)?;
    let snapshot_name = artifact_identity_key(&discovered.identity);
    let destination = store.join(&snapshot_name);
    let temp_store = current_temp_store(paths)?;
    let copying = temp_store.join(&snapshot_name);
    if !real_snapshot_directory_exists(&destination)? {
        remove_real_directory_if_exists(&copying, &temp_store)?;
        copy_snapshot(&discovered.artifact_root, &copying)?;
        validate_artifact_snapshot_at(&discovered, &copying)?;
        std::fs::rename(&copying, &destination)
            .map_err(|error| io_error(format!("install {}", destination.display()), error))?;
        sync_directory(&temp_store)?;
        sync_directory(&store)?;
    } else {
        validate_existing_staged_directory(&destination, &store)?;
        validate_artifact_snapshot_at(&discovered, &destination)?;
    }
    Ok(rebase_artifact(&discovered, destination))
}

pub(crate) fn validate_artifact_snapshot(artifact: &ArtifactRecord) -> Result<()> {
    validate_artifact_snapshot_at(artifact, &artifact.artifact_root)
}

pub(crate) fn validate_artifact_snapshot_at(
    artifact: &ArtifactRecord,
    root: &Path,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| io_error(format!("inspect {}", root.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "snapshot root is not a real directory: {}",
            root.display()
        )));
    }
    let manifest = read_installed_manifest(root)?;
    if manifest.build_id != artifact.identity.build_id
        || manifest.source_commit != artifact.identity.source_commit
    {
        return Err(LauncherError::InvalidArtifact(
            "snapshot manifest identity does not match artifact record".to_string(),
        ));
    }
    let expected_entrypoint = artifact
        .entrypoint
        .strip_prefix(&artifact.artifact_root)
        .map_err(|_| {
            LauncherError::InvalidArtifact(
                "artifact entrypoint escapes its snapshot root".to_string(),
            )
        })?;
    if expected_entrypoint != manifest.entrypoint {
        return Err(LauncherError::InvalidArtifact(
            "snapshot manifest entrypoint does not match artifact record".to_string(),
        ));
    }
    validate_resource_tree(root, &manifest, false)?;
    collect_leaf_paths(root)?;
    Ok(())
}

pub(crate) fn copy_snapshot(source: &Path, destination: &Path) -> Result<()> {
    if real_snapshot_directory_exists(destination)? {
        return Err(LauncherError::Conflict(format!(
            "snapshot destination already exists: {}",
            destination.display()
        )));
    }
    std::fs::create_dir(destination)
        .map_err(|error| io_error(format!("create {}", destination.display()), error))?;
    let result = copy_snapshot_contents(source, destination);
    if let Err(error) = result {
        if let Some(parent) = destination.parent() {
            let _ = remove_real_directory_if_exists(destination, parent);
        }
        return Err(error);
    }
    sync_tree(destination)
}

fn remove_real_directory_if_exists(path: &Path, intended_parent: &Path) -> Result<()> {
    let canonical_parent = std::fs::canonicalize(intended_parent)
        .map_err(|error| io_error(format!("canonicalize {}", intended_parent.display()), error))?;
    let path_parent = path.parent().ok_or_else(|| {
        LauncherError::Conflict("snapshot temporary path has no parent".to_string())
    })?;
    if std::fs::canonicalize(path_parent)
        .map_err(|error| io_error(format!("canonicalize {}", path_parent.display()), error))?
        != canonical_parent
        || path == intended_parent
    {
        return Err(LauncherError::Conflict(format!(
            "snapshot temporary path is outside its intended parent: {}",
            path.display()
        )));
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(format!("inspect {}", path.display()), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "snapshot temporary path is not a real directory: {}",
            path.display()
        )));
    }
    std::fs::remove_dir_all(path)
        .map_err(|error| io_error(format!("remove {}", path.display()), error))
}

fn copy_snapshot_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in std::fs::read_dir(source)
        .map_err(|error| io_error(format!("read {}", source.display()), error))?
    {
        let entry = entry.map_err(|error| io_error(format!("read {}", source.display()), error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| io_error(format!("inspect {}", entry.path().display()), error))?;
        if file_type.is_symlink() {
            return Err(LauncherError::InvalidArtifact(format!(
                "symlink is not allowed: {}",
                entry.path().display()
            )));
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            std::fs::create_dir(&target)
                .map_err(|error| io_error(format!("create {}", target.display()), error))?;
            copy_snapshot_contents(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(|error| io_error(format!("copy {}", entry.path().display()), error))?;
        }
    }
    Ok(())
}

pub(crate) fn validate_identifier(name: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || matches!(
            value,
            "." | ".." | "artifacts" | "candidates" | "current" | "temp"
        )
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(LauncherError::InvalidRequest(format!(
            "{name} contains unsafe characters"
        )));
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "unsafe relative path {}",
            path.display()
        )));
    }
    Ok(())
}

fn real_snapshot_directory_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(LauncherError::Conflict(format!(
                "snapshot path is occupied by an untrusted entry: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    reject_symlink(path)?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|err| io_error(format!("canonicalize {}", path.display()), err))?;
    if !canonical.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    Ok(canonical)
}

fn reject_symlink(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| io_error(format!("inspect {}", path.display()), err))?;
    if metadata.file_type().is_symlink() {
        return Err(LauncherError::InvalidArtifact(format!(
            "symlink is not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_chain(root: &Path, relative: &Path) -> Result<()> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(LauncherError::InvalidArtifact(format!(
                "unsafe path {}",
                relative.display()
            )));
        };
        path.push(component);
        reject_symlink(&path)?;
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).map_err(|err| io_error(format!("open {}", path.display()), err))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| io_error(format!("read {}", path.display()), err))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn read_installed_manifest(resources: &Path) -> Result<ArtifactManifest> {
    let path = resources.join(INSTALLED_MANIFEST);
    reject_symlink(&path)?;
    let bytes =
        std::fs::read(&path).map_err(|err| io_error(format!("read {}", path.display()), err))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| crate::json_error(format!("parse {}", path.display()), err))
}

fn collect_leaf_paths(root: &Path) -> Result<BTreeSet<PathBuf>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeSet<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(current)
            .map_err(|err| io_error(format!("read {}", current.display()), err))?
        {
            let entry =
                entry.map_err(|err| io_error(format!("read {}", current.display()), err))?;
            let file_type = entry
                .file_type()
                .map_err(|err| io_error(format!("inspect {}", entry.path().display()), err))?;
            if file_type.is_symlink() {
                return Err(LauncherError::InvalidArtifact(format!(
                    "symlink is not allowed: {}",
                    entry.path().display()
                )));
            }
            if file_type.is_dir() {
                visit(root, &entry.path(), result)?;
            } else if file_type.is_file() {
                result.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| {
                            LauncherError::InvalidArtifact(
                                "resource leaf escaped its root".to_string(),
                            )
                        })?
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut result = BTreeSet::new();
    visit(root, root, &mut result)?;
    Ok(result)
}

fn sync_tree(root: &Path) -> Result<()> {
    for entry in std::fs::read_dir(root)
        .map_err(|err| io_error(format!("read {}", root.display()), err))?
    {
        let entry = entry.map_err(|err| io_error(format!("read {}", root.display()), err))?;
        if entry
            .file_type()
            .map_err(|err| io_error(format!("inspect {}", entry.path().display()), err))?
            .is_dir()
        {
            sync_tree(&entry.path())?;
        } else {
            File::open(entry.path())
                .and_then(|file| file.sync_all())
                .map_err(|err| io_error(format!("sync {}", entry.path().display()), err))?;
        }
    }
    sync_directory(root)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|err| io_error(format!("sync {}", path.display()), err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn manifest_rejects_path_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(temp.path().join("resources")).expect("resources");
        let request = fixture_request(temp.path());
        let manifest = ArtifactManifest {
            schema_version: 1,
            build_id: "build".to_string(),
            source_commit: "commit".to_string(),
            entrypoint: PathBuf::from("../escape"),
            artifacts: vec![],
        };
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize"),
        )
        .expect("write");
        assert!(load_and_validate_manifest(temp.path(), &request).is_err());
    }

    #[test]
    fn identifier_rejects_dot_segments_and_reserved_names() {
        for value in [".", "..", "artifacts", "candidates", "current", "temp"] {
            assert!(validate_identifier("transactionId", value).is_err());
        }
        assert!(validate_identifier("transactionId", "release.1").is_ok());
        assert!(validate_identifier("transactionId", &"a".repeat(MAX_IDENTIFIER_BYTES)).is_ok());
        assert!(
            validate_identifier("transactionId", &"a".repeat(MAX_IDENTIFIER_BYTES + 1)).is_err()
        );
    }

    #[test]
    fn identity_key_is_fixed_length_and_unambiguous() {
        let first = ArtifactIdentity {
            transaction_id: "c".to_string(),
            build_id: "a-b".to_string(),
            source_commit: "commit".to_string(),
        };
        let second = ArtifactIdentity {
            transaction_id: "b-c".to_string(),
            build_id: "a".to_string(),
            source_commit: "commit".to_string(),
        };
        assert_eq!(artifact_identity_key(&first).len(), 64);
        assert_ne!(artifact_identity_key(&first), artifact_identity_key(&second));
        let changed_commit = ArtifactIdentity {
            source_commit: "other".to_string(),
            ..first.clone()
        };
        assert_ne!(
            artifact_identity_key(&first),
            artifact_identity_key(&changed_commit)
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_snapshot_rejects_dangling_destination_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir(&source).expect("source");
        symlink(temp.path().join("missing"), &destination).expect("destination symlink");

        let error = copy_snapshot(&source, &destination)
            .expect_err("dangling symlink must count as occupied");

        assert!(error.to_string().contains("untrusted entry"));
        assert!(
            std::fs::symlink_metadata(&destination)
                .expect("symlink remains")
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn validates_declared_hash_for_app_asar() {
        let temp = tempfile::tempdir().expect("tempdir");
        let resources = temp.path().join("resources");
        std::fs::create_dir(&resources).expect("resources");
        for (relative, contents) in [
            ("app.asar", b"runtime".as_slice()),
            ("bin/app-server", b"server".as_slice()),
            ("default-config/compact/COMPACT.md", b"compact".as_slice()),
        ] {
            let path = resources.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("parent");
            let mut file = File::create(path).expect("artifact");
            file.write_all(contents).expect("write");
        }
        let manifest = ArtifactManifest {
            schema_version: 1,
            build_id: "build".to_string(),
            source_commit: "commit".to_string(),
            entrypoint: PathBuf::from("app.asar"),
            artifacts: REQUIRED_RUNTIME_ARTIFACTS
                .iter()
                .map(|relative| ArtifactManifestEntry {
                    relative_path: PathBuf::from(*relative),
                    sha256: sha256_file(&resources.join(relative)).expect("hash"),
                })
                .collect(),
        };
        std::fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize"),
        )
        .expect("write");
        load_and_validate_manifest(temp.path(), &fixture_request(temp.path())).expect("valid");
    }

    fn fixture_request(root: &Path) -> PreparedArtifactRequest {
        PreparedArtifactRequest {
            schema_version: 1,
            transaction_id: "tx".to_string(),
            build_id: "build".to_string(),
            source_commit: "commit".to_string(),
            prepared_root: root.to_path_buf(),
            app_bundle_path: root.to_path_buf(),
            reason: "test".to_string(),
            changes: Default::default(),
        }
    }
}
