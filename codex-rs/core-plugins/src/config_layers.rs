use codex_config_types::ConfigLayerSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use toml::Value as TomlValue;

/// Plugin-specific view of one loaded config layer.
///
/// This carries only the data plugin loading, marketplace discovery, and plugin
/// skill projection need so plugin runtime code does not have to consume the
/// full config loader stack API.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub config: TomlValue,
    config_folder: Option<AbsolutePathBuf>,
    disabled: bool,
}

impl PluginConfigLayerEntry {
    pub fn new(name: ConfigLayerSource, config: TomlValue) -> Self {
        let config_folder = default_config_folder(&name);
        Self {
            name,
            config,
            config_folder,
            disabled: false,
        }
    }

    pub fn new_with_config_folder(
        name: ConfigLayerSource,
        config: TomlValue,
        config_folder: Option<AbsolutePathBuf>,
        disabled: bool,
    ) -> Self {
        Self {
            name,
            config,
            config_folder,
            disabled,
        }
    }

    pub fn config_folder(&self) -> Option<AbsolutePathBuf> {
        self.config_folder.clone()
    }

    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginConfigLayerStackOrdering {
    LowestPrecedenceFirst,
    HighestPrecedenceFirst,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginConfigLayerStack {
    layers: Vec<PluginConfigLayerEntry>,
    active_user_layer_index: Option<usize>,
}

impl PluginConfigLayerStack {
    pub fn new(layers: Vec<PluginConfigLayerEntry>) -> Self {
        let active_user_layer_index = layers.iter().enumerate().rev().find_map(|(index, layer)| {
            if matches!(layer.name, ConfigLayerSource::User { .. }) {
                Some(index)
            } else {
                None
            }
        });
        Self {
            layers,
            active_user_layer_index,
        }
    }

    pub fn get_active_user_layer(&self) -> Option<&PluginConfigLayerEntry> {
        self.active_user_layer_index
            .and_then(|index| self.layers.get(index))
    }

    pub(crate) fn get_layers(
        &self,
        ordering: PluginConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&PluginConfigLayerEntry> {
        let mut layers = self
            .layers
            .iter()
            .filter(|layer| include_disabled || !layer.is_disabled())
            .collect::<Vec<_>>();
        if ordering == PluginConfigLayerStackOrdering::HighestPrecedenceFirst {
            layers.reverse();
        }
        layers
    }

    pub fn effective_user_config(&self) -> Option<TomlValue> {
        let user_layers = self
            .get_layers(
                PluginConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ false,
            )
            .into_iter()
            .filter(|layer| matches!(layer.name, ConfigLayerSource::User { .. }))
            .collect::<Vec<_>>();
        if user_layers.is_empty() {
            return None;
        }

        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in user_layers {
            merge_toml_values(&mut merged, &layer.config);
        }
        Some(merged)
    }

    pub fn effective_config(&self) -> TomlValue {
        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in self.get_layers(
            PluginConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ false,
        ) {
            merge_toml_values(&mut merged, &layer.config);
        }
        merged
    }
}

fn default_config_folder(source: &ConfigLayerSource) -> Option<AbsolutePathBuf> {
    match source {
        ConfigLayerSource::Mdm { .. } => None,
        ConfigLayerSource::System { file } => file.parent(),
        ConfigLayerSource::User { file, .. } => file.parent(),
        ConfigLayerSource::Project { dot_codex_folder } => Some(dot_codex_folder.clone()),
        ConfigLayerSource::SessionFlags => None,
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. } => None,
        ConfigLayerSource::LegacyManagedConfigTomlFromMdm => None,
    }
}

fn merge_toml_values(base: &mut TomlValue, overlay: &TomlValue) {
    if let TomlValue::Table(overlay_table) = overlay
        && let TomlValue::Table(base_table) = base
    {
        for (key, value) in overlay_table {
            if let Some(existing) = base_table.get_mut(key) {
                merge_toml_values(existing, value);
            } else {
                base_table.insert(key.clone(), value.clone());
            }
        }
    } else {
        *base = overlay.clone();
    }
}

#[cfg(test)]
impl From<codex_config::ConfigLayerStack> for PluginConfigLayerStack {
    fn from(stack: codex_config::ConfigLayerStack) -> Self {
        let layers = stack
            .get_layers(
                codex_config::ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .map(|layer| {
                PluginConfigLayerEntry::new_with_config_folder(
                    layer.name.clone(),
                    layer.config.clone(),
                    layer.config_folder(),
                    layer.is_disabled(),
                )
            })
            .collect();
        Self::new(layers)
    }
}
