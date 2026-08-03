use codex_utils_absolute_path::AbsolutePathBuf;
use dirs::home_dir as dirs_home_dir;
use std::path::PathBuf;

const MORPHEUS_HOME_ENV: &str = "MORPHEUS_HOME";
const DEFAULT_MORPHEUS_HOME_DIR: &str = ".morpheus";

pub fn home_dir() -> Option<PathBuf> {
    dirs_home_dir()
}

/// Returns the path to the Morpheus configuration directory, which can be
/// specified by the `MORPHEUS_HOME` environment variable. If not set, defaults
/// to `~/.morpheus`.
///
/// - If `MORPHEUS_HOME` is set, the value must exist and be a directory. The
///   value will be canonicalized and this function will Err otherwise.
/// - If `MORPHEUS_HOME` is not set, this function does not verify that the
///   directory exists.
pub fn find_codex_home() -> std::io::Result<AbsolutePathBuf> {
    let morpheus_home_env = std::env::var(MORPHEUS_HOME_ENV)
        .ok()
        .filter(|val| !val.is_empty());
    find_codex_home_from_env(morpheus_home_env.as_deref())
}

fn find_codex_home_from_env(morpheus_home_env: Option<&str>) -> std::io::Result<AbsolutePathBuf> {
    match morpheus_home_env {
        Some(val) => {
            let path = PathBuf::from(val);
            let metadata = std::fs::metadata(&path).map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{MORPHEUS_HOME_ENV} points to {val:?}, but that path does not exist"),
                ),
                _ => std::io::Error::new(
                    err.kind(),
                    format!("failed to read {MORPHEUS_HOME_ENV} {val:?}: {err}"),
                ),
            })?;

            if !metadata.is_dir() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "{MORPHEUS_HOME_ENV} points to {val:?}, but that path is not a directory"
                    ),
                ))
            } else {
                let canonical = path.canonicalize().map_err(|err| {
                    std::io::Error::new(
                        err.kind(),
                        format!("failed to canonicalize {MORPHEUS_HOME_ENV} {val:?}: {err}"),
                    )
                })?;
                AbsolutePathBuf::from_absolute_path(canonical)
            }
        }
        None => {
            let mut p = home_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find home directory",
                )
            })?;
            p.push(DEFAULT_MORPHEUS_HOME_DIR);
            AbsolutePathBuf::from_absolute_path(p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::find_codex_home;
    use super::find_codex_home_from_env;
    use super::home_dir;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::ffi::OsString;
    use std::fs;
    use std::io::ErrorKind;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var_os(key);
            // SAFETY: Tests that mutate process environment take ENV_LOCK for
            // the full duration of the mutation and restore the original value.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }

        fn remove(key: &'static str) -> Self {
            let original = std::env::var_os(key);
            // SAFETY: Tests that mutate process environment take ENV_LOCK for
            // the full duration of the mutation and restore the original value.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: Tests that mutate process environment take ENV_LOCK for
            // the full duration of the mutation and restore the original value.
            unsafe {
                if let Some(value) = &self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn find_codex_home_morpheus_env_missing_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let missing = temp_home.path().join("missing-morpheus-home");
        let missing_str = missing
            .to_str()
            .expect("missing morpheus home path should be valid utf-8");

        let err = find_codex_home_from_env(Some(missing_str)).expect_err("missing MORPHEUS_HOME");
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(
            err.to_string().contains("MORPHEUS_HOME"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_codex_home_morpheus_env_file_path_is_fatal() {
        let temp_home = TempDir::new().expect("temp home");
        let file_path = temp_home.path().join("morpheus-home.txt");
        fs::write(&file_path, "not a directory").expect("write temp file");
        let file_str = file_path
            .to_str()
            .expect("file morpheus home path should be valid utf-8");

        let err = find_codex_home_from_env(Some(file_str)).expect_err("file MORPHEUS_HOME");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(
            err.to_string().contains("MORPHEUS_HOME"),
            "unexpected error: {err}"
        );
        assert!(
            err.to_string().contains("not a directory"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn find_codex_home_morpheus_env_valid_directory_canonicalizes() {
        let temp_home = TempDir::new().expect("temp home");
        let temp_str = temp_home
            .path()
            .to_str()
            .expect("temp morpheus home path should be valid utf-8");

        let resolved = find_codex_home_from_env(Some(temp_str)).expect("valid MORPHEUS_HOME");
        let expected = temp_home
            .path()
            .canonicalize()
            .expect("canonicalize temp home");
        let expected = AbsolutePathBuf::from_absolute_path(expected).expect("absolute home");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn find_codex_home_without_env_uses_default_morpheus_home_dir() {
        let resolved = find_codex_home_from_env(None).expect("default MORPHEUS_HOME");
        let mut expected = home_dir().expect("home dir");
        expected.push(".morpheus");
        let expected = AbsolutePathBuf::from_absolute_path(expected).expect("absolute home");
        assert_eq!(resolved, expected);
    }

    #[test]
    fn find_codex_home_ignores_legacy_codex_home_env() {
        let _lock = ENV_LOCK.lock().expect("env lock");
        let legacy_home = TempDir::new().expect("legacy codex home");
        let legacy_home_str = legacy_home
            .path()
            .to_str()
            .expect("legacy codex home path should be valid utf-8");
        let _morpheus_home = EnvGuard::remove("MORPHEUS_HOME");
        let _codex_home = EnvGuard::set("CODEX_HOME", legacy_home_str);

        let resolved = find_codex_home().expect("default MORPHEUS_HOME");
        let mut expected = home_dir().expect("home dir");
        expected.push(".morpheus");
        let expected = AbsolutePathBuf::from_absolute_path(expected).expect("absolute home");

        assert_eq!(resolved, expected);
    }
}
