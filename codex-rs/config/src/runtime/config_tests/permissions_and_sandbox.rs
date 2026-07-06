use super::*;

#[tokio::test]
async fn default_permissions_profile_populates_runtime_sandbox_policy() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::create_dir_all(cwd.path().join("docs"))?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    let cfg = ConfigToml {
        default_permissions: Some("workspace".to_string()),
        permissions: Some(PermissionsToml {
            entries: BTreeMap::from([(
                "workspace".to_string(),
                PermissionProfileToml {
                    workspace_roots: None,
                    filesystem: Some(FilesystemPermissionsToml {
                        glob_scan_max_depth: None,
                        entries: BTreeMap::from([
                            (
                                ":minimal".to_string(),
                                FilesystemPermissionToml::Access(FileSystemAccessMode::Read),
                            ),
                            (
                                ":workspace_roots".to_string(),
                                FilesystemPermissionToml::Scoped(BTreeMap::from([
                                    (".".to_string(), FileSystemAccessMode::Write),
                                    ("docs".to_string(), FileSystemAccessMode::Read),
                                ])),
                            ),
                        ]),
                    }),
                    network: None,
                },
            )]),
        }),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let cwd_root = cwd.path().abs();
    let memories_root = codex_home.path().join("memories").abs();
    assert_eq!(
        config.permissions.file_system_sandbox_policy(),
        FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Minimal,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: cwd_root.clone(),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: cwd_root.join("docs"),
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: memories_root.clone(),
                },
                access: FileSystemAccessMode::Write,
            },
        ]),
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![memories_root],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
    assert!(
        !config
            .permissions
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(&cwd.path().join(".git"), cwd.path())
    );
    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Restricted
    );
    assert_eq!(
        config
            .permissions
            .active_permission_profile()
            .as_ref()
            .map(|active| active.id.as_str()),
        Some("workspace")
    );
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_populates_runtime_permissions() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let permission_profile = PermissionProfile::Disabled;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            permission_profile: Some(permission_profile.clone()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.permissions.effective_permission_profile(),
        permission_profile
    );
    assert_eq!(config.permissions.active_permission_profile(), None);
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::DangerFullAccess
    );
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_preserves_managed_unrestricted_filesystem()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let permission_profile = PermissionProfile::Managed {
        file_system: ManagedFileSystemPermissions::Unrestricted,
        network: NetworkSandboxPolicy::Restricted,
    };

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            permission_profile: Some(permission_profile.clone()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.permissions.effective_permission_profile(),
        permission_profile
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::ExternalSandbox {
            network_access: NetworkAccess::Restricted,
        }
    );
    Ok(())
}

#[tokio::test]
async fn managed_unrestricted_permission_profile_still_enables_network_requirements()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let permission_profile = PermissionProfile::Managed {
        file_system: ManagedFileSystemPermissions::Unrestricted,
        network: NetworkSandboxPolicy::Enabled,
    };

    let mut config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            permission_profile: Some(permission_profile),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::DangerFullAccess,
        "the legacy projection is intentionally lossy for managed unrestricted profiles"
    );

    let layers = config
        .config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .cloned()
        .collect();
    let mut requirements = config.config_layer_stack.requirements().clone();
    requirements.network = Some(Sourced::new(
        config_service::NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        },
        RequirementSource::CloudRequirements,
    ));
    let mut requirements_toml = config.config_layer_stack.requirements_toml().clone();
    requirements_toml.network = Some(config_service::NetworkRequirementsToml {
        enabled: Some(true),
        ..Default::default()
    });
    config.config_layer_stack = ConfigLayerStack::new(layers, requirements, requirements_toml)
        .expect("config layer stack with network requirements");

    assert!(config.managed_network_requirements_enabled());
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_applies_runtime_roots_to_legacy_projection()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &FileSystemSandboxPolicy::restricted(vec![
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
        ]),
        NetworkSandboxPolicy::Restricted,
    );

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            permission_profile: Some(permission_profile),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let memories_root = codex_home.path().join("memories").abs();
    assert!(
        config
            .permissions
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(memories_root.as_path(), cwd.path())
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![memories_root],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_preserves_configured_network_policy_without_starting_proxy()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let permission_profile = PermissionProfile::Disabled;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":minimal".to_string(),
                                FilesystemPermissionToml::Access(FileSystemAccessMode::Read),
                            )]),
                        }),
                        network: Some(NetworkToml {
                            enabled: Some(true),
                            proxy_url: Some("http://127.0.0.1:43128".to_string()),
                            enable_socks5: Some(false),
                            allow_upstream_proxy: Some(false),
                            domains: Some(NetworkDomainPermissionsToml {
                                entries: BTreeMap::from([(
                                    "openai.com".to_string(),
                                    NetworkDomainPermissionToml::Allow,
                                )]),
                            }),
                            ..Default::default()
                        }),
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            permission_profile: Some(permission_profile.clone()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;
    assert!(
        config.permissions.network.is_none(),
        "profile network.enabled should not start the managed network proxy"
    );
    assert_eq!(
        config.permissions.effective_permission_profile(),
        permission_profile
    );
    Ok(())
}

