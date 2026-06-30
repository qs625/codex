use super::*;
use crate::config::CONFIG_TOML_FILE;
use crate::config::ConfigBuilder;
use codex_config::AppRequirementToml;
use codex_config::AppToolRequirementToml;
use codex_config::AppToolsRequirementsToml;
use codex_config::AppsRequirementsToml;
use codex_config::CloudRequirementsLoader;
use codex_config::ConfigLayerStack;
use codex_config::ConfigRequirements;
use codex_config::ConfigRequirementsToml;
use codex_config::types::AppConfig;
use codex_config::types::AppToolConfig;
use codex_config::types::AppToolsConfig;
use codex_config::types::AppsDefaultConfig;
use codex_connectors_api::merge::plugin_connector_to_app_info;
use codex_core_plugins::PluginsManager;
use codex_login::CodexAuth;
use codex_mcp_tool_types::ToolAnnotations;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::collections::HashMap;
use tempfile::tempdir;

fn annotations(destructive_hint: Option<bool>, open_world_hint: Option<bool>) -> ToolAnnotations {
    ToolAnnotations {
        destructive_hint,
        idempotent_hint: None,
        open_world_hint,
        read_only_hint: None,
        title: None,
    }
}

fn app(id: &str) -> AppInfo {
    AppInfo {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        logo_url: None,
        logo_url_dark: None,
        distribution_channel: None,
        install_url: None,
        branding: None,
        app_metadata: None,
        labels: None,
        is_accessible: false,
        is_enabled: true,
        plugin_display_names: Vec::new(),
    }
}

#[test]
fn app_tool_policy_uses_global_defaults_for_destructive_hints() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: true,
            destructive_enabled: false,
            open_world_enabled: true,
        }),
        apps: HashMap::new(),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/create",
        /*tool_title*/ None,
        Some(&annotations(Some(true), /*open_world_hint*/ None)),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_defaults_missing_destructive_hint_to_true() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: true,
            destructive_enabled: false,
            open_world_enabled: true,
        }),
        apps: HashMap::new(),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/create",
        /*tool_title*/ None,
        Some(&annotations(/*destructive_hint*/ None, Some(false))),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_defaults_missing_open_world_hint_to_true() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: true,
            destructive_enabled: true,
            open_world_enabled: false,
        }),
        apps: HashMap::new(),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/create",
        /*tool_title*/ None,
        Some(&annotations(Some(false), /*open_world_hint*/ None)),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_is_enabled_uses_default_for_unconfigured_apps() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: false,
            destructive_enabled: true,
            open_world_enabled: true,
        }),
        apps: HashMap::new(),
    };

    assert!(!app_is_enabled(&apps_config, Some("calendar")));
    assert!(!app_is_enabled(&apps_config, /*connector_id*/ None));
}

#[test]
fn app_is_enabled_prefers_per_app_override_over_default() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: false,
            destructive_enabled: true,
            open_world_enabled: true,
        }),
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: None,
                open_world_enabled: None,
                default_tools_approval_mode: None,
                default_tools_enabled: None,
                tools: None,
            },
        )]),
    };

    assert!(app_is_enabled(&apps_config, Some("calendar")));
    assert!(!app_is_enabled(&apps_config, Some("drive")));
}

#[test]
fn requirements_disabled_connector_overrides_enabled_connector() {
    let mut effective_apps = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "connector_123123".to_string(),
            AppConfig {
                enabled: true,
                ..Default::default()
            },
        )]),
    };
    let requirements_apps = AppsRequirementsToml {
        apps: BTreeMap::from([(
            "connector_123123".to_string(),
            AppRequirementToml {
                enabled: Some(false),
                tools: None,
            },
        )]),
    };

    apply_requirements_apps_constraints(&mut effective_apps, Some(&requirements_apps));

    assert_eq!(
        effective_apps
            .apps
            .get("connector_123123")
            .map(|app| app.enabled),
        Some(false)
    );
}

#[test]
fn requirements_enabled_does_not_override_disabled_connector() {
    let mut effective_apps = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "connector_123123".to_string(),
            AppConfig {
                enabled: false,
                ..Default::default()
            },
        )]),
    };
    let requirements_apps = AppsRequirementsToml {
        apps: BTreeMap::from([(
            "connector_123123".to_string(),
            AppRequirementToml {
                enabled: Some(true),
                tools: None,
            },
        )]),
    };

    apply_requirements_apps_constraints(&mut effective_apps, Some(&requirements_apps));

    assert_eq!(
        effective_apps
            .apps
            .get("connector_123123")
            .map(|app| app.enabled),
        Some(false)
    );
}

