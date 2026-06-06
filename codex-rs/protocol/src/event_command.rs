use crate::models::ContentItem;
use crate::models::ResponseItem;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

const START_MARKER: &str = "<event_command>";
const END_MARKER: &str = "</event_command>";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EventCommandEvent {
    pub subscription_id: String,
    pub kind: EventCommandEventKind,
    pub label: Option<String>,
    pub command: String,
    pub cwd: Option<String>,
    pub line: Option<String>,
    pub sequence: Option<u32>,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub message: Option<String>,
    pub truncated: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash, TS, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "camelCase")]
pub enum EventCommandEventKind {
    Output,
    Exited,
    Cancelled,
    FailedToStart,
}

impl EventCommandEvent {
    pub fn render_message_text(&self) -> String {
        let body = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"subscriptionId":"{}","kind":"failed_to_start","command":"{}","truncated":false,"createdAt":{}}}"#,
                self.subscription_id, self.command, self.created_at
            )
        });
        format!("{START_MARKER}{body}{END_MARKER}")
    }

    pub fn parse_message_text(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        let body = trimmed
            .strip_prefix(START_MARKER)?
            .strip_suffix(END_MARKER)?
            .trim();
        serde_json::from_str(body).ok()
    }

    pub fn parse_message_content(content: &[ContentItem]) -> Option<Self> {
        let [ContentItem::InputText { text }] = content else {
            return None;
        };
        Self::parse_message_text(text)
    }

    pub fn to_response_item(&self) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: self.render_message_text(),
            }],
            phase: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EventCommandEvent;
    use super::EventCommandEventKind;
    use crate::models::ContentItem;
    use pretty_assertions::assert_eq;

    #[test]
    fn event_command_event_round_trips() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".to_string(),
            kind: EventCommandEventKind::Output,
            label: Some("tests".to_string()),
            command: "cargo test".to_string(),
            cwd: Some("/repo".to_string()),
            line: Some("done".to_string()),
            sequence: Some(1),
            exit_code: None,
            signal: None,
            message: None,
            truncated: false,
            created_at: 1_775_000_000,
        };

        let text = event.render_message_text();
        assert_eq!(
            EventCommandEvent::parse_message_text(&text),
            Some(event.clone())
        );

        let content = vec![ContentItem::InputText { text }];
        assert_eq!(
            EventCommandEvent::parse_message_content(&content),
            Some(event)
        );
    }
}
