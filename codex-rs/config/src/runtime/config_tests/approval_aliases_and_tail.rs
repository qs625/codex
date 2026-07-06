use super::*;

#[tokio::test]
async fn approvals_reviewer_preserves_valid_user_choice_when_allowed_by_requirements()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"approvals_reviewer = "guardian_subagent"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approvals_reviewers: Some(vec![
                    ApprovalsReviewer::User,
                    ApprovalsReviewer::AutoReview,
                ]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    assert!(
        config
            .startup_warnings
            .iter()
            .all(|warning| !warning.contains("approvals_reviewer")),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn smart_approvals_alias_is_ignored() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features]
smart_approvals = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert!(config.features.enabled(Feature::GuardianApproval));
    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::User);

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    assert!(serialized.contains("smart_approvals = true"));
    assert!(!serialized.contains("guardian_approval"));
    assert!(!serialized.contains("approvals_reviewer"));

    Ok(())
}

#[tokio::test]
async fn smart_approvals_alias_is_ignored_in_profiles() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"profile = "guardian"

[profiles.guardian.features]
smart_approvals = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert!(config.features.enabled(Feature::GuardianApproval));
    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::User);

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    assert!(serialized.contains("[profiles.guardian.features]"));
    assert!(serialized.contains("smart_approvals = true"));
    assert!(!serialized.contains("guardian_approval"));
    assert!(!serialized.contains("approvals_reviewer"));

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_config_from_feature_table() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
max_concurrent_threads_per_session = 5
min_wait_timeout_ms = 2500
max_wait_timeout_ms = 120000
default_wait_timeout_ms = 30000
usage_hint_enabled = false
usage_hint_text = "Custom delegation guidance."
root_agent_usage_hint_text = "Root guidance."
subagent_usage_hint_text = "Subagent guidance."
hide_spawn_agent_metadata = true
non_code_mode_only = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert!(config.features.enabled(Feature::MultiAgentV2));
    assert_eq!(config.multi_agent_v2.max_concurrent_threads_per_session, 5);
    assert_eq!(config.multi_agent_v2.min_wait_timeout_ms, 2500);
    assert_eq!(config.multi_agent_v2.max_wait_timeout_ms, 120000);
    assert_eq!(config.multi_agent_v2.default_wait_timeout_ms, 30000);
    assert_eq!(config.agent_max_threads, Some(4));
    assert!(!config.multi_agent_v2.usage_hint_enabled);
    assert_eq!(
        config.multi_agent_v2.usage_hint_text.as_deref(),
        Some("Custom delegation guidance.")
    );
    assert_eq!(
        config.multi_agent_v2.root_agent_usage_hint_text.as_deref(),
        Some("Root guidance.")
    );
    assert_eq!(
        config.multi_agent_v2.subagent_usage_hint_text.as_deref(),
        Some("Subagent guidance.")
    );
    assert!(config.multi_agent_v2.hide_spawn_agent_metadata);
    assert!(config.multi_agent_v2.non_code_mode_only);

    Ok(())
}

#[tokio::test]
async fn profile_multi_agent_v2_config_overrides_base() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"profile = "no_hint"

[features.multi_agent_v2]
max_concurrent_threads_per_session = 4
min_wait_timeout_ms = 3000
max_wait_timeout_ms = 120000
default_wait_timeout_ms = 30000
usage_hint_enabled = true
usage_hint_text = "base hint"
root_agent_usage_hint_text = "base root hint"
subagent_usage_hint_text = "base subagent hint"
hide_spawn_agent_metadata = true
non_code_mode_only = false