#[tokio::test]
async fn cloud_requirements_disable_connector_overrides_user_apps_config() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[apps.connector_123123]
enabled = true
"#,
    )
    .expect("write config");

    let requirements = ConfigRequirementsToml {
        apps: Some(AppsRequirementsToml {
            apps: BTreeMap::from([(
                "connector_123123".to_string(),
                AppRequirementToml {
                    enabled: Some(false),
                    tools: None,
                },
            )]),
        }),
        ..Default::default()
    };

    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await
        .expect("config should build");

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "events.list",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[tokio::test]
async fn cloud_requirements_disable_connector_applies_without_user_apps_table() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(codex_home.path().join(CONFIG_TOML_FILE), "").expect("write config");

    let requirements = ConfigRequirementsToml {
        apps: Some(AppsRequirementsToml {
            apps: BTreeMap::from([(
                "connector_123123".to_string(),
                AppRequirementToml {
                    enabled: Some(false),
                    tools: None,
                },
            )]),
        }),
        ..Default::default()
    };

    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await
        .expect("config should build");

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "events.list",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[tokio::test]
async fn local_requirements_disable_connector_overrides_user_apps_config() {
    let codex_home = tempdir().expect("tempdir should succeed");
    let config_toml_path =
        AbsolutePathBuf::try_from(codex_home.path().join(CONFIG_TOML_FILE)).expect("abs path");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect("config should build");

    let requirements = ConfigRequirementsToml {
        apps: Some(AppsRequirementsToml {
            apps: BTreeMap::from([(
                "connector_123123".to_string(),
                AppRequirementToml {
                    enabled: Some(false),
                    tools: None,
                },
            )]),
        }),
        ..Default::default()
    };
    config.config_layer_stack =
        ConfigLayerStack::new(Vec::new(), ConfigRequirements::default(), requirements)
            .expect("requirements stack")
            .with_user_config(
                &config_toml_path,
                toml::from_str::<toml::Value>(
                    r#"
[apps.connector_123123]
enabled = true
"#,
                )
                .expect("apps config"),
            );

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "events.list",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[tokio::test]
async fn local_requirements_disable_connector_applies_without_user_apps_table() {
    let codex_home = tempdir().expect("tempdir should succeed");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect("config should build");

    let requirements = ConfigRequirementsToml {
        apps: Some(AppsRequirementsToml {
            apps: BTreeMap::from([(
                "connector_123123".to_string(),
                AppRequirementToml {
                    enabled: Some(false),
                    tools: None,
                },
            )]),
        }),
        ..Default::default()
    };
    config.config_layer_stack =
        ConfigLayerStack::new(Vec::new(), ConfigRequirements::default(), requirements)
            .expect("requirements stack");

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "events.list",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[tokio::test]
async fn with_app_enabled_state_preserves_unrelated_disabled_connector() {
    let codex_home = tempdir().expect("tempdir should succeed");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect("config should build");

    let requirements = ConfigRequirementsToml {
        apps: Some(AppsRequirementsToml {
            apps: BTreeMap::from([(
                "connector_drive".to_string(),
                AppRequirementToml {
                    enabled: Some(false),
                    tools: None,
                },
            )]),
        }),
        ..Default::default()
    };
    config.config_layer_stack =
        ConfigLayerStack::new(Vec::new(), ConfigRequirements::default(), requirements)
            .expect("requirements stack");

    let mut slack = app("connector_slack");
    slack.is_enabled = false;

    let mut drive = app("connector_drive");
    drive.is_enabled = false;

    assert_eq!(
        with_app_enabled_state(vec![slack.clone(), app("connector_drive")], &config),
        vec![slack, drive]
    );
}

#[test]
fn app_tool_policy_honors_default_app_enabled_false() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: false,
            destructive_enabled: true,
            open_world_enabled: true,
        }),
        apps: HashMap::new(),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/list",
        /*tool_title*/ None,
        Some(&annotations(
            /*destructive_hint*/ None, /*open_world_hint*/ None,
        )),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_uses_managed_approval_without_apps_config() {
    let policy = app_tool_policy_from_apps_config(
        /*apps_config*/ None,
        Some("calendar"),
        "events/list",
        /*tool_title*/ None,
        /*annotations*/ None,
        Some(AppToolApproval::Approve),
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Approve,
        }
    );
}

fn app_tool_requirements(
    app_id: &str,
    tool_name: &str,
    approval_mode: AppToolApproval,
) -> AppsRequirementsToml {
    AppsRequirementsToml {
        apps: BTreeMap::from([(
            app_id.to_string(),
            AppRequirementToml {
                enabled: None,
                tools: Some(AppToolsRequirementsToml {
                    tools: BTreeMap::from([(
                        tool_name.to_string(),
                        AppToolRequirementToml {
                            approval_mode: Some(approval_mode),
                        },
                    )]),
                }),
            },
        )]),
    }
}

