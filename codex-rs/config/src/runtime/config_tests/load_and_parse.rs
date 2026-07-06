use super::*;

#[tokio::test]
async fn load_config_normalizes_relative_cwd_override() -> std::io::Result<()> {
    let expected_cwd = AbsolutePathBuf::relative_to_current_dir("nested")?;
    let codex_home = tempdir()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(PathBuf::from("nested")),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.cwd, expected_cwd);
    Ok(())
}

#[tokio::test]
async fn load_config_does_not_inline_global_agents_instructions() -> std::io::Result<()> {
    let codex_home = tempdir()?;
    std::fs::write(
        codex_home.path().join(DEFAULT_AGENTS_MD_FILENAME),
        "\n  global instructions  \n",
    )?;

    let mut config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;
    let _ = config.features.enable(Feature::MemoryTool);

    assert_eq!(config.user_instructions, None);
    Ok(())
}

#[tokio::test]
async fn load_config_does_not_inline_global_agents_override_instructions() -> std::io::Result<()> {
    let codex_home = tempdir()?;
    std::fs::write(
        codex_home.path().join(DEFAULT_AGENTS_MD_FILENAME),
        "global instructions",
    )?;
    let global_agents_override_path = codex_home.path().join(LOCAL_AGENTS_MD_FILENAME);
    std::fs::write(&global_agents_override_path, "local override instructions")?;

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.user_instructions, None);
    Ok(())
}

#[tokio::test]
async fn load_config_reads_explicit_instruction_files() -> std::io::Result<()> {
    let cwd = tempdir()?;
    let parsed = deserialize_config_toml_with_base(
        toml::toml! {
            instruction_files = ["instructions/project.md", "instructions/user.md"]
        },
        cwd.path(),
    )
    .expect("instruction files config should deserialize");
    let config = Config::load_from_base_config_with_overrides(
        parsed,
        ConfigOverrides {
            cwd: Some(cwd.abs()),
            ..Default::default()
        },
        tempdir()?.abs(),
    )
    .await?;

    assert_eq!(config.instruction_files.len(), 2);
    assert_eq!(
        config.instruction_files[0],
        cwd.abs().join("instructions").join("project.md")
    );
    assert_eq!(
        config.instruction_files[1],
        cwd.abs().join("instructions").join("user.md")
    );
    Ok(())
}

