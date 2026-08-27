//! Responses API tool definition for publishing conversation artifacts.

use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub const PUBLISH_ARTIFACT_TOOL_NAME: &str = "publish_artifact";

pub fn create_publish_artifact_tool() -> ToolSpec {
    let inline_source = JsonSchema::object(
        BTreeMap::from([
            (
                "type".to_string(),
                JsonSchema::string_enum(
                    vec![json!("inline")],
                    Some("Publish inline artifact content.".to_string()),
                ),
            ),
            (
                "content".to_string(),
                JsonSchema::string(Some("Inline artifact content.".to_string())),
            ),
            (
                "mimeType".to_string(),
                JsonSchema::string(Some(
                    "Artifact MIME type, such as text/html or image/svg+xml.".to_string(),
                )),
            ),
            (
                "language".to_string(),
                JsonSchema::string(Some(
                    "Optional source language label for code-like artifacts.".to_string(),
                )),
            ),
        ]),
        Some(vec![
            "type".to_string(),
            "content".to_string(),
            "mimeType".to_string(),
        ]),
        Some(false.into()),
    );
    let url_source = JsonSchema::object(
        BTreeMap::from([
            (
                "type".to_string(),
                JsonSchema::string_enum(
                    vec![json!("url")],
                    Some("Publish an artifact backed by an existing URL.".to_string()),
                ),
            ),
            (
                "url".to_string(),
                JsonSchema::string(Some(
                    "HTTP or HTTPS URL to display as the artifact target.".to_string(),
                )),
            ),
            (
                "mimeType".to_string(),
                JsonSchema::string(Some(
                    "Optional MIME type for the URL-backed artifact.".to_string(),
                )),
            ),
            (
                "fallbackContent".to_string(),
                JsonSchema::string(Some(
                    "Optional bounded fallback text shown when the URL cannot be previewed."
                        .to_string(),
                )),
            ),
        ]),
        Some(vec!["type".to_string(), "url".to_string()]),
        Some(false.into()),
    );
    let properties = BTreeMap::from([
        (
            "title".to_string(),
            JsonSchema::string(Some("Short artifact title shown in the conversation.".to_string())),
        ),
        (
            "source".to_string(),
            JsonSchema::any_of(
                vec![inline_source, url_source],
                Some("Artifact source. Use inline for small content and url for browser-backed artifacts.".to_string()),
            ),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: PUBLISH_ARTIFACT_TOOL_NAME.to_string(),
        description:
            "Publish a declarative conversation artifact as a typed UI item. This is for artifact/UI publishing, not runtime inspection or runtime control."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["title".to_string(), "source".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
