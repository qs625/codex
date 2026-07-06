use super::*;

use super::*;
async fn test_precedence_fixture_with_gpt5_profile() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let gpt5_profile_overrides = ConfigOverrides {
        config_profile: Some("gpt5".to_string()),
        cwd: Some(fixture.cwd_path()),
        ..Default::default()
    };
    let gpt5_profile_config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        gpt5_profile_overrides,
        fixture.codex_home(),
    )
    .await?;
    let expected_gpt5_profile_config = Config {
        model: Some("gpt-5.4".to_string()),
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        service_tier: None,
        model_provider_id: "openai".to_string(),
        model_provider: fixture.openai_provider.clone(),
        model_options: Vec::new(),
        permissions: Permissions {
            approval_policy: Constrained::allow_any(AskForApproval::OnFailure),
            permission_profile_state: active_permission_profile_state(
                PermissionProfile::read_only(),
                BUILT_IN_PERMISSION_PROFILE_READ_ONLY,
            ),
            workspace_roots: vec![fixture.cwd()],
            network: None,
            allow_login_shell: true,
            shell_environment_policy: ShellEnvironmentPolicy::default(),
            windows_sandbox_mode: None,
            windows_sandbox_private_desktop: true,
        },
        approvals_reviewer: ApprovalsReviewer::User,
        enforce_residency: Constrained::allow_any(/*initial_value*/ None),
        user_instructions: None,
        notify: None,
        cwd: fixture.cwd(),
        workspace_roots: vec![fixture.cwd()],
        workspace_roots_explicit: false,
        cli_auth_credentials_store_mode: Default::default(),
        mcp_servers: Constrained::allow_any(HashMap::new()),
        mcp_oauth_credentials_store_mode: resolve_mcp_oauth_credentials_store_mode(
            Default::default(),
            LOCAL_DEV_BUILD_VERSION,
        ),
        mcp_oauth_callback_port: None,
        mcp_oauth_callback_url: None,
        model_providers: fixture.model_provider_map.clone(),
        project_doc_max_bytes: AGENTS_MD_MAX_BYTES,
        project_doc_fallback_filenames: Vec::new(),
        tool_output_token_limit: None,
        agent_max_threads: Some(DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION - 1),
        agent_max_depth: DEFAULT_AGENT_MAX_DEPTH,
        agent_roles: BTreeMap::new(),
        agent_tool_patterns: None,
        agent_skill_patterns: None,
        memories: MemoriesConfig::default(),
        agent_job_max_runtime_seconds: DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS,
        agent_interrupt_message_enabled: true,
        codex_home: fixture.codex_home(),
        sqlite_home: fixture.codex_home().to_path_buf(),
        log_dir: fixture.codex_home().join("log").to_path_buf(),
        config_lock_export_dir: None,
        config_lock_allow_codex_version_mismatch: false,
        config_lock_save_fields_resolved_from_model_catalog: true,
        config_lock_toml: None,
        config_layer_stack: Default::default(),
        startup_warnings: Vec::new(),
        history: History::default(),
        ephemeral: false,
        bypass_hook_trust: false,
        file_opener: UriBasedFileOpener::VsCode,
        codex_self_exe: None,
        codex_linux_sandbox_exe: None,
        main_execve_wrapper_exe: None,
        zsh_path: None,
        hide_agent_reasoning: false,
        show_raw_agent_reasoning: false,
        model_reasoning_effort: Some(ReasoningEffort::High),
        plan_mode_reasoning_effort: None,
        model_reasoning_summary: Some(ReasoningSummary::Detailed),
        model_supports_reasoning_summaries: None,
        model_catalog: None,
        model_verbosity: Some(Verbosity::High),
        personality: Some(Personality::Pragmatic),
        chatgpt_base_url: "https://chatgpt.com/backend-api/".to_string(),
        apps_mcp_path_override: None,
        realtime_audio: RealtimeAudioConfig::default(),
        experimental_realtime_start_instructions: None,
        experimental_realtime_ws_base_url: None,
        experimental_realtime_ws_model: None,
        realtime: RealtimeConfig::default(),
        experimental_realtime_ws_backend_prompt: None,
        experimental_realtime_ws_startup_context: None,
        experimental_thread_config_endpoint: None,
        experimental_thread_store: ThreadStoreConfig::Local,
        base_instructions: None,
        developer_instructions: None,
        guardian_policy_config: None,
        include_permissions_instructions: true,
        include_apps_instructions: true,
        include_collaboration_mode_instructions: true,
        include_skill_instructions: true,
        include_environment_context: true,
        compact_prompt: None,
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search_mode: Constrained::allow_any(WebSearchMode::Cached),
        web_search_config: None,
        use_experimental_unified_exec_tool: !cfg!(windows),
        background_terminal_max_timeout: DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS,
        ghost_snapshot: GhostSnapshotConfig::default(),
        multi_agent_v2: MultiAgentV2Config::default(),
        features: Features::with_defaults().into(),
        suppress_unstable_features_warning: false,
        active_profile: Some("gpt5".to_string()),
        active_project: ProjectConfig { trust_level: None },
        notices: Default::default(),
        check_for_update_on_startup: true,
        disable_paste_burst: false,
        tui_notifications: Default::default(),
        animations: true,
        show_tooltips: true,
        tui_vim_mode_default: false,
        tui_raw_output_mode: false,
        tui_keymap: TuiKeymap::default(),
        model_availability_nux: ModelAvailabilityNuxConfig::default(),
        terminal_resize_reflow: TerminalResizeReflowConfig::default(),
        analytics_enabled: Some(true),
        feedback_enabled: true,
        tool_suggest: ToolSuggestConfig::default(),
        tui_alternate_screen: AltScreenMode::Auto,
        tui_status_line: None,
        tui_status_line_use_colors: true,
        tui_terminal_title: None,
        tui_theme: None,
        tui_pet: None,
        tui_pet_anchor: TuiPetAnchor::Composer,
        tui_session_picker_view: SessionPickerViewMode::Dense,
        otel: OtelConfig::default(),
    };

    assert_eq!(expected_gpt5_profile_config, gpt5_profile_config);

    Ok(())
}
async fn test_requirements_web_search_mode_allowlist_does_not_warn_when_unset() -> anyhow::Result<()>
{
    let fixture = create_test_fixture()?;

    let requirements_toml = config_service::ConfigRequirementsToml {
        allowed_approval_policies: None,
        allowed_approvals_reviewers: None,
        allowed_sandbox_modes: None,
        remote_sandbox_config: None,
        allowed_web_search_modes: Some(vec![config_service::WebSearchModeRequirement::Cached]),
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
    let requirement_source = config_service::RequirementSource::Unknown;
    let requirement_source_for_error = requirement_source.clone();
    let allowed = vec![WebSearchMode::Disabled, WebSearchMode::Cached];
    let constrained = Constrained::new(WebSearchMode::Cached, move |candidate| {
        if matches!(candidate, WebSearchMode::Cached | WebSearchMode::Disabled) {
            Ok(())
        } else {
            Err(ConstraintError::InvalidValue {
                field_name: "web_search_mode",
                candidate: format!("{candidate:?}"),
                allowed: format!("{allowed:?}"),
                requirement_source: requirement_source_for_error.clone(),
            })
        }
    })?;
    let requirements = config_service::ConfigRequirements {
        web_search_mode: config_service::ConstrainedWithSource::new(
            constrained,
            Some(requirement_source),
        ),
        ..Default::default()
    };
    let config_layer_stack =
        config_service::ConfigLayerStack::new(Vec::new(), requirements, requirements_toml)
            .expect("config layer stack");

    let config = Config::load_config_with_layer_stack(
        LOCAL_FS.as_ref(),
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
        config_layer_stack,
    )
    .await?;

    assert!(
        !config
            .startup_warnings
            .iter()
            .any(|warning| warning.contains("Configured value for `web_search_mode`")),
        "{:?}",
        config.startup_warnings
    );

    Ok(())
}

#[test]
fn test_set_project_trusted_writes_explicit_tables() -> anyhow::Result<()> {
    let project_dir = Path::new("/some/path");
    let mut doc = toml_edit::DocumentMut::new();

    crate::editing::set_project_trust_level_in_document(
        &mut doc,
        project_dir,
        TrustLevel::Trusted,
    )?;

    let contents = doc.to_string();

    let raw_path = project_dir.to_string_lossy();
    let path_str = if raw_path.contains('\\') {
        format!("'{raw_path}'")
    } else {
        format!("\"{raw_path}\"")
    };
    let expected = format!(
        r#"[projects.{path_str}]
trust_level = "trusted"
"#
    );
    assert_eq!(contents, expected);

    Ok(())
}

#[test]
fn test_set_project_trusted_converts_inline_to_explicit() -> anyhow::Result<()> {
    let project_dir = Path::new("/some/path");

    // Seed config.toml with an inline project entry under [projects]
    let raw_path = project_dir.to_string_lossy();
    let path_str = if raw_path.contains('\\') {
        format!("'{raw_path}'")
    } else {
        format!("\"{raw_path}\"")
    };
    // Use a quoted key so backslashes don't require escaping on Windows
    let initial = format!(
        r#"[projects]
{path_str} = {{ trust_level = "untrusted" }}
"#
    );
    let mut doc = initial.parse::<toml_edit::DocumentMut>()?;

    // Run the function; it should convert to explicit tables and set trusted
    crate::editing::set_project_trust_level_in_document(
        &mut doc,
        project_dir,
        TrustLevel::Trusted,
    )?;

    let contents = doc.to_string();

    // Assert exact output after conversion to explicit table
    let expected = format!(
        r#"[projects]

[projects.{path_str}]
trust_level = "trusted"
"#
    );
    assert_eq!(contents, expected);

    Ok(())
}

#[test]
fn test_set_project_trusted_migrates_top_level_inline_projects_preserving_entries()
-> anyhow::Result<()> {
    let initial = r#"toplevel = "baz"
projects = { "/Users/mbolin/code/codex4" = { trust_level = "trusted", foo = "bar" } , "/Users/mbolin/code/codex3" = { trust_level = "trusted" } }
model = "foo""#;
    let mut doc = initial.parse::<toml_edit::DocumentMut>()?;

    // Approve a new directory
    let new_project = Path::new("/Users/mbolin/code/codex2");
    crate::editing::set_project_trust_level_in_document(
        &mut doc,
        new_project,
        TrustLevel::Trusted,
    )?;

    let contents = doc.to_string();

    // Since we created the [projects] table as part of migration, it is kept implicit.
    // Expect explicit per-project tables, preserving prior entries and appending the new one.
    let new_project_key = crate::loader::project_trust_key(new_project);
    let expected = format!(
        r#"toplevel = "baz"
model = "foo"

[projects."/Users/mbolin/code/codex4"]
trust_level = "trusted"
foo = "bar"

[projects."/Users/mbolin/code/codex3"]
trust_level = "trusted"

[projects."{new_project_key}"]
trust_level = "trusted"
"#
    );
    assert_eq!(contents, expected);

    Ok(())
}

#[cfg(unix)]
async fn active_project_does_not_match_configured_alias_for_canonical_cwd() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let project_root = tmp.path().join("project");
    let alias_root = tmp.path().join("project_alias");
    std::fs::create_dir_all(&project_root)?;
    std::os::unix::fs::symlink(&project_root, &alias_root)?;

    let config = ConfigToml {
        projects: Some(HashMap::from([(
            alias_root.to_string_lossy().to_string(),
            ProjectConfig {
                trust_level: Some(TrustLevel::Trusted),
            },
        )])),
        ..Default::default()
    };

    assert_eq!(
        config.get_active_project(&project_root, /*repo_root*/ None),
        None
    );

    Ok(())
}