#[test]
fn managed_app_tool_approval_uses_raw_tool_name() {
    let requirements_apps = app_tool_requirements(
        "connector_123123",
        "calendar/list_events",
        AppToolApproval::Approve,
    );

    assert_eq!(
        managed_app_tool_approval(
            Some(&requirements_apps),
            Some("connector_123123"),
            "calendar/list_events",
        ),
        Some(AppToolApproval::Approve)
    );
    assert_eq!(
        managed_app_tool_approval(
            Some(&requirements_apps),
            Some("connector_123123"),
            "calendar/create_event",
        ),
        None
    );
}

#[tokio::test]
async fn cloud_requirements_tool_approval_overrides_user_apps_config() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[apps.connector_123123.tools."calendar/list_events"]
approval_mode = "prompt"
"#,
    )
    .expect("write config");

    let requirements = ConfigRequirementsToml {
        apps: Some(app_tool_requirements(
            "connector_123123",
            "calendar/list_events",
            AppToolApproval::Approve,
        )),
        ..Default::default()
    };

    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .cloud_requirements(CloudRequirementsLoader::new(async move {
            Ok(Some(requirements))
        }))
        .build()
        .await
        .expect("config should build");

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "calendar/list_events",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Approve,
        }
    );
}

#[tokio::test]
async fn local_requirements_tool_approval_overrides_user_apps_config() {
    let codex_home = tempdir().expect("tempdir should succeed");
    let config_toml_path =
        AbsolutePathBuf::try_from(codex_home.path().join(CONFIG_TOML_FILE)).expect("abs path");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect("config should build");

    let requirements = ConfigRequirementsToml {
        apps: Some(app_tool_requirements(
            "connector_123123",
            "calendar/list_events",
            AppToolApproval::Approve,
        )),
        ..Default::default()
    };
    config.config_layer_stack =
        ConfigLayerStack::new(Vec::new(), ConfigRequirements::default(), requirements)
            .expect("requirements stack")
            .with_user_config(
                &config_toml_path,
                toml::from_str::<toml::Value>(
                    r#"
[apps.connector_123123.tools."calendar/list_events"]
approval_mode = "prompt"
"#,
                )
                .expect("apps config"),
            );

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "calendar/list_events",
        /*tool_title*/ None,
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Approve,
        }
    );
}

#[tokio::test]
async fn local_requirements_tool_approval_does_not_match_tool_title() {
    let codex_home = tempdir().expect("tempdir should succeed");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .fallback_cwd(Some(codex_home.path().to_path_buf()))
        .build()
        .await
        .expect("config should build");

    let requirements = ConfigRequirementsToml {
        apps: Some(app_tool_requirements(
            "connector_123123",
            "calendar/list_events",
            AppToolApproval::Approve,
        )),
        ..Default::default()
    };
    config.config_layer_stack =
        ConfigLayerStack::new(Vec::new(), ConfigRequirements::default(), requirements)
            .expect("requirements stack");

    let policy = app_tool_policy(
        &config,
        Some("connector_123123"),
        "calendar/create_event",
        Some("calendar/list_events"),
        /*annotations*/ None,
    );
    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_allows_per_app_enable_when_default_is_disabled() {
    let apps_config = AppsConfigToml {
        default: Some(AppsDefaultConfig {
            enabled: false,
            destructive_enabled: true,
            open_world_enabled: true,
        }),
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: None,
                open_world_enabled: None,
                default_tools_approval_mode: None,
                default_tools_enabled: None,
                tools: None,
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/list",
        /*tool_title*/ None,
        Some(&annotations(
            /*destructive_hint*/ None, /*open_world_hint*/ None,
        )),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_per_tool_enabled_true_overrides_app_level_disable_flags() {
    let apps_config = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: Some(false),
                open_world_enabled: Some(false),
                default_tools_approval_mode: None,
                default_tools_enabled: None,
                tools: Some(AppToolsConfig {
                    tools: HashMap::from([(
                        "events/create".to_string(),
                        AppToolConfig {
                            enabled: Some(true),
                            approval_mode: None,
                        },
                    )]),
                }),
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/create",
        /*tool_title*/ None,
        Some(&annotations(Some(true), Some(true))),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_default_tools_enabled_true_overrides_app_level_tool_hints() {
    let apps_config = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: Some(false),
                open_world_enabled: Some(false),
                default_tools_approval_mode: None,
                default_tools_enabled: Some(true),
                tools: None,
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/create",
        /*tool_title*/ None,
        Some(&annotations(Some(true), Some(true))),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Auto,
        }
    );
}

#[test]
fn app_tool_policy_default_tools_enabled_false_overrides_app_level_tool_hints() {
    let apps_config = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: Some(true),
                open_world_enabled: Some(true),
                default_tools_approval_mode: Some(AppToolApproval::Approve),
                default_tools_enabled: Some(false),
                tools: None,
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/list",
        /*tool_title*/ None,
        Some(&annotations(
            /*destructive_hint*/ None, /*open_world_hint*/ None,
        )),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: false,
            approval: AppToolApproval::Approve,
        }
    );
}