#[tokio::test]
async fn workspace_root_glob_none_compiles_to_filesystem_pattern_entry() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = TempDir::new()?;
    tokio::fs::write(cwd.path().join(".git"), "gitdir: nowhere").await?;
    tokio::fs::write(extra_root.path().join(".git"), "gitdir: nowhere").await?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: Some(2),
                            entries: BTreeMap::from([(
                                ":workspace_roots".to_string(),
                                FilesystemPermissionToml::Scoped(BTreeMap::from([
                                    (".".to_string(), FileSystemAccessMode::Write),
                                    ("**/*.env".to_string(), FileSystemAccessMode::None),
                                ])),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            additional_writable_roots: vec![extra_root.path().to_path_buf()],
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config
            .permissions
            .file_system_sandbox_policy()
            .glob_scan_max_depth,
        Some(2)
    );
    for root in [cwd.path(), extra_root.path()] {
        let expected_pattern = AbsolutePathBuf::resolve_path_against_base("**/*.env", root)
            .to_string_lossy()
            .into_owned();
        assert!(
            config
                .permissions
                .file_system_sandbox_policy()
                .entries
                .contains(&FileSystemSandboxEntry {
                    path: FileSystemPath::GlobPattern {
                        pattern: expected_pattern,
                    },
                    access: FileSystemAccessMode::None,
                })
        );
    }
    assert!(
        !config
            .permissions
            .file_system_sandbox_policy()
            .entries
            .iter()
            .any(|entry| matches!(
                &entry.path,
                FileSystemPath::Special {
                    value: FileSystemSpecialPath::ProjectRoots { subpath: Some(subpath) },
                } if subpath == std::path::Path::new("**/*.env")
            )),
        "glob should compile to a filesystem pattern entry, not a literal filesystem entry"
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_require_default_permissions() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":minimal".to_string(),
                                FilesystemPermissionToml::Access(FileSystemAccessMode::Read),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("missing default_permissions should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "config defines `[permissions]` profiles but does not set `default_permissions`"
    );
    Ok(())
}

#[tokio::test]
async fn default_permissions_can_select_builtin_profile_without_permissions_table()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert_eq!(
        config
            .permissions
            .active_permission_profile()
            .as_ref()
            .map(|active| active.id.as_str()),
        Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE)
    );
    assert!(
        policy.can_write_path_with_cwd(cwd.path(), cwd.path()),
        "expected :workspace to allow writing the project root, policy: {policy:?}"
    );
    assert!(
        !policy.can_write_path_with_cwd(&cwd.path().join(".git"), cwd.path()),
        "expected :workspace to protect project metadata, policy: {policy:?}"
    );
    Ok(())
}

