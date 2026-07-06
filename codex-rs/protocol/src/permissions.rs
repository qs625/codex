use std::collections::HashSet;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::canonicalize_preserving_symlinks;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use strum_macros::Display;
use tracing::error;
use ts_rs::TS;

use crate::protocol::NetworkAccess;
use crate::protocol::SandboxPolicy;
use crate::protocol::WritableRoot;

const PROTECTED_METADATA_GIT_PATH_NAME: &str = ".git";
const PROTECTED_METADATA_AGENTS_PATH_NAME: &str = ".agents";
const PROTECTED_METADATA_CODEX_PATH_NAME: &str = ".codex";

/// Top-level workspace metadata paths that stay protected under writable roots.
pub const PROTECTED_METADATA_PATH_NAMES: &[&str] = &[
    PROTECTED_METADATA_GIT_PATH_NAME,
    PROTECTED_METADATA_AGENTS_PATH_NAME,
    PROTECTED_METADATA_CODEX_PATH_NAME,
];

/// Returns true when a path basename is one of the protected workspace metadata names.
pub fn is_protected_metadata_name(name: &OsStr) -> bool {
    PROTECTED_METADATA_PATH_NAMES
        .iter()
        .any(|metadata_name| name == OsStr::new(metadata_name))
}

pub fn is_protected_metadata_directory_name(name: &OsStr) -> bool {
    name == OsStr::new(PROTECTED_METADATA_AGENTS_PATH_NAME)
        || name == OsStr::new(PROTECTED_METADATA_CODEX_PATH_NAME)
}

