use super::*;

#[tokio::test]
async fn forced_chatgpt_workspace_id_empty_values_disable_runtime_restriction()
-> std::io::Result<()> {
    let cases: Vec<(&str, &str, Option<Vec<&str>>)> = vec![
        ("unset", "", None),
        ("empty string", r#"forced_chatgpt_workspace_id = """#, None),
        (
            "whitespace string",
            r#"forced_chatgpt_workspace_id = "   ""#,
            None,
        ),
        ("empty list", r#"forced_chatgpt_workspace_id = []"#, None),
        (
            "blank list entries",
            r#"forced_chatgpt_workspace_id = ["", "  "]"#,
            None,
        ),
        (
            "mixed list entries",
            r#"forced_chatgpt_workspace_id = ["", " 123e4567-e89b-42d3-a456-426614174000 ", "123e4567-e89b-42d3-a456-426614174001"]"#,
            Some(vec![
                "123e4567-e89b-42d3-a456-426614174000",
                "123e4567-e89b-42d3-a456-426614174001",
            ]),
        ),
    ];

    for (name, toml, expected) in cases {
        let cfg_toml: ConfigToml = toml::from_str(toml)
            .unwrap_or_else(|err| panic!("{name} should parse forced_chatgpt_workspace_id: {err}"));
        let config = Config::load_from_base_config_with_overrides(
            cfg_toml,
            ConfigOverrides::default(),
            tempdir().expect("tempdir").abs(),
        )
        .await?;

        let expected = expected.map(|values| {
            values
                .into_iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        });
        assert_eq!(config.forced_chatgpt_workspace_id, expected, "{name}");
    }

    Ok(())
}

#[tokio::test]
async fn legacy_remote_thread_store_endpoint_is_rejected() {
    let cfg: ConfigToml =
        toml::from_str(r#"experimental_thread_store_endpoint = "https://example.com""#)
            .expect("legacy remote thread-store endpoint should still deserialize");

    let err = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect_err("legacy remote thread-store endpoint should be rejected at load time");

    assert!(
        err.to_string()
            .contains("experimental_thread_store_endpoint")
    );
    assert!(err.to_string().contains("no longer supported"));
}

#[test]
fn profile_tui_rejects_unsupported_settings() {
    let err = toml::from_str::<ConfigToml>(
        r#"profile = "work"

[profiles.work.tui]
theme = "dark"
"#,
    )
    .expect_err("profile TUI config should only accept supported fields");

    assert!(err.to_string().contains("unknown field"));
    assert!(err.to_string().contains("theme"));
}

#[tokio::test]
async fn runtime_config_resolves_session_picker_view_default_and_override() {
    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load default config");

    assert_eq!(cfg.tui_session_picker_view, SessionPickerViewMode::Dense);

    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml {
            tui: Some(Tui {
                session_picker_view: Some(SessionPickerViewMode::Comfortable),
                ..Default::default()
            }),
            ..Default::default()
        },
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load root override config");

    assert_eq!(
        cfg.tui_session_picker_view,
        SessionPickerViewMode::Comfortable
    );

    let cfg_toml = toml::from_str::<ConfigToml>(
        r#"profile = "work"

[tui]
session_picker_view = "dense"

[profiles.work.tui]
session_picker_view = "comfortable"
"#,
    )
    .expect("parse profile scoped tui config");

    let cfg = Config::load_from_base_config_with_overrides(
        cfg_toml,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load profile override config");

    assert_eq!(
        cfg.tui_session_picker_view,
        SessionPickerViewMode::Comfortable
    );
}

#[tokio::test]
async fn test_sandbox_config_parsing() {
    let sandbox_full_access = r#"
sandbox_mode = "danger-full-access"

[sandbox_workspace_write]
network_access = false  # This should be ignored.
"#;
    let sandbox_full_access_cfg = toml::from_str::<ConfigToml>(sandbox_full_access)
        .expect("TOML deserialization should succeed");
    let sandbox_mode_override = None;
    let resolution = derive_legacy_sandbox_policy_for_test(
        &sandbox_full_access_cfg,
        sandbox_mode_override,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        /*active_project*/ None,
        /*permission_profile_constraint*/ None,
    )
    .await;
    assert_eq!(resolution, SandboxPolicy::DangerFullAccess);

    let sandbox_read_only = r#"
sandbox_mode = "read-only"

[sandbox_workspace_write]
network_access = true  # This should be ignored.
"#;

    let sandbox_read_only_cfg = toml::from_str::<ConfigToml>(sandbox_read_only)
        .expect("TOML deserialization should succeed");
    let sandbox_mode_override = None;
    let resolution = derive_legacy_sandbox_policy_for_test(
        &sandbox_read_only_cfg,
        sandbox_mode_override,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        /*active_project*/ None,
        /*permission_profile_constraint*/ None,
    )
    .await;
    assert_eq!(resolution, SandboxPolicy::new_read_only_policy());

    let writable_root = test_absolute_path("/my/workspace");
    let sandbox_workspace_write = format!(
        r#"
sandbox_mode = "workspace-write"

[sandbox_workspace_write]
writable_roots = [
    {},
]
exclude_tmpdir_env_var = true
exclude_slash_tmp = true

[projects."/tmp/test"]
trust_level = "trusted"
"#,
        serde_json::json!(writable_root)
    );

    let sandbox_workspace_write_cfg = toml::from_str::<ConfigToml>(&sandbox_workspace_write)
        .expect("TOML deserialization should succeed");
    let sandbox_mode_override = None;
    let resolution = derive_legacy_sandbox_policy_for_test(
        &sandbox_workspace_write_cfg,
        sandbox_mode_override,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        /*active_project*/ None,
        /*permission_profile_constraint*/ None,
    )
    .await;
    if cfg!(target_os = "windows") {
        assert_eq!(resolution, SandboxPolicy::new_read_only_policy());
    } else {
        assert_eq!(
            resolution,
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![writable_root.clone()],
                network_access: false,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            }
        );
    }

    let sandbox_workspace_write = format!(
        r#"
sandbox_mode = "workspace-write"

[sandbox_workspace_write]
writable_roots = [
    {},
]
exclude_tmpdir_env_var = true
exclude_slash_tmp = true
"#,
        serde_json::json!(writable_root)
    );

    let sandbox_workspace_write_cfg = toml::from_str::<ConfigToml>(&sandbox_workspace_write)
        .expect("TOML deserialization should succeed");
    let sandbox_mode_override = None;
    let resolution = derive_legacy_sandbox_policy_for_test(
        &sandbox_workspace_write_cfg,
        sandbox_mode_override,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        /*active_project*/ None,
        /*permission_profile_constraint*/ None,
    )
    .await;
    if cfg!(target_os = "windows") {
        assert_eq!(resolution, SandboxPolicy::new_read_only_policy());
    } else {
        assert_eq!(
            resolution,
            SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![writable_root],
                network_access: false,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            }
        );
    }
}

#[tokio::test]
async fn legacy_sandbox_mode_builds_profiles_with_compatible_projection() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let extra_root = test_absolute_path("/tmp/legacy-extra-root");
    let cases = vec![
        (
            "danger-full-access".to_string(),
            r#"sandbox_mode = "danger-full-access"
"#
            .to_string(),
        ),
        (
            "read-only".to_string(),
            r#"sandbox_mode = "read-only"
"#
            .to_string(),
        ),
        (
            "workspace-write".to_string(),
            format!(
                r#"sandbox_mode = "workspace-write"

[sandbox_workspace_write]
writable_roots = [{}]
exclude_tmpdir_env_var = true
exclude_slash_tmp = true
"#,
                serde_json::json!(extra_root)
            ),
        ),
    ];

    for (name, config_toml) in cases {
        let cfg = toml::from_str::<ConfigToml>(&config_toml)
            .unwrap_or_else(|err| panic!("case `{name}` should parse: {err}"));
        let config = Config::load_from_base_config_with_overrides(
            cfg,
            ConfigOverrides {
                cwd: Some(cwd.path().to_path_buf()),
                ..Default::default()
            },
            codex_home.abs(),
        )
        .await?;

        let sandbox_policy = config.legacy_sandbox_policy();
        let file_system_policy = config.permissions.file_system_sandbox_policy();
        let network_policy = config.permissions.network_sandbox_policy();

        assert_eq!(
            network_policy,
            NetworkSandboxPolicy::from(&sandbox_policy),
            "case `{name}` should preserve network semantics from legacy config"
        );
        assert_eq!(
            file_system_policy
                .to_legacy_sandbox_policy(network_policy, cwd.path())
                .unwrap_or_else(|err| panic!("case `{name}` should round-trip: {err}")),
            sandbox_policy,
            "case `{name}` should preserve its legacy compatibility projection"
        );

        match name.as_str() {
            "danger-full-access" | "read-only" => {
                assert_eq!(
                    file_system_policy,
                    FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
                        &sandbox_policy,
                        cwd.path()
                    ),
                    "case `{name}` should match the legacy filesystem projection exactly"
                );
            }
            "workspace-write" => {
                if cfg!(target_os = "windows") {
                    assert_eq!(
                        sandbox_policy,
                        SandboxPolicy::new_read_only_policy(),
                        "legacy workspace-write should keep the existing Windows downgrade when \
                         the experimental Windows sandbox is disabled"
                    );
                    assert_eq!(
                        file_system_policy,
                        FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
                            &sandbox_policy,
                            cwd.path()
                        ),
                        "downgraded workspace-write should match the legacy read-only projection"
                    );
                    continue;
                }
                assert_eq!(
                    config.permissions.workspace_roots(),
                    &[cwd.abs(), extra_root.clone()]
                );
                assert!(
                    file_system_policy
                        .entries
                        .contains(&FileSystemSandboxEntry {
                            path: FileSystemPath::Path { path: cwd.abs() },
                            access: FileSystemAccessMode::Write,
                        })
                );
                assert!(
                    file_system_policy
                        .entries
                        .contains(&FileSystemSandboxEntry {
                            path: FileSystemPath::Path {
                                path: extra_root.clone(),
                            },
                            access: FileSystemAccessMode::Write,
                        })
                );
                for subpath in [".git", ".agents", ".codex"] {
                    assert!(
                        file_system_policy
                            .entries
                            .contains(&FileSystemSandboxEntry {
                                path: FileSystemPath::Path {
                                    path: AbsolutePathBuf::resolve_path_against_base(
                                        subpath,
                                        cwd.path()
                                    ),
                                },
                                access: FileSystemAccessMode::Read,
                            }),
                        "case `{name}` should materialize `{subpath}` for the runtime workspace \
                         root"
                    );
                }
            }
            _ => unreachable!("unexpected test case `{name}`"),
        }
    }

    Ok(())
}

