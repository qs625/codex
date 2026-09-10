use crate::LauncherError;
use crate::Result;
use crate::io_error;
use crate::json_error;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[cfg(unix)]
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::CString;
use std::fs::File;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::io::Read;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

const CAPSULE_MANIFEST: &str = "capsule.json";
const CAPSULE_SCHEMA_VERSION: u32 = 2;
const RELEASE_ID_PREFIX: &str = "sha256:";
const RELEASE_PREIMAGE_DOMAIN: &[u8] = b"runtime-capsule-v1\0";
const PROCESS_SUPERVISION_CONTRACT: &str = "cooperative-observed-v1";
const PROHIBITED_PROCESS_BEHAVIORS: [&str; 4] =
    ["daemonize", "double-fork", "setsid", "process-group-escape"];
const LEGACY_PROHIBITED_PROCESS_BEHAVIORS: [&str; 3] = ["daemonize", "double-fork", "setsid"];
const MAX_ACTIVATION_ID_BYTES: usize = 96;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapsuleManifest {
    pub schema_version: u32,
    pub release_id: String,
    pub target: CapsuleTarget,
    pub launch: CapsuleLaunch,
    pub process_supervision: CapsuleProcessSupervision,
    pub entries: Vec<CapsuleEntry>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapsuleProcessSupervision {
    pub contract: String,
    pub prohibited_behaviors: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapsuleTarget {
    pub os: String,
    pub arch: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapsuleLaunch {
    pub executable: String,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapsuleEntry {
    Directory {
        path: String,
    },
    File {
        path: String,
        sha256: String,
        #[serde(default)]
        executable: bool,
    },
    Symlink {
        path: String,
        target: String,
    },
}

impl CapsuleEntry {
    pub fn path(&self) -> &str {
        match self {
            Self::Directory { path } | Self::File { path, .. } | Self::Symlink { path, .. } => path,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::Directory { .. } => "directory",
            Self::File { .. } => "file",
            Self::Symlink { .. } => "symlink",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapsuleRecord {
    pub release_id: String,
    pub digest: String,
    pub root: PathBuf,
    pub executable: PathBuf,
    pub cwd: Option<PathBuf>,
    pub manifest: CapsuleManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportRequest {
    pub state_root: PathBuf,
    pub activation_id: String,
    pub target: CapsuleTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportReceipt {
    schema_version: u32,
    activation_id: String,
    release_id: String,
    digest: String,
}

impl ImportRequest {
    pub fn import(&self) -> Result<CapsuleRecord> {
        import_incoming(&self.state_root, &self.activation_id, &self.target)
    }
}

pub fn compute_release_preimage(manifest: &CapsuleManifest) -> Result<Vec<u8>> {
    let entries = validate_manifest_structure(manifest, None)?;
    let mut preimage = RELEASE_PREIMAGE_DOMAIN.to_vec();
    push_frame(
        &mut preimage,
        "schema",
        &manifest.schema_version.to_be_bytes(),
    )?;
    push_frame(&mut preimage, "target.os", manifest.target.os.as_bytes())?;
    push_frame(
        &mut preimage,
        "target.arch",
        manifest.target.arch.as_bytes(),
    )?;
    push_frame(
        &mut preimage,
        "launch.executable",
        manifest.launch.executable.as_bytes(),
    )?;
    match &manifest.launch.cwd {
        Some(cwd) => {
            push_frame(&mut preimage, "launch.cwd.present", &[1])?;
            push_frame(&mut preimage, "launch.cwd", cwd.as_bytes())?;
        }
        None => {
            push_frame(&mut preimage, "launch.cwd.present", &[0])?;
        }
    }
    push_frame(
        &mut preimage,
        "launch.arguments.count",
        &usize_as_u64(manifest.launch.arguments.len())?.to_be_bytes(),
    )?;
    for (index, argument) in manifest.launch.arguments.iter().enumerate() {
        push_frame(
            &mut preimage,
            &format!("launch.argument.{index}"),
            argument.as_bytes(),
        )?;
    }
    push_frame(
        &mut preimage,
        "process_supervision.contract",
        manifest.process_supervision.contract.as_bytes(),
    )?;
    push_frame(
        &mut preimage,
        "process_supervision.prohibited.count",
        &usize_as_u64(manifest.process_supervision.prohibited_behaviors.len())?.to_be_bytes(),
    )?;
    for behavior in &manifest.process_supervision.prohibited_behaviors {
        push_frame(
            &mut preimage,
            "process_supervision.prohibited",
            behavior.as_bytes(),
        )?;
    }
    push_frame(
        &mut preimage,
        "entries.count",
        &usize_as_u64(entries.len())?.to_be_bytes(),
    )?;
    for (index, entry) in entries.values().enumerate() {
        push_frame(
            &mut preimage,
            &format!("entry.{index}.path"),
            entry.path().as_bytes(),
        )?;
        push_frame(
            &mut preimage,
            &format!("entry.{index}.type"),
            entry.kind_name().as_bytes(),
        )?;
        match entry {
            CapsuleEntry::Directory { .. } => {}
            CapsuleEntry::File {
                sha256, executable, ..
            } => {
                push_frame(
                    &mut preimage,
                    &format!("entry.{index}.sha256"),
                    &decode_sha256(sha256)?,
                )?;
                push_frame(
                    &mut preimage,
                    &format!("entry.{index}.executable"),
                    &[u8::from(*executable)],
                )?;
            }
            CapsuleEntry::Symlink { target, .. } => {
                push_frame(
                    &mut preimage,
                    &format!("entry.{index}.target"),
                    target.as_bytes(),
                )?;
            }
        }
    }
    Ok(preimage)
}

pub fn compute_release_id(manifest: &CapsuleManifest) -> Result<String> {
    let preimage = compute_release_preimage(manifest)?;
    Ok(format!("{RELEASE_ID_PREFIX}{:x}", Sha256::digest(preimage)))
}

pub fn load_and_verify_capsule(root: &Path, target: &CapsuleTarget) -> Result<CapsuleRecord> {
    let root_directory = open_verified_directory(root, true)?;
    let root_metadata = root_directory
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", root.display()), error))?;
    let manifest_path = root.join(CAPSULE_MANIFEST);
    let manifest_bytes = read_stable_regular_file_in_directory(
        &root_directory,
        CAPSULE_MANIFEST,
        &manifest_path,
        false,
    )?;
    let manifest: CapsuleManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| json_error(format!("parse {}", manifest_path.display()), error))?;
    validate_release_id_syntax(&manifest.release_id)?;
    let entries = validate_manifest_structure(&manifest, Some(target))?;
    let expected_release_id = compute_release_id(&manifest)?;
    if manifest.release_id != expected_release_id {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule releaseId {} does not match computed {}",
            manifest.release_id, expected_release_id
        )));
    }

    validate_filesystem_tree(root, &root_directory, &entries)?;
    validate_symlink_graph(&entries)?;
    let root_after = stable_symlink_metadata(root)?;
    require_unchanged(root, &root_metadata, &root_after)?;

    let digest = release_digest(&manifest.release_id)?.to_string();
    Ok(CapsuleRecord {
        release_id: manifest.release_id.clone(),
        digest,
        executable: root.join(&manifest.launch.executable),
        cwd: manifest.launch.cwd.as_ref().map(|cwd| root.join(cwd)),
        root: root.to_path_buf(),
        manifest,
    })
}

pub fn import_incoming(
    state_root: &Path,
    activation_id: &str,
    target: &CapsuleTarget,
) -> Result<CapsuleRecord> {
    validate_activation_id(activation_id)?;
    let incoming_store = ensure_real_directory(&state_root.join("incoming"))?;
    let artifact_store = ensure_real_directory(&state_root.join("artifacts"))?;
    let temp_store = ensure_real_directory(&artifact_store.join(".temp"))?;
    let incoming = incoming_store.join(activation_id);
    let owned = temp_store.join(activation_id);
    let receipt_path = temp_store.join(format!("{activation_id}.import.json"));
    let receipt = load_import_receipt(&receipt_path, activation_id)?;
    let incoming_exists = path_exists_without_following(&incoming)?;
    let owned_exists = path_exists_without_following(&owned)?;

    if receipt.is_some() && incoming_exists {
        return Err(LauncherError::Conflict(format!(
            "activation {activation_id} is already owned or published; refusing a new incoming capsule"
        )));
    }
    if receipt.is_none() && incoming_exists && owned_exists {
        return Err(LauncherError::Conflict(format!(
            "activation {activation_id} has both incoming and owned capsule state"
        )));
    }
    if receipt.is_none() && incoming_exists {
        rename_noreplace(&incoming, &owned).map_err(|error| {
            io_error(
                format!(
                    "acquire incoming capsule {} as {}",
                    incoming.display(),
                    owned.display()
                ),
                error,
            )
        })?;
        sync_directory(&incoming_store)?;
        sync_directory(&temp_store)?;
    }
    if receipt.is_none() && path_exists_without_following(&owned)? {
        sync_directory(&incoming_store)?;
        sync_directory(&temp_store)?;
    }

    if let Some(receipt) = &receipt {
        let destination = artifact_store.join(&receipt.digest);
        if path_exists_without_following(&destination)? {
            let existing = load_and_verify_capsule(&destination, target)?;
            require_receipt_matches_record(receipt, &existing, &destination)?;
            if path_exists_without_following(&owned)? {
                let candidate = load_and_verify_capsule(&owned, target)?;
                return finish_idempotent_import(candidate, &owned, &destination, target);
            }
            return Ok(existing);
        }
        if !path_exists_without_following(&owned)? {
            return Err(LauncherError::Conflict(format!(
                "activation {activation_id} has a durable import receipt but neither owned nor published capsule"
            )));
        }
    } else if !path_exists_without_following(&owned)? {
        return Err(LauncherError::InvalidRequest(format!(
            "incoming capsule does not exist for activation {activation_id}"
        )));
    }

    let candidate = match load_and_verify_capsule(&owned, target) {
        Ok(candidate) => candidate,
        Err(error) if receipt.is_none() => {
            remove_invalid_owned_import(&owned)?;
            sync_directory(&temp_store)?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    sync_capsule_tree(&owned)?;
    let destination = artifact_store.join(&candidate.digest);
    let receipt = match receipt {
        Some(receipt) => {
            require_receipt_matches_record(&receipt, &candidate, &owned)?;
            receipt
        }
        None => {
            let receipt = ImportReceipt {
                schema_version: 1,
                activation_id: activation_id.to_string(),
                release_id: candidate.release_id.clone(),
                digest: candidate.digest.clone(),
            };
            crate::control::write_json_atomic(&receipt_path, &receipt)?;
            load_import_receipt(&receipt_path, activation_id)?.ok_or_else(|| {
                LauncherError::Conflict(format!(
                    "durable import receipt disappeared for activation {activation_id}"
                ))
            })?
        }
    };

    if path_exists_without_following(&destination)? {
        let existing = finish_idempotent_import(candidate, &owned, &destination, target)?;
        require_receipt_matches_record(&receipt, &existing, &destination)?;
        return Ok(existing);
    }

    match rename_noreplace(&owned, &destination) {
        Ok(()) => {
            sync_directory(&temp_store)?;
            sync_directory(&artifact_store)?;
            let published = load_and_verify_capsule(&destination, target)?;
            require_receipt_matches_record(&receipt, &published, &destination)?;
            Ok(published)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = finish_idempotent_import(candidate, &owned, &destination, target)?;
            require_receipt_matches_record(&receipt, &existing, &destination)?;
            Ok(existing)
        }
        Err(error) => Err(io_error(
            format!("publish capsule {}", destination.display()),
            error,
        )),
    }
}

fn load_import_receipt(path: &Path, activation_id: &str) -> Result<Option<ImportReceipt>> {
    if !path_exists_without_following(path)? {
        return Ok(None);
    }
    let bytes = read_stable_private_regular_file(path)?;
    let receipt: ImportReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| json_error(format!("parse {}", path.display()), error))?;
    if receipt.schema_version != 1
        || receipt.activation_id != activation_id
        || release_digest(&receipt.release_id)? != receipt.digest
    {
        return Err(LauncherError::Conflict(format!(
            "invalid durable import receipt for activation {activation_id}"
        )));
    }
    Ok(Some(receipt))
}

fn require_receipt_matches_record(
    receipt: &ImportReceipt,
    record: &CapsuleRecord,
    path: &Path,
) -> Result<()> {
    if receipt.release_id == record.release_id && receipt.digest == record.digest {
        return Ok(());
    }
    Err(LauncherError::Conflict(format!(
        "durable import receipt does not match capsule at {}",
        path.display()
    )))
}

pub fn verify_record_for_spawn(
    record: &CapsuleRecord,
    target: &CapsuleTarget,
) -> Result<CapsuleRecord> {
    let verified = load_and_verify_capsule(&record.root, target)?;
    if verified.release_id != record.release_id
        || verified.digest != record.digest
        || verified.manifest != record.manifest
        || verified.executable != record.executable
        || verified.cwd != record.cwd
    {
        return Err(LauncherError::Conflict(format!(
            "capsule record for {} no longer matches its verified filesystem state",
            record.release_id
        )));
    }
    Ok(verified)
}

pub fn remove_external_capsule(state_root: &Path, record: &CapsuleRecord) -> Result<bool> {
    let artifacts = ensure_real_directory(&state_root.join("artifacts"))?;
    let expected = artifacts.join(&record.digest);
    if record.root != expected {
        return Err(LauncherError::Conflict(format!(
            "external capsule cleanup path is not its content-addressed store path: {}",
            record.root.display()
        )));
    }
    let metadata = match std::fs::symlink_metadata(&expected) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error(format!("inspect {}", expected.display()), error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "external capsule cleanup target is not a real directory: {}",
            expected.display()
        )));
    }
    std::fs::remove_dir_all(&expected)
        .map_err(|error| io_error(format!("remove {}", expected.display()), error))?;
    sync_directory(&artifacts)?;
    Ok(true)
}

fn finish_idempotent_import(
    candidate: CapsuleRecord,
    owned: &Path,
    destination: &Path,
    target: &CapsuleTarget,
) -> Result<CapsuleRecord> {
    let existing = load_and_verify_capsule(destination, target)?;
    if candidate.release_id != existing.release_id || candidate.digest != existing.digest {
        return Err(LauncherError::Conflict(format!(
            "artifact store entry {} does not match the incoming capsule identity",
            destination.display()
        )));
    }
    remove_owned_capsule(owned)?;
    if let Some(parent) = owned.parent() {
        sync_directory(parent)?;
    }
    Ok(existing)
}

fn validate_manifest_structure<'a>(
    manifest: &'a CapsuleManifest,
    target: Option<&CapsuleTarget>,
) -> Result<BTreeMap<String, &'a CapsuleEntry>> {
    if manifest.schema_version != CAPSULE_SCHEMA_VERSION {
        return Err(LauncherError::InvalidArtifact(format!(
            "unsupported capsule schema {}",
            manifest.schema_version
        )));
    }
    validate_target_component("target.os", &manifest.target.os)?;
    validate_target_component("target.arch", &manifest.target.arch)?;
    if target.is_some_and(|target| target != &manifest.target) {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule target {}/{} does not match requested {}/{}",
            manifest.target.os,
            manifest.target.arch,
            target.map_or("", |value| value.os.as_str()),
            target.map_or("", |value| value.arch.as_str())
        )));
    }
    validate_portable_path(&manifest.launch.executable)?;
    if let Some(cwd) = &manifest.launch.cwd {
        validate_portable_path(cwd)?;
    }
    for (index, argument) in manifest.launch.arguments.iter().enumerate() {
        if argument.as_bytes().contains(&0) {
            return Err(LauncherError::InvalidArtifact(format!(
                "launch argument {index} contains NUL"
            )));
        }
    }
    let prohibited_behaviors = &manifest.process_supervision.prohibited_behaviors;
    let supported_process_contract = prohibited_behaviors
        == &PROHIBITED_PROCESS_BEHAVIORS.map(str::to_string)
        || prohibited_behaviors == &LEGACY_PROHIBITED_PROCESS_BEHAVIORS.map(str::to_string);
    if manifest.process_supervision.contract != PROCESS_SUPERVISION_CONTRACT
        || !supported_process_contract
    {
        return Err(LauncherError::InvalidArtifact(
            "Capsule must declare cooperative observed supervision and a supported prohibited-process contract"
                .to_string(),
        ));
    }

    let mut entries = BTreeMap::new();
    let mut casefolded = BTreeMap::new();
    for entry in &manifest.entries {
        validate_portable_path(entry.path())?;
        if entry.path().rsplit('/').next() == Some(CAPSULE_MANIFEST) {
            return Err(LauncherError::InvalidArtifact(format!(
                "{CAPSULE_MANIFEST} is reserved for the Capsule root sidecar"
            )));
        }
        if entries.insert(entry.path().to_string(), entry).is_some() {
            return Err(LauncherError::InvalidArtifact(format!(
                "duplicate capsule entry {}",
                entry.path()
            )));
        }
        let folded = entry.path().to_ascii_lowercase();
        if let Some(existing) = casefolded.insert(folded, entry.path()) {
            return Err(LauncherError::InvalidArtifact(format!(
                "case-folding collision between {existing} and {}",
                entry.path()
            )));
        }
        match entry {
            CapsuleEntry::Directory { .. } => {}
            CapsuleEntry::File { sha256, .. } => {
                decode_sha256(sha256)?;
            }
            CapsuleEntry::Symlink { target, .. } => {
                validate_symlink_target_text(target)?;
            }
        }
    }
    if entries.is_empty() {
        return Err(LauncherError::InvalidArtifact(
            "capsule must declare at least one entry".to_string(),
        ));
    }

    for (path, entry) in &entries {
        let mut parent = Path::new(path).parent();
        while let Some(value) = parent {
            if value.as_os_str().is_empty() {
                break;
            }
            let parent_text = path_to_portable_string(value)?;
            match entries.get(&parent_text) {
                Some(CapsuleEntry::Directory { .. }) => {}
                Some(_) => {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "entry {} has non-directory parent {}",
                        entry.path(),
                        parent_text
                    )));
                }
                None => {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "entry {} has undeclared parent directory {}",
                        entry.path(),
                        parent_text
                    )));
                }
            }
            parent = value.parent();
        }
    }

    match entries.get(&manifest.launch.executable) {
        Some(CapsuleEntry::File {
            executable: true, ..
        }) => {}
        _ => {
            return Err(LauncherError::InvalidArtifact(
                "launch executable must name a declared executable file".to_string(),
            ));
        }
    }
    if let Some(cwd) = &manifest.launch.cwd
        && !matches!(entries.get(cwd), Some(CapsuleEntry::Directory { .. }))
    {
        return Err(LauncherError::InvalidArtifact(
            "launch cwd must name a declared directory".to_string(),
        ));
    }
    Ok(entries)
}

