use app_server_protocol::DynamicToolCallOutputContentItem;
use protocol::models::MessagePhase;
use protocol::protocol::ReviewOutputEvent;

pub(super) struct PendingAgentMessageResponse {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) phase: Option<MessagePhase>,
}

impl PendingAgentMessageResponse {
    pub(super) fn matches(&self, text: &str, phase: Option<&MessagePhase>) -> bool {
        self.text == text && phases_are_compatible(self.phase.as_ref(), phase)
    }
}

fn phases_are_compatible(
    response_phase: Option<&MessagePhase>,
    event_phase: Option<&MessagePhase>,
) -> bool {
    response_phase.is_none() || event_phase.is_none() || response_phase == event_phase
}

pub(super) const REVIEW_FALLBACK_MESSAGE: &str = "Reviewer failed to output a response.";

pub(super) fn render_review_output_text(output: &ReviewOutputEvent) -> String {
    let explanation = output.overall_explanation.trim();
    if explanation.is_empty() {
        REVIEW_FALLBACK_MESSAGE.to_string()
    } else {
        explanation.to_string()
    }
}

pub(super) fn convert_dynamic_tool_content_items(
    items: &[protocol::dynamic_tools::DynamicToolCallOutputContentItem],
) -> Vec<DynamicToolCallOutputContentItem> {
    items
        .iter()
        .cloned()
        .map(|item| match item {
            protocol::dynamic_tools::DynamicToolCallOutputContentItem::InputText { text } => {
                DynamicToolCallOutputContentItem::InputText { text }
            }
            protocol::dynamic_tools::DynamicToolCallOutputContentItem::InputImage { image_url } => {
                DynamicToolCallOutputContentItem::InputImage { image_url }
            }
        })
        .collect()
}