/// Returns the protected workspace metadata name when an agent write to `path`
/// should be blocked before execution.
pub fn forbidden_agent_metadata_write(
    path: &Path,
    cwd: &Path,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> Option<&'static str> {
    if !matches!(
        file_system_sandbox_policy.kind,
        FileSystemSandboxKind::Restricted
    ) {
        return None;
    }

    let target = resolve_candidate_path(path, cwd)?;
    let (protected_metadata_path, metadata_name) =
        metadata_child_of_writable_root(file_system_sandbox_policy, target.as_path(), cwd)?;
    if has_explicit_write_entry_for_metadata_path(
        file_system_sandbox_policy,
        &protected_metadata_path,
        target.as_path(),
        cwd,
    ) {
        return None;
    }

    if !file_system_sandbox_policy.can_write_path_with_cwd(target.as_path(), cwd) {
        return Some(metadata_name);
    }

    None
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, Default, JsonSchema, TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum NetworkSandboxPolicy {
    #[default]
    Restricted,
    Enabled,
}

impl NetworkSandboxPolicy {
    pub fn is_enabled(self) -> bool {
        matches!(self, NetworkSandboxPolicy::Enabled)
    }
}

/// Access mode for a filesystem entry.
///
/// When two equally specific entries target the same path, we compare these by
/// conflict precedence rather than by capability breadth: `none` beats
/// `write`, and `write` beats `read`.
#[derive(
    Debug,
    Clone,
    Copy,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    Display,
    JsonSchema,
    TS,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum FileSystemAccessMode {
    Read,
    Write,
    None,
}

impl FileSystemAccessMode {
    pub fn can_read(self) -> bool {
        !matches!(self, FileSystemAccessMode::None)
    }

    pub fn can_write(self) -> bool {
        matches!(self, FileSystemAccessMode::Write)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind")]
pub enum FileSystemSpecialPath {
    Root,
    Minimal,
    #[serde(alias = "current_working_directory")]
    ProjectRoots {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        subpath: Option<PathBuf>,
    },
    Tmpdir,
    SlashTmp,
    /// WARNING: `:special_path` tokens are part of config compatibility.
    /// Do not make older runtimes reject newly introduced tokens.
    /// New parser support should be additive, while unknown values must stay
    /// representable so config from a newer Codex degrades to warn-and-ignore
    /// instead of failing to load. Codex 0.112.0 rejected unknown values here,
    /// which broke forward compatibility for newer config.
    /// Preserves future special-path tokens so older runtimes can ignore them
    /// without rejecting config authored by a newer release.
    Unknown {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        subpath: Option<PathBuf>,
    },
}

impl FileSystemSpecialPath {
    pub fn project_roots(subpath: Option<PathBuf>) -> Self {
        Self::ProjectRoots { subpath }
    }

    pub fn unknown(path: impl Into<String>, subpath: Option<PathBuf>) -> Self {
        Self::Unknown {
            path: path.into(),
            subpath,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct FileSystemSandboxEntry {
    pub path: FileSystemPath,
    pub access: FileSystemAccessMode,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, Default, JsonSchema, TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum FileSystemSandboxKind {
    #[default]
    Restricted,
    Unrestricted,
    ExternalSandbox,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct FileSystemSandboxPolicy {
    pub kind: FileSystemSandboxKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub glob_scan_max_depth: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<FileSystemSandboxEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFileSystemEntry {
    path: AbsolutePathBuf,
    access: FileSystemAccessMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSystemSemanticSignature {
    has_full_disk_read_access: bool,
    has_full_disk_write_access: bool,
    include_platform_defaults: bool,
    readable_roots: Vec<AbsolutePathBuf>,
    writable_roots: Vec<WritableRoot>,
    unreadable_roots: Vec<AbsolutePathBuf>,
    unreadable_globs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type")]
pub enum FileSystemPath {
    Path {
        path: AbsolutePathBuf,
    },
    /// A git-style glob pattern. Pattern entries currently support
    /// FileSystemAccessMode::None only.
    GlobPattern {
        pattern: String,
    },
    Special {
        value: FileSystemSpecialPath,
    },
}

const PROJECT_ROOTS_GLOB_PATTERN_PREFIX: &str = "codex-project-roots://";

pub fn project_roots_glob_pattern(subpath: &Path) -> String {
    format!("{PROJECT_ROOTS_GLOB_PATTERN_PREFIX}{}", subpath.display())
}

impl Default for FileSystemSandboxPolicy {
    fn default() -> Self {
        Self {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            }],
        }
    }
}

impl FileSystemSandboxPolicy {
    pub fn unrestricted() -> Self {
        Self {
            kind: FileSystemSandboxKind::Unrestricted,
            glob_scan_max_depth: None,
            entries: Vec::new(),
        }
    }

    pub fn external_sandbox() -> Self {
        Self {
            kind: FileSystemSandboxKind::ExternalSandbox,
            glob_scan_max_depth: None,
            entries: Vec::new(),
        }
    }

    pub fn restricted(entries: Vec<FileSystemSandboxEntry>) -> Self {
        Self {
            kind: FileSystemSandboxKind::Restricted,
            glob_scan_max_depth: None,
            entries,
        }
    }

    fn has_root_access(&self, predicate: impl Fn(FileSystemAccessMode) -> bool) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                matches!(
                    &entry.path,
                    FileSystemPath::Special { value }
                        if matches!(value, FileSystemSpecialPath::Root) && predicate(entry.access)
                )
            })
    }

    pub fn has_denied_read_restrictions(&self) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self
                .entries
                .iter()
                .any(|entry| entry.access == FileSystemAccessMode::None)
    }

    pub fn from_legacy_sandbox_policy_preserving_deny_entries(
        sandbox_policy: &SandboxPolicy,
        cwd: &Path,
        existing: &Self,
    ) -> Self {
        let mut rebuilt = Self::from_legacy_sandbox_policy_for_cwd(sandbox_policy, cwd);
        if !matches!(rebuilt.kind, FileSystemSandboxKind::Restricted) {
            return rebuilt;
        }
        rebuilt.glob_scan_max_depth = existing.glob_scan_max_depth;

        for deny_entry in existing
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::None)
        {
            if !rebuilt.entries.iter().any(|entry| entry == deny_entry) {
                rebuilt.entries.push(deny_entry.clone());
            }
        }

        rebuilt
    }

    /// Preserve explicit read-deny rules from `existing` when a caller
    /// replaces the allow side of a policy.
    pub fn preserve_deny_read_restrictions_from(&mut self, existing: &Self) {
        let has_deny_read_entries = existing
            .entries
            .iter()
            .any(|entry| entry.access == FileSystemAccessMode::None);
        if matches!(self.kind, FileSystemSandboxKind::Unrestricted) && has_deny_read_entries {
            *self = Self::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            }]);
        }

        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return;
        }

        if self.glob_scan_max_depth.is_none() {
            self.glob_scan_max_depth = existing.glob_scan_max_depth;
        }

        for deny_entry in existing
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::None)
        {
            if !self.entries.iter().any(|entry| entry == deny_entry) {
                self.entries.push(deny_entry.clone());
            }
        }
    }

    /// Returns true when a restricted policy contains any entry that really
    /// reduces a broader `:root = write` grant.
    ///
    /// Raw entry presence is not enough here: an equally specific `write`
    /// entry for the same target wins under the normal precedence rules, so a
    /// shadowed `read` entry must not downgrade the policy out of full-disk
    /// write mode.
    fn has_write_narrowing_entries(&self) -> bool {
        matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                if entry.access.can_write() {
                    return false;
                }

                match &entry.path {
                    FileSystemPath::Path { .. } => !self.has_same_target_write_override(entry),
                    FileSystemPath::GlobPattern { .. } => true,
                    FileSystemPath::Special { value } => match value {
                        FileSystemSpecialPath::Root => entry.access == FileSystemAccessMode::None,
                        FileSystemSpecialPath::Minimal | FileSystemSpecialPath::Unknown { .. } => {
                            false
                        }
                        _ => !self.has_same_target_write_override(entry),
                    },
                }
            })
    }

    /// Returns true when a higher-priority `write` entry targets the same
    /// location as `entry`, so `entry` cannot narrow effective write access.
    fn has_same_target_write_override(&self, entry: &FileSystemSandboxEntry) -> bool {
        self.entries.iter().any(|candidate| {
            candidate.access.can_write()
                && candidate.access > entry.access
                && file_system_paths_share_target(&candidate.path, &entry.path)
        })
    }

    /// Filesystem policy matching `WorkspaceWrite` semantics without requiring
    /// callers to construct a legacy [`SandboxPolicy`] first.
    pub fn workspace_write(
        writable_roots: &[AbsolutePathBuf],
        exclude_tmpdir_env_var: bool,
        exclude_slash_tmp: bool,
    ) -> Self {
        let mut entries = vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        }];

        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        });
        if !exclude_slash_tmp {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::SlashTmp,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        if !exclude_tmpdir_env_var {
            entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Tmpdir,
                },
                access: FileSystemAccessMode::Write,
            });
        }
        entries.extend(
            writable_roots
                .iter()
                .cloned()
                .map(|path| FileSystemSandboxEntry {
                    path: FileSystemPath::Path { path },
                    access: FileSystemAccessMode::Write,
                }),
        );

        append_default_read_only_project_root_subpath_if_no_explicit_rule(&mut entries, ".git");
        append_default_read_only_project_root_subpath_if_no_explicit_rule(&mut entries, ".agents");
        append_default_read_only_project_root_subpath_if_no_explicit_rule(&mut entries, ".codex");
        for writable_root in writable_roots {
            for protected_path in default_read_only_subpaths_for_writable_root(
                writable_root,
                /*protect_missing_dot_codex*/ false,
            ) {
                append_default_read_only_path_if_no_explicit_rule(&mut entries, protected_path);
            }
        }

        FileSystemSandboxPolicy::restricted(entries)
    }

    /// Converts a legacy sandbox policy into an equivalent filesystem policy
    /// after resolving cwd-sensitive legacy defaults for the provided cwd.
    ///
    /// Legacy `WorkspaceWrite` policies may list readable roots that live
    /// under an already-writable root. Those paths were redundant in the
    /// legacy model and should not become read-only carveouts when projected
    /// into split filesystem policy.
    pub fn from_legacy_sandbox_policy_for_cwd(sandbox_policy: &SandboxPolicy, cwd: &Path) -> Self {
        let mut file_system_policy = Self::from(sandbox_policy);
        if let SandboxPolicy::WorkspaceWrite { writable_roots, .. } = sandbox_policy {
            if let Ok(cwd_root) = AbsolutePathBuf::from_absolute_path(cwd) {
                for protected_path in default_read_only_subpaths_for_writable_root(
                    &cwd_root, /*protect_missing_dot_codex*/ true,
                ) {
                    append_default_read_only_path_if_no_explicit_rule(
                        &mut file_system_policy.entries,
                        protected_path,
                    );
                }
            }
            for writable_root in writable_roots {
                for protected_path in default_read_only_subpaths_for_writable_root(
                    writable_root,
                    /*protect_missing_dot_codex*/ false,
                ) {
                    append_default_read_only_path_if_no_explicit_rule(
                        &mut file_system_policy.entries,
                        protected_path,
                    );
                }
            }
        }

        file_system_policy
    }

    /// Returns true when filesystem reads are unrestricted.
    pub fn has_full_disk_read_access(&self) -> bool {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => true,
            FileSystemSandboxKind::Restricted => {
                self.has_root_access(FileSystemAccessMode::can_read)
                    && !self.has_denied_read_restrictions()
            }
        }
    }

    /// Returns true when filesystem writes are unrestricted.
    pub fn has_full_disk_write_access(&self) -> bool {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => true,
            FileSystemSandboxKind::Restricted => {
                self.has_root_access(FileSystemAccessMode::can_write)
                    && !self.has_write_narrowing_entries()
            }
        }
    }

    /// Returns true when platform-default readable roots should be included.
    pub fn include_platform_defaults(&self) -> bool {
        !self.has_full_disk_read_access()
            && matches!(self.kind, FileSystemSandboxKind::Restricted)
            && self.entries.iter().any(|entry| {
                matches!(
                    &entry.path,
                    FileSystemPath::Special { value }
                        if matches!(value, FileSystemSpecialPath::Minimal)
                            && entry.access.can_read()
                )
            })
    }

    pub fn resolve_access_with_cwd(&self, path: &Path, cwd: &Path) -> FileSystemAccessMode {
        match self.kind {
            FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                return FileSystemAccessMode::Write;
            }
            FileSystemSandboxKind::Restricted => {}
        }

        let Some(path) = resolve_candidate_path(path, cwd) else {
            return FileSystemAccessMode::None;
        };

        self.resolved_entries_with_cwd(cwd)
            .into_iter()
            .filter(|entry| path.as_path().starts_with(entry.path.as_path()))
            .max_by_key(resolved_entry_precedence)
            .map(|entry| entry.access)
            .unwrap_or(FileSystemAccessMode::None)
    }

    pub fn can_read_path_with_cwd(&self, path: &Path, cwd: &Path) -> bool {
        self.resolve_access_with_cwd(path, cwd).can_read()
    }

    pub fn can_write_path_with_cwd(&self, path: &Path, cwd: &Path) -> bool {
        if !self.resolve_access_with_cwd(path, cwd).can_write() {
            return false;
        }
        if self.has_full_disk_write_access() {
            return true;
        }
        !self.is_metadata_write_denied(path, cwd)
    }

    fn is_metadata_write_denied(&self, path: &Path, cwd: &Path) -> bool {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return false;
        }

        let Some(target) = resolve_candidate_path(path, cwd) else {
            return true;
        };
        let Some((protected_metadata_path, _)) =
            metadata_child_of_writable_root(self, target.as_path(), cwd)
        else {
            return false;
        };

        !has_explicit_write_entry_for_metadata_path(
            self,
            &protected_metadata_path,
            target.as_path(),
            cwd,
        )
    }

    /// Replaces symbolic `:workspace_roots` entries with absolute paths resolved
    /// against `cwd`.
    ///
    /// Use this when a durable permission profile must survive a cwd-only
    /// update without rebinding its project-root authority to the new cwd.
    pub fn materialize_project_roots_with_cwd(mut self, cwd: &Path) -> Self {
        let cwd = AbsolutePathBuf::from_absolute_path(cwd).ok();
        for entry in &mut self.entries {
            match &entry.path {
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { .. },
                } => {
                    if let Some(path) = resolve_file_system_path(&entry.path, cwd.as_ref()) {
                        entry.path = FileSystemPath::Path { path };
                    }
                }
                FileSystemPath::GlobPattern { pattern } => {
                    if let (Some(cwd), Some(subpath)) =
                        (cwd.as_ref(), parse_project_roots_glob_pattern(pattern))
                    {
                        entry.path = FileSystemPath::GlobPattern {
                            pattern: resolve_project_roots_glob_pattern(subpath, cwd),
                        };
                    }
                }
                FileSystemPath::Special { value: _ } => {}
                FileSystemPath::Path { .. } => {}
            }
        }
        self
    }

    /// Replaces symbolic `:workspace_roots` entries with concrete entries for
    /// each workspace root.
    pub fn materialize_project_roots_with_workspace_roots(
        mut self,
        workspace_roots: &[AbsolutePathBuf],
    ) -> Self {
        let mut entries = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            match entry.path {
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { subpath },
                } => {
                    entries.extend(workspace_roots.iter().map(|root| FileSystemSandboxEntry {
                        path: FileSystemPath::Path {
                            path: match subpath.as_ref() {
                                Some(subpath) => AbsolutePathBuf::resolve_path_against_base(
                                    subpath,
                                    root.as_path(),
                                ),
                                None => root.clone(),
                            },
                        },
                        access: entry.access,
                    }));
                }
                FileSystemPath::GlobPattern { pattern } => {
                    if let Some(subpath) = parse_project_roots_glob_pattern(&pattern) {
                        entries.extend(workspace_roots.iter().map(|root| FileSystemSandboxEntry {
                            path: FileSystemPath::GlobPattern {
                                pattern: resolve_project_roots_glob_pattern(subpath, root),
                            },
                            access: entry.access,
                        }));
                    } else {
                        entries.push(FileSystemSandboxEntry {
                            path: FileSystemPath::GlobPattern { pattern },
                            access: entry.access,
                        });
                    }
                }
                FileSystemPath::Path { path } => {
                    entries.push(FileSystemSandboxEntry {
                        path: FileSystemPath::Path { path },
                        access: entry.access,
                    });
                }
                FileSystemPath::Special { value } => {
                    entries.push(FileSystemSandboxEntry {
                        path: FileSystemPath::Special { value },
                        access: entry.access,
                    });
                }
            }
        }
        self.entries = entries;
        self
    }

    /// Preserves symbolic `:workspace_roots` entries while also adding concrete
    /// entries for each provided workspace root.
    pub fn with_materialized_project_roots_for_workspace_roots(
        mut self,
        workspace_roots: &[AbsolutePathBuf],
    ) -> Self {
        let materialized = self
            .clone()
            .materialize_project_roots_with_workspace_roots(workspace_roots);
        for entry in materialized.entries {
            if !self.entries.contains(&entry) {
                self.entries.push(entry);
            }
        }
        self
    }

    pub fn with_additional_readable_roots(
        mut self,
        cwd: &Path,
        additional_readable_roots: &[AbsolutePathBuf],
    ) -> Self {
        if self.has_full_disk_read_access() {
            return self;
        }

        for path in additional_readable_roots {
            if self.can_read_path_with_cwd(path.as_path(), cwd) {
                continue;
            }

            self.entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: path.clone() },
                access: FileSystemAccessMode::Read,
            });
        }

        self
    }

    pub fn with_additional_writable_roots(
        mut self,
        cwd: &Path,
        additional_writable_roots: &[AbsolutePathBuf],
    ) -> Self {
        for path in additional_writable_roots {
            if self.can_write_path_with_cwd(path.as_path(), cwd) {
                continue;
            }

            self.entries.push(FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: path.clone() },
                access: FileSystemAccessMode::Write,
            });
        }

        self
    }

    /// Add roots using legacy `WorkspaceWrite` behavior.
    ///
    /// Unlike [`Self::with_additional_writable_roots`], this mirrors legacy
    /// writable-roots semantics by adding exact roots even when they are
    /// already writable through `:workspace_roots`, and by adding the default
    /// read-only protected subpaths for each new root.
    pub fn with_additional_legacy_workspace_writable_roots(
        mut self,
        additional_writable_roots: &[AbsolutePathBuf],
    ) -> Self {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return self;
        }

        for path in additional_writable_roots {
            if !self.entries.iter().any(|entry| {
                entry.access.can_write()
                    && matches!(&entry.path, FileSystemPath::Path { path: existing } if existing == path)
            }) {
                self.entries.push(FileSystemSandboxEntry {
                    path: FileSystemPath::Path { path: path.clone() },
                    access: FileSystemAccessMode::Write,
                });
            }

            for protected_path in default_read_only_subpaths_for_writable_root(
                path, /*protect_missing_dot_codex*/ false,
            ) {
                append_default_read_only_path_if_no_explicit_rule(
                    &mut self.entries,
                    protected_path,
                );
            }
        }

        self
    }

    pub fn needs_direct_runtime_enforcement(
        &self,
        network_policy: NetworkSandboxPolicy,
        cwd: &Path,
    ) -> bool {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return false;
        }

        let Ok(legacy_policy) = self.to_legacy_sandbox_policy(network_policy, cwd) else {
            return true;
        };

        if protected_metadata_names_need_direct_runtime_enforcement(self, &legacy_policy, cwd) {
            return true;
        }

        self.semantic_signature(cwd)
            != legacy_runtime_file_system_policy_for_cwd(&legacy_policy, cwd)
                .semantic_signature(cwd)
    }

    /// Returns true when two policies resolve to the same filesystem access
    /// model for `cwd`, ignoring incidental entry ordering.
    pub fn is_semantically_equivalent_to(&self, other: &Self, cwd: &Path) -> bool {
        self.semantic_signature(cwd) == other.semantic_signature(cwd)
    }

    /// Returns the explicit readable roots resolved against the provided cwd.
    pub fn get_readable_roots_with_cwd(&self, cwd: &Path) -> Vec<AbsolutePathBuf> {
        if self.has_full_disk_read_access() {
            return Vec::new();
        }

        dedup_absolute_paths(
            self.resolved_entries_with_cwd(cwd)
                .into_iter()
                .filter(|entry| entry.access.can_read())
                .filter(|entry| self.can_read_path_with_cwd(entry.path.as_path(), cwd))
                .map(|entry| entry.path)
                .collect(),
            /*normalize_effective_paths*/ true,
        )
    }

    /// Returns the writable roots together with read-only carveouts resolved
    /// against the provided cwd.
    pub fn get_writable_roots_with_cwd(&self, cwd: &Path) -> Vec<WritableRoot> {
        if self.has_full_disk_write_access() {
            return Vec::new();
        }

        let resolved_entries = self.resolved_entries_with_cwd(cwd);
        let writable_entries: Vec<AbsolutePathBuf> = resolved_entries
            .iter()
            .filter(|entry| entry.access.can_write())
            .filter(|entry| self.can_write_path_with_cwd(entry.path.as_path(), cwd))
            .map(|entry| entry.path.clone())
            .collect();

        dedup_absolute_paths(
            writable_entries.clone(),
            /*normalize_effective_paths*/ true,
        )
        .into_iter()
        .map(|root| {
            // Filesystem-root policies stay in their effective canonical form
            // so root-wide aliases do not create duplicate top-level masks.
            // Example: keep `/var/...` normalized under `/` instead of
            // materializing both `/var/...` and `/private/var/...`.
            // Nested symlink paths under a writable root stay logical so
            // downstream sandboxes can still bind the real target while
            // masking the user-visible symlink inode when needed.
            let preserve_raw_carveout_paths = root.as_path().parent().is_some();
            let raw_writable_roots: Vec<&AbsolutePathBuf> = writable_entries
                .iter()
                .filter(|path| normalize_effective_absolute_path((*path).clone()) == root)
                .collect();
            let protected_metadata_names =
                protected_metadata_names_for_writable_root(self, &root, &raw_writable_roots, cwd);
            let protect_missing_dot_codex = AbsolutePathBuf::from_absolute_path(cwd)
                .ok()
                .is_some_and(|cwd| normalize_effective_absolute_path(cwd) == root);
            let mut read_only_subpaths: Vec<AbsolutePathBuf> =
                default_read_only_subpaths_for_writable_root(&root, protect_missing_dot_codex)
                    .into_iter()
                    .filter(|path| !has_explicit_resolved_path_entry(&resolved_entries, path))
                    .collect();
            // Narrower explicit non-write entries carve out broader writable roots.
            // More specific write entries still remain writable because they appear
            // as separate WritableRoot values and are checked independently.
            // Preserve symlink path components that live under the writable root
            // so downstream sandboxes can still mask the symlink inode itself.
            // Example: if `<root>/.codex -> <root>/decoy`, bwrap must still see
            // `<root>/.codex`, not only the resolved `<root>/decoy`.
            read_only_subpaths.extend(
                resolved_entries
                    .iter()
                    .filter(|entry| !entry.access.can_write())
                    .filter(|entry| !self.can_write_path_with_cwd(entry.path.as_path(), cwd))
                    .filter_map(|entry| {
                        let effective_path = normalize_effective_absolute_path(entry.path.clone());
                        // Preserve the literal in-root path whenever the
                        // carveout itself lives under this writable root, even
                        // if following symlinks would resolve back to the root
                        // or escape outside it. Downstream sandboxes need that
                        // raw path so they can mask the symlink inode itself.
                        // Examples:
                        // - `<root>/linked-private -> <root>/decoy-private`
                        // - `<root>/linked-private -> /tmp/outside-private`
                        // - `<root>/alias-root -> <root>`
                        let raw_carveout_path = if preserve_raw_carveout_paths {
                            if entry.path == root {
                                None
                            } else if entry.path.as_path().starts_with(root.as_path()) {
                                Some(entry.path.clone())
                            } else {
                                raw_writable_roots.iter().find_map(|raw_root| {
                                    let suffix = entry
                                        .path
                                        .as_path()
                                        .strip_prefix(raw_root.as_path())
                                        .ok()?;
                                    if suffix.as_os_str().is_empty() {
                                        return None;
                                    }
                                    Some(root.join(suffix))
                                })
                            }
                        } else {
                            None
                        };

                        if let Some(raw_carveout_path) = raw_carveout_path {
                            return Some(raw_carveout_path);
                        }

                        if effective_path == root
                            || !effective_path.as_path().starts_with(root.as_path())
                        {
                            return None;
                        }

                        Some(effective_path)
                    }),
            );
            WritableRoot {
                protected_metadata_names,
                root,
                // Preserve literal in-root protected paths like `.git` and
                // `.codex` so downstream sandboxes can still detect and mask
                // the symlink itself instead of only its resolved target.
                read_only_subpaths: dedup_absolute_paths(
                    read_only_subpaths,
                    /*normalize_effective_paths*/ false,
                ),
            }
        })
        .collect()
    }

    /// Returns explicit unreadable roots resolved against the provided cwd.
    pub fn get_unreadable_roots_with_cwd(&self, cwd: &Path) -> Vec<AbsolutePathBuf> {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return Vec::new();
        }

        let root = AbsolutePathBuf::from_absolute_path(cwd)
            .ok()
            .map(|cwd| absolute_root_path_for_cwd(&cwd));

        dedup_absolute_paths(
            self.resolved_entries_with_cwd(cwd)
                .iter()
                .filter(|entry| entry.access == FileSystemAccessMode::None)
                .filter(|entry| !self.can_read_path_with_cwd(entry.path.as_path(), cwd))
                // Restricted policies already deny reads outside explicit allow roots,
                // so materializing the filesystem root here would erase narrower
                // readable carveouts when downstream sandboxes apply deny masks last.
                .filter(|entry| root.as_ref() != Some(&entry.path))
                .map(|entry| entry.path.clone())
                .collect(),
            /*normalize_effective_paths*/ true,
        )
    }

    /// Returns unreadable glob patterns resolved against the provided cwd.
    pub fn get_unreadable_globs_with_cwd(&self, cwd: &Path) -> Vec<String> {
        if !matches!(self.kind, FileSystemSandboxKind::Restricted) {
            return Vec::new();
        }

        let mut patterns = self
            .entries
            .iter()
            .filter(|entry| entry.access == FileSystemAccessMode::None)
            .filter_map(|entry| match &entry.path {
                FileSystemPath::GlobPattern { pattern } => {
                    Some(AbsolutePathBuf::resolve_path_against_base(pattern, cwd))
                }
                FileSystemPath::Path { .. } | FileSystemPath::Special { .. } => None,
            })
            .map(|pattern| pattern.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        patterns.sort();
        patterns.dedup();
        patterns
    }

    pub fn to_legacy_sandbox_policy(
        &self,
        network_policy: NetworkSandboxPolicy,
        cwd: &Path,
    ) -> io::Result<SandboxPolicy> {
        Ok(match self.kind {
            FileSystemSandboxKind::ExternalSandbox => SandboxPolicy::ExternalSandbox {
                network_access: if network_policy.is_enabled() {
                    NetworkAccess::Enabled
                } else {
                    NetworkAccess::Restricted
                },
            },
            FileSystemSandboxKind::Unrestricted => {
                if network_policy.is_enabled() {
                    SandboxPolicy::DangerFullAccess
                } else {
                    SandboxPolicy::ExternalSandbox {
                        network_access: NetworkAccess::Restricted,
                    }
                }
            }
            FileSystemSandboxKind::Restricted => {
                let cwd_absolute = AbsolutePathBuf::from_absolute_path(cwd).ok();
                let has_full_disk_write_access = self.has_full_disk_write_access();
                let mut workspace_root_writable = false;
                let mut writable_roots = Vec::new();
                let mut tmpdir_writable = false;
                let mut slash_tmp_writable = false;
                let mut unbridgeable_root_write = false;

                for entry in &self.entries {
                    match &entry.path {
                        FileSystemPath::GlobPattern { .. } => {}
                        FileSystemPath::Path { path } => {
                            if entry.access.can_write() {
                                if cwd_absolute.as_ref().is_some_and(|cwd| cwd == path) {
                                    workspace_root_writable = true;
                                } else {
                                    writable_roots.push(path.clone());
                                }
                            }
                        }
                        FileSystemPath::Special { value } => match value {
                            FileSystemSpecialPath::Root => match entry.access {
                                FileSystemAccessMode::None => {}
                                FileSystemAccessMode::Read => {}
                                FileSystemAccessMode::Write => {
                                    unbridgeable_root_write = true;
                                }
                            },
                            FileSystemSpecialPath::Minimal => {}
                            FileSystemSpecialPath::ProjectRoots { subpath } => {
                                if subpath.is_none() && entry.access.can_write() {
                                    workspace_root_writable = true;
                                } else if let Some(path) =
                                    resolve_file_system_special_path(value, cwd_absolute.as_ref())
                                    && entry.access.can_write()
                                {
                                    writable_roots.push(path);
                                }
                            }
                            FileSystemSpecialPath::Tmpdir => {
                                if entry.access.can_write() {
                                    tmpdir_writable = true;
                                }
                            }
                            FileSystemSpecialPath::SlashTmp => {
                                if entry.access.can_write() {
                                    slash_tmp_writable = true;
                                }
                            }
                            FileSystemSpecialPath::Unknown { .. } => {}
                        },
                    }
                }

                if has_full_disk_write_access {
                    return Ok(if network_policy.is_enabled() {
                        SandboxPolicy::DangerFullAccess
                    } else {
                        SandboxPolicy::ExternalSandbox {
                            network_access: NetworkAccess::Restricted,
                        }
                    });
                }

                if workspace_root_writable {
                    SandboxPolicy::WorkspaceWrite {
                        writable_roots: dedup_absolute_paths(
                            writable_roots,
                            /*normalize_effective_paths*/ false,
                        ),
                        network_access: network_policy.is_enabled(),
                        exclude_tmpdir_env_var: !tmpdir_writable,
                        exclude_slash_tmp: !slash_tmp_writable,
                    }
                } else if unbridgeable_root_write
                    || !writable_roots.is_empty()
                    || tmpdir_writable
                    || slash_tmp_writable
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "permissions profile requests filesystem writes outside the workspace root, which is not supported until the runtime enforces FileSystemSandboxPolicy directly",
                    ));
                } else {
                    SandboxPolicy::ReadOnly {
                        network_access: network_policy.is_enabled(),
                    }
                }
            }
        })
    }

    fn resolved_entries_with_cwd(&self, cwd: &Path) -> Vec<ResolvedFileSystemEntry> {
        let cwd_absolute = AbsolutePathBuf::from_absolute_path(cwd).ok();
        self.entries
            .iter()
            .filter_map(|entry| {
                resolve_entry_path(&entry.path, cwd_absolute.as_ref()).map(|path| {
                    ResolvedFileSystemEntry {
                        path,
                        access: entry.access,
                    }
                })
            })
            .collect()
    }

    fn semantic_signature(&self, cwd: &Path) -> FileSystemSemanticSignature {
        FileSystemSemanticSignature {
            has_full_disk_read_access: self.has_full_disk_read_access(),
            has_full_disk_write_access: self.has_full_disk_write_access(),
            include_platform_defaults: self.include_platform_defaults(),
            readable_roots: sorted_absolute_paths(self.get_readable_roots_with_cwd(cwd)),
            writable_roots: sorted_writable_roots(self.get_writable_roots_with_cwd(cwd)),
            unreadable_roots: sorted_absolute_paths(self.get_unreadable_roots_with_cwd(cwd)),
            unreadable_globs: self.get_unreadable_globs_with_cwd(cwd),
        }
    }
}

