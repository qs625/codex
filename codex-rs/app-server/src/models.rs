use codex_app_server_protocol::Model;
use codex_app_server_protocol::ModelServiceTier;
use codex_app_server_protocol::ModelUpgradeInfo;
use codex_app_server_protocol::ReasoningEffortOption;
use codex_model_provider_info::ModelProviderInfo;
use codex_models_manager::model_info;
use codex_models_manager_api::RefreshStrategy;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use thread_service::ThreadService;
use thread_service::config::Config;
use futures::future::BoxFuture;

const OPENAI_PROVIDER_ID: &str = "openai";
const AMAZON_BEDROCK_PROVIDER_ID: &str = "amazon-bedrock";

pub trait ModelCatalogRuntime: Send + Sync {
    fn list_models_for_provider<'a>(
        &'a self,
        config: &'a Config,
        provider_info: ModelProviderInfo,
        model_catalog: Option<ModelsResponse>,
        refresh_strategy: RefreshStrategy,
    ) -> BoxFuture<'a, Vec<ModelPreset>>;
}

impl ModelCatalogRuntime for ThreadService {
    fn list_models_for_provider<'a>(
        &'a self,
        config: &'a Config,
        provider_info: ModelProviderInfo,
        model_catalog: Option<ModelsResponse>,
        refresh_strategy: RefreshStrategy,
    ) -> BoxFuture<'a, Vec<ModelPreset>> {
        Box::pin(ThreadService::list_models_for_provider(
            self,
            config,
            provider_info,
            model_catalog,
            refresh_strategy,
        ))
    }
}

pub async fn supported_models(
    model_catalog_runtime: &(impl ModelCatalogRuntime + ?Sized),
    config: &Config,
    include_hidden: bool,
) -> Vec<Model> {
    let mut model_provider_ids = config.model_providers.keys().collect::<Vec<_>>();
    model_provider_ids
        .sort_by(|left, right| provider_sort_key(left).cmp(&provider_sort_key(right)));

    let mut models = Vec::new();
    for model_provider_id in model_provider_ids {
        let Some(provider_info) = config.model_providers.get(model_provider_id) else {
            continue;
        };
        if model_provider_id == AMAZON_BEDROCK_PROVIDER_ID
            && !should_list_bedrock_catalog(
                provider_info
                    .aws
                    .as_ref()
                    .and_then(|aws| aws.profile.as_deref()),
                provider_info
                    .aws
                    .as_ref()
                    .and_then(|aws| aws.region.as_deref()),
            )
        {
            continue;
        }
        let model_catalog = if model_provider_id == &config.model_provider_id {
            config.model_catalog.clone()
        } else {
            None
        };
        models.extend(
            model_catalog_runtime
                .list_models_for_provider(
                    config,
                    provider_info.clone(),
                    model_catalog,
                    RefreshStrategy::OnlineIfUncached,
                )
                .await
                .into_iter()
                .filter(|preset| include_hidden || preset.show_in_picker)
                .map(|preset| model_from_preset(preset, model_provider_id)),
        );
    }
    models
}

pub fn add_configured_model(models: &mut Vec<Model>, config: &Config) {
    if let Some(model) = config.model.as_deref() {
        add_active_configured_model(models, config, model);
    }
    add_configured_model_options(models, config);
}

