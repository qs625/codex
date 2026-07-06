use super::*;

#[tokio::test]
async fn load_plugins_loads_default_skills_and_mcp_servers() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample",
  "description": "Plugin that includes the sample MCP server and Skills"
}"#,
    );
    write_file(
        &plugin_root.join("skills/sample-search/SKILL.md"),
        "---\nname: sample-search\ndescription: search sample data\n---\n",
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp",
      "oauth": {
        "clientId": "client-id",
        "callbackPort": 3118
      }
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join(".app.json"),
        r#"{
  "apps": {
    "example": {
      "id": "connector_example"
    }
  }
}"#,
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins(),
        vec![LoadedPlugin {
            config_name: "sample@test".to_string(),
            manifest_name: Some("sample".to_string()),
            manifest_description: Some(
                "Plugin that includes the sample MCP server and Skills".to_string(),
            ),
            root: AbsolutePathBuf::try_from(plugin_root.clone()).unwrap(),
            enabled: true,
            skill_roots: vec![plugin_root.join("skills").abs()],
            disabled_skill_paths: HashSet::new(),
            has_enabled_skills: true,
            mcp_servers: HashMap::from([(
                "sample".to_string(),
                McpServerConfig {
                    transport: McpServerTransportConfig::StreamableHttp {
                        url: "https://sample.example/mcp".to_string(),
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
                        client_id: Some("client-id".to_string()),
                    }),
                    oauth_resource: None,
                    tools: HashMap::new(),
                },
            )]),
            apps: vec![AppConnectorId("connector_example".to_string())],
            hook_sources: Vec::new(),
            hook_load_warnings: Vec::new(),
            error: None,
        }]
    );
    assert_eq!(
        outcome.capability_summaries(),
        &[PluginCapabilitySummary {
            config_name: "sample@test".to_string(),
            display_name: "sample".to_string(),
            description: Some("Plugin that includes the sample MCP server and Skills".to_string(),),
            has_skills: true,
            mcp_server_names: vec!["sample".to_string()],
            app_connector_ids: vec![AppConnectorId("connector_example".to_string())],
        }]
    );
    assert_eq!(
        outcome.effective_skill_roots(),
        vec![plugin_root.join("skills").abs()]
    );
    assert_eq!(outcome.effective_mcp_servers().len(), 1);
    assert_eq!(
        outcome.effective_apps(),
        vec![AppConnectorId("connector_example".to_string())]
    );
}

#[tokio::test]
async fn load_plugins_applies_plugin_mcp_server_policy() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample"
}"#,
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp",
      "default_tools_approval_mode": "prompt",
      "enabled_tools": ["read", "search"],
      "tools": {
        "search": { "approval_mode": "prompt" }
      }
    }
  }
}"#,
    );
    let config_toml = r#"
[features]
plugins = true

[plugins."sample@test"]
enabled = true

[plugins."sample@test".mcp_servers.sample]
enabled = false
default_tools_approval_mode = "approve"
enabled_tools = ["search"]
disabled_tools = ["delete"]

[plugins."sample@test".mcp_servers.sample.tools.search]
approval_mode = "approve"
"#;

    let outcome = load_plugins_from_config(config_toml, codex_home.path()).await;
    let server = outcome.plugins()[0]
        .mcp_servers
        .get("sample")
        .expect("sample server");

    assert!(!server.enabled);
    assert_eq!(
        server.default_tools_approval_mode,
        Some(AppToolApproval::Approve)
    );
    assert_eq!(server.enabled_tools, Some(vec!["search".to_string()]));
    assert_eq!(server.disabled_tools, Some(vec!["delete".to_string()]));
    assert_eq!(
        server.tools.get("search"),
        Some(&McpServerToolConfig {
            approval_mode: Some(AppToolApproval::Approve),
        })
    );
}