impl From<&SandboxPolicy> for NetworkSandboxPolicy {
    fn from(value: &SandboxPolicy) -> Self {
        if value.has_full_network_access() {
            NetworkSandboxPolicy::Enabled
        } else {
            NetworkSandboxPolicy::Restricted
        }
    }
}

impl From<&SandboxPolicy> for FileSystemSandboxPolicy {
    fn from(value: &SandboxPolicy) -> Self {
        match value {
            SandboxPolicy::DangerFullAccess => FileSystemSandboxPolicy::unrestricted(),
            SandboxPolicy::ExternalSandbox { .. } => FileSystemSandboxPolicy::external_sandbox(),
            SandboxPolicy::ReadOnly { .. } => {
                FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    access: FileSystemAccessMode::Read,
                }])
            }
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                exclude_tmpdir_env_var,
                exclude_slash_tmp,
                ..
            } => FileSystemSandboxPolicy::workspace_write(
                writable_roots,
                *exclude_tmpdir_env_var,
                *exclude_slash_tmp,
            ),
        }
    }
}

fn resolve_file_system_path(
    path: &FileSystemPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match path {
        FileSystemPath::Path { path } => Some(path.clone()),
        FileSystemPath::GlobPattern { .. } => None,
        FileSystemPath::Special { value } => resolve_file_system_special_path(value, cwd),
    }
}

