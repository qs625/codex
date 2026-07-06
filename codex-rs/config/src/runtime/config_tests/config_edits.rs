use super::*;

#[tokio::test]
async fn set_model_updates_profile() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_model(Some("gpt-5.4"), Some(ReasoningEffort::Medium))
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;
    let profile = parsed
        .profiles
        .get("dev")
        .expect("profile should be created");

    assert_eq!(profile.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(
        profile.model_reasoning_effort,
        Some(ReasoningEffort::Medium)
    );

    Ok(())
}

#[tokio::test]
async fn set_model_updates_existing_profile() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    tokio::fs::write(
        &config_path,
        r#"
[profiles.dev]
model = "gpt-4"
model_reasoning_effort = "medium"

[profiles.prod]
model = "gpt-5.4"
"#,
    )
    .await?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_model(Some("o4-high"), Some(ReasoningEffort::Medium))
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(config_path).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;

    let dev_profile = parsed
        .profiles
        .get("dev")
        .expect("dev profile should survive updates");
    assert_eq!(dev_profile.model.as_deref(), Some("o4-high"));
    assert_eq!(
        dev_profile.model_reasoning_effort,
        Some(ReasoningEffort::Medium)
    );

    assert_eq!(
        parsed
            .profiles
            .get("prod")
            .and_then(|profile| profile.model.as_deref()),
        Some("gpt-5.4"),
    );

    Ok(())
}

#[tokio::test]
async fn set_feature_enabled_updates_profile() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_feature_enabled("guardian_approval", /*enabled*/ true)
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;
    let profile = parsed
        .profiles
        .get("dev")
        .expect("profile should be created");

    assert_eq!(
        profile
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        Some(true),
    );
    assert_eq!(
        parsed
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        None,
    );

    Ok(())
}

#[tokio::test]
async fn set_feature_enabled_persists_feature_disable_in_profile() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_feature_enabled("guardian_approval", /*enabled*/ true)
        .apply()
        .await?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_feature_enabled("guardian_approval", /*enabled*/ false)
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;
    let profile = parsed
        .profiles
        .get("dev")
        .expect("profile should be created");

    assert_eq!(
        profile
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        Some(false),
    );
    assert_eq!(
        parsed
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        None,
    );

    Ok(())
}

#[tokio::test]
async fn set_feature_enabled_profile_disable_overrides_root_enable() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    ConfigEditsBuilder::new(codex_home.path())
        .set_feature_enabled("guardian_approval", /*enabled*/ true)
        .apply()
        .await?;

    ConfigEditsBuilder::new(codex_home.path())
        .with_profile(Some("dev"))
        .set_feature_enabled("guardian_approval", /*enabled*/ false)
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;
    let profile = parsed
        .profiles
        .get("dev")
        .expect("profile should be created");

    assert_eq!(
        parsed
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        Some(true),
    );
    assert_eq!(
        profile
            .features
            .as_ref()
            .and_then(|features| features.entries().get("guardian_approval").copied()),
        Some(false),
    );

    Ok(())
}

pub(crate) struct PrecedenceTestFixture {
    pub(crate) cwd: TempDir,
    pub(crate) codex_home: TempDir,
    pub(crate) cfg: ConfigToml,
    pub(crate) model_provider_map: HashMap<String, ModelProviderInfo>,
    pub(crate) openai_provider: ModelProviderInfo,
    pub(crate) openai_custom_provider: ModelProviderInfo,
}

impl PrecedenceTestFixture {
    pub(crate) fn cwd(&self) -> AbsolutePathBuf {
        self.cwd.abs()
    }

    pub(crate) fn cwd_path(&self) -> PathBuf {
        self.cwd.path().to_path_buf()
    }

    pub(crate) fn codex_home(&self) -> AbsolutePathBuf {
        self.codex_home.abs()
    }
}

#[tokio::test]
async fn cli_override_sets_compact_prompt() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let overrides = ConfigOverrides {
        compact_prompt: Some("Use the compact override".to_string()),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        overrides,
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("Use the compact override")
    );

    Ok(())
}

#[tokio::test]
async fn loads_compact_prompt_from_file() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = codex_home.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;

    let prompt_path = workspace.join("compact_prompt.txt");
    std::fs::write(&prompt_path, "  summarize differently  ")?;

    let cfg = ConfigToml {
        experimental_compact_prompt_file: Some(prompt_path.abs()),
        ..Default::default()
    };

    let overrides = ConfigOverrides {
        cwd: Some(workspace),
        ..Default::default()
    };

    let config =
        Config::load_from_base_config_with_overrides(cfg, overrides, codex_home.abs()).await?;

    assert_eq!(
        config.compact_prompt.as_deref(),
        Some("summarize differently")
    );

    Ok(())
}