#[test]
fn app_tool_policy_uses_default_tools_approval_mode() {
    let apps_config = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: None,
                open_world_enabled: None,
                default_tools_approval_mode: Some(AppToolApproval::Prompt),
                default_tools_enabled: None,
                tools: Some(AppToolsConfig {
                    tools: HashMap::new(),
                }),
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "events/list",
        /*tool_title*/ None,
        Some(&annotations(
            /*destructive_hint*/ None, /*open_world_hint*/ None,
        )),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Prompt,
        }
    );
}

#[test]
fn app_tool_policy_matches_prefix_stripped_tool_name_for_tool_config() {
    let apps_config = AppsConfigToml {
        default: None,
        apps: HashMap::from([(
            "calendar".to_string(),
            AppConfig {
                enabled: true,
                destructive_enabled: Some(false),
                open_world_enabled: Some(false),
                default_tools_approval_mode: Some(AppToolApproval::Auto),
                default_tools_enabled: Some(false),
                tools: Some(AppToolsConfig {
                    tools: HashMap::from([(
                        "events/create".to_string(),
                        AppToolConfig {
                            enabled: Some(true),
                            approval_mode: Some(AppToolApproval::Approve),
                        },
                    )]),
                }),
            },
        )]),
    };

    let policy = app_tool_policy_from_apps_config(
        Some(&apps_config),
        Some("calendar"),
        "calendar_events/create",
        Some("events/create"),
        Some(&annotations(Some(true), Some(true))),
        /*managed_approval*/ None,
    );

    assert_eq!(
        policy,
        AppToolPolicy {
            enabled: true,
            approval: AppToolApproval::Approve,
        }
    );
}

#[tokio::test]
async fn tool_suggest_connector_ids_include_configured_tool_suggest_discoverables() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[tool_suggest]
discoverables = [
  { type = "connector", id = "connector_2128aebfecb84f64a069897515042a44" },
  { type = "plugin", id = "slack@openai-curated" },
  { type = "connector", id = "   " }
]
"#,
    )
    .expect("write config");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("config should load");
    let plugins_manager = PluginsManager::new(config.codex_home.to_path_buf());

    assert_eq!(
        tool_suggest_connector_ids(&config, &plugins_manager).await,
        HashSet::from(["connector_2128aebfecb84f64a069897515042a44".to_string()])
    );
}

#[tokio::test]
async fn tool_suggest_connector_ids_exclude_disabled_tool_suggestions() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[tool_suggest]
discoverables = [
  { type = "connector", id = "connector_calendar" },
  { type = "connector", id = "connector_gmail" }
]
disabled_tools = [
  { type = "connector", id = "connector_calendar" }
]
"#,
    )
    .expect("write config");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("config should load");
    let plugins_manager = PluginsManager::new(config.codex_home.to_path_buf());

    assert_eq!(
        tool_suggest_connector_ids(&config, &plugins_manager).await,
        HashSet::from(["connector_gmail".to_string()])
    );
}

#[tokio::test]
async fn tool_suggest_uses_connector_id_fallback_when_directory_cache_is_empty() {
    let codex_home = tempdir().expect("tempdir should succeed");
    std::fs::write(
        codex_home.path().join(CONFIG_TOML_FILE),
        r#"
[features]
apps = true

[tool_suggest]
discoverables = [
  { type = "connector", id = "connector_gmail" }
]
"#,
    )
    .expect("write config");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("config should load");
    let auth = CodexAuth::create_dummy_chatgpt_auth_for_testing();
    let auth_snapshot = auth.request_auth_snapshot();
    let auth_context = mcp_service::codex_apps_auth_context(Some(&auth_snapshot));

    let plugins_manager = PluginsManager::new(config.codex_home.to_path_buf());
    let discoverable_tools = list_tool_suggest_discoverable_tools_with_auth(
        &config,
        &plugins_manager,
        auth_context.as_ref(),
        &[],
    )
    .await
    .expect("discoverable tools should load");

    assert_eq!(
        discoverable_tools,
        vec![DiscoverableTool::from(plugin_connector_to_app_info(
            "connector_gmail".to_string(),
        ))]
    );
}