[profiles.no_hint.features.multi_agent_v2]
max_concurrent_threads_per_session = 6
min_wait_timeout_ms = 1500
max_wait_timeout_ms = 90000
default_wait_timeout_ms = 15000
usage_hint_enabled = false
usage_hint_text = "profile hint"
root_agent_usage_hint_text = "profile root hint"
subagent_usage_hint_text = "profile subagent hint"
hide_spawn_agent_metadata = false
non_code_mode_only = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.multi_agent_v2.max_concurrent_threads_per_session, 6);
    assert_eq!(config.multi_agent_v2.min_wait_timeout_ms, 1500);
    assert_eq!(config.multi_agent_v2.max_wait_timeout_ms, 90000);
    assert_eq!(config.multi_agent_v2.default_wait_timeout_ms, 15000);
    assert!(!config.multi_agent_v2.usage_hint_enabled);
    assert_eq!(
        config.multi_agent_v2.usage_hint_text.as_deref(),
        Some("profile hint")
    );
    assert_eq!(
        config.multi_agent_v2.root_agent_usage_hint_text.as_deref(),
        Some("profile root hint")
    );
    assert_eq!(
        config.multi_agent_v2.subagent_usage_hint_text.as_deref(),
        Some("profile subagent hint")
    );
    assert!(!config.multi_agent_v2.hide_spawn_agent_metadata);
    assert!(config.multi_agent_v2.non_code_mode_only);

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_default_session_thread_cap_counts_root() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.multi_agent_v2.max_concurrent_threads_per_session, 4);
    assert_eq!(config.multi_agent_v2.min_wait_timeout_ms, 10_000);
    assert_eq!(config.multi_agent_v2.max_wait_timeout_ms, 1_800_000);
    assert_eq!(config.multi_agent_v2.default_wait_timeout_ms, 60_000);
    assert_eq!(config.agent_max_threads, Some(3));
    assert!(!config.multi_agent_v2.non_code_mode_only);

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_accepts_agents_max_threads_as_compat_alias() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true

