use std::collections::HashMap;

use codex_context_manager::is_contextual_user_message_content;
pub(crate) use codex_guardian::GuardianPromptItems;
pub(crate) use codex_guardian::GuardianPromptMode;
pub(crate) use codex_guardian::GuardianTranscriptCursor;
pub(crate) use codex_guardian::GuardianTranscriptEntry;
pub(crate) use codex_guardian::GuardianTranscriptEntryKind;
pub(crate) use codex_guardian::guardian_output_schema;
pub(crate) use codex_guardian::guardian_policy_prompt;
pub(crate) use codex_guardian::guardian_policy_prompt_with_config;
pub(crate) use codex_guardian::parse_guardian_assessment;
use codex_protocol::models::ResponseItem;

use crate::compact::content_items_to_text;
use crate::session::session::Session;

use super::AUTO_REVIEW_DENIED_ACTION_APPROVAL_DEVELOPER_PREFIX;
use super::GuardianApprovalRequest;

/// Builds the guardian user content items from:
/// - a compact transcript for authorization and local context
/// - the exact action JSON being proposed for approval
///
/// The fixed guardian policy lives in the review session developer message.
/// Split the variable request into separate user content items so the
/// Responses request snapshot shows clear boundaries while preserving exact
/// prompt text through trailing newlines.
pub(crate) async fn build_guardian_prompt_items(
    session: &Session,
    retry_reason: Option<String>,
    request: GuardianApprovalRequest,
    mode: GuardianPromptMode,
) -> serde_json::Result<GuardianPromptItems> {
    let history = session.clone_history().await;
    let transcript_entries = collect_guardian_transcript_entries(history.raw_items());
    codex_guardian::build_guardian_prompt_items_from_entries(
        &session.conversation_id.to_string(),
        history.history_version(),
        transcript_entries.as_slice(),
        retry_reason,
        request,
        mode,
    )
}

/// Retains the human-readable conversation plus recent tool call / result
/// evidence for guardian review and skips synthetic contextual scaffolding that
/// would just add noise because the guardian reviewer already gets the normal
/// inherited top-level context from session startup.
///
/// Keep both tool calls and tool results here. The reviewer often needs the
/// agent's exact queried path / arguments as well as the returned evidence to
/// decide whether the pending approval is justified.
pub(crate) fn collect_guardian_transcript_entries(
    items: &[ResponseItem],
) -> Vec<GuardianTranscriptEntry> {
    let mut entries = Vec::new();
    let mut tool_names_by_call_id = HashMap::new();
    let non_empty_entry = |kind, text: String| {
        (!text.trim().is_empty()).then_some(GuardianTranscriptEntry { kind, text })
    };
    let content_entry =
        |kind, content| content_items_to_text(content).and_then(|text| non_empty_entry(kind, text));
    let serialized_entry =
        |kind, serialized: Option<String>| serialized.and_then(|text| non_empty_entry(kind, text));

    for item in items {
        let entry = match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                if is_contextual_user_message_content(content) {
                    None
                } else {
                    content_entry(GuardianTranscriptEntryKind::User, content)
                }
            }
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                content_items_to_text(content).and_then(|text| {
                    // Preserve only the explicit auto-review approval marker for
                    // Guardian context; other developer messages are intentionally
                    // excluded from the review transcript.
                    text.starts_with(AUTO_REVIEW_DENIED_ACTION_APPROVAL_DEVELOPER_PREFIX)
                        .then_some(GuardianTranscriptEntry {
                            kind: GuardianTranscriptEntryKind::Developer,
                            text,
                        })
                })
            }
            ResponseItem::Message { role, content, .. } if role == "assistant" => {
                content_entry(GuardianTranscriptEntryKind::Assistant, content)
            }
            ResponseItem::LocalShellCall { action, .. } => serialized_entry(
                GuardianTranscriptEntryKind::Tool("tool shell call".to_string()),
                serde_json::to_string(action).ok(),
            ),
            ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                tool_names_by_call_id.insert(call_id.clone(), name.clone());
                (!arguments.trim().is_empty()).then(|| GuardianTranscriptEntry {
                    kind: GuardianTranscriptEntryKind::Tool(format!("tool {name} call")),
                    text: arguments.clone(),
                })
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                tool_names_by_call_id.insert(call_id.clone(), name.clone());
                (!input.trim().is_empty()).then(|| GuardianTranscriptEntry {
                    kind: GuardianTranscriptEntryKind::Tool(format!("tool {name} call")),
                    text: input.clone(),
                })
            }
            ResponseItem::WebSearchCall { action, .. } => action.as_ref().and_then(|action| {
                serialized_entry(
                    GuardianTranscriptEntryKind::Tool("tool web_search call".to_string()),
                    serde_json::to_string(action).ok(),
                )
            }),
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => output.body.to_text().and_then(|text| {
                non_empty_entry(
                    GuardianTranscriptEntryKind::Tool(
                        tool_names_by_call_id.get(call_id).map_or_else(
                            || "tool result".to_string(),
                            |name| format!("tool {name} result"),
                        ),
                    ),
                    text,
                )
            }),
            _ => None,
        };

        if let Some(entry) = entry {
            entries.push(entry);
        }
    }

    entries
}