#[test]
fn filter_mcp_servers_by_allowlist_enforces_identity_rules() {
    const MISMATCHED_COMMAND_SERVER: &str = "mismatched-command-should-disable";
    const MISMATCHED_URL_SERVER: &str = "mismatched-url-should-disable";
    const MATCHED_COMMAND_SERVER: &str = "matched-command-should-allow";
    const MATCHED_URL_SERVER: &str = "matched-url-should-allow";
    const DIFFERENT_NAME_SERVER: &str = "different-name-should-disable";

    const GOOD_CMD: &str = "good-cmd";
    const GOOD_URL: &str = "https://example.com/good";

    let mut servers = HashMap::from([
        (MISMATCHED_COMMAND_SERVER.to_string(), stdio_mcp("docs-cmd")),
        (
            MISMATCHED_URL_SERVER.to_string(),
            http_mcp("https://example.com/mcp"),
        ),
        (MATCHED_COMMAND_SERVER.to_string(), stdio_mcp(GOOD_CMD)),
        (MATCHED_URL_SERVER.to_string(), http_mcp(GOOD_URL)),
        (DIFFERENT_NAME_SERVER.to_string(), stdio_mcp("same-cmd")),
    ]);
    let source = RequirementSource::LegacyManagedConfigTomlFromMdm;
    let requirements = Sourced::new(
        BTreeMap::from([
            (
                MISMATCHED_URL_SERVER.to_string(),
                McpServerRequirement {
                    identity: McpServerIdentity::Url {
                        url: "https://example.com/other".to_string(),
                    },
                },
            ),
            (
                MISMATCHED_COMMAND_SERVER.to_string(),
                McpServerRequirement {
                    identity: McpServerIdentity::Command {
                        command: "other-cmd".to_string(),
                    },
                },
            ),
            (
                MATCHED_URL_SERVER.to_string(),
                McpServerRequirement {
                    identity: McpServerIdentity::Url {
                        url: GOOD_URL.to_string(),
                    },
                },
            ),
            (
                MATCHED_COMMAND_SERVER.to_string(),
                McpServerRequirement {
                    identity: McpServerIdentity::Command {
                        command: GOOD_CMD.to_string(),
                    },
                },
            ),
        ]),
        source.clone(),
    );
    filter_mcp_servers_by_requirements(&mut servers, Some(&requirements));

    let reason = Some(McpServerDisabledReason::Requirements { source });
    assert_eq!(
        servers
            .iter()
            .map(|(name, server)| (
                name.clone(),
                (server.enabled, server.disabled_reason.clone())
            ))
            .collect::<HashMap<String, (bool, Option<McpServerDisabledReason>)>>(),
        HashMap::from([
            (MISMATCHED_URL_SERVER.to_string(), (false, reason.clone())),
            (
                MISMATCHED_COMMAND_SERVER.to_string(),
                (false, reason.clone()),
            ),
            (MATCHED_URL_SERVER.to_string(), (true, None)),
            (MATCHED_COMMAND_SERVER.to_string(), (true, None)),
            (DIFFERENT_NAME_SERVER.to_string(), (false, reason)),
        ])
    );
}

