use super::*;

async fn config_loads_mcp_oauth_callback_url_from_toml() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let toml = r#"
model = "gpt-5.4"
mcp_oauth_callback_url = "https://example.com/callback"
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for callback URL");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.mcp_oauth_callback_url.as_deref(),
        Some("https://example.com/callback")
    );
    Ok(())
}

#[tokio::test]
async fn test_untrusted_project_gets_unless_trusted_approval_policy() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let test_project_dir = TempDir::new()?;
    let test_path = test_project_dir.path();

    let config = Config::load_from_base_config_with_overrides(
        ConfigToml {
            projects: Some(HashMap::from([(
                test_path.to_string_lossy().to_string(),
                ProjectConfig {
                    trust_level: Some(TrustLevel::Untrusted),
                },
            )])),
            ..Default::default()
        },
        ConfigOverrides {
            cwd: Some(test_path.to_path_buf()),
            ..Default::default()
        },
        codex_home.abs(),
    )
    .await?;

    // Verify that untrusted projects get UnlessTrusted approval policy
    assert_eq!(
        config.permissions.approval_policy.value(),
        AskForApproval::UnlessTrusted,
        "Expected UnlessTrusted approval policy for untrusted project"
    );

    // Verify that untrusted projects still get WorkspaceWrite sandbox (or ReadOnly on Windows)
    if cfg!(target_os = "windows") {
        assert!(
            matches!(
                &config.legacy_sandbox_policy(),
                SandboxPolicy::ReadOnly { .. }
            ),
            "Expected ReadOnly on Windows"
        );
    } else {
        assert!(
            matches!(
                &config.legacy_sandbox_policy(),
                SandboxPolicy::WorkspaceWrite { .. }
            ),
            "Expected WorkspaceWrite sandbox for untrusted project"
        );
    }

    Ok(())
}

#[tokio::test]
async fn requirements_disallowing_default_sandbox_falls_back_to_required_default()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_sandbox_modes: Some(vec![config_service::SandboxModeRequirement::ReadOnly]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;
    assert_eq!(
        config.legacy_sandbox_policy(),
        SandboxPolicy::new_read_only_policy()
    );
    Ok(())
}

#[tokio::test]
async fn explicit_sandbox_mode_falls_back_when_disallowed_by_requirements() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"sandbox_mode = "danger-full-access"
"#,
    )?;

    let requirements = config_service::ConfigRequirementsToml {
        allowed_approval_policies: None,
        allowed_approvals_reviewers: None,
        allowed_sandbox_modes: Some(vec![config_service::SandboxModeRequirement::ReadOnly]),
        remote_sandbox_config: None,
        allowed_web_search_modes: None,
        allow_managed_hooks_only: None,
        feature_requirements: None,
        hooks: None,
        mcp_servers: None,
        plugins: None,
        apps: None,
        rules: None,
        enforce_residency: None,
        network: None,
        permissions: None,
        guardian_policy_config: None,
    };

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await?;
    assert_eq!(
        config.legacy_sandbox_policy(),
        SandboxPolicy::new_read_only_policy()
    );
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_falls_back_when_disallowed_by_requirements()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let requirements = config_service::ConfigRequirementsToml {
        allowed_sandbox_modes: Some(vec![config_service::SandboxModeRequirement::ReadOnly]),
        ..Default::default()
    };

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .harness_overrides(ConfigOverrides {
            permission_profile: Some(PermissionProfile::Disabled),
            ..Default::default()
        })
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await?;

    let expected_sandbox_policy = SandboxPolicy::new_read_only_policy();
    assert_eq!(config.legacy_sandbox_policy(), expected_sandbox_policy);
    assert_eq!(
        config.permissions.effective_permission_profile(),
        PermissionProfile::read_only()
    );
    Ok(())
}

