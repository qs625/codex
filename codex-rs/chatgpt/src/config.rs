use std::path::PathBuf;

use codex_auth_types::AuthManagerConfig;
use codex_config_types::AuthCredentialsStoreMode;
use codex_login::CodexAuth;
use plugin_service_api::PluginsConfigInput;

/// Resolved ChatGPT backend settings required by this crate.
#[derive(Clone, Debug)]
pub struct ChatGptConfig {
    pub codex_home: PathBuf,
    pub cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
    pub forced_chatgpt_workspace_id: Option<Vec<String>>,
    pub chatgpt_base_url: String,
    pub apps_feature_enabled: bool,
    pub plugins_config_input: PluginsConfigInput,
}

impl ChatGptConfig {
    pub fn apps_enabled_for_auth(&self, auth: Option<&CodexAuth>) -> bool {
        self.apps_feature_enabled && auth.is_some_and(CodexAuth::uses_codex_backend)
    }
}

impl AuthManagerConfig for ChatGptConfig {
    fn codex_home(&self) -> PathBuf {
        self.codex_home.clone()
    }

    fn cli_auth_credentials_store_mode(&self) -> AuthCredentialsStoreMode {
        self.cli_auth_credentials_store_mode
    }

    fn forced_chatgpt_workspace_id(&self) -> Option<Vec<String>> {
        self.forced_chatgpt_workspace_id.clone()
    }

    fn chatgpt_base_url(&self) -> String {
        self.chatgpt_base_url.clone()
    }
}
