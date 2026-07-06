use super::*;

#[tokio::test]
async fn profile_sandbox_mode_overrides_base() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let mut profiles = HashMap::new();
    profiles.insert(
        "work".to_string(),
        ConfigProfile {
            sandbox_mode: Some(SandboxMode::DangerFullAccess),
            ..Default::default()
        },
    );
    let cfg = ConfigToml {
        profiles,
        profile: Some("work".to_string()),
        sandbox_mode: Some(SandboxMode::ReadOnly),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(matches!(
        &config.legacy_sandbox_policy(),
        &SandboxPolicy::DangerFullAccess
    ));

    Ok(())
}

#[tokio::test]
async fn cli_override_takes_precedence_over_profile_sandbox_mode() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let mut profiles = HashMap::new();
    profiles.insert(
        "work".to_string(),
        ConfigProfile {
            sandbox_mode: Some(SandboxMode::DangerFullAccess),
            ..Default::default()
        },
    );
    let cfg = ConfigToml {
        profiles,
        profile: Some("work".to_string()),
        ..Default::default()
    };

    let overrides = ConfigOverrides {
        sandbox_mode: Some(SandboxMode::WorkspaceWrite),
        ..Default::default()
    };

    let config =
        Config::load_from_base_config_with_overrides(cfg, overrides, codex_home.abs()).await?;

    if cfg!(target_os = "windows") {
        assert!(matches!(
            &config.legacy_sandbox_policy(),
            SandboxPolicy::ReadOnly { .. }
        ));
    } else {
        assert!(matches!(
            &config.legacy_sandbox_policy(),
            SandboxPolicy::WorkspaceWrite { .. }
        ));
    }

    Ok(())
}

#[tokio::test]
async fn feature_table_overrides_legacy_flags() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let mut entries = BTreeMap::new();
    entries.insert("apply_patch_freeform".to_string(), false);
    let cfg = ConfigToml {
        features: Some(FeaturesToml::from(entries)),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(!config.features.enabled(Feature::ApplyPatchFreeform));

    Ok(())
}

#[tokio::test]
async fn legacy_toggles_map_to_features() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        experimental_use_unified_exec_tool: Some(true),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert!(config.features.enabled(Feature::UnifiedExec));

    assert!(config.use_experimental_unified_exec_tool);

    Ok(())
}

#[tokio::test]
async fn responses_websocket_features_do_not_change_wire_api() -> std::io::Result<()> {
    for feature_key in ["responses_websockets", "responses_websockets_v2"] {
        let codex_home = TempDir::new()?;
        let mut entries = BTreeMap::new();
        entries.insert(feature_key.to_string(), true);
        let cfg = ConfigToml {
            features: Some(FeaturesToml::from(entries)),
            ..Default::default()
        };

        let config = Config::load_from_base_config_with_overrides(
            cfg,
            ConfigOverrides::default(),
            codex_home.abs(),
        )
        .await?;

        assert_eq!(config.model_provider.wire_api, WireApi::Responses);
    }

    Ok(())
}

#[tokio::test]
async fn config_honors_explicit_file_oauth_store_mode() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let cfg = ConfigToml {
        mcp_oauth_credentials_store: Some(OAuthCredentialsStoreMode::File),
        ..Default::default()
    };

    let config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;

    assert_eq!(
        config.mcp_oauth_credentials_store_mode,
        OAuthCredentialsStoreMode::File,
    );

    Ok(())
}

#[tokio::test]
async fn managed_config_overrides_oauth_store_mode() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let managed_path = codex_home.path().join("managed_config.toml");
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    std::fs::write(&config_path, "mcp_oauth_credentials_store = \"file\"\n")?;
    std::fs::write(&managed_path, "mcp_oauth_credentials_store = \"keyring\"\n")?;

    let overrides = LoaderOverrides::with_managed_config_path_for_tests(managed_path.clone());

    let cwd = codex_home.path().abs();
    let config_layer_stack = load_config_layers_state(
        LOCAL_FS.as_ref(),
        codex_home.path(),
        Some(cwd),
        &Vec::new(),
        overrides,
        CloudRequirementsLoader::default(),
        &config_service::NoopThreadConfigLoader,
    )
    .await?;
    let cfg =
        deserialize_config_toml_with_base(config_layer_stack.effective_config(), codex_home.path())
            .map_err(|e| {
                tracing::error!("Failed to deserialize overridden config: {e}");
                e
            })?;
    assert_eq!(
        cfg.mcp_oauth_credentials_store,
        Some(OAuthCredentialsStoreMode::Keyring),
    );

    let final_config = Config::load_from_base_config_with_overrides(
        cfg,
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;
    assert_eq!(
        final_config.mcp_oauth_credentials_store_mode,
        resolve_mcp_oauth_credentials_store_mode(
            OAuthCredentialsStoreMode::Keyring,
            env!("CARGO_PKG_VERSION"),
        ),
    );

    Ok(())
}