fn resolve_entry_path(
    path: &FileSystemPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match path {
        FileSystemPath::Special {
            value: FileSystemSpecialPath::Root,
        } => cwd.map(absolute_root_path_for_cwd),
        _ => resolve_file_system_path(path, cwd),
    }
}

fn parse_project_roots_glob_pattern(pattern: &str) -> Option<&Path> {
    pattern
        .strip_prefix(PROJECT_ROOTS_GLOB_PATTERN_PREFIX)
        .map(Path::new)
}

fn resolve_project_roots_glob_pattern(subpath: &Path, root: &AbsolutePathBuf) -> String {
    AbsolutePathBuf::resolve_path_against_base(subpath, root.as_path())
        .to_string_lossy()
        .into_owned()
}

fn resolve_candidate_path(path: &Path, cwd: &Path) -> Option<AbsolutePathBuf> {
    if path.is_absolute() {
        AbsolutePathBuf::from_absolute_path(path).ok()
    } else {
        Some(AbsolutePathBuf::from_absolute_path(cwd).ok()?.join(path))
    }
}

/// Returns true when two config paths refer to the same exact target before
/// any prefix matching is applied.
///
/// This is intentionally narrower than full path resolution: it only answers
/// the "can one entry shadow another at the same specificity?" question used
/// by `has_write_narrowing_entries`.
fn file_system_paths_share_target(left: &FileSystemPath, right: &FileSystemPath) -> bool {
    match (left, right) {
        (FileSystemPath::Path { path: left }, FileSystemPath::Path { path: right }) => {
            left == right
        }
        (FileSystemPath::Special { value: left }, FileSystemPath::Special { value: right }) => {
            special_paths_share_target(left, right)
        }
        (FileSystemPath::Path { path }, FileSystemPath::Special { value })
        | (FileSystemPath::Special { value }, FileSystemPath::Path { path }) => {
            special_path_matches_absolute_path(value, path)
        }
        (
            FileSystemPath::GlobPattern { pattern: left },
            FileSystemPath::GlobPattern { pattern: right },
        ) => left == right,
        (FileSystemPath::GlobPattern { .. }, _) | (_, FileSystemPath::GlobPattern { .. }) => false,
    }
}

