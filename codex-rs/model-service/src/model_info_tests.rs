use super::*;
use crate::bundled_models_response;
use model_service_api::ModelsManagerConfig;
use pretty_assertions::assert_eq;

const LEGACY_ARTIFACT_TOKEN: &str = concat!("MORPHEUS", "_ARTIFACT");

#[test]
fn reasoning_summaries_override_true_enables_support() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(true),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.supports_reasoning_summaries = true;

    assert_eq!(updated, expected);
}

#[test]
fn reasoning_summaries_override_false_does_not_disable_support() {
    let mut model = model_info_from_slug("unknown-model");
    model.supports_reasoning_summaries = true;
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn reasoning_summaries_override_false_is_noop_when_model_is_false() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn model_context_window_override_clamps_to_max_context_window() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(500_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(400_000);

    assert_eq!(updated, expected);
}

#[test]
fn model_context_window_uses_model_value_without_override() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig::default();

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn base_instructions_include_artifact_publishing_guidance() {
    let model = model_info_from_slug("unknown-model");

    assert!(BASE_INSTRUCTIONS.contains("# Artifact Publishing"));
    assert!(BASE_INSTRUCTIONS.contains("publish_artifact"));
    assert!(!BASE_INSTRUCTIONS.contains(LEGACY_ARTIFACT_TOKEN));
    assert!(model.base_instructions.contains("publish_artifact"));
}

#[test]
fn bundled_catalog_models_include_artifact_publishing_guidance_after_resolution() {
    let models_response = bundled_models_response().expect("bundled models should parse");
    let candidate = models_response
        .models
        .iter()
        .find(|model| model.slug == "gpt-5.4")
        .cloned()
        .expect("catalog slug should exist");

    let model = construct_model_info_from_candidates(
        "gpt-5.4",
        &[candidate],
        &ModelsManagerConfig::default(),
    );

    assert!(model.base_instructions.contains("# Artifact Publishing"));
    assert!(model.base_instructions.contains("publish_artifact"));
    assert!(
        model
            .get_model_instructions(None)
            .contains("Use the `publish_artifact` tool")
    );
}

#[test]
fn config_base_instructions_override_keeps_artifact_publishing_guidance() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        base_instructions: Some("custom instructions".to_string()),
        ..Default::default()
    };

    let updated = with_config_overrides(model, &config);

    assert!(updated.base_instructions.starts_with("custom instructions"));
    assert!(updated.base_instructions.contains("publish_artifact"));
    assert!(
        !updated
            .base_instructions
            .contains(LEGACY_ARTIFACT_TOKEN)
    );
}