[agents]
max_threads = 3
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.agent_max_threads, Some(3));
    assert_eq!(config.multi_agent_v2.max_concurrent_threads_per_session, 4);

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_rejects_zero_agents_max_threads_alias() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[agents]
max_threads = 0
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("zero agents.max_threads should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(err.to_string(), "agents.max_threads must be at least 1");

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_rejects_invalid_wait_timeouts() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = 0
max_wait_timeout_ms = 0
default_wait_timeout_ms = 0
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.multi_agent_v2.min_wait_timeout_ms, 0);
    assert_eq!(config.multi_agent_v2.max_wait_timeout_ms, 0);
    assert_eq!(config.multi_agent_v2.default_wait_timeout_ms, 0);

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = -1
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("negative min_wait_timeout_ms should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.min_wait_timeout_ms must be at least 0"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = 1800001
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("too large min_wait_timeout_ms should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.min_wait_timeout_ms must be at most 1800000"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
max_wait_timeout_ms = -1
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("negative max_wait_timeout_ms should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.max_wait_timeout_ms must be at least 0"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
max_wait_timeout_ms = 1800001
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("too large max_wait_timeout_ms should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.max_wait_timeout_ms must be at most 1800000"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
default_wait_timeout_ms = -1
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("negative default_wait_timeout_ms should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.default_wait_timeout_ms must be at least 0"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = 1000
max_wait_timeout_ms = 500
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("min greater than max should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.min_wait_timeout_ms must be at most features.multi_agent_v2.max_wait_timeout_ms"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = 1000
max_wait_timeout_ms = 2000
default_wait_timeout_ms = 500
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("default less than min should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.default_wait_timeout_ms must be at least features.multi_agent_v2.min_wait_timeout_ms"
    );

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
min_wait_timeout_ms = 1000
max_wait_timeout_ms = 2000
default_wait_timeout_ms = 2500
"#,
    )?;

    let err = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect_err("default greater than max should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        err.to_string(),
        "features.multi_agent_v2.default_wait_timeout_ms must be at most features.multi_agent_v2.max_wait_timeout_ms"
    );

    Ok(())
}

#[tokio::test]
async fn multi_agent_v2_session_thread_cap_one_disallows_subagents() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features.multi_agent_v2]
enabled = true
max_concurrent_threads_per_session = 1
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.multi_agent_v2.max_concurrent_threads_per_session, 1);
    assert_eq!(config.agent_max_threads, Some(0));

    Ok(())
}

#[tokio::test]
async fn feature_requirements_normalize_runtime_feature_mutations() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                feature_requirements: Some(config_service::FeatureRequirementsToml {
                    entries: BTreeMap::from([
                        ("personality".to_string(), true),
                        ("shell_tool".to_string(), false),
                    ]),
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    let mut requested = config.features.get().clone();
    requested
        .disable(Feature::Personality)
        .enable(Feature::ShellTool);
    assert!(config.features.can_set(&requested).is_ok());
    config
        .features
        .set(requested)
        .expect("managed feature mutations should normalize successfully");

    assert!(config.features.enabled(Feature::Personality));
    assert!(!config.features.enabled(Feature::ShellTool));

    Ok(())
}

#[tokio::test]
async fn feature_requirements_warn_on_collab_legacy_alias() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                feature_requirements: Some(config_service::FeatureRequirementsToml {
                    entries: BTreeMap::from([("collab".to_string(), true)]),
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert!(config.features.enabled(Feature::Collab));
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning.contains("Using legacy `features` requirement `collab`")
                && warning.contains("prefer canonical feature key `multi_agent`")
        }),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn feature_requirements_warn_and_ignore_unknown_feature() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                feature_requirements: Some(config_service::FeatureRequirementsToml {
                    entries: BTreeMap::from([("made_up_feature".to_string(), true)]),
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert!(
        config
            .startup_warnings
            .iter()
            .any(|warning| warning
                .contains("Ignoring unknown `features` requirement `made_up_feature`")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn tool_suggest_discoverables_load_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
[tool_suggest]
discoverables = [
  { type = "connector", id = "connector_alpha" },
  { type = "plugin", id = "plugin_alpha@openai-curated" },
  { type = "connector", id = "   " }
]
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.tool_suggest,
        Some(ToolSuggestConfig {
            discoverables: vec![
                ToolSuggestDiscoverable {
                    kind: ToolSuggestDiscoverableType::Connector,
                    id: "connector_alpha".to_string(),
                },
                ToolSuggestDiscoverable {
                    kind: ToolSuggestDiscoverableType::Plugin,
                    id: "plugin_alpha@openai-curated".to_string(),
                },
                ToolSuggestDiscoverable {
                    kind: ToolSuggestDiscoverableType::Connector,
                    id: "   ".to_string(),
                },
            ],
            disabled_tools: Vec::new(),
        })
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.tool_suggest,
        ToolSuggestConfig {
            discoverables: vec![
                ToolSuggestDiscoverable {
                    kind: ToolSuggestDiscoverableType::Connector,
                    id: "connector_alpha".to_string(),
                },
                ToolSuggestDiscoverable {
                    kind: ToolSuggestDiscoverableType::Plugin,
                    id: "plugin_alpha@openai-curated".to_string(),
                },
            ],
            disabled_tools: Vec::new(),
        }
    );
    Ok(())
}

#[tokio::test]
async fn tool_suggest_disabled_tools_load_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
[tool_suggest]
disabled_tools = [
  { type = "connector", id = " connector_calendar " },
  { type = "connector", id = "connector_calendar" },
  { type = "connector", id = "   " },
  { type = "plugin", id = "slack@openai-curated" }
]
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.tool_suggest,
        Some(ToolSuggestConfig {
            discoverables: Vec::new(),
            disabled_tools: vec![
                ToolSuggestDisabledTool::connector(" connector_calendar "),
                ToolSuggestDisabledTool::connector("connector_calendar"),
                ToolSuggestDisabledTool::connector("   "),
                ToolSuggestDisabledTool::plugin("slack@openai-curated"),
            ],
        })
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.tool_suggest,
        ToolSuggestConfig {
            discoverables: Vec::new(),
            disabled_tools: vec![
                ToolSuggestDisabledTool::connector("connector_calendar"),
                ToolSuggestDisabledTool::plugin("slack@openai-curated"),
            ],
        }
    );
    Ok(())
}

#[tokio::test]
async fn tool_suggest_disabled_tools_merge_across_config_layers() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = TempDir::new()?;
    let workspace_key = workspace.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"
[projects."{workspace_key}"]
trust_level = "trusted"

[tool_suggest]
disabled_tools = [
  {{ type = "connector", id = " user_connector " }},
  {{ type = "plugin", id = "shared_plugin" }},
  {{ type = "connector", id = "project_connector" }},
]
"#
        ),
    )?;

    let project_config_dir = workspace.path().join(".codex");
    std::fs::create_dir_all(&project_config_dir)?;
    std::fs::write(
        project_config_dir.join(CONFIG_TOML_FILE),
        r#"
[tool_suggest]
disabled_tools = [
  { type = "connector", id = "project_connector" },
  { type = "plugin", id = "project_plugin" },
  { type = "plugin", id = "shared_plugin" },
]
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

    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("user_connector"),
            ToolSuggestDisabledTool::plugin("shared_plugin"),
            ToolSuggestDisabledTool::connector("project_connector"),
            ToolSuggestDisabledTool::plugin("project_plugin"),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn experimental_realtime_start_instructions_load_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_realtime_start_instructions = "start instructions from config"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_realtime_start_instructions.as_deref(),
        Some("start instructions from config")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_realtime_start_instructions.as_deref(),
        Some("start instructions from config")
    );
    Ok(())
}

