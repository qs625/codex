use super::*;
use anyhow::Result;
use app_server_protocol::AppConfig;
use app_server_protocol::AppToolApproval;
use app_server_protocol::AppsConfig;
use app_server_protocol::AskForApproval;
use codex_utils_absolute_path::AbsolutePathBuf;
use config_service::CloudRequirementsLoader;
use config_service::FeatureRequirementsToml;
use config_service::LoaderOverrides;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use tempfile::{TempDir, tempdir};

fn config_path(tmp: &TempDir) -> std::path::PathBuf {
    tmp.path().join(CONFIG_TOML_FILE)
}

fn config_path_string(tmp: &TempDir) -> String {
    config_path(tmp).display().to_string()
}

fn unmanaged_service(tmp: &TempDir) -> ConfigManager {
    ConfigManager::without_managed_config_for_tests(tmp.path().to_path_buf())
}

fn managed_service(tmp: &TempDir, managed_path: std::path::PathBuf) -> ConfigManager {
    ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        vec![],
        LoaderOverrides::with_managed_config_path_for_tests(managed_path),
        CloudRequirementsLoader::default(),
    )
}

fn write_params(
    file_path: Option<String>,
    key_path: &str,
    value: serde_json::Value,
) -> ConfigValueWriteParams {
    ConfigValueWriteParams {
        file_path,
        key_path: key_path.to_string(),
        value,
        merge_strategy: MergeStrategy::Replace,
        expected_version: None,
    }
}

fn config_write_params(
    tmp: &TempDir,
    key_path: &str,
    value: serde_json::Value,
) -> ConfigValueWriteParams {
    write_params(Some(config_path_string(tmp)), key_path, value)
}

fn read_params(include_layers: bool) -> ConfigReadParams {
    ConfigReadParams {
        include_layers,
        cwd: None,
    }
}

fn feature_requirement_service(tmp: &TempDir) -> ConfigManager {
    ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        vec![],
        LoaderOverrides::without_managed_config_for_tests(),
        CloudRequirementsLoader::new(async {
            Ok(Some(ConfigRequirementsToml {
                feature_requirements: Some(FeatureRequirementsToml {
                    entries: BTreeMap::from([("personality".to_string(), true)]),
                }),
                ..Default::default()
            }))
        }),
    )
}

fn linear_server_write_params(
    path: &std::path::Path,
    value: serde_json::Value,
    merge_strategy: MergeStrategy,
) -> ConfigValueWriteParams {
    ConfigValueWriteParams {
        file_path: Some(path.display().to_string()),
        key_path: "mcp_servers.linear".to_string(),
        value,
        merge_strategy,
        expected_version: None,
    }
}

#[test]
fn config_write_request_preserves_batch_metadata_and_edit_order() {
    let request = ConfigWriteRequest::batch(ConfigBatchWriteParams {
        edits: vec![
            app_server_protocol::ConfigEdit {
                key_path: "model".to_string(),
                value: serde_json::json!("gpt-5.2"),
                merge_strategy: MergeStrategy::Replace,
            },
            app_server_protocol::ConfigEdit {
                key_path: "features.personality".to_string(),
                value: serde_json::json!(true),
                merge_strategy: MergeStrategy::Upsert,
            },
        ],
        file_path: Some("/tmp/config.toml".to_string()),
        expected_version: Some("sha256:expected".to_string()),
        reload_user_config: true,
    });

    assert_eq!(request.file_path.as_deref(), Some("/tmp/config.toml"));
    assert_eq!(request.expected_version.as_deref(), Some("sha256:expected"));
    assert_eq!(request.edits.len(), 2);
    assert_eq!(request.edits[0].key_path, "model");
    assert_eq!(request.edits[0].merge_strategy, MergeStrategy::Replace);
    assert_eq!(request.edits[1].key_path, "features.personality");
    assert_eq!(request.edits[1].merge_strategy, MergeStrategy::Upsert);
}

#[test]
fn config_write_target_allows_only_user_config_path() {
    let tmp = tempdir().expect("tempdir");
    let allowed = AbsolutePathBuf::from_absolute_path(config_path(&tmp)).expect("allowed path");
    let default_target = ConfigWriteTarget::resolve(None, allowed.clone()).expect("default target");
    assert_eq!(default_target.path, allowed);

    let disallowed = tmp.path().join("other.toml");
    let result = ConfigWriteTarget::resolve(
        Some(disallowed.display().to_string()),
        default_target.path.clone(),
    );
    let Err(err) = result else {
        panic!("non-user config path should be rejected");
    };
    assert_eq!(
        err.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigLayerReadonly)
    );
    assert_eq!(
        err.to_string(),
        "Only writes to the user config are allowed"
    );
}