#[test]
fn test_set_default_oss_provider() -> std::io::Result<()> {
    let temp_dir = TempDir::new()?;
    let codex_home = temp_dir.path();
    let config_path = codex_home.join(CONFIG_TOML_FILE);

    // Test setting valid provider on empty config
    set_default_oss_provider(codex_home, OLLAMA_OSS_PROVIDER_ID)?;
    let content = std::fs::read_to_string(&config_path)?;
    assert!(content.contains("oss_provider = \"ollama\""));

    // Test updating existing config
    std::fs::write(&config_path, "model = \"gpt-4\"\n")?;
    set_default_oss_provider(codex_home, LMSTUDIO_OSS_PROVIDER_ID)?;
    let content = std::fs::read_to_string(&config_path)?;
    assert!(content.contains("oss_provider = \"lmstudio\""));
    assert!(content.contains("model = \"gpt-4\""));

    // Test overwriting existing oss_provider
    set_default_oss_provider(codex_home, OLLAMA_OSS_PROVIDER_ID)?;
    let content = std::fs::read_to_string(&config_path)?;
    assert!(content.contains("oss_provider = \"ollama\""));
    assert!(!content.contains("oss_provider = \"lmstudio\""));

    // Test invalid provider
    let result = set_default_oss_provider(codex_home, "invalid_provider");
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("Invalid OSS provider"));
    assert!(error.to_string().contains("invalid_provider"));

    Ok(())
}

