use super::*;

#[tokio::test]
async fn list_marketplaces_installed_git_source_reads_metadata_from_cache_without_cloning() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    let missing_remote_repo = tmp.path().join("missing-remote-plugin-repo");
    let missing_remote_repo_url = url::Url::from_directory_path(&missing_remote_repo)
        .unwrap()
        .to_string();
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    write_file(
        &repo_root.join(".agents/plugins/marketplace.json"),
        &format!(
            r#"{{
  "name": "debug",
  "plugins": [
    {{
      "name": "toolkit",
      "source": {{
        "source": "git-subdir",
        "url": "{missing_remote_repo_url}",
        "path": "plugins/toolkit"
      }},
      "category": "Developer Tools"
    }}
  ]
}}"#
        ),
    );
    let cached_plugin_root = tmp.path().join("plugins/cache/debug/toolkit/local");
    write_file(
        &cached_plugin_root.join(".codex-plugin/plugin.json"),
        r##"{
  "name": "toolkit",
  "interface": {
    "displayName": "Toolkit",
    "shortDescription": "Search cached data",
    "category": "Cached Category",
    "brandColor": "#3B82F6",
    "composerIcon": "./assets/icon.png",
    "logo": "./assets/logo.png",
    "screenshots": ["./assets/screenshot.png"]
  }
}"##,
    );
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."toolkit@debug"]
enabled = true
"#,
    );

    let config = load_config(tmp.path(), &repo_root).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[AbsolutePathBuf::try_from(repo_root).unwrap()])
        .unwrap()
        .marketplaces;

    let marketplace = marketplaces
        .into_iter()
        .find(|marketplace| marketplace.name == "debug")
        .expect("debug marketplace should be listed");

    assert_eq!(
        marketplace.plugins,
        vec![ConfiguredMarketplacePlugin {
            id: "toolkit@debug".to_string(),
            name: "toolkit".to_string(),
            local_version: None,
            source: MarketplacePluginSource::Git {
                url: missing_remote_repo_url,
                path: Some("plugins/toolkit".to_string()),
                ref_name: None,
                sha: None,
            },
            policy: MarketplacePluginPolicy {
                installation: MarketplacePluginInstallPolicy::Available,
                authentication: MarketplacePluginAuthPolicy::OnInstall,
                products: None,
            },
            interface: Some(PluginManifestInterface {
                display_name: Some("Toolkit".to_string()),
                short_description: Some("Search cached data".to_string()),
                category: Some("Developer Tools".to_string()),
                brand_color: Some("#3B82F6".to_string()),
                composer_icon: Some(
                    AbsolutePathBuf::try_from(cached_plugin_root.join("assets/icon.png")).unwrap(),
                ),
                logo: Some(
                    AbsolutePathBuf::try_from(cached_plugin_root.join("assets/logo.png")).unwrap(),
                ),
                screenshots: vec![
                    AbsolutePathBuf::try_from(cached_plugin_root.join("assets/screenshot.png"))
                        .unwrap(),
                ],
                ..Default::default()
            }),
            keywords: Vec::new(),
            installed: true,
            enabled: true,
        }]
    );
    assert!(
        !tmp.path()
            .join("plugins/.marketplace-plugin-source-staging")
            .exists()
    );
}

#[tokio::test]
async fn list_marketplaces_includes_curated_repo_marketplace() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    let plugin_root = curated_root.join("plugins/linear");

    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );
    fs::create_dir_all(curated_root.join(".agents/plugins")).unwrap();
    fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
    fs::write(
        curated_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "openai-curated",
  "plugins": [
    {
      "name": "linear",
      "source": {
        "source": "local",
        "path": "./plugins/linear"
      }
    }
  ]
}"#,
    )
    .unwrap();
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"linear"}"#,
    )
    .unwrap();

    let config = load_config(tmp.path(), tmp.path()).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[])
        .unwrap()
        .marketplaces;

    let curated_marketplace = marketplaces
        .into_iter()
        .find(|marketplace| marketplace.name == "openai-curated")
        .expect("curated marketplace should be listed");

    assert_eq!(
        curated_marketplace,
        ConfiguredMarketplace {
            name: "openai-curated".to_string(),
            path: AbsolutePathBuf::try_from(curated_root.join(".agents/plugins/marketplace.json"))
                .unwrap(),
            interface: None,
            plugins: vec![ConfiguredMarketplacePlugin {
                id: "linear@openai-curated".to_string(),
                name: "linear".to_string(),
                local_version: None,
                source: MarketplacePluginSource::Local {
                    path: AbsolutePathBuf::try_from(curated_root.join("plugins/linear")).unwrap(),
                },
                policy: MarketplacePluginPolicy {
                    installation: MarketplacePluginInstallPolicy::Available,
                    authentication: MarketplacePluginAuthPolicy::OnInstall,
                    products: None,
                },
                interface: None,
                keywords: Vec::new(),
                installed: false,
                enabled: false,
            }],
        }
    );
}

