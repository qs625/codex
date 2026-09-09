use protocol::models::ResponseInputItem;
use protocol::models::ResponseItem;
use protocol::protocol::InterAgentCommunication;

/// Typed input buffered between mailbox/user/tool events and the next model
/// request.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingInputItem {
    HookInspectable(ResponseItem),
    ResponseItem(ResponseItem),
    RecordedResponseItem {
        response_item: ResponseItem,
        recovery_id: String,
    },
    InterAgentCommunication(InterAgentCommunication),
}

impl PendingInputItem {
    pub fn trigger_turn(&self) -> bool {
        match self {
            Self::InterAgentCommunication(communication) => communication.trigger_turn,
            Self::HookInspectable(_)
            | Self::ResponseItem(_)
            | Self::RecordedResponseItem { .. } => true,
        }
    }

    pub fn recovery_id(&self) -> Option<&str> {
        match self {
            Self::RecordedResponseItem { recovery_id, .. } => Some(recovery_id),
            _ => None,
        }
    }

    pub fn into_response_item(self) -> ResponseItem {
        match self {
            Self::HookInspectable(item) => item,
            Self::ResponseItem(item) => item,
            Self::RecordedResponseItem { response_item, .. } => response_item,
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
