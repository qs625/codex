use protocol::config_types::ReasoningSummary;
use protocol::openai_models::ConfigShellToolType;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ModelInstructionsVariables;
use protocol::openai_models::ModelMessages;
use protocol::openai_models::ModelVisibility;
use protocol::openai_models::TruncationMode;
use protocol::openai_models::TruncationPolicyConfig;
use protocol::openai_models::WebSearchToolType;
use protocol::openai_models::default_input_modalities;

use codex_utils_output_truncation::approx_bytes_for_tokens;
use model_service_api::ModelsManagerConfig;
use tracing::warn;

pub const BASE_INSTRUCTIONS: &str = include_str!("../prompt.md");
const ARTIFACT_PUBLISHING_MARKER: &str = "publish_artifact";
const ARTIFACT_PUBLISHING_INSTRUCTIONS: &str = r#"
# Artifact Publishing

Default assistant replies are Markdown. Use the `publish_artifact` tool when the user needs a distinct previewable artifact, such as self-contained `text/html`, `image/svg+xml`, `text/markdown`, `text/mermaid`, `application/json`, `text/csv`, source/code content, or a URL-backed preview. Artifact content should be self-contained unless published as a URL source.
"#;
const DEFAULT_PERSONALITY_HEADER: &str = "You are Codex, a coding agent based on GPT-5. You and the user share the same workspace and collaborate to achieve the user's goals.";
const LOCAL_FRIENDLY_TEMPLATE: &str =
    "You optimize for team morale and being a supportive teammate as much as code quality.";
const LOCAL_PRAGMATIC_TEMPLATE: &str = "You are a deeply pragmatic, effective software engineer.";
const PERSONALITY_PLACEHOLDER: &str = "{{ personality }}";

pub fn with_config_overrides(mut model: ModelInfo, config: &ModelsManagerConfig) -> ModelInfo {
    if let Some(supports_reasoning_summaries) = config.model_supports_reasoning_summaries
        && supports_reasoning_summaries
    {
        model.supports_reasoning_summaries = true;
    }
    if let Some(context_window) = config.model_context_window {
        model.context_window = Some(
            model
                .max_context_window
                .map_or(context_window, |max_context_window| {
                    context_window.min(max_context_window)
                }),
        );
    }
    if let Some(auto_compact_token_limit) = config.model_auto_compact_token_limit {
        model.auto_compact_token_limit = Some(auto_compact_token_limit);
    }
    if let Some(model_override) = config
        .model_metadata_overrides
        .iter()
        .find(|model_override| model_override.model == model.slug)
    {
        if let Some(max_context_window) = model_override.max_context_window {
            model.max_context_window = Some(max_context_window);
        }
        if let Some(context_window) = model_override.context_window {
            model.context_window = Some(
                model
                    .max_context_window
                    .map_or(context_window, |max_context_window| {
                        context_window.min(max_context_window)
                    }),
            );
        }
        if let Some(auto_compact_token_limit) = model_override.auto_compact_token_limit {
            model.auto_compact_token_limit = Some(auto_compact_token_limit);
        }
    }
    if let Some(token_limit) = config.tool_output_token_limit {
        model.truncation_policy = match model.truncation_policy.mode {
            TruncationMode::Bytes => {
                let byte_limit =
                    i64::try_from(approx_bytes_for_tokens(token_limit)).unwrap_or(i64::MAX);
                TruncationPolicyConfig::bytes(byte_limit)
            }
            TruncationMode::Tokens => {
                let limit = i64::try_from(token_limit).unwrap_or(i64::MAX);
                TruncationPolicyConfig::tokens(limit)
            }
        };
    }

    if let Some(base_instructions) = &config.base_instructions {
        model.base_instructions = ensure_artifact_publishing_instructions(base_instructions.clone());
        model.model_messages = None;
    } else if !config.personality_enabled {
        model.model_messages = None;
    }
    model.base_instructions = ensure_artifact_publishing_instructions(model.base_instructions);
    if let Some(model_messages) = &mut model.model_messages
        && let Some(template) = &mut model_messages.instructions_template
    {
        *template = ensure_artifact_publishing_instructions(std::mem::take(template));
    }

    model
}