#[test]
fn config_write_plan_tracks_segments_without_persisting_noops() {
    let user_config: TomlValue = toml::from_str(
        r#"model = "gpt-5.2"

[features]
personality = true
"#,
    )
    .expect("parse config");

    let plan = ConfigWritePlan::new(user_config)
        .apply_edits(vec![
            ConfigWriteEdit {
                key_path: "model".to_string(),
                value: serde_json::json!("gpt-5.2"),
                merge_strategy: MergeStrategy::Replace,
            },
            ConfigWriteEdit {
                key_path: "features.personality".to_string(),
                value: serde_json::json!(false),
                merge_strategy: MergeStrategy::Replace,
            },
            ConfigWriteEdit {
                key_path: "features.missing".to_string(),
                value: serde_json::Value::Null,
                merge_strategy: MergeStrategy::Replace,
            },
        ])
        .expect("edits apply");

    assert_eq!(
        plan.parsed_segments,
        vec![
            vec!["model".to_string()],
            vec!["features".to_string(), "personality".to_string()],
            vec!["features".to_string(), "missing".to_string()],
        ]
    );
    assert_eq!(
        value_at_path(
            &plan.user_config,
            &["features".to_string(), "personality".to_string()]
        ),
        Some(&TomlValue::Boolean(false))
    );
    assert_eq!(plan.config_edits.len(), 1);
    match &plan.config_edits[0] {
        ConfigEdit::SetPath { segments, value } => {
            assert_eq!(
                segments,
                &vec!["features".to_string(), "personality".to_string()]
            );
            assert_eq!(
                value.as_value().and_then(toml_edit::Value::as_bool),
                Some(false)
            );
        }
        other => panic!("expected SetPath edit, got {other:?}"),
    }
}

#[test]
fn toml_value_to_item_handles_nested_config_tables() {
    let config = r#"
[mcp_servers.docs]
command = "docs-server"

[mcp_servers.docs.http_headers]
X-Doc = "42"
"#;

    let value: TomlValue = toml::from_str(config).expect("parse config example");
    let item = toml_value_to_item(&value).expect("convert to toml_edit item");

    let root = item.as_table().expect("root table");
    assert!(!root.is_implicit(), "root table should be explicit");

    let mcp_servers = root
        .get("mcp_servers")
        .and_then(TomlItem::as_table)
        .expect("mcp_servers table");
    assert!(
        !mcp_servers.is_implicit(),
        "mcp_servers table should be explicit"
    );

    let docs = mcp_servers
        .get("docs")
        .and_then(TomlItem::as_table)
        .expect("docs table");
    assert_eq!(
        docs.get("command")
            .and_then(TomlItem::as_value)
            .and_then(toml_edit::Value::as_str),
        Some("docs-server")
    );

    let http_headers = docs
        .get("http_headers")
        .and_then(TomlItem::as_table)
        .expect("http_headers table");
    assert_eq!(
        http_headers
            .get("X-Doc")
            .and_then(TomlItem::as_value)
            .and_then(toml_edit::Value::as_str),
        Some("42")
    );
}

#[tokio::test]
async fn write_value_preserves_comments_and_order() -> Result<()> {
    let tmp = tempdir().expect("tempdir");
    let original = r#"# Codex user configuration
model = "gpt-5.2"
approval_policy = "on-request"

[notice]
# Preserve this comment
hide_full_access_warning = true

[features]
unified_exec = true
"#;
    std::fs::write(config_path(&tmp), original)?;

    let service = unmanaged_service(&tmp);
    service
        .write_value(config_write_params(
            &tmp,
            "features.personality",
            serde_json::json!(true),
        ))
        .await
        .expect("write succeeds");

    let updated = std::fs::read_to_string(config_path(&tmp)).expect("read config");
    let expected = r#"# Codex user configuration
model = "gpt-5.2"
approval_policy = "on-request"

[notice]
# Preserve this comment
hide_full_access_warning = true

[features]
unified_exec = true
personality = true
"#;
    assert_eq!(updated, expected);
    Ok(())
}