#[test]
fn filter_mcp_servers_by_allowlist_allows_all_when_unset() {
    let mut servers = HashMap::from([
        ("server-a".to_string(), stdio_mcp("cmd-a")),
        ("server-b".to_string(), http_mcp("https://example.com/b")),
    ]);

    filter_mcp_servers_by_requirements(&mut servers, /*mcp_requirements*/ None);

    assert_eq!(
        servers
            .iter()
            .map(|(name, server)| (
                name.clone(),
                (server.enabled, server.disabled_reason.clone())
            ))
            .collect::<HashMap<String, (bool, Option<McpServerDisabledReason>)>>(),
        HashMap::from([
            ("server-a".to_string(), (true, None)),
            ("server-b".to_string(), (true, None)),
        ])
    );
}

#[test]
fn filter_mcp_servers_by_allowlist_blocks_all_when_empty() {
    let mut servers = HashMap::from([
        ("server-a".to_string(), stdio_mcp("cmd-a")),
        ("server-b".to_string(), http_mcp("https://example.com/b")),
    ]);

    let source = RequirementSource::LegacyManagedConfigTomlFromMdm;
    let requirements = Sourced::new(BTreeMap::new(), source.clone());
    filter_mcp_servers_by_requirements(&mut servers, Some(&requirements));

    let reason = Some(McpServerDisabledReason::Requirements { source });
    assert_eq!(
        servers
            .iter()
            .map(|(name, server)| (
                name.clone(),
                (server.enabled, server.disabled_reason.clone())
            ))
            .collect::<HashMap<String, (bool, Option<McpServerDisabledReason>)>>(),
        HashMap::from([
            ("server-a".to_string(), (false, reason.clone())),
            ("server-b".to_string(), (false, reason)),
        ])
    );
}