#[test]
fn test_set_default_oss_provider_rejects_legacy_ollama_chat_provider() -> std::io::Result<()> {
    let temp_dir = TempDir::new()?;
    let codex_home = temp_dir.path();

    let result = set_default_oss_provider(codex_home, LEGACY_OLLAMA_CHAT_PROVIDER_ID);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        error
            .to_string()
            .contains(OLLAMA_CHAT_PROVIDER_REMOVED_ERROR)
    );

    Ok(())
}

async fn test_load_config_rejects_legacy_ollama_chat_provider_with_helpful_error()
-> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        model_provider: Some(LEGACY_OLLAMA_CHAT_PROVIDER_ID.to_string()),
        ..Default::default()
    };

    let result = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await;
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(
        error
            .to_string()
            .contains(OLLAMA_CHAT_PROVIDER_REMOVED_ERROR)
    );

    Ok(())
}

async fn test_untrusted_project_gets_workspace_write_sandbox() -> anyhow::Result<()> {
    let config_with_untrusted = r#"
[projects."/tmp/test"]
trust_level = "untrusted"
"#;

    let cfg = toml::from_str::<ConfigToml>(config_with_untrusted)
        .expect("TOML deserialization should succeed");
    let active_project = ProjectConfig {
        trust_level: Some(TrustLevel::Untrusted),
    };

    let resolution = derive_legacy_sandbox_policy_for_test(
        &cfg,
        /*sandbox_mode_override*/ None,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        Some(&active_project),
        /*permission_profile_constraint*/ None,
    )
    .await;

    // Verify that untrusted projects get WorkspaceWrite (or ReadOnly on Windows due to downgrade)
    if cfg!(target_os = "windows") {
        assert!(
            matches!(resolution, SandboxPolicy::ReadOnly { .. }),
            "Expected ReadOnly on Windows, got {resolution:?}"
        );
    } else {
        assert!(
            matches!(resolution, SandboxPolicy::WorkspaceWrite { .. }),
            "Expected WorkspaceWrite for untrusted project, got {resolution:?}"
        );
    }

    Ok(())
}

