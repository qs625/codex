use protocol::items::AgentMessageContent;
use protocol::items::AgentMessageItem;
use protocol::items::ConversationArtifactItem;
use protocol::items::TurnItem;
use serde_json::Value;

pub const ARTIFACT_MARKER_START_PREFIX: &str = "<<<MORPHEUS_ARTIFACT ";
pub const ARTIFACT_MARKER_START_SUFFIX: &str = ">>>";
pub const ARTIFACT_MARKER_END: &str = "<<<END_MORPHEUS_ARTIFACT>>>";
pub const MAX_CONVERSATION_ARTIFACT_CONTENT_BYTES: usize = 256 * 1024;
const MAX_ARTIFACT_TITLE_CHARS: usize = 120;
const MAX_ARTIFACT_MIME_TYPE_CHARS: usize = 100;
const MAX_ARTIFACT_LANGUAGE_CHARS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantArtifactPart {
    Text(String),
    Artifact {
        title: String,
        mime_type: String,
        content: String,
        language: Option<String>,
        truncated: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactMetadata {
    title: String,
    mime_type: String,
    language: Option<String>,
}

pub fn split_assistant_artifact_markers(text: &str) -> Vec<AssistantArtifactPart> {
    let mut parts = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find(ARTIFACT_MARKER_START_PREFIX) {
        let start = cursor + relative_start;
        let metadata_start = start + ARTIFACT_MARKER_START_PREFIX.len();
        let Some(relative_metadata_end) = text[metadata_start..].find(ARTIFACT_MARKER_START_SUFFIX)
        else {
            break;
        };
        let metadata_end = metadata_start + relative_metadata_end;
        let content_start = metadata_end + ARTIFACT_MARKER_START_SUFFIX.len();
        let Some(relative_end) = text[content_start..].find(ARTIFACT_MARKER_END) else {
            break;
        };
        let content_end = content_start + relative_end;
        let marker_end = content_end + ARTIFACT_MARKER_END.len();

        let metadata_json = text[metadata_start..metadata_end].trim();
        let Some(metadata) = parse_artifact_metadata(metadata_json) else {
            break;
        };

        push_text_part(&mut parts, &text[cursor..start]);
        let (content, truncated) = truncate_to_byte_limit(
            &text[content_start..content_end],
            MAX_CONVERSATION_ARTIFACT_CONTENT_BYTES,
        );
        parts.push(AssistantArtifactPart::Artifact {
            title: metadata.title,
            mime_type: metadata.mime_type,
            content,
            language: metadata.language,
            truncated,
        });
        cursor = marker_end;
    }

    if parts.is_empty() {
        parts.push(AssistantArtifactPart::Text(text.to_string()));
    } else {
        push_text_part(&mut parts, &text[cursor..]);
    }

    parts
}

pub fn split_agent_message_into_artifact_turn_items(
    agent_message: AgentMessageItem,
) -> Vec<TurnItem> {
    let AgentMessageItem {
        id,
        content,
        phase,
        memory_citation,
    } = agent_message;
    let text = content
        .iter()
        .map(|entry| match entry {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect::<String>();
    let parts = split_assistant_artifact_markers(&text);
    if parts.len() == 1 && matches!(parts.first(), Some(AssistantArtifactPart::Text(_))) {
        return vec![TurnItem::AgentMessage(AgentMessageItem {
            id,
            content,
            phase,
            memory_citation,
        })];
    }

    let mut artifact_index = 0usize;
    let mut text_index = 0usize;
    let mut items = Vec::new();
    for part in parts {
        let item_id = if items.is_empty() {
            id.clone()
        } else {
            match &part {
                AssistantArtifactPart::Text(_) => {
                    let value = format!("{id}-text-{text_index}");
                    text_index += 1;
                    value
                }
                AssistantArtifactPart::Artifact { .. } => {
                    let value = format!("{id}-artifact-{artifact_index}");
                    artifact_index += 1;
                    value
                }
            }
        };
        match part {
            AssistantArtifactPart::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                items.push(TurnItem::AgentMessage(AgentMessageItem {
                    id: item_id,
                    content: vec![AgentMessageContent::Text { text }],
                    phase: phase.clone(),
                    memory_citation: memory_citation.clone(),
                }));
            }
            AssistantArtifactPart::Artifact {
                title,
                mime_type,
                content,
                language,
                truncated,
            } => {
                items.push(TurnItem::ConversationArtifact(ConversationArtifactItem {
                    id: item_id,
                    title,
                    mime_type,
                    content,
                    language,
                    truncated,
                }));
            }
        }
    }
    if items.is_empty() {
        items.push(TurnItem::AgentMessage(AgentMessageItem {
            id,
            content: Vec::new(),
            phase,
            memory_citation,
        }));
    }
    items
}

pub fn strip_artifact_markers_for_display(text: &str) -> String {
    split_assistant_artifact_markers(text)
        .into_iter()
        .filter_map(|part| match part {
            AssistantArtifactPart::Text(text) => Some(text),
            AssistantArtifactPart::Artifact { .. } => None,
        })
        .collect()
}

fn parse_artifact_metadata(metadata_json: &str) -> Option<ArtifactMetadata> {
    let value: Value = serde_json::from_str(metadata_json).ok()?;
    let title = string_field(&value, "title")
        .map(|value| clamp_chars(value.trim(), MAX_ARTIFACT_TITLE_CHARS))
        .filter(|value| !value.is_empty())?;
    let mime_type = string_field(&value, "mime_type")
        .or_else(|| string_field(&value, "mimeType"))
        .map(|value| clamp_chars(value.trim(), MAX_ARTIFACT_MIME_TYPE_CHARS))
        .filter(|value| !value.is_empty())?;
    let language = string_field(&value, "language")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| clamp_chars(value, MAX_ARTIFACT_LANGUAGE_CHARS));
    Some(ArtifactMetadata {
        title,
        mime_type,
        language,
    })
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

fn push_text_part(parts: &mut Vec<AssistantArtifactPart>, text: &str) {
    if !text.is_empty() {
        parts.push(AssistantArtifactPart::Text(text.to_string()));
    }
}

fn truncate_to_byte_limit(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn clamp_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}
