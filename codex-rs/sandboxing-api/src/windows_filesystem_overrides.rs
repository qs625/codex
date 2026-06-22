use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::permissions::FileSystemSandboxKind;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path::normalize_for_native_workdir;

use crate::SandboxType;
use crate::resolve_windows_deny_read_paths;

/// Resolved filesystem overrides for the Windows sandbox backends.
///
/// The elevated Windows backend consumes extra deny-read paths plus explicit
/// read and write roots during setup/refresh. The unelevated restricted-token
/// backend only consumes extra deny-write carveouts on top of the legacy
/// `WorkspaceWrite` allow set. Read-root overrides are layered on top of the
/// baseline helper roots that the elevated setup path needs to launch the
/// sandboxed command; split policies that opt into platform defaults carry
/// that explicitly with the override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsSandboxFilesystemOverrides {
    pub read_roots_override: Option<Vec<PathBuf>>,
    pub read_roots_include_platform_defaults: bool,
    pub write_roots_override: Option<Vec<PathBuf>>,
    pub additional_deny_read_paths: Vec<AbsolutePathBuf>,
    pub additional_deny_write_paths: Vec<AbsolutePathBuf>,
}

pub fn windows_sandbox_uses_elevated_backend(
    sandbox_level: WindowsSandboxLevel,
    proxy_enforced: bool,
) -> bool {
    // Windows firewall enforcement is tied to the logon-user sandbox identities, so
    // proxy-enforced sessions must use that backend even when the configured mode is
    // the default restricted-token sandbox.
    proxy_enforced || matches!(sandbox_level, WindowsSandboxLevel::Elevated)
}

pub fn should_use_windows_restricted_token_sandbox(
    sandbox: SandboxType,
    sandbox_policy: &SandboxPolicy,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> bool {
    sandbox == SandboxType::WindowsRestrictedToken
        && file_system_sandbox_policy.kind == FileSystemSandboxKind::Restricted
        && !matches!(
            sandbox_policy,
            SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. }
        )
}

pub fn unsupported_windows_restricted_token_sandbox_reason(
    sandbox: SandboxType,
    sandbox_policy: &SandboxPolicy,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    sandbox_policy_cwd: &AbsolutePathBuf,
    windows_sandbox_level: WindowsSandboxLevel,
) -> Option<String> {
    if windows_sandbox_level == WindowsSandboxLevel::Elevated {
        resolve_windows_elevated_filesystem_overrides(
            sandbox,
            sandbox_policy,
            file_system_sandbox_policy,
            network_sandbox_policy,
            sandbox_policy_cwd,
            windows_sandbox_level == WindowsSandboxLevel::Elevated,
        )
        .err()
    } else {
        resolve_windows_restricted_token_filesystem_overrides(
            sandbox,
            sandbox_policy,
            file_system_sandbox_policy,
            network_sandbox_policy,
            sandbox_policy_cwd,
            windows_sandbox_level,
        )
        .err()
    }
}

