use super::*;

#[test]
fn skills_list_params_serialization_uses_force_reload() {
    assert_eq!(
        serde_json::to_value(SkillsListParams {
            cwds: Vec::new(),
            force_reload: false,
        })
        .unwrap(),
        json!({}),
    );

    assert_eq!(
        serde_json::to_value(SkillsListParams {
            cwds: vec![PathBuf::from("/repo")],
            force_reload: true,
        })
        .unwrap(),
        json!({
            "cwds": ["/repo"],
            "forceReload": true,
        }),
    );
}

#[test]
fn plugin_source_serializes_local_git_and_remote_variants() {
    let local_path = if cfg!(windows) {
        r"C:\plugins\linear"
    } else {
        "/plugins/linear"
    };
    let local_path = AbsolutePathBuf::try_from(PathBuf::from(local_path)).unwrap();
    let local_path_json = local_path.as_path().display().to_string();

    assert_eq!(
        serde_json::to_value(PluginSource::Local { path: local_path }).unwrap(),
        json!({
            "type": "local",
            "path": local_path_json,
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginSource::Git {
            url: "https://github.com/openai/example.git".to_string(),
            path: Some("plugins/example".to_string()),
            ref_name: Some("main".to_string()),
            sha: Some("abc123".to_string()),
        })
        .unwrap(),
        json!({
            "type": "git",
            "url": "https://github.com/openai/example.git",
            "path": "plugins/example",
            "refName": "main",
            "sha": "abc123",
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginSource::Remote).unwrap(),
        json!({
            "type": "remote",
        }),
    );
}

#[test]
fn marketplace_add_params_serialization_uses_optional_ref_name_and_sparse_paths() {
    assert_eq!(
        serde_json::to_value(MarketplaceAddParams {
            source: "owner/repo".to_string(),
            ref_name: None,
            sparse_paths: None,
        })
        .unwrap(),
        json!({
            "source": "owner/repo",
            "refName": null,
            "sparsePaths": null,
        }),
    );

    assert_eq!(
        serde_json::to_value(MarketplaceAddParams {
            source: "owner/repo".to_string(),
            ref_name: Some("main".to_string()),
            sparse_paths: Some(vec!["plugins/foo".to_string()]),
        })
        .unwrap(),
        json!({
            "source": "owner/repo",
            "refName": "main",
            "sparsePaths": ["plugins/foo"],
        }),
    );
}

#[test]
fn marketplace_upgrade_params_serialization_uses_optional_marketplace_name() {
    assert_eq!(
        serde_json::to_value(MarketplaceUpgradeParams {
            marketplace_name: None,
        })
        .unwrap(),
        json!({
            "marketplaceName": null,
        }),
    );

    assert_eq!(
        serde_json::from_value::<MarketplaceUpgradeParams>(json!({})).unwrap(),
        MarketplaceUpgradeParams {
            marketplace_name: None,
        },
    );

    assert_eq!(
        serde_json::to_value(MarketplaceUpgradeParams {
            marketplace_name: Some("debug".to_string()),
        })
        .unwrap(),
        json!({
            "marketplaceName": "debug",
        }),
    );
}

#[test]
fn plugin_marketplace_entry_serializes_remote_only_path_as_null() {
    assert_eq!(
        serde_json::to_value(PluginMarketplaceEntry {
            name: "openai-curated".to_string(),
            path: None,
            interface: None,
            plugins: Vec::new(),
        })
        .unwrap(),
        json!({
            "name": "openai-curated",
            "path": null,
            "interface": null,
            "plugins": [],
        }),
    );
}

#[test]
fn plugin_interface_serializes_local_paths_and_remote_urls_separately() {
    let composer_icon = if cfg!(windows) {
        r"C:\plugins\linear\icon.png"
    } else {
        "/plugins/linear/icon.png"
    };
    let composer_icon = AbsolutePathBuf::try_from(PathBuf::from(composer_icon)).unwrap();
    let composer_icon_json = composer_icon.as_path().display().to_string();

    let interface = PluginInterface {
        display_name: Some("Linear".to_string()),
        short_description: None,
        long_description: None,
        developer_name: None,
        category: Some("Productivity".to_string()),
        capabilities: Vec::new(),
        website_url: None,
        privacy_policy_url: None,
        terms_of_service_url: None,
        default_prompt: None,
        brand_color: None,
        composer_icon: Some(composer_icon),
        composer_icon_url: Some("https://example.com/linear/icon.png".to_string()),
        logo: None,
        logo_url: Some("https://example.com/linear/logo.png".to_string()),
        screenshots: Vec::new(),
        screenshot_urls: vec!["https://example.com/linear/screenshot.png".to_string()],
    };

    assert_eq!(
        serde_json::to_value(interface).unwrap(),
        json!({
            "displayName": "Linear",
            "shortDescription": null,
            "longDescription": null,
            "developerName": null,
            "category": "Productivity",
            "capabilities": [],
            "websiteUrl": null,
            "privacyPolicyUrl": null,
            "termsOfServiceUrl": null,
            "defaultPrompt": null,
            "brandColor": null,
            "composerIcon": composer_icon_json,
            "composerIconUrl": "https://example.com/linear/icon.png",
            "logo": null,
            "logoUrl": "https://example.com/linear/logo.png",
            "screenshots": [],
            "screenshotUrls": ["https://example.com/linear/screenshot.png"],
        }),
    );
}

#[test]
fn plugin_list_params_ignore_removed_force_remote_sync_field() {
    assert_eq!(
        serde_json::from_value::<PluginListParams>(json!({
            "cwds": null,
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginListParams {
            cwds: None,
            marketplace_kinds: None,
        },
    );
}

#[test]
fn plugin_list_params_serializes_marketplace_kind_filter() {
    assert_eq!(
        serde_json::to_value(PluginListParams {
            cwds: None,
            marketplace_kinds: Some(vec![
                PluginListMarketplaceKind::Local,
                PluginListMarketplaceKind::WorkspaceDirectory,
                PluginListMarketplaceKind::SharedWithMe,
            ]),
        })
        .unwrap(),
        json!({
            "cwds": null,
            "marketplaceKinds": [
                "local",
                "workspace-directory",
                "shared-with-me",
            ],
        }),
    );
}

#[test]
fn plugin_read_params_serialization_uses_install_source_fields() {
    let marketplace_path = if cfg!(windows) {
        r"C:\plugins\marketplace.json"
    } else {
        "/plugins/marketplace.json"
    };
    let marketplace_path = AbsolutePathBuf::try_from(PathBuf::from(marketplace_path)).unwrap();
    let marketplace_path_json = marketplace_path.as_path().display().to_string();
    assert_eq!(
        serde_json::to_value(PluginReadParams {
            marketplace_path: Some(marketplace_path.clone()),
            remote_marketplace_name: None,
            plugin_name: "gmail".to_string(),
        })
        .unwrap(),
        json!({
            "marketplacePath": marketplace_path_json,
            "remoteMarketplaceName": null,
            "pluginName": "gmail",
        }),
    );

    assert_eq!(
        serde_json::from_value::<PluginReadParams>(json!({
            "marketplacePath": marketplace_path_json,
            "pluginName": "gmail",
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginReadParams {
            marketplace_path: Some(marketplace_path),
            remote_marketplace_name: None,
            plugin_name: "gmail".to_string(),
        },
    );

    assert_eq!(
        serde_json::from_value::<PluginReadParams>(json!({
            "remoteMarketplaceName": "openai-curated",
            "pluginName": "gmail",
        }))
        .unwrap(),
        PluginReadParams {
            marketplace_path: None,
            remote_marketplace_name: Some("openai-curated".to_string()),
            plugin_name: "gmail".to_string(),
        },
    );
}

#[test]
fn plugin_install_params_serialization_omits_force_remote_sync() {
    let marketplace_path = if cfg!(windows) {
        r"C:\plugins\marketplace.json"
    } else {
        "/plugins/marketplace.json"
    };
    let marketplace_path = AbsolutePathBuf::try_from(PathBuf::from(marketplace_path)).unwrap();
    let marketplace_path_json = marketplace_path.as_path().display().to_string();
    assert_eq!(
        serde_json::to_value(PluginInstallParams {
            marketplace_path: Some(marketplace_path.clone()),
            remote_marketplace_name: None,
            plugin_name: "gmail".to_string(),
        })
        .unwrap(),
        json!({
            "marketplacePath": marketplace_path_json,
            "remoteMarketplaceName": null,
            "pluginName": "gmail",
        }),
    );

    assert_eq!(
        serde_json::from_value::<PluginInstallParams>(json!({
            "marketplacePath": marketplace_path_json,
            "pluginName": "gmail",
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginInstallParams {
            marketplace_path: Some(marketplace_path),
            remote_marketplace_name: None,
            plugin_name: "gmail".to_string(),
        },
    );

    assert_eq!(
        serde_json::from_value::<PluginInstallParams>(json!({
            "remoteMarketplaceName": "openai-curated",
            "pluginName": "gmail",
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginInstallParams {
            marketplace_path: None,
            remote_marketplace_name: Some("openai-curated".to_string()),
            plugin_name: "gmail".to_string(),
        },
    );
}

#[test]
fn plugin_skill_read_params_serialization_uses_remote_plugin_id() {
    assert_eq!(
        serde_json::to_value(PluginSkillReadParams {
            remote_marketplace_name: "chatgpt-global".to_string(),
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
            skill_name: "plan-work".to_string(),
        })
        .unwrap(),
        json!({
            "remoteMarketplaceName": "chatgpt-global",
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
            "skillName": "plan-work",
        }),
    );
}

#[test]
fn plugin_share_params_and_response_serialization_use_camel_case_fields() {
    let plugin_path = if cfg!(windows) {
        r"C:\plugins\gmail"
    } else {
        "/plugins/gmail"
    };
    let plugin_path = AbsolutePathBuf::try_from(PathBuf::from(plugin_path)).unwrap();
    let plugin_path_json = plugin_path.as_path().display().to_string();

    assert_eq!(
        serde_json::to_value(PluginShareSaveParams {
            plugin_path: plugin_path.clone(),
            remote_plugin_id: None,
            discoverability: None,
            share_targets: None,
        })
        .unwrap(),
        json!({
            "pluginPath": plugin_path_json,
            "remotePluginId": null,
            "discoverability": null,
            "shareTargets": null,
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginShareSaveParams {
            plugin_path,
            remote_plugin_id: Some("plugins~Plugin_00000000000000000000000000000000".to_string(),),
            discoverability: Some(PluginShareDiscoverability::Private),
            share_targets: Some(vec![
                PluginShareTarget {
                    principal_type: PluginSharePrincipalType::User,
                    principal_id: "user-1".to_string(),
                    role: PluginShareTargetRole::Reader,
                },
                PluginShareTarget {
                    principal_type: PluginSharePrincipalType::Group,
                    principal_id: "group-1".to_string(),
                    role: PluginShareTargetRole::Reader,
                },
            ]),
        })
        .unwrap(),
        json!({
            "pluginPath": plugin_path_json,
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
            "discoverability": "PRIVATE",
            "shareTargets": [
                {
                    "principalType": "user",
                    "principalId": "user-1",
                    "role": "reader",
                },
                {
                    "principalType": "group",
                    "principalId": "group-1",
                    "role": "reader",
                },
            ],
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginShareSaveResponse {
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
            share_url: String::new(),
        })
        .unwrap(),
        json!({
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
            "shareUrl": "",
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginShareUpdateTargetsParams {
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
            discoverability: PluginShareUpdateDiscoverability::Unlisted,
            share_targets: vec![PluginShareTarget {
                principal_type: PluginSharePrincipalType::Group,
                principal_id: "group-1".to_string(),
                role: PluginShareTargetRole::Editor,
            }],
        })
        .unwrap(),
        json!({
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
            "discoverability": "UNLISTED",
            "shareTargets": [{
                "principalType": "group",
                "principalId": "group-1",
                "role": "editor",
            }],
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginShareUpdateTargetsResponse {
            principals: vec![PluginSharePrincipal {
                principal_type: PluginSharePrincipalType::User,
                principal_id: "user-1".to_string(),
                role: PluginSharePrincipalRole::Owner,
                name: "Gavin".to_string(),
            }],
            discoverability: PluginShareDiscoverability::Unlisted,
        })
        .unwrap(),
        json!({
            "principals": [{
                "principalType": "user",
                "principalId": "user-1",
                "role": "owner",
                "name": "Gavin",
            }],
            "discoverability": "UNLISTED",
        }),
    );

    assert_eq!(
        serde_json::from_value::<PluginShareListParams>(json!({})).unwrap(),
        PluginShareListParams {},
    );

    assert_eq!(
        serde_json::to_value(PluginShareCheckoutParams {
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
        })
        .unwrap(),
        json!({
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
        }),
    );

    let plugin_path = if cfg!(windows) {
        r"C:\Users\me\plugins\gmail"
    } else {
        "/Users/me/plugins/gmail"
    };
    let plugin_path = AbsolutePathBuf::try_from(PathBuf::from(plugin_path)).unwrap();
    let plugin_path_json = plugin_path.as_path().display().to_string();
    let marketplace_path = if cfg!(windows) {
        r"C:\Users\me\.agents\plugins\marketplace.json"
    } else {
        "/Users/me/.agents/plugins/marketplace.json"
    };
    let marketplace_path = AbsolutePathBuf::try_from(PathBuf::from(marketplace_path)).unwrap();
    let marketplace_path_json = marketplace_path.as_path().display().to_string();
    assert_eq!(
        serde_json::to_value(PluginShareCheckoutResponse {
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
            plugin_id: "gmail@codex-curated".to_string(),
            plugin_name: "gmail".to_string(),
            plugin_path,
            marketplace_name: "codex-curated".to_string(),
            marketplace_path,
            remote_version: Some("1.2.3".to_string()),
        })
        .unwrap(),
        json!({
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
            "pluginId": "gmail@codex-curated",
            "pluginName": "gmail",
            "pluginPath": plugin_path_json,
            "marketplaceName": "codex-curated",
            "marketplacePath": marketplace_path_json,
            "remoteVersion": "1.2.3",
        }),
    );

    assert_eq!(
        serde_json::to_value(PluginShareDeleteParams {
            remote_plugin_id: "plugins~Plugin_00000000000000000000000000000000".to_string(),
        })
        .unwrap(),
        json!({
            "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
        }),
    );
}

#[test]
fn plugin_share_list_response_serializes_share_items() {
    assert_eq!(
        serde_json::to_value(PluginShareListResponse {
            data: vec![PluginShareListItem {
                plugin: PluginSummary {
                    id: "gmail@chatgpt-global".to_string(),
                    remote_plugin_id: Some(
                        "plugins~Plugin_00000000000000000000000000000000".to_string(),
                    ),
                    local_version: None,
                    name: "gmail".to_string(),
                    share_context: None,
                    source: PluginSource::Remote,
                    installed: false,
                    enabled: false,
                    install_policy: PluginInstallPolicy::Available,
                    auth_policy: PluginAuthPolicy::OnUse,
                    availability: PluginAvailability::Available,
                    interface: None,
                    keywords: Vec::new(),
                },
                local_plugin_path: None,
            }],
        })
        .unwrap(),
        json!({
            "data": [{
                "plugin": {
                    "id": "gmail@chatgpt-global",
                    "remotePluginId": "plugins~Plugin_00000000000000000000000000000000",
                    "localVersion": null,
                    "name": "gmail",
                    "shareContext": null,
                    "source": { "type": "remote" },
                    "installed": false,
                    "enabled": false,
                    "installPolicy": "AVAILABLE",
                    "authPolicy": "ON_USE",
                    "availability": "AVAILABLE",
                    "interface": null,
                    "keywords": [],
                },
                "localPluginPath": null,
            }],
        }),
    );
}

#[test]
fn plugin_summary_defaults_missing_availability_to_available() {
    let summary: PluginSummary = serde_json::from_value(json!({
        "id": "plugins~Plugin_00000000000000000000000000000000",
        "name": "gmail",
        "source": { "type": "remote" },
        "installed": false,
        "enabled": false,
        "installPolicy": "AVAILABLE",
        "authPolicy": "ON_USE",
        "interface": null,
    }))
    .unwrap();

    assert_eq!(summary.availability, PluginAvailability::Available);
    assert_eq!(summary.local_version, None);
    assert_eq!(summary.share_context, None);
}

#[test]
fn plugin_availability_deserializes_enabled_alias() {
    let availability: PluginAvailability = serde_json::from_value(json!("ENABLED")).unwrap();

    assert_eq!(availability, PluginAvailability::Available);
    assert_eq!(
        serde_json::to_value(availability).unwrap(),
        json!("AVAILABLE")
    );
}

#[test]
fn plugin_uninstall_params_serialization_omits_force_remote_sync() {
    assert_eq!(
        serde_json::to_value(PluginUninstallParams {
            plugin_id: "gmail@openai-curated".to_string(),
        })
        .unwrap(),
        json!({
            "pluginId": "gmail@openai-curated",
        }),
    );

    assert_eq!(
        serde_json::from_value::<PluginUninstallParams>(json!({
            "pluginId": "gmail@openai-curated",
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginUninstallParams {
            plugin_id: "gmail@openai-curated".to_string(),
        },
    );

    assert_eq!(
        serde_json::to_value(PluginUninstallParams {
            plugin_id: "plugins~Plugin_gmail".to_string(),
        })
        .unwrap(),
        json!({
            "pluginId": "plugins~Plugin_gmail",
        }),
    );

    assert_eq!(
        serde_json::from_value::<PluginUninstallParams>(json!({
            "pluginId": "plugins~Plugin_gmail",
            "forceRemoteSync": true,
        }))
        .unwrap(),
        PluginUninstallParams {
            plugin_id: "plugins~Plugin_gmail".to_string(),
        },
    );
}

#[test]
fn marketplace_remove_response_serializes_nullable_installed_root() {
    let installed_root = if cfg!(windows) {
        r"C:\marketplaces\debug"
    } else {
        "/tmp/marketplaces/debug"
    };
    let installed_root = AbsolutePathBuf::try_from(PathBuf::from(installed_root)).unwrap();
    let installed_root_json = installed_root.as_path().display().to_string();
    assert_eq!(
        serde_json::to_value(MarketplaceRemoveResponse {
            marketplace_name: "debug".to_string(),
            installed_root: Some(installed_root),
        })
        .unwrap(),
        json!({
            "marketplaceName": "debug",
            "installedRoot": installed_root_json,
        }),
    );

    assert_eq!(
        serde_json::to_value(MarketplaceRemoveResponse {
            marketplace_name: "debug".to_string(),
            installed_root: None,
        })
        .unwrap(),
        json!({
            "marketplaceName": "debug",
            "installedRoot": null,
        }),
    );
}

#[test]
fn marketplace_upgrade_response_serializes_camel_case_fields() {
    let upgraded_root = if cfg!(windows) {
        r"C:\marketplaces\debug"
    } else {
        "/tmp/marketplaces/debug"
    };
    let upgraded_root = AbsolutePathBuf::try_from(PathBuf::from(upgraded_root)).unwrap();
    let upgraded_root_json = upgraded_root.as_path().display().to_string();

    assert_eq!(
        serde_json::to_value(MarketplaceUpgradeResponse {
            selected_marketplaces: vec!["debug".to_string()],
            upgraded_roots: vec![upgraded_root],
            errors: vec![MarketplaceUpgradeErrorInfo {
                marketplace_name: "broken".to_string(),
                message: "failed to clone".to_string(),
            }],
        })
        .unwrap(),
        json!({
            "selectedMarketplaces": ["debug"],
            "upgradedRoots": [upgraded_root_json],
            "errors": [{
                "marketplaceName": "broken",
                "message": "failed to clone",
            }],
        }),
    );
}

#[test]
fn codex_error_info_serializes_http_status_code_in_camel_case() {
    let value = CodexErrorInfo::ResponseTooManyFailedAttempts {
        http_status_code: Some(401),
    };

    assert_eq!(
        serde_json::to_value(value).unwrap(),
        json!({
            "responseTooManyFailedAttempts": {
                "httpStatusCode": 401
            }
        })
    );
}

#[test]
fn codex_error_info_serializes_cyber_policy_in_camel_case() {
    assert_eq!(
        serde_json::to_value(CodexErrorInfo::CyberPolicy).unwrap(),
        json!("cyberPolicy")
    );
}

#[test]
fn codex_error_info_serializes_active_turn_not_steerable_turn_kind_in_camel_case() {
    let value = CodexErrorInfo::ActiveTurnNotSteerable {
        turn_kind: NonSteerableTurnKind::Review,
    };

    assert_eq!(
        serde_json::to_value(value).unwrap(),
        json!({
            "activeTurnNotSteerable": {
                "turnKind": "review"
            }
        })
    );
}

#[test]
fn dynamic_tool_response_serializes_content_items() {
    let value = serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![DynamicToolCallOutputContentItem::InputText {
            text: "dynamic-ok".to_string(),
        }],
        success: true,
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "contentItems": [
                {
                    "type": "inputText",
                    "text": "dynamic-ok"
                }
            ],
            "success": true,
        })
    );
}

#[test]
fn dynamic_tool_response_serializes_text_and_image_content_items() {
    let value = serde_json::to_value(DynamicToolCallResponse {
        content_items: vec![
            DynamicToolCallOutputContentItem::InputText {
                text: "dynamic-ok".to_string(),
            },
            DynamicToolCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,AAA".to_string(),
            },
        ],
        success: true,
    })
    .unwrap();

    assert_eq!(
        value,
        json!({
            "contentItems": [
                {
                    "type": "inputText",
                    "text": "dynamic-ok"
                },
                {
                    "type": "inputImage",
                    "imageUrl": "data:image/png;base64,AAA"
                }
            ],
            "success": true,
        })
    );
}

#[test]
fn dynamic_tool_spec_deserializes_defer_loading() {
    let value = json!({
        "name": "lookup_ticket",
        "description": "Fetch a ticket",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": { "type": "string" }
            }
        },
        "deferLoading": true,
    });

    let actual: DynamicToolSpec = serde_json::from_value(value).expect("deserialize");

    assert_eq!(
        actual,
        DynamicToolSpec {
            namespace: None,
            name: "lookup_ticket".to_string(),
            description: "Fetch a ticket".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                }
            }),
            defer_loading: true,
        }
    );
}

#[test]
fn dynamic_tool_spec_legacy_expose_to_context_inverts_to_defer_loading() {
    let value = json!({
        "name": "lookup_ticket",
        "description": "Fetch a ticket",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "exposeToContext": false,
    });

    let actual: DynamicToolSpec = serde_json::from_value(value).expect("deserialize");

    assert!(actual.defer_loading);
}