#[tokio::test]
async fn remote_installed_cache_adds_plugin_skill_roots_without_remote_plugin_flag() {
    let codex_home = TempDir::new().unwrap();
    let plugin_base = codex_home
        .path()
        .join("plugins/cache/chatgpt-global/linear");
    write_plugin(&plugin_base, "local", "linear");
    write_file(
        &codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );

    let config = load_config(codex_home.path(), codex_home.path()).await;
    let manager = PluginsManager::new(codex_home.path().to_path_buf());
    manager.replace_remote_installed_plugins_cache(vec![RemoteInstalledPlugin {
        marketplace_name: "chatgpt-global".to_string(),
        id: "plugins~Plugin_linear".to_string(),
        name: "linear".to_string(),
        enabled: true,
    }]);

    let outcome = manager.plugins_for_config(&config).await;
    assert_eq!(
        outcome.effective_skill_roots(),
        vec![AbsolutePathBuf::try_from(plugin_base.join("local/skills")).unwrap()]
    );
    assert_eq!(outcome.plugins().len(), 1);
    assert_eq!(outcome.plugins()[0].config_name, "linear@chatgpt-global");
}

#[tokio::test]
async fn remote_installed_cache_ignores_plugins_missing_local_cache() {
    let codex_home = TempDir::new().unwrap();
    write_file(
        &codex_home.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
remote_plugin = true
"#,
    );

    let config = load_config(codex_home.path(), codex_home.path()).await;
    let manager = PluginsManager::new(codex_home.path().to_path_buf());
    manager.replace_remote_installed_plugins_cache(vec![RemoteInstalledPlugin {
        marketplace_name: "chatgpt-global".to_string(),
        id: "plugins~Plugin_linear".to_string(),
        name: "linear".to_string(),
        enabled: true,
    }]);

    let outcome = manager.plugins_for_config(&config).await;
    assert_eq!(outcome, PluginLoadOutcome::default());
}

#[tokio::test]
async fn load_plugins_resolves_disabled_skill_names_against_loaded_plugin_skills() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");
    let skill_path = plugin_root.join("skills/sample-search/SKILL.md");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );
    write_file(
        &skill_path,
        "---\nname: sample-search\ndescription: search sample data\n---\n",
    );

    let config_toml = r#"[features]
plugins = true

[[skills.config]]
name = "sample:sample-search"
enabled = false

[plugins."sample@test"]
enabled = true
"#;
    let outcome = load_plugins_from_config(config_toml, codex_home.path()).await;
    let skill_path = std::fs::canonicalize(skill_path)
        .expect("skill path should canonicalize")
        .abs();

    assert_eq!(
        outcome.plugins()[0].disabled_skill_paths,
        HashSet::from([skill_path])
    );
    assert!(!outcome.plugins()[0].has_enabled_skills);
    assert!(outcome.capability_summaries().is_empty());
}

#[tokio::test]
async fn load_plugins_ignores_unknown_disabled_skill_names() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );
    write_file(
        &plugin_root.join("skills/sample-search/SKILL.md"),
        "---\nname: sample-search\ndescription: search sample data\n---\n",
    );

    let config_toml = r#"[features]
plugins = true

[[skills.config]]
name = "sample:missing-skill"
enabled = false

[plugins."sample@test"]
enabled = true
"#;
    let outcome = load_plugins_from_config(config_toml, codex_home.path()).await;

    assert!(outcome.plugins()[0].disabled_skill_paths.is_empty());
    assert!(outcome.plugins()[0].has_enabled_skills);
    assert_eq!(
        outcome.capability_summaries(),
        &[PluginCapabilitySummary {
            config_name: "sample@test".to_string(),
            display_name: "sample".to_string(),
            description: None,
            has_skills: true,
            mcp_server_names: Vec::new(),
            app_connector_ids: Vec::new(),
        }]
    );
}

