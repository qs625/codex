use std::fs;

use codex_config_edit::CONFIG_TOML_FILE;
use crate::OPENAI_CURATED_MARKETPLACE_NAME;
use crate::PluginsManager;
use crate::startup_sync::curated_plugins_repo_path;
use crate::store::PluginStoreError;
use codex_login::CodexAuth;
use codex_protocol::protocol::Product;
use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;
use wiremock::matchers::query_param;

use crate::PluginRemoteSyncError;
use crate::RemotePluginAuth;
use crate::RemotePluginSyncResult;
use crate::featured_plugin_ids_for_config;
use crate::sync_plugins_from_remote;
use crate::test_support::TEST_CURATED_PLUGIN_CACHE_VERSION;
use crate::test_support::load_plugins_config;
use crate::test_support::write_curated_plugin_sha;
use crate::test_support::write_file;
use crate::test_support::write_openai_curated_marketplace;
use crate::test_support::write_plugin;

fn test_remote_plugin_auth() -> RemotePluginAuth {
    let auth = CodexAuth::create_dummy_chatgpt_auth_for_testing();
    RemotePluginAuth::new(
        auth.request_auth_snapshot(),
        auth.get_account_id(),
        auth.get_chatgpt_user_id(),
        auth.is_workspace_account(),
    )
}

#[tokio::test]
async fn sync_plugins_from_remote_returns_default_when_feature_disabled() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = false
"#,
    );

    let config = load_plugins_config(tmp.path());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let outcome = sync_plugins_from_remote(&manager, &config, /*auth*/ None, false)
        .await
        .unwrap();

    assert_eq!(outcome, RemotePluginSyncResult::default());
}

#[tokio::test]
async fn sync_plugins_from_remote_reconciles_cache_and_config() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    write_openai_curated_marketplace(&curated_root, &["linear", "gmail", "calendar"]);
    write_curated_plugin_sha(tmp.path());
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "linear/local",
        "linear",
    );
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "gmail/local",
        "gmail",
    );
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "calendar/local",
        "calendar",
    );
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."linear@openai-curated"]
enabled = false

[plugins."gmail@openai-curated"]
enabled = false