fn validate_filesystem_tree(
    root: &Path,
    root_directory: &File,
    entries: &BTreeMap<String, &CapsuleEntry>,
) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        validate_filesystem_tree_fd_bound(root, root_directory, entries)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn visit(
        root: &Path,
        directory: &Path,
        entries: &BTreeMap<String, &CapsuleEntry>,
        seen: &mut BTreeSet<String>,
    ) -> Result<()> {
        let before = stable_symlink_metadata(directory)?;
        require_directory(directory, &before, directory == root)?;
        reject_path_xattrs(directory)?;
        let mut children = std::fs::read_dir(directory)
            .map_err(|error| io_error(format!("read {}", directory.display()), error))?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| io_error(format!("read {}", directory.display()), error))?;
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let path = child.path();
            let relative = path.strip_prefix(root).map_err(|_| {
                LauncherError::InvalidArtifact(format!(
                    "capsule entry escaped root: {}",
                    path.display()
                ))
            })?;
            let relative = path_to_portable_string(relative)?;
            if relative == CAPSULE_MANIFEST {
                if directory != root {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "{CAPSULE_MANIFEST} is only allowed at the capsule root"
                    )));
                }
                continue;
            }
            let declared = entries.get(&relative).ok_or_else(|| {
                LauncherError::InvalidArtifact(format!("undeclared capsule entry {relative}"))
            })?;
            let metadata = stable_symlink_metadata(&path)?;
            match declared {
                CapsuleEntry::Directory { .. } => {
                    require_directory(&path, &metadata, false)?;
                    seen.insert(relative);
                    visit(root, &path, entries, seen)?;
                }
                CapsuleEntry::File {
                    sha256, executable, ..
                } => {
                    require_regular_file(&path, &metadata, *executable)?;
                    let bytes = read_stable_regular_file(&path, *executable)?;
                    let actual = format!("{:x}", Sha256::digest(bytes));
                    if &actual != sha256 {
                        return Err(LauncherError::InvalidArtifact(format!(
                            "sha256 mismatch for {relative}: expected {sha256}, got {actual}"
                        )));
                    }
                    seen.insert(relative);
                }
                CapsuleEntry::Symlink { target, .. } => {
                    require_symlink(&path, &metadata)?;
                    reject_path_xattrs(&path)?;
                    let actual = std::fs::read_link(&path)
                        .map_err(|error| io_error(format!("read {}", path.display()), error))?;
                    if actual != Path::new(target) {
                        return Err(LauncherError::InvalidArtifact(format!(
                            "symlink target mismatch for {relative}"
                        )));
                    }
                    let after = stable_symlink_metadata(&path)?;
                    require_unchanged(&path, &metadata, &after)?;
                    seen.insert(relative);
                }
            }
        }
        let after = stable_symlink_metadata(directory)?;
        require_unchanged(directory, &before, &after)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let mut seen = BTreeSet::new();
        visit(root, root, entries, &mut seen)?;
        require_all_declared_entries_seen(entries, &seen)
    }
}