#[tokio::test]
async fn active_profile_is_cleared_when_requirements_force_fallback() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let requirements = config_service::ConfigRequirementsToml {
        allowed_sandbox_modes: Some(vec![config_service::SandboxModeRequirement::ReadOnly]),
        ..Default::default()
    };

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .harness_overrides(ConfigOverrides {
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS.to_string()),
            ..Default::default()
        })
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await?;

    assert_eq!(
        config.permissions.effective_permission_profile(),
        PermissionProfile::read_only()
    );
    assert_eq!(config.permissions.active_permission_profile(), None);
    assert!(
        config.startup_warnings.iter().any(|warning| warning
            .contains("Configured value for `permission_profile` is disallowed by requirements")),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn bypass_hook_trust_adds_startup_warning() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .harness_overrides(ConfigOverrides {
            bypass_hook_trust: Some(true),
            ..Default::default()
        })
        .build()
        .await?;

    assert!(
        config.startup_warnings.iter().any(|warning| warning
            == "`--dangerously-bypass-hook-trust` is enabled. Enabled hooks may run without review for this invocation."),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn permission_profile_override_preserves_split_write_roots() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cwd = codex_home.path().join("workspace");
    let outside_root = codex_home.path().join("outside-write");
    std::fs::create_dir_all(&cwd)?;
    std::fs::create_dir_all(&outside_root)?;
    let outside_root =
        AbsolutePathBuf::from_absolute_path(outside_root).expect("outside root is absolute");
    let file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: outside_root.clone(),
            },
            access: FileSystemAccessMode::Write,
        },
    ]);
    let permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        SandboxEnforcement::Managed,
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(cwd))
        .harness_overrides(ConfigOverrides {
            permission_profile: Some(permission_profile),
            ..Default::default()
        })
        .build()
        .await?;

    assert!(
        config
            .permissions
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(outside_root.as_path(), config.cwd.as_path())
    );
    assert!(matches!(
        &config.legacy_sandbox_policy(),
        SandboxPolicy::WorkspaceWrite { .. }
    ));
    assert_eq!(
        config.permissions.network_sandbox_policy(),
        NetworkSandboxPolicy::Restricted
    );
    Ok(())
}

#[tokio::test]
async fn requirements_web_search_mode_overrides_danger_full_access_default() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"sandbox_mode = "danger-full-access"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_web_search_modes: Some(vec![
                    config_service::WebSearchModeRequirement::Cached,
                ]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(config.web_search_mode.value(), WebSearchMode::Cached);
    assert_eq!(
        resolve_web_search_mode_for_turn(
            &config.web_search_mode,
            &config.permissions.effective_permission_profile(),
        ),
        WebSearchMode::Cached,
    );
    Ok(())
}

#[tokio::test]
async fn requirements_disallowing_default_approval_falls_back_to_required_default()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let workspace = TempDir::new()?;
    let workspace_key = workspace.path().to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"
[projects."{workspace_key}"]
trust_level = "untrusted"
"#
        ),
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(workspace.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approval_policies: Some(vec![AskForApproval::OnRequest]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(
        config.permissions.approval_policy.value(),
        AskForApproval::OnRequest
    );
    Ok(())
}

#[tokio::test]
async fn explicit_approval_policy_falls_back_when_disallowed_by_requirements() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"approval_policy = "untrusted"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approval_policies: Some(vec![AskForApproval::OnRequest]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;
    assert_eq!(
        config.permissions.approval_policy.value(),
        AskForApproval::OnRequest
    );
    Ok(())
}

#[tokio::test]
async fn feature_requirements_normalize_effective_feature_values() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
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

    assert!(config.features.enabled(Feature::Personality));
    assert!(!config.features.enabled(Feature::ShellTool));
    assert!(
        !config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("Configured value for `features`")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn feature_requirements_auto_review_disables_guardian_approval() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                feature_requirements: Some(config_service::FeatureRequirementsToml {
                    entries: BTreeMap::from([("auto_review".to_string(), false)]),
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert!(!config.features.enabled(Feature::GuardianApproval));

    Ok(())
}

#[tokio::test]
async fn browser_feature_requirements_are_valid() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                feature_requirements: Some(config_service::FeatureRequirementsToml {
                    entries: BTreeMap::from([
                        ("in_app_browser".to_string(), false),
                        ("browser_use".to_string(), false),
                    ]),
                }),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert!(!config.features.enabled(Feature::InAppBrowser));
    assert!(!config.features.enabled(Feature::BrowserUse));

    Ok(())
}

#[tokio::test]
async fn debug_config_lockfile_export_settings_load_from_nested_table() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[debug.config_lockfile]
export_dir = "locks"
allow_codex_version_mismatch = true
save_fields_resolved_from_model_catalog = false
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(
        config.config_lock_export_dir,
        Some(AbsolutePathBuf::resolve_path_against_base(
            "locks",
            codex_home.path()
        ))
    );
    assert!(config.config_lock_allow_codex_version_mismatch);
    assert!(!config.config_lock_save_fields_resolved_from_model_catalog);

    Ok(())
}