pub fn resolve_windows_restricted_token_filesystem_overrides(
    sandbox: SandboxType,
    sandbox_policy: &SandboxPolicy,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    sandbox_policy_cwd: &AbsolutePathBuf,
    windows_sandbox_level: WindowsSandboxLevel,
) -> std::result::Result<Option<WindowsSandboxFilesystemOverrides>, String> {
    if sandbox != SandboxType::WindowsRestrictedToken
        || windows_sandbox_level == WindowsSandboxLevel::Elevated
    {
        return Ok(None);
    }

    let needs_direct_runtime_enforcement = file_system_sandbox_policy
        .needs_direct_runtime_enforcement(network_sandbox_policy, sandbox_policy_cwd);

    if should_use_windows_restricted_token_sandbox(
        sandbox,
        sandbox_policy,
        file_system_sandbox_policy,
    ) && !needs_direct_runtime_enforcement
    {
        return Ok(None);
    }

    if !should_use_windows_restricted_token_sandbox(
        sandbox,
        sandbox_policy,
        file_system_sandbox_policy,
    ) {
        return Err(format!(
            "windows sandbox backend cannot enforce file_system={:?}, network={network_sandbox_policy:?}, legacy_policy={sandbox_policy:?}; refusing to run unsandboxed",
            file_system_sandbox_policy.kind,
        ));
    }

    // The restricted-token backend can still enforce split write restrictions,
    // but its WRITE_RESTRICTED token does not make capability SID deny-read ACEs
    // participate in read access checks. Read restrictions therefore require the
    // elevated backend, even when the filesystem root remains readable.
    if !windows_policy_has_root_read_access(file_system_sandbox_policy, sandbox_policy_cwd) {
        return Err(
            "windows unelevated restricted-token sandbox cannot enforce split filesystem read restrictions directly; refusing to run unsandboxed"
                .to_string(),
        );
    }

    let additional_deny_read_paths =
        resolve_windows_deny_read_paths(file_system_sandbox_policy, sandbox_policy_cwd)?;
    if !additional_deny_read_paths.is_empty() {
        return Err(
            "windows unelevated restricted-token sandbox cannot enforce deny-read restrictions directly; refusing to run unsandboxed"
                .to_string(),
        );
    }

    let legacy_writable_roots = sandbox_policy.get_writable_roots_with_cwd(sandbox_policy_cwd);
    let split_writable_roots =
        file_system_sandbox_policy.get_writable_roots_with_cwd(sandbox_policy_cwd);
    let legacy_root_paths: BTreeSet<PathBuf> = legacy_writable_roots
        .iter()
        .map(|root| normalize_windows_override_path(root.root.as_path()))
        .collect::<std::result::Result<_, _>>()?;
    let split_root_paths: BTreeSet<PathBuf> = split_writable_roots
        .iter()
        .map(|root| normalize_windows_override_path(root.root.as_path()))
        .collect::<std::result::Result<_, _>>()?;

    if legacy_root_paths != split_root_paths {
        return Err(
            "windows unelevated restricted-token sandbox cannot enforce split writable root sets directly; refusing to run unsandboxed"
                .to_string(),
        );
    }

    for writable_root in &split_writable_roots {
        for read_only_subpath in &writable_root.read_only_subpaths {
            if split_writable_roots.iter().any(|candidate| {
                candidate.root.as_path() != writable_root.root.as_path()
                    && candidate
                        .root
                        .as_path()
                        .starts_with(read_only_subpath.as_path())
            }) {
                return Err(
                    "windows unelevated restricted-token sandbox cannot reopen writable descendants under read-only carveouts directly; refusing to run unsandboxed"
                        .to_string(),
                );
            }
        }
    }

    let mut additional_deny_write_paths = BTreeSet::new();
    for split_root in &split_writable_roots {
        let split_root_path = normalize_windows_override_path(split_root.root.as_path())?;
        let Some(legacy_root) = legacy_writable_roots.iter().find(|candidate| {
            normalize_windows_override_path(candidate.root.as_path())
                .is_ok_and(|candidate_path| candidate_path == split_root_path)
        }) else {
            return Err(
                "windows unelevated restricted-token sandbox cannot enforce split writable root sets directly; refusing to run unsandboxed"
                    .to_string(),
            );
        };

        for read_only_subpath in &split_root.read_only_subpaths {
            if !legacy_root
                .read_only_subpaths
                .iter()
                .any(|candidate| candidate == read_only_subpath)
            {
                additional_deny_write_paths.insert(normalize_windows_override_path(
                    read_only_subpath.as_path(),
                )?);
            }
        }
    }

    if additional_deny_read_paths.is_empty() && additional_deny_write_paths.is_empty() {
        return Ok(None);
    }

    Ok(Some(WindowsSandboxFilesystemOverrides {
        read_roots_override: None,
        read_roots_include_platform_defaults: false,
        write_roots_override: None,
        additional_deny_read_paths,
        additional_deny_write_paths: additional_deny_write_paths
            .into_iter()
            .map(|path| AbsolutePathBuf::from_absolute_path(path).map_err(|err| err.to_string()))
            .collect::<std::result::Result<_, _>>()?,
    }))
}