fn require_all_declared_entries_seen(
    entries: &BTreeMap<String, &CapsuleEntry>,
    seen: &BTreeSet<String>,
) -> Result<()> {
    let declared = entries.keys().cloned().collect::<BTreeSet<_>>();
    if seen == &declared {
        return Ok(());
    }
    let missing = declared.difference(seen).cloned().collect::<Vec<_>>();
    Err(LauncherError::InvalidArtifact(format!(
        "declared capsule entries are missing: {}",
        missing.join(", ")
    )))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn validate_filesystem_tree_fd_bound(
    root: &Path,
    root_directory: &File,
    entries: &BTreeMap<String, &CapsuleEntry>,
) -> Result<()> {
    fn visit(
        root: &Path,
        directory: &File,
        relative_directory: &str,
        entries: &BTreeMap<String, &CapsuleEntry>,
        seen: &mut BTreeSet<String>,
    ) -> Result<()> {
        let display_path = if relative_directory.is_empty() {
            root.to_path_buf()
        } else {
            root.join(relative_directory)
        };
        let before = directory
            .metadata()
            .map_err(|error| io_error(format!("inspect {}", display_path.display()), error))?;
        require_directory(&display_path, &before, relative_directory.is_empty())?;
        reject_file_xattrs(&display_path, directory)?;

        let children = read_directory_names(directory, &display_path)?;
        for child_name in children {
            let child_text = child_name.to_str().ok_or_else(|| {
                LauncherError::InvalidArtifact(format!(
                    "capsule path is not UTF-8 below {}",
                    display_path.display()
                ))
            })?;
            let relative = if relative_directory.is_empty() {
                child_text.to_string()
            } else {
                format!("{relative_directory}/{child_text}")
            };
            validate_portable_path(&relative)?;
            if relative == CAPSULE_MANIFEST {
                continue;
            }
            let declared = entries.get(&relative).ok_or_else(|| {
                LauncherError::InvalidArtifact(format!("undeclared capsule entry {relative}"))
            })?;
            let child_path = root.join(&relative);
            match declared {
                CapsuleEntry::Directory { .. } => {
                    let child = open_directory_at(directory, child_name.as_bytes(), &child_path)?;
                    seen.insert(relative.clone());
                    visit(root, &child, &relative, entries, seen)?;
                }
                CapsuleEntry::File {
                    sha256, executable, ..
                } => {
                    let bytes = read_stable_regular_file_in_directory(
                        directory,
                        child_text,
                        &child_path,
                        *executable,
                    )?;
                    let actual = format!("{:x}", Sha256::digest(bytes));
                    if &actual != sha256 {
                        return Err(LauncherError::InvalidArtifact(format!(
                            "sha256 mismatch for {relative}: expected {sha256}, got {actual}"
                        )));
                    }
                    seen.insert(relative);
                }
                CapsuleEntry::Symlink { target, .. } => {
                    verify_symlink_at(
                        directory,
                        child_name.as_bytes(),
                        &child_path,
                        target.as_bytes(),
                    )?;
                    seen.insert(relative);
                }
            }
        }
        let after = directory
            .metadata()
            .map_err(|error| io_error(format!("inspect {}", display_path.display()), error))?;
        require_unchanged(&display_path, &before, &after)
    }

    let mut seen = BTreeSet::new();
    visit(root, root_directory, "", entries, &mut seen)?;
    require_all_declared_entries_seen(entries, &seen)
}

fn validate_symlink_graph(entries: &BTreeMap<String, &CapsuleEntry>) -> Result<()> {
    for (path, entry) in entries {
        let CapsuleEntry::Symlink { target, .. } = entry else {
            continue;
        };
        resolve_manifest_path(entries, path, target)?;
    }
    Ok(())
}

const MAX_SYMLINK_HOPS: usize = 64;

fn resolve_manifest_path(
    entries: &BTreeMap<String, &CapsuleEntry>,
    source_path: &str,
    target: &str,
) -> Result<String> {
    let mut components = Path::new(source_path)
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(LauncherError::InvalidArtifact(format!(
                "invalid symlink path {source_path}"
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
    append_lexical_target(&mut components, source_path, target)?;
    let mut visited = BTreeSet::from([source_path.to_string()]);
    let mut hops = 1_usize;
    let mut index = 0_usize;
    while index < components.len() {
        let current = components[..=index].join("/");
        let entry = entries.get(&current).ok_or_else(|| {
            LauncherError::InvalidArtifact(format!(
                "symlink {source_path} resolves through undeclared entry {current}"
            ))
        })?;
        match entry {
            CapsuleEntry::Directory { .. } => {
                index += 1;
            }
            CapsuleEntry::File { .. } if index + 1 == components.len() => {
                return Ok(current);
            }
            CapsuleEntry::File { .. } => {
                return Err(LauncherError::InvalidArtifact(format!(
                    "symlink {source_path} resolves through non-directory file {current}"
                )));
            }
            CapsuleEntry::Symlink { target, .. } => {
                if !visited.insert(current.clone()) {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "symlink cycle includes {current}"
                    )));
                }
                hops += 1;
                if hops > MAX_SYMLINK_HOPS {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "symlink {source_path} exceeds {MAX_SYMLINK_HOPS} manifest hops"
                    )));
                }
                let suffix = components.split_off(index + 1);
                components.truncate(index);
                append_lexical_target(&mut components, &current, target)?;
                components.extend(suffix);
                index = 0;
            }
        }
    }
    let resolved = components.join("/");
    if matches!(entries.get(&resolved), Some(CapsuleEntry::Directory { .. })) {
        Ok(resolved)
    } else {
        Err(LauncherError::InvalidArtifact(format!(
            "symlink {source_path} did not resolve to a declared directory or regular file"
        )))
    }
}

