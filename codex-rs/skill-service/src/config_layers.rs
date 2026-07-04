use codex_config_types::ConfigLayerSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use toml::Value as TomlValue;

#[derive(Debug, Clone, PartialEq)]
pub struct SkillConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub config: TomlValue,
    config_folder: Option<AbsolutePathBuf>,
    disabled: bool,
}

impl SkillConfigLayerEntry {
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

    fn is_disabled(&self) -> bool {
        self.disabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkillConfigLayerStackOrdering {
    LowestPrecedenceFirst,
    HighestPrecedenceFirst,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkillConfigLayerStack {
    layers: Vec<SkillConfigLayerEntry>,
}

impl SkillConfigLayerStack {
    pub fn new(layers: Vec<SkillConfigLayerEntry>) -> Self {
        Self { layers }
    }

    pub(crate) fn get_layers(
        &self,
        ordering: SkillConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&SkillConfigLayerEntry> {
        let mut layers = self
            .layers
            .iter()
            .filter(|layer| include_disabled || !layer.is_disabled())
            .collect::<Vec<_>>();
        if ordering == SkillConfigLayerStackOrdering::HighestPrecedenceFirst {
            layers.reverse();
        }
        layers
    }

    pub(crate) fn effective_config_without_project_layers(&self) -> TomlValue {
        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in self.get_layers(
            SkillConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ false,
        ) {
            if matches!(layer.name, ConfigLayerSource::Project { .. }) {
                continue;
            }
            merge_toml_values(&mut merged, &layer.config);
        }
        merged
    }

    pub(crate) fn effective_config(&self) -> TomlValue {
        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in self.get_layers(
            SkillConfigLayerStackOrdering::LowestPrecedenceFirst,
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
impl From<config_service::ConfigLayerStack> for SkillConfigLayerStack {
    fn from(stack: config_service::ConfigLayerStack) -> Self {
        let layers = stack
            .get_layers(
                config_service::ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ true,
            )
            .into_iter()
            .map(|layer| {
                SkillConfigLayerEntry::new_with_config_folder(
                    layer.name.clone(),
                    layer.config.clone(),
                    layer.config_folder(),
                    layer.is_disabled(),
                )
            })
            .collect();
        SkillConfigLayerStack::new(layers)
    }
}