#[tokio::test]
async fn load_config_uses_requirements_guardian_policy_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let config_layer_stack = ConfigLayerStack::new(
        Vec::new(),
        Default::default(),
        config_service::ConfigRequirementsToml {
            guardian_policy_config: Some(
                "  Use the workspace-managed guardian policy.  ".to_string(),
            ),
            ..Default::default()
        },
    )
    .map_err(std::io::Error::other)?;

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        config_layer_stack,
    )
    .await?;

    assert_eq!(
        config.guardian_policy_config.as_deref(),
        Some("Use the workspace-managed guardian policy.")
    );

    Ok(())
}

#[test]
fn config_toml_deserializes_auto_review_policy() {
    let cfg = toml::from_str::<ConfigToml>(
        r#"
[auto_review]
policy = "Use the user-configured guardian policy."
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.auto_review
            .as_ref()
            .and_then(|auto_review| auto_review.policy.as_deref()),
        Some("Use the user-configured guardian policy.")
    );
}

#[tokio::test]
async fn load_config_uses_auto_review_guardian_policy_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        auto_review: Some(AutoReviewToml {
            policy: Some("  Use the user-configured guardian policy.  ".to_string()),
        }),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.guardian_policy_config.as_deref(),
        Some("Use the user-configured guardian policy.")
    );

    Ok(())
}

#[tokio::test]
async fn requirements_guardian_policy_beats_auto_review() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let config_layer_stack = ConfigLayerStack::new(
        Vec::new(),
        Default::default(),
        config_service::ConfigRequirementsToml {
            guardian_policy_config: Some("Use the managed guardian policy.".to_string()),
            ..Default::default()
        },
    )
    .map_err(std::io::Error::other)?;
    let cfg = ConfigToml {
        auto_review: Some(AutoReviewToml {
            policy: Some("Use the user-configured guardian policy.".to_string()),
        }),
        ..Default::default()
    };

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        cfg,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        config_layer_stack,
    )
    .await?;

    assert_eq!(
        config.guardian_policy_config.as_deref(),
        Some("Use the managed guardian policy.")
    );

    Ok(())
}

#[tokio::test]
async fn load_config_ignores_empty_auto_review_guardian_policy_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        auto_review: Some(AutoReviewToml {
            policy: Some("   ".to_string()),
        }),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.guardian_policy_config, None);

    Ok(())
}

#[tokio::test]
async fn load_config_ignores_empty_requirements_guardian_policy_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let config_layer_stack = ConfigLayerStack::new(
        Vec::new(),
        Default::default(),
        config_service::ConfigRequirementsToml {
            guardian_policy_config: Some("   ".to_string()),
            ..Default::default()
        },
    )
    .map_err(std::io::Error::other)?;

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        config_layer_stack,
    )
    .await?;

    assert_eq!(config.guardian_policy_config, None);

    Ok(())
}

#[tokio::test]
async fn load_config_rejects_missing_agent_role_config_file() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let missing_path = codex_home.path().join("agents").join("researcher.toml");
    let cfg = ConfigToml {
        agents: Some(AgentsToml {
            max_threads: None,
            max_depth: None,
            job_max_runtime_seconds: None,
            interrupt_message: None,
            roles: BTreeMap::from([(
                "researcher".to_string(),
                AgentRoleToml {
                    description: Some("Research role".to_string()),
                    config_file: Some(missing_path.abs()),
                    nickname_candidates: None,
                },
            )]),
        }),
        ..Default::default()
    };

    let result = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await;
    let err = result.expect_err("missing role config file should be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let message = err.to_string();
    assert!(message.contains("agents.researcher.config_file"));
    assert!(message.contains("must point to an existing file"));

    Ok(())
}

#[tokio::test]
async fn agent_role_relative_config_file_resolves_against_config_toml() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        "developer_instructions = \"Research carefully\"\nmodel = \"gpt-5\"",
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
description = "Research role"
config_file = "./agents/researcher.toml"
nickname_candidates = ["Hypatia", "Noether"]
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.config_file.as_ref()),
        Some(&role_config_path)
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Hypatia", "Noether"])
    );

    Ok(())
}