#[test]
fn filter_plugin_mcp_servers_by_allowlist_enforces_plugin_and_identity_rules() {
    const MATCHED_SERVER: &str = "matched-should-allow";
    const MISMATCHED_SERVER: &str = "mismatched-should-disable";
    const UNLISTED_SERVER: &str = "unlisted-should-disable";
    const GOOD_CMD: &str = "good-cmd";

    let mut servers = HashMap::from([
        (MATCHED_SERVER.to_string(), stdio_mcp(GOOD_CMD)),
        (MISMATCHED_SERVER.to_string(), stdio_mcp("bad-cmd")),
        (
            UNLISTED_SERVER.to_string(),
            http_mcp("https://example.com/mcp"),
        ),
    ]);
    let source = RequirementSource::CloudRequirements;
    let requirements = Sourced::new(
        BTreeMap::from([(
            "sample@test".to_string(),
            config_service::PluginRequirementsToml {
                mcp_servers: Some(BTreeMap::from([
                    (
                        MATCHED_SERVER.to_string(),
                        McpServerRequirement {
                            identity: McpServerIdentity::Command {
                                command: GOOD_CMD.to_string(),
                            },
                        },
                    ),
                    (
                        MISMATCHED_SERVER.to_string(),
                        McpServerRequirement {
                            identity: McpServerIdentity::Command {
                                command: GOOD_CMD.to_string(),
                            },
                        },
                    ),
                ])),
            },
        )]),
        source.clone(),
    );

    filter_plugin_mcp_servers_by_requirements("sample@test", &mut servers, Some(&requirements));

    let reason = Some(McpServerDisabledReason::Requirements { source });
    assert_eq!(
        servers
            .iter()
            .map(|(name, server)| (
                name.clone(),
                (server.enabled, server.disabled_reason.clone())
            ))
            .collect::<HashMap<String, (bool, Option<McpServerDisabledReason>)>>(),
        HashMap::from([
            (MATCHED_SERVER.to_string(), (true, None)),
            (MISMATCHED_SERVER.to_string(), (false, reason.clone())),
            (UNLISTED_SERVER.to_string(), (false, reason)),
        ])
    );
}

#[test]
fn filter_plugin_mcp_servers_by_allowlist_blocks_unlisted_plugin() {
    let mut servers = HashMap::from([("server-a".to_string(), stdio_mcp("cmd-a"))]);
    let source = RequirementSource::CloudRequirements;
    let requirements = Sourced::new(
        BTreeMap::from([(
            "other@test".to_string(),
            config_service::PluginRequirementsToml {
                mcp_servers: Some(BTreeMap::from([(
                    "server-a".to_string(),
                    McpServerRequirement {
                        identity: McpServerIdentity::Command {
                            command: "cmd-a".to_string(),
                        },
                    },
                )])),
            },
        )]),
        source.clone(),
    );

    filter_plugin_mcp_servers_by_requirements("sample@test", &mut servers, Some(&requirements));

    assert_eq!(
        servers
            .iter()
            .map(|(name, server)| (
                name.clone(),
                (server.enabled, server.disabled_reason.clone())
            ))
            .collect::<HashMap<String, (bool, Option<McpServerDisabledReason>)>>(),
        HashMap::from([(
            "server-a".to_string(),
            (
                false,
                Some(McpServerDisabledReason::Requirements { source })
            )
        )])
    );
}