fn append_lexical_target(
    components: &mut Vec<String>,
    source_path: &str,
    target: &str,
) -> Result<()> {
    for component in Path::new(target).components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    LauncherError::InvalidArtifact(format!(
                        "symlink target for {source_path} is not UTF-8"
                    ))
                })?;
                components.push(value.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(LauncherError::InvalidArtifact(format!(
                        "symlink {source_path} escapes the capsule root"
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(LauncherError::InvalidArtifact(format!(
                    "symlink {source_path} has an absolute target"
                )));
            }
        }
    }
    if components.is_empty() {
        return Err(LauncherError::InvalidArtifact(format!(
            "symlink {source_path} resolves to the capsule root"
        )));
    }
    Ok(())
}

fn validate_portable_path(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > u16::MAX as usize {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule path must contain 1 to {} bytes",
            u16::MAX
        )));
    }
    if !path.is_ascii()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.as_bytes().contains(&0)
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule path is not portable ASCII: {path:?}"
        )));
    }
    for component in path.split('/') {
        validate_portable_component(component, path)?;
    }
    Ok(())
}

fn validate_portable_component(component: &str, full_path: &str) -> Result<()> {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(' ')
        || component.ends_with('.')
        || component
            .bytes()
            .any(|byte| !(0x20..=0x7e).contains(&byte) || b"<>:\"|?*".contains(&byte))
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule path has non-portable component {component:?}: {full_path}"
        )));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_lowercase();
    if matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    ) {
        return Err(LauncherError::InvalidArtifact(format!(
            "capsule path uses reserved component {component:?}: {full_path}"
        )));
    }
    Ok(())
}

fn validate_symlink_target_text(target: &str) -> Result<()> {
    if target.is_empty()
        || target.len() > u16::MAX as usize
        || !target.is_ascii()
        || target.starts_with('/')
        || target.ends_with('/')
        || target.contains('\\')
        || target.as_bytes().contains(&0)
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "symlink target is not portable relative ASCII: {target:?}"
        )));
    }
    for component in target.split('/') {
        if component == "." || component == ".." {
            continue;
        }
        if component.is_empty()
            || component.ends_with(' ')
            || component.ends_with('.')
            || component
                .bytes()
                .any(|byte| !(0x20..=0x7e).contains(&byte) || b"<>:\"|?*".contains(&byte))
        {
            return Err(LauncherError::InvalidArtifact(format!(
                "symlink target has non-portable component {component:?}: {target}"
            )));
        }
    }
    Ok(())
}

fn validate_target_component(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value.is_ascii()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "{label} is not a portable target identifier"
        )));
    }
    Ok(())
}

fn validate_activation_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_ACTIVATION_ID_BYTES
        || !value.is_ascii()
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(LauncherError::InvalidRequest(
            "activationId must be a portable identifier".to_string(),
        ));
    }
    Ok(())
}

fn validate_release_id_syntax(release_id: &str) -> Result<()> {
    let digest = release_digest(release_id)?;
    decode_sha256(digest)?;
    Ok(())
}

fn release_digest(release_id: &str) -> Result<&str> {
    let digest = release_id.strip_prefix(RELEASE_ID_PREFIX).ok_or_else(|| {
        LauncherError::InvalidArtifact(
            "capsule releaseId must use the sha256:<lowerhex> form".to_string(),
        )
    })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidArtifact(
            "capsule releaseId must use the sha256:<lowerhex> form".to_string(),
        ));
    }
    Ok(digest)
}

fn decode_sha256(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "invalid lowercase sha256 digest {value:?}"
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("digest syntax checked before decoding"),
    }
}

fn push_frame(output: &mut Vec<u8>, tag: &str, value: &[u8]) -> Result<()> {
    if !tag.is_ascii() {
        return Err(LauncherError::InvalidArtifact(
            "release preimage tag is not ASCII".to_string(),
        ));
    }
    let tag_len = u16::try_from(tag.len()).map_err(|_| {
        LauncherError::InvalidArtifact("release preimage tag is too long".to_string())
    })?;
    let value_len = u64::try_from(value.len()).map_err(|_| {
        LauncherError::InvalidArtifact("release preimage value is too long".to_string())
    })?;
    output.extend_from_slice(&tag_len.to_be_bytes());
    output.extend_from_slice(tag.as_bytes());
    output.extend_from_slice(&value_len.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn usize_as_u64(value: usize) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| LauncherError::InvalidArtifact("capsule collection is too large".to_string()))
}

fn path_to_portable_string(path: &Path) -> Result<String> {
    let value = path.to_str().ok_or_else(|| {
        LauncherError::InvalidArtifact(format!("capsule path is not UTF-8: {}", path.display()))
    })?;
    validate_portable_path(value)?;
    Ok(value.to_string())
}

fn read_stable_regular_file(path: &Path, executable: bool) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(path)
        .map_err(|error| io_error(format!("open {}", path.display()), error))?;
    let before = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_regular_file(path, &before, executable)?;
    reject_file_xattrs(path, &file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| io_error(format!("read {}", path.display()), error))?;
    let after = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_unchanged(path, &before, &after)?;
    Ok(bytes)
}

fn read_stable_private_regular_file(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options
        .open(path)
        .map_err(|error| io_error(format!("open {}", path.display()), error))?;
    let before = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(LauncherError::Conflict(format!(
            "durable import receipt is not a regular file: {}",
            path.display()
        )));
    }
    require_single_link(path, &before)?;
    require_mode(path, &before, 0o600)?;
    reject_file_xattrs(path, &file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| io_error(format!("read {}", path.display()), error))?;
    let after = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_unchanged(path, &before, &after)?;
    Ok(bytes)
}

fn open_verified_directory(path: &Path, is_root: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let directory = options
        .open(path)
        .map_err(|error| io_error(format!("open directory {}", path.display()), error))?;
    let metadata = directory
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_directory(path, &metadata, is_root)?;
    reject_file_xattrs(path, &directory)?;
    Ok(directory)
}