#[tokio::test]
async fn clear_missing_nested_config_is_noop() -> Result<()> {
    let tmp = tempdir().expect("tempdir");
    let path = config_path(&tmp);
    std::fs::write(&path, "")?;

    let service = unmanaged_service(&tmp);
    let response = service
        .write_value(write_params(
            Some(path.display().to_string()),
            "features.personality",
            serde_json::Value::Null,
        ))
        .await
        .expect("clear missing config succeeds");

    assert_eq!(response.status, WriteStatus::Ok);
    assert_eq!(response.overridden_metadata, None);
    assert_eq!(std::fs::read_to_string(&path)?, "");
    Ok(())
}

#[tokio::test]
async fn write_value_supports_nested_app_paths() -> Result<()> {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "")?;

    let service = unmanaged_service(&tmp);
    service
        .write_value(config_write_params(
            &tmp,
            "apps",
            serde_json::json!({
                "app1": {
                    "enabled": false,
                },
            }),
        ))
        .await
        .expect("write apps succeeds");

    service
        .write_value(config_write_params(
            &tmp,
            "apps.app1.default_tools_approval_mode",
            serde_json::json!("prompt"),
        ))
        .await
        .expect("write apps.app1.default_tools_approval_mode succeeds");

    let read = service
        .read(read_params(false))
        .await
        .expect("config read succeeds");

    assert_eq!(
        read.config.apps,
        Some(AppsConfig {
            default: None,
            apps: std::collections::HashMap::from([(
                "app1".to_string(),
                AppConfig {
                    enabled: false,
                    destructive_enabled: None,
                    open_world_enabled: None,
                    default_tools_approval_mode: Some(AppToolApproval::Prompt),
                    default_tools_enabled: None,
                    tools: None,
                },
            )]),
        })
    );

    Ok(())
}

#[tokio::test]
async fn write_value_supports_custom_mcp_server_default_tool_approval_mode() -> Result<()> {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(
        config_path(&tmp),
        "[mcp_servers.docs]\ncommand = \"docs-server\"\n",
    )?;

    let service = unmanaged_service(&tmp);
    service
        .write_value(config_write_params(
            &tmp,
            "mcp_servers.docs.default_tools_approval_mode",
            serde_json::json!("approve"),
        ))
        .await
        .expect("write mcp server default_tools_approval_mode succeeds");

    let contents = std::fs::read_to_string(config_path(&tmp))?;
    assert!(contents.contains("default_tools_approval_mode = \"approve\""));

    let read = service
        .read(read_params(false))
        .await
        .expect("config read succeeds");

    assert_eq!(
        read.config
            .additional
            .get("mcp_servers")
            .and_then(|servers| servers.get("docs"))
            .and_then(|docs| docs.get("default_tools_approval_mode")),
        Some(&serde_json::json!("approve"))
    );

    Ok(())
}

#[tokio::test]
async fn read_includes_origins_and_layers() {
    let tmp = tempdir().expect("tempdir");
    let user_path = config_path(&tmp);
    std::fs::write(&user_path, "model = \"user\"").unwrap();
    let user_file = AbsolutePathBuf::try_from(user_path.clone()).expect("user file");

    let managed_path = tmp.path().join("managed_config.toml");
    std::fs::write(&managed_path, "approval_policy = \"never\"").unwrap();
    let managed_file = AbsolutePathBuf::try_from(managed_path.clone()).expect("managed file");

    let service = managed_service(&tmp, managed_path.clone());

    let response = service.read(read_params(true)).await.expect("response");

    assert_eq!(response.config.approval_policy, Some(AskForApproval::Never));

    assert_eq!(
        response
            .origins
            .get("approval_policy")
            .expect("origin")
            .name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile {
            file: managed_file.clone()
        },
    );
    let layers = response.layers.expect("layers present");
    // Local macOS machines can surface an MDM-managed config layer at the
    // top of the stack; ignore it so this test stays focused on file/user/system ordering.
    let layers = if matches!(
        layers.first().map(|layer| &layer.name),
        Some(ConfigLayerSource::LegacyManagedConfigTomlFromMdm)
    ) {
        &layers[1..]
    } else {
        layers.as_slice()
    };
    assert_eq!(layers.len(), 3, "expected three layers");
    assert_eq!(
        layers.first().unwrap().name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile {
            file: managed_file.clone()
        }
    );
    assert_eq!(
        layers.get(1).unwrap().name,
        ConfigLayerSource::User {
            file: user_file.clone(),
            profile: None,
        }
    );
    assert!(matches!(
        layers.get(2).unwrap().name,
        ConfigLayerSource::System { .. }
    ));
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn write_value_succeeds_when_managed_preferences_expand_home_directory_paths() -> Result<()> {
    use base64::Engine;

    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "model = \"user\"\n")?;

    let mut loader_overrides =
        LoaderOverrides::with_managed_config_path_for_tests(tmp.path().join("managed_config.toml"));
    loader_overrides.managed_preferences_base64 = Some(
        base64::prelude::BASE64_STANDARD.encode(
            r#"
sandbox_mode = "workspace-write"
[sandbox_workspace_write]
writable_roots = ["~/code"]
"#
            .as_bytes(),
        ),
    );

    let service = ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        vec![],
        loader_overrides,
        CloudRequirementsLoader::default(),
    );

    let response = service
        .write_value(config_write_params(
            &tmp,
            "model",
            serde_json::json!("updated"),
        ))
        .await
        .expect("write succeeds");

    assert_eq!(response.status, WriteStatus::Ok);
    assert_eq!(
        std::fs::read_to_string(config_path(&tmp)).expect("read config"),
        "model = \"updated\"\n"
    );

    Ok(())
}