#[tokio::test]
async fn plugin_telemetry_metadata_uses_default_mcp_config_path() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample"
}"#,
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp"
    }
  }
}"#,
    );

    let metadata = plugin_telemetry_metadata_from_root(
        &PluginId::parse("sample@test").expect("plugin id should parse"),
        &plugin_root.abs(),
    )
    .await;

    assert_eq!(
        metadata.capability_summary,
        Some(PluginCapabilitySummary {
            config_name: "sample@test".to_string(),
            display_name: "sample".to_string(),
            description: None,
            has_skills: false,
            mcp_server_names: vec!["sample".to_string()],
            app_connector_ids: Vec::new(),
        })
    );
}

#[tokio::test]
async fn capability_summary_sanitizes_plugin_descriptions_to_one_line() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample",
  "description": "Plugin that\n includes   the sample\tserver"
}"#,
    );
    write_file(
        &plugin_root.join("skills/sample-search/SKILL.md"),
        "---\nname: sample-search\ndescription: search sample data\n---\n",
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins()[0].manifest_description.as_deref(),
        Some("Plugin that\n includes   the sample\tserver")
    );
    assert_eq!(
        outcome.capability_summaries()[0].description.as_deref(),
        Some("Plugin that includes the sample server")
    );
}

#[tokio::test]
async fn capability_summary_truncates_overlong_plugin_descriptions() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");
    let too_long = "x".repeat(MAX_CAPABILITY_SUMMARY_DESCRIPTION_LEN + 1);

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        &format!(
            r#"{{
  "name": "sample",
  "description": "{too_long}"
}}"#
        ),
    );
    write_file(
        &plugin_root.join("skills/sample-search/SKILL.md"),
        "---\nname: sample-search\ndescription: search sample data\n---\n",
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins()[0].manifest_description.as_deref(),
        Some(too_long.as_str())
    );
    assert_eq!(
        outcome.capability_summaries()[0].description,
        Some("x".repeat(MAX_CAPABILITY_SUMMARY_DESCRIPTION_LEN))
    );
}