#[tokio::test]
async fn default_permissions_read_only_keeps_add_dir_read_only() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = TempDir::new()?;
    let extra_root = extra_root.path().abs();

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_READ_ONLY.to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            additional_writable_roots: vec![extra_root.to_path_buf()],
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert!(
        !policy.can_write_path_with_cwd(extra_root.as_path(), cwd.path()),
        "expected :read-only to stay read-only for runtime workspace roots, policy: {policy:?}"
    );
    assert_eq!(
        config.permissions.active_permission_profile(),
        Some(ActivePermissionProfile::new(
            BUILT_IN_PERMISSION_PROFILE_READ_ONLY,
        ))
    );
    Ok(())
}

#[tokio::test]
async fn workspace_profile_applies_rules_to_runtime_and_profile_workspace_roots()
-> std::io::Result<()> {
    let temp_dir = TempDir::new()?;
    let codex_home = temp_dir.path().join("codex-home");
    let cwd = temp_dir.path().join("frontend");
    let runtime_root = temp_dir.path().join("backend");
    let profile_root = temp_dir.path().join("shared");
    for root in [&cwd, &runtime_root, &profile_root] {
        std::fs::create_dir_all(root.join(".git"))?;
        std::fs::create_dir_all(root.join(".codex"))?;
    }

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("dev".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "dev".to_string(),
                    PermissionProfileToml {
                        workspace_roots: Some(WorkspaceRootsToml {
                            entries: BTreeMap::from([(
                                profile_root.to_string_lossy().into_owned(),
                                true,
                            )]),
                        }),
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":workspace_roots".to_string(),
                                FilesystemPermissionToml::Scoped(BTreeMap::from([
                                    (".".to_string(), FileSystemAccessMode::Write),
                                    (".git".to_string(), FileSystemAccessMode::Read),
                                    (".codex".to_string(), FileSystemAccessMode::Read),
                                ])),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.clone()),
            additional_writable_roots: vec![runtime_root.clone()],
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let cwd_abs = cwd.abs();
    let runtime_root_abs = runtime_root.abs();
    let profile_root_abs = profile_root.abs();
    assert_eq!(
        config.workspace_roots,
        vec![cwd_abs.clone(), runtime_root_abs.clone()]
    );
    assert_eq!(
        config.permissions.workspace_roots(),
        &[cwd_abs.clone(), runtime_root_abs.clone()]
    );
    assert_eq!(
        config.effective_workspace_roots(),
        vec![
            cwd_abs.clone(),
            runtime_root_abs.clone(),
            profile_root_abs.clone()
        ]
    );

    let policy = config.permissions.file_system_sandbox_policy();
    for root in [cwd_abs, runtime_root_abs, profile_root_abs.clone()] {
        assert!(
            policy.can_write_path_with_cwd(root.as_path(), cwd.as_path()),
            "expected workspace root to be writable, policy: {policy:?}"
        );
        assert!(
            !policy.can_write_path_with_cwd(&root.join(".git"), cwd.as_path()),
            "expected .git carveout under {root:?}, policy: {policy:?}"
        );
        assert!(
            !policy.can_write_path_with_cwd(&root.join(".codex"), cwd.as_path()),
            "expected .codex carveout under {root:?}, policy: {policy:?}"
        );
    }
    assert_eq!(
        config.permissions.profile_workspace_roots(),
        std::slice::from_ref(&profile_root_abs)
    );
    assert_eq!(
        config.permissions.active_permission_profile(),
        Some(ActivePermissionProfile::new("dev"))
    );
    Ok(())
}

#[tokio::test]
async fn explicit_builtin_workspace_profile_ignores_legacy_workspace_write_settings()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = TempDir::new()?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.to_string()),
            sandbox_workspace_write: Some(SandboxWorkspaceWrite {
                writable_roots: vec![extra_root.path().abs()],
                network_access: true,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Restricted
    );
    assert!(
        !policy.entries.iter().any(|entry| matches!(
            &entry.path,
            FileSystemPath::Path { path } if path.as_path() == extra_root.path()
        )),
        "explicit :workspace should not inherit sandbox_workspace_write roots as concrete grants, \
         policy: {policy:?}"
    );
    Ok(())
}

