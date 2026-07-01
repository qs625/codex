use codex_config_state::ConfigLayerEntry;
use codex_config_state::ConfigLayerStack;
use codex_config_state::ConfigLayerStackOrdering;
use plugin_service_api::PluginConfigLayerEntry;
use plugin_service_api::PluginConfigLayerStack;
use codex_core_skills_api::SkillConfigLayerEntry;
use codex_core_skills_api::SkillConfigLayerStack;
use codex_hooks_api::HookConfigLayerEntry;
use codex_hooks_api::HookConfigLayerStack;
use codex_hooks_api::HookManagedHooksRequirement;

use crate::Config;

pub fn child_uses_parent_exec_policy(parent_config: &Config, child_config: &Config) -> bool {
    fn exec_policy_config_folders(config: &Config) -> Vec<crate::AbsolutePathBuf> {
        config
            .config_layer_stack
            .get_layers(
                ConfigLayerStackOrdering::LowestPrecedenceFirst,
                /*include_disabled*/ false,
            )
            .into_iter()
            .filter_map(ConfigLayerEntry::config_folder)
            .collect()
    }

    exec_policy_config_folders(parent_config) == exec_policy_config_folders(child_config)
        && parent_config
            .config_layer_stack
            .ignore_user_and_project_exec_policy_rules()
            == child_config
                .config_layer_stack
                .ignore_user_and_project_exec_policy_rules()
        && parent_config.config_layer_stack.requirements().exec_policy
            == child_config.config_layer_stack.requirements().exec_policy
}

pub fn hook_config_layer_stack_from_config_layer_stack(
    config_layer_stack: &ConfigLayerStack,
) -> HookConfigLayerStack {
    let layers = config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .map(|layer| {
            HookConfigLayerEntry::new_with_hooks_config_folder(
                layer.name.clone(),
                layer.config.clone(),
                layer.hooks_config_folder(),
                layer.is_disabled(),
            )
        })
        .collect();
    let requirements = config_layer_stack.requirements();
    let managed_hooks =
        requirements
            .managed_hooks
            .as_ref()
            .map(|managed_hooks| HookManagedHooksRequirement {
                value: managed_hooks.get().clone(),
                source: managed_hooks.source.clone(),
            });
    HookConfigLayerStack::new(
        layers,
        requirements
            .allow_managed_hooks_only
            .as_ref()
            .is_some_and(|requirement| requirement.value),
        managed_hooks,
    )
}

pub fn skill_config_layer_stack_from_config_layer_stack(
    config_layer_stack: &ConfigLayerStack,
) -> SkillConfigLayerStack {
    let layers = config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
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

pub fn plugin_config_layer_stack_from_config_layer_stack(
    config_layer_stack: &ConfigLayerStack,
) -> PluginConfigLayerStack {
    let layers = config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
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
    PluginConfigLayerStack::new(layers)
}
