use codex_protocol::openai_models::ModelsResponse;

#[derive(Debug, Clone, Default)]
pub struct ModelsManagerConfig {
    pub model_context_window: Option<i64>,
    pub model_auto_compact_token_limit: Option<i64>,
    pub tool_output_token_limit: Option<usize>,
    pub base_instructions: Option<String>,
    pub personality_enabled: bool,
    pub model_supports_reasoning_summaries: Option<bool>,
    pub model_catalog: Option<ModelsResponse>,
    pub model_metadata_overrides: Vec<ModelMetadataOverride>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelMetadataOverride {
    pub model: String,
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    pub auto_compact_token_limit: Option<i64>,
}
