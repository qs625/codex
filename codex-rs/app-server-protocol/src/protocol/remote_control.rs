#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

/// Current remote-control connection status and remote identity exposed to clients.
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct RemoteControlStatusChangedNotification {
    pub status: RemoteControlConnectionStatus,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct RemoteControlEnableResponse {
    pub status: RemoteControlConnectionStatus,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct RemoteControlDisableResponse {
    pub status: RemoteControlConnectionStatus,
    pub installation_id: String,
    pub environment_id: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum RemoteControlConnectionStatus {
    Disabled,
    Connecting,
    Connected,
    Errored,
}

impl From<RemoteControlStatusChangedNotification> for RemoteControlEnableResponse {
    fn from(notification: RemoteControlStatusChangedNotification) -> Self {
        let RemoteControlStatusChangedNotification {
            status,
            installation_id,
            environment_id,
        } = notification;
        Self {
            status,
            installation_id,
            environment_id,
        }
    }
}

impl From<RemoteControlStatusChangedNotification> for RemoteControlDisableResponse {
    fn from(notification: RemoteControlStatusChangedNotification) -> Self {
        let RemoteControlStatusChangedNotification {
            status,
            installation_id,
            environment_id,
        } = notification;
        Self {
            status,
            installation_id,
            environment_id,
        }
    }
}
