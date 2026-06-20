use std::path::Path;
use std::path::PathBuf;

use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use globset::GlobBuilder;
use globset::GlobMatcher;

/// Runtime matcher for read-deny entries in a filesystem sandbox policy.
pub struct ReadDenyMatcher {
    denied_candidates: Vec<Vec<PathBuf>>,
    deny_read_matchers: Vec<GlobMatcher>,
    invalid_pattern: bool,
}

impl ReadDenyMatcher {
    /// Builds a matcher from exact deny-read roots and deny-read glob entries.
    ///
    /// Returns `None` when the policy has no deny-read restrictions, so callers
    /// can skip read-deny checks without allocating matcher state. The `cwd`
    /// resolves cwd-relative policy paths and special paths before matching.
    pub fn new(file_system_sandbox_policy: &FileSystemSandboxPolicy, cwd: &Path) -> Option<Self> {
        match Self::build(
            file_system_sandbox_policy,
            cwd,
            InvalidDenyReadGlobBehavior::FailClosed,
        ) {
            Ok(matcher) => matcher,
            Err(_) => unreachable!("fail-closed glob handling does not return errors"),
        }
    }

    /// Builds a matcher for callers that must reject malformed glob patterns.
    ///
    /// Runtime read checks intentionally fail closed on malformed deny patterns.
    /// Host-side expansion work should use this constructor instead so a typo
    /// cannot broaden the set of paths it mutates before execution starts.
    pub fn try_new(
        file_system_sandbox_policy: &FileSystemSandboxPolicy,
        cwd: &Path,
    ) -> Result<Option<Self>, String> {
        Self::build(
            file_system_sandbox_policy,
            cwd,
            InvalidDenyReadGlobBehavior::ReturnError,
        )
    }

    fn build(
        file_system_sandbox_policy: &FileSystemSandboxPolicy,
        cwd: &Path,
        invalid_glob_behavior: InvalidDenyReadGlobBehavior,
    ) -> Result<Option<Self>, String> {
        if !file_system_sandbox_policy.has_denied_read_restrictions() {
            return Ok(None);
        }

        // Exact roots are stored as all meaningful path spellings we can derive
        // cheaply. This lets direct tool checks catch both a symlink path and
        // its canonical target without changing the policy entries themselves.
        let denied_candidates = file_system_sandbox_policy
            .get_unreadable_roots_with_cwd(cwd)
            .into_iter()
            .map(|path| normalized_and_canonical_candidates(path.as_path()))
            .collect();
        // Pattern entries stay as policy-level globs. They are matched at read
        // time here instead of being snapshotted to startup filesystem state.
        let mut invalid_pattern = false;
        let mut deny_read_matchers = Vec::new();
        for pattern in file_system_sandbox_policy.get_unreadable_globs_with_cwd(cwd) {
            match build_glob_matcher(&pattern) {
                Ok(matcher) => deny_read_matchers.push(matcher),
                Err(err) => match invalid_glob_behavior {
                    InvalidDenyReadGlobBehavior::FailClosed => invalid_pattern = true,
                    InvalidDenyReadGlobBehavior::ReturnError => {
                        return Err(format!("invalid deny-read glob pattern `{pattern}`: {err}"));
                    }
                },
            }
        }
        Ok(Some(Self {
            denied_candidates,
            deny_read_matchers,
            invalid_pattern,
        }))
    }

    /// Returns whether `path` is denied by the policy used to build this matcher.
    pub fn is_read_denied(&self, path: &Path) -> bool {
        if self.invalid_pattern {
            // Direct tool reads fail closed on malformed deny patterns. Silent
            // allow would turn a config typo into a policy bypass.
            return true;
        }

        // Check exact roots against each candidate spelling before evaluating
        // glob matchers. Exact entries are subtree denies; glob entries match
        // according to the pattern compiler's path-separator rules.
        let path_candidates = normalized_and_canonical_candidates(path);
        if self.denied_candidates.iter().any(|denied_candidates| {
            path_candidates.iter().any(|candidate| {
                denied_candidates.iter().any(|denied_candidate| {
                    candidate == denied_candidate || candidate.starts_with(denied_candidate)
                })
            })
        }) {
            return true;
        }

        self.deny_read_matchers.iter().any(|matcher| {
            path_candidates
                .iter()
                .any(|candidate| matcher.is_match(candidate))
        })
    }
}