pub fn ensure_artifact_publishing_instructions(mut instructions: String) -> String {
    if instructions.contains(ARTIFACT_PUBLISHING_MARKER) {
        return instructions;
    }
    if !instructions.ends_with('\n') {
        instructions.push('\n');
    }
    instructions.push_str(ARTIFACT_PUBLISHING_INSTRUCTIONS);
    instructions
}

/// Build a minimal fallback model descriptor for missing/unknown slugs.
pub fn model_info_from_slug(slug: &str) -> ModelInfo {
    warn!("Unknown model {slug} is used. This will use fallback model metadata.");
    ModelInfo {
        slug: slug.to_string(),
        display_name: slug.to_string(),
        description: None,
        default_reasoning_level: None,
        supported_reasoning_levels: Vec::new(),
        shell_type: ConfigShellToolType::Default,
        visibility: ModelVisibility::None,
        supported_in_api: true,
        priority: 99,
        additional_speed_tiers: Vec::new(),
        service_tiers: Vec::new(),
        availability_nux: None,
        upgrade: None,
        base_instructions: BASE_INSTRUCTIONS.to_string(),
        model_messages: local_personality_messages_for_slug(slug),
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::bytes(/*limit*/ 10_000),
        supports_parallel_tool_calls: false,
        supports_image_detail_original: false,
        context_window: Some(272_000),
        max_context_window: Some(272_000),
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: default_input_modalities(),
        used_fallback_model_metadata: true, // this is the fallback model metadata
        supports_search_tool: false,
    }
}

fn find_model_by_longest_prefix(model: &str, candidates: &[ModelInfo]) -> Option<ModelInfo> {
    let mut best: Option<ModelInfo> = None;
    for candidate in candidates {
        if !model.starts_with(&candidate.slug) {
            continue;
        }
        let is_better_match = if let Some(current) = best.as_ref() {
            candidate.slug.len() > current.slug.len()
        } else {
            true
        };
        if is_better_match {
            best = Some(candidate.clone());
        }
    }
    best
}

fn find_model_by_namespaced_suffix(model: &str, candidates: &[ModelInfo]) -> Option<ModelInfo> {
    let (namespace, suffix) = model.split_once('/')?;
    if suffix.contains('/') {
        return None;
    }
    if namespace.is_empty()
        || !namespace
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return None;
    }
    find_model_by_longest_prefix(suffix, candidates)
}

/// Resolve `ModelInfo` from an online/offline candidate list and apply runtime overrides.
pub fn construct_model_info_from_candidates(
    model: &str,
    candidates: &[ModelInfo],
    config: &ModelsManagerConfig,
) -> ModelInfo {
    let remote = find_model_by_longest_prefix(model, candidates)
        .or_else(|| find_model_by_namespaced_suffix(model, candidates));
    let model_info = if let Some(remote) = remote {
        ModelInfo {
            slug: model.to_string(),
            used_fallback_model_metadata: false,
            ..remote
        }
    } else {
        model_info_from_slug(model)
    };
    with_config_overrides(model_info, config)
}

fn local_personality_messages_for_slug(slug: &str) -> Option<ModelMessages> {
    match slug {
        "gpt-5.2-codex" | "exp-codex-personality" => Some(ModelMessages {
            instructions_template: Some(format!(
                "{DEFAULT_PERSONALITY_HEADER}\n\n{PERSONALITY_PLACEHOLDER}\n\n{BASE_INSTRUCTIONS}"
            )),
            instructions_variables: Some(ModelInstructionsVariables {
                personality_default: Some(String::new()),
                personality_friendly: Some(LOCAL_FRIENDLY_TEMPLATE.to_string()),
                personality_pragmatic: Some(LOCAL_PRAGMATIC_TEMPLATE.to_string()),
            }),
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "model_info_tests.rs"]
mod tests;