#[tokio::test]
async fn agent_role_relative_config_file_resolves_from_config_layer() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        "developer_instructions = \"Research carefully\"\nmodel = \"gpt-5\"",
    )
    .await?;
    let layer_config = toml::from_str(
        r#"[agents.researcher]
description = "Research role"
config_file = "./agents/researcher.toml"
"#,
    )
    .expect("agent role layer config should parse");
    let config_layer_stack = config_service::ConfigLayerStack::new(
        vec![config_service::ConfigLayerEntry::new(
            codex_config_types::ConfigLayerSource::User {
                file: codex_home.path().join(CONFIG_TOML_FILE).abs(),
                profile: None,
            },
            layer_config,
        )],
        Default::default(),
        config_service::ConfigRequirementsToml::default(),
    )
    .map_err(std::io::Error::other)?;

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        ConfigToml::default(),
        ConfigOverrides {
            cwd: Some(codex_home.path().to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
        config_layer_stack,
    )
    .await?;

    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.config_file.as_ref()),
        Some(&role_config_path)
    );

    Ok(())
}

#[tokio::test]
async fn agent_role_file_metadata_overrides_config_toml_metadata() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        r#"
description = "Role metadata from file"
nickname_candidates = ["Hypatia"]
developer_instructions = "Research carefully"
model = "gpt-5.2"
"#,
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
description = "Research role from config"
config_file = "./agents/researcher.toml"
nickname_candidates = ["Noether"]
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;
    let role = config
        .agent_roles
        .get("researcher")
        .expect("researcher role should load");
    assert_eq!(role.description.as_deref(), Some("Role metadata from file"));
    assert_eq!(role.config_file.as_ref(), Some(&role_config_path));
    assert_eq!(
        role.nickname_candidates
            .as_ref()
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Hypatia"])
    );

    Ok(())
}

#[tokio::test]
async fn agent_role_file_without_developer_instructions_is_dropped_with_warning()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    let nested_cwd = repo_root.path().join("packages").join("app");
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(&nested_cwd)?;

    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )
    .await?;

    let standalone_agents_dir = repo_root.path().join(".codex").join("agents");
    tokio::fs::create_dir_all(&standalone_agents_dir).await?;
    tokio::fs::write(
        standalone_agents_dir.join("researcher.toml"),
        r#"
name = "researcher"
description = "Role metadata from file"
model = "gpt-5.2"
"#,
    )
    .await?;
    tokio::fs::write(
        standalone_agents_dir.join("reviewer.toml"),
        r#"
name = "reviewer"
description = "Review role"
developer_instructions = "Review carefully"
model = "gpt-5.2"
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(nested_cwd),
            ..Default::default()
        })
        .build()
        .await?;
    assert!(!config.agent_roles.contains_key("researcher"));
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("Review role")
    );
    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("must define `developer_instructions`"))
    );

    Ok(())
}

#[tokio::test]
async fn legacy_agent_role_config_file_allows_missing_developer_instructions() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        r#"
model = "gpt-5.2"
model_reasoning_effort = "high"
"#,
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
description = "Research role from config"
config_file = "./agents/researcher.toml"
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.description.as_deref()),
        Some("Research role from config")
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.config_file.as_ref()),
        Some(&role_config_path)
    );

    Ok(())
}

#[tokio::test]
async fn agent_role_without_description_after_merge_is_dropped_with_warning() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        r#"
developer_instructions = "Research carefully"
model = "gpt-5.2"
"#,
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
config_file = "./agents/researcher.toml"

[agents.reviewer]
description = "Review role"
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;
    assert!(!config.agent_roles.contains_key("researcher"));
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("Review role")
    );
    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("agent role `researcher` must define a description"))
    );

    Ok(())
}

#[tokio::test]
async fn discovered_agent_role_file_without_name_is_dropped_with_warning() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    let nested_cwd = repo_root.path().join("packages").join("app");
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(&nested_cwd)?;

    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )
    .await?;

    let standalone_agents_dir = repo_root.path().join(".codex").join("agents");
    tokio::fs::create_dir_all(&standalone_agents_dir).await?;
    tokio::fs::write(
        standalone_agents_dir.join("researcher.toml"),
        r#"
description = "Role metadata from file"
developer_instructions = "Research carefully"
"#,
    )
    .await?;
    tokio::fs::write(
        standalone_agents_dir.join("reviewer.toml"),
        r#"
name = "reviewer"
description = "Review role"
developer_instructions = "Review carefully"
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(nested_cwd),
            ..Default::default()
        })
        .build()
        .await?;
    assert!(!config.agent_roles.contains_key("researcher"));
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("Review role")
    );
    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("must define a non-empty `name`"))
    );

    Ok(())
}