#[derive(Clone, Copy)]
enum InvalidDenyReadGlobBehavior {
    FailClosed,
    ReturnError,
}

fn normalized_and_canonical_candidates(path: &Path) -> Vec<PathBuf> {
    // Compare the lexical absolute form plus the canonical target when it
    // exists. Missing paths still need the lexical candidate so future-created
    // denied paths remain blocked by direct tool checks.
    let mut candidates = Vec::new();

    if let Ok(normalized) = AbsolutePathBuf::from_absolute_path(path) {
        push_unique(&mut candidates, normalized.to_path_buf());
    } else {
        push_unique(&mut candidates, path.to_path_buf());
    }

    if let Ok(canonical) = path.canonicalize()
        && let Ok(canonical_absolute) = AbsolutePathBuf::from_absolute_path(canonical)
    {
        push_unique(&mut candidates, canonical_absolute.to_path_buf());
    }

    candidates
}

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn build_glob_matcher(pattern: &str) -> Result<GlobMatcher, String> {
    // Keep `*` and `?` within a single path component and preserve an unclosed
    // `[` as a literal so matcher behavior stays aligned with config parsing.
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .allow_unclosed_class(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::permissions::FileSystemAccessMode;
    use codex_protocol::permissions::FileSystemPath;
    use codex_protocol::permissions::FileSystemSandboxEntry;
    use tempfile::TempDir;

    #[cfg(unix)]
    fn symlink_dir(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    fn deny_policy(path: &Path) -> FileSystemSandboxPolicy {
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: AbsolutePathBuf::try_from(path).expect("absolute deny path"),
            },
            access: FileSystemAccessMode::None,
        }])
    }

    fn unreadable_glob_entry(pattern: String) -> FileSystemSandboxEntry {
        FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern { pattern },
            access: FileSystemAccessMode::None,
        }
    }

    fn default_policy_with_unreadable_glob(pattern: String) -> FileSystemSandboxPolicy {
        let mut policy = FileSystemSandboxPolicy::default();
        policy.entries.push(unreadable_glob_entry(pattern));
        policy
    }

    fn is_read_denied(
        path: &Path,
        file_system_sandbox_policy: &FileSystemSandboxPolicy,
        cwd: &Path,
    ) -> bool {
        ReadDenyMatcher::new(file_system_sandbox_policy, cwd)
            .is_some_and(|matcher| matcher.is_read_denied(path))
    }

    #[test]
    fn exact_path_and_descendants_are_denied() {
        let temp = TempDir::new().expect("tempdir");
        let denied_dir = temp.path().join("denied");
        let nested = denied_dir.join("nested.txt");
        std::fs::create_dir_all(&denied_dir).expect("create denied dir");
        std::fs::write(&nested, "secret").expect("write secret");

        let policy = deny_policy(&denied_dir);
        assert!(is_read_denied(&denied_dir, &policy, temp.path()));
        assert!(is_read_denied(&nested, &policy, temp.path()));
        assert!(!is_read_denied(
            &temp.path().join("other.txt"),
            &policy,
            temp.path()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn canonical_target_matches_denied_symlink_alias() {
        let temp = TempDir::new().expect("tempdir");
        let real_dir = temp.path().join("real");
        let alias_dir = temp.path().join("alias");
        std::fs::create_dir_all(&real_dir).expect("create real dir");
        symlink_dir(&real_dir, &alias_dir).expect("symlink alias");

        let secret = real_dir.join("secret.txt");
        std::fs::write(&secret, "secret").expect("write secret");
        let alias_secret = alias_dir.join("secret.txt");

        let policy = deny_policy(&real_dir);
        assert!(is_read_denied(&alias_secret, &policy, temp.path()));
    }

    #[test]
    fn literal_patterns_and_globs_are_denied() {
        let temp = TempDir::new().expect("tempdir");
        let literal = temp.path().join("private");
        let other = temp.path().join("notes.txt");
        std::fs::create_dir_all(&literal).expect("create literal dir");
        std::fs::write(&other, "notes").expect("write notes");

        let mut policy = deny_policy(&literal);
        policy.entries.push(unreadable_glob_entry(format!(
            "{}/**/*.txt",
            temp.path().display()
        )));

        assert!(is_read_denied(&literal, &policy, temp.path()));
        assert!(is_read_denied(&other, &policy, temp.path()));
    }

    #[test]
    fn glob_patterns_deny_matching_paths() {
        let temp = TempDir::new().expect("tempdir");
        let denied = temp.path().join("private").join("secret1.txt");
        std::fs::create_dir_all(denied.parent().expect("parent")).expect("create parent");
        std::fs::write(&denied, "secret").expect("write secret");

        let policy = default_policy_with_unreadable_glob(format!(
            "{}/private/secret?.txt",
            temp.path().display()
        ));

        assert!(is_read_denied(&denied, &policy, temp.path()));
    }

    #[test]
    fn glob_patterns_do_not_cross_path_separators() {
        let temp = TempDir::new().expect("tempdir");
        let matching = temp.path().join("app").join("file42.txt");
        let nested = temp.path().join("app").join("nested").join("file42.txt");
        let short = temp.path().join("app").join("file4.txt");
        let letters = temp.path().join("app").join("fileab.txt");
        std::fs::create_dir_all(nested.parent().expect("parent")).expect("create parent");
        std::fs::write(&matching, "secret").expect("write matching");
        std::fs::write(&nested, "secret").expect("write nested");
        std::fs::write(&short, "secret").expect("write short");
        std::fs::write(&letters, "secret").expect("write letters");

        let policy = default_policy_with_unreadable_glob(format!(
            "{}/*/file[0-9]?.txt",
            temp.path().display()
        ));

        assert!(is_read_denied(&matching, &policy, temp.path()));
        assert!(!is_read_denied(&nested, &policy, temp.path()));
        assert!(!is_read_denied(&short, &policy, temp.path()));
        assert!(!is_read_denied(&letters, &policy, temp.path()));
    }

    #[test]
    fn globstar_patterns_deny_root_and_nested_matches() {
        let temp = TempDir::new().expect("tempdir");
        let root_env = temp.path().join(".env");
        let nested_env = temp.path().join("app").join(".env");
        let other = temp.path().join("app").join("notes.txt");
        std::fs::create_dir_all(nested_env.parent().expect("parent")).expect("create parent");
        std::fs::write(&root_env, "secret").expect("write root env");
        std::fs::write(&nested_env, "secret").expect("write nested env");
        std::fs::write(&other, "notes").expect("write notes");

        let policy =
            default_policy_with_unreadable_glob(format!("{}/**/*.env", temp.path().display()));

        assert!(is_read_denied(&root_env, &policy, temp.path()));
        assert!(is_read_denied(&nested_env, &policy, temp.path()));
        assert!(!is_read_denied(&other, &policy, temp.path()));
    }

    #[test]
    fn unclosed_character_classes_match_literal_brackets() {
        let temp = TempDir::new().expect("tempdir");
        let bracket_file = temp.path().join("[");
        let other = temp.path().join("notes.txt");
        std::fs::write(&bracket_file, "secret").expect("write bracket file");
        std::fs::write(&other, "notes").expect("write notes");
        let policy = default_policy_with_unreadable_glob(format!("{}/[", temp.path().display()));

        assert!(is_read_denied(&bracket_file, &policy, temp.path()));
        assert!(!is_read_denied(&other, &policy, temp.path()));
    }
}