#[tokio::test]
async fn write_value_reports_override() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "approval_policy = \"on-request\"").unwrap();

    let managed_path = tmp.path().join("managed_config.toml");
    std::fs::write(&managed_path, "approval_policy = \"never\"").unwrap();
    let managed_file = AbsolutePathBuf::try_from(managed_path.clone()).expect("managed file");

    let service = managed_service(&tmp, managed_path.clone());

    let result = service
        .write_value(config_write_params(
            &tmp,
            "approval_policy",
            serde_json::json!("never"),
        ))
        .await
        .expect("result");

    let read_after = service.read(read_params(true)).await.expect("read");
    assert_eq!(
        read_after.config.approval_policy,
        Some(AskForApproval::Never)
    );
    assert_eq!(
        read_after
            .origins
            .get("approval_policy")
            .expect("origin")
            .name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile {
            file: managed_file.clone()
        }
    );
    assert_eq!(result.status, WriteStatus::Ok);
    assert!(result.overridden_metadata.is_none());
}

#[tokio::test]
async fn version_conflict_rejected() {
    let tmp = tempdir().expect("tempdir");
    let user_path = config_path(&tmp);
    std::fs::write(&user_path, "model = \"user\"").unwrap();

    let service = unmanaged_service(&tmp);
    let error = service
        .write_value(ConfigValueWriteParams {
            file_path: Some(config_path_string(&tmp)),
            key_path: "model".to_string(),
            value: serde_json::json!("gpt-5.2"),
            merge_strategy: MergeStrategy::Replace,
            expected_version: Some("sha256:bogus".to_string()),
        })
        .await
        .expect_err("should fail");

    assert_eq!(
        error.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigVersionConflict)
    );
}

#[tokio::test]
async fn write_value_defaults_to_user_config_path() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "").unwrap();

    let service = unmanaged_service(&tmp);
    service
        .write_value(write_params(None, "model", serde_json::json!("gpt-new")))
        .await
        .expect("write succeeds");

    let contents = std::fs::read_to_string(config_path(&tmp)).expect("read config");
    assert!(
        contents.contains("model = \"gpt-new\""),
        "config.toml should be updated even when file_path is omitted"
    );
}

#[tokio::test]
async fn write_value_defaults_to_selected_user_config_path() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "model = \"gpt-main\"").unwrap();
    let selected_path = tmp.path().join("work.config.toml");
    std::fs::write(&selected_path, "").unwrap();

    let mut loader_overrides =
        LoaderOverrides::with_managed_config_path_for_tests(tmp.path().join("managed_config.toml"));
    loader_overrides.user_config_path =
        Some(AbsolutePathBuf::from_absolute_path(&selected_path).expect("selected config path"));
    loader_overrides.user_config_profile = Some("work".parse().expect("profile name"));
    let service = ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        vec![],
        loader_overrides,
        CloudRequirementsLoader::default(),
    );
    service
        .write_value(write_params(None, "model", serde_json::json!("gpt-work")))
        .await
        .expect("write succeeds");

    assert_eq!(
        std::fs::read_to_string(&selected_path).expect("read selected config"),
        "model = \"gpt-work\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(config_path(&tmp)).expect("read main config"),
        "model = \"gpt-main\""
    );
}