#[tokio::test]
async fn rebuild_preserving_session_layers_refreshes_requirements() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let user_file = AbsolutePathBuf::resolve_path_against_base(CONFIG_TOML_FILE, codex_home.path());
    let project_dot_codex =
        AbsolutePathBuf::resolve_path_against_base("project/.codex", codex_home.path());
    let mcp_requirements = BTreeMap::from([
        (
            "session_overrides_user".to_string(),
            McpServerRequirement {
                identity: McpServerIdentity::Command {
                    command: "session-command".to_string(),
                },
            },
        ),
        (
            "managed_overrides_session".to_string(),
            McpServerRequirement {
                identity: McpServerIdentity::Command {
                    command: "managed-command".to_string(),
                },
            },
        ),
        (
            "fresh_global".to_string(),
            McpServerRequirement {
                identity: McpServerIdentity::Command {
                    command: "fresh-global-command".to_string(),
                },
            },
        ),
        (
            "fresh_project".to_string(),
            McpServerRequirement {
                identity: McpServerIdentity::Command {
                    command: "fresh-project-command".to_string(),
                },
            },
        ),
    ]);
    let requirements_toml = config_service::ConfigRequirementsToml {
        mcp_servers: Some(mcp_requirements.clone()),
        ..Default::default()
    };
    let requirements = config_service::ConfigRequirements {
        mcp_servers: Some(Sourced::new(mcp_requirements, RequirementSource::Unknown)),
        ..Default::default()
    };
    let refreshed_layer_stack = ConfigLayerStack::new(
        vec![
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::User {
                    file: user_file.clone(),
                    profile: None,
                },
                toml::toml! {
                    [mcp_servers.session_overrides_user]
                    command = "new-user-command"
                    [mcp_servers.managed_overrides_session]
                    command = "new-user-command"
                    [mcp_servers.fresh_global]
                    command = "fresh-global-command"
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::Project {
                    dot_codex_folder: project_dot_codex.clone(),
                },
                toml::toml! {
                    [mcp_servers.fresh_project]
                    command = "fresh-project-command"
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::LegacyManagedConfigTomlFromMdm,
                toml::toml! {
                    [mcp_servers.managed_overrides_session]
                    command = "managed-command"
                }
                .into(),
            ),
        ],
        requirements,
        requirements_toml,
    )
    .map_err(std::io::Error::other)?;
    let refreshed_toml = refreshed_layer_stack
        .effective_config()
        .try_into()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let refreshed_config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        refreshed_toml,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        refreshed_layer_stack,
    )
    .await?;
    let thread_layer_stack = ConfigLayerStack::new(
        vec![
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::User {
                    file: user_file.clone(),
                    profile: None,
                },
                toml::toml! {
                    [mcp_servers.session_overrides_user]
                    command = "old-user-command"
                    [mcp_servers.managed_overrides_session]
                    command = "old-user-command"
                    [mcp_servers.fresh_global]
                    command = "old-global-command"
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::Project {
                    dot_codex_folder: project_dot_codex,
                },
                toml::toml! {
                    [mcp_servers.fresh_project]
                    command = "old-project-command"
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::SessionFlags,
                toml::toml! {
                    [mcp_servers.session_overrides_user]
                    command = "session-command"
                    [mcp_servers.managed_overrides_session]
                    command = "session-command"
                    [mcp_servers.blocked_session]
                    command = "blocked-session-command"
                }
                .into(),
            ),
            ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::LegacyManagedConfigTomlFromMdm,
                toml::toml! {
                    [mcp_servers.managed_overrides_session]
                    command = "old-managed-command"
                }
                .into(),
            ),
        ],
        Default::default(),
        Default::default(),
    )
    .map_err(std::io::Error::other)?;
    let thread_toml = thread_layer_stack
        .effective_config()
        .try_into()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let thread_config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        thread_toml,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        thread_layer_stack,
    )
    .await?;
    let config = thread_config
        .rebuild_preserving_session_layers(&refreshed_config)
        .await?;

    assert_eq!(
        config.mcp_servers.get(),
        &HashMap::from([
            (
                "session_overrides_user".to_string(),
                stdio_mcp("session-command"),
            ),
            (
                "managed_overrides_session".to_string(),
                stdio_mcp("managed-command"),
            ),
            (
                "fresh_global".to_string(),
                stdio_mcp("fresh-global-command"),
            ),
            (
                "fresh_project".to_string(),
                stdio_mcp("fresh-project-command"),
            ),
            (
                "blocked_session".to_string(),
                McpServerConfig {
                    enabled: false,
                    disabled_reason: Some(McpServerDisabledReason::Requirements {
                        source: RequirementSource::Unknown,
                    }),
                    ..stdio_mcp("blocked-session-command")
                },
            ),
        ])
    );

    Ok(())
}