#[tokio::test]
async fn agent_role_file_name_takes_precedence_over_config_key() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let role_config_path = codex_home.path().join("agents").join("researcher.toml");
    tokio::fs::create_dir_all(
        role_config_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &role_config_path,
        r#"
name = "archivist"
description = "Role metadata from file"
developer_instructions = "Research carefully"
model = "gpt-5.2"
"#,
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
description = "Research role from config"
config_file = "./agents/researcher.toml"
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;
    assert_eq!(config.agent_roles.contains_key("researcher"), false);
    let role = config
        .agent_roles
        .get("archivist")
        .expect("role should use file-provided name");
    assert_eq!(role.description.as_deref(), Some("Role metadata from file"));
    assert_eq!(role.config_file.as_ref(), Some(&role_config_path));

    Ok(())
}

#[tokio::test]
async fn loads_legacy_split_agent_roles_from_config_toml() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let researcher_path = codex_home.path().join("agents").join("researcher.toml");
    let reviewer_path = codex_home.path().join("agents").join("reviewer.toml");
    tokio::fs::create_dir_all(
        researcher_path
            .parent()
            .expect("role config should have a parent directory"),
    )
    .await?;
    tokio::fs::write(
        &researcher_path,
        "developer_instructions = \"Research carefully\"\nmodel = \"gpt-5\"",
    )
    .await?;
    tokio::fs::write(
        &reviewer_path,
        "developer_instructions = \"Review carefully\"\nmodel = \"gpt-4.1\"",
    )
    .await?;
    tokio::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents.researcher]
description = "Research role"
config_file = "./agents/researcher.toml"
nickname_candidates = ["Hypatia", "Noether"]

[agents.reviewer]
description = "Review role"
config_file = "./agents/reviewer.toml"
nickname_candidates = ["Atlas"]
"#,
    )
    .await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.description.as_deref()),
        Some("Research role")
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.config_file.as_ref()),
        Some(&researcher_path)
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Hypatia", "Noether"])
    );
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("Review role")
    );
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.config_file.as_ref()),
        Some(&reviewer_path)
    );
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Atlas"])
    );

    Ok(())
}

#[tokio::test]
async fn discovers_multiple_standalone_agent_role_files() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    let nested_cwd = repo_root.path().join("packages").join("app");
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(&nested_cwd)?;

    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )?;

    let root_agent = repo_root
        .path()
        .join(".codex")
        .join("agents")
        .join("root.toml");
    std::fs::create_dir_all(
        root_agent
            .parent()
            .expect("root agent should have a parent directory"),
    )?;
    std::fs::write(
        &root_agent,
        r#"
name = "researcher"
description = "from root"
developer_instructions = "Research carefully"
"#,
    )?;

    let nested_agent = repo_root
        .path()
        .join("packages")
        .join(".codex")
        .join("agents")
        .join("review")
        .join("nested.toml");
    std::fs::create_dir_all(
        nested_agent
            .parent()
            .expect("nested agent should have a parent directory"),
    )?;
    std::fs::write(
        &nested_agent,
        r#"
name = "reviewer"
description = "from nested"
nickname_candidates = ["Atlas"]
developer_instructions = "Review carefully"
"#,
    )?;

    let sibling_agent = repo_root
        .path()
        .join("packages")
        .join(".codex")
        .join("agents")
        .join("writer.toml");
    std::fs::create_dir_all(
        sibling_agent
            .parent()
            .expect("sibling agent should have a parent directory"),
    )?;
    std::fs::write(
        &sibling_agent,
        r#"
name = "writer"
description = "from sibling"
nickname_candidates = ["Sagan"]
developer_instructions = "Write carefully"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(nested_cwd),
            ..Default::default()
        })
        .build()
        .await?;

    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.description.as_deref()),
        Some("from root")
    );
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("from nested")
    );
    assert_eq!(
        config
            .agent_roles
            .get("reviewer")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Atlas"])
    );
    assert_eq!(
        config
            .agent_roles
            .get("writer")
            .and_then(|role| role.description.as_deref()),
        Some("from sibling")
    );
    assert_eq!(
        config
            .agent_roles
            .get("writer")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Sagan"])
    );

    Ok(())
}