#[cfg(unix)]
fn read_stable_regular_file_in_directory(
    directory: &File,
    name: &str,
    display_path: &Path,
    executable: bool,
) -> Result<Vec<u8>> {
    let name = CString::new(name.as_bytes()).map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", display_path.display()))
    })?;
    // SAFETY: directory owns a valid descriptor, name is NUL-terminated, and the returned
    // descriptor is immediately transferred into File ownership on success.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(
            format!("open {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: fd was returned uniquely by openat above.
    let mut file = unsafe { File::from_raw_fd(fd) };
    read_stable_regular_file_from_open_file(&mut file, display_path, executable)
}

#[cfg(not(unix))]
fn read_stable_regular_file_in_directory(
    _directory: &File,
    _name: &str,
    display_path: &Path,
    executable: bool,
) -> Result<Vec<u8>> {
    read_stable_regular_file(display_path, executable)
}

fn read_stable_regular_file_from_open_file(
    file: &mut File,
    path: &Path,
    executable: bool,
) -> Result<Vec<u8>> {
    let before = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_regular_file(path, &before, executable)?;
    reject_file_xattrs(path, file)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| io_error(format!("read {}", path.display()), error))?;
    let after = file
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    require_unchanged(path, &before, &after)?;
    Ok(bytes)
}

#[cfg(unix)]
fn open_directory_at(directory: &File, name: &[u8], display_path: &Path) -> Result<File> {
    let name = CString::new(name).map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", display_path.display()))
    })?;
    // SAFETY: directory owns a valid descriptor, name is NUL-terminated, and the returned
    // descriptor is immediately transferred into File ownership on success.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(
            format!("open directory {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: fd was returned uniquely by openat above.
    let child = unsafe { File::from_raw_fd(fd) };
    let metadata = child
        .metadata()
        .map_err(|error| io_error(format!("inspect {}", display_path.display()), error))?;
    require_directory(display_path, &metadata, false)?;
    reject_file_xattrs(display_path, &child)?;
    Ok(child)
}