#[tokio::test]
async fn load_plugins_uses_manifest_configured_component_paths() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample",
  "skills": "./custom-skills/",
  "mcpServers": "./config/custom.mcp.json",
  "apps": "./config/custom.app.json"
}"#,
    );
    write_file(
        &plugin_root.join("skills/default-skill/SKILL.md"),
        "---\nname: default-skill\ndescription: default skill\n---\n",
    );
    write_file(
        &plugin_root.join("custom-skills/custom-skill/SKILL.md"),
        "---\nname: custom-skill\ndescription: custom skill\n---\n",
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "default": {
      "type": "http",
      "url": "https://default.example/mcp"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join("config/custom.mcp.json"),
        r#"{
  "mcpServers": {
    "custom": {
      "type": "http",
      "url": "https://custom.example/mcp"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join(".app.json"),
        r#"{
  "apps": {
    "default": {
      "id": "connector_default"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join("config/custom.app.json"),
        r#"{
  "apps": {
    "custom": {
      "id": "connector_custom"
    }
  }
}"#,
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins()[0].skill_roots,
        vec![
            plugin_root.join("custom-skills").abs(),
            plugin_root.join("skills").abs()
        ]
    );
    assert_eq!(
        outcome.plugins()[0].mcp_servers,
        HashMap::from([(
            "custom".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::StreamableHttp {
                    url: "https://custom.example/mcp".to_string(),
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
        )])
    );
    assert_eq!(
        outcome.plugins()[0].apps,
        vec![AppConnectorId("connector_custom".to_string())]
    );
}

#[tokio::test]
async fn load_plugins_ignores_manifest_component_paths_without_dot_slash() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{
  "name": "sample",
  "skills": "custom-skills",
  "mcpServers": "config/custom.mcp.json",
  "apps": "config/custom.app.json"
}"#,
    );
    write_file(
        &plugin_root.join("skills/default-skill/SKILL.md"),
        "---\nname: default-skill\ndescription: default skill\n---\n",
    );
    write_file(
        &plugin_root.join("custom-skills/custom-skill/SKILL.md"),
        "---\nname: custom-skill\ndescription: custom skill\n---\n",
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "default": {
      "type": "http",
      "url": "https://default.example/mcp"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join("config/custom.mcp.json"),
        r#"{
  "mcpServers": {
    "custom": {
      "type": "http",
      "url": "https://custom.example/mcp"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join(".app.json"),
        r#"{
  "apps": {
    "default": {
      "id": "connector_default"
    }
  }
}"#,
    );
    write_file(
        &plugin_root.join("config/custom.app.json"),
        r#"{
  "apps": {
    "custom": {
      "id": "connector_custom"
    }
  }
}"#,
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(/*enabled*/ true, /*plugins_feature_enabled*/ true),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins()[0].skill_roots,
        vec![plugin_root.join("skills").abs()]
    );
    assert_eq!(
        outcome.plugins()[0].mcp_servers,
        HashMap::from([(
            "default".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::StreamableHttp {
                    url: "https://default.example/mcp".to_string(),
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
        )])
    );
    assert_eq!(
        outcome.plugins()[0].apps,
        vec![AppConnectorId("connector_default".to_string())]
    );
}

#[tokio::test]
async fn load_plugins_preserves_disabled_plugins_without_effective_contributions() {
    let codex_home = TempDir::new().unwrap();
    let plugin_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/sample/local");

    write_file(
        &plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    );
    write_file(
        &plugin_root.join(".mcp.json"),
        r#"{
  "mcpServers": {
    "sample": {
      "type": "http",
      "url": "https://sample.example/mcp"
    }
  }
}"#,
    );

    let outcome = load_plugins_from_config(
        &plugin_config_toml(
            /*enabled*/ false, /*plugins_feature_enabled*/ true,
        ),
        codex_home.path(),
    )
    .await;

    assert_eq!(
        outcome.plugins(),
        vec![LoadedPlugin {
            config_name: "sample@test".to_string(),
            manifest_name: None,
            manifest_description: None,
            root: AbsolutePathBuf::try_from(plugin_root).unwrap(),
            enabled: false,
            skill_roots: Vec::new(),
            disabled_skill_paths: HashSet::new(),
            has_enabled_skills: false,
            mcp_servers: HashMap::new(),
            apps: Vec::new(),
            hook_sources: Vec::new(),
            hook_load_warnings: Vec::new(),
            error: None,
        }]
    );
    assert!(outcome.effective_skill_roots().is_empty());
    assert!(outcome.effective_mcp_servers().is_empty());
}

#[tokio::test]
async fn effective_apps_dedupes_connector_ids_across_plugins() {
    let codex_home = TempDir::new().unwrap();
    let plugin_a_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/plugin-a/local");
    let plugin_b_root = codex_home
        .path()
        .join("plugins/cache")
        .join("test/plugin-b/local");

    write_file(
        &plugin_a_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"plugin-a"}"#,
    );
    write_file(
        &plugin_a_root.join(".app.json"),
        r#"{
  "apps": {
    "example": {
      "id": "connector_example"
    }
  }
}"#,
    );
    write_file(
        &plugin_b_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"plugin-b"}"#,
    );
    write_file(
        &plugin_b_root.join(".app.json"),
        r#"{
  "apps": {
    "chat": {
      "id": "connector_example"
    },
    "gmail": {
      "id": "connector_gmail"
    }
  }
}"#,
    );

    let mut root = toml::map::Map::new();
    let mut features = toml::map::Map::new();
    features.insert("plugins".to_string(), Value::Boolean(true));
    root.insert("features".to_string(), Value::Table(features));

    let mut plugins = toml::map::Map::new();

    let mut plugin_a = toml::map::Map::new();
    plugin_a.insert("enabled".to_string(), Value::Boolean(true));
    plugins.insert("plugin-a@test".to_string(), Value::Table(plugin_a));

    let mut plugin_b = toml::map::Map::new();
    plugin_b.insert("enabled".to_string(), Value::Boolean(true));
    plugins.insert("plugin-b@test".to_string(), Value::Table(plugin_b));

    root.insert("plugins".to_string(), Value::Table(plugins));
    let config_toml =
        toml::to_string(&Value::Table(root)).expect("plugin test config should serialize");

    let outcome = load_plugins_from_config(&config_toml, codex_home.path()).await;

    assert_eq!(
        outcome.effective_apps(),
        vec![
            AppConnectorId("connector_example".to_string()),
            AppConnectorId("connector_gmail".to_string()),
        ]
    );
}