#[tokio::test]
async fn test_toml_parsing() {
    let history_with_persistence = r#"
[history]
persistence = "save-all"
"#;
    let history_with_persistence_cfg = toml::from_str::<ConfigToml>(history_with_persistence)
        .expect("TOML deserialization should succeed");
    assert_eq!(
        Some(History {
            persistence: HistoryPersistence::SaveAll,
            max_bytes: None,
        }),
        history_with_persistence_cfg.history
    );

    let history_no_persistence = r#"
[history]
persistence = "none"
"#;

    let history_no_persistence_cfg = toml::from_str::<ConfigToml>(history_no_persistence)
        .expect("TOML deserialization should succeed");
    assert_eq!(
        Some(History {
            persistence: HistoryPersistence::None,
            max_bytes: None,
        }),
        history_no_persistence_cfg.history
    );

    let memories = r#"
[memories]
disable_on_external_context = true
generate_memories = false
use_memories = false
max_raw_memories_for_consolidation = 512
max_unused_days = 21
max_rollout_age_days = 42
max_rollouts_per_startup = 9
min_rollout_idle_hours = 24
min_rate_limit_remaining_percent = 12
extract_model = "gpt-5-mini"
consolidation_model = "gpt-5.2"
"#;
    let memories_cfg =
        toml::from_str::<ConfigToml>(memories).expect("TOML deserialization should succeed");
    assert_eq!(
        Some(MemoriesToml {
            disable_on_external_context: Some(true),
            generate_memories: Some(false),
            use_memories: Some(false),
            max_raw_memories_for_consolidation: Some(512),
            max_unused_days: Some(21),
            max_rollout_age_days: Some(42),
            max_rollouts_per_startup: Some(9),
            min_rollout_idle_hours: Some(24),
            min_rate_limit_remaining_percent: Some(12),
            extract_model: Some("gpt-5-mini".to_string()),
            consolidation_model: Some("gpt-5.2".to_string()),
            ..Default::default()
        }),
        memories_cfg.memories
    );

    let config = Config::load_from_base_config_with_overrides(
        memories_cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load config from memories settings");
    assert_eq!(config.memories.disable_on_external_context, true);
    assert_eq!(config.memories.generate_memories, false);
    assert_eq!(config.memories.use_memories, false);
    assert_eq!(config.memories.max_raw_memories_for_consolidation, 512);
    assert_eq!(config.memories.max_unused_days, 21);
    assert_eq!(config.memories.max_rollout_age_days, 42);
    assert_eq!(config.memories.max_rollouts_per_startup, 9);
    assert_eq!(config.memories.min_rollout_idle_hours, 24);
    assert_eq!(config.memories.min_rate_limit_remaining_percent, 12);
    assert_eq!(
        config.memories.extract_model,
        Some("gpt-5-mini".to_string())
    );
    assert_eq!(
        config.memories.consolidation_model,
        Some("gpt-5.2".to_string())
    );
    assert_eq!(
        config.memories.compact_replacement_file_token_limit,
        DEFAULT_COMPACT_REPLACEMENT_FILE_TOKEN_LIMIT
    );
    assert!(config.memories.compact_replacement_files.is_empty());

    let legacy_memories_cfg =
        toml::from_str::<ConfigToml>("[memories]\nno_memories_if_mcp_or_web_search = true\n")
            .expect("legacy memories TOML should deserialize");
    assert!(
        MemoriesConfig::from(
            legacy_memories_cfg
                .memories
                .expect("legacy memories config")
        )
        .disable_on_external_context
    );
}

#[test]
fn parses_bundled_skills_config() {
    let cfg: ConfigToml = toml::from_str(
        r#"
[skills]
include_instructions = false

[skills.bundled]
enabled = false
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.skills,
        Some(SkillsConfig {
            bundled: Some(BundledSkillsConfig { enabled: false }),
            include_instructions: Some(false),
            config: Vec::new(),
        })
    );
}

#[test]
fn tools_web_search_true_deserializes_to_none() {
    let cfg: ConfigToml = toml::from_str(
        r#"
[tools]
web_search = true
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(cfg.tools, Some(ToolsToml { web_search: None }));
}

#[test]
fn tools_web_search_false_deserializes_to_none() {
    let cfg: ConfigToml = toml::from_str(
        r#"
[tools]
web_search = false
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(cfg.tools, Some(ToolsToml { web_search: None }));
}

#[test]
fn rejects_provider_auth_with_env_key() {
    let err = toml::from_str::<ConfigToml>(
        r#"
[model_providers.corp]
name = "Corp"
env_key = "CORP_TOKEN"

[model_providers.corp.auth]
command = "print-token"
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("model_providers.corp: provider auth cannot be combined with env_key")
    );
}

#[test]
fn rejects_provider_aws_for_custom_provider() {
    let err = toml::from_str::<ConfigToml>(
        r#"
[model_providers.custom]
name = "Custom Provider"

[model_providers.custom.aws]
profile = "codex-bedrock"
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string().contains(
            "model_providers.custom: provider aws is only supported for `amazon-bedrock`"
        )
    );
}

#[test]
fn accepts_amazon_bedrock_aws_profile_override() {
    let cfg = toml::from_str::<ConfigToml>(
        r#"
[model_providers.amazon-bedrock.aws]
profile = "codex-bedrock"
region = "us-west-2"
"#,
    )
    .expect("Amazon Bedrock AWS overrides should deserialize");

    assert_eq!(
        cfg.model_providers
            .get("amazon-bedrock")
            .and_then(|provider| provider.aws.as_ref())
            .and_then(|aws| aws.profile.as_deref()),
        Some("codex-bedrock")
    );
    assert_eq!(
        cfg.model_providers
            .get("amazon-bedrock")
            .and_then(|provider| provider.aws.as_ref())
            .and_then(|aws| aws.region.as_deref()),
        Some("us-west-2")
    );
}

#[tokio::test]
async fn load_config_applies_amazon_bedrock_aws_profile_override() {
    let cfg = toml::from_str::<ConfigToml>(
        r#"
model_provider = "amazon-bedrock"

[model_providers.amazon-bedrock.aws]
profile = "codex-bedrock"
region = "us-west-2"
"#,
    )
    .expect("Amazon Bedrock AWS overrides should deserialize");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load config");

    assert_eq!(config.model_provider_id, "amazon-bedrock");
    assert_eq!(
        config
            .model_provider
            .aws
            .as_ref()
            .and_then(|aws| aws.profile.as_deref()),
        Some("codex-bedrock")
    );
    assert_eq!(
        config
            .model_provider
            .aws
            .as_ref()
            .and_then(|aws| aws.region.as_deref()),
        Some("us-west-2")
    );
}

#[tokio::test]
async fn load_config_rejects_unsupported_amazon_bedrock_overrides() {
    let cfg = toml::from_str::<ConfigToml>(
        r#"
model_provider = "amazon-bedrock"

[model_providers.amazon-bedrock]
name = "Custom Bedrock"
base_url = "https://bedrock.example.com/v1"
requires_openai_auth = true
supports_websockets = true

[model_providers.amazon-bedrock.aws]
profile = "codex-bedrock"
region = "us-west-2"
"#,
    )
    .expect("Amazon Bedrock unsupported overrides should deserialize");

    let err = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains(
        "model_providers.amazon-bedrock only supports changing `aws.profile` and `aws.region`; other non-default provider fields are not supported"
    ));
}

#[test]
fn config_toml_deserializes_model_availability_nux() {
    let toml = r#"
[tui.model_availability_nux]
"gpt-foo" = 2
"gpt-bar" = 4
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for TUI NUX");

    assert_eq!(
        cfg.tui.expect("tui config should deserialize"),
        Tui {
            notification_settings: TuiNotificationSettings::default(),
            animations: true,
            show_tooltips: true,
            vim_mode_default: false,
            raw_output_mode: false,
            alternate_screen: AltScreenMode::default(),
            status_line: None,
            status_line_use_colors: true,
            terminal_title: None,
            theme: None,
            pet: None,
            pet_anchor: TuiPetAnchor::Composer,
            session_picker_view: None,
            keymap: TuiKeymap::default(),
            model_availability_nux: ModelAvailabilityNuxConfig {
                shown_count: HashMap::from([
                    ("gpt-bar".to_string(), 4),
                    ("gpt-foo".to_string(), 2),
                ]),
            },
            terminal_resize_reflow_max_rows: None,
        }
    );
}

#[test]
fn config_toml_status_line_use_colors_defaults_to_enabled() {
    let toml = r#"
[tui]
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for TUI config");

    assert!(
        cfg.tui
            .expect("tui config should deserialize")
            .status_line_use_colors
    );
}

#[test]
fn config_toml_deserializes_status_line_use_colors_disabled() {
    let toml = r#"
[tui]
status_line_use_colors = false
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for TUI config");

    assert!(
        !cfg.tui
            .expect("tui config should deserialize")
            .status_line_use_colors
    );
}

#[test]
fn config_toml_deserializes_terminal_resize_reflow_config() {
    let toml = r#"
[tui]
terminal_resize_reflow_max_rows = 9000
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for resize reflow config");

    assert_eq!(
        cfg.tui
            .expect("tui config should deserialize")
            .terminal_resize_reflow_max_rows,
        Some(9000)
    );
}

#[tokio::test]
async fn runtime_config_defaults_model_availability_nux() {
    let cfg = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load config");

    assert_eq!(
        cfg.model_availability_nux,
        ModelAvailabilityNuxConfig::default()
    );
}

#[test]
fn test_tui_vim_mode_default_defaults_to_false() {
    let toml = r#"
        [tui]
    "#;
    let parsed: ConfigToml = toml::from_str(toml).expect("deserialize empty [tui] table");
    assert!(
        !parsed
            .tui
            .expect("config should include tui section")
            .vim_mode_default
    );
}

#[test]
fn test_tui_vim_mode_default_true() {
    let toml = r#"
        [tui]
        vim_mode_default = true
    "#;
    let parsed: ConfigToml = toml::from_str(toml).expect("deserialize vim_mode_default=true");
    assert!(
        parsed
            .tui
            .expect("config should include tui section")
            .vim_mode_default
    );
}

#[test]
fn test_tui_raw_output_mode_defaults_to_false() {
    let toml = r#"
        [tui]
    "#;
    let parsed: ConfigToml = toml::from_str(toml).expect("deserialize empty [tui] table");
    assert!(
        !parsed
            .tui
            .expect("config should include tui section")
            .raw_output_mode
    );
}

#[test]
fn test_tui_raw_output_mode_true() {
    let toml = r#"
        [tui]
        raw_output_mode = true
    "#;
    let parsed: ConfigToml = toml::from_str(toml).expect("deserialize raw_output_mode=true");
    assert!(
        parsed
            .tui
            .expect("config should include tui section")
            .raw_output_mode
    );
}

#[tokio::test]
async fn runtime_config_uses_tui_raw_output_mode() {
    let toml = r#"
        [tui]
        raw_output_mode = true
    "#;
    let cfg_toml: ConfigToml = toml::from_str(toml).expect("deserialize raw_output_mode=true");
    let cfg = Config::load_from_base_config_with_overrides(
        cfg_toml,
        ConfigOverrides::default(),
        tempdir().expect("tempdir").abs(),
    )
    .await
    .expect("load config");

    assert!(cfg.tui_raw_output_mode);
}

#[test]
fn config_toml_deserializes_permission_profiles() {
    let toml = r#"
default_permissions = "workspace"

[permissions.workspace.workspace_roots]
"~/code/openai" = true
"~/code/ignored" = false

[permissions.workspace.filesystem]
":minimal" = "read"

[permissions.workspace.filesystem.":workspace_roots"]
"." = "write"
"docs" = "read"

[permissions.workspace.network]
enabled = true
proxy_url = "http://127.0.0.1:43128"
enable_socks5 = false
allow_upstream_proxy = false

[permissions.workspace.network.domains]
"openai.com" = "allow"
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for permissions profiles");

    assert_eq!(cfg.default_permissions.as_deref(), Some("workspace"));
    assert_eq!(
        cfg.permissions.expect("[permissions] should deserialize"),
        PermissionsToml {
            entries: BTreeMap::from([(
                "workspace".to_string(),
                PermissionProfileToml {
                    workspace_roots: Some(WorkspaceRootsToml {
                        entries: BTreeMap::from([
                            ("~/code/ignored".to_string(), false),
                            ("~/code/openai".to_string(), true),
                        ]),
                    }),
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
                    network: Some(NetworkToml {
                        enabled: Some(true),
                        proxy_url: Some("http://127.0.0.1:43128".to_string()),
                        enable_socks5: Some(false),
                        socks_url: None,
                        enable_socks5_udp: None,
                        allow_upstream_proxy: Some(false),
                        dangerously_allow_non_loopback_proxy: None,
                        dangerously_allow_all_unix_sockets: None,
                        mode: None,
                        domains: Some(NetworkDomainPermissionsToml {
                            entries: BTreeMap::from([(
                                "openai.com".to_string(),
                                NetworkDomainPermissionToml::Allow,
                            )]),
                        }),
                        unix_sockets: None,
                        allow_local_binding: None,
                    }),
                },
            )]),
        }
    );
}

#[tokio::test]
async fn permissions_profiles_proxy_policy_does_not_start_managed_network_proxy_without_feature()
-> std::io::Result<()> {
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
                            proxy_url: Some("http://127.0.0.1:43128".to_string()),
                            enable_socks5: Some(false),
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
    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    assert!(
        config.permissions.network.is_none(),
        "profile proxy policy should not start the managed network proxy without the feature"
    );
    Ok(())
}

#[tokio::test]
async fn network_proxy_feature_is_no_op_without_sandbox_network() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            features: Some(toml::from_str("network_proxy = true").expect("valid features")),
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
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Restricted
    );
    assert!(
        config.permissions.network.is_none(),
        "network_proxy should not start the managed network proxy while network access is off"
    );
    Ok(())
}

