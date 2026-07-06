use super::*;

#[tokio::test]
async fn merging_missing_agent_role_dirs_does_not_override_existing_roles() -> std::io::Result<()> {
    let temp = TempDir::new()?;
    let plugin_agents = temp.path().join("plugin").join("agents");
    let other_plugin_agents = temp.path().join("other-plugin").join("agents");
    std::fs::create_dir_all(&plugin_agents)?;
    std::fs::create_dir_all(&other_plugin_agents)?;
    std::fs::write(
        plugin_agents.join("reviewer.md"),
        r#"---
name: reviewer
description: From plugin.
---

Plugin review instructions.
"#,
    )?;
    std::fs::write(
        plugin_agents.join("writer.md"),
        r#"---
name: writer
description: From plugin writer.
---

Plugin writing instructions.
"#,
    )?;
    std::fs::write(
        plugin_agents.join("hidden.md"),
        r#"---
name: hidden
---

Plugin hidden instructions.
"#,
    )?;
    std::fs::write(
        other_plugin_agents.join("writer.md"),
        r#"---
name: writer
description: From other plugin writer.
---

Other plugin writing instructions.
"#,
    )?;

    let mut roles = BTreeMap::from([(
        "reviewer".to_string(),
        AgentRoleConfig {
            description: Some("From project.".to_string()),
            ..Default::default()
        },
    )]);
    let mut warnings = Vec::new();
    crate::config::agent_roles::merge_missing_agent_roles_from_plugin_dirs(
        LOCAL_FS.as_ref(),
        &mut roles,
        &[
            (
                "code-review".to_string(),
                AbsolutePathBuf::try_from(plugin_agents)?,
            ),
            (
                "other-review".to_string(),
                AbsolutePathBuf::try_from(other_plugin_agents)?,
            ),
        ],
        &mut warnings,
    )
    .await?;

    assert_eq!(
        roles
            .get("reviewer")
            .and_then(|role| role.description.as_deref()),
        Some("From project.")
    );
    assert_eq!(
        roles
            .get("writer")
            .and_then(|role| role.description.as_deref()),
        Some("From plugin writer.")
    );
    assert_eq!(
        roles.get("writer").and_then(|role| role.source.as_ref()),
        Some(&AgentRoleSource::Plugin {
            plugin_id: "code-review".to_string()
        })
    );
    assert!(!roles.contains_key("hidden"));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("agent role `hidden` must define a description"))
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("duplicate plugin agent role name `writer`"))
    );

    Ok(())
}

#[tokio::test]
async fn mixed_legacy_and_standalone_agent_role_sources_merge_with_precedence()
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

[agents.researcher]
description = "Research role from config"
config_file = "./agents/researcher.toml"
nickname_candidates = ["Noether"]