#[tokio::test]
async fn experimental_thread_config_endpoint_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_thread_config_endpoint = "http://127.0.0.1:8061"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_thread_config_endpoint.as_deref(),
        Some("http://127.0.0.1:8061")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_thread_config_endpoint.as_deref(),
        Some("http://127.0.0.1:8061")
    );
    Ok(())
}

#[tokio::test]
async fn experimental_realtime_ws_base_url_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_realtime_ws_base_url = "http://127.0.0.1:8011"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_realtime_ws_base_url.as_deref(),
        Some("http://127.0.0.1:8011")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_realtime_ws_base_url.as_deref(),
        Some("http://127.0.0.1:8011")
    );
    Ok(())
}

#[tokio::test]
async fn experimental_realtime_ws_backend_prompt_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_realtime_ws_backend_prompt = "prompt from config"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_realtime_ws_backend_prompt.as_deref(),
        Some("prompt from config")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_realtime_ws_backend_prompt.as_deref(),
        Some("prompt from config")
    );
    Ok(())
}

#[tokio::test]
async fn experimental_realtime_ws_startup_context_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_realtime_ws_startup_context = "startup context from config"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_realtime_ws_startup_context.as_deref(),
        Some("startup context from config")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_realtime_ws_startup_context.as_deref(),
        Some("startup context from config")
    );
    Ok(())
}

#[tokio::test]
async fn experimental_realtime_ws_model_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
experimental_realtime_ws_model = "realtime-test-model"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.experimental_realtime_ws_model.as_deref(),
        Some("realtime-test-model")
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.experimental_realtime_ws_model.as_deref(),
        Some("realtime-test-model")
    );
    Ok(())
}

#[tokio::test]
async fn realtime_config_partial_table_uses_realtime_defaults() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
[realtime]
voice = "marin"
"#,
    )
    .expect("TOML deserialization should succeed");

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.realtime,
        RealtimeConfig {
            voice: Some(RealtimeVoice::Marin),
            ..RealtimeConfig::default()
        }
    );
    Ok(())
}