#[test]
fn capability_index_filters_inactive_and_zero_capability_plugins() {
    let codex_home = TempDir::new().unwrap();
    let connector = |id: &str| AppConnectorId(id.to_string());
    let http_server = |url: &str| McpServerConfig {
        transport: McpServerTransportConfig::StreamableHttp {
            url: url.to_string(),
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
    };
    let plugin = |config_name: &str, dir_name: &str, manifest_name: &str| LoadedPlugin {
        config_name: config_name.to_string(),
        manifest_name: Some(manifest_name.to_string()),
        manifest_description: None,
        root: AbsolutePathBuf::try_from(codex_home.path().join(dir_name)).unwrap(),
        enabled: true,
        skill_roots: Vec::new(),
        disabled_skill_paths: HashSet::new(),
        has_enabled_skills: false,
        mcp_servers: HashMap::new(),
        apps: Vec::new(),
        hook_sources: Vec::new(),
        hook_load_warnings: Vec::new(),
        error: None,
    };
    let summary = |config_name: &str, display_name: &str| PluginCapabilitySummary {
        config_name: config_name.to_string(),
        display_name: display_name.to_string(),
        description: None,
        ..PluginCapabilitySummary::default()
    };
    let outcome = PluginLoadOutcome::from_plugins(vec![
        LoadedPlugin {
            skill_roots: vec![codex_home.path().join("skills-plugin/skills").abs()],
            has_enabled_skills: true,
            ..plugin("skills@test", "skills-plugin", "skills-plugin")
        },
        LoadedPlugin {
            mcp_servers: HashMap::from([("alpha".to_string(), http_server("https://alpha"))]),
            apps: vec![connector("connector_example")],
            ..plugin("alpha@test", "alpha-plugin", "alpha-plugin")
        },
        LoadedPlugin {
            mcp_servers: HashMap::from([("beta".to_string(), http_server("https://beta"))]),
            apps: vec![connector("connector_example"), connector("connector_gmail")],
            ..plugin("beta@test", "beta-plugin", "beta-plugin")
        },
        plugin("empty@test", "empty-plugin", "empty-plugin"),
        LoadedPlugin {
            enabled: false,
            skill_roots: vec![codex_home.path().join("disabled-plugin/skills").abs()],
            apps: vec![connector("connector_hidden")],
            ..plugin("disabled@test", "disabled-plugin", "disabled-plugin")
        },
        LoadedPlugin {
            apps: vec![connector("connector_broken")],
            error: Some("failed to load".to_string()),
            ..plugin("broken@test", "broken-plugin", "broken-plugin")
        },
    ]);

    assert_eq!(
        outcome.capability_summaries(),
        &[
            PluginCapabilitySummary {
                has_skills: true,
                ..summary("skills@test", "skills-plugin")
            },
            PluginCapabilitySummary {
                mcp_server_names: vec!["alpha".to_string()],
                app_connector_ids: vec![connector("connector_example")],
                ..summary("alpha@test", "alpha-plugin")
            },
            PluginCapabilitySummary {
                mcp_server_names: vec!["beta".to_string()],
                app_connector_ids: vec![
                    connector("connector_example"),
                    connector("connector_gmail"),
                ],
                ..summary("beta@test", "beta-plugin")
            },
        ]
    );
}