#[tokio::test]
async fn load_global_mcp_servers_returns_empty_if_missing() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = load_global_mcp_servers(codex_home.path()).await?;
    assert!(servers.is_empty());

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_round_trips_entries() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let mut servers = BTreeMap::new();
    servers.insert(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: Some("remote".to_string()),
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: Some(Duration::from_secs(3)),
            tool_timeout_sec: Some(Duration::from_secs(5)),
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    );

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    assert_eq!(loaded.len(), 1);
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } => {
            assert_eq!(command, "echo");
            assert_eq!(args, &vec!["hello".to_string()]);
            assert!(env.is_none());
            assert!(env_vars.is_empty());
            assert!(cwd.is_none());
        }
        other => panic!("unexpected transport {other:?}"),
    }
    assert_eq!(docs.startup_timeout_sec, Some(Duration::from_secs(3)));
    assert_eq!(docs.tool_timeout_sec, Some(Duration::from_secs(5)));
    assert_eq!(docs.experimental_environment.as_deref(), Some("remote"));
    assert!(docs.enabled);

    let empty = BTreeMap::new();
    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(empty.clone())],
    )?;
    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    assert!(loaded.is_empty());

    Ok(())
}

#[tokio::test]
async fn managed_config_wins_over_cli_overrides() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let managed_path = codex_home.path().join("managed_config.toml");

    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        "model = \"base\"\n",
    )?;
    std::fs::write(&managed_path, "model = \"managed_config\"\n")?;

    let overrides = LoaderOverrides::with_managed_config_path_for_tests(managed_path);

    let cwd = codex_home.path().abs();
    let config_layer_stack = load_config_layers_state(
        LOCAL_FS.as_ref(),
        codex_home.path(),
        Some(cwd),
        &[("model".to_string(), TomlValue::String("cli".to_string()))],
        overrides,
        CloudRequirementsLoader::default(),
        &config_service::NoopThreadConfigLoader,
    )
    .await?;

    let cfg =
        deserialize_config_toml_with_base(config_layer_stack.effective_config(), codex_home.path())
            .map_err(|e| {
                tracing::error!("Failed to deserialize overridden config: {e}");
                e
            })?;

    assert_eq!(cfg.model.as_deref(), Some("managed_config"));
    Ok(())
}

#[tokio::test]
async fn load_global_mcp_servers_accepts_legacy_ms_field() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    std::fs::write(
        &config_path,
        r#"
[mcp_servers]
[mcp_servers.docs]
command = "echo"
startup_timeout_ms = 2500
"#,
    )?;

    let servers = load_global_mcp_servers(codex_home.path()).await?;
    let docs = servers.get("docs").expect("docs entry");
    assert_eq!(docs.startup_timeout_sec, Some(Duration::from_millis(2500)));

    Ok(())
}

#[test]
fn mcp_servers_toml_parses_per_tool_approval_overrides() {
    let config = toml::from_str::<ConfigToml>(
        r#"
[mcp_servers.docs]
command = "docs-server"
name = "Docs"
default_tools_approval_mode = "prompt"

[mcp_servers.docs.tools.search]
approval_mode = "approve"
"#,
    )
    .expect("TOML deserialization should succeed");
    let server = config
        .mcp_servers
        .get("docs")
        .expect("docs server config exists");

    assert_eq!(
        server.default_tools_approval_mode,
        Some(AppToolApproval::Prompt)
    );

    assert_eq!(
        server.tools.get("search"),
        Some(&McpServerToolConfig {
            approval_mode: Some(AppToolApproval::Approve),
        })
    );
}

#[test]
fn mcp_servers_toml_ignores_unknown_server_fields() {
    let config = toml::from_str::<ConfigToml>(
        r#"
[mcp_servers.docs]
command = "docs-server"
trust_level = "trusted"
"#,
    )
    .expect("unknown MCP server fields should be ignored");

    assert_eq!(
        config.mcp_servers.get("docs"),
        Some(&stdio_mcp("docs-server"))
    );
}