async fn derive_sandbox_policy_falls_back_to_read_only_for_implicit_defaults() -> anyhow::Result<()>
{
    let project_dir = TempDir::new()?;
    let project_path = project_dir.path().to_path_buf();
    let project_key = project_path.to_string_lossy().to_string();
    let cfg = ConfigToml {
        projects: Some(HashMap::from([(
            project_key,
            ProjectConfig {
                trust_level: Some(TrustLevel::Trusted),
            },
        )])),
        ..Default::default()
    };
    let active_project = ProjectConfig {
        trust_level: Some(TrustLevel::Trusted),
    };
    let constrained = Constrained::new(PermissionProfile::read_only(), |candidate| {
        if candidate == &PermissionProfile::read_only() {
            Ok(())
        } else {
            Err(ConstraintError::InvalidValue {
                field_name: "sandbox_mode",
                candidate: format!("{candidate:?}"),
                allowed: "[ReadOnly]".to_string(),
                requirement_source: RequirementSource::Unknown,
            })
        }
    })?;

    let resolution = derive_legacy_sandbox_policy_for_test(
        &cfg,
        /*sandbox_mode_override*/ None,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        Some(&active_project),
        Some(&constrained),
    )
    .await;

    assert_eq!(resolution, SandboxPolicy::new_read_only_policy());
    Ok(())
}