#[tokio::test]
async fn list_marketplaces_includes_installed_marketplace_roots() {
    let tmp = tempfile::tempdir().unwrap();
    let marketplace_root = marketplace_install_root(tmp.path()).join("debug");
    let plugin_root = marketplace_root.join("plugins/sample");

    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[marketplaces.debug]
last_updated = "2026-04-10T12:34:56Z"
source_type = "git"
source = "/tmp/debug"
"#,
    );
    fs::create_dir_all(marketplace_root.join(".agents/plugins")).unwrap();
    fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
    fs::write(
        marketplace_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "sample",
      "source": {
        "source": "local",
        "path": "./plugins/sample"
      }
    }
  ]
}"#,
    )
    .unwrap();
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )
    .unwrap();
    let config = load_config(tmp.path(), tmp.path()).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[])
        .unwrap()
        .marketplaces;

    let marketplace = marketplaces
        .into_iter()
        .find(|marketplace| {
            marketplace.path
                == AbsolutePathBuf::try_from(
                    marketplace_root.join(".agents/plugins/marketplace.json"),
                )
                .unwrap()
        })
        .expect("installed marketplace should be listed");

    assert_eq!(
        marketplace.path,
        AbsolutePathBuf::try_from(marketplace_root.join(".agents/plugins/marketplace.json"))
            .unwrap()
    );
    assert_eq!(marketplace.plugins.len(), 1);
    assert_eq!(marketplace.plugins[0].id, "sample@debug");
    assert_eq!(
        marketplace.plugins[0].source,
        MarketplacePluginSource::Local {
            path: AbsolutePathBuf::try_from(plugin_root).unwrap(),
        }
    );
}

#[tokio::test]
async fn list_marketplaces_uses_config_when_known_registry_is_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    let marketplace_root = marketplace_install_root(tmp.path()).join("debug");
    let plugin_root = marketplace_root.join("plugins/sample");
    let registry_path = tmp.path().join(".tmp/known_marketplaces.json");

    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[marketplaces.debug]
last_updated = "2026-04-10T12:34:56Z"
source_type = "git"
source = "/tmp/debug"
"#,
    );
    fs::create_dir_all(marketplace_root.join(".agents/plugins")).unwrap();
    fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
    fs::write(
        marketplace_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "sample",
      "source": {
        "source": "local",
        "path": "./plugins/sample"
      }
    }
  ]
}"#,
    )
    .unwrap();
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )
    .unwrap();
    fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
    fs::write(registry_path, "{not valid json").unwrap();

    let config = load_config(tmp.path(), tmp.path()).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[])
        .unwrap()
        .marketplaces;

    let marketplace = marketplaces
        .into_iter()
        .find(|marketplace| {
            marketplace.path
                == AbsolutePathBuf::try_from(
                    marketplace_root.join(".agents/plugins/marketplace.json"),
                )
                .unwrap()
        })
        .expect("configured marketplace should be discovered");

    assert_eq!(marketplace.plugins[0].id, "sample@debug");
}

#[tokio::test]
async fn list_marketplaces_ignores_installed_roots_missing_from_config() {
    let tmp = tempfile::tempdir().unwrap();
    let marketplace_root = marketplace_install_root(tmp.path()).join("debug");
    let plugin_root = marketplace_root.join("plugins/sample");

    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );
    fs::create_dir_all(marketplace_root.join(".agents/plugins")).unwrap();
    fs::create_dir_all(plugin_root.join(".codex-plugin")).unwrap();
    fs::write(
        marketplace_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "sample",
      "source": {
        "source": "local",
        "path": "./plugins/sample"
      }
    }
  ]
}"#,
    )
    .unwrap();
    fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        r#"{"name":"sample"}"#,
    )
    .unwrap();
    let config = load_config(tmp.path(), tmp.path()).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[])
        .unwrap()
        .marketplaces;

    assert!(
        marketplaces.iter().all(|marketplace| {
            marketplace.path
                != AbsolutePathBuf::try_from(
                    marketplace_root.join(".agents/plugins/marketplace.json"),
                )
                .unwrap()
        }),
        "installed marketplace root missing from config should not be listed"
    );
}

#[tokio::test]
async fn list_marketplaces_uses_first_duplicate_plugin_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_a_root = tmp.path().join("repo-a");
    let repo_b_root = tmp.path().join("repo-b");
    fs::create_dir_all(repo_a_root.join(".git")).unwrap();
    fs::create_dir_all(repo_b_root.join(".git")).unwrap();
    fs::create_dir_all(repo_a_root.join(".agents/plugins")).unwrap();
    fs::create_dir_all(repo_b_root.join(".agents/plugins")).unwrap();
    fs::write(
        repo_a_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "dup-plugin",
      "source": {
        "source": "local",
        "path": "./from-a"
      }
    }
  ]
}"#,
    )
    .unwrap();
    fs::write(
        repo_b_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "dup-plugin",
      "source": {
        "source": "local",
        "path": "./from-b"
      }
    },
    {
      "name": "b-only-plugin",
      "source": {
        "source": "local",
        "path": "./from-b-only"
      }
    }
  ]
}"#,
    )
    .unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."dup-plugin@debug"]