/// Compares special-path tokens that resolve to the same concrete target
/// without needing a cwd.
fn special_paths_share_target(left: &FileSystemSpecialPath, right: &FileSystemSpecialPath) -> bool {
    match (left, right) {
        (FileSystemSpecialPath::Root, FileSystemSpecialPath::Root)
        | (FileSystemSpecialPath::Minimal, FileSystemSpecialPath::Minimal)
        | (FileSystemSpecialPath::Tmpdir, FileSystemSpecialPath::Tmpdir)
        | (FileSystemSpecialPath::SlashTmp, FileSystemSpecialPath::SlashTmp) => true,
        (
            FileSystemSpecialPath::ProjectRoots { subpath: left },
            FileSystemSpecialPath::ProjectRoots { subpath: right },
        ) => left == right,
        (
            FileSystemSpecialPath::Unknown {
                path: left,
                subpath: left_subpath,
            },
            FileSystemSpecialPath::Unknown {
                path: right,
                subpath: right_subpath,
            },
        ) => left == right && left_subpath == right_subpath,
        _ => false,
    }
}

/// Matches cwd-independent special paths against absolute `Path` entries when
/// they name the same location.
///
/// We intentionally only fold the special paths whose concrete meaning is
/// stable without a cwd, such as `/` and `/tmp`.
fn special_path_matches_absolute_path(
    value: &FileSystemSpecialPath,
    path: &AbsolutePathBuf,
) -> bool {
    match value {
        FileSystemSpecialPath::Root => path.as_path().parent().is_none(),
        FileSystemSpecialPath::SlashTmp => path.as_path() == Path::new("/tmp"),
        _ => false,
    }
}

