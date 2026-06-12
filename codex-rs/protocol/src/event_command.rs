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
    pub fn stable_item_id(&self) -> String {
        let sequence = self
            .sequence
            .map(|sequence| sequence.to_string())
            .unwrap_or_else(|| "terminal".to_string());
        format!(
            "event-command:{}:{}:{}:{}",
            self.subscription_id,
            self.kind.stable_item_id_part(),
            sequence,
            self.created_at
        )
    }

    pub fn render_message_text(&self) -> String {
        let body = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"subscriptionId":"{}","kind":"failed_to_start","command":"{}","truncated":false,"createdAt":{}}}"#,
                self.subscription_id, self.command, self.created_at
            )
        });
        format!("{START_MARKER}{body}{END_MARKER}")
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

impl EventCommandEventKind {
    fn stable_item_id_part(&self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Exited => "exited",
            Self::Cancelled => "cancelled",
            Self::FailedToStart => "failed_to_start",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EventCommandEvent;
    use super::EventCommandEventKind;
    use pretty_assertions::assert_eq;

    #[test]
    fn event_command_event_renders_provider_message() {
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
        assert!(text.starts_with("<event_command>"));
        assert!(text.ends_with("</event_command>"));
    }

    #[test]
    fn event_command_event_stable_item_id_uses_event_identity() {
        let event = EventCommandEvent {
            subscription_id: "sub-1".to_string(),
            kind: EventCommandEventKind::Exited,
            label: Some("tests".to_string()),
            command: "cargo test".to_string(),
            cwd: None,
            line: None,
            sequence: None,
            exit_code: Some(0),
            signal: None,
            message: Some("done".to_string()),
            truncated: false,
            created_at: 1_700_000_000,
        };
        let same_identity = EventCommandEvent {
            label: Some("renamed".to_string()),
            command: "cargo nextest run".to_string(),
            message: Some("still done".to_string()),
            ..event.clone()
        };
        let different_sequence = EventCommandEvent {
            sequence: Some(1),
            ..event.clone()
        };

        assert_eq!(
            event.stable_item_id(),
            "event-command:sub-1:exited:terminal:1700000000"
        );
        assert_eq!(same_identity.stable_item_id(), event.stable_item_id());
        assert_ne!(different_sequence.stable_item_id(), event.stable_item_id());
    }
}
