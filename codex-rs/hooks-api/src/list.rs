use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::protocol::HookEventName;
use protocol::protocol::HookHandlerType;
use protocol::protocol::HookSource;
use protocol::protocol::HookTrustStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookListEntry {
    pub key: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub matcher: Option<String>,
    pub command: Option<String>,
    pub timeout_sec: u64,
    pub status_message: Option<String>,
    pub source_path: AbsolutePathBuf,
    pub source: HookSource,
    pub plugin_id: Option<String>,
    pub display_order: i64,
    pub enabled: bool,
    pub is_managed: bool,
    pub current_hash: String,
    pub trust_status: HookTrustStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookListOutcome {
    pub hooks: Vec<HookListEntry>,
    pub warnings: Vec<String>,
}