#[tokio::test]
async fn discovers_markdown_agent_role_files_from_agents_dir() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(repo_root.path().join(".codex").join("agents"))?;
    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )?;
    std::fs::write(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("anything.agent.md"),
        r#"---
name: reviewer
description: Reviews code changes.
model: gpt-5.4
effort: high
tools:
  - exec_command
  - apply_patch
skills: "*"
---

Review the code carefully.
"#,
    )?;
    std::fs::write(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("limited.agent.md"),
        r#"---
name: "limited-reviewer" # quoted scalar with trailing comment
description: 'Reviews with limited tools.'
tools: [exec_command, web]
skills: [code-review, "code-review-*"]
---

Review with a restricted capability set.
"#,
    )?;
    std::fs::create_dir_all(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("nested"),
    )?;
    std::fs::write(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("nested")
            .join("nested.md"),
        r#"---
name: nested
description: Nested markdown should be discovered.
---

Nested markdown should load.
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(repo_root.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    let reviewer = config
        .agent_roles
        .get("reviewer")
        .expect("frontmatter name should define agent_type");
    assert_eq!(
        reviewer.description.as_deref(),
        Some("Reviews code changes.")
    );
    assert_eq!(reviewer.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(reviewer.model_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        reviewer.tool_allowlist,
        AgentCapabilityAllowlist::Patterns(vec![
            "exec_command".to_string(),
            "apply_patch".to_string()
        ])
    );
    assert_eq!(reviewer.skill_allowlist, AgentCapabilityAllowlist::All);
    assert_eq!(
        config
            .agent_roles
            .get("nested")
            .and_then(|role| role.description.as_deref()),
        Some("Nested markdown should be discovered.")
    );
    let limited = config
        .agent_roles
        .get("limited-reviewer")
        .expect("quoted frontmatter name should define agent_type");
    assert_eq!(
        limited.description.as_deref(),
        Some("Reviews with limited tools.")
    );
    assert_eq!(
        limited.tool_allowlist,
        AgentCapabilityAllowlist::Patterns(vec!["exec_command".to_string(), "web".to_string()])
    );
    assert_eq!(
        limited.skill_allowlist,
        AgentCapabilityAllowlist::Patterns(vec![
            "code-review".to_string(),
            "code-review-*".to_string()
        ])
    );

    Ok(())
}

#[tokio::test]
async fn discovered_markdown_agent_defaults_optional_frontmatter_fields() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(repo_root.path().join(".codex").join("agents"))?;
    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )?;
    std::fs::write(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("optimizer.md"),
        r#"---
description: Optimizes workflows.
level: project
parallelizable: true
schema_version: 1
---

Optimize the workflow carefully.
"#,
    )?;
    std::fs::write(
        repo_root
            .path()
            .join(".codex")
            .join("agents")
            .join("hidden.md"),
        r#"---
level: project
---

Missing descriptions are still invalid.
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(repo_root.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    let optimizer = config
        .agent_roles
        .get("optimizer")
        .expect("file stem should define the agent role name");
    assert_eq!(
        optimizer.description.as_deref(),
        Some("Optimizes workflows.")
    );
    assert_eq!(optimizer.tool_allowlist, AgentCapabilityAllowlist::All);
    assert_eq!(optimizer.skill_allowlist, AgentCapabilityAllowlist::All);
    assert!(!config.agent_roles.contains_key("hidden"));
    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("agent role `hidden` must define a description")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn declared_markdown_agent_requires_frontmatter_description() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    let agent_file = repo_root.path().join("reviewer.md");
    std::fs::write(
        &agent_file,
        r#"---
name: reviewer
---

Review the code carefully.
"#,
    )?;
    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    let agent_file = agent_file.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"

[agents.reviewer]
description = "Config description must not satisfy Markdown frontmatter"
config_file = "{agent_file}"
"#
        ),
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(repo_root.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    assert!(!config.agent_roles.contains_key("reviewer"));
    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("agent role `reviewer` must define a description")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn ignores_non_directory_agents_discovery_paths() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let repo_root = TempDir::new()?;
    std::fs::create_dir_all(repo_root.path().join(".git"))?;
    std::fs::create_dir_all(repo_root.path().join(".codex"))?;
    std::fs::write(codex_home.path().join("agents"), "not a directory")?;
    std::fs::write(
        repo_root.path().join(".codex").join("agents"),
        "not a directory",
    )?;
    let workspace_key = repo_root.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[projects."{workspace_key}"]
trust_level = "trusted"
"#
        ),
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(repo_root.path().to_path_buf()),
            ..Default::default()
        })
        .build()
        .await?;

    assert!(config.agent_roles.is_empty());

    Ok(())
}