#[tokio::test]
async fn empty_config_defaults_to_builtin_profile_for_trusted_project() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let project_key = cwd.path().to_string_lossy().to_string();

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            projects: Some(HashMap::from([(
                project_key,
                ProjectConfig {
                    trust_level: Some(TrustLevel::Trusted),
                },
            )])),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert_eq!(
        config
            .permissions
            .active_permission_profile()
            .as_ref()
            .map(|active| active.id.as_str()),
        Some(if cfg!(target_os = "windows") {
            BUILT_IN_PERMISSION_PROFILE_READ_ONLY
        } else {
            BUILT_IN_PERMISSION_PROFILE_WORKSPACE
        })
    );
    if cfg!(target_os = "windows") {
        assert!(
            !policy.can_write_path_with_cwd(cwd.path(), cwd.path()),
            "expected trusted project fallback to stay read-only without Windows sandbox support, policy: {policy:?}"
        );
    } else {
        assert!(
            policy.can_write_path_with_cwd(cwd.path(), cwd.path()),
            "expected trusted project fallback to use :workspace, policy: {policy:?}"
        );
        assert!(
            !policy.can_write_path_with_cwd(&cwd.path().join(".codex"), cwd.path()),
            "expected :workspace metadata carveouts, policy: {policy:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn implicit_builtin_workspace_profile_preserves_sandbox_workspace_write_settings()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = TempDir::new()?;
    let extra_root = extra_root.path().abs();
    let project_key = cwd.path().to_string_lossy().to_string();

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            projects: Some(HashMap::from([(
                project_key,
                ProjectConfig {
                    trust_level: Some(TrustLevel::Trusted),
                },
            )])),
            sandbox_workspace_write: Some(SandboxWorkspaceWrite {
                writable_roots: vec![extra_root.clone()],
                network_access: true,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: false,
            }),
            windows: Some(WindowsToml {
                sandbox: Some(WindowsSandboxModeToml::Elevated),
                sandbox_private_desktop: None,
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert!(
        policy.can_write_path_with_cwd(extra_root.as_path(), cwd.path()),
        "expected implicit :workspace to preserve sandbox_workspace_write.writable_roots, policy: {policy:?}"
    );
    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    assert_eq!(
        config.permissions.active_permission_profile(),
        None,
        "implicit :workspace cannot be faithfully re-selected when it includes \
         legacy sandbox_workspace_write settings"
    );
    match config.legacy_sandbox_policy() {
        SandboxPolicy::WorkspaceWrite {
            writable_roots,
            network_access,
            exclude_tmpdir_env_var,
            exclude_slash_tmp,
        } => {
            assert!(writable_roots.contains(&extra_root));
            assert!(network_access);
            assert!(exclude_tmpdir_env_var);
            assert!(!exclude_slash_tmp);
        }
        sandbox_policy => panic!("expected workspace-write projection, got {sandbox_policy:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn implicit_builtin_workspace_profile_preserves_add_dir_metadata_carveouts()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = TempDir::new()?;
    for subpath in [".git", ".agents", ".codex"] {
        std::fs::create_dir_all(extra_root.path().join(subpath))?;
    }
    let project_key = cwd.path().to_string_lossy().to_string();

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            projects: Some(HashMap::from([(
                project_key,
                ProjectConfig {
                    trust_level: Some(TrustLevel::Trusted),
                },
            )])),
            windows: Some(WindowsToml {
                sandbox: Some(WindowsSandboxModeToml::Elevated),
                sandbox_private_desktop: None,
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            additional_writable_roots: vec![extra_root.path().to_path_buf()],
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    let extra_root = extra_root.path().abs();
    assert!(
        policy.can_write_path_with_cwd(extra_root.as_path(), cwd.path()),
        "expected implicit :workspace to preserve additional writable roots, policy: {policy:?}"
    );
    for subpath in [".git", ".agents", ".codex"] {
        assert!(
            !policy.can_write_path_with_cwd(&extra_root.join(subpath), cwd.path()),
            "expected implicit :workspace to preserve legacy metadata carveout for {subpath}, \
             policy: {policy:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn empty_config_defaults_to_builtin_read_only_without_trust_decision() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let policy = config.permissions.file_system_sandbox_policy();
    assert!(
        policy.can_read_path_with_cwd(cwd.path(), cwd.path()),
        "expected :read-only to allow reads, policy: {policy:?}"
    );
    assert!(
        !policy.can_write_path_with_cwd(cwd.path(), cwd.path()),
        "expected :read-only to deny writes, policy: {policy:?}"
    );
    Ok(())
}

#[tokio::test]
async fn default_permissions_can_select_builtin_full_access_profile() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS.to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.permissions.effective_permission_profile(),
        PermissionProfile::Disabled
    );
    assert_eq!(
        config
            .permissions
            .active_permission_profile()
            .as_ref()
            .map(|active| active.id.as_str()),
        Some(BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS)
    );
    Ok(())
}

#[tokio::test]
async fn legacy_danger_no_sandbox_is_rejected() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(":danger-no-sandbox".to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("legacy full-access alias should be rejected");

    assert_eq!(
        err.to_string(),
        "default_permissions refers to unknown built-in profile `:danger-no-sandbox`"
    );
    Ok(())
}

#[tokio::test]
async fn user_defined_permission_profile_names_cannot_use_builtin_prefix() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(":custom".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    ":custom".to_string(),
                    PermissionProfileToml::default(),
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("reserved profile name should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "permissions profile `:custom` uses a reserved built-in profile prefix"
    );
    Ok(())
}

#[tokio::test]
async fn unknown_builtin_permission_profile_name_is_rejected() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some(":unknown".to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("unknown built-in profile name should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "default_permissions refers to unknown built-in profile `:unknown`"
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_allow_direct_write_roots_outside_workspace_root()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;
    let external_write_dir = TempDir::new()?;
    let external_write_path =
        AbsolutePathBuf::from_absolute_path(std::fs::canonicalize(external_write_dir.path())?)?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                external_write_path.to_string_lossy().into_owned(),
                                FilesystemPermissionToml::Access(FileSystemAccessMode::Write),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    let memories_root = AbsolutePathBuf::from_absolute_path(std::fs::canonicalize(
        codex_home.path().join("memories"),
    )?)?;
    assert!(
        config
            .permissions
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(external_write_path.as_path(), cwd.path())
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![external_write_path, memories_root],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_reject_nested_entries_for_non_workspace_roots() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":minimal".to_string(),
                                FilesystemPermissionToml::Scoped(BTreeMap::from([(
                                    "docs".to_string(),
                                    FileSystemAccessMode::Read,
                                )])),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("nested entries outside :workspace_roots should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "filesystem path `:minimal` does not support nested entries"
    );
    Ok(())
}

async fn load_workspace_permission_profile(
    profile: PermissionProfileToml,
) -> std::io::Result<Config> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([("workspace".to_string(), profile)]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
}

#[tokio::test]
async fn permissions_profiles_allow_unknown_special_paths() -> std::io::Result<()> {
    let config = load_workspace_permission_profile(PermissionProfileToml {
        workspace_roots: None,
        filesystem: Some(FilesystemPermissionsToml {
            glob_scan_max_depth: None,
            entries: BTreeMap::from([(
                ":future_special_path".to_string(),
                FilesystemPermissionToml::Access(FileSystemAccessMode::Read),
            )]),
        }),
        network: None,
    })
    .await?;

    assert_eq!(
        config.permissions.file_system_sandbox_policy(),
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::unknown(
                    ":future_special_path",
                    /*subpath*/ None
                ),
            },
            access: FileSystemAccessMode::Read,
        }]),
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::ReadOnly {
            network_access: false,
        }
    );
    assert!(
        config.startup_warnings.iter().any(|warning| warning.contains(
            "Configured filesystem path `:future_special_path` is not recognized by this version of Codex and will be ignored."
        )),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_allow_unknown_special_paths_with_nested_entries()
-> std::io::Result<()> {
    let config = load_workspace_permission_profile(PermissionProfileToml {
        workspace_roots: None,
        filesystem: Some(FilesystemPermissionsToml {
            glob_scan_max_depth: None,
            entries: BTreeMap::from([(
                ":future_special_path".to_string(),
                FilesystemPermissionToml::Scoped(BTreeMap::from([(
                    "docs".to_string(),
                    FileSystemAccessMode::Read,
                )])),
            )]),
        }),
        network: None,
    })
    .await?;

    assert_eq!(
        config.permissions.file_system_sandbox_policy(),
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::unknown(":future_special_path", Some("docs".into())),
            },
            access: FileSystemAccessMode::Read,
        }]),
    );
    assert!(
        config.startup_warnings.iter().any(|warning| warning.contains(
            "Configured filesystem path `:future_special_path` with nested entry `docs` is not recognized by this version of Codex and will be ignored."
        )),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_allow_missing_filesystem_with_warning() -> std::io::Result<()> {
    let config = load_workspace_permission_profile(PermissionProfileToml {
        workspace_roots: None,
        filesystem: None,
        network: None,
    })
    .await?;

    assert_eq!(
        config.permissions.file_system_sandbox_policy(),
        FileSystemSandboxPolicy::restricted(Vec::new())
    );
    assert_eq!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::ReadOnly {
            network_access: false,
        }
    );
    assert!(
        config.startup_warnings.iter().any(|warning| warning.contains(
            "Permissions profile `workspace` does not define any recognized filesystem entries for this version of Codex."
        )),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_allow_empty_filesystem_with_warning() -> std::io::Result<()> {
    let config = load_workspace_permission_profile(PermissionProfileToml {
        workspace_roots: None,
        filesystem: Some(FilesystemPermissionsToml {
            glob_scan_max_depth: None,
            entries: BTreeMap::new(),
        }),
        network: None,
    })
    .await?;

    assert_eq!(
        config.permissions.file_system_sandbox_policy(),
        FileSystemSandboxPolicy::restricted(Vec::new())
    );
    assert!(
        config.startup_warnings.iter().any(|warning| warning.contains(
            "Permissions profile `workspace` does not define any recognized filesystem entries for this version of Codex."
        )),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_reject_workspace_root_parent_traversal() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    let err = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":workspace_roots".to_string(),
                                FilesystemPermissionToml::Scoped(BTreeMap::from([(
                                    "../sibling".to_string(),
                                    FileSystemAccessMode::Read,
                                )])),
                            )]),
                        }),
                        network: None,
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await
    .expect_err("parent traversal should be rejected for project root subpaths");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "filesystem subpath `../sibling` must be a descendant path without `.` or `..` components"
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_allow_network_enablement() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            default_permissions: Some("workspace".to_string()),
            permissions: Some(PermissionsToml {
                entries: BTreeMap::from([(
                    "workspace".to_string(),
                    PermissionProfileToml {
                        workspace_roots: None,
                        filesystem: Some(FilesystemPermissionsToml {
                            glob_scan_max_depth: None,
                            entries: BTreeMap::from([(
                                ":minimal".to_string(),
                                FilesystemPermissionToml::Access(FileSystemAccessMode::Read),
                            )]),
                        }),
                        network: Some(NetworkToml {
                            enabled: Some(true),
                            ..Default::default()
                        }),
                    },
                )]),
            }),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert!(
        config.permissions.network_sandbox_policy().is_enabled(),
        "expected network sandbox policy to be enabled",
    );
    assert!(config.legacy_sandbox_policy().has_full_network_access());
    Ok(())
}

#[test]
fn tui_theme_deserializes_from_toml() {
    let cfg = r#"
[tui]
theme = "dracula"
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().and_then(|t| t.theme.as_deref()),
        Some("dracula"),
    );
}

#[test]
fn tui_theme_defaults_to_none() {
    let cfg = r#"
[tui]
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(parsed.tui.as_ref().and_then(|t| t.theme.as_deref()), None);
}

#[test]
fn tui_session_picker_view_deserializes_from_toml() {
    let cfg = r#"
[tui]
session_picker_view = "dense"
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().and_then(|t| t.session_picker_view),
        Some(SessionPickerViewMode::Dense),
    );
}

