use codex_config_loader::SessionThreadConfig;
use codex_config_loader::ThreadConfigContext;
use codex_config_loader::ThreadConfigLoadError;
use codex_config_loader::ThreadConfigLoadErrorCode;
use codex_config_loader::ThreadConfigLoader;
use codex_config_loader::ThreadConfigSource;
use codex_config_state::ConfigLayerEntry;
use codex_config_types::ConfigLayerSource;
use toml::Value as TomlValue;

pub(crate) async fn load_thread_config_layers(
    loader: &dyn ThreadConfigLoader,
    context: ThreadConfigContext,
) -> Result<Vec<ConfigLayerEntry>, ThreadConfigLoadError> {
    let sources = loader.load(context).await?;
    sources
        .into_iter()
        .map(thread_config_source_to_layer)
        .collect::<Result<Vec<_>, _>>()
        .map(|layers| layers.into_iter().flatten().collect())
}

fn thread_config_source_to_layer(
    source: ThreadConfigSource,
) -> Result<Option<ConfigLayerEntry>, ThreadConfigLoadError> {
    match source {
        ThreadConfigSource::Session(config) => {
            let config = session_thread_config_to_toml(config)?;
            if is_empty_table(&config) {
                Ok(None)
            } else {
                Ok(Some(ConfigLayerEntry::new(
                    ConfigLayerSource::SessionFlags,
                    config,
                )))
            }
        }
        // UserThreadConfig has no TOML-backed fields yet. When it grows one,
        // fold it into the existing user layer instead of adding another
        // ConfigLayerSource variant.
        ThreadConfigSource::User(_config) => Ok(None),
    }
}

fn is_empty_table(config: &TomlValue) -> bool {
    config.as_table().is_some_and(toml::map::Map::is_empty)
}

fn session_thread_config_to_toml(
    config: SessionThreadConfig,
) -> Result<TomlValue, ThreadConfigLoadError> {
    let mut table = toml::map::Map::new();

    if let Some(model_provider) = config.model_provider {
        table.insert(
            "model_provider".to_string(),
            TomlValue::String(model_provider),
        );
    }

    if !config.model_providers.is_empty() {
        let model_providers = TomlValue::try_from(config.model_providers).map_err(|err| {
            ThreadConfigLoadError::new(
                ThreadConfigLoadErrorCode::Parse,
                /*status_code*/ None,
                format!("failed to convert session model providers to config TOML: {err}"),
            )
        })?;
        table.insert("model_providers".to_string(), model_providers);
    }

    if !config.features.is_empty() {
        let features = config
            .features
            .into_iter()
            .map(|(feature, enabled)| (feature, TomlValue::Boolean(enabled)))
            .collect();
        table.insert("features".to_string(), TomlValue::Table(features));
    }

    Ok(TomlValue::Table(table))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;

    use codex_config_loader::StaticThreadConfigLoader;
    use codex_config_loader::ThreadConfigSource;
    use codex_config_loader::UserThreadConfig;
    use codex_model_provider_info::ModelProviderInfo;
    use codex_model_provider_info::WireApi;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;

    use super::*;

    #[tokio::test]
    async fn load_thread_config_layers_translates_sources() {
        let loader = StaticThreadConfigLoader::new(vec![
            ThreadConfigSource::User(UserThreadConfig::default()),
            ThreadConfigSource::Session(SessionThreadConfig {
                model_provider: Some("local".to_string()),
                model_providers: HashMap::from([("local".to_string(), test_provider("local"))]),
                features: BTreeMap::from([("plugins".to_string(), false)]),
            }),
        ]);
        let layers = load_thread_config_layers(
            &loader,
            ThreadConfigContext {
                cwd: Some(
                    AbsolutePathBuf::from_absolute_path_checked(
                        std::env::temp_dir().join("project"),
                    )
                    .expect("absolute cwd"),
                ),
                ..Default::default()
            },
        )
        .await
        .expect("thread config layers load");

        assert_eq!(
            layers,
            vec![ConfigLayerEntry::new(
                ConfigLayerSource::SessionFlags,
                toml::toml! {
                    model_provider = "local"

                    [model_providers.local]
                    name = "local"
                    base_url = "http://127.0.0.1:8061/api/codex"
                    wire_api = "responses"
                    requires_openai_auth = false
                    supports_websockets = true

                    [features]
                    plugins = false
                }
                .into()
            )]
        );
    }

    fn test_provider(name: &str) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            base_url: Some("http://127.0.0.1:8061/api/codex".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            auth: None,
            aws: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: true,
        }
    }
}
