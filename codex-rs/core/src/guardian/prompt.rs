pub(crate) use codex_guardian::GuardianPromptItems;
pub(crate) use codex_guardian::GuardianPromptMode;
pub(crate) use codex_guardian::GuardianTranscriptCursor;
pub(crate) use codex_guardian::collect_guardian_transcript_entries;
pub(crate) use codex_guardian::guardian_output_schema;
pub(crate) use codex_guardian::guardian_policy_prompt;
pub(crate) use codex_guardian::guardian_policy_prompt_with_config;
pub(crate) use codex_guardian::parse_guardian_assessment;

use crate::session::session::Session;

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