/// Orders resolved entries so the most specific path wins first, then applies
/// the access tie-breaker from [`FileSystemAccessMode`].
fn resolved_entry_precedence(entry: &ResolvedFileSystemEntry) -> (usize, FileSystemAccessMode) {
    let specificity = entry.path.as_path().components().count();
    (specificity, entry.access)
}

fn absolute_root_path_for_cwd(cwd: &AbsolutePathBuf) -> AbsolutePathBuf {
    let root = cwd
        .as_path()
        .ancestors()
        .last()
        .unwrap_or_else(|| panic!("cwd must have a filesystem root"));
    AbsolutePathBuf::from_absolute_path(root)
        .unwrap_or_else(|err| panic!("cwd root must be an absolute path: {err}"))
}

fn resolve_file_system_special_path(
    value: &FileSystemSpecialPath,
    cwd: Option<&AbsolutePathBuf>,
) -> Option<AbsolutePathBuf> {
    match value {
        FileSystemSpecialPath::Root
        | FileSystemSpecialPath::Minimal
        | FileSystemSpecialPath::Unknown { .. } => None,
        FileSystemSpecialPath::ProjectRoots { subpath } => {
            let cwd = cwd?;
            match subpath.as_ref() {
                Some(subpath) => Some(AbsolutePathBuf::resolve_path_against_base(
                    subpath,
                    cwd.as_path(),
                )),
                None => Some(cwd.clone()),
            }
        }
        FileSystemSpecialPath::Tmpdir => {
            let tmpdir = std::env::var_os("TMPDIR")?;
            if tmpdir.is_empty() {
                None
            } else {
                let tmpdir = AbsolutePathBuf::from_absolute_path(PathBuf::from(tmpdir)).ok()?;
                Some(tmpdir)
            }
        }
        FileSystemSpecialPath::SlashTmp => {
            #[allow(clippy::expect_used)]
            let slash_tmp = AbsolutePathBuf::from_absolute_path("/tmp").expect("/tmp is absolute");
            if !slash_tmp.as_path().is_dir() {
                return None;
            }
            Some(slash_tmp)
        }
    }
}