#[tokio::test]
async fn network_proxy_feature_matrix_preserves_sandbox_network_semantics() -> std::io::Result<()> {
    #[derive(Clone, Copy)]
    enum Surface {
        PermissionProfile,
        LegacyWorkspaceWrite,
    }

    struct Case {
        name: &'static str,
        surface: Surface,
        network_enabled: bool,
        proxy_enabled: bool,
        expected_network_policy: NetworkSandboxPolicy,
    }

    let cases = [
        Case {
            name: "permission profile network disabled without proxy",
            surface: Surface::PermissionProfile,
            network_enabled: false,
            proxy_enabled: false,
            expected_network_policy: NetworkSandboxPolicy::Restricted,
        },
        Case {
            name: "permission profile network disabled with proxy",
            surface: Surface::PermissionProfile,
            network_enabled: false,
            proxy_enabled: true,
            expected_network_policy: NetworkSandboxPolicy::Restricted,
        },
        Case {
            name: "permission profile network enabled without proxy",
            surface: Surface::PermissionProfile,
            network_enabled: true,
            proxy_enabled: false,
            expected_network_policy: NetworkSandboxPolicy::Enabled,
        },
        Case {
            name: "permission profile network enabled with proxy",
            surface: Surface::PermissionProfile,
            network_enabled: true,
            proxy_enabled: true,
            expected_network_policy: NetworkSandboxPolicy::Enabled,
        },
        Case {
            name: "legacy workspace write network disabled without proxy",
            surface: Surface::LegacyWorkspaceWrite,
            network_enabled: false,
            proxy_enabled: false,
            expected_network_policy: NetworkSandboxPolicy::Restricted,
        },
        Case {
            name: "legacy workspace write network disabled with proxy",
            surface: Surface::LegacyWorkspaceWrite,
            network_enabled: false,
            proxy_enabled: true,
            expected_network_policy: NetworkSandboxPolicy::Restricted,
        },
        Case {
            name: "legacy workspace write network enabled without proxy",
            surface: Surface::LegacyWorkspaceWrite,
            network_enabled: true,
            proxy_enabled: false,
            expected_network_policy: NetworkSandboxPolicy::Enabled,
        },
        Case {
            name: "legacy workspace write network enabled with proxy",
            surface: Surface::LegacyWorkspaceWrite,
            network_enabled: true,
            proxy_enabled: true,
            expected_network_policy: NetworkSandboxPolicy::Enabled,
        },
    ];

    for case in cases {
        let codex_home = TempDir::new()?;
        let cwd = TempDir::new()?;
        std::fs::write(cwd.path().join(".git"), "gitdir: nowhere")?;
        let features = case
            .proxy_enabled
            .then(|| toml::from_str("network_proxy = true").expect("valid features"));
        let base_config = match case.surface {
            Surface::PermissionProfile => ConfigToml {
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
                                enabled: Some(case.network_enabled),
                                ..Default::default()
                            }),
                        },
                    )]),
                }),
                features,
                ..Default::default()
            },
            Surface::LegacyWorkspaceWrite => ConfigToml {
                sandbox_mode: Some(SandboxMode::WorkspaceWrite),
                sandbox_workspace_write: Some(SandboxWorkspaceWrite {
                    network_access: case.network_enabled,
                    ..Default::default()
                }),
                windows: Some(WindowsToml {
                    sandbox: Some(WindowsSandboxModeToml::Elevated),
                    sandbox_private_desktop: None,
                }),
                features,
                ..Default::default()
            },
        };
        let config = Config::load_from_base_config_with_overrides(
            base_config,
            ConfigOverrides {
                cwd: Some(cwd.path().to_path_buf()),
                ..Default::default()
            },
            codex_home.abs(),
        )
        .await?;

        assert_eq!(
            config.permissions.network_sandbox_policy(),
            case.expected_network_policy,
            "{}",
            case.name
        );
        assert_eq!(
            config.permissions.network.is_some(),
            case.network_enabled && case.proxy_enabled,
            "{}",
            case.name
        );
    }

    Ok(())
}