#[test]
fn mcp_servers_toml_parses_tool_approval_override_for_reserved_name() {
    let config = toml::from_str::<ConfigToml>(
        r#"
[mcp_servers.docs]
command = "docs-server"

[mcp_servers.docs.tools.command]
approval_mode = "approve"
"#,
    )
    .expect("TOML deserialization should succeed");
    let tool = config
        .mcp_servers
        .get("docs")
        .and_then(|server| server.tools.get("command"))
        .expect("docs/command tool config exists");

    assert_eq!(
        tool,
        &McpServerToolConfig {
            approval_mode: Some(AppToolApproval::Approve),
        }
    );
}

#[test]
fn desktop_toml_round_trips_opaque_nested_values() -> anyhow::Result<()> {
    let parsed = toml::from_str::<ConfigToml>(
        r#"
[desktop]
appearanceTheme = "dark"
selected-avatar-id = "codex"
recentViews = ["threads", "settings"]

[desktop.workspace]
collapsed = true
width = 320
pane = { selected = "console", expanded = false }
"#,
    )?;

    let desktop = parsed
        .desktop
        .as_ref()
        .expect("desktop settings should deserialize");
    assert_eq!(
        desktop.get("appearanceTheme"),
        Some(&serde_json::json!("dark"))
    );
    assert_eq!(
        desktop.get("selected-avatar-id"),
        Some(&serde_json::json!("codex"))
    );
    assert_eq!(
        desktop.get("recentViews"),
        Some(&serde_json::json!(["threads", "settings"]))
    );
    assert_eq!(
        desktop.get("workspace"),
        Some(&serde_json::json!({
            "collapsed": true,
            "width": 320,
            "pane": {
                "selected": "console",
                "expanded": false,
            },
        }))
    );

    let serialized = toml::to_string(&parsed)?;
    let reparsed = toml::from_str::<ConfigToml>(&serialized)?;
    assert_eq!(reparsed.desktop, parsed.desktop);

    Ok(())
}

#[tokio::test]
async fn to_mcp_config_preserves_apps_feature_from_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;
    let plugin_runtime = TestPluginRuntime::default();

    config.apps_mcp_path_override = Some("/custom/mcp".to_string());
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;
    assert!(mcp_config.apps_enabled);
    assert_eq!(
        mcp_config.apps_mcp_path_override.as_deref(),
        Some("/custom/mcp")
    );

    let _ = config.features.disable(Feature::Apps);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;
    assert!(!mcp_config.apps_enabled);

    let _ = config.features.enable(Feature::Apps);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;
    assert!(mcp_config.apps_enabled);

    Ok(())
}

#[tokio::test]
async fn to_mcp_config_preserves_auth_elicitation_feature_from_config() -> std::io::Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        codex_home.abs(),
    )
    .await?;
    let plugin_runtime = TestPluginRuntime::default();

    let mcp_config = config.to_mcp_config(&plugin_runtime).await;
    assert_eq!(
        mcp_config.client_elicitation_support,
        mcp_types::McpClientElicitationSupport::Disabled
    );

    let _ = config.features.enable(Feature::AuthElicitation);
    let mcp_config = config.to_mcp_config(&plugin_runtime).await;
    assert_eq!(
        mcp_config.client_elicitation_support,
        mcp_types::McpClientElicitationSupport::AuthElicitation
    );

    Ok(())
}

#[tokio::test]
async fn load_global_mcp_servers_rejects_inline_bearer_token() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    std::fs::write(
        &config_path,
        r#"
[mcp_servers.docs]
url = "https://example.com/mcp"
bearer_token = "secret"
"#,
    )?;

    let err = load_global_mcp_servers(codex_home.path())
        .await
        .expect_err("bearer_token entries should be rejected");

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("bearer_token"));
    assert!(err.to_string().contains("bearer_token_env_var"));

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_env_sorted() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: vec!["--verbose".to_string()],
                env: Some(HashMap::from([
                    ("ZIG_VAR".to_string(), "3".to_string()),
                    ("ALPHA_VAR".to_string(), "1".to_string()),
                ])),
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.docs]
command = "docs-server"
args = ["--verbose"]