#[tokio::test]
async fn load_default_config_preserves_selected_user_config_path_after_load_error() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "model = \"gpt-main\"").unwrap();
    let selected_path = tmp.path().join("work.config.toml");
    std::fs::write(&selected_path, "not valid toml").unwrap();
    let selected_file =
        AbsolutePathBuf::from_absolute_path(&selected_path).expect("selected config path");

    let mut loader_overrides =
        LoaderOverrides::with_managed_config_path_for_tests(tmp.path().join("managed_config.toml"));
    loader_overrides.user_config_path = Some(selected_file.clone());
    loader_overrides.user_config_profile = Some("work".parse().expect("profile name"));
    let service = ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        vec![],
        loader_overrides,
        CloudRequirementsLoader::default(),
    );

    service
        .load_latest_config(/*fallback_cwd*/ None)
        .await
        .expect_err("selected config should fail to load");
    let config = service
        .load_default_config()
        .await
        .expect("default config loads after selected config error");

    assert_eq!(
        config.config_layer_stack.get_user_config_file(),
        Some(&selected_file)
    );
}

#[tokio::test]
async fn invalid_user_value_rejected_even_if_overridden_by_managed() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "model = \"user\"").unwrap();

    let managed_path = tmp.path().join("managed_config.toml");
    std::fs::write(&managed_path, "approval_policy = \"never\"").unwrap();

    let service = managed_service(&tmp, managed_path);

    let error = service
        .write_value(config_write_params(
            &tmp,
            "approval_policy",
            serde_json::json!("bogus"),
        ))
        .await
        .expect_err("should fail validation");

    assert_eq!(
        error.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigValidationError)
    );

    let contents = std::fs::read_to_string(config_path(&tmp)).expect("read config");
    assert_eq!(contents.trim(), "model = \"user\"");
}

#[tokio::test]
async fn reserved_builtin_provider_override_rejected() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "model = \"user\"\n").unwrap();

    let service = unmanaged_service(&tmp);
    let error = service
        .write_value(config_write_params(
            &tmp,
            "model_providers.openai.name",
            serde_json::json!("OpenAI Override"),
        ))
        .await
        .expect_err("should reject reserved provider override");

    assert_eq!(
        error.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigValidationError)
    );
    assert!(error.to_string().contains("reserved built-in provider IDs"));
    assert!(error.to_string().contains("`openai`"));

    let contents = std::fs::read_to_string(config_path(&tmp)).expect("read config");
    assert_eq!(contents, "model = \"user\"\n");
}

#[tokio::test]
async fn write_value_rejects_feature_requirement_conflict() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "").unwrap();

    let service = feature_requirement_service(&tmp);

    let error = service
        .write_value(config_write_params(
            &tmp,
            "features.personality",
            serde_json::json!(false),
        ))
        .await
        .expect_err("conflicting feature write should fail");

    assert_eq!(
        error.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigValidationError)
    );
    assert!(
        error
            .to_string()
            .contains("invalid value for `features`: `features.personality=false`"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(config_path(&tmp)).unwrap(), "");
}

#[tokio::test]
async fn write_value_rejects_profile_feature_requirement_conflict() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "").unwrap();

    let service = feature_requirement_service(&tmp);

    let error = service
        .write_value(config_write_params(
            &tmp,
            "profiles.enterprise.features.personality",
            serde_json::json!(false),
        ))
        .await
        .expect_err("conflicting profile feature write should fail");

    assert_eq!(
        error.write_error_code(),
        Some(ConfigWriteErrorCode::ConfigValidationError)
    );
    assert!(
        error.to_string().contains(
            "invalid value for `features`: `profiles.enterprise.features.personality=false`"
        ),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(config_path(&tmp)).unwrap(), "");
}

