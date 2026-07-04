//! Helpers for mapping config layer failures to file locations.

use crate::ConfigLayerEntry;
use crate::ConfigLayerStack;
use crate::ConfigLayerStackOrdering;
use crate::ConfigError;
use crate::config_error_from_typed_toml;
use codex_config_types::ConfigLayerSource;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::de::DeserializeOwned;
use std::io;
use std::path::PathBuf;

pub async fn first_layer_config_error<T: DeserializeOwned>(
    layers: &ConfigLayerStack,
    config_toml_file: &str,
) -> Option<ConfigError> {
    // When the merged config fails schema validation, surface the first concrete
    // per-file error to point users at a specific file and range rather than an
    // opaque merged-layer failure.
    first_layer_config_error_for_entries::<T, _>(
        layers.get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ false,
        ),
        config_toml_file,
    )
    .await
}

pub async fn first_layer_config_error_from_entries<T: DeserializeOwned>(
    layers: &[ConfigLayerEntry],
    config_toml_file: &str,
) -> Option<ConfigError> {
    first_layer_config_error_for_entries::<T, _>(layers.iter(), config_toml_file).await
}

async fn first_layer_config_error_for_entries<'a, T: DeserializeOwned, I>(
    layers: I,
    config_toml_file: &str,
) -> Option<ConfigError>
where
    I: IntoIterator<Item = &'a ConfigLayerEntry>,
{
    for layer in layers {
        let Some(path) = config_path_for_layer(layer, config_toml_file) else {
            continue;
        };
        let contents = match tokio::fs::read_to_string(&path).await {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                tracing::debug!("Failed to read config file {}: {err}", path.display());
                continue;
            }
        };

        let Some(parent) = path.parent() else {
            tracing::debug!("Config file {} has no parent directory", path.display());
            continue;
        };
        let _guard = AbsolutePathBufGuard::new(parent);
        if let Some(error) = config_error_from_typed_toml::<T>(&path, &contents) {
            return Some(error);
        }
    }

    None
}

fn config_path_for_layer(layer: &ConfigLayerEntry, config_toml_file: &str) -> Option<PathBuf> {
    match &layer.name {
        ConfigLayerSource::System { file } => Some(file.to_path_buf()),
        ConfigLayerSource::User { file, .. } => Some(file.to_path_buf()),
        ConfigLayerSource::Project { dot_codex_folder } => {
            Some(dot_codex_folder.as_path().join(config_toml_file))
        }
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { file } => Some(file.to_path_buf()),
        ConfigLayerSource::Mdm { .. }
        | ConfigLayerSource::SessionFlags
        | ConfigLayerSource::LegacyManagedConfigTomlFromMdm => None,
    }
}