[mcp_servers.docs.env]
ALPHA_VAR = "1"
ZIG_VAR = "3"
"#
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } => {
            assert_eq!(command, "docs-server");
            assert_eq!(args, &vec!["--verbose".to_string()]);
            let env = env
                .as_ref()
                .expect("env should be preserved for stdio transport");
            assert_eq!(env.get("ALPHA_VAR"), Some(&"1".to_string()));
            assert_eq!(env.get("ZIG_VAR"), Some(&"3".to_string()));
            assert!(env_vars.is_empty());
            assert!(cwd.is_none());
        }
        other => panic!("unexpected transport {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_env_vars() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: vec!["ALPHA".into(), "BETA".into()],
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized.contains(r#"env_vars = ["ALPHA", "BETA"]"#),
        "serialized config missing env_vars field:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::Stdio { env_vars, .. } => {
            assert_eq!(env_vars, &vec!["ALPHA".into(), "BETA".into()]);
        }
        other => panic!("unexpected transport {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_sourced_env_vars() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: vec![
                    "LEGACY".into(),
                    McpServerEnvVar::Config {
                        name: "REMOTE_TOKEN".to_string(),
                        source: Some("remote".to_string()),
                    },
                ],
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized
            .contains(r#"env_vars = ["LEGACY", { name = "REMOTE_TOKEN", source = "remote" }]"#),
        "serialized config missing sourced env_vars field:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    assert_eq!(loaded, servers);

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_cwd() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let cwd_path = PathBuf::from("/tmp/codex-mcp");
    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: Some(cwd_path.clone()),
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized.contains(r#"cwd = "/tmp/codex-mcp""#),
        "serialized config missing cwd field:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::Stdio { cwd, .. } => {
            assert_eq!(cwd.as_deref(), Some(Path::new("/tmp/codex-mcp")));
        }
        other => panic!("unexpected transport {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_streamable_http_serializes_bearer_token() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: Some(Duration::from_secs(2)),
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.docs]
url = "https://example.com/mcp"
bearer_token_env_var = "MCP_TOKEN"
startup_timeout_sec = 2.0
"#
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            assert_eq!(url, "https://example.com/mcp");
            assert_eq!(bearer_token_env_var.as_deref(), Some("MCP_TOKEN"));
            assert!(http_headers.is_none());
            assert!(env_http_headers.is_none());
        }
        other => panic!("unexpected transport {other:?}"),
    }
    assert_eq!(docs.startup_timeout_sec, Some(Duration::from_secs(2)));

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_streamable_http_serializes_custom_headers() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                http_headers: Some(HashMap::from([("X-Doc".to_string(), "42".to_string())])),
                env_http_headers: Some(HashMap::from([(
                    "X-Auth".to_string(),
                    "DOCS_AUTH".to_string(),
                )])),
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: Some(Duration::from_secs(2)),
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);
    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.docs]
url = "https://example.com/mcp"
bearer_token_env_var = "MCP_TOKEN"
startup_timeout_sec = 2.0

[mcp_servers.docs.http_headers]
X-Doc = "42"