fn dedup_absolute_paths(
    paths: Vec<AbsolutePathBuf>,
    normalize_effective_paths: bool,
) -> Vec<AbsolutePathBuf> {
    let mut deduped = Vec::with_capacity(paths.len());
    let mut seen = HashSet::new();
    for path in paths {
        let dedup_path = if normalize_effective_paths {
            normalize_effective_absolute_path(path)
        } else {
            path
        };
        if seen.insert(dedup_path.to_path_buf()) {
            deduped.push(dedup_path);
        }
    }
    deduped
}

fn sorted_absolute_paths(mut paths: Vec<AbsolutePathBuf>) -> Vec<AbsolutePathBuf> {
    paths.sort_by(|left, right| left.as_path().cmp(right.as_path()));
    paths
}

fn sorted_writable_roots(mut roots: Vec<WritableRoot>) -> Vec<WritableRoot> {
    for root in &mut roots {
        root.read_only_subpaths =
            sorted_absolute_paths(std::mem::take(&mut root.read_only_subpaths));
        root.protected_metadata_names.sort();
        root.protected_metadata_names.dedup();
    }
    roots.sort_by(|left, right| left.root.as_path().cmp(right.root.as_path()));
    roots
}

fn normalize_effective_absolute_path(path: AbsolutePathBuf) -> AbsolutePathBuf {
    let raw_path = path.to_path_buf();
    for ancestor in raw_path.ancestors() {
        if std::fs::symlink_metadata(ancestor).is_err() {
            continue;
        }
        let Ok(normalized_ancestor) = canonicalize_preserving_symlinks(ancestor) else {
            continue;
        };
        let Ok(suffix) = raw_path.strip_prefix(ancestor) else {
            continue;
        };
        if let Ok(normalized_path) =
            AbsolutePathBuf::from_absolute_path(normalized_ancestor.join(suffix))
        {
            return normalized_path;
        }
    }
    path
}

pub(crate) fn default_read_only_subpaths_for_writable_root(
    writable_root: &AbsolutePathBuf,
    protect_missing_dot_codex: bool,
) -> Vec<AbsolutePathBuf> {
    let mut subpaths: Vec<AbsolutePathBuf> = Vec::new();
    let top_level_git = writable_root.join(PROTECTED_METADATA_GIT_PATH_NAME);
    // This applies to typical repos (directory .git), worktrees/submodules
    // (file .git with gitdir pointer), and bare repos when the gitdir is the
    // writable root itself.
    let top_level_git_is_file = top_level_git.as_path().is_file();
    let top_level_git_is_dir = top_level_git.as_path().is_dir();
    let should_protect_top_level = top_level_git_is_dir || top_level_git_is_file;
    if should_protect_top_level {
        if top_level_git_is_file
            && is_git_pointer_file(&top_level_git)
            && let Some(gitdir) = resolve_gitdir_from_file(&top_level_git)
        {
            subpaths.push(gitdir);
        }
        subpaths.push(top_level_git);
    }

    let top_level_agents = writable_root.join(PROTECTED_METADATA_AGENTS_PATH_NAME);
    if top_level_agents.as_path().is_dir() {
        subpaths.push(top_level_agents);
    }

    // Keep top-level project metadata under .codex read-only to the agent by
    // default. For the workspace root itself, protect it even before the
    // directory exists so first-time creation still goes through the
    // protected-path approval flow.
    let top_level_codex = writable_root.join(PROTECTED_METADATA_CODEX_PATH_NAME);
    if protect_missing_dot_codex || top_level_codex.as_path().is_dir() {
        subpaths.push(top_level_codex);
    }

    dedup_absolute_paths(subpaths, /*normalize_effective_paths*/ false)
}

/// Rebuilds the filesystem policy that legacy sandbox runtimes enforce for a
/// concrete cwd.
///
/// Unlike [`FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd`], this
/// intentionally does not add symbolic project-root metadata carveouts. Legacy
/// runtime expansion only protected `.git`/`.agents` when those paths already
/// existed, so missing-path carveouts still require direct profile enforcement.
fn legacy_runtime_file_system_policy_for_cwd(
    sandbox_policy: &SandboxPolicy,
    cwd: &Path,
) -> FileSystemSandboxPolicy {
    let SandboxPolicy::WorkspaceWrite {
        writable_roots,
        exclude_tmpdir_env_var,
        exclude_slash_tmp,
        ..
    } = sandbox_policy
    else {
        return FileSystemSandboxPolicy::from(sandbox_policy);
    };

    let mut entries = vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        },
    ];

    if !*exclude_slash_tmp {
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::SlashTmp,
            },
            access: FileSystemAccessMode::Write,
        });
    }
    if !*exclude_tmpdir_env_var {
        entries.push(FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Tmpdir,
            },
            access: FileSystemAccessMode::Write,
        });
    }
    entries.extend(
        writable_roots
            .iter()
            .cloned()
            .map(|path| FileSystemSandboxEntry {
                path: FileSystemPath::Path { path },
                access: FileSystemAccessMode::Write,
            }),
    );

    if let Ok(cwd_root) = AbsolutePathBuf::from_absolute_path(cwd) {
        for protected_path in default_read_only_subpaths_for_writable_root(
            &cwd_root, /*protect_missing_dot_codex*/ true,
        ) {
            append_default_read_only_path_if_no_explicit_rule(&mut entries, protected_path);
        }
    }
    for writable_root in writable_roots {
        for protected_path in default_read_only_subpaths_for_writable_root(
            writable_root,
            /*protect_missing_dot_codex*/ false,
        ) {
            append_default_read_only_path_if_no_explicit_rule(&mut entries, protected_path);
        }
    }

    FileSystemSandboxPolicy::restricted(entries)
}