[agents.critic]
description = "Critic role from config"
config_file = "./agents/critic.toml"
nickname_candidates = ["Ada"]
"#
        ),
    )
    .await?;

    let home_agents_dir = codex_home.path().join("agents");
    tokio::fs::create_dir_all(&home_agents_dir).await?;
    tokio::fs::write(
        home_agents_dir.join("researcher.toml"),
        r#"
developer_instructions = "Research carefully"
model = "gpt-5.2"
"#,
    )
    .await?;
    tokio::fs::write(
        home_agents_dir.join("critic.toml"),
        r#"
developer_instructions = "Critique carefully"
model = "gpt-4.1"
"#,
    )
    .await?;

    let standalone_agents_dir = repo_root.path().join(".codex").join("agents");
    tokio::fs::create_dir_all(&standalone_agents_dir).await?;
    tokio::fs::write(
        standalone_agents_dir.join("researcher.toml"),
        r#"
name = "researcher"
description = "Research role from file"
nickname_candidates = ["Hypatia"]
developer_instructions = "Research from file"
model = "gpt-5-mini"
"#,
    )
    .await?;
    tokio::fs::write(
        standalone_agents_dir.join("writer.toml"),
        r#"
name = "writer"
description = "Writer role from file"
nickname_candidates = ["Sagan"]
developer_instructions = "Write carefully"
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

    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.description.as_deref()),
        Some("Research role from file")
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.config_file.as_ref()),
        Some(&standalone_agents_dir.join("researcher.toml"))
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Hypatia"])
    );
    assert_eq!(
        config
            .agent_roles
            .get("critic")
            .and_then(|role| role.description.as_deref()),
        Some("Critic role from config")
    );
    assert_eq!(
        config
            .agent_roles
            .get("critic")
            .and_then(|role| role.config_file.as_ref()),
        Some(&home_agents_dir.join("critic.toml"))
    );
    assert_eq!(
        config
            .agent_roles
            .get("critic")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Ada"])
    );
    assert_eq!(
        config
            .agent_roles
            .get("writer")
            .and_then(|role| role.description.as_deref()),
        Some("Writer role from file")
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
async fn higher_precedence_agent_role_can_inherit_description_from_lower_layer()
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

[agents.researcher]
description = "Research role from config"
config_file = "./agents/researcher.toml"
"#
        ),
    )
    .await?;

    let home_agents_dir = codex_home.path().join("agents");
    tokio::fs::create_dir_all(&home_agents_dir).await?;
    tokio::fs::write(
        home_agents_dir.join("researcher.toml"),
        r#"
developer_instructions = "Research carefully"
model = "gpt-5.2"
"#,
    )
    .await?;

    let standalone_agents_dir = repo_root.path().join(".codex").join("agents");
    tokio::fs::create_dir_all(&standalone_agents_dir).await?;
    tokio::fs::write(
        standalone_agents_dir.join("researcher.toml"),
        r#"
name = "researcher"
nickname_candidates = ["Hypatia"]
developer_instructions = "Research from file"
model = "gpt-5-mini"
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
        Some(&standalone_agents_dir.join("researcher.toml"))
    );
    assert_eq!(
        config
            .agent_roles
            .get("researcher")
            .and_then(|role| role.nickname_candidates.as_ref())
            .map(|candidates| candidates.iter().map(String::as_str).collect::<Vec<_>>()),
        Some(vec!["Hypatia"])
    );

    Ok(())
}

#[tokio::test]
async fn load_config_resolves_agent_interrupt_message() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        agents: Some(AgentsToml {
            interrupt_message: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(!config.agent_interrupt_message_enabled);

    Ok(())
}

#[tokio::test]
async fn load_config_normalizes_agent_role_nickname_candidates() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
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
                    config_file: None,
                    nickname_candidates: Some(vec![
                        "  Hypatia  ".to_string(),
                        "Noether".to_string(),
                    ]),
                },
            )]),
        }),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

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
async fn load_config_rejects_empty_agent_role_nickname_candidates() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
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
                    config_file: None,
                    nickname_candidates: Some(Vec::new()),
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
    let err = result.expect_err("empty nickname candidates should be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        err.to_string()
            .contains("agents.researcher.nickname_candidates")
    );

    Ok(())
}

#[tokio::test]
async fn load_config_rejects_duplicate_agent_role_nickname_candidates() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
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
                    config_file: None,
                    nickname_candidates: Some(vec!["Hypatia".to_string(), " Hypatia ".to_string()]),
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
    let err = result.expect_err("duplicate nickname candidates should be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(
        err.to_string()
            .contains("agents.researcher.nickname_candidates cannot contain duplicates")
    );

    Ok(())
}

#[tokio::test]
async fn load_config_rejects_unsafe_agent_role_nickname_candidates() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
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
                    config_file: None,
                    nickname_candidates: Some(vec!["Agent <One>".to_string()]),
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
    let err = result.expect_err("unsafe nickname candidates should be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains(
            "agents.researcher.nickname_candidates may only contain ASCII letters, digits, spaces, hyphens, and underscores"
        ));

    Ok(())
}

#[tokio::test]
async fn model_catalog_json_loads_from_path() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let catalog_path = codex_home.path().join("catalog.json");
    let mut catalog = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    catalog.models = catalog.models.into_iter().take(1).collect();
    std::fs::write(
        &catalog_path,
        serde_json::to_string(&catalog).expect("serialize catalog"),
    )?;

    let cfg = ConfigToml {
        model_catalog_json: Some(catalog_path.abs()),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(config.model_catalog, Some(catalog));
    Ok(())
}

#[tokio::test]
async fn model_catalog_json_rejects_empty_catalog() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let catalog_path = codex_home.path().join("catalog.json");
    std::fs::write(&catalog_path, r#"{"models":[]}"#)?;

    let cfg = ConfigToml {
        model_catalog_json: Some(catalog_path.abs()),
        ..Default::default()
    };

    let err = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await
    .expect_err("empty custom catalog should fail config load");

    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("must contain at least one model"),
        "unexpected error: {err}"
    );
    Ok(())
}