[plugins."calendar@openai-curated"]
enabled = true
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/list"))
        .and(header("authorization", "Bearer Access Token"))
        .and(header("chatgpt-account-id", "account_id"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"linear","marketplace_name":"openai-curated","version":"1.0.0","enabled":true},
  {"id":"2","name":"gmail","marketplace_name":"openai-curated","version":"1.0.0","enabled":false}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let result =
        sync_plugins_from_remote(&manager, &config, Some(&test_remote_plugin_auth()), false)
            .await
            .unwrap();

    assert_eq!(
        result,
        RemotePluginSyncResult {
            installed_plugin_ids: Vec::new(),
            enabled_plugin_ids: vec!["linear@openai-curated".to_string()],
            disabled_plugin_ids: Vec::new(),
            uninstalled_plugin_ids: vec![
                "gmail@openai-curated".to_string(),
                "calendar@openai-curated".to_string(),
            ],
        }
    );

    assert!(
        tmp.path()
            .join("plugins/cache/openai-curated/linear/local")
            .is_dir()
    );
    assert!(
        !tmp.path()
            .join("plugins/cache/openai-curated/gmail")
            .exists()
    );
    assert!(
        !tmp.path()
            .join("plugins/cache/openai-curated/calendar")
            .exists()
    );

    let config = fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).unwrap();
    assert!(config.contains(r#"[plugins."linear@openai-curated"]"#));
    assert!(config.contains("enabled = true"));
    assert!(!config.contains(r#"[plugins."gmail@openai-curated"]"#));
    assert!(!config.contains(r#"[plugins."calendar@openai-curated"]"#));

    let synced_config = load_plugins_config(tmp.path());
    let curated_marketplace = manager
        .list_marketplaces_for_config(&synced_config, &[])
        .unwrap()
        .marketplaces
        .into_iter()
        .find(|marketplace| marketplace.name == OPENAI_CURATED_MARKETPLACE_NAME)
        .unwrap();
    assert_eq!(
        curated_marketplace
            .plugins
            .into_iter()
            .map(|plugin| (plugin.id, plugin.installed, plugin.enabled))
            .collect::<Vec<_>>(),
        vec![
            ("linear@openai-curated".to_string(), true, true),
            ("gmail@openai-curated".to_string(), false, false),
            ("calendar@openai-curated".to_string(), false, false),
        ]
    );
}

#[tokio::test]
async fn sync_plugins_from_remote_additive_only_keeps_existing_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    write_openai_curated_marketplace(&curated_root, &["linear", "gmail", "calendar"]);
    write_curated_plugin_sha(tmp.path());
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "linear/local",
        "linear",
    );
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "gmail/local",
        "gmail",
    );
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "calendar/local",
        "calendar",
    );
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."linear@openai-curated"]
enabled = false

[plugins."gmail@openai-curated"]
enabled = false

[plugins."calendar@openai-curated"]
enabled = true
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/list"))
        .and(header("authorization", "Bearer Access Token"))
        .and(header("chatgpt-account-id", "account_id"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"linear","marketplace_name":"openai-curated","version":"1.0.0","enabled":true},
  {"id":"2","name":"gmail","marketplace_name":"openai-curated","version":"1.0.0","enabled":false}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let result =
        sync_plugins_from_remote(&manager, &config, Some(&test_remote_plugin_auth()), true)
            .await
            .unwrap();

    assert_eq!(
        result,
        RemotePluginSyncResult {
            installed_plugin_ids: Vec::new(),
            enabled_plugin_ids: vec!["linear@openai-curated".to_string()],
            disabled_plugin_ids: Vec::new(),
            uninstalled_plugin_ids: Vec::new(),
        }
    );

    assert!(
        tmp.path()
            .join("plugins/cache/openai-curated/linear/local")
            .is_dir()
    );
    assert!(
        tmp.path()
            .join("plugins/cache/openai-curated/gmail/local")
            .is_dir()
    );
    assert!(
        tmp.path()
            .join("plugins/cache/openai-curated/calendar/local")
            .is_dir()
    );

    let config = fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).unwrap();
    assert!(config.contains(r#"[plugins."linear@openai-curated"]"#));
    assert!(config.contains(r#"[plugins."gmail@openai-curated"]"#));
    assert!(config.contains(r#"[plugins."calendar@openai-curated"]"#));
    assert!(config.contains("enabled = true"));
}

#[tokio::test]
async fn sync_plugins_from_remote_ignores_unknown_remote_plugins() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    write_openai_curated_marketplace(&curated_root, &["linear"]);
    write_curated_plugin_sha(tmp.path());
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."linear@openai-curated"]
enabled = false
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/list"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"plugin-one","marketplace_name":"openai-curated","version":"1.0.0","enabled":true}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let result =
        sync_plugins_from_remote(&manager, &config, Some(&test_remote_plugin_auth()), false)
            .await
            .unwrap();

    assert_eq!(
        result,
        RemotePluginSyncResult {
            installed_plugin_ids: Vec::new(),
            enabled_plugin_ids: Vec::new(),
            disabled_plugin_ids: Vec::new(),
            uninstalled_plugin_ids: vec!["linear@openai-curated".to_string()],
        }
    );
    let config = fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).unwrap();
    assert!(!config.contains(r#"[plugins."linear@openai-curated"]"#));
    assert!(
        !tmp.path()
            .join("plugins/cache/openai-curated/linear")
            .exists()
    );
}

#[tokio::test]
async fn sync_plugins_from_remote_keeps_existing_plugins_when_install_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    write_openai_curated_marketplace(&curated_root, &["linear", "gmail"]);
    write_curated_plugin_sha(tmp.path());
    fs::remove_dir_all(curated_root.join("plugins/gmail")).unwrap();
    write_plugin(
        &tmp.path().join("plugins/cache/openai-curated"),
        "linear/local",
        "linear",
    );
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true

[plugins."linear@openai-curated"]
enabled = false
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/list"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"gmail","marketplace_name":"openai-curated","version":"1.0.0","enabled":true}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let err = sync_plugins_from_remote(&manager, &config, Some(&test_remote_plugin_auth()), false)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        PluginRemoteSyncError::Store(PluginStoreError::Invalid(ref message))
            if message.contains("plugin source path is not a directory")
    ));
    assert!(
        tmp.path()
            .join("plugins/cache/openai-curated/linear/local")
            .is_dir()
    );
    assert!(
        !tmp.path()
            .join("plugins/cache/openai-curated/gmail")
            .exists()
    );

    let config = fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).unwrap();
    assert!(config.contains(r#"[plugins."linear@openai-curated"]"#));
    assert!(!config.contains(r#"[plugins."gmail@openai-curated"]"#));
    assert!(config.contains("enabled = false"));
}

#[tokio::test]
async fn sync_plugins_from_remote_uses_first_duplicate_local_plugin_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let curated_root = curated_plugins_repo_path(tmp.path());
    write_curated_plugin_sha(tmp.path());
    fs::create_dir_all(curated_root.join(".agents/plugins")).unwrap();
    fs::write(
        curated_root.join(".agents/plugins/marketplace.json"),
        r#"{
  "name": "openai-curated",
  "plugins": [
    {
      "name": "gmail",
      "source": {
        "source": "local",
        "path": "./plugins/gmail-first"
      }
    },
    {
      "name": "gmail",
      "source": {
        "source": "local",
        "path": "./plugins/gmail-second"
      }
    }
  ]
}"#,
    )
    .unwrap();
    write_plugin(&curated_root, "plugins/gmail-first", "gmail");
    write_plugin(&curated_root, "plugins/gmail-second", "gmail");
    fs::write(curated_root.join("plugins/gmail-first/marker.txt"), "first").unwrap();
    fs::write(
        curated_root.join("plugins/gmail-second/marker.txt"),
        "second",
    )
    .unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/list"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"gmail","marketplace_name":"openai-curated","version":"1.0.0","enabled":true}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = PluginsManager::new(tmp.path().to_path_buf());
    let result =
        sync_plugins_from_remote(&manager, &config, Some(&test_remote_plugin_auth()), false)
            .await
            .unwrap();

    assert_eq!(
        result,
        RemotePluginSyncResult {
            installed_plugin_ids: vec!["gmail@openai-curated".to_string()],
            enabled_plugin_ids: vec!["gmail@openai-curated".to_string()],
            disabled_plugin_ids: Vec::new(),
            uninstalled_plugin_ids: Vec::new(),
        }
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join(format!(
            "plugins/cache/openai-curated/gmail/{TEST_CURATED_PLUGIN_CACHE_VERSION}/marker.txt"
        )))
        .unwrap(),
        "first"
    );
}

#[tokio::test]
async fn featured_plugin_ids_for_config_uses_restriction_product_query_param() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/featured"))
        .and(query_param("platform", "chat"))
        .and(header("authorization", "Bearer Access Token"))
        .and(header("chatgpt-account-id", "account_id"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"["chat-plugin"]"#))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let featured_plugin_ids = featured_plugin_ids_for_config(
        &config,
        Some(&test_remote_plugin_auth()),
        Some(Product::Chatgpt),
    )
    .await
    .unwrap();

    assert_eq!(featured_plugin_ids, vec!["chat-plugin".to_string()]);
}

#[tokio::test]
async fn featured_plugin_ids_for_config_defaults_query_param_to_codex() {
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        &tmp.path().join(CONFIG_TOML_FILE),
        r#"[features]
plugins = true
"#,
    );

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/backend-api/plugins/featured"))
        .and(query_param("platform", "codex"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"["codex-plugin"]"#))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let featured_plugin_ids = featured_plugin_ids_for_config(
        &config, /*auth*/ None, /*restriction_product*/ None,
    )
    .await
    .unwrap();

    assert_eq!(featured_plugin_ids, vec!["codex-plugin".to_string()]);
}