#[test]
fn tui_pet_deserializes_from_toml() {
    let cfg = r#"
[tui]
pet = "chefito"
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().and_then(|t| t.pet.as_deref()),
        Some("chefito"),
    );
}

#[test]
fn tui_session_picker_view_defaults_to_none() {
    let cfg = r#"
[tui]
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().and_then(|t| t.session_picker_view),
        None,
    );
}

#[test]
fn tui_pet_defaults_to_none() {
    let cfg = r#"
[tui]
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(parsed.tui.as_ref().and_then(|t| t.pet.as_deref()), None);
}

#[test]
fn tui_pet_anchor_deserializes_from_toml() {
    let cfg = r#"
[tui]
pet_anchor = "screen-bottom"
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().map(|t| t.pet_anchor),
        Some(TuiPetAnchor::ScreenBottom),
    );
}

#[test]
fn tui_pet_anchor_defaults_to_composer() {
    let cfg = r#"
[tui]
"#;
    let parsed = toml::from_str::<ConfigToml>(cfg).expect("TOML deserialization should succeed");
    assert_eq!(
        parsed.tui.as_ref().map(|t| t.pet_anchor),
        Some(TuiPetAnchor::Composer),
    );
}

#[test]
fn tui_pet_anchor_rejects_unknown_value() {
    let cfg = r#"
[tui]
pet_anchor = "bottom"
"#;
    let err = toml::from_str::<ConfigToml>(cfg).expect_err("reject unknown pet anchor");
    let err = err.to_string();
    assert!(
        err.contains("unknown variant `bottom`")
            && err.contains("composer")
            && err.contains("screen-bottom"),
        "unexpected error: {err}"
    );
}

