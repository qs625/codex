use crate::LauncherError;
use crate::Result;
use crate::io_error;
use std::path::Path;

pub(crate) trait CommandRunner {
    fn run(&self, program: &Path, arguments: &[&str], target: &Path) -> Result<bool>;
}

pub(crate) struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &Path, arguments: &[&str], target: &Path) -> Result<bool> {
        let status = std::process::Command::new(program)
            .args(arguments)
            .arg(target)
            .status()
            .map_err(|err| io_error(format!("run {}", program.display()), err))?;
        Ok(status.success())
    }
}

pub(crate) trait BundleSigner {
    fn sign_and_verify(&self, path: &Path) -> Result<()>;
}

pub(crate) struct PlatformBundleSigner;

impl BundleSigner for PlatformBundleSigner {
    fn sign_and_verify(&self, path: &Path) -> Result<()> {
        sign_and_verify_app_bundle(path)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn verify_app_bundle(path: &Path) -> Result<()> {
    reject_bundle(path)?;
    verify_with_runner(path, &SystemCommandRunner)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn verify_app_bundle(path: &Path) -> Result<()> {
    reject_bundle(path)
}

#[cfg(target_os = "macos")]
pub(crate) fn sign_and_verify_app_bundle(path: &Path) -> Result<()> {
    reject_bundle(path)?;
    sign_and_verify_with_runner(path, &SystemCommandRunner)
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn sign_and_verify_app_bundle(path: &Path) -> Result<()> {
    reject_bundle(path)
}

#[cfg(target_os = "macos")]
fn sign_and_verify_with_runner(path: &Path, runner: &dyn CommandRunner) -> Result<()> {
    let codesign = Path::new("/usr/bin/codesign");
    if !runner.run(codesign, &["--force", "--sign", "-"], path)? {
        return Err(LauncherError::InvalidArtifact(format!(
            "codesign signing failed for {}",
            path.display()
        )));
    }
    verify_with_runner(path, runner)
}

#[cfg(target_os = "macos")]
fn verify_with_runner(path: &Path, runner: &dyn CommandRunner) -> Result<()> {
    if !runner.run(
        Path::new("/usr/bin/codesign"),
        &["--verify", "--deep", "--strict"],
        path,
    )? {
        return Err(LauncherError::InvalidArtifact(format!(
            "codesign verification failed for {}",
            path.display()
        )));
    }
    Ok(())
}

fn reject_bundle(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| io_error(format!("inspect {}", path.display()), err))?;
    if metadata.file_type().is_symlink() {
        return Err(LauncherError::InvalidArtifact(format!(
            "app bundle must not be a symlink: {}",
            path.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(LauncherError::InvalidArtifact(format!(
            "{} is not an application bundle directory",
            path.display()
        )));
    }
    #[cfg(target_os = "macos")]
    if path.extension().and_then(|value| value.to_str()) != Some("app") {
        return Err(LauncherError::InvalidArtifact(format!(
            "{} is not a macOS .app bundle",
            path.display()
        )));
    }
    Ok(())
}