fn normalize_windows_override_path(path: &Path) -> std::result::Result<PathBuf, String> {
    AbsolutePathBuf::from_absolute_path(normalize_for_native_workdir(path))
        .map(AbsolutePathBuf::into_path_buf)
        .map_err(|err| err.to_string())
}

fn windows_policy_has_root_read_access(
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    cwd: &AbsolutePathBuf,
) -> bool {
    let Some(root) = cwd.as_path().ancestors().last() else {
        return false;
    };
    file_system_sandbox_policy.can_read_path_with_cwd(root, cwd.as_path())
}

pub fn resolve_windows_elevated_filesystem_overrides(
    sandbox: SandboxType,
    sandbox_policy: &SandboxPolicy,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    sandbox_policy_cwd: &AbsolutePathBuf,
    use_windows_elevated_backend: bool,
) -> std::result::Result<Option<WindowsSandboxFilesystemOverrides>, String> {
    if sandbox != SandboxType::WindowsRestrictedToken || !use_windows_elevated_backend {
        return Ok(None);
    }

    if !should_use_windows_restricted_token_sandbox(
        sandbox,
        sandbox_policy,
        file_system_sandbox_policy,
    ) {
        return Err(format!(
            "windows sandbox backend cannot enforce file_system={:?}, network={network_sandbox_policy:?}, legacy_policy={sandbox_policy:?}; refusing to run unsandboxed",
            file_system_sandbox_policy.kind,
        ));
    }

    let additional_deny_read_paths =
        resolve_windows_deny_read_paths(file_system_sandbox_policy, sandbox_policy_cwd)?;

    let split_writable_roots =
        file_system_sandbox_policy.get_writable_roots_with_cwd(sandbox_policy_cwd);
    if has_reopened_writable_descendant(&split_writable_roots) {
        return Err(
            "windows elevated sandbox cannot reopen writable descendants under read-only carveouts directly; refusing to run unsandboxed"
                .to_string(),
        );
    }

    let needs_direct_runtime_enforcement = file_system_sandbox_policy
        .needs_direct_runtime_enforcement(network_sandbox_policy, sandbox_policy_cwd);
    let normalize_path = canonicalize_absolute_path_or_original;
    let legacy_writable_roots = sandbox_policy.get_writable_roots_with_cwd(sandbox_policy_cwd);
    let legacy_root_paths: BTreeSet<PathBuf> = legacy_writable_roots
        .iter()
        .map(|root| normalize_path(root.root.to_path_buf()))
        .collect();
    let split_readable_roots: Vec<PathBuf> = file_system_sandbox_policy
        .get_readable_roots_with_cwd(sandbox_policy_cwd)
        .into_iter()
        .map(codex_utils_absolute_path::AbsolutePathBuf::into_path_buf)
        .map(&normalize_path)
        .collect();
    let split_root_paths: Vec<PathBuf> = split_writable_roots
        .iter()
        .map(|root| normalize_path(root.root.to_path_buf()))
        .collect();
    let split_root_path_set: BTreeSet<PathBuf> = split_root_paths.iter().cloned().collect();

    // `has_full_disk_read_access()` is intentionally false when deny-read
    // entries exist. For Windows setup overrides, the important question is
    // whether the baseline still reads from the filesystem root and only needs
    // additional deny ACLs layered on top.
    let split_has_root_read_access =
        windows_policy_has_root_read_access(file_system_sandbox_policy, sandbox_policy_cwd);
    let read_roots_override = if split_has_root_read_access {
        None
    } else {
        Some(split_readable_roots)
    };

    let write_roots_override = if split_root_path_set == legacy_root_paths {
        None
    } else {
        Some(split_root_paths)
    };

    let additional_deny_write_paths = if needs_direct_runtime_enforcement {
        let mut deny_paths = BTreeSet::new();
        for writable_root in &split_writable_roots {
            let writable_root_path = normalize_path(writable_root.root.to_path_buf());
            let legacy_root = legacy_writable_roots.iter().find(|candidate| {
                normalize_path(candidate.root.to_path_buf()) == writable_root_path
            });
            for read_only_subpath in &writable_root.read_only_subpaths {
                let read_only_subpath_suffix = read_only_subpath
                    .as_path()
                    .strip_prefix(writable_root.root.as_path())
                    .ok();
                let already_denied_by_legacy = legacy_root.is_some_and(|legacy_root| {
                    legacy_root.read_only_subpaths.iter().any(|candidate| {
                        candidate
                            .as_path()
                            .strip_prefix(legacy_root.root.as_path())
                            .ok()
                            == read_only_subpath_suffix
                    })
                });
                if !already_denied_by_legacy {
                    deny_paths.insert(normalize_path(read_only_subpath.to_path_buf()));
                }
            }
        }
        deny_paths
            .into_iter()
            .map(|path| AbsolutePathBuf::from_absolute_path(path).map_err(|err| err.to_string()))
            .collect::<std::result::Result<_, _>>()?
    } else {
        Vec::new()
    };

    if read_roots_override.is_none()
        && write_roots_override.is_none()
        && additional_deny_read_paths.is_empty()
        && additional_deny_write_paths.is_empty()
    {
        return Ok(None);
    }

    Ok(Some(WindowsSandboxFilesystemOverrides {
        read_roots_include_platform_defaults: read_roots_override.is_some()
            && file_system_sandbox_policy.include_platform_defaults(),
        read_roots_override,
        write_roots_override,
        additional_deny_read_paths,
        additional_deny_write_paths,
    }))
}