#[tokio::test]
async fn derive_sandbox_policy_preserves_windows_downgrade_for_unsupported_fallback()
-> anyhow::Result<()> {
    let project_dir = TempDir::new()?;
    let project_path = project_dir.path().to_path_buf();
    let project_key = project_path.to_string_lossy().to_string();
    let cfg = ConfigToml {
        projects: Some(HashMap::from([(
            project_key,
            ProjectConfig {
                trust_level: Some(TrustLevel::Trusted),
            },
        )])),
        ..Default::default()
    };
    let active_project = ProjectConfig {
        trust_level: Some(TrustLevel::Trusted),
    };
    let constrained = Constrained::new(
        PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        |candidate| {
            if matches!(
                candidate,
                PermissionProfile::Managed {
                    file_system: ManagedFileSystemPermissions::Restricted { entries, .. },
                    ..
                } if entries
                        .iter()
                        .any(|entry| entry.access.can_write())
            ) {
                Ok(())
            } else {
                Err(ConstraintError::InvalidValue {
                    field_name: "sandbox_mode",
                    candidate: format!("{candidate:?}"),
                    allowed: "[WorkspaceWrite]".to_string(),
                    requirement_source: RequirementSource::Unknown,
                })
            }
        },
    )?;

    let resolution = derive_legacy_sandbox_policy_for_test(
        &cfg,
        /*sandbox_mode_override*/ None,
        /*profile_sandbox_mode*/ None,
        WindowsSandboxLevel::Disabled,
        Some(&active_project),
        Some(&constrained),
    )
    .await;

    if cfg!(target_os = "windows") {
        assert_eq!(resolution, SandboxPolicy::new_read_only_policy());
    } else {
        assert_eq!(resolution, SandboxPolicy::new_workspace_write_policy());
    }
    Ok(())
}

#[test]
fn test_resolve_oss_provider_explicit_override() {
    let config_toml = ConfigToml::default();
    let result = resolve_oss_provider(
        Some("custom-provider"),
        &config_toml,
        /*config_profile*/ None,
    );
    assert_eq!(result, Some("custom-provider".to_string()));
}

#[test]
fn test_resolve_oss_provider_from_profile() {
    let mut profiles = std::collections::HashMap::new();
    let profile = ConfigProfile {
        oss_provider: Some("profile-provider".to_string()),
        ..Default::default()
    };
    profiles.insert("test-profile".to_string(), profile);
    let config_toml = ConfigToml {
        profiles,
        ..Default::default()
    };

    let result = resolve_oss_provider(
        /*explicit_provider*/ None,
        &config_toml,
        Some("test-profile".to_string()),
    );
    assert_eq!(result, Some("profile-provider".to_string()));
}

#[test]
fn test_resolve_oss_provider_from_global_config() {
    let config_toml = ConfigToml {
        oss_provider: Some("global-provider".to_string()),
        ..Default::default()
    };

    let result = resolve_oss_provider(
        /*explicit_provider*/ None,
        &config_toml,
        /*config_profile*/ None,
    );
    assert_eq!(result, Some("global-provider".to_string()));
}

