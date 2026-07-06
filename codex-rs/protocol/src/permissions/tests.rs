    use super::*;
    use pretty_assertions::assert_eq;
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[cfg(unix)]
    const SYMLINKED_TMPDIR_TEST_ENV: &str = "CODEX_PROTOCOL_TEST_SYMLINKED_TMPDIR";

    #[cfg(unix)]
    fn symlink_dir(original: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(original, link)
    }

    #[test]
    fn unknown_special_paths_are_ignored_by_legacy_bridge() -> std::io::Result<()> {
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::unknown(
                        ":future_special_path",
                        /*subpath*/ None,
                    ),
                },
                access: FileSystemAccessMode::Write,
            },
        ]);

        let sandbox_policy = policy.to_legacy_sandbox_policy(
            NetworkSandboxPolicy::Restricted,
            Path::new("/tmp/workspace"),
        )?;

        assert_eq!(
            sandbox_policy,
            SandboxPolicy::ReadOnly {
                network_access: false,
            }
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_proactively_protect_missing_dot_codex() {
        let cwd = TempDir::new().expect("tempdir");
        let expected_root = AbsolutePathBuf::from_absolute_path(
            cwd.path().canonicalize().expect("canonicalize cwd"),
        )
        .expect("absolute canonical root");
        let expected_dot_codex = expected_root.join(".codex");

        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_dot_codex)
        );
    }

    #[test]
    fn legacy_workspace_write_projection_preserves_symbolic_project_root() {
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };

        assert_eq!(
            FileSystemSandboxPolicy::from(&policy),
            FileSystemSandboxPolicy::restricted(vec![
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
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(Some(".git".into())),
                    },
                    access: FileSystemAccessMode::Read,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(Some(".agents".into())),
                    },
                    access: FileSystemAccessMode::Read,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(Some(".codex".into())),
                    },
                    access: FileSystemAccessMode::Read,
                },
            ])
        );
    }

    #[test]
    fn legacy_current_working_directory_special_path_deserializes_as_project_roots()
    -> serde_json::Result<()> {
        let value = serde_json::json!({
            "kind": "current_working_directory",
        });

        let special_path = serde_json::from_value::<FileSystemSpecialPath>(value)?;
        assert_eq!(
            special_path,
            FileSystemSpecialPath::project_roots(/*subpath*/ None)
        );
        assert_eq!(
            serde_json::to_value(&special_path)?,
            serde_json::json!({
                "kind": "project_roots",
            })
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_skip_default_dot_codex_when_explicit_user_rule_exists() {
        let cwd = TempDir::new().expect("tempdir");
        let expected_root = AbsolutePathBuf::from_absolute_path(
            cwd.path().canonicalize().expect("canonicalize cwd"),
        )
        .expect("absolute canonical root");
        let explicit_dot_codex = expected_root.join(".codex");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: explicit_dot_codex.clone(),
                },
                access: FileSystemAccessMode::Write,
            },
        ]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        let workspace_root = writable_roots
            .iter()
            .find(|root| root.root == expected_root)
            .expect("workspace writable root");
        assert!(
            !workspace_root
                .protected_metadata_names
                .contains(&".codex".to_string()),
            "explicit .codex rule should remove the metadata-name protection"
        );
        assert!(
            !workspace_root
                .read_only_subpaths
                .contains(&explicit_dot_codex),
            "explicit .codex rule should win over the default protected carveout"
        );
        assert!(
            policy.can_write_path_with_cwd(
                explicit_dot_codex.join("config.toml").as_path(),
                cwd.path()
            )
        );
    }

    #[test]
    fn filesystem_policy_blocks_protected_metadata_path_writes_by_default() {
        let cwd = TempDir::new().expect("tempdir");
        let dot_git_config = cwd.path().join(".git").join("config");
        let dot_agents_config = cwd.path().join(".agents").join("config");
        let dot_codex_config = cwd.path().join(".codex").join("config.toml");
        let root = AbsolutePathBuf::from_absolute_path(cwd.path()).expect("absolute cwd");
        let file_system_policy =
            FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: root },
                access: FileSystemAccessMode::Write,
            }]);

        assert!(!file_system_policy.can_write_path_with_cwd(&dot_git_config, cwd.path()));
        assert!(!file_system_policy.can_write_path_with_cwd(&dot_agents_config, cwd.path()));
        assert!(!file_system_policy.can_write_path_with_cwd(&dot_codex_config, cwd.path()));

        let writable_roots = file_system_policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(
            writable_roots[0].protected_metadata_names,
            vec![
                ".git".to_string(),
                ".agents".to_string(),
                ".codex".to_string(),
            ]
        );
        assert!(!writable_roots[0].is_path_writable(&dot_git_config));
        assert!(!writable_roots[0].is_path_writable(&dot_agents_config));
        assert!(!writable_roots[0].is_path_writable(&dot_codex_config));
    }

    #[test]
    fn legacy_workspace_write_projection_accepts_relative_cwd() {
        let relative_cwd = Path::new("workspace");
        let expected_root = AbsolutePathBuf::from_absolute_path(
            std::env::current_dir()
                .expect("current dir")
                .join(relative_cwd),
        )
        .expect("absolute root");
        let policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };

        let file_system_policy =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(&policy, relative_cwd);

        let mut expected_entries = vec![
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
        expected_entries.extend(PROTECTED_METADATA_PATH_NAMES.iter().map(|name| {
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(Some((*name).into())),
                },
                access: FileSystemAccessMode::Read,
            }
        }));
        expected_entries.extend(
            default_read_only_subpaths_for_writable_root(
                &expected_root,
                /*protect_missing_dot_codex*/ true,
            )
            .into_iter()
            .map(|path| FileSystemSandboxEntry {
                path: FileSystemPath::Path { path },
                access: FileSystemAccessMode::Read,
            }),
        );

        assert_eq!(
            file_system_policy,
            FileSystemSandboxPolicy::restricted(expected_entries)
        );
        assert_eq!(
            forbidden_agent_metadata_write(
                Path::new(".git/config"),
                relative_cwd,
                &file_system_policy,
            ),
            Some(".git")
        );
        assert!(
            !file_system_policy
                .can_write_path_with_cwd(Path::new(".codex/config.toml"), relative_cwd,)
        );
        assert!(
            !file_system_policy.can_write_path_with_cwd(
                Path::new(".agents/skills/example/SKILL.md"),
                relative_cwd,
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn effective_runtime_roots_preserve_symlinked_paths() {
        let cwd = TempDir::new().expect("tempdir");
        let real_root = cwd.path().join("real");
        let link_root = cwd.path().join("link");
        let blocked = real_root.join("blocked");
        let codex_dir = real_root.join(".codex");

        fs::create_dir_all(&blocked).expect("create blocked");
        fs::create_dir_all(&codex_dir).expect("create .codex");
        symlink_dir(&real_root, &link_root).expect("create symlinked root");

        let link_root =
            AbsolutePathBuf::from_absolute_path(&link_root).expect("absolute symlinked root");
        let link_blocked = link_root.join("blocked");
        let expected_root = link_root.clone();
        let expected_blocked = link_blocked.clone();
        let expected_codex = link_root.join(".codex");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_root },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_blocked },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            policy.get_unreadable_roots_with_cwd(cwd.path()),
            vec![expected_blocked.clone()]
        );

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_blocked)
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_codex)
        );
    }

    #[cfg(unix)]
    #[test]
    fn project_roots_special_path_preserves_symlinked_root() {
        let cwd = TempDir::new().expect("tempdir");
        let real_root = cwd.path().join("real");
        let link_root = cwd.path().join("link");
        let blocked = real_root.join("blocked");
        let agents_dir = real_root.join(".agents");
        let codex_dir = real_root.join(".codex");

        fs::create_dir_all(&blocked).expect("create blocked");
        fs::create_dir_all(&agents_dir).expect("create .agents");
        fs::create_dir_all(&codex_dir).expect("create .codex");
        symlink_dir(&real_root, &link_root).expect("create symlinked cwd");

        let link_blocked =
            AbsolutePathBuf::from_absolute_path(link_root.join("blocked")).expect("link blocked");
        let expected_root =
            AbsolutePathBuf::from_absolute_path(&link_root).expect("absolute symlinked root");
        let expected_blocked = link_blocked.clone();
        let expected_agents = expected_root.join(".agents");
        let expected_codex = expected_root.join(".codex");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Minimal,
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
                path: FileSystemPath::Path { path: link_blocked },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            policy.get_readable_roots_with_cwd(&link_root),
            vec![expected_root.clone()]
        );
        assert_eq!(
            policy.get_unreadable_roots_with_cwd(&link_root),
            vec![expected_blocked.clone()]
        );

        let writable_roots = policy.get_writable_roots_with_cwd(&link_root);
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_blocked)
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_agents)
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_codex)
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_preserve_symlinked_protected_subpaths() {
        let cwd = TempDir::new().expect("tempdir");
        let root = cwd.path().join("root");
        let decoy = root.join("decoy-codex");
        let dot_codex = root.join(".codex");
        fs::create_dir_all(&decoy).expect("create decoy");
        symlink_dir(&decoy, &dot_codex).expect("create .codex symlink");

        let root = AbsolutePathBuf::from_absolute_path(&root).expect("absolute root");
        let expected_dot_codex = AbsolutePathBuf::from_absolute_path(
            root.as_path()
                .canonicalize()
                .expect("canonicalize root")
                .join(".codex"),
        )
        .expect("absolute .codex symlink");
        let unexpected_decoy =
            AbsolutePathBuf::from_absolute_path(decoy.canonicalize().expect("canonicalize decoy"))
                .expect("absolute canonical decoy");

        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: root },
            access: FileSystemAccessMode::Write,
        }]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(
            writable_roots[0].read_only_subpaths,
            vec![expected_dot_codex]
        );
        assert!(
            !writable_roots[0]
                .read_only_subpaths
                .contains(&unexpected_decoy)
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_preserve_explicit_symlinked_carveouts_under_symlinked_roots() {
        let cwd = TempDir::new().expect("tempdir");
        let real_root = cwd.path().join("real");
        let link_root = cwd.path().join("link");
        let decoy = real_root.join("decoy-private");
        let linked_private = real_root.join("linked-private");
        fs::create_dir_all(&decoy).expect("create decoy");
        symlink_dir(&real_root, &link_root).expect("create symlinked root");
        symlink_dir(&decoy, &linked_private).expect("create linked-private symlink");

        let link_root =
            AbsolutePathBuf::from_absolute_path(&link_root).expect("absolute symlinked root");
        let link_private = link_root.join("linked-private");
        let expected_root = link_root.clone();
        let expected_linked_private = link_private.clone();
        let unexpected_decoy =
            AbsolutePathBuf::from_absolute_path(decoy.canonicalize().expect("canonicalize decoy"))
                .expect("absolute canonical decoy");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_root },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_private },
                access: FileSystemAccessMode::None,
            },
        ]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert_eq!(
            writable_roots[0].read_only_subpaths,
            vec![expected_linked_private]
        );
        assert!(
            !writable_roots[0]
                .read_only_subpaths
                .contains(&unexpected_decoy)
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_preserve_explicit_symlinked_carveouts_that_escape_root() {
        let cwd = TempDir::new().expect("tempdir");
        let real_root = cwd.path().join("real");
        let link_root = cwd.path().join("link");
        let decoy = cwd.path().join("outside-private");
        let linked_private = real_root.join("linked-private");
        fs::create_dir_all(&decoy).expect("create decoy");
        fs::create_dir_all(&real_root).expect("create real root");
        symlink_dir(&real_root, &link_root).expect("create symlinked root");
        symlink_dir(&decoy, &linked_private).expect("create linked-private symlink");

        let link_root =
            AbsolutePathBuf::from_absolute_path(&link_root).expect("absolute symlinked root");
        let link_private = link_root.join("linked-private");
        let expected_root = link_root.clone();
        let expected_linked_private = link_private.clone();
        let unexpected_decoy =
            AbsolutePathBuf::from_absolute_path(decoy.canonicalize().expect("canonicalize decoy"))
                .expect("absolute canonical decoy");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_root },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_private },
                access: FileSystemAccessMode::None,
            },
        ]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert_eq!(
            writable_roots[0].read_only_subpaths,
            vec![expected_linked_private]
        );
        assert!(
            !writable_roots[0]
                .read_only_subpaths
                .contains(&unexpected_decoy)
        );
    }

    #[cfg(unix)]
    #[test]
    fn writable_roots_preserve_explicit_symlinked_carveouts_that_alias_root() {
        let cwd = TempDir::new().expect("tempdir");
        let root = cwd.path().join("root");
        let alias = root.join("alias-root");
        fs::create_dir_all(&root).expect("create root");
        symlink_dir(&root, &alias).expect("create alias symlink");

        let root = AbsolutePathBuf::from_absolute_path(&root).expect("absolute root");
        let alias = root.join("alias-root");
        let expected_root = AbsolutePathBuf::from_absolute_path(
            root.as_path().canonicalize().expect("canonicalize root"),
        )
        .expect("absolute canonical root");
        let expected_alias = expected_root.join("alias-root");

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: root },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: alias },
                access: FileSystemAccessMode::None,
            },
        ]);

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert_eq!(writable_roots[0].read_only_subpaths, vec![expected_alias]);
    }

    #[cfg(unix)]
    #[test]
    fn tmpdir_special_path_preserves_symlinked_tmpdir() {
        if std::env::var_os(SYMLINKED_TMPDIR_TEST_ENV).is_none() {
            let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
                .env(SYMLINKED_TMPDIR_TEST_ENV, "1")
                .arg("--exact")
                .arg("permissions::tests::tmpdir_special_path_preserves_symlinked_tmpdir")
                .output()
                .expect("run tmpdir subprocess test");

            assert!(
                output.status.success(),
                "tmpdir subprocess test failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let cwd = TempDir::new().expect("tempdir");
        let real_tmpdir = cwd.path().join("real-tmpdir");
        let link_tmpdir = cwd.path().join("link-tmpdir");
        let blocked = real_tmpdir.join("blocked");
        let codex_dir = real_tmpdir.join(".codex");

        fs::create_dir_all(&blocked).expect("create blocked");
        fs::create_dir_all(&codex_dir).expect("create .codex");
        symlink_dir(&real_tmpdir, &link_tmpdir).expect("create symlinked tmpdir");

        let link_blocked =
            AbsolutePathBuf::from_absolute_path(link_tmpdir.join("blocked")).expect("link blocked");
        let expected_root =
            AbsolutePathBuf::from_absolute_path(&link_tmpdir).expect("absolute symlinked tmpdir");
        let expected_blocked = link_blocked.clone();
        let expected_codex = expected_root.join(".codex");

        unsafe {
            std::env::set_var("TMPDIR", &link_tmpdir);
        }

        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Tmpdir,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: link_blocked },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert_eq!(
            policy.get_unreadable_roots_with_cwd(cwd.path()),
            vec![expected_blocked.clone()]
        );

        let writable_roots = policy.get_writable_roots_with_cwd(cwd.path());
        assert_eq!(writable_roots.len(), 1);
        assert_eq!(writable_roots[0].root, expected_root);
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_blocked)
        );
        assert!(
            writable_roots[0]
                .read_only_subpaths
                .contains(&expected_codex)
        );
    }

    #[test]
    fn resolve_access_with_cwd_uses_most_specific_entry() {
        let cwd = TempDir::new().expect("tempdir");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let docs_private = AbsolutePathBuf::resolve_path_against_base("docs/private", cwd.path());
        let docs_private_public =
            AbsolutePathBuf::resolve_path_against_base("docs/private/public", cwd.path());
        let policy = FileSystemSandboxPolicy::restricted(vec![
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
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: docs_private.clone(),
                },
                access: FileSystemAccessMode::None,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path {
                    path: docs_private_public.clone(),
                },
                access: FileSystemAccessMode::Write,
            },
        ]);

        assert_eq!(
            policy.resolve_access_with_cwd(cwd.path(), cwd.path()),
            FileSystemAccessMode::Write
        );
        assert_eq!(
            policy.resolve_access_with_cwd(docs.as_path(), cwd.path()),
            FileSystemAccessMode::Read
        );
        assert_eq!(
            policy.resolve_access_with_cwd(docs_private.as_path(), cwd.path()),
            FileSystemAccessMode::None
        );
        assert_eq!(
            policy.resolve_access_with_cwd(docs_private_public.as_path(), cwd.path()),
            FileSystemAccessMode::Write
        );
    }

    #[test]
    fn split_only_nested_carveouts_need_direct_runtime_enforcement() {
        let cwd = TempDir::new().expect("tempdir");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert!(
            policy.needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path(),)
        );

        let legacy_workspace_write = legacy_runtime_file_system_policy_for_cwd(
            &SandboxPolicy::new_workspace_write_policy(),
            cwd.path(),
        );
        assert!(
            legacy_workspace_write
                .needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path(),),
            "metadata-name protections must stay in the direct enforcement path even when legacy concrete read-only paths match"
        );
    }

    #[test]
    fn legacy_projection_runtime_enforcement_ignores_entry_order() {
        let cwd = TempDir::new().expect("tempdir");
        let legacy_policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };
        let legacy_order = legacy_runtime_file_system_policy_for_cwd(&legacy_policy, cwd.path());
        let mut reordered_entries = legacy_order.entries.clone();
        reordered_entries.reverse();
        let reordered = FileSystemSandboxPolicy::restricted(reordered_entries);

        assert!(
            legacy_order.is_semantically_equivalent_to(&reordered, cwd.path()),
            "entry order should not affect filesystem semantics"
        );
        assert_eq!(
            legacy_order
                .needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path()),
            reordered
                .needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path()),
            "entry order should not affect direct-enforcement classification"
        );
    }

    #[test]
    fn missing_symbolic_metadata_carveouts_need_direct_runtime_enforcement() {
        let cwd = TempDir::new().expect("tempdir");
        let legacy_policy = SandboxPolicy::WorkspaceWrite {
            writable_roots: Vec::new(),
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        };

        let profile_projection =
            FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(&legacy_policy, cwd.path());
        assert!(
            profile_projection
                .needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path()),
            "symbolic .git/.agents carveouts protect missing paths that legacy sandboxes cannot represent"
        );

        let legacy_runtime_projection =
            legacy_runtime_file_system_policy_for_cwd(&legacy_policy, cwd.path());
        assert!(
            legacy_runtime_projection
                .needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path()),
            "metadata-name protections are outside the legacy SandboxPolicy writable-root contract"
        );
    }

    #[test]
    fn root_write_with_read_only_child_is_not_full_disk_write() {
        let cwd = TempDir::new().expect("tempdir");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs.clone() },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert!(!policy.has_full_disk_write_access());
        assert_eq!(
            policy.resolve_access_with_cwd(docs.as_path(), cwd.path()),
            FileSystemAccessMode::Read
        );
        assert!(
            policy.needs_direct_runtime_enforcement(NetworkSandboxPolicy::Restricted, cwd.path(),)
        );
        assert!(
            policy
                .to_legacy_sandbox_policy(NetworkSandboxPolicy::Restricted, cwd.path())
                .is_err()
        );
    }

    #[test]
    fn root_deny_does_not_materialize_as_unreadable_root() {
        let cwd = TempDir::new().expect("tempdir");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let expected_docs = AbsolutePathBuf::from_absolute_path(
            canonicalize_preserving_symlinks(cwd.path())
                .expect("canonicalize cwd")
                .join("docs"),
        )
        .expect("canonical docs");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::None,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs.clone() },
                access: FileSystemAccessMode::Read,
            },
        ]);

        assert_eq!(
            policy.resolve_access_with_cwd(docs.as_path(), cwd.path()),
            FileSystemAccessMode::Read
        );
        assert_eq!(
            policy.get_readable_roots_with_cwd(cwd.path()),
            vec![expected_docs]
        );
        assert!(policy.get_unreadable_roots_with_cwd(cwd.path()).is_empty());
    }

    #[test]
    fn duplicate_root_deny_prevents_full_disk_write_access() {
        let cwd = TempDir::new().expect("tempdir");
        let root = AbsolutePathBuf::from_absolute_path(cwd.path())
            .map(|cwd| absolute_root_path_for_cwd(&cwd))
            .expect("resolve filesystem root");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::None,
            },
        ]);

        assert!(!policy.has_full_disk_write_access());
        assert_eq!(
            policy.resolve_access_with_cwd(root.as_path(), cwd.path()),
            FileSystemAccessMode::None
        );
    }

    #[test]
    fn same_specificity_write_override_keeps_full_disk_write_access() {
        let cwd = TempDir::new().expect("tempdir");
        let docs = AbsolutePathBuf::resolve_path_against_base("docs", cwd.path());
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs.clone() },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Path { path: docs.clone() },
                access: FileSystemAccessMode::Write,
            },
        ]);

        assert!(policy.has_full_disk_write_access());
        assert_eq!(
            policy.resolve_access_with_cwd(docs.as_path(), cwd.path()),
            FileSystemAccessMode::Write
        );
    }

    #[test]
    fn with_additional_readable_roots_skips_existing_effective_access() {
        let cwd = TempDir::new().expect("tempdir");
        let cwd_root = AbsolutePathBuf::from_absolute_path(cwd.path()).expect("absolute cwd");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Read,
        }]);

        let actual = policy
            .clone()
            .with_additional_readable_roots(cwd.path(), std::slice::from_ref(&cwd_root));

        assert_eq!(actual, policy);
    }

    #[test]
    fn with_additional_writable_roots_skips_existing_effective_access() {
        let cwd = TempDir::new().expect("tempdir");
        let cwd_root = AbsolutePathBuf::from_absolute_path(cwd.path()).expect("absolute cwd");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);

        let actual = policy
            .clone()
            .with_additional_writable_roots(cwd.path(), std::slice::from_ref(&cwd_root));

        assert_eq!(actual, policy);
    }

    #[test]
    fn with_additional_writable_roots_adds_new_root() {
        let temp_dir = TempDir::new().expect("tempdir");
        let cwd = temp_dir.path().join("workspace");
        let extra = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("extra"))
            .expect("resolve extra root");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);

        let actual = policy.with_additional_writable_roots(&cwd, std::slice::from_ref(&extra));

        assert_eq!(
            actual,
            FileSystemSandboxPolicy::restricted(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path { path: extra },
                    access: FileSystemAccessMode::Write,
                },
            ])
        );
    }

    #[test]
    fn materialize_project_roots_with_workspace_roots_expands_exact_and_glob_entries() {
        let temp_dir = TempDir::new().expect("tempdir");
        let first = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("first"))
            .expect("resolve first root");
        let second = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("second"))
            .expect("resolve second root");
        let policy = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(Some(".git".into())),
                },
                access: FileSystemAccessMode::Read,
            },
            FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: project_roots_glob_pattern(Path::new("**/*.env")),
                },
                access: FileSystemAccessMode::None,
            },
        ]);

        let actual =
            policy.materialize_project_roots_with_workspace_roots(&[first.clone(), second.clone()]);

        assert_eq!(
            actual,
            FileSystemSandboxPolicy::restricted(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: first.clone(),
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: second.clone(),
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: first.join(".git"),
                    },
                    access: FileSystemAccessMode::Read,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: second.join(".git"),
                    },
                    access: FileSystemAccessMode::Read,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::GlobPattern {
                        pattern: AbsolutePathBuf::resolve_path_against_base(
                            "**/*.env",
                            first.as_path(),
                        )
                        .to_string_lossy()
                        .into_owned(),
                    },
                    access: FileSystemAccessMode::None,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::GlobPattern {
                        pattern: AbsolutePathBuf::resolve_path_against_base(
                            "**/*.env",
                            second.as_path(),
                        )
                        .to_string_lossy()
                        .into_owned(),
                    },
                    access: FileSystemAccessMode::None,
                },
            ])
        );
    }

    #[test]
    fn materialize_project_roots_with_cwd_expands_symbolic_glob_entries() {
        let cwd = TempDir::new().expect("tempdir");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern {
                pattern: project_roots_glob_pattern(Path::new("**/*.env")),
            },
            access: FileSystemAccessMode::None,
        }]);

        let actual = policy.materialize_project_roots_with_cwd(cwd.path());

        assert_eq!(
            actual,
            FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
                path: FileSystemPath::GlobPattern {
                    pattern: AbsolutePathBuf::resolve_path_against_base("**/*.env", cwd.path())
                        .to_string_lossy()
                        .into_owned(),
                },
                access: FileSystemAccessMode::None,
            }])
        );
    }

    #[test]
    fn with_additional_legacy_workspace_writable_roots_protects_metadata() {
        let temp_dir = TempDir::new().expect("tempdir");
        let extra = AbsolutePathBuf::from_absolute_path(temp_dir.path().join("extra"))
            .expect("resolve extra root");
        std::fs::create_dir_all(extra.join(".git")).expect("create .git dir");
        let policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);

        let actual =
            policy.with_additional_legacy_workspace_writable_roots(std::slice::from_ref(&extra));

        assert_eq!(
            actual,
            FileSystemSandboxPolicy::restricted(vec![
                FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: extra.clone()
                    },
                    access: FileSystemAccessMode::Write,
                },
                FileSystemSandboxEntry {
                    path: FileSystemPath::Path {
                        path: extra.join(".git")
                    },
                    access: FileSystemAccessMode::Read,
                },
            ])
        );
    }

    #[test]
    fn file_system_access_mode_orders_by_conflict_precedence() {
        assert!(FileSystemAccessMode::Write > FileSystemAccessMode::Read);
        assert!(FileSystemAccessMode::None > FileSystemAccessMode::Write);
    }

    #[test]
    fn legacy_bridge_preserves_explicit_deny_entries() {
        let denied = AbsolutePathBuf::try_from("/tmp/private").expect("absolute path");
        let existing = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: denied.clone(),
            },
            access: FileSystemAccessMode::None,
        }]);

        let rebuilt = FileSystemSandboxPolicy::from_legacy_sandbox_policy_preserving_deny_entries(
            &SandboxPolicy::new_workspace_write_policy(),
            Path::new("/tmp/workspace"),
            &existing,
        );

        assert!(
            rebuilt.entries.iter().any(|entry| {
                entry.path
                    == FileSystemPath::Path {
                        path: denied.clone(),
                    }
                    && entry.access == FileSystemAccessMode::None
            }),
            "expected explicit deny entry to be preserved"
        );
    }

    #[test]
    fn preserving_deny_entries_keeps_unrestricted_policy_enforceable() {
        let deny_entry = unreadable_glob_entry("/tmp/project/**/*.env".to_string());
        let mut existing = FileSystemSandboxPolicy::restricted(vec![deny_entry.clone()]);
        existing.glob_scan_max_depth = Some(2);
        let mut replacement = FileSystemSandboxPolicy::unrestricted();

        replacement.preserve_deny_read_restrictions_from(&existing);

        let mut expected = FileSystemSandboxPolicy::restricted(vec![
            FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::Root,
                },
                access: FileSystemAccessMode::Write,
            },
            deny_entry,
        ]);
        expected.glob_scan_max_depth = Some(2);
        assert_eq!(replacement, expected);
    }

    fn unreadable_glob_entry(pattern: String) -> FileSystemSandboxEntry {
        FileSystemSandboxEntry {
            path: FileSystemPath::GlobPattern { pattern },
            access: FileSystemAccessMode::None,
        }
    }