fn canonicalize_absolute_path_or_original(path: PathBuf) -> PathBuf {
    AbsolutePathBuf::from_absolute_path(&path)
        .and_then(|path| path.canonicalize())
        .map(AbsolutePathBuf::into_path_buf)
        .unwrap_or(path)
}

fn has_reopened_writable_descendant(
    writable_roots: &[codex_protocol::protocol::WritableRoot],
) -> bool {
    writable_roots.iter().any(|writable_root| {
        writable_root
            .read_only_subpaths
            .iter()
            .any(|read_only_subpath| {
                writable_roots.iter().any(|candidate| {
                    candidate.root.as_path() != writable_root.root.as_path()
                        && candidate
                            .root
                            .as_path()
                            .starts_with(read_only_subpath.as_path())
                })
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_protocol::permissions::FileSystemAccessMode;
    use codex_protocol::permissions::FileSystemPath;
    use codex_protocol::permissions::FileSystemSandboxEntry;
    use codex_protocol::permissions::FileSystemSpecialPath;
    use codex_protocol::protocol::NetworkAccess;
    use pretty_assertions::assert_eq;

    fn abs(path: impl AsRef<Path>) -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path(path.as_ref()).expect("absolute path")
    }

    fn canonical_abs(path: impl AsRef<Path>) -> AbsolutePathBuf {
        abs(dunce::canonicalize(path).expect("canonical path"))
    }

    #[test]
    fn windows_restricted_token_skips_external_sandbox_policies() {
        let policy = SandboxPolicy::ExternalSandbox {
            network_access: NetworkAccess::Restricted,
        };
        let file_system_policy = FileSystemSandboxPolicy::from(&policy);

        assert_eq!(
            should_use_windows_restricted_token_sandbox(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
            ),
            false
        );
    }

    #[test]
    fn windows_restricted_token_runs_for_legacy_restricted_policies() {
        let policy = SandboxPolicy::new_read_only_policy();
        let file_system_policy = FileSystemSandboxPolicy::from(&policy);

        assert_eq!(
            should_use_windows_restricted_token_sandbox(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
            ),
            true
        );
    }

    #[test]
    fn windows_proxy_enforcement_uses_elevated_backend() {
        assert!(!windows_sandbox_uses_elevated_backend(
            WindowsSandboxLevel::RestrictedToken,
            /*proxy_enforced*/ false,
        ));
        assert!(windows_sandbox_uses_elevated_backend(
            WindowsSandboxLevel::RestrictedToken,
            /*proxy_enforced*/ true,
        ));
        assert!(windows_sandbox_uses_elevated_backend(
            WindowsSandboxLevel::Elevated,
            /*proxy_enforced*/ false,
        ));
    }

    #[test]
    fn windows_restricted_token_rejects_network_only_restrictions() {
        let policy = SandboxPolicy::ExternalSandbox {
            network_access: NetworkAccess::Restricted,
        };
        let file_system_policy = FileSystemSandboxPolicy::unrestricted();
        let sandbox_policy_cwd = AbsolutePathBuf::current_dir().expect("cwd");

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &sandbox_policy_cwd,
                WindowsSandboxLevel::RestrictedToken,
            ),
            Some(
                "windows sandbox backend cannot enforce file_system=Unrestricted, network=Restricted, legacy_policy=ExternalSandbox { network_access: Restricted }; refusing to run unsandboxed".to_string()
            )
        );
    }

    #[test]
    fn windows_restricted_token_allows_legacy_restricted_policies() {
        let policy = SandboxPolicy::new_read_only_policy();
        let file_system_policy = FileSystemSandboxPolicy::from(&policy);
        let sandbox_policy_cwd = AbsolutePathBuf::current_dir().expect("cwd");

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &sandbox_policy_cwd,
                WindowsSandboxLevel::RestrictedToken,
            ),
            None
        );
    }

    #[test]
    fn windows_restricted_token_allows_legacy_workspace_write_policies() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::from(&policy);
        let sandbox_policy_cwd = AbsolutePathBuf::current_dir().expect("cwd");

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &sandbox_policy_cwd,
                WindowsSandboxLevel::RestrictedToken,
            ),
            None
        );
    }

    #[test]
    fn windows_elevated_allows_split_restricted_read_policies() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = abs(temp_dir.path().join("docs"));
        std::fs::create_dir_all(docs.as_path()).expect("create docs");
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let file_system_policy =
            FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs },
                access: FileSystemAccessMode::Read,
            }]);

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                WindowsSandboxLevel::Elevated,
            ),
            None
        );
    }

    #[test]
    fn windows_restricted_token_rejects_split_only_filesystem_policies() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = temp_dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("create docs");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&docs) },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                WindowsSandboxLevel::RestrictedToken,
            ),
            Some(
                "windows unelevated restricted-token sandbox cannot enforce split filesystem read restrictions directly; refusing to run unsandboxed"
                    .to_string()
            )
        );
    }

    #[test]
    fn windows_restricted_token_rejects_root_write_read_only_carveouts() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = temp_dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("create docs");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&docs) },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                WindowsSandboxLevel::RestrictedToken,
            ),
            Some(
                "windows unelevated restricted-token sandbox cannot enforce split writable root sets directly; refusing to run unsandboxed"
                    .to_string()
            )
        );
    }

    #[test]
    fn windows_restricted_token_supports_full_read_split_write_read_carveouts() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let cwd = canonical_abs(temp_dir.path());
        let docs = cwd.join("docs");
        std::fs::create_dir_all(docs.as_path()).expect("create docs");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs.clone() },
                access: FileSystemAccessMode::Read,
            },
        ]);

        // The legacy workspace-write root already protects top-level `.codex`, so
        // the restricted-token overlay only needs the extra read-only docs carveout.
        let expected_deny_write_paths = vec![docs];

        assert_eq!(
            resolve_windows_restricted_token_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &cwd,
                WindowsSandboxLevel::RestrictedToken,
            ),
            Ok(Some(WindowsSandboxFilesystemOverrides {
                read_roots_override: None,
                read_roots_include_platform_defaults: false,
                write_roots_override: None,
                additional_deny_read_paths: vec![],
                additional_deny_write_paths: expected_deny_write_paths,
            }))
        );
    }

    #[test]
    fn windows_restricted_token_rejects_unreadable_split_carveouts() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let cwd = canonical_abs(temp_dir.path());
        let blocked = cwd.join("blocked");
        std::fs::create_dir_all(blocked.as_path()).expect("create blocked");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: blocked },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            resolve_windows_restricted_token_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &cwd,
                WindowsSandboxLevel::RestrictedToken,
            ),
            Err(
                "windows unelevated restricted-token sandbox cannot enforce deny-read restrictions directly; refusing to run unsandboxed"
                    .to_string()
            )
        );
    }

    #[test]
    fn windows_elevated_supports_split_restricted_read_roots() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = temp_dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("create docs");
        let expected_docs = dunce::canonicalize(&docs).expect("canonical docs");
        let policy = SandboxPolicy::ReadOnly {
            network_access: false,
        };
        let file_system_policy =
            FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&docs) },
                access: FileSystemAccessMode::Read,
            }]);

        assert_eq!(
            resolve_windows_elevated_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                /*use_windows_elevated_backend*/ true,
            ),
            Ok(Some(WindowsSandboxFilesystemOverrides {
                read_roots_override: Some(vec![expected_docs]),
                read_roots_include_platform_defaults: false,
                write_roots_override: None,
                additional_deny_read_paths: vec![],
                additional_deny_write_paths: vec![],
            }))
        );
    }

    #[test]
    fn windows_elevated_supports_split_write_read_carveouts() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = temp_dir.path().join("docs");
        std::fs::create_dir_all(&docs).expect("create docs");
        let expected_docs = dunce::canonicalize(&docs).expect("canonical docs");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&docs) },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert_eq!(
            resolve_windows_elevated_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                /*use_windows_elevated_backend*/ true,
            ),
            Ok(Some(WindowsSandboxFilesystemOverrides {
                read_roots_override: None,
                read_roots_include_platform_defaults: false,
                write_roots_override: None,
                additional_deny_read_paths: vec![],
                additional_deny_write_paths: vec![abs(expected_docs)],
            }))
        );
    }

    #[test]
    fn windows_elevated_supports_unreadable_split_carveouts() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let blocked = temp_dir.path().join("blocked");
        std::fs::create_dir_all(&blocked).expect("create blocked");
        let expected_blocked = dunce::canonicalize(&blocked).expect("canonical blocked");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: abs(&blocked),
                },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            resolve_windows_elevated_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                /*use_windows_elevated_backend*/ true,
            ),
            Ok(Some(WindowsSandboxFilesystemOverrides {
                read_roots_override: None,
                read_roots_include_platform_defaults: false,
                write_roots_override: None,
                additional_deny_read_paths: vec![abs(expected_blocked.clone())],
                additional_deny_write_paths: vec![abs(expected_blocked)],
            }))
        );
    }

    #[test]
    fn windows_elevated_supports_unreadable_globs() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let secret = temp_dir.path().join("app").join(".env");
        std::fs::create_dir_all(secret.parent().expect("parent")).expect("create parent");
        std::fs::write(&secret, "secret").expect("write secret");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: "**/*.env".to_string(),
                },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            resolve_windows_elevated_filesystem_overrides(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                /*use_windows_elevated_backend*/ true,
            ),
            Ok(Some(WindowsSandboxFilesystemOverrides {
                read_roots_override: None,
                read_roots_include_platform_defaults: false,
                write_roots_override: None,
                additional_deny_read_paths: vec![abs(secret)],
                additional_deny_write_paths: vec![],
            }))
        );
    }

    #[test]
    fn windows_elevated_rejects_reopened_writable_descendants() {
        let temp_dir = tempfile::TempDir::new().expect("tempdir");
        let docs = temp_dir.path().join("docs");
        let nested = docs.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let file_system_policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&docs) },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: abs(&nested) },
                access: FileSystemAccessMode::Write,
            },
        ]);

        assert_eq!(
            unsupported_windows_restricted_token_sandbox_reason(
                SandboxType::WindowsRestrictedToken,
                &policy,
                &file_system_policy,
                NetworkSandboxPolicy::Restricted,
                &abs(temp_dir.path()),
                WindowsSandboxLevel::Elevated,
            ),
            Some(
                "windows elevated sandbox cannot reopen writable descendants under read-only carveouts directly; refusing to run unsandboxed"
                    .to_string()
            )
        );
    }
}