#[tokio::test]
async fn network_proxy_cli_overrides_merge_toggle_with_proxy_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
sandbox_mode = "workspace-write"

[sandbox_workspace_write]
network_access = true

[windows]
sandbox = "elevated"
"#,
    )?;
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cli_overrides(vec![
            (
                "features.network_proxy.enabled".to_string(),
                toml::Value::Boolean(true),
            ),
            (
                "features.network_proxy.enable_socks5".to_string(),
                toml::Value::Boolean(false),
            ),
        ])
        .harness_overrides(ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    let network = config
        .permissions
        .network
        .as_ref()
        .expect("network_proxy should start the managed network proxy");
    assert_eq!(network.proxy_host_and_port(), "127.0.0.1:3128");
    assert!(!network.socks_enabled());
    Ok(())
}

#[tokio::test]
async fn experimental_network_requirements_enable_proxy_without_feature() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                network: Some(config_service::NetworkRequirementsToml {
                    enabled: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert!(!config.features.enabled(Feature::NetworkProxy));
    assert!(config.managed_network_requirements_enabled());
    assert!(
        config
            .permissions
            .network
            .as_ref()
            .expect("experimental_network should configure the managed proxy")
            .enabled()
    );
    Ok(())
}

#[tokio::test]
async fn network_proxy_feature_uses_profile_network_proxy_settings() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            features: Some(toml::from_str("network_proxy = true").expect("valid features")),
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

    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Enabled
    );
    let network = config
        .permissions
        .network
        .as_ref()
        .expect("network_proxy should start the managed network proxy");
    assert_eq!(network.proxy_host_and_port(), "127.0.0.1:43128");
    assert!(!network.socks_enabled());
    Ok(())
}