#[tokio::test]
async fn read_reports_managed_overrides_user_and_session_flags() {
    let tmp = tempdir().expect("tempdir");
    let user_path = config_path(&tmp);
    std::fs::write(&user_path, "model = \"user\"").unwrap();
    let user_file = AbsolutePathBuf::try_from(user_path.clone()).expect("user file");

    let managed_path = tmp.path().join("managed_config.toml");
    std::fs::write(&managed_path, "model = \"system\"").unwrap();
    let managed_file = AbsolutePathBuf::try_from(managed_path.clone()).expect("managed file");

    let cli_overrides = vec![(
        "model".to_string(),
        TomlValue::String("session".to_string()),
    )];

    let service = ConfigManager::new_for_tests(
        tmp.path().to_path_buf(),
        cli_overrides,
        LoaderOverrides::with_managed_config_path_for_tests(managed_path.clone()),
        CloudRequirementsLoader::default(),
    );

    let response = service.read(read_params(true)).await.expect("response");

    assert_eq!(response.config.model.as_deref(), Some("system"));
    assert_eq!(
        response.origins.get("model").expect("origin").name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile {
            file: managed_file.clone()
        },
    );
    let layers = response.layers.expect("layers");
    // Local macOS machines can surface an MDM-managed config layer at the
    // top of the stack; ignore it so this test stays focused on file/session/user ordering.
    let layers = if matches!(
        layers.first().map(|layer| &layer.name),
        Some(ConfigLayerSource::LegacyManagedConfigTomlFromMdm)
    ) {
        &layers[1..]
    } else {
        layers.as_slice()
    };
    assert_eq!(
        layers.first().unwrap().name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { file: managed_file }
    );
    assert_eq!(layers.get(1).unwrap().name, ConfigLayerSource::SessionFlags);
    assert_eq!(
        layers.get(2).unwrap().name,
        ConfigLayerSource::User {
            file: user_file,
            profile: None
        }
    );
}

#[tokio::test]
async fn write_value_reports_managed_override() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(config_path(&tmp), "").unwrap();

    let managed_path = tmp.path().join("managed_config.toml");
    std::fs::write(&managed_path, "approval_policy = \"never\"").unwrap();
    let managed_file = AbsolutePathBuf::try_from(managed_path.clone()).expect("managed file");

    let service = managed_service(&tmp, managed_path.clone());

    let result = service
        .write_value(config_write_params(
            &tmp,
            "approval_policy",
            serde_json::json!("on-request"),
        ))
        .await
        .expect("result");

    assert_eq!(result.status, WriteStatus::OkOverridden);
    let overridden = result.overridden_metadata.expect("overridden metadata");
    assert_eq!(
        overridden.overriding_layer.name,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { file: managed_file }
    );
    assert_eq!(overridden.effective_value, serde_json::json!("never"));
}

#[tokio::test]
async fn upsert_merges_tables_replace_overwrites() -> Result<()> {
    let tmp = tempdir().expect("tempdir");
    let path = config_path(&tmp);
    let base = r#"[mcp_servers.linear]
bearer_token_env_var = "TOKEN"
name = "linear"
url = "https://linear.example"

[mcp_servers.linear.env_http_headers]
existing = "keep"

[mcp_servers.linear.http_headers]
alpha = "a"
"#;

    let overlay = serde_json::json!({
        "bearer_token_env_var": "NEW_TOKEN",
        "http_headers": {
            "alpha": "updated",
            "beta": "b"
        },
        "name": "linear",
        "url": "https://linear.example"
    });

    std::fs::write(&path, base)?;

    let service = unmanaged_service(&tmp);
    service
        .write_value(linear_server_write_params(
            &path,
            overlay.clone(),
            MergeStrategy::Upsert,
        ))
        .await
        .expect("upsert succeeds");

    let upserted: TomlValue = toml::from_str(&std::fs::read_to_string(&path)?)?;
    let expected_upsert: TomlValue = toml::from_str(
        r#"[mcp_servers.linear]
bearer_token_env_var = "NEW_TOKEN"
name = "linear"
url = "https://linear.example"

[mcp_servers.linear.env_http_headers]
existing = "keep"

[mcp_servers.linear.http_headers]
alpha = "updated"
beta = "b"
"#,
    )?;
    assert_eq!(upserted, expected_upsert);

    std::fs::write(&path, base)?;

    service
        .write_value(linear_server_write_params(
            &path,
            overlay,
            MergeStrategy::Replace,
        ))
        .await
        .expect("replace succeeds");

    let replaced: TomlValue = toml::from_str(&std::fs::read_to_string(&path)?)?;
    let expected_replace: TomlValue = toml::from_str(
        r#"[mcp_servers.linear]
bearer_token_env_var = "NEW_TOKEN"
name = "linear"
url = "https://linear.example"

[mcp_servers.linear.http_headers]
alpha = "updated"
beta = "b"
"#,
    )?;
    assert_eq!(replaced, expected_replace);

    Ok(())
}