#[tokio::test]
async fn rebuild_preserving_session_layers_refreshes_plugin_derived_mcp_config()
-> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");
    std::fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    std::fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )?;
    std::fs::write(
        plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp"
    }
  }
}"#,
    )?;

    let user_file = AbsolutePathBuf::resolve_path_against_base(CONFIG_TOML_FILE, codex_home.path());
    let refreshed_layer_stack = ConfigLayerStack::new(
        vec![ConfigLayerEntry::new(
            codex_config_types::ConfigLayerSource::User {
                file: user_file.clone(),
                profile: None,
            },
            toml::toml! {
                [features]
                plugins = true

                [plugins."sample@test"]
                enabled = true
            }
            .into(),
        )],
        Default::default(),
        Default::default(),
    )?;
    let refreshed_config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        refreshed_layer_stack.effective_config().try_into()?,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        refreshed_layer_stack,
    )
    .await?;
    let thread_layer_stack = ConfigLayerStack::new(
        vec![ConfigLayerEntry::new(
            codex_config_types::ConfigLayerSource::User {
                file: user_file,
                profile: None,
            },
            toml::toml! {
                [features]
                plugins = false

                [plugins."sample@test"]
                enabled = true
            }
            .into(),
        )],
        Default::default(),
        Default::default(),
    )?;
    let thread_config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        thread_layer_stack.effective_config().try_into()?,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        thread_layer_stack,
    )
    .await?;
    let config = thread_config
        .rebuild_preserving_session_layers(&refreshed_config)
        .await?;
    let plugin_runtime = TestPluginRuntime::with_mcp_servers([(
        "sample".to_string(),
        http_mcp("https://sample.example/mcp"),
    )]);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;

    assert_eq!(
        mcp_config.configured_mcp_servers.get("sample"),
        Some(&http_mcp("https://sample.example/mcp"))
    );

    Ok(())
}

#[tokio::test]
async fn to_mcp_config_applies_plugin_mcp_cloud_requirements() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");
    std::fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    std::fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )?;
    std::fs::write(
        plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp"
    },
    "unlisted": {
      "type": "http",
      "url": "https://unlisted.example/mcp"
    }
  }
}"#,
    )?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[features]
plugins = true

[plugins."sample@test"]
enabled = true
"#,
    )?;

    let requirements = config_service::ConfigRequirementsToml {
        plugins: Some(BTreeMap::from([(
            "sample@test".to_string(),
            config_service::PluginRequirementsToml {
                mcp_servers: Some(BTreeMap::from([(
                    "sample".to_string(),
                    McpServerRequirement {
                        identity: McpServerIdentity::Url {
                            url: "https://sample.example/mcp".to_string(),
                        },
                    },
                )])),
            },
        )])),
        ..Default::default()
    };
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await?;
    let plugin_runtime = TestPluginRuntime::with_mcp_servers([
        ("sample".to_string(), http_mcp("https://sample.example/mcp")),
        (
            "unlisted".to_string(),
            http_mcp("https://unlisted.example/mcp"),
        ),
    ]);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;

    assert_eq!(
        mcp_config
            .configured_mcp_servers
            .get("sample")
            .map(|server| (server.enabled, server.disabled_reason.clone())),
        Some((true, None))
    );
    assert_eq!(
        mcp_config
            .configured_mcp_servers
            .get("unlisted")
            .map(|server| (server.enabled, server.disabled_reason.clone())),
        Some((
            false,
            Some(McpServerDisabledReason::Requirements {
                source: RequirementSource::CloudRequirements,
            })
        ))
    );
    Ok(())
}

#[tokio::test]
async fn to_mcp_config_empty_mcp_requirements_disable_plugin_mcps() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");
    std::fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    std::fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )?;
    std::fs::write(
        plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp"
    }
  }
}"#,
    )?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[features]
plugins = true

[plugins."sample@test"]
enabled = true
"#,
    )?;

    let requirements = config_service::ConfigRequirementsToml {
        mcp_servers: Some(BTreeMap::new()),
        ..Default::default()
    };
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await?;
    let plugin_runtime = TestPluginRuntime::with_mcp_servers([(
        "sample".to_string(),
        http_mcp("https://sample.example/mcp"),
    )]);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;

    assert_eq!(
        mcp_config
            .configured_mcp_servers
            .get("sample")
            .map(|server| (server.enabled, server.disabled_reason.clone())),
        Some((
            false,
            Some(McpServerDisabledReason::Requirements {
                source: RequirementSource::CloudRequirements,
            })
        ))
    );
    Ok(())
}