#[test]
fn test_resolve_oss_provider_profile_fallback_to_global() {
    let mut profiles = std::collections::HashMap::new();
    let profile = ConfigProfile::default(); // No oss_provider set
    profiles.insert("test-profile".to_string(), profile);
    let config_toml = ConfigToml {
        oss_provider: Some("global-provider".to_string()),
        profiles,
        ..Default::default()
    };

    let result = resolve_oss_provider(
        /*explicit_provider*/ None,
        &config_toml,
        Some("test-profile".to_string()),
    );
    assert_eq!(result, Some("global-provider".to_string()));
}

#[test]
fn test_resolve_oss_provider_none_when_not_configured() {
    let config_toml = ConfigToml::default();
    let result = resolve_oss_provider(
        /*explicit_provider*/ None,
        &config_toml,
        /*config_profile*/ None,
    );
    assert_eq!(result, None);
}

#[test]
fn test_resolve_oss_provider_explicit_overrides_all() {
    let mut profiles = std::collections::HashMap::new();
    let profile = ConfigProfile {
        oss_provider: Some("profile-provider".to_string()),
        ..Default::default()
    };
    profiles.insert("test-profile".to_string(), profile);
    let config_toml = ConfigToml {
        oss_provider: Some("global-provider".to_string()),
        profiles,
        ..Default::default()
    };

    let result = resolve_oss_provider(
        Some("explicit-provider"),
        &config_toml,
        Some("test-profile".to_string()),
    );
    assert_eq!(result, Some("explicit-provider".to_string()));
}

#[test]
fn config_toml_deserializes_mcp_oauth_callback_port() {
    let toml = r#"mcp_oauth_callback_port = 4321"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for callback port");
    assert_eq!(cfg.mcp_oauth_callback_port, Some(4321));
}

#[test]
fn config_toml_deserializes_mcp_oauth_callback_url() {
    let toml = r#"mcp_oauth_callback_url = "https://example.com/callback""#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for callback URL");
    assert_eq!(
        cfg.mcp_oauth_callback_url.as_deref(),
        Some("https://example.com/callback")
    );
}

#[tokio::test]
async fn config_loads_mcp_oauth_callback_port_from_toml() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let toml = r#"
model = "gpt-5.4"
mcp_oauth_callback_port = 5678
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for callback port");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.mcp_oauth_callback_port, Some(5678));
    Ok(())
}

#[tokio::test]
async fn config_loads_allow_login_shell_from_toml() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg: ConfigToml = toml::from_str(
        r#"
model = "gpt-5.4"
allow_login_shell = false
"#,
    )
    .expect("TOML deserialization should succeed for allow_login_shell");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(!config.permissions.allow_login_shell);
    Ok(())
}

#[tokio::test]
async fn config_loads_apps_mcp_path_override_from_feature_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let toml = r#"
model = "gpt-5.4"

[features.apps_mcp_path_override]
path = "/custom/mcp"
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for apps MCP feature");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.apps_mcp_path_override.as_deref(),
        Some("/custom/mcp")
    );
    Ok(())
}

#[tokio::test]
async fn config_defaults_enabled_apps_mcp_path_override_to_plugin_service() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let toml = r#"
model = "gpt-5.4"

[features]
apps_mcp_path_override = true
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for apps MCP feature");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(config.features.enabled(Feature::AppsMcpPathOverride));
    assert_eq!(config.apps_mcp_path_override.as_deref(), Some("/ps/mcp"));
    Ok(())
}

#[tokio::test]
async fn config_preserves_explicit_apps_mcp_path_override_path() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let toml = r#"
model = "gpt-5.4"

[features.apps_mcp_path_override]
enabled = true
path = "/custom/mcp"
"#;
    let cfg: ConfigToml =
        toml::from_str(toml).expect("TOML deserialization should succeed for apps MCP feature");

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.apps_mcp_path_override.as_deref(),
        Some("/custom/mcp")
    );
    assert!(config.features.enabled(Feature::AppsMcpPathOverride));
    Ok(())
}
