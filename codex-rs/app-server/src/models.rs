use std::sync::Arc;

use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelServiceTier;
use codex_app_server_protocol::ModelUpgradeInfo;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;

pub async fn supported_models(
    thread_manager: Arc<ThreadManager>,
    include_hidden: bool,
) -> Vec<Model> {
    thread_manager
        .list_models(RefreshStrategy::OnlineIfUncached)
        .await
        .into_iter()
        .filter(|preset| include_hidden || preset.show_in_picker)
        .map(model_from_preset)
        .collect()
}

pub fn add_configured_model(models: &mut Vec<Model>, config: &Config) {
    let Some(model) = config.model.as_deref() else {
        return;
    };

    add_active_configured_model(models, config, model);
    add_extra_configured_provider_models(models, config, model);
}

fn add_active_configured_model(models: &mut Vec<Model>, config: &Config, model: &str) {
    let provider_name = configured_provider_name(&config.model_provider.name);
    if let Some(existing) = models
        .iter_mut()
        .find(|entry| entry.model == model || entry.id == model)
    {
        if let Some(provider_name) = provider_name {
            existing.description = format!("{} · 当前配置: {provider_name}", existing.description);
        }
        existing.model_provider = Some(config.model_provider_id.clone());
        return;
    }

    models.push(configured_model(
        &config.model_provider_id,
        model,
        config.model_reasoning_effort,
        configured_model_description(provider_name.as_deref()),
    ));
}

fn add_extra_configured_provider_models(models: &mut Vec<Model>, config: &Config, model: &str) {
    let mut provider_ids = config
        .model_providers
        .keys()
        .filter(|provider_id| **provider_id != config.model_provider_id)
        .filter(|provider_id| !is_builtin_provider_id(provider_id))
        .cloned()
        .collect::<Vec<_>>();
    provider_ids.sort();

    for provider_id in provider_ids {
        let Some(provider) = config.model_providers.get(&provider_id) else {
            continue;
        };
        let id = configured_model_id(&provider_id, model);
        if models.iter().any(|entry| entry.id == id) {
            continue;
        }
        let provider_name = configured_provider_name(&provider.name);
        models.push(configured_model(
            &provider_id,
            model,
            config.model_reasoning_effort,
            configured_provider_description(provider_name.as_deref()),
        ));
    }
}

fn configured_model(
    provider_id: &str,
    model: &str,
    model_reasoning_effort: Option<ReasoningEffort>,
    description: String,
) -> Model {
    let default_reasoning_effort = model_reasoning_effort.unwrap_or(ReasoningEffort::Medium);
    Model {
        id: configured_model_id(provider_id, model),
        model: model.to_string(),
        model_provider: Some(provider_id.to_string()),
        upgrade: None,
        upgrade_info: None,
        availability_nux: None,
        display_name: model.to_string(),
        description,
        hidden: false,
        supported_reasoning_efforts: vec![ReasoningEffortOption {
            reasoning_effort: default_reasoning_effort,
            description: "当前配置的默认 reasoning".to_string(),
        }],
        default_reasoning_effort,
        input_modalities: codex_protocol::openai_models::default_input_modalities(),
        supports_personality: false,
        additional_speed_tiers: Vec::new(),
        service_tiers: Vec::new(),
        is_default: false,
    }
}

fn configured_provider_name(provider_name: &str) -> Option<String> {
    if provider_name.is_empty() {
        None
    } else {
        Some(provider_name.to_string())
    }
}

fn is_builtin_provider_id(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "openai" | "amazon-bedrock" | "lmstudio" | "ollama"
    )
}

fn configured_model_id(provider_id: &str, model: &str) -> String {
    format!("configured:{provider_id}:{model}")
}

fn configured_model_description(provider_name: Option<&str>) -> String {
    match provider_name {
        Some(provider_name) => format!("当前配置中的模型 · {provider_name}"),
        None => "当前配置中的模型".to_string(),
    }
}

fn configured_provider_description(provider_name: Option<&str>) -> String {
    match provider_name {
        Some(provider_name) => format!("已配置 provider · {provider_name}"),
        None => "已配置 provider".to_string(),
    }
}

fn model_from_preset(preset: ModelPreset) -> Model {
    Model {
        id: preset.id.to_string(),
        model: preset.model.to_string(),
        model_provider: None,
        upgrade: preset.upgrade.as_ref().map(|upgrade| upgrade.id.clone()),
        upgrade_info: preset.upgrade.as_ref().map(|upgrade| ModelUpgradeInfo {
            model: upgrade.id.clone(),
            upgrade_copy: upgrade.upgrade_copy.clone(),
            model_link: upgrade.model_link.clone(),
            migration_markdown: upgrade.migration_markdown.clone(),
        }),
        availability_nux: preset.availability_nux.map(Into::into),
        display_name: preset.display_name.to_string(),
        description: preset.description.to_string(),
        hidden: !preset.show_in_picker,
        supported_reasoning_efforts: reasoning_efforts_from_preset(
            preset.supported_reasoning_efforts,
        ),
        default_reasoning_effort: preset.default_reasoning_effort,
        input_modalities: preset.input_modalities,
        supports_personality: preset.supports_personality,
        additional_speed_tiers: preset.additional_speed_tiers,
        service_tiers: preset
            .service_tiers
            .into_iter()
            .map(|service_tier| ModelServiceTier {
                id: service_tier.id,
                name: service_tier.name,
                description: service_tier.description,
            })
            .collect(),
        is_default: preset.is_default,
    }
}

fn reasoning_efforts_from_preset(
    efforts: Vec<ReasoningEffortPreset>,
) -> Vec<ReasoningEffortOption> {
    efforts
        .iter()
        .map(|preset| ReasoningEffortOption {
            reasoning_effort: preset.effort,
            description: preset.description.to_string(),
        })
        .collect()
}