#[tokio::test]
async fn add_dir_override_extends_workspace_writable_roots() -> std::io::Result<()> {
    let temp_dir = TempDir::new()?;
    let frontend = temp_dir.path().join("frontend");
    let backend = temp_dir.path().join("backend");
    std::fs::create_dir_all(&frontend)?;
    std::fs::create_dir_all(&backend)?;

    let overrides = ConfigOverrides {
        cwd: Some(frontend),
        sandbox_mode: Some(SandboxMode::WorkspaceWrite),
        additional_writable_roots: vec![PathBuf::from("../backend"), backend.clone()],
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        overrides,
        temp_dir.path().abs(),
    )
    .await?;

    let expected_backend = backend.abs();
    if cfg!(target_os = "windows") {
        match &config.legacy_sandbox_policy() {
            SandboxPolicy::ReadOnly { .. } => {}
            other => panic!("expected read-only policy on Windows, got {other:?}"),
        }
    } else {
        match &config.legacy_sandbox_policy() {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => {
                assert_eq!(
                    writable_roots
                        .iter()
                        .filter(|root| **root == expected_backend)
                        .count(),
                    1,
                    "expected single writable root entry for {}",
                    expected_backend.display()
                );
            }
            other => panic!("expected workspace-write policy, got {other:?}"),
        }
    }

    Ok(())
}

#[tokio::test]
async fn sqlite_home_defaults_to_codex_home_for_workspace_write() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            sandbox_mode: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.sqlite_home, codex_home.path().to_path_buf());

    Ok(())
}

#[tokio::test]
async fn workspace_write_always_includes_memories_root_once() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let memories_root = codex_home.path().join("memories");
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            sandbox_workspace_write: Some(SandboxWorkspaceWrite {
                writable_roots: vec![memories_root.abs()],
                ..Default::default()
            }),
            ..Default::default()
        },
        ConfigOverrides {
            sandbox_mode: Some(SandboxMode::WorkspaceWrite),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    if cfg!(target_os = "windows") {
        match &config.legacy_sandbox_policy() {
            SandboxPolicy::ReadOnly { .. } => {}
            other => panic!("expected read-only policy on Windows, got {other:?}"),
        }
    } else {
        assert!(
            memories_root.is_dir(),
            "expected memories root directory to exist at {}",
            memories_root.display()
        );
        let expected_memories_root = memories_root.abs();
        match &config.legacy_sandbox_policy() {
            SandboxPolicy::WorkspaceWrite { writable_roots, .. } => {
                assert_eq!(
                    writable_roots
                        .iter()
                        .filter(|root| **root == expected_memories_root)
                        .count(),
                    1,
                    "expected single writable root entry for {}",
                    expected_memories_root.display()
                );
            }
            other => panic!("expected workspace-write policy, got {other:?}"),
        }
    }

    Ok(())
}

#[tokio::test]
async fn config_defaults_to_file_cli_auth_store_mode() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml::default();

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.cli_auth_credentials_store_mode,
        AuthCredentialsStoreMode::File,
    );

    Ok(())
}

#[tokio::test]
async fn config_resolves_explicit_keyring_auth_store_mode() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        cli_auth_credentials_store: Some(AuthCredentialsStoreMode::Keyring),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.cli_auth_credentials_store_mode,
        resolve_cli_auth_credentials_store_mode(
            AuthCredentialsStoreMode::Keyring,
            env!("CARGO_PKG_VERSION"),
        ),
    );

    Ok(())
}

#[tokio::test]
async fn config_resolves_default_oauth_store_mode() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml::default();

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.mcp_oauth_credentials_store_mode,
        resolve_mcp_oauth_credentials_store_mode(
            OAuthCredentialsStoreMode::Auto,
            env!("CARGO_PKG_VERSION"),
        ),
    );

    Ok(())
}

#[test]
fn local_dev_builds_force_file_cli_auth_store_modes() {
    assert_eq!(
        resolve_cli_auth_credentials_store_mode(
            AuthCredentialsStoreMode::Keyring,
            LOCAL_DEV_BUILD_VERSION,
        ),
        AuthCredentialsStoreMode::File,
    );
    assert_eq!(
        resolve_cli_auth_credentials_store_mode(
            AuthCredentialsStoreMode::Auto,
            LOCAL_DEV_BUILD_VERSION,
        ),
        AuthCredentialsStoreMode::File,
    );
    assert_eq!(
        resolve_cli_auth_credentials_store_mode(
            AuthCredentialsStoreMode::Ephemeral,
            LOCAL_DEV_BUILD_VERSION,
        ),
        AuthCredentialsStoreMode::Ephemeral,
    );
    assert_eq!(
        resolve_cli_auth_credentials_store_mode(AuthCredentialsStoreMode::Keyring, "1.2.3"),
        AuthCredentialsStoreMode::Keyring,
    );
}