fn add_active_configured_model(models: &mut Vec<Model>, config: &Config, model: &str) {
    let provider_name = configured_provider_name(&config.model_provider.name);
    if let Some(model_option) = config.model_options.iter().find(|model_option| {
        model_option.provider == config.model_provider_id && model_option.model == model
    }) {
        let model = configured_model_from_option(
            ConfiguredModelOptionView {
                provider_id: &model_option.provider,
                model: &model_option.model,
                max_context_window: model_option.max_context_window,
                context_window: model_option.context_window,
                auto_compact_token_limit: model_option.auto_compact_token_limit,
            },
            config.model_reasoning_effort,
            configured_model_description(provider_name.as_deref()),
        );
        if let Some(existing) = models
            .iter_mut()
            .find(|entry| same_provider_model(entry, &model))
        {
            *existing = model;
        } else {
            models.push(model);
        }
        return;
    }

    if let Some(existing) = models.iter_mut().find(|entry| {
        (entry.model == model || entry.id == model)
            && entry.model_provider.as_deref() == Some(config.model_provider_id.as_str())
    }) {
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

fn add_configured_model_options(models: &mut Vec<Model>, config: &Config) {
    for model_option in &config.model_options {
        if !config.model_providers.contains_key(&model_option.provider) {
            continue;
        }
        let provider = config.model_providers.get(&model_option.provider);
        let provider_name = provider.and_then(|provider| configured_provider_name(&provider.name));
        let model = configured_model_from_option(
            ConfiguredModelOptionView {
                provider_id: &model_option.provider,
                model: &model_option.model,
                max_context_window: model_option.max_context_window,
                context_window: model_option.context_window,
                auto_compact_token_limit: model_option.auto_compact_token_limit,
            },
            config.model_reasoning_effort,
            configured_model_option_description(provider_name.as_deref()),
        );
        if let Some(existing) = models
            .iter_mut()
            .find(|entry| same_provider_model(entry, &model))
        {
            *existing = model;
        } else {
            models.push(model);
        }
    }
}

fn configured_model_from_option(
    model_option: ConfiguredModelOptionView<'_>,
    model_reasoning_effort: Option<ReasoningEffort>,
    description: String,
) -> Model {
    let mut model_info = model_info::model_info_from_slug(model_option.model);
    model_info.visibility = codex_protocol::openai_models::ModelVisibility::List;
    if let Some(max_context_window) = model_option.max_context_window {
        model_info.max_context_window = Some(max_context_window);
    }
    if let Some(context_window) = model_option.context_window {
        model_info.context_window = Some(
            model_info
                .max_context_window
                .map_or(context_window, |max_context_window| {
                    context_window.min(max_context_window)
                }),
        );
    }
    if let Some(auto_compact_token_limit) = model_option.auto_compact_token_limit {
        model_info.auto_compact_token_limit = Some(auto_compact_token_limit);
    }
    if let Some(reasoning_effort) = model_reasoning_effort {
        model_info.default_reasoning_level = Some(reasoning_effort);
    }
    configured_model_from_preset(model_option.provider_id, model_info.into(), description)
}

struct ConfiguredModelOptionView<'a> {
    provider_id: &'a str,
    model: &'a str,
    max_context_window: Option<i64>,
    context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
}

fn configured_model_from_preset(
    provider_id: &str,
    preset: ModelPreset,
    description: String,
) -> Model {
    Model {
        id: configured_model_id(provider_id, &preset.model),
        description,
        model_provider: Some(provider_id.to_string()),
        ..model_from_preset(preset, provider_id)
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
        context_window: None,
        max_context_window: None,
        auto_compact_token_limit: None,
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

fn configured_model_id(provider_id: &str, model: &str) -> String {
    format!("configured:{provider_id}:{model}")
}

fn catalog_model_id(provider_id: &str, id: &str) -> String {
    if provider_id == OPENAI_PROVIDER_ID {
        id.to_string()
    } else {
        format!("provider:{provider_id}:{id}")
    }
}

fn same_provider_model(left: &Model, right: &Model) -> bool {
    left.model == right.model && left.model_provider == right.model_provider
}

fn provider_sort_key(provider_id: &str) -> (u8, &str) {
    match provider_id {
        OPENAI_PROVIDER_ID => (0, provider_id),
        AMAZON_BEDROCK_PROVIDER_ID => (1, provider_id),
        _ => (2, provider_id),
    }
}

fn should_list_bedrock_catalog(profile: Option<&str>, region: Option<&str>) -> bool {
    let has_bearer_token = std::env::var("AWS_BEARER_TOKEN_BEDROCK")
        .ok()
        .is_some_and(|value| !value.trim().is_empty());
    let has_configured_aws_profile = profile.is_some_and(|profile| !profile.trim().is_empty());
    let has_configured_bearer_region =
        region.is_some_and(|region| !region.trim().is_empty()) && has_bearer_token;

    has_configured_aws_profile || has_configured_bearer_region
}

fn configured_model_description(provider_name: Option<&str>) -> String {
    match provider_name {
        Some(provider_name) => format!("当前配置中的模型 · {provider_name}"),
        None => "当前配置中的模型".to_string(),
    }
}

fn configured_model_option_description(provider_name: Option<&str>) -> String {
    match provider_name {
        Some(provider_name) => format!("配置文件中的模型 · {provider_name}"),
        None => "配置文件中的模型".to_string(),
    }
}

fn model_from_preset(preset: ModelPreset, model_provider_id: &str) -> Model {
    Model {
        id: catalog_model_id(model_provider_id, &preset.id),
        model: preset.model.to_string(),
        model_provider: Some(model_provider_id.to_string()),
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
        context_window: preset.context_window,
        max_context_window: preset.max_context_window,
        auto_compact_token_limit: preset.auto_compact_token_limit,
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