fn append_default_read_only_project_root_subpath_if_no_explicit_rule(
    entries: &mut Vec<FileSystemSandboxEntry>,
    subpath: impl Into<PathBuf>,
) {
    append_default_read_only_entry_if_no_explicit_rule(
        entries,
        FileSystemPath::Special {
            value: FileSystemSpecialPath::project_roots(Some(subpath.into())),
        },
    );
}

fn append_default_read_only_path_if_no_explicit_rule(
    entries: &mut Vec<FileSystemSandboxEntry>,
    path: AbsolutePathBuf,
) {
    append_default_read_only_entry_if_no_explicit_rule(entries, FileSystemPath::Path { path });
}

fn append_default_read_only_entry_if_no_explicit_rule(
    entries: &mut Vec<FileSystemSandboxEntry>,
    path: FileSystemPath,
) {
    if entries
        .iter()
        .any(|entry| file_system_paths_share_target(&entry.path, &path))
    {
        return;
    }

    entries.push(FileSystemSandboxEntry {
        path,
        access: FileSystemAccessMode::Read,
    });
}

fn has_explicit_resolved_path_entry(
    entries: &[ResolvedFileSystemEntry],
    path: &AbsolutePathBuf,
) -> bool {
    entries.iter().any(|entry| &entry.path == path)
}

fn metadata_path_name(name: &OsStr) -> Option<&'static str> {
    PROTECTED_METADATA_PATH_NAMES
        .iter()
        .copied()
        .find(|metadata_name| name == OsStr::new(metadata_name))
}

fn metadata_child_of_writable_root(
    policy: &FileSystemSandboxPolicy,
    target: &Path,
    cwd: &Path,
) -> Option<(AbsolutePathBuf, &'static str)> {
    policy
        .resolved_entries_with_cwd(cwd)
        .iter()
        .filter(|entry| entry.access.can_write())
        .filter_map(|entry| {
            let relative_path = target.strip_prefix(entry.path.as_path()).ok()?;
            let first_component = relative_path.components().next()?;
            let metadata_name = metadata_path_name(first_component.as_os_str())?;
            Some((entry.path.join(metadata_name), metadata_name))
        })
        .next()
}

fn protected_metadata_names_for_writable_root(
    policy: &FileSystemSandboxPolicy,
    root: &AbsolutePathBuf,
    raw_writable_roots: &[&AbsolutePathBuf],
    cwd: &Path,
) -> Vec<String> {
    let mut protected_names = Vec::new();
    for metadata_name in PROTECTED_METADATA_PATH_NAMES {
        let mut metadata_paths = vec![root.join(*metadata_name)];
        metadata_paths.extend(
            raw_writable_roots
                .iter()
                .map(|raw_root| raw_root.join(*metadata_name)),
        );

        if metadata_paths
            .iter()
            .all(|metadata_path| !policy.can_write_path_with_cwd(metadata_path.as_path(), cwd))
        {
            protected_names.push((*metadata_name).to_string());
        }
    }
    protected_names
}

fn protected_metadata_names_need_direct_runtime_enforcement(
    policy: &FileSystemSandboxPolicy,
    legacy_policy: &SandboxPolicy,
    cwd: &Path,
) -> bool {
    let legacy_roots = legacy_policy.get_writable_roots_with_cwd(cwd);
    policy
        .get_writable_roots_with_cwd(cwd)
        .into_iter()
        .any(|writable_root| {
            let Some(legacy_root) = legacy_roots
                .iter()
                .find(|candidate| candidate.root == writable_root.root)
            else {
                return !writable_root.protected_metadata_names.is_empty();
            };

            writable_root
                .protected_metadata_names
                .iter()
                .any(|metadata_name| {
                    let metadata_path = writable_root.root.join(metadata_name);
                    !legacy_root
                        .read_only_subpaths
                        .iter()
                        .any(|subpath| subpath == &metadata_path)
                })
        })
}

fn has_explicit_write_entry_for_metadata_path(
    policy: &FileSystemSandboxPolicy,
    protected_metadata_path: &AbsolutePathBuf,
    target: &Path,
    cwd: &Path,
) -> bool {
    policy.resolved_entries_with_cwd(cwd).iter().any(|entry| {
        entry.access.can_write()
            && target.starts_with(entry.path.as_path())
            && entry
                .path
                .as_path()
                .starts_with(protected_metadata_path.as_path())
    })
}

fn is_git_pointer_file(path: &AbsolutePathBuf) -> bool {
    path.as_path().is_file()
        && path.as_path().file_name() == Some(OsStr::new(PROTECTED_METADATA_GIT_PATH_NAME))
}

fn resolve_gitdir_from_file(dot_git: &AbsolutePathBuf) -> Option<AbsolutePathBuf> {
    let contents = match std::fs::read_to_string(dot_git.as_path()) {
        Ok(contents) => contents,
        Err(err) => {
            error!(
                "Failed to read {path} for gitdir pointer: {err}",
                path = dot_git.as_path().display()
            );
            return None;
        }
    };

    let trimmed = contents.trim();
    let (_, gitdir_raw) = match trimmed.split_once(':') {
        Some((prefix, gitdir_raw)) if prefix.trim() == "gitdir" => (prefix, gitdir_raw),
        Some(_) => {
            error!(
                "Expected {path} to contain a gitdir pointer, but it did not match `gitdir: <path>`.",
                path = dot_git.as_path().display()
            );
            return None;
        }
        None => {
            error!(
                "Expected {path} to contain a gitdir pointer, but it did not match `gitdir: <path>`.",
                path = dot_git.as_path().display()
            );
            return None;
        }
    };
    let gitdir_raw = gitdir_raw.trim();
    if gitdir_raw.is_empty() {
        error!(
            "Expected {path} to contain a gitdir pointer, but it was empty.",
            path = dot_git.as_path().display()
        );
        return None;
    }
    let base = match dot_git.as_path().parent() {
        Some(base) => base,
        None => {
            error!(
                "Unable to resolve parent directory for {path}.",
                path = dot_git.as_path().display()
            );
            return None;
        }
    };
    let gitdir_path = AbsolutePathBuf::resolve_path_against_base(gitdir_raw, base);
    if !gitdir_path.as_path().exists() {
        error!(
            "Resolved gitdir path {path} does not exist.",
            path = gitdir_path.as_path().display()
        );
        return None;
    }
    Some(gitdir_path)
}

#[cfg(test)]
mod tests;
