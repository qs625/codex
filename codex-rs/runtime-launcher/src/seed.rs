use crate::LauncherError;
use crate::Result;
use crate::capsule::CapsuleRecord;
use crate::capsule::CapsuleTarget;
use crate::capsule::load_and_verify_capsule;
use crate::control::CapsuleRef;
use crate::control::TrustedSeed;
use crate::io_error;
use std::path::Path;

pub const SEED_RELATIVE_PATH: &str = "Contents/Resources/seed-capsule";

pub fn discover_seed(
    outer_bundle: &Path,
    target: &CapsuleTarget,
) -> Result<(TrustedSeed, CapsuleRecord)> {
    let expected_release = option_env!("RUNTIME_CAPSULE_SEED_RELEASE_ID").ok_or_else(|| {
        LauncherError::Conflict(
            "RUNTIME_CAPSULE_SEED_RELEASE_ID was not embedded at build time".to_string(),
        )
    })?;
    discover_seed_with_expected_release(outer_bundle, target, expected_release)
}

pub(crate) fn discover_seed_with_expected_release(
    outer_bundle: &Path,
    target: &CapsuleTarget,
    expected_release: &str,
) -> Result<(TrustedSeed, CapsuleRecord)> {
    verify_outer_bundle(outer_bundle)?;
    let record = load_and_verify_capsule(&outer_bundle.join(SEED_RELATIVE_PATH), target)?;
    if record.release_id != expected_release {
        return Err(LauncherError::InvalidArtifact(format!(
            "Seed release {} does not match embedded release {}",
            record.release_id, expected_release
        )));
    }
    let capsule = CapsuleRef {
        release_id: record.release_id.clone(),
        root: record.root.clone(),
        entrypoint: record.executable.clone(),
        metadata: record.manifest.metadata.clone(),
    };
    Ok((
        TrustedSeed {
            capsule,
            trust_anchor: outer_bundle.to_path_buf(),
            metadata: serde_json::json!({
                "integrity": "outer-bundle-deep-strict",
                "identityClaim": "development-integrity-only"
            }),
        },
        record,
    ))
}

#[cfg(target_os = "macos")]
fn verify_outer_bundle(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "outer trust anchor is not a real bundle directory: {}",
            path.display()
        )));
    }
    let status = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(path)
        .status()
        .map_err(|error| io_error("run codesign verification", error))?;
    if !status.success() {
        return Err(LauncherError::InvalidArtifact(format!(
            "outer bundle codesign verification failed: {}",
            path.display()
        )));
    }
    let expected_identifier = option_env!("RUNTIME_CAPSULE_BUNDLE_ID").ok_or_else(|| {
        LauncherError::Conflict(
            "RUNTIME_CAPSULE_BUNDLE_ID was not embedded at build time".to_string(),
        )
    })?;
    let output = std::process::Command::new("/usr/bin/codesign")
        .args(["-d", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|error| io_error("read outer bundle identity", error))?;
    let details = String::from_utf8_lossy(&output.stderr);
    let actual = details
        .lines()
        .find_map(|line| line.strip_prefix("Identifier="))
        .ok_or_else(|| {
            LauncherError::InvalidArtifact("outer bundle signature has no Identifier".to_string())
        })?;
    if actual != expected_identifier {
        return Err(LauncherError::InvalidArtifact(format!(
            "outer bundle identifier {actual} does not match embedded {expected_identifier}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_location_matches_installed_packaging_contract() {
        assert_eq!(
            Path::new("/Applications/Runtime.app").join(SEED_RELATIVE_PATH),
            Path::new("/Applications/Runtime.app/Contents/Resources/seed-capsule")
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn verify_outer_bundle(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io_error(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "outer trust anchor is not a real directory: {}",
            path.display()
        )));
    }
    Ok(())
}