enabled = true

[plugins."b-only-plugin@debug"]
enabled = false
"#,
    );

    let config = load_config(tmp.path(), &repo_a_root).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(
            &config,
            &[
                AbsolutePathBuf::try_from(repo_a_root).unwrap(),
                AbsolutePathBuf::try_from(repo_b_root).unwrap(),
            ],
        )
        .unwrap()
        .marketplaces;

    let repo_a_marketplace = marketplaces
        .iter()
        .find(|marketplace| {
            marketplace.path
                == AbsolutePathBuf::try_from(
                    tmp.path().join("repo-a/.agents/plugins/marketplace.json"),
                )
                .unwrap()
        })
        .expect("repo-a marketplace should be listed");
    assert_eq!(
        repo_a_marketplace.plugins,
        vec![ConfiguredMarketplacePlugin {
            id: "dup-plugin@debug".to_string(),
            name: "dup-plugin".to_string(),
            local_version: None,
            source: MarketplacePluginSource::Local {
                path: AbsolutePathBuf::try_from(tmp.path().join("repo-a/from-a")).unwrap(),
            },
            policy: MarketplacePluginPolicy {
                installation: MarketplacePluginInstallPolicy::Available,
                authentication: MarketplacePluginAuthPolicy::OnInstall,
                products: None,
            },
            interface: None,
            keywords: Vec::new(),
            installed: false,
            enabled: true,
        }]
    );

    let repo_b_marketplace = marketplaces
        .iter()
        .find(|marketplace| {
            marketplace.path
                == AbsolutePathBuf::try_from(
                    tmp.path().join("repo-b/.agents/plugins/marketplace.json"),
                )
                .unwrap()
        })
        .expect("repo-b marketplace should be listed");
    assert_eq!(
        repo_b_marketplace.plugins,
        vec![ConfiguredMarketplacePlugin {
            id: "b-only-plugin@debug".to_string(),
            name: "b-only-plugin".to_string(),
            local_version: None,
            source: MarketplacePluginSource::Local {
                path: AbsolutePathBuf::try_from(tmp.path().join("repo-b/from-b-only")).unwrap(),
            },
            policy: MarketplacePluginPolicy {
                installation: MarketplacePluginInstallPolicy::Available,
                authentication: MarketplacePluginAuthPolicy::OnInstall,
                products: None,
            },
            interface: None,
            keywords: Vec::new(),
            installed: false,
            enabled: false,
        }]
    );

    let duplicate_plugin_count = marketplaces
        .iter()
        .flat_map(|marketplace| marketplace.plugins.iter())
        .filter(|plugin| plugin.name == "dup-plugin")
        .count();
    assert_eq!(duplicate_plugin_count, 1);
}

#[tokio::test]
async fn list_marketplaces_marks_configured_plugin_uninstalled_when_cache_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_root = tmp.path().join("repo");
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    fs::create_dir_all(repo_root.join(".agents/plugins")).unwrap();
    fs::write(
        repo_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "debug",
  "plugins": [
    {
      "name": "sample-plugin",
      "source": {
        "source": "local",
        "path": "./sample-plugin"
      }
    }
  ]
}"#,
    )
    .unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."sample-plugin@debug"]
enabled = true
"#,
    );

    let config = load_config(tmp.path(), &repo_root).await;
    let marketplaces = PluginsManager::new(tmp.path().to_path_buf())
        .list_marketplaces_for_config(&config, &[AbsolutePathBuf::try_from(repo_root).unwrap()])
        .unwrap()
        .marketplaces;

    let marketplace = marketplaces
        .into_iter()
        .find(|marketplace| {
            marketplace.path
                == AbsolutePathBuf::try_from(
                    tmp.path().join("repo/.agents/plugins/marketplace.json"),
                )
                .unwrap()
        })
        .expect("expected repo marketplace entry");

    assert_eq!(
        marketplace,
        ConfiguredMarketplace {
            name: "debug".to_string(),
            path: AbsolutePathBuf::try_from(
                tmp.path().join("repo/.agents/plugins/marketplace.json"),
            )
            .unwrap(),
            interface: None,
            plugins: vec![ConfiguredMarketplacePlugin {
                id: "sample-plugin@debug".to_string(),
                name: "sample-plugin".to_string(),
                local_version: None,
                source: MarketplacePluginSource::Local {
                    path: AbsolutePathBuf::try_from(tmp.path().join("repo/sample-plugin")).unwrap(),
                },
                policy: MarketplacePluginPolicy {
                    installation: MarketplacePluginInstallPolicy::Available,
                    authentication: MarketplacePluginAuthPolicy::OnInstall,
                    products: None,
                },
                interface: None,
                keywords: Vec::new(),
                installed: false,
                enabled: true,
            }],
        }
    );
}

