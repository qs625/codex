use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::InterAgentCommunication;

/// Typed input buffered between mailbox/user/tool events and the next model
/// request.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PendingInputItem {
    HookInspectable(ResponseItem),
    ResponseItem(ResponseItem),
    InterAgentCommunication(InterAgentCommunication),
}

impl PendingInputItem {
    pub(crate) fn trigger_turn(&self) -> bool {
        match self {
            Self::InterAgentCommunication(communication) => communication.trigger_turn,
            Self::HookInspectable(_) | Self::ResponseItem(_) => true,
        }
    }

    pub(crate) fn into_response_item(self) -> ResponseItem {
        match self {
            Self::HookInspectable(item) => item,
            Self::ResponseItem(item) => item,
            Self::InterAgentCommunication(communication) => ResponseItem::InterAgentCommunication {
                id: None,
                communication,
            },
        }
    }
}

impl From<ResponseInputItem> for PendingInputItem {
    fn from(value: ResponseInputItem) -> Self {
        Self::HookInspectable(value.into())
    }
}

impl From<ResponseItem> for PendingInputItem {
    fn from(value: ResponseItem) -> Self {
        match value {
            ResponseItem::InterAgentCommunication { communication, .. } => {
                Self::InterAgentCommunication(communication)
            }
            value => Self::ResponseItem(value),
        }
    }
}

impl From<InterAgentCommunication> for PendingInputItem {
    fn from(value: InterAgentCommunication) -> Self {
        Self::InterAgentCommunication(value)
    }
}