pub(crate) fn create_test_fixture() -> std::io::Result<PrecedenceTestFixture> {
    let toml = r#"
model = "o3"
approval_policy = "untrusted"

# Can be used to determine which profile to use if not specified by
# `ConfigOverrides`.
profile = "gpt3"

[analytics]
enabled = true

[model_providers.openai-custom]
name = "OpenAI custom"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
request_max_retries = 4            # retry failed HTTP requests
stream_max_retries = 10            # retry dropped SSE streams
stream_idle_timeout_ms = 300000    # 5m idle timeout
websocket_connect_timeout_ms = 15000

[profiles.o3]
model = "o3"
model_provider = "openai"
approval_policy = "never"
model_reasoning_effort = "high"
model_reasoning_summary = "detailed"

[profiles.gpt3]
model = "gpt-3.5-turbo"
model_provider = "openai-custom"

[profiles.zdr]
model = "o3"
model_provider = "openai"
approval_policy = "on-failure"

[profiles.zdr.analytics]
enabled = false

[profiles.gpt5]
model = "gpt-5.4"
model_provider = "openai"
approval_policy = "on-failure"
model_reasoning_effort = "high"
model_reasoning_summary = "detailed"
model_verbosity = "high"
"#;

    let cfg: ConfigToml = toml::from_str(toml).expect("TOML deserialization should succeed");

    // Use a temporary directory for the cwd so it does not contain an
    // AGENTS.md file.
    let cwd_temp_dir = TempDir::new().unwrap();
    let cwd = cwd_temp_dir.path().to_path_buf();
    // Make it look like a Git repo so it does not search for AGENTS.md in
    // a parent folder, either.
    std::fs::write(cwd.join(".git"), "gitdir: nowhere")?;

    let codex_home_temp_dir = TempDir::new().unwrap();

    let openai_custom_provider = ModelProviderInfo {
        name: "OpenAI custom".to_string(),
        base_url: Some("https://api.openai.com/v1".to_string()),
        env_key: Some("OPENAI_API_KEY".to_string()),
        wire_api: WireApi::Responses,
        env_key_instructions: None,
        experimental_bearer_token: None,
        auth: None,
        aws: None,
        query_params: None,
        http_headers: None,
        env_http_headers: None,
        request_max_retries: Some(4),
        stream_max_retries: Some(10),
        stream_idle_timeout_ms: Some(300_000),
        websocket_connect_timeout_ms: Some(15_000),
        requires_openai_auth: false,
        supports_websockets: false,
    };
    let model_provider_map = {
        let mut model_provider_map =
            built_in_model_providers(/* openai_base_url */ /*openai_base_url*/ None);
        model_provider_map.insert("openai-custom".to_string(), openai_custom_provider.clone());
        model_provider_map
    };

    let openai_provider = model_provider_map
        .get("openai")
        .expect("openai provider should exist")
        .clone();

    Ok(PrecedenceTestFixture {
        cwd: cwd_temp_dir,
        codex_home: codex_home_temp_dir,
        cfg,
        model_provider_map,
        openai_provider,
        openai_custom_provider,
    })
}

/// Users can specify config values at multiple levels that have the
/// following precedence:
///
/// 1. custom command-line argument, e.g. `--model o3`
/// 2. as part of a profile, where the `--profile` is specified via a CLI
///    (or in the config file itself)
/// 3. as an entry in `config.toml`, e.g. `model = "o3"`
/// 4. the default value for a required field defined in code.
///
/// Note that profiles are the recommended way to specify a group of
/// configuration options together.
#[tokio::test]
async fn test_precedence_fixture_with_o3_profile() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let o3_profile_overrides = ConfigOverrides {
        config_profile: Some("o3".to_string()),
        cwd: Some(fixture.cwd_path()),
        ..Default::default()
    };
    let o3_profile_config: Config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        o3_profile_overrides,
        fixture.codex_home(),
    )
    .await?;
    assert_eq!(
        Config {
            model: Some("o3".to_string()),
            review_model: None,
            model_context_window: None,
            model_auto_compact_token_limit: None,
            service_tier: None,
            model_provider_id: "openai".to_string(),
            model_provider: fixture.openai_provider.clone(),
            model_options: Vec::new(),
            permissions: Permissions {
                approval_policy: Constrained::allow_any(AskForApproval::Never),
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
            model_verbosity: None,
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
            active_profile: Some("o3".to_string()),
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
        },
        o3_profile_config
    );
    Ok(())
}

