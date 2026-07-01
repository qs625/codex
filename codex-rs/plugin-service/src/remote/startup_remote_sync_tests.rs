use std::sync::Arc;
use std::time::Duration;

use codex_config_edit::CONFIG_TOML_FILE;
use crate::PluginsManager;
use crate::startup_sync::curated_plugins_repo_path;
use codex_login::CodexAuth;
use pretty_assertions::assert_eq;
use tempfile::tempdir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

use super::*;
use crate::RemotePluginAuth;
use crate::RemotePluginAuthFuture;
use crate::RemotePluginAuthProvider;
use crate::test_support::TEST_CURATED_PLUGIN_CACHE_VERSION;
use crate::test_support::load_plugins_config;
use crate::test_support::write_curated_plugin_sha;
use crate::test_support::write_file;
use crate::test_support::write_openai_curated_marketplace;

struct TestRemotePluginAuthProvider {
    auth: RemotePluginAuth,
}

impl RemotePluginAuthProvider for TestRemotePluginAuthProvider {
    fn remote_plugin_auth(&self) -> RemotePluginAuthFuture {
        let auth = self.auth.clone();
        Box::pin(async move { Some(auth) })
    }
}

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
async fn startup_remote_plugin_sync_writes_marker_and_reconciles_state() {
    let tmp = tempdir().expect("tempdir");
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
        .and(header("authorization", "Bearer Access Token"))
        .and(header("chatgpt-account-id", "account_id"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"[
  {"id":"1","name":"linear","marketplace_name":"openai-curated","version":"1.0.0","enabled":true}
]"#,
        ))
        .mount(&server)
        .await;

    let mut config = load_plugins_config(tmp.path());
    config.chatgpt_base_url = format!("{}/backend-api/", server.uri());
    let manager = Arc::new(PluginsManager::new(tmp.path().to_path_buf()));

    start_startup_remote_plugin_sync_once(
        Arc::clone(&manager),
        tmp.path().to_path_buf(),
        config,
        Arc::new(TestRemotePluginAuthProvider {
            auth: test_remote_plugin_auth(),
        }),
    );

    let marker_path = tmp.path().join(STARTUP_REMOTE_PLUGIN_SYNC_MARKER_FILE);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if marker_path.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("marker should be written");

    assert!(
        tmp.path()
            .join(format!(
                "plugins/cache/openai-curated/linear/{TEST_CURATED_PLUGIN_CACHE_VERSION}"
            ))
            .is_dir()
    );
    let config =
        std::fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).expect("config should exist");
    assert!(config.contains(r#"[plugins."linear@openai-curated"]"#));
    assert!(config.contains("enabled = true"));

    let marker_contents = std::fs::read_to_string(marker_path).expect("marker should be readable");
    assert_eq!(marker_contents, "ok\n");
}
