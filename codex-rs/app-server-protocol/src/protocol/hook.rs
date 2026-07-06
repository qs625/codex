use super::shared::camel_case_enum_from_core;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::protocol::HookEventName as CoreHookEventName;
use protocol::protocol::HookExecutionMode as CoreHookExecutionMode;
use protocol::protocol::HookHandlerType as CoreHookHandlerType;
use protocol::protocol::HookOutputEntry as CoreHookOutputEntry;
use protocol::protocol::HookOutputEntryKind as CoreHookOutputEntryKind;
use protocol::protocol::HookRunStatus as CoreHookRunStatus;
use protocol::protocol::HookRunSummary as CoreHookRunSummary;
use protocol::protocol::HookScope as CoreHookScope;
use protocol::protocol::HookSource as CoreHookSource;
use protocol::protocol::HookTrustStatus as CoreHookTrustStatus;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

camel_case_enum_from_core!(
    pub enum HookEventName from CoreHookEventName {
        PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SessionStart, UserPromptSubmit, Stop
    }
);

camel_case_enum_from_core!(
    pub enum HookHandlerType from CoreHookHandlerType {
        Command, Prompt, Agent
    }
);

camel_case_enum_from_core!(
    pub enum HookExecutionMode from CoreHookExecutionMode {
        Sync, Async
    }
);

camel_case_enum_from_core!(
    pub enum HookScope from CoreHookScope {
        Thread, Turn
    }
);

camel_case_enum_from_core!(
    pub enum HookSource from CoreHookSource {
        System,
        User,
        Project,
        Mdm,
        SessionFlags,
        Plugin,
        CloudRequirements,
        LegacyManagedConfigFile,
        LegacyManagedConfigMdm,
        Unknown,
    }
);

camel_case_enum_from_core!(
    pub enum HookTrustStatus from CoreHookTrustStatus {
        Managed, Untrusted, Trusted, Modified
    }
);

fn default_hook_source() -> HookSource {
    HookSource::Unknown
}

camel_case_enum_from_core!(
    pub enum HookRunStatus from CoreHookRunStatus {
        Running, Completed, Failed, Blocked, Stopped
    }
);

camel_case_enum_from_core!(
    pub enum HookOutputEntryKind from CoreHookOutputEntryKind {
        Warning, Stop, Feedback, Context, Error
    }
);

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct HookOutputEntry {
    pub kind: HookOutputEntryKind,
    pub text: String,
}

impl From<CoreHookOutputEntry> for HookOutputEntry {
    fn from(value: CoreHookOutputEntry) -> Self {
        Self {
            kind: value.kind.into(),
            text: value.text,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct HookRunSummary {
    pub id: String,
    pub event_name: HookEventName,
    pub handler_type: HookHandlerType,
    pub execution_mode: HookExecutionMode,
    pub scope: HookScope,
    pub source_path: AbsolutePathBuf,
    #[serde(default = "default_hook_source")]
    pub source: HookSource,
    pub display_order: i64,
    pub status: HookRunStatus,
    pub status_message: Option<String>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub entries: Vec<HookOutputEntry>,
}

impl From<CoreHookRunSummary> for HookRunSummary {
    fn from(value: CoreHookRunSummary) -> Self {
        Self {
            id: value.id,
            event_name: value.event_name.into(),
            handler_type: value.handler_type.into(),
            execution_mode: value.execution_mode.into(),
            scope: value.scope.into(),
            source_path: value.source_path,
            source: value.source.into(),
            display_order: value.display_order,
            status: value.status.into(),
            status_message: value.status_message,
            started_at: value.started_at,
            completed_at: value.completed_at,
            duration_ms: value.duration_ms,
            entries: value.entries.into_iter().map(Into::into).collect(),
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct HookStartedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct HookCompletedNotification {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub run: HookRunSummary,
}
