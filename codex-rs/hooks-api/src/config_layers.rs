use codex_config_types::ConfigLayerSource;
use codex_config_types::ManagedHooksRequirementsToml;
use codex_config_types::RequirementSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use toml::Value as TomlValue;

/// Hook-specific view of one loaded config layer.
///
/// This intentionally carries only the fields hook discovery needs so hook
/// runtime implementations do not depend on the full config loader/evaluator
/// crate.
#[derive(Debug, Clone, PartialEq)]
pub struct HookConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub config: TomlValue,
    hooks_config_folder: Option<AbsolutePathBuf>,
    disabled: bool,
}

impl HookConfigLayerEntry {
    pub fn new(name: ConfigLayerSource, config: TomlValue) -> Self {
        let hooks_config_folder = default_hooks_config_folder(&name);
        Self {
            name,
            config,
            hooks_config_folder,
            disabled: false,
        }
    }

    pub fn new_with_hooks_config_folder(
        name: ConfigLayerSource,
        config: TomlValue,
        hooks_config_folder: Option<AbsolutePathBuf>,
        disabled: bool,
    ) -> Self {
        Self {
            name,
            config,
            hooks_config_folder,
            disabled,
        }
    }

    pub fn hooks_config_folder(&self) -> Option<AbsolutePathBuf> {
        self.hooks_config_folder.clone()
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookConfigLayerStackOrdering {
    LowestPrecedenceFirst,
    HighestPrecedenceFirst,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookManagedHooksRequirement {
    pub value: ManagedHooksRequirementsToml,
    pub source: Option<RequirementSource>,
}

/// Hook-specific view of the config stack and hook-related requirements.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookConfigLayerStack {
    layers: Vec<HookConfigLayerEntry>,
    allow_managed_hooks_only: bool,
    managed_hooks: Option<HookManagedHooksRequirement>,
}

impl HookConfigLayerStack {
    pub fn new(
        layers: Vec<HookConfigLayerEntry>,
        allow_managed_hooks_only: bool,
        managed_hooks: Option<HookManagedHooksRequirement>,
    ) -> Self {
        Self {
            layers,
            allow_managed_hooks_only,
            managed_hooks,
        }
    }

    pub fn allow_managed_hooks_only(&self) -> bool {
        self.allow_managed_hooks_only
    }

    pub fn managed_hooks(&self) -> Option<&HookManagedHooksRequirement> {
        self.managed_hooks.as_ref()
    }

    pub fn get_layers(
        &self,
        ordering: HookConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&HookConfigLayerEntry> {
        let mut layers = self
            .layers
            .iter()
            .filter(|layer| include_disabled || !layer.is_disabled())
            .collect::<Vec<_>>();
        if ordering == HookConfigLayerStackOrdering::HighestPrecedenceFirst {
            layers.reverse();
        }
        layers
    }
}

fn default_hooks_config_folder(source: &ConfigLayerSource) -> Option<AbsolutePathBuf> {
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