#[cfg(unix)]
fn read_directory_names(directory: &File, display_path: &Path) -> Result<Vec<std::ffi::OsString>> {
    // SAFETY: dup creates an independently owned descriptor for fdopendir.
    let duplicate = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicate < 0 {
        return Err(io_error(
            format!("duplicate directory {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: duplicate is a valid directory descriptor and ownership transfers to DIR.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        // SAFETY: fdopendir did not take ownership on failure.
        unsafe {
            libc::close(duplicate);
        }
        return Err(io_error(
            format!("read directory {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    let mut names = Vec::new();
    loop {
        set_errno(0);
        // SAFETY: stream remains valid until closed below; readdir's pointer is consumed before
        // the next call.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let error = current_errno();
            // SAFETY: stream is valid and closed exactly once.
            unsafe {
                libc::closedir(stream);
            }
            if error != 0 {
                return Err(io_error(
                    format!("read directory {}", display_path.display()),
                    std::io::Error::from_raw_os_error(error),
                ));
            }
            break;
        }
        // SAFETY: d_name is guaranteed to be NUL-terminated for the returned dirent.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name != b"." && name != b".." {
            names.push(std::ffi::OsStr::from_bytes(name).to_os_string());
        }
    }
    names.sort();
    Ok(names)
}

#[cfg(unix)]
fn verify_symlink_at(
    directory: &File,
    name: &[u8],
    display_path: &Path,
    expected_target: &[u8],
) -> Result<()> {
    let name = CString::new(name).map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", display_path.display()))
    })?;
    let before = symlink_stat_at(directory, &name, display_path)?;
    if before.st_mode & libc::S_IFMT != libc::S_IFLNK {
        return Err(LauncherError::InvalidArtifact(format!(
            "expected symlink at {}",
            display_path.display()
        )));
    }
    if before.st_nlink != 1 {
        return Err(LauncherError::InvalidArtifact(format!(
            "hard-linked symlink is not allowed: {}",
            display_path.display()
        )));
    }
    reject_symlink_xattrs_at(directory, &name, display_path)?;

    let capacity = expected_target.len().checked_add(1).ok_or_else(|| {
        LauncherError::InvalidArtifact(format!(
            "symlink target is too long: {}",
            display_path.display()
        ))
    })?;
    let mut target = vec![0_u8; capacity];
    // SAFETY: directory and name are valid, and target points to capacity writable bytes.
    let count = unsafe {
        libc::readlinkat(
            directory.as_raw_fd(),
            name.as_ptr(),
            target.as_mut_ptr().cast(),
            target.len(),
        )
    };
    if count < 0 {
        return Err(io_error(
            format!("read {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    let count = usize::try_from(count).map_err(|_| {
        LauncherError::InvalidArtifact(format!(
            "invalid symlink target length: {}",
            display_path.display()
        ))
    })?;
    if count == target.len() {
        return Err(LauncherError::InvalidArtifact(format!(
            "symlink target changed or exceeds manifest length: {}",
            display_path.display()
        )));
    }
    target.truncate(count);
    if target != expected_target {
        return Err(LauncherError::InvalidArtifact(format!(
            "symlink target mismatch for {}",
            display_path.display()
        )));
    }

    let after = symlink_stat_at(directory, &name, display_path)?;
    require_unchanged_symlink_stat(display_path, &before, &after)
}

#[cfg(unix)]
fn symlink_stat_at(directory: &File, name: &CString, display_path: &Path) -> Result<libc::stat> {
    // SAFETY: zero is a valid initial byte representation for stat before fstatat fills it.
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: directory and name are valid and stat points to writable storage.
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(io_error(
            format!("inspect {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    Ok(stat)
}

#[cfg(target_os = "linux")]
fn reject_symlink_xattrs_at(directory: &File, name: &CString, display_path: &Path) -> Result<()> {
    let proc_path = CString::new(format!(
        "/proc/self/fd/{}/{}",
        directory.as_raw_fd(),
        String::from_utf8_lossy(name.as_bytes())
    ))
    .map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", display_path.display()))
    })?;
    // SAFETY: proc_path is NUL-terminated and this is a size-only attribute probe.
    let count = unsafe { libc::llistxattr(proc_path.as_ptr(), std::ptr::null_mut(), 0) };
    reject_xattr_count(display_path, count)
}

#[cfg(target_os = "macos")]
fn reject_symlink_xattrs_at(directory: &File, name: &CString, display_path: &Path) -> Result<()> {
    // SAFETY: directory and name are valid; the returned descriptor is transferred to File.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_SYMLINK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io_error(
            format!("open symlink {}", display_path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: fd was returned uniquely by openat above.
    let file = unsafe { File::from_raw_fd(fd) };
    reject_file_xattrs(display_path, &file)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn reject_symlink_xattrs_at(
    _directory: &File,
    _name: &CString,
    _display_path: &Path,
) -> Result<()> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn require_unchanged_symlink_stat(
    path: &Path,
    before: &libc::stat,
    after: &libc::stat,
) -> Result<()> {
    let unchanged = before.st_dev == after.st_dev
        && before.st_ino == after.st_ino
        && before.st_mode == after.st_mode
        && before.st_nlink == after.st_nlink
        && before.st_uid == after.st_uid
        && before.st_gid == after.st_gid
        && before.st_size == after.st_size
        && before.st_mtime == after.st_mtime
        && before.st_mtime_nsec == after.st_mtime_nsec
        && before.st_ctime == after.st_ctime
        && before.st_ctime_nsec == after.st_ctime_nsec;
    require_unchanged_stat_result(path, unchanged)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn require_unchanged_symlink_stat(
    path: &Path,
    before: &libc::stat,
    after: &libc::stat,
) -> Result<()> {
    let unchanged = before.st_dev == after.st_dev
        && before.st_ino == after.st_ino
        && before.st_mode == after.st_mode
        && before.st_nlink == after.st_nlink
        && before.st_uid == after.st_uid
        && before.st_gid == after.st_gid
        && before.st_size == after.st_size
        && before.st_mtime == after.st_mtime
        && before.st_ctime == after.st_ctime;
    require_unchanged_stat_result(path, unchanged)
}

#[cfg(unix)]
fn require_unchanged_stat_result(path: &Path, unchanged: bool) -> Result<()> {
    if unchanged {
        return Ok(());
    }
    Err(LauncherError::InvalidArtifact(format!(
        "filesystem entry changed while being verified: {}",
        path.display()
    )))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn current_errno() -> i32 {
    // SAFETY: libc exposes a valid thread-local errno pointer.
    unsafe { *libc::__errno_location() }
}

#[cfg(target_os = "macos")]
fn current_errno() -> i32 {
    // SAFETY: libc exposes a valid thread-local errno pointer.
    unsafe { *libc::__error() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_errno(value: i32) {
    // SAFETY: libc exposes a valid thread-local errno pointer.
    unsafe {
        *libc::__errno_location() = value;
    }
}

#[cfg(target_os = "macos")]
fn set_errno(value: i32) {
    // SAFETY: libc exposes a valid thread-local errno pointer.
    unsafe {
        *libc::__error() = value;
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn current_errno() -> i32 {
    0
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn set_errno(_value: i32) {}

fn stable_symlink_metadata(path: &Path) -> Result<Metadata> {
    std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))
}

fn require_directory(path: &Path, metadata: &Metadata, is_root: bool) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "expected directory at {}",
            path.display()
        )));
    }
    require_mode(path, metadata, 0o755)?;
    if is_root {
        require_single_directory_identity(path, metadata)?;
    }
    Ok(())
}

fn require_regular_file(path: &Path, metadata: &Metadata, executable: bool) -> Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(LauncherError::InvalidArtifact(format!(
            "expected regular file at {}",
            path.display()
        )));
    }
    require_single_link(path, metadata)?;
    require_mode(path, metadata, if executable { 0o755 } else { 0o644 })
}

fn require_symlink(path: &Path, metadata: &Metadata) -> Result<()> {
    if !metadata.file_type().is_symlink() {
        return Err(LauncherError::InvalidArtifact(format!(
            "expected symlink at {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn require_mode(path: &Path, metadata: &Metadata, expected: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let actual = metadata.mode() & 0o7777;
    if actual != expected {
        return Err(LauncherError::InvalidArtifact(format!(
            "{} has mode {actual:o}; expected {expected:o}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn require_mode(_path: &Path, _metadata: &Metadata, _expected: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn require_single_link(path: &Path, metadata: &Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.nlink() != 1 {
        return Err(LauncherError::InvalidArtifact(format!(
            "hard-linked file is not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn require_single_link(path: &Path, metadata: &Metadata) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    if metadata.number_of_links() != Some(1) {
        return Err(LauncherError::InvalidArtifact(format!(
            "hard-linked file is not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn require_single_link(_path: &Path, _metadata: &Metadata) -> Result<()> {
    Ok(())
}

fn require_single_directory_identity(_path: &Path, _metadata: &Metadata) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn require_unchanged(path: &Path, before: &Metadata, after: &Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let unchanged = before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.mode() == after.mode()
        && before.nlink() == after.nlink()
        && before.uid() == after.uid()
        && before.gid() == after.gid()
        && before.size() == after.size()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec();
    if !unchanged {
        return Err(LauncherError::InvalidArtifact(format!(
            "filesystem entry changed while being verified: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn require_unchanged(path: &Path, before: &Metadata, after: &Metadata) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let unchanged = before.file_attributes() == after.file_attributes()
        && before.creation_time() == after.creation_time()
        && before.last_write_time() == after.last_write_time()
        && before.file_size() == after.file_size()
        && before.volume_serial_number() == after.volume_serial_number()
        && before.file_index() == after.file_index()
        && before.number_of_links() == after.number_of_links();
    if !unchanged {
        return Err(LauncherError::InvalidArtifact(format!(
            "filesystem entry changed while being verified: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn require_unchanged(path: &Path, before: &Metadata, after: &Metadata) -> Result<()> {
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || before.file_type() != after.file_type()
    {
        return Err(LauncherError::InvalidArtifact(format!(
            "filesystem entry changed while being verified: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_path_xattrs(path: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path_bytes = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", path.display()))
    })?;
    // SAFETY: path_bytes is NUL-terminated and both list pointer and length request an
    // attribute-size probe without writing memory.
    let count = unsafe { libc::llistxattr(path_bytes.as_ptr(), std::ptr::null_mut(), 0) };
    reject_xattr_count(path, count)
}

#[cfg(target_os = "macos")]
fn reject_path_xattrs(path: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path_bytes = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        LauncherError::InvalidArtifact(format!("path contains NUL: {}", path.display()))
    })?;
    // SAFETY: path_bytes is NUL-terminated and the first call only probes the required size.
    let count = unsafe {
        libc::listxattr(
            path_bytes.as_ptr(),
            std::ptr::null_mut(),
            0,
            libc::XATTR_NOFOLLOW,
        )
    };
    read_and_reject_macos_xattrs(path, count, |names, size| unsafe {
        // SAFETY: names points to size writable bytes and path_bytes remains NUL-terminated.
        libc::listxattr(path_bytes.as_ptr(), names, size, libc::XATTR_NOFOLLOW)
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn reject_path_xattrs(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn reject_file_xattrs(path: &Path, file: &File) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: file owns a valid descriptor and both list pointer and length request an
    // attribute-size probe without writing memory.
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    reject_xattr_count(path, count)
}

#[cfg(target_os = "macos")]
fn reject_file_xattrs(path: &Path, file: &File) -> Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: file owns a valid descriptor and the first call only probes the required size.
    let count = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0) };
    read_and_reject_macos_xattrs(path, count, |names, size| unsafe {
        // SAFETY: names points to size writable bytes and file owns a valid descriptor.
        libc::flistxattr(file.as_raw_fd(), names, size, 0)
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn reject_file_xattrs(_path: &Path, _file: &File) -> Result<()> {
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn reject_xattr_count(path: &Path, count: libc::ssize_t) -> Result<()> {
    if count < 0 {
        return Err(io_error(
            format!("inspect extended attributes for {}", path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    if count != 0 {
        return Err(LauncherError::InvalidArtifact(format!(
            "extended attributes are not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_and_reject_macos_xattrs(
    path: &Path,
    count: libc::ssize_t,
    read: impl FnOnce(*mut libc::c_char, usize) -> libc::ssize_t,
) -> Result<()> {
    if count < 0 {
        return Err(io_error(
            format!("inspect extended attributes for {}", path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    if count == 0 {
        return Ok(());
    }
    let mut names = vec![0_u8; count as usize];
    let actual = read(names.as_mut_ptr().cast(), names.len());
    if actual < 0 {
        return Err(io_error(
            format!("read extended attributes for {}", path.display()),
            std::io::Error::last_os_error(),
        ));
    }
    names.truncate(actual as usize);
    if names.last() != Some(&0) {
        return Err(LauncherError::InvalidArtifact(format!(
            "malformed extended attribute list: {}",
            path.display()
        )));
    }
    for name in names.split_inclusive(|byte| *byte == 0) {
        let name = &name[..name.len() - 1];
        // macOS 26 automatically attaches this system-managed attribute to newly created
        // filesystem objects and reports successful removal without making it disappear.
        // It is not payload metadata, so ignore this exact name while rejecting every other
        // extended attribute.
        if name != b"com.apple.provenance" {
            return Err(LauncherError::InvalidArtifact(format!(
                "extended attributes are not allowed: {} ({})",
                path.display(),
                String::from_utf8_lossy(name)
            )));
        }
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(path)
        .map_err(|error| io_error(format!("create {}", path.display()), error))?;
    let metadata = stable_symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "{} is not a real directory",
            path.display()
        )));
    }
    std::fs::canonicalize(path)
        .map_err(|error| io_error(format!("canonicalize {}", path.display()), error))
}

fn path_exists_without_following(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(format!("inspect {}", path.display()), error)),
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source contains NUL")
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination contains NUL")
    })?;
    // SAFETY: both paths are valid NUL-terminated C strings. RENAME_NOREPLACE gives the
    // ownership and publication operations their required no-clobber semantics.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "source contains NUL")
    })?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "destination contains NUL")
    })?;
    // SAFETY: both paths are valid NUL-terminated C strings. RENAME_EXCL gives the ownership
    // and publication operations their required no-clobber semantics.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    if destination.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "destination already exists",
        ));
    }
    // Windows rename does not replace an existing destination. This precheck is only needed
    // for other targets where std does not expose an atomic no-replace rename primitive.
    std::fs::rename(source, destination)
}

fn sync_capsule_tree(root: &Path) -> Result<()> {
    let mut children = std::fs::read_dir(root)
        .map_err(|error| io_error(format!("read {}", root.display()), error))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| io_error(format!("read {}", root.display()), error))?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let metadata = stable_symlink_metadata(&path)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            sync_capsule_tree(&path)?;
        } else if metadata.is_file() && !metadata.file_type().is_symlink() {
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            let file = options
                .open(&path)
                .map_err(|error| io_error(format!("open {}", path.display()), error))?;
            file.sync_all()
                .map_err(|error| io_error(format!("sync {}", path.display()), error))?;
        }
    }
    sync_directory(root)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(format!("sync {}", path.display()), error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn remove_owned_capsule(path: &Path) -> Result<()> {
    let metadata = stable_symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::Conflict(format!(
            "owned capsule path is not a real directory: {}",
            path.display()
        )));
    }
    std::fs::remove_dir_all(path)
        .map_err(|error| io_error(format!("remove {}", path.display()), error))
}

fn remove_invalid_owned_import(path: &Path) -> Result<()> {
    let metadata = stable_symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        std::fs::remove_file(path)
            .map_err(|error| io_error(format!("remove {}", path.display()), error))
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|error| io_error(format!("remove {}", path.display()), error))
    } else {
        Err(LauncherError::Conflict(format!(
            "owned import path has unsupported filesystem type: {}",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn toy_manifest() -> CapsuleManifest {
        CapsuleManifest {
            schema_version: CAPSULE_SCHEMA_VERSION,
            release_id: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            target: CapsuleTarget {
                os: "toy-os".to_string(),
                arch: "toy-arch".to_string(),
            },
            launch: CapsuleLaunch {
                executable: "bin/runtime".to_string(),
                arguments: vec!["--mode".to_string(), "toy".to_string()],
                cwd: Some("work".to_string()),
            },
            process_supervision: CapsuleProcessSupervision {
                contract: PROCESS_SUPERVISION_CONTRACT.to_string(),
                prohibited_behaviors: PROHIBITED_PROCESS_BEHAVIORS.map(str::to_string).to_vec(),
            },
            entries: vec![
                CapsuleEntry::Symlink {
                    path: "current".to_string(),
                    target: "work".to_string(),
                },
                CapsuleEntry::File {
                    path: "bin/runtime".to_string(),
                    sha256: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
                        .to_string(),
                    executable: true,
                },
                CapsuleEntry::Directory {
                    path: "work".to_string(),
                },
                CapsuleEntry::Directory {
                    path: "bin".to_string(),
                },
            ],
            metadata: serde_json::json!({"opaque": ["ignored", 1]}),
        }
    }

    #[test]
    fn release_preimage_is_nonempty_and_excludes_metadata() {
        let preimage = compute_release_preimage(&toy_manifest()).expect("preimage");
        assert!(!preimage.is_empty());
        assert!(!format_hex(&preimage).contains("72656164696e657373"));
    }

    #[test]
    fn legacy_readiness_manifest_is_rejected() {
        let mut legacy = serde_json::to_value(toy_manifest()).expect("manifest value");
        legacy["schemaVersion"] = serde_json::Value::from(1);
        legacy["launch"]["readiness"] = serde_json::json!({
            "protocol": "launcher-ready-v1",
            "timeoutMs": 5_000
        });

        assert!(
            serde_json::from_value::<CapsuleManifest>(legacy).is_err(),
            "schema v2 must not accept the retired readiness protocol"
        );
    }

    #[test]
    fn opaque_metadata_does_not_change_release_id() {
        let manifest = toy_manifest();
        let expected = compute_release_id(&manifest).expect("release id");
        let mut changed = manifest;
        changed.metadata = serde_json::json!({"different": true});
        assert_eq!(
            compute_release_id(&changed).expect("changed release id"),
            expected
        );
    }

    #[test]
    fn legacy_three_behavior_manifest_keeps_its_original_release_identity() {
        let mut legacy = toy_manifest();
        legacy.process_supervision.prohibited_behaviors = LEGACY_PROHIBITED_PROCESS_BEHAVIORS
            .map(str::to_string)
            .to_vec();
        let legacy_release_id = compute_release_id(&legacy).expect("legacy release id");
        validate_manifest_structure(&legacy, None).expect("legacy manifest remains supported");

        let current_release_id = compute_release_id(&toy_manifest()).expect("current release id");
        assert_ne!(legacy_release_id, current_release_id);
    }

    #[cfg(unix)]
    #[test]
    fn idempotent_import_ignores_opaque_metadata_differences() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = CapsuleTarget {
            os: "toy-os".to_string(),
            arch: "toy-arch".to_string(),
        };
        let destination = temp.path().join("destination");
        let owned = temp.path().join("owned");
        write_test_capsule(&destination, &target, b"same payload");
        write_test_capsule(&owned, &target, b"same payload");
        let manifest_path = owned.join(CAPSULE_MANIFEST);
        let mut owned_manifest: CapsuleManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).expect("owned manifest"))
                .expect("parse owned manifest");
        owned_manifest.metadata = serde_json::json!({"sourceCommit": "different"});
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&owned_manifest).expect("manifest json"),
        )
        .expect("rewrite metadata");
        std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644))
            .expect("manifest mode");

        let candidate = load_and_verify_capsule(&owned, &target).expect("candidate");
        let existing = finish_idempotent_import(candidate, &owned, &destination, &target)
            .expect("idempotent import");
        assert_eq!(
            existing.release_id,
            load_and_verify_capsule(&destination, &target)
                .expect("destination")
                .release_id
        );
        assert!(!owned.exists());
    }

    #[test]
    fn contained_directory_symlink_is_accepted() {
        let manifest = toy_manifest();
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        validate_symlink_graph(&entries).expect("symlink graph");
    }

    #[test]
    fn nested_capsule_manifest_sidecar_is_rejected() {
        let mut manifest = toy_manifest();
        manifest.entries.push(CapsuleEntry::File {
            path: "work/capsule.json".to_string(),
            sha256: "0".repeat(64),
            executable: false,
        });
        let error = validate_manifest_structure(&manifest, None).expect_err("nested sidecar");
        assert!(
            error
                .to_string()
                .contains("reserved for the Capsule root sidecar")
        );
    }

    #[test]
    fn versioned_framework_current_alias_resolves_suffix_to_canonical_leaf() {
        let mut manifest = toy_manifest();
        manifest.entries.extend([
            CapsuleEntry::Directory {
                path: "Frameworks".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Frameworks/Versioned Framework.framework".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Frameworks/Versioned Framework.framework/Versions".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Frameworks/Versioned Framework.framework/Versions/A".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Frameworks/Versioned Framework.framework/Versions/A/Resources".to_string(),
            },
            CapsuleEntry::File {
                path: "Frameworks/Versioned Framework.framework/Versions/A/Resources/Info.plist"
                    .to_string(),
                sha256: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
                    .to_string(),
                executable: false,
            },
            CapsuleEntry::Symlink {
                path: "Frameworks/Versioned Framework.framework/Versions/Current".to_string(),
                target: "A".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "framework-resource-leaf".to_string(),
                target:
                    "Frameworks/Versioned Framework.framework/Versions/Current/Resources/Info.plist"
                        .to_string(),
            },
        ]);
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        validate_symlink_graph(&entries).expect("component resolver");
        assert_eq!(
            resolve_manifest_path(
                &entries,
                "framework-resource-leaf",
                "Frameworks/Versioned Framework.framework/Versions/Current/Resources/Info.plist",
            )
            .expect("canonical leaf"),
            "Frameworks/Versioned Framework.framework/Versions/A/Resources/Info.plist"
        );
    }

    #[test]
    fn framework_top_level_resources_and_helpers_aliases_are_directories() {
        let mut manifest = toy_manifest();
        manifest.entries.extend([
            CapsuleEntry::Directory {
                path: "Framework".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Framework/Versions".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Framework/Versions/A".to_string(),
            },
            CapsuleEntry::Directory {
                path: "Framework/Versions/A/Resources".to_string(),
            },
            CapsuleEntry::File {
                path: "Framework/Versions/A/Resources/runtime.json".to_string(),
                sha256: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
                    .to_string(),
                executable: false,
            },
            CapsuleEntry::Directory {
                path: "Framework/Versions/A/Helpers".to_string(),
            },
            CapsuleEntry::File {
                path: "Framework/Versions/A/Helpers/helper".to_string(),
                sha256: "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
                    .to_string(),
                executable: true,
            },
            CapsuleEntry::Symlink {
                path: "Framework/Versions/Current".to_string(),
                target: "A".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "Framework/Resources".to_string(),
                target: "Versions/Current/Resources".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "Framework/Helpers".to_string(),
                target: "Versions/Current/Helpers".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "resource-leaf".to_string(),
                target: "Framework/Resources/runtime.json".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "helper-leaf".to_string(),
                target: "Framework/Helpers/helper".to_string(),
            },
        ]);
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        validate_symlink_graph(&entries).expect("directory aliases");
        assert_eq!(
            resolve_manifest_path(
                &entries,
                "Framework/Resources",
                "Versions/Current/Resources",
            )
            .expect("resources directory"),
            "Framework/Versions/A/Resources"
        );
        assert_eq!(
            resolve_manifest_path(&entries, "Framework/Helpers", "Versions/Current/Helpers")
                .expect("helpers directory"),
            "Framework/Versions/A/Helpers"
        );
        assert_eq!(
            resolve_manifest_path(
                &entries,
                "resource-leaf",
                "Framework/Resources/runtime.json",
            )
            .expect("resource canonical leaf"),
            "Framework/Versions/A/Resources/runtime.json"
        );
        assert_eq!(
            resolve_manifest_path(&entries, "helper-leaf", "Framework/Helpers/helper")
                .expect("helper canonical leaf"),
            "Framework/Versions/A/Helpers/helper"
        );
    }

    #[test]
    fn directory_symlink_cycle_with_suffix_is_rejected() {
        let mut manifest = toy_manifest();
        manifest.entries.extend([
            CapsuleEntry::Symlink {
                path: "loop-a".to_string(),
                target: "loop-b/data".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "loop-b".to_string(),
                target: "loop-a".to_string(),
            },
        ]);
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        assert!(validate_symlink_graph(&entries).is_err());
    }

    #[test]
    fn symlink_that_escapes_capsule_root_is_rejected() {
        let mut manifest = toy_manifest();
        manifest.entries.push(CapsuleEntry::Symlink {
            path: "escape".to_string(),
            target: "../outside".to_string(),
        });
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        let error = validate_symlink_graph(&entries).expect_err("escaping symlink");
        assert!(error.to_string().contains("escapes the capsule root"));
    }

    #[test]
    fn dangling_symlink_leaf_is_rejected() {
        let mut manifest = toy_manifest();
        manifest.entries.push(CapsuleEntry::Symlink {
            path: "dangling".to_string(),
            target: "work/missing".to_string(),
        });
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        let error = validate_symlink_graph(&entries).expect_err("dangling symlink");
        assert!(
            error
                .to_string()
                .contains("resolves through undeclared entry work/missing")
        );
    }

    #[test]
    fn symlink_suffix_through_undeclared_component_is_rejected() {
        let mut manifest = toy_manifest();
        manifest.entries.extend([
            CapsuleEntry::Directory {
                path: "work/data".to_string(),
            },
            CapsuleEntry::Symlink {
                path: "missing-suffix".to_string(),
                target: "current/undeclared/item".to_string(),
            },
        ]);
        let entries = validate_manifest_structure(&manifest, None).expect("manifest");
        let error = validate_symlink_graph(&entries).expect_err("undeclared suffix");
        assert!(
            error
                .to_string()
                .contains("resolves through undeclared entry work/undeclared")
        );
    }

    #[test]
    fn launch_executable_cannot_name_a_symlink_to_an_executable_file() {
        let mut manifest = toy_manifest();
        manifest.entries.push(CapsuleEntry::Symlink {
            path: "runtime-link".to_string(),
            target: "bin/runtime".to_string(),
        });
        manifest.launch.executable = "runtime-link".to_string();
        let error =
            validate_manifest_structure(&manifest, None).expect_err("symlink launch executable");
        assert!(
            error
                .to_string()
                .contains("launch executable must name a declared executable file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn import_resumes_after_move_publish_and_return_crash_points() {
        let state = tempfile::tempdir().expect("state");
        let target = CapsuleTarget {
            os: "toy-os".to_string(),
            arch: "toy-arch".to_string(),
        };

        let moved_activation = "moved";
        let moved_incoming = state.path().join("incoming").join(moved_activation);
        write_test_capsule(&moved_incoming, &target, b"moved runtime");
        let temp_store = state.path().join("artifacts/.temp");
        std::fs::create_dir_all(&temp_store).expect("temp store");
        let moved_owned = temp_store.join(moved_activation);
        std::fs::rename(&moved_incoming, &moved_owned).expect("simulate acquired move");
        let moved = import_incoming(state.path(), moved_activation, &target)
            .expect("resume after acquired move");
        assert!(moved.root.is_dir());

        let published_activation = "published";
        let published_incoming = state.path().join("incoming").join(published_activation);
        write_test_capsule(&published_incoming, &target, b"published runtime");
        let published_owned = temp_store.join(published_activation);
        std::fs::rename(&published_incoming, &published_owned).expect("acquire published");
        let candidate =
            load_and_verify_capsule(&published_owned, &target).expect("candidate capsule");
        let receipt = ImportReceipt {
            schema_version: 1,
            activation_id: published_activation.to_string(),
            release_id: candidate.release_id.clone(),
            digest: candidate.digest.clone(),
        };
        crate::control::write_json_atomic(
            &temp_store.join(format!("{published_activation}.import.json")),
            &receipt,
        )
        .expect("receipt");
        let destination = state.path().join("artifacts").join(&candidate.digest);
        std::fs::rename(&published_owned, &destination).expect("simulate published move");
        let published = import_incoming(state.path(), published_activation, &target)
            .expect("resume after published move");
        assert_eq!(published.release_id, candidate.release_id);

        let returned = import_incoming(state.path(), published_activation, &target)
            .expect("retry after successful return");
        assert_eq!(returned.release_id, published.release_id);
        assert_eq!(returned.root, published.root);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_owned_capsule_is_removed_before_retry() {
        let state = tempfile::tempdir().expect("state");
        let target = CapsuleTarget {
            os: "toy-os".to_string(),
            arch: "toy-arch".to_string(),
        };
        let incoming = state.path().join("incoming/invalid");
        std::fs::create_dir_all(&incoming).expect("invalid incoming");
        std::fs::set_permissions(&incoming, std::fs::Permissions::from_mode(0o755))
            .expect("incoming mode");

        import_incoming(state.path(), "invalid", &target).expect_err("invalid capsule");
        assert!(!state.path().join("artifacts/.temp/invalid").exists());
        assert!(
            !state
                .path()
                .join("artifacts/.temp/invalid.import.json")
                .exists()
        );
    }

    #[cfg(unix)]
    fn write_test_capsule(root: &Path, target: &CapsuleTarget, payload: &[u8]) {
        let executable = root.join("bin/runtime");
        std::fs::create_dir_all(executable.parent().expect("bin")).expect("bin");
        std::fs::create_dir_all(root.join("work")).expect("work");
        std::fs::write(&executable, payload).expect("runtime");
        std::os::unix::fs::symlink("work", root.join("current")).expect("current symlink");
        for directory in [root.to_path_buf(), root.join("bin"), root.join("work")] {
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
                .expect("directory mode");
        }
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("executable mode");
        let digest = format!("{:x}", Sha256::digest(payload));
        let mut manifest = CapsuleManifest {
            schema_version: CAPSULE_SCHEMA_VERSION,
            release_id: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            target: target.clone(),
            launch: CapsuleLaunch {
                executable: "bin/runtime".to_string(),
                arguments: Vec::new(),
                cwd: Some("work".to_string()),
            },
            process_supervision: CapsuleProcessSupervision {
                contract: PROCESS_SUPERVISION_CONTRACT.to_string(),
                prohibited_behaviors: PROHIBITED_PROCESS_BEHAVIORS.map(str::to_string).to_vec(),
            },
            entries: vec![
                CapsuleEntry::Directory {
                    path: "bin".to_string(),
                },
                CapsuleEntry::File {
                    path: "bin/runtime".to_string(),
                    sha256: digest,
                    executable: true,
                },
                CapsuleEntry::Symlink {
                    path: "current".to_string(),
                    target: "work".to_string(),
                },
                CapsuleEntry::Directory {
                    path: "work".to_string(),
                },
            ],
            metadata: serde_json::Value::Null,
        };
        manifest.release_id = compute_release_id(&manifest).expect("release id");
        let bytes = serde_json::to_vec_pretty(&manifest).expect("manifest json");
        std::fs::write(root.join(CAPSULE_MANIFEST), bytes).expect("manifest");
        std::fs::set_permissions(
            root.join(CAPSULE_MANIFEST),
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("manifest mode");
    }

    fn format_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