#[tokio::test]
async fn debug_config_lockfile_load_path_loads_lock_from_nested_table() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let lock_path = codex_home.path().join("session.config.lock.toml");
    std::fs::write(
        &lock_path,
        format!(
            r#"version = {}
codex_version = "older-version"

[config]
"#,
            codex_config_toml::CONFIG_LOCK_VERSION
        ),
    )?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        format!(
            r#"[debug.config_lockfile]
load_path = '{}'
allow_codex_version_mismatch = true
save_fields_resolved_from_model_catalog = false
"#,
            lock_path.display()
        ),
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert!(config.config_lock_toml.is_some());
    assert!(config.config_lock_allow_codex_version_mismatch);
    assert!(!config.config_lock_save_fields_resolved_from_model_catalog);

    Ok(())
}

#[tokio::test]
async fn explicit_feature_config_is_normalized_by_requirements() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[features]
personality = false
shell_tool = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
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

    assert!(config.features.enabled(Feature::Personality));
    assert!(!config.features.enabled(Feature::ShellTool));
    assert!(
        !config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("Configured value for `features`")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[tokio::test]
async fn approvals_reviewer_defaults_to_manual_only_without_guardian_feature() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::User);
    Ok(())
}

#[tokio::test]
async fn prompt_instruction_blocks_can_be_disabled_from_config_and_profiles() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"include_permissions_instructions = false
include_apps_instructions = false
include_collaboration_mode_instructions = false
include_environment_context = false
profile = "chatty"

[skills]
include_instructions = false

[profiles.chatty]
include_permissions_instructions = true
include_collaboration_mode_instructions = true
include_environment_context = true
"#,
    )?;

    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert!(config.include_permissions_instructions);
    assert!(!config.include_apps_instructions);
    assert!(config.include_collaboration_mode_instructions);
    assert!(!config.include_skill_instructions);
    assert!(config.include_environment_context);
    Ok(())
}

#[tokio::test]
async fn approvals_reviewer_stays_manual_only_when_guardian_feature_is_enabled()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features]
guardian_approval = true
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::User);
    Ok(())
}

#[tokio::test]
async fn approvals_reviewer_can_be_set_in_config_without_guardian_approval() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"approvals_reviewer = "user"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::User);
    Ok(())
}

#[tokio::test]
async fn approvals_reviewer_can_be_set_in_profile_without_guardian_approval() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"profile = "guardian"

[profiles.guardian]
approvals_reviewer = "guardian_subagent"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    Ok(())
}

#[tokio::test]
async fn requirements_disallowing_default_approvals_reviewer_falls_back_to_required_default()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approvals_reviewers: Some(vec![ApprovalsReviewer::AutoReview]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    Ok(())
}

#[tokio::test]
async fn root_approvals_reviewer_falls_back_when_disallowed_by_requirements() -> std::io::Result<()>
{
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"approvals_reviewer = "user"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approvals_reviewers: Some(vec![ApprovalsReviewer::AutoReview]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning
                .contains("Configured value for `approvals_reviewer` is disallowed by requirements")
        }),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn profile_approvals_reviewer_falls_back_when_disallowed_by_requirements()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"profile = "default"

[profiles.default]
approvals_reviewer = "user"
"#,
    )?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async {
            Ok(Some(config_service::ConfigRequirementsToml {
                allowed_approvals_reviewers: Some(vec![ApprovalsReviewer::AutoReview]),
                ..Default::default()
            }))
        }))
        .build()
        .await?;

    assert_eq!(config.approvals_reviewer, ApprovalsReviewer::AutoReview);
    Ok(())
}