#[test]
fn tui_config_missing_notifications_field_defaults_to_enabled() {
    let cfg = r#"
[tui]
"#;

    let parsed =
        toml::from_str::<ConfigToml>(cfg).expect("TUI config without notifications should succeed");
    let tui = parsed.tui.expect("config should include tui section");

    assert_eq!(
        tui,
        Tui {
            notification_settings: TuiNotificationSettings::default(),
            animations: true,
            show_tooltips: true,
            vim_mode_default: false,
            raw_output_mode: false,
            alternate_screen: AltScreenMode::Auto,
            status_line: None,
            status_line_use_colors: true,
            terminal_title: None,
            theme: None,
            pet: None,
            pet_anchor: TuiPetAnchor::Composer,
            session_picker_view: None,
            keymap: TuiKeymap::default(),
            model_availability_nux: ModelAvailabilityNuxConfig::default(),
            terminal_resize_reflow_max_rows: None,
        }
    );
}

#[tokio::test]
async fn runtime_config_resolves_terminal_resize_reflow_defaults_and_overrides() {
    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load default config");

    assert_eq!(
        cfg.terminal_resize_reflow,
        TerminalResizeReflowConfig::default()
    );
    assert_eq!(
        cfg.terminal_resize_reflow.max_rows,
        TerminalResizeReflowMaxRows::Auto
    );

    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml {
            tui: Some(Tui {
                terminal_resize_reflow_max_rows: Some(9000),
                ..Default::default()
            }),
            ..Default::default()
        },
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load overridden config");

    assert_eq!(
        cfg.terminal_resize_reflow.max_rows,
        TerminalResizeReflowMaxRows::Limit(9000)
    );

    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml {
            tui: Some(Tui {
                terminal_resize_reflow_max_rows: Some(0),
                ..Default::default()
            }),
            ..Default::default()
        },
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load config with disabled resize reflow limits");

    assert_eq!(
        cfg.terminal_resize_reflow.max_rows,
        TerminalResizeReflowMaxRows::Disabled
    );
}