#[test]
fn local_dev_builds_force_file_mcp_oauth_store_modes() {
    assert_eq!(
        resolve_mcp_oauth_credentials_store_mode(
            OAuthCredentialsStoreMode::Keyring,
            LOCAL_DEV_BUILD_VERSION,
        ),
        OAuthCredentialsStoreMode::File,
    );
    assert_eq!(
        resolve_mcp_oauth_credentials_store_mode(
            OAuthCredentialsStoreMode::Auto,
            LOCAL_DEV_BUILD_VERSION,
        ),
        OAuthCredentialsStoreMode::File,
    );
    assert_eq!(
        resolve_mcp_oauth_credentials_store_mode(OAuthCredentialsStoreMode::Keyring, "1.2.3"),
        OAuthCredentialsStoreMode::Keyring,
    );
}

#[tokio::test]
async fn feedback_enabled_defaults_to_true() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        feedback: Some(FeedbackConfigToml::default()),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.feedback_enabled, true);

    Ok(())
}

#[test]
fn web_search_mode_defaults_to_none_if_unset() {
    let cfg = ConfigToml::default();
    let profile = ConfigProfile::default();
    let features = Features::with_defaults();

    assert_eq!(resolve_web_search_mode(&cfg, &profile, &features), None);
}

#[test]
fn web_search_mode_prefers_profile_over_legacy_flags() {
    let cfg = ConfigToml::default();
    let profile = ConfigProfile {
        web_search: Some(WebSearchMode::Live),
        ..Default::default()
    };
    let mut features = Features::with_defaults();
    features.enable(Feature::WebSearchCached);

    assert_eq!(
        resolve_web_search_mode(&cfg, &profile, &features),
        Some(WebSearchMode::Live)
    );
}

#[test]
fn web_search_mode_disabled_overrides_legacy_request() {
    let cfg = ConfigToml {
        web_search: Some(WebSearchMode::Disabled),
        ..Default::default()
    };
    let profile = ConfigProfile::default();
    let mut features = Features::with_defaults();
    features.enable(Feature::WebSearchRequest);

    assert_eq!(
        resolve_web_search_mode(&cfg, &profile, &features),
        Some(WebSearchMode::Disabled)
    );
}

#[test]
fn web_search_mode_for_turn_uses_preference_for_read_only() {
    let web_search_mode = Constrained::allow_any(WebSearchMode::Cached);
    let permission_profile =
        PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_read_only_policy());
    let mode = resolve_web_search_mode_for_turn(&web_search_mode, &permission_profile);

    assert_eq!(mode, WebSearchMode::Cached);
}

#[test]
fn web_search_mode_for_turn_prefers_live_for_disabled_permissions() {
    let web_search_mode = Constrained::allow_any(WebSearchMode::Cached);
    let mode = resolve_web_search_mode_for_turn(&web_search_mode, &PermissionProfile::Disabled);

    assert_eq!(mode, WebSearchMode::Live);
}

#[test]
fn web_search_mode_for_turn_respects_disabled_for_disabled_permissions() {
    let web_search_mode = Constrained::allow_any(WebSearchMode::Disabled);
    let mode = resolve_web_search_mode_for_turn(&web_search_mode, &PermissionProfile::Disabled);

    assert_eq!(mode, WebSearchMode::Disabled);
}

#[test]
fn web_search_mode_for_turn_falls_back_when_live_is_disallowed() -> anyhow::Result<()> {
    let allowed = [WebSearchMode::Disabled, WebSearchMode::Cached];
    let web_search_mode = Constrained::new(WebSearchMode::Cached, move |candidate| {
        if allowed.contains(candidate) {
            Ok(())
        } else {
            Err(ConstraintError::InvalidValue {
                field_name: "web_search_mode",
                candidate: format!("{candidate:?}"),
                allowed: format!("{allowed:?}"),
                requirement_source: RequirementSource::Unknown,
            })
        }
    })?;
    let mode = resolve_web_search_mode_for_turn(&web_search_mode, &PermissionProfile::Disabled);

    assert_eq!(mode, WebSearchMode::Cached);
    Ok(())
}

#[tokio::test]
async fn project_profiles_are_ignored() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = TempDir::new()?;
    let workspace_key = workspace.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"
profile = "global"

[profiles.global]
model = "gpt-global"

[profiles.project]
model = "gpt-project"

[projects."{workspace_key}"]
trust_level = "trusted"
"#,
        ),
    )?;
    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    std::fs::write(
        project_config_dir.join(CONFIG_TOML_FILE),
        r#"
profile = "project"

[profiles.project]
model = "gpt-project-local"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(workspace.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    assert_eq!(config.active_profile.as_deref(), Some("global"));
    assert_eq!(config.model.as_deref(), Some("gpt-global"));
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning.contains("profile")
                && warning.contains("profiles")
                && warning.contains(
                    "If you want these settings to apply, manually set them in your user-level config.toml."
                )
        }),
        "expected warning for ignored project-local profile keys: {:?}",
        config.startup_warnings
    );

    Ok(())
}

