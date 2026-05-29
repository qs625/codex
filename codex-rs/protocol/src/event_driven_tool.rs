use crate::models::ContentItem;
use crate::models::ResponseItem;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;

const START_MARKER: &str = "<event_driven_tool>";
const END_MARKER: &str = "</event_driven_tool>";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, TS, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventDrivenToolTrigger {
    pub tool: String,
    pub title: String,
    pub text: String,
}

impl EventDrivenToolTrigger {
    pub fn render_message_text(&self) -> String {
        let body = serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"tool":"{}","title":"{}","text":"{}"}}"#,
                self.tool, self.title, self.text
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
    use super::EventDrivenToolTrigger;
    use crate::models::ContentItem;
    use pretty_assertions::assert_eq;

    #[test]
    fn event_driven_tool_trigger_round_trips() {
        let trigger = EventDrivenToolTrigger {
            tool: "fs_subscribe".to_string(),
            title: "File watch triggered".to_string(),
            text: "build.log changed".to_string(),
        };

        let text = trigger.render_message_text();
        assert_eq!(
            EventDrivenToolTrigger::parse_message_text(&text),
            Some(trigger.clone())
        );

        let content = vec![ContentItem::InputText { text }];
        assert_eq!(
            EventDrivenToolTrigger::parse_message_content(&content),
            Some(trigger)
        );
    }
}