#[tokio::test]
async fn metrics_exporter_defaults_to_statsig_when_missing() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(config.otel.metrics_exporter, OtelExporterKind::Statsig);
    Ok(())
}

#[tokio::test]
async fn trace_exporter_defaults_to_none_when_log_exporter_is_set() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;
    let mut cfg = fixture.cfg.clone();
    cfg.otel = Some(OtelConfigToml {
        exporter: Some(OtelExporterKind::OtlpHttp {
            endpoint: "http://localhost:14318/v1/logs".to_string(),
            headers: HashMap::new(),
            protocol: config_service::types::OtelHttpProtocol::Binary,
            tls: None,
        }),
        metrics_exporter: Some(OtelExporterKind::None),
        ..Default::default()
    });

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert!(matches!(
        config.otel.exporter,
        OtelExporterKind::OtlpHttp { .. }
    ));
    assert_eq!(config.otel.trace_exporter, OtelExporterKind::None);
    Ok(())
}

#[tokio::test]
async fn load_config_applies_otel_trace_metadata() -> std::io::Result<()> {
    let mut fixture = create_test_fixture()?;
    fixture.cfg = toml::from_str(
        r#"
[otel.span_attributes]
"example.trace_attr" = "enabled"

[otel.tracestate.example]
alpha = "one"
beta = "two"
"#,
    )
    .expect("TOML deserialization should succeed");

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(
        config.otel.span_attributes,
        BTreeMap::from([("example.trace_attr".to_string(), "enabled".to_string())])
    );
    assert_eq!(
        config.otel.tracestate,
        BTreeMap::from([(
            "example".to_string(),
            BTreeMap::from([
                ("alpha".to_string(), "one".to_string()),
                ("beta".to_string(), "two".to_string()),
            ]),
        )])
    );
    Ok(())
}

#[tokio::test]
async fn load_config_drops_invalid_otel_trace_metadata_entries() -> std::io::Result<()> {
    let mut fixture = create_test_fixture()?;
    fixture.cfg = toml::from_str(
        r#"
[otel]
environment = "test"

[otel.span_attributes]
"" = "missing-key"
"example.trace_attr" = "enabled"

[otel.tracestate.example]
alpha = "one"
beta = "two\ntoo"

[otel.tracestate.bad]
alpha = "one\ntwo"
"#,
    )
    .expect("TOML deserialization should succeed");

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(config.otel.environment, "test");
    assert_eq!(
        config.otel.span_attributes,
        BTreeMap::from([("example.trace_attr".to_string(), "enabled".to_string())])
    );
    assert_eq!(
        config.otel.tracestate,
        BTreeMap::from([(
            "example".to_string(),
            BTreeMap::from([("alpha".to_string(), "one".to_string())]),
        )])
    );
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning.contains("Ignoring invalid `otel.span_attributes` config")
                && warning.contains("configured span attribute key must not be empty")
        }),
        "{:?}",
        config.startup_warnings
    );
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning.contains("Ignoring invalid `otel.tracestate` config")
                && warning.contains("invalid configured tracestate value for example.beta")
        }),
        "{:?}",
        config.startup_warnings
    );
    assert!(
        config.startup_warnings.iter().any(|warning| {
            warning.contains("Ignoring invalid `otel.tracestate` config")
                && warning.contains("invalid configured tracestate value for bad.alpha")
        }),
        "{:?}",
        config.startup_warnings
    );
    Ok(())
}

#[tokio::test]
async fn explicit_null_service_tier_override_sets_fast_default_opt_out() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            service_tier: Some(None),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(config.service_tier, None);
    assert_eq!(config.notices.fast_default_opt_out, Some(true));
    Ok(())
}

#[tokio::test]
async fn legacy_fast_service_tier_override_uses_priority_request_value() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            service_tier: Some(Some("fast".to_string())),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(
        config.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );
    Ok(())
}

#[tokio::test]
async fn config_toml_priority_service_tier_uses_priority_request_value() -> std::io::Result<()> {
    let mut fixture = create_test_fixture()?;
    fixture.cfg.service_tier = Some(ServiceTier::Fast.request_value().to_string());
    let cwd = fixture.cwd_path();
    let codex_home = fixture.codex_home();

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg,
        ConfigOverrides {
            cwd: Some(cwd),
            ..Default::default()
        },
        codex_home,
    )
    .await?;

    assert_eq!(
        config.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );
    Ok(())
}

