use protocol::config_types::CollaborationModeMask as CoreCollaborationModeMask;
use protocol::config_types::ModeKind;
use protocol::openai_models::ReasoningEffort;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

/// EXPERIMENTAL - list collaboration mode presets.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CollaborationModeListParams {}

/// EXPERIMENTAL - collaboration mode preset metadata for clients.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CollaborationModeMask {
    pub name: String,
    pub mode: Option<ModeKind>,
    pub model: Option<String>,
    #[serde(rename = "reasoning_effort")]
    #[cfg_attr(feature = "schema-export", ts(rename = "reasoning_effort"))]
    pub reasoning_effort: Option<Option<ReasoningEffort>>,
}

impl From<CoreCollaborationModeMask> for CollaborationModeMask {
    fn from(value: CoreCollaborationModeMask) -> Self {
        Self {
            name: value.name,
            mode: value.mode,
            model: value.model,
            reasoning_effort: value.reasoning_effort,
        }
    }
}

/// EXPERIMENTAL - collaboration mode presets response.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CollaborationModeListResponse {
    pub data: Vec<CollaborationModeMask>,
}
