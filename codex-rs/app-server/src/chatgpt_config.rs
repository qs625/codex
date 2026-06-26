use codex_chatgpt::ChatGptConfig;
use codex_thread_runtime::config::Config;
use codex_features::Feature;

pub(crate) fn chatgpt_config_from_core(config: &Config) -> ChatGptConfig {
    ChatGptConfig {
        codex_home: config.codex_home.to_path_buf(),
        cli_auth_credentials_store_mode: config.cli_auth_credentials_store_mode,
        forced_chatgpt_workspace_id: config.forced_chatgpt_workspace_id.clone(),
        chatgpt_base_url: config.chatgpt_base_url.clone(),
        apps_feature_enabled: config.features.enabled(Feature::Apps),
        plugins_config_input: config.plugins_config_input(),
    }
}