#[tokio::test]
async fn config_toml_service_tier_accepts_arbitrary_string() -> std::io::Result<()> {
    let mut fixture = create_test_fixture()?;
    fixture.cfg.service_tier = Some("experimental-tier-id".to_string());
    let cwd = fixture.cwd_path();
    let codex_home = fixture.codex_home();

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg,
        ConfigOverrides {
            cwd: Some(cwd),
            ..Default::default()
        },
        codex_home,
    )
    .await?;

    assert_eq!(
        config.service_tier,
        Some("experimental-tier-id".to_string())
    );
    Ok(())
}

#[tokio::test]
async fn config_toml_legacy_fast_service_tier_uses_priority_request_value() -> std::io::Result<()> {
    let mut fixture = create_test_fixture()?;
    fixture.cfg.service_tier = Some("fast".to_string());
    let cwd = fixture.cwd_path();
    let codex_home = fixture.codex_home();

    let config = Config::load_from_base_config_with_overrides(
        fixture.cfg,
        ConfigOverrides {
            cwd: Some(cwd),
            ..Default::default()
        },
        codex_home,
    )
    .await?;

    assert_eq!(
        config.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );
    Ok(())
}

#[tokio::test]
async fn fast_default_opt_out_notice_config_is_respected() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;
    let mut cfg = fixture.cfg.clone();
    cfg.notice = Some(Notice {
        fast_default_opt_out: Some(true),
        ..Default::default()
    });

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides {
            cwd: Some(fixture.cwd_path()),
            ..Default::default()
        },
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(config.service_tier, None);
    assert_eq!(config.notices.fast_default_opt_out, Some(true));
    Ok(())
}

#[tokio::test]
async fn test_precedence_fixture_with_gpt3_profile() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let gpt3_profile_overrides = ConfigOverrides {
        config_profile: Some("gpt3".to_string()),
        cwd: Some(fixture.cwd_path()),
        ..Default::default()
    };
    let gpt3_profile_config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        gpt3_profile_overrides,
        fixture.codex_home(),
    )
    .await?;
    let expected_gpt3_profile_config = Config {
        model: Some("gpt-3.5-turbo".to_string()),
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        service_tier: None,
        model_provider_id: "openai-custom".to_string(),
        model_provider: fixture.openai_custom_provider.clone(),
        model_options: Vec::new(),
        permissions: Permissions {
            approval_policy: Constrained::allow_any(AskForApproval::UnlessTrusted),
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
        model_reasoning_effort: None,
        plan_mode_reasoning_effort: None,
        model_reasoning_summary: None,
        model_supports_reasoning_summaries: None,
        model_catalog: None,
        model_verbosity: None,
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
        active_profile: Some("gpt3".to_string()),
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

    assert_eq!(expected_gpt3_profile_config, gpt3_profile_config);

    // Verify that loading without specifying a profile in ConfigOverrides
    // uses the default profile from the config file (which is "gpt3").
    let default_profile_overrides = ConfigOverrides {
        cwd: Some(fixture.cwd_path()),
        ..Default::default()
    };

    let default_profile_config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        default_profile_overrides,
        fixture.codex_home(),
    )
    .await?;

    assert_eq!(expected_gpt3_profile_config, default_profile_config);
    Ok(())
}

#[tokio::test]
async fn test_precedence_fixture_with_zdr_profile() -> std::io::Result<()> {
    let fixture = create_test_fixture()?;

    let zdr_profile_overrides = ConfigOverrides {
        config_profile: Some("zdr".to_string()),
        cwd: Some(fixture.cwd_path()),
        ..Default::default()
    };
    let zdr_profile_config = Config::load_from_base_config_with_overrides(
        fixture.cfg.clone(),
        zdr_profile_overrides,
        fixture.codex_home(),
    )
    .await?;
    let expected_zdr_profile_config = Config {
        model: Some("o3".to_string()),
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
        model_reasoning_effort: None,
        plan_mode_reasoning_effort: None,
        model_reasoning_summary: None,
        model_supports_reasoning_summaries: None,
        model_catalog: None,
        model_verbosity: None,
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
        active_profile: Some("zdr".to_string()),
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
        analytics_enabled: Some(false),
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

    assert_eq!(expected_zdr_profile_config, zdr_profile_config);

    Ok(())
}