#[tokio::test]
async fn profile_network_proxy_disable_ignores_base_feature_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            features: Some(
                toml::from_str(
                    r#"
[network_proxy]
enabled = true
proxy_url = "http://127.0.0.1:43128"
"#,
                )
                .expect("valid base features"),
            ),
            profiles: HashMap::from([(
                "no_proxy".to_string(),
                ConfigProfile {
                    features: Some(
                        toml::from_str("network_proxy = false").expect("valid profile features"),
                    ),
                    ..Default::default()
                },
            )]),
            profile: Some("no_proxy".to_string()),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert!(!config.features.enabled(Feature::NetworkProxy));
    assert!(config.permissions.network.is_none());
    Ok(())
}

#[tokio::test]
async fn disabled_network_proxy_feature_does_not_start_profile_proxy_policy() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    let cwd = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            features: Some(
                toml::from_str(
                    r#"
[network_proxy]
enabled = false
"#,
                )
                .expect("valid features"),
            ),
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

    assert!(!config.features.enabled(Feature::NetworkProxy));
    assert!(
        config.permissions.network.is_none(),
        "disabled feature should keep profile proxy policy from starting the managed proxy"
    );
    Ok(())
}

#[tokio::test]
async fn permissions_profiles_network_disabled_by_default_does_not_start_proxy()
-> std::io::Result<()> {
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
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert!(config.permissions.network.is_none());
    Ok(())
}