[mcp_servers.docs.env_http_headers]
X-Auth = "DOCS_AUTH"
"#
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::StreamableHttp {
            http_headers,
            env_http_headers,
            ..
        } => {
            assert_eq!(
                http_headers,
                &Some(HashMap::from([("X-Doc".to_string(), "42".to_string())]))
            );
            assert_eq!(
                env_http_headers,
                &Some(HashMap::from([(
                    "X-Auth".to_string(),
                    "DOCS_AUTH".to_string()
                )]))
            );
        }
        other => panic!("unexpected transport {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_streamable_http_removes_optional_sections() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    let mut servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                http_headers: Some(HashMap::from([("X-Doc".to_string(), "42".to_string())])),
                env_http_headers: Some(HashMap::from([(
                    "X-Auth".to_string(),
                    "DOCS_AUTH".to_string(),
                )])),
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: Some(Duration::from_secs(2)),
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;
    let serialized_with_optional = std::fs::read_to_string(&config_path)?;
    assert!(serialized_with_optional.contains("bearer_token_env_var = \"MCP_TOKEN\""));
    assert!(serialized_with_optional.contains("[mcp_servers.docs.http_headers]"));
    assert!(serialized_with_optional.contains("[mcp_servers.docs.env_http_headers]"));

    servers.insert(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    );
    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let serialized = std::fs::read_to_string(&config_path)?;
    assert_eq!(
        serialized,
        r#"[mcp_servers.docs]
url = "https://example.com/mcp"
"#
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            assert_eq!(url, "https://example.com/mcp");
            assert!(bearer_token_env_var.is_none());
            assert!(http_headers.is_none());
            assert!(env_http_headers.is_none());
        }
        other => panic!("unexpected transport {other:?}"),
    }

    assert!(docs.startup_timeout_sec.is_none());

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_streamable_http_isolates_headers_between_servers() -> anyhow::Result<()>
{
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    let servers = BTreeMap::from([
        (
            "docs".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::StreamableHttp {
                    url: "https://example.com/mcp".to_string(),
                    bearer_token_env_var: Some("MCP_TOKEN".to_string()),
                    http_headers: Some(HashMap::from([("X-Doc".to_string(), "42".to_string())])),
                    env_http_headers: Some(HashMap::from([(
                        "X-Auth".to_string(),
                        "DOCS_AUTH".to_string(),
                    )])),
                },
                experimental_environment: None,
                enabled: true,
                required: false,
                supports_parallel_tool_calls: false,
                disabled_reason: None,
                startup_timeout_sec: Some(Duration::from_secs(2)),
                tool_timeout_sec: None,
                default_tools_approval_mode: None,
                enabled_tools: None,
                disabled_tools: None,
                scopes: None,
                oauth: None,
                oauth_resource: None,
                tools: HashMap::new(),
            },
        ),
        (
            "logs".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::Stdio {
                    command: "logs-server".to_string(),
                    args: vec!["--follow".to_string()],
                    env: None,
                    env_vars: Vec::new(),
                    cwd: None,
                },
                experimental_environment: None,
                enabled: true,
                required: false,
                supports_parallel_tool_calls: false,
                disabled_reason: None,
                startup_timeout_sec: None,
                tool_timeout_sec: None,
                default_tools_approval_mode: None,
                enabled_tools: None,
                disabled_tools: None,
                scopes: None,
                oauth: None,
                oauth_resource: None,
                tools: HashMap::new(),
            },
        ),
    ]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized.contains("[mcp_servers.docs.http_headers]"),
        "serialized config missing docs headers section:\n{serialized}"
    );
    assert!(
        !serialized.contains("[mcp_servers.logs.http_headers]"),
        "serialized config should not add logs headers section:\n{serialized}"
    );
    assert!(
        !serialized.contains("[mcp_servers.logs.env_http_headers]"),
        "serialized config should not add logs env headers section:\n{serialized}"
    );
    assert!(
        !serialized.contains("mcp_servers.logs.bearer_token_env_var"),
        "serialized config should not add bearer token to logs:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    match &docs.transport {
        McpServerTransportConfig::StreamableHttp {
            http_headers,
            env_http_headers,
            ..
        } => {
            assert_eq!(
                http_headers,
                &Some(HashMap::from([("X-Doc".to_string(), "42".to_string())]))
            );
            assert_eq!(
                env_http_headers,
                &Some(HashMap::from([(
                    "X-Auth".to_string(),
                    "DOCS_AUTH".to_string()
                )]))
            );
        }
        other => panic!("unexpected transport {other:?}"),
    }
    let logs = loaded.get("logs").expect("logs entry");
    match &logs.transport {
        McpServerTransportConfig::Stdio { env, .. } => {
            assert!(env.is_none());
        }
        other => panic!("unexpected transport {other:?}"),
    }

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_disabled_flag() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: false,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized.contains("enabled = false"),
        "serialized config missing disabled flag:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    assert!(!docs.enabled);

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_required_flag() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: true,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(
        serialized.contains("required = true"),
        "serialized config missing required flag:\n{serialized}"
    );

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    assert!(docs.required);

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_serializes_tool_filters() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::Stdio {
                command: "docs-server".to_string(),
                args: Vec::new(),
                env: None,
                env_vars: Vec::new(),
                cwd: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: Some(vec!["allowed".to_string()]),
            disabled_tools: Some(vec!["blocked".to_string()]),
            scopes: None,
            oauth: None,
            oauth_resource: None,
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(serialized.contains(r#"enabled_tools = ["allowed"]"#));
    assert!(serialized.contains(r#"disabled_tools = ["blocked"]"#));

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    assert_eq!(
        docs.enabled_tools.as_ref(),
        Some(&vec!["allowed".to_string()])
    );
    assert_eq!(
        docs.disabled_tools.as_ref(),
        Some(&vec!["blocked".to_string()])
    );

    Ok(())
}

#[tokio::test]
async fn replace_mcp_servers_streamable_http_serializes_oauth_resource() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    let servers = BTreeMap::from([(
        "docs".to_string(),
        McpServerConfig {
            transport: McpServerTransportConfig::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                bearer_token_env_var: None,
                http_headers: None,
                env_http_headers: None,
            },
            experimental_environment: None,
            enabled: true,
            required: false,
            supports_parallel_tool_calls: false,
            disabled_reason: None,
            startup_timeout_sec: None,
            tool_timeout_sec: None,
            default_tools_approval_mode: None,
            enabled_tools: None,
            disabled_tools: None,
            scopes: None,
            oauth: Some(McpServerOAuthConfig {
                client_id: Some("eci-prd-pub-codex-123".to_string()),
            }),
            oauth_resource: Some("https://resource.example.com".to_string()),
            tools: HashMap::new(),
        },
    )]);

    apply_blocking(
        codex_home.path(),
        /*profile*/ None,
        &[ConfigEdit::ReplaceMcpServers(servers.clone())],
    )?;

    let config_path = codex_home.path().join(CONFIG_TOML_FILE);
    let serialized = std::fs::read_to_string(&config_path)?;
    assert!(serialized.contains("[mcp_servers.docs.oauth]"));
    assert!(serialized.contains(r#"client_id = "eci-prd-pub-codex-123""#));
    assert!(serialized.contains(r#"oauth_resource = "https://resource.example.com""#));

    let loaded = load_global_mcp_servers(codex_home.path()).await?;
    let docs = loaded.get("docs").expect("docs entry");
    assert_eq!(
        docs.oauth_resource.as_deref(),
        Some("https://resource.example.com")
    );
    assert_eq!(docs.oauth_client_id(), Some("eci-prd-pub-codex-123"));

    Ok(())
}

#[tokio::test]
async fn set_model_updates_defaults() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;

    ConfigEditsBuilder::new(codex_home.path())
        .set_model(Some("gpt-5.4"), Some(ReasoningEffort::High))
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(codex_home.path().join(CONFIG_TOML_FILE)).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(parsed.model_reasoning_effort, Some(ReasoningEffort::High));

    Ok(())
}

#[tokio::test]
async fn for_config_writes_selected_user_config_file() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let base_config = codex_home.path().join(CONFIG_TOML_FILE);
    let selected_config = codex_home.path().join("work.config.toml");
    tokio::fs::write(&base_config, r#"model_provider = "openai""#).await?;
    tokio::fs::write(&selected_config, r#"model = "gpt-old""#).await?;

    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.path().to_path_buf())
        .loader_overrides(LoaderOverrides {
            user_config_path: Some(selected_config.abs()),
            user_config_profile: Some("work".parse().expect("profile-v2 name")),
            ..LoaderOverrides::without_managed_config_for_tests()
        })
        .build()
        .await?;

    ConfigEditsBuilder::for_config(&config)
        .set_model(Some("gpt-new"), Some(ReasoningEffort::High))
        .apply()
        .await?;

    let selected_serialized = tokio::fs::read_to_string(&selected_config).await?;
    let selected: ConfigToml = toml::from_str(&selected_serialized)?;
    assert_eq!(selected.model.as_deref(), Some("gpt-new"));
    assert_eq!(selected.model_reasoning_effort, Some(ReasoningEffort::High));
    assert_eq!(
        tokio::fs::read_to_string(&base_config).await?,
        r#"model_provider = "openai""#
    );

    Ok(())
}

#[test]
fn profile_v2_config_path_resolves_validated_names() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let profile_name: ProfileV2Name = "work".parse()?;
    assert_eq!(
        resolve_profile_v2_config_path(codex_home.path(), &profile_name),
        codex_home.path().join("work.config.toml").abs()
    );
    Ok(())
}

#[tokio::test]
async fn set_model_overwrites_existing_model() -> anyhow::Result<()> {
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join(CONFIG_TOML_FILE);

    tokio::fs::write(
        &config_path,
        r#"
model = "gpt-5.4"
model_reasoning_effort = "medium"

[profiles.dev]
model = "gpt-4.1"
"#,
    )
    .await?;

    ConfigEditsBuilder::new(codex_home.path())
        .set_model(Some("o4-mini"), Some(ReasoningEffort::High))
        .apply()
        .await?;

    let serialized = tokio::fs::read_to_string(config_path).await?;
    let parsed: ConfigToml = toml::from_str(&serialized)?;

    assert_eq!(parsed.model.as_deref(), Some("o4-mini"));
    assert_eq!(parsed.model_reasoning_effort, Some(ReasoningEffort::High));
    assert_eq!(
        parsed
            .profiles
            .get("dev")
            .and_then(|profile| profile.model.as_deref()),
        Some("gpt-4.1"),
    );

    Ok(())
}