#[tokio::test]
async fn realtime_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
[realtime]
version = "v2"
type = "transcription"
transport = "webrtc"
voice = "cedar"
"#,
    )
    .expect("TOML deserialization should succeed");

    assert_eq!(
        cfg.realtime,
        Some(RealtimeToml {
            version: Some(RealtimeWsVersion::V2),
            session_type: Some(RealtimeWsMode::Transcription),
            transport: Some(RealtimeTransport::WebRtc),
            voice: Some(RealtimeVoice::Cedar),
        })
    );

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.realtime,
        RealtimeConfig {
            version: RealtimeWsVersion::V2,
            session_type: RealtimeWsMode::Transcription,
            transport: RealtimeTransport::WebRtc,
            voice: Some(RealtimeVoice::Cedar),
        }
    );
    Ok(())
}

#[tokio::test]
async fn realtime_audio_loads_from_config_toml() -> std::io::Result<()> {
    let cfg: ConfigToml = toml::from_str(
        r#"
[audio]
microphone = "USB Mic"
speaker = "Desk Speakers"
"#,
    )
    .expect("TOML deserialization should succeed");

    let realtime_audio = cfg
        .audio
        .as_ref()
        .expect("realtime audio config should be present");
    assert_eq!(realtime_audio.microphone.as_deref(), Some("USB Mic"));
    assert_eq!(realtime_audio.speaker.as_deref(), Some("Desk Speakers"));

    let codex_home = TempDir::new()?;
    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.realtime_audio.microphone.as_deref(), Some("USB Mic"));
    assert_eq!(
        config.realtime_audio.speaker.as_deref(),
        Some("Desk Speakers")
    );
    Ok(())
}

#[derive(Deserialize, Debug, PartialEq)]
struct TuiTomlTest {
    #[serde(default, flatten)]
    notifications: TuiNotificationSettings,
}

#[derive(Deserialize, Debug, PartialEq)]
struct RootTomlTest {
    tui: TuiTomlTest,
}

#[test]
fn test_tui_notifications_true() {
    let toml = r#"
            [tui]
            notifications = true
        "#;
    let parsed: RootTomlTest = toml::from_str(toml).expect("deserialize notifications=true");
    assert_matches!(
        parsed.tui.notifications.notifications,
        Notifications::Enabled(true)
    );
}

#[test]
fn test_tui_notifications_custom_array() {
    let toml = r#"
            [tui]
            notifications = ["foo"]
        "#;
    let parsed: RootTomlTest = toml::from_str(toml).expect("deserialize notifications=[\"foo\"]");
    assert_matches!(
        parsed.tui.notifications.notifications,
        Notifications::Custom(ref v) if v == &vec!["foo".to_string()]
    );
}

#[test]
fn test_tui_notification_method() {
    let toml = r#"
            [tui]
            notification_method = "bel"
        "#;
    let parsed: RootTomlTest =
        toml::from_str(toml).expect("deserialize notification_method=\"bel\"");
    assert_eq!(parsed.tui.notifications.method, NotificationMethod::Bel);
}

#[test]
fn test_tui_notification_condition_defaults_to_unfocused() {
    let toml = r#"
            [tui]
        "#;
    let parsed: RootTomlTest =
        toml::from_str(toml).expect("deserialize default notification condition");
    assert_eq!(
        parsed.tui.notifications.condition,
        NotificationCondition::Unfocused
    );
}

#[test]
fn test_tui_notification_condition_always() {
    let toml = r#"
            [tui]
            notification_condition = "always"
        "#;
    let parsed: RootTomlTest =
        toml::from_str(toml).expect("deserialize notification_condition=\"always\"");
    assert_eq!(
        parsed.tui.notifications.condition,
        NotificationCondition::Always
    );
}

#[test]
fn test_tui_notification_condition_rejects_unknown_value() {
    let toml = r#"
            [tui]
            notification_condition = "background"
        "#;
    let err = toml::from_str::<RootTomlTest>(toml).expect_err("reject unknown condition");
    let err = err.to_string();
    assert!(
        err.contains("unknown variant `background`")
            && err.contains("unfocused")
            && err.contains("always"),
        "unexpected error: {err}"
    );
}
